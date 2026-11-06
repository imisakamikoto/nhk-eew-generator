"""Find the exact font weight / size / horizontal scale used by the reference screenshots."""
import glob
import os

import numpy as np
from PIL import Image, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ref = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(int)


def glyph_mask(text, font, scale_x=1.0):
    f = font
    bbox = f.getbbox(text, anchor="lt", stroke_width=0)
    w = max(1, int(bbox[2] - bbox[0]) + 8)
    h = max(1, int(bbox[3] - bbox[1]) + 8)
    im = Image.new("L", (w, h), 0)
    ImageDraw = __import__("PIL.ImageDraw", fromlist=["Draw"]).Draw
    ImageDraw(im).text((4 - bbox[0], 4 - bbox[1]), text, font=f, fill=255, anchor="lt")
    ar = np.array(im) > 127
    if scale_x != 1.0:
        nw = max(1, int(round(w * scale_x)))
        ar = np.array(Image.fromarray((ar * 255).astype(np.uint8)).resize((nw, h), Image.Resampling.BILINEAR)) > 127
    return ar


def trim(mask):
    ys, xs = np.where(mask)
    if len(xs) == 0:
        return mask
    return mask[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


def iou(a, b):
    inter = np.logical_and(a, b).sum()
    union = np.logical_or(a, b).sum()
    return inter / max(1, union)


def best_align(a, b, rng=4):
    # a is rendered glyph (smaller), b is reference (larger); slide a over b
    best = (0, 0, 0)
    H, W = b.shape
    for dy in range(-rng, rng + 1):
        for dx in range(-rng, rng + 1):
            canvas = np.zeros_like(b)
            y0, x0 = dy, dx
            ah, aw = a.shape
            ys0, ys1 = max(0, y0), min(H, y0 + ah)
            xs0, xs1 = max(0, x0), min(W, x0 + aw)
            if ys1 <= ys0 or xs1 <= xs0:
                continue
            canvas[ys0:ys1, xs0:xs1] = a[ys0 - y0:ys1 - y0, xs0 - x0:xs1 - x0]
            v = iou(canvas, b)
            if v > best[0]:
                best = (v, dx, dy)
    return best


def reference(text_region, threshold, box):
    x0, y0, x1, y1 = box
    m = threshold(ref[y0:y1, x0:x1])
    return m


def main():
    white = lambda a: (a[:, :, 0] > 248) & (a[:, :, 1] > 250) & (a[:, :, 2] > 250)
    target = trim(reference("line1", white, (345, 180, 465, 262)))
    print("target shape", target.shape, "pixels", target.sum())
    results = []
    for path in sorted(glob.glob(os.path.join(ROOT, "assets", "fonts", "*.otf"))):
        for size in range(56, 90, 2):
            font = ImageFont.truetype(path, size)
            for sx in (0.78, 0.82, 0.86, 0.90, 0.94, 1.0):
                m = trim(glyph_mask("福岡", font, sx))
                v, dx, dy = best_align(m, target)
                results.append((v, os.path.basename(path), size, sx, m.shape, (dx, dy)))
    results.sort(reverse=True)
    for r in results[:20]:
        print("  score %.4f %-24s size=%d sx=%.2f shape=%s shift=%s" % r)
    print("=== per-font best")
    for path in sorted(glob.glob(os.path.join(ROOT, "assets", "fonts", "*.otf"))):
        b = max(r for r in results if r[1] == os.path.basename(path))
        print("  %-24s score %.4f size=%d sx=%.2f shape=%s" % (b[1], b[0], b[2], b[3], b[4]))


if __name__ == "__main__":
    main()
