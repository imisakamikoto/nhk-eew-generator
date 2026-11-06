"""Coarse-to-fine search for the split subtitle layout."""
import os
import subprocess

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)[94:175, 340:1202]
OUT = os.path.join(ROOT, "output", "tune.png")


def score(env):
    e = os.environ.copy()
    e.update({k: str(v) for k, v in env.items()})
    subprocess.run([BIN, "--demo", "--out", OUT], env=e,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    g = np.array(Image.open(OUT).convert("RGB")).astype(np.int16)[94:175, 340:1202]
    return float(np.abs(REF - g).mean())


def sweep(grid, label, topn=6):
    res = []
    for c in grid:
        res.append((score(c), c))
    res.sort(key=lambda x: x[0])
    print("=== %s (best %d)" % (label, len(res)))
    for s, c in res[:topn]:
        print("   %.3f %s" % (s, c))
    return res[0][1]


def main():
    print("baseline", score({}))
    g1 = []
    for font in ("medium", "debold", "bold"):
        for size in (53, 54, 55, 56):
            for sx in (0.70, 0.72, 0.74, 0.76, 0.78):
                for gap in (14, 18, 22):
                    g1.append(dict(EEW_SUB_FONT=font, EEW_SUB_SIZE=size,
                                   EEW_SUB_SX=sx, EEW_SUB_GAP=gap,
                                   EEW_SUB_LEFT=352, EEW_SUB_TOP=103, EEW_SUB_OUTLINE=5))
    b1 = sweep(g1, "stage1")
    g2 = []
    for left in (350, 352, 354):
        for top in (101, 103, 105):
            for ol in (4, 5, 6):
                c = dict(b1); c.update(EEW_SUB_LEFT=left, EEW_SUB_TOP=top, EEW_SUB_OUTLINE=ol)
                g2.append(c)
    b2 = sweep(g2, "stage2")
    g3 = []
    for sx in (b1["EEW_SUB_SX"] - 0.02, b1["EEW_SUB_SX"], b1["EEW_SUB_SX"] + 0.02):
        for gap in (b1["EEW_SUB_GAP"] - 4, b1["EEW_SUB_GAP"], b1["EEW_SUB_GAP"] + 4):
            for size in (b1["EEW_SUB_SIZE"] - 1, b1["EEW_SUB_SIZE"], b1["EEW_SUB_SIZE"] + 1):
                c = dict(b2); c.update(EEW_SUB_SX=round(sx, 3), EEW_SUB_GAP=gap, EEW_SUB_SIZE=size)
                g3.append(c)
    b3 = sweep(g3, "stage3", topn=8)
    print("FINAL", b3, "score", score(b3))


if __name__ == "__main__":
    main()
