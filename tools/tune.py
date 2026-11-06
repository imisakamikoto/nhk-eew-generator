"""Parameter search: match the dynamic text layers to the reference screenshot."""
import os
import subprocess
import sys

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)
OUT = os.path.join(ROOT, "output", "tune.png")

REGIONS = {
    "sub": (340, 94, 1202, 175),
    "names": (340, 175, 1202, 345),
}


def score(env, region):
    e = os.environ.copy()
    for k, v in env.items():
        e[k] = str(v)
    subprocess.run([BIN, "--demo", "--out", OUT], env=e,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    gen = np.array(Image.open(OUT).convert("RGB")).astype(np.int16)
    x0, y0, x1, y1 = REGIONS[region]
    d = np.abs(REF[y0:y1, x0:x1] - gen[y0:y1, x0:x1])
    return float(d.mean())


def sweep(region, base, grid, topn=8):
    results = []
    for combo in grid:
        env = dict(base)
        env.update(combo)
        results.append((score(env, region), combo))
    results.sort(key=lambda x: x[0])
    print("=== %s best:" % region)
    for s, c in results[:topn]:
        print("   %.3f  %s" % (s, c))
    return results[0][1]


def main():
    print("baseline sub", score({}, "sub"), "names", score({}, "names"))
    # stage 1: font weight, size and horizontal scale
    grid = []
    for font in ("medium", "debold", "bold", "heavy"):
        for size in (53, 54, 55, 56, 57):
            for sx in (0.68, 0.70, 0.72, 0.74, 0.76):
                grid.append(dict(EEW_SUB_FONT=font, EEW_SUB_SIZE=size,
                                 EEW_SUB_SX=sx, EEW_SUB_OUTLINE=5))
    best1 = sweep("sub", {}, grid)
    print("stage1 best", best1)
    # stage 2: position + outline + fine size/scale
    grid2 = []
    f = best1["EEW_SUB_FONT"]
    sz = best1["EEW_SUB_SIZE"]
    sx = best1["EEW_SUB_SX"]
    for size in (sz - 1, sz, sz + 1):
        for s in (sx - 0.02, sx, sx + 0.02):
            for left in (348, 350, 352):
                for top in (101, 103, 105):
                    for ol in (4, 5, 6):
                        grid2.append(dict(EEW_SUB_FONT=f, EEW_SUB_SIZE=size,
                                          EEW_SUB_SX=round(s, 3), EEW_SUB_LEFT=left,
                                          EEW_SUB_TOP=top, EEW_SUB_OUTLINE=ol))
    best2 = sweep("sub", {}, grid2, topn=10)
    print("FINAL SUB", best2, "score", score(best2, "sub"))


if __name__ == "__main__":
    main()
