"""Match font weight / size / horizontal scale for each text line in the references."""
import glob
import os

import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONTS = sorted(glob.glob(os.path.join(ROOT, "assets", "fonts", "*.otf")))


def mask_for(text, font_path, size, sx):
    f = ImageFont.truetype(font_path, size)
    bb = f.getbbox(text, anchor="lt")
    w = int(bb[2] - bb[0]) + 10
    h = int(bb[3] - bb[1]) + 10
    im = Image.new("L", (w, h), 0)
    ImageDraw.Draw(im).text((5 - bb[0], 5 - bb[1]), text, font=f, fill=255, anchor="lt")
    ar = np.array(im) > 127
    if sx != 1.0:
        ar = np.array(Image.fromarray((ar * 255).astype(np.uint8)).resize(
            (max(1, int(round(w * sx))), h), Image.Resampling.BILINEAR)) > 127
    ys, xs = np.where(ar)
    return ar[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


def align_score(a, b, rng=5):
    H, W = b.shape
    best = 0.0
    for dy in range(-rng, rng + 1):
        for dx in range(-rng, rng + 1):
            c = np.zeros_like(b)
            ah, aw = a.shape
            ys0, ys1 = max(0, dy), min(H, dy + ah)
            xs0, xs1 = max(0, dx), min(W, dx + aw)
            if ys1 <= ys0 or xs1 <= xs0:
                continue
            c[ys0:ys1, xs0:xs1] = a[ys0 - dy:ys1 - dy, xs0 - dx:xs1 - dx]
            inter = np.logical_and(c, b).sum()
            union = np.logical_or(c, b).sum()
            v = inter / max(1, union)
            best = max(best, v)
    return best


def target(fn, cond, box, trim_x=None):
    a = np.array(Image.open(os.path.join(ROOT, fn)).convert("RGB")).astype(int)
    x0, y0, x1, y1 = box
    m = cond(a[y0:y1, x0:x1])
    ys, xs = np.where(m)
    return m[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


CASES = [
    ("prefecture 福", "NHK地震速报屏模板.png",
     lambda a: (a[:, :, 0] > 248) & (a[:, :, 1] > 250) & (a[:, :, 2] > 250),
     (345, 180, 410, 262), "福", range(56, 82, 2)),
    ("yellow 福", "NHK地震速报屏模板.png",
     lambda a: (a[:, :, 0] > 230) & (a[:, :, 1] > 230) & (a[:, :, 2] < 100),
     (345, 98, 405, 168), "福", range(38, 66, 2)),
    ("title 緊", "NHK地震速报屏模板.png",
     lambda a: (a[:, :, 0] > 248) & (a[:, :, 1] > 248) & (a[:, :, 2] > 248),
     (435, 15, 510, 90), "緊", range(46, 76, 2)),
    ("cancel 気", "NHK地震警报取消.png",
     lambda a: (np.abs(a[:, :, 0] - 241) < 6) & (np.abs(a[:, :, 1] - 241) < 6) & (np.abs(a[:, :, 2] - 245) < 8),
     (150, 120, 215, 185), "気", range(44, 72, 2)),
]


def main():
    for label, fn, cond, box, ch, sizes in CASES:
        t = target(fn, cond, box)
        print("=== %s target shape %s pixels %d" % (label, t.shape, t.sum()))
        res = []
        for fp in FONTS:
            for size in sizes:
                for sx in (0.74, 0.78, 0.82, 0.86, 0.90, 0.94, 1.0):
                    m = mask_for(ch, fp, size, sx)
                    res.append((align_score(m, t), os.path.basename(fp), size, sx, m.shape))
        res.sort(reverse=True)
        for r in res[:6]:
            print("   %.4f %-24s size=%d sx=%.2f shape=%s" % r)
        print("   per font best:")
        for fp in FONTS:
            b = max(r for r in res if r[1] == os.path.basename(fp))
            print("     %-24s %.4f size=%d sx=%.2f" % (b[1], b[0], b[2], b[3]))


if __name__ == "__main__":
    main()
