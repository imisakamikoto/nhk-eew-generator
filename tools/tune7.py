"""Match glyph height/width ratios against the reference card."""
import itertools
import os
import subprocess

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)
OUT = os.path.join(ROOT, "output", "tune7.png")
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


def sweep(grid, crop, label, topn=5):
    res = sorted(((score(c, crop), c) for c in grid), key=lambda x: x[0])
    print("=== %s (%d)" % (label, len(res)), flush=True)
    for s, c in res[:topn]:
        print("   %.3f %s" % (s, c), flush=True)
    return res[0][1]


def names():
    grid = []
    for size, sx, gap, top in itertools.product(
            (68, 69, 70), (0.77, 0.78, 0.79), (46, 48, 50), (187, 188, 189)):
        grid.append(dict(EEW_PREF_FONT="heavy", EEW_PREF_SIZE=size, EEW_PREF_SX=sx,
                         EEW_PREF_GAP=gap, EEW_PREF_OUTLINE=0, EEW_PREF_LEFT=353,
                         EEW_PREF_TOP=top, EEW_PREF_PITCH=76))
    best = sweep(grid, NAMES, "names ratio")
    print("NAMES", best, flush=True)


def sub():
    grid = []
    for font, size, sx, gap, top in itertools.product(
            ("debold", "bold"), (56, 57, 58), (0.70, 0.72), (10, 12, 14), (102, 103, 104)):
        grid.append(dict(EEW_SUB_FONT=font, EEW_SUB_SIZE=size, EEW_SUB_SX=sx,
                         EEW_SUB_GAP=gap, EEW_SUB_OUTLINE=0, EEW_SUB_LEFT=350,
                         EEW_SUB_TOP=top))
    best = sweep(grid, SUB, "sub ratio")
    print("SUB", best, flush=True)


if __name__ == "__main__":
    print("baseline names %.3f sub %.3f" % (score({}, NAMES), score({}, SUB)), flush=True)
    names()
    sub()
