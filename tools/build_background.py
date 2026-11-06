"""Build pixel-exact static background plates from the reference screenshots.

The blue panel is a smooth 2D gradient; it is recovered by a robust polynomial
fit to clean (non-text) pixels, then used to erase the dynamic text. Static
parts (black/white rounded frame, pink bar with the fixed title, white divider)
are copied verbatim. The cancellation plate has no dynamic content, so the
reference is used as-is.
"""
import os

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets")
os.makedirs(OUT, exist_ok=True)


def fit_gradient(rgb, x0, y0, x1, y1):
    """Robust bicubic-ish polynomial fit of the blue background in a rectangle."""
    sub = rgb[y0:y1, x0:x1].astype(float)
    hh, ww = sub.shape[:2]
    yy, xx = np.mgrid[0:hh, 0:ww]
    u = xx / max(1, ww - 1)
    v = yy / max(1, hh - 1)
    feats = np.stack([np.ones_like(u), u, v, u * u, v * v, u * v,
                      u ** 3, v ** 3, u * u * v, u * v * v,
                      u ** 4, v ** 4, u ** 2 * v ** 2], axis=-1).reshape(-1, 13)
    # initial clean mask: blue-ish, not white/yellow/gray-outlined text
    r, g, b = sub[:, :, 0], sub[:, :, 1], sub[:, :, 2]
    clean = (r < 90) & (g < 150) & (b > 140) & (b > g + 40) & (g > r + 25)
    coef = None
    for _ in range(3):
        vals = sub.reshape(-1, 3)[clean.reshape(-1)]
        X = feats[clean.reshape(-1)]
        coef, *_ = np.linalg.lstsq(X, vals, rcond=None)
        pred = (feats @ coef).reshape(hh, ww, 3)
        resid = np.abs(sub - pred).max(2)
        clean = clean & (resid < 2.5)
    pred = np.clip((feats @ coef).reshape(hh, ww, 3), 0, 255)
    print("  gradient fit: clean px", int(clean.sum()), "/", hh * ww,
          "max residual on clean", round(float(resid[clean].max()), 2) if clean.any() else "-")
    return pred


def build_alert():
    a = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGBA")).astype(np.uint8)
    h, w, _ = a.shape
    out = a.copy()
    # blue panel: erase dynamic text with the recovered gradient
    x0, y0, x1, y1 = 340, 94, 1202, 399
    pred = fit_gradient(out[:, :, :3], x0, y0, x1, y1)
    out[y0:y1, x0:x1, :3] = pred.astype(np.uint8)
    # map panel: flat sea colour (the rectangle contains no frame pixels)
    mx0, my0, mx1, my1 = 15, 13, 332, 399
    out[my0:my1, mx0:mx1, :3] = np.array([80, 85, 141], dtype=np.uint8)
    open(os.path.join(OUT, "bg_alert.bin"), "wb").write(out.tobytes())
    Image.fromarray(out, "RGBA").save(os.path.join(OUT, "bg_alert.png"))
    print("  bg_alert.bin", (w, h), out.nbytes, "bytes")


def build_cancel():
    a = np.array(Image.open(os.path.join(ROOT, "NHK地震警报取消.png")).convert("RGBA")).astype(np.uint8)
    open(os.path.join(OUT, "bg_cancel.bin"), "wb").write(a.tobytes())
    print("  bg_cancel.bin", a.shape[1::-1], a.nbytes, "bytes")


def build_x_sprite():
    a = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGBA")).astype(float)
    x0, y0, x1, y1 = (48, 118, 114, 184)
    sub = a[y0:y1, x0:x1].copy()
    rgb = sub[:, :, :3]
    sea = np.array([80, 85, 141], dtype=float)
    white = np.array([255, 255, 255], dtype=float)
    red = np.array([235, 42, 85], dtype=float)
    alpha = np.zeros(sub.shape[:2], dtype=float)
    fg = np.zeros_like(rgb)
    for col in (white, red):
        d = col - sea
        t = np.clip(((rgb - sea) * d).sum(2) / (d * d).sum(), 0, 1)
        err = np.abs(rgb - (sea + t[:, :, None] * d)).sum(2)
        take = (err < 90) & (t > alpha)
        alpha = np.where(take, t, alpha)
        fg = np.where(take[:, :, None], col, fg)
    alpha = np.where(alpha > 0.08, alpha, 0)
    # keep only the largest connected component (removes stray speckles)
    seen = np.zeros_like(alpha, dtype=bool)
    best = []
    hh0, ww0 = alpha.shape
    for sy in range(hh0):
        for sx in range(ww0):
            if seen[sy, sx] or alpha[sy, sx] == 0:
                continue
            stack, comp = [(sy, sx)], []
            seen[sy, sx] = True
            while stack:
                cy, cx = stack.pop()
                comp.append((cy, cx))
                for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    ny, nx = cy + dy, cx + dx
                    if 0 <= ny < hh0 and 0 <= nx < ww0 and not seen[ny, nx] and alpha[ny, nx] > 0:
                        seen[ny, nx] = True
                        stack.append((ny, nx))
            if len(comp) > len(best):
                best = comp
    keepmask = np.zeros_like(alpha, dtype=bool)
    for cy, cx in best:
        keepmask[cy, cx] = True
    alpha = np.where(keepmask, alpha, 0)
    sub[:, :, 3] = np.clip(alpha * 255, 0, 255).astype(np.uint8)
    sub[:, :, :3] = np.where(alpha[:, :, None] > 0.08, fg, white).astype(np.uint8)
    out = sub.astype(np.uint8)
    hh, ww = out.shape[:2]
    open(os.path.join(OUT, "x_marker.bin"), "wb").write(out.tobytes())
    Image.fromarray(out, "RGBA").resize((ww * 4, hh * 4), Image.Resampling.NEAREST).save(
        os.path.join(ROOT, "analysis", "x_sprite.png"))
    print("  x_marker.bin", (ww, hh), "opaque px", int((alpha > 0.5).sum()))


def main():
    build_alert()
    build_cancel()
    build_x_sprite()
    print("DONE")


if __name__ == "__main__":
    main()
