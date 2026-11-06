"""Independent sweeps for the two dynamic text layers.

Both layers are scored on their own crop, so the searches do not interact:
  * prefecture names   y 175..345
  * yellow sub-headline y  94..175
"""
import itertools
import os
import subprocess

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)
OUT = os.path.join(ROOT, "output", "tune.png")

NAMES = (slice(175, 345), slice(340, 1202))
SUB = (slice(94, 175), slice(340, 1060))


def score(env, crop):
    e = os.environ.copy()
    e.update({k: str(v) for k, v in env.items()})
    subprocess.run([BIN, "--demo", "--out", OUT], env=e,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    g = np.array(Image.open(OUT).convert("RGB")).astype(np.int16)
    ys, xs = crop
    return float(np.abs(REF[ys, xs] - g[ys, xs]).mean())


def sweep(grid, crop, label, topn=6):
    res = [(score(c, crop), c) for c in grid]
    res.sort(key=lambda x: x[0])
    print("=== %s (%d runs)" % (label, len(res)), flush=True)
    for s, c in res[:topn]:
        print("   %.3f %s" % (s, c), flush=True)
    return res[0]


def names_stage():
    base = dict(EEW_PREF_LEFT=353, EEW_PREF_TOP=189, EEW_PREF_PITCH=76)
    grid = []
    for font, size, sx, gap, ol in itertools.product(
            ("heavy", "debold", "bold"), (63, 65, 67), (0.80, 0.82, 0.84),
            (50, 54, 58), (3, 4)):
        c = dict(base, EEW_PREF_FONT=font, EEW_PREF_SIZE=size, EEW_PREF_SX=sx,
                 EEW_PREF_GAP=gap, EEW_PREF_OUTLINE=ol)
        grid.append(c)
    best_score, best = sweep(grid, NAMES, "names/face")
    grid = []
    for left, top, pitch in itertools.product((351, 353, 355), (187, 189, 191), (74, 76, 78)):
        c = dict(best, EEW_PREF_LEFT=left, EEW_PREF_TOP=top, EEW_PREF_PITCH=pitch)
        grid.append(c)
    _, best = sweep(grid, NAMES, "names/position")
    print("NAMES", best, flush=True)
    return best


def sub_stage():
    base = dict(EEW_SUB_LEFT=352, EEW_SUB_TOP=103)
    grid = []
    for font, size, sx, gap, ol in itertools.product(
            ("bold", "debold", "heavy"), (54, 55, 56), (0.70, 0.72, 0.74),
            (14, 18, 22), (4, 5, 6)):
        c = dict(base, EEW_SUB_FONT=font, EEW_SUB_SIZE=size, EEW_SUB_SX=sx,
                 EEW_SUB_GAP=gap, EEW_SUB_OUTLINE=ol)
        grid.append(c)
    _, best = sweep(grid, SUB, "sub/face")
    grid = []
    for left, top in itertools.product((350, 352, 354), (101, 103, 105)):
        grid.append(dict(best, EEW_SUB_LEFT=left, EEW_SUB_TOP=top))
    _, best = sweep(grid, SUB, "sub/position")
    print("SUB", best, flush=True)
    return best


if __name__ == "__main__":
    print("baseline names %.3f  sub %.3f" % (score({}, NAMES), score({}, SUB)), flush=True)
    names_stage()
    sub_stage()
