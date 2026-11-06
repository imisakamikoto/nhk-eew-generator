"""Final typography sweep with outline disabled (the reference has no stroke)."""
import itertools
import os
import subprocess

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)
OUT = os.path.join(ROOT, "output", "tune6.png")
NAMES = (slice(178, 345), slice(340, 1202))
SUB = (slice(96, 175), slice(340, 1060))


def score(env, crop):
    e = os.environ.copy()
    e.update({k: str(v) for k, v in env.items()})
    subprocess.run([BIN, "--demo", "--out", OUT], env=e,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    g = np.array(Image.open(OUT).convert("RGB")).astype(np.int16)
    ys, xs = crop
    return float(np.abs(REF[ys, xs] - g[ys, xs]).mean())


def sweep(grid, crop, label, topn=6):
    res = sorted(((score(c, crop), c) for c in grid), key=lambda x: x[0])
    print("=== %s (%d)" % (label, len(res)), flush=True)
    for s, c in res[:topn]:
        print("   %.3f %s" % (s, c), flush=True)
    return res[0]


def names():
    grid = []
    for font, size, sx, gap in itertools.product(
            ("heavy", "debold"), (64, 66, 68, 70), (0.80, 0.82, 0.84), (44, 48, 52, 56)):
        grid.append(dict(EEW_PREF_FONT=font, EEW_PREF_SIZE=size, EEW_PREF_SX=sx,
                         EEW_PREF_GAP=gap, EEW_PREF_OUTLINE=0, EEW_PREF_LEFT=353,
                         EEW_PREF_TOP=189, EEW_PREF_PITCH=76))
    _, best = sweep(grid, NAMES, "names face")
    grid = [dict(best, EEW_PREF_LEFT=l, EEW_PREF_TOP=t, EEW_PREF_PITCH=p)
            for l, t, p in itertools.product((351, 353, 355), (187, 189, 191), (74, 76, 78))]
    _, best = sweep(grid, NAMES, "names position")
    print("NAMES", best, flush=True)


def sub():
    grid = []
    for font, size, sx, gap in itertools.product(
            ("bold", "debold", "medium"), (54, 56, 58), (0.68, 0.70, 0.72), (12, 16, 20)):
        grid.append(dict(EEW_SUB_FONT=font, EEW_SUB_SIZE=size, EEW_SUB_SX=sx,
                         EEW_SUB_GAP=gap, EEW_SUB_OUTLINE=0, EEW_SUB_LEFT=352,
                         EEW_SUB_TOP=103))
    _, best = sweep(grid, SUB, "sub face")
    grid = [dict(best, EEW_SUB_LEFT=l, EEW_SUB_TOP=t)
            for l, t in itertools.product((350, 352, 354), (101, 103, 105))]
    _, best = sweep(grid, SUB, "sub position")
    print("SUB", best, flush=True)


if __name__ == "__main__":
    print("baseline names %.3f sub %.3f" % (score({}, NAMES), score({}, SUB)), flush=True)
    names()
    sub()
