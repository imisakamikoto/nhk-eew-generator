"""Compare generated demo.png with the reference screenshot."""
import os
import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ref = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(int)
gen = np.array(Image.open(os.path.join(ROOT, "output", "demo.png")).convert("RGB")).astype(int)


def bbox(mask, label):
    ys, xs = np.where(mask)
    if len(xs) == 0:
        print("  %-22s NONE" % label)
        return
    print("  %-22s x %4d-%4d  y %3d-%3d  n=%d" % (label, xs.min(), xs.max(), ys.min(), ys.max(), len(xs)))


def report(a, tag):
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    white = (r > 248) & (g > 250) & (b > 250)
    yellow = (r > 230) & (g > 230) & (b < 110)
    red = (r > 200) & (g < 90) & (b < 140)
    gray = (abs(r - 131) < 12) & (abs(g - 131) < 12) & (abs(b - 131) < 12)
    print("=== %s" % tag)
    # restrict to right panel for text
    w = white.copy(); w[:, :340] = False; w[:94, :] = False; w[:, 1202:] = False
    bbox(w, "white text (right)")
    y = yellow.copy(); y[:, :340] = False; y[:94, :] = False; y[:, 1202:] = False
    bbox(y, "yellow text (right)")
    # row bands
    for name, m in (("white", w), ("yellow", y)):
        rh = m.sum(1)
        bands = []
        on = False
        for i, v in enumerate(rh):
            if v > 8 and not on:
                s = i; on = True
            if v <= 8 and on:
                bands.append((s, i - 1)); on = False
        print("    %s row bands %s" % (name, bands[:8]))
    # gaps between names (white columns)
    wr = w[180:262, :].sum(0)
    gaps = []
    s = None
    for i, v in enumerate(wr):
        if v == 0 and s is None: s = i
        if v > 0 and s is not None:
            if i - s >= 8: gaps.append((s, i - 1))
            s = None
    print("    white gaps line1", gaps[:10])
    # map area
    red2 = red.copy(); red2[:100, :] = False; red2[:, 340:] = False
    bbox(red2, "X red (map)")
    ymap = yellow.copy(); ymap[:, 340:] = False
    bbox(ymap, "yellow highlight")
    gray2 = gray.copy(); gray2[:, 340:] = False
    bbox(gray2, "gray land")


report(ref, "REFERENCE")
report(gen, "GENERATED")

# overall difference
d = np.abs(ref - gen).astype(float)
print("=== diff: mean %.2f  max %.0f  pixels>20 %.1f%%" %
      (d.mean(), d.max(), (d.max(2) > 20).mean() * 100))
# difference in specific areas
for name, box in [("title", (340, 13, 1202, 94)),
                  ("subtitle", (340, 94, 1202, 175)),
                  ("names", (340, 175, 1202, 345)),
                  ("map", (15, 13, 332, 399)),
                  ("frame", (0, 0, 1215, 13))]:
    x0, y0, x1, y1 = box
    sub = d[y0:y1, x0:x1]
    print("  %-10s mean %.2f  >20 %.1f%%" % (name, sub.mean(), (sub.max(2) > 20).mean() * 100))
