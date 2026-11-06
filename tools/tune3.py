"""Coarse-to-fine search for the white prefecture-name layer."""
import os
import subprocess

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)[175:345, 340:1202]
OUT = os.path.join(ROOT, "output", "tune.png")


def score(env):
    e = os.environ.copy()
    e.update({k: str(v) for k, v in env.items()})
    subprocess.run([BIN, "--demo", "--out", OUT], env=e,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    g = np.array(Image.open(OUT).convert("RGB")).astype(np.int16)[175:345, 340:1202]
    return float(np.abs(REF - g).mean())


def sweep(grid, label, topn=6):
    res = [(score(c), c) for c in grid]
    res.sort(key=lambda x: x[0])
    print("=== %s (%d runs)" % (label, len(res)))
    for s, c in res[:topn]:
        print("   %.3f %s" % (s, c))
    return res[0][1]


def main():
    print("baseline", score({}))
    g1 = []
    for size in (62, 64, 66):
        for sx in (0.82, 0.845, 0.87):
            for gap in (50, 53, 56):
                for ol in (3, 4, 5):
                    g1.append(dict(EEW_PREF_SIZE=size, EEW_PREF_SX=sx, EEW_PREF_GAP=gap,
                                   EEW_PREF_OUTLINE=ol, EEW_PREF_LEFT=353,
                                   EEW_PREF_TOP=189, EEW_PREF_PITCH=76))
    b1 = sweep(g1, "stage1")
    g2 = []
    for left in (351, 353, 355):
        for top in (187, 189, 191):
            for pitch in (74, 76, 78):
                c = dict(b1); c.update(EEW_PREF_LEFT=left, EEW_PREF_TOP=top, EEW_PREF_PITCH=pitch)
                g2.append(c)
    b2 = sweep(g2, "stage2")
    g3 = []
    for sx in (b1["EEW_PREF_SX"] - 0.015, b1["EEW_PREF_SX"], b1["EEW_PREF_SX"] + 0.015):
        for gap in (b1["EEW_PREF_GAP"] - 2, b1["EEW_PREF_GAP"], b1["EEW_PREF_GAP"] + 2):
            for size in (b1["EEW_PREF_SIZE"] - 1, b1["EEW_PREF_SIZE"], b1["EEW_PREF_SIZE"] + 1):
                c = dict(b2); c.update(EEW_PREF_SX=round(sx, 4), EEW_PREF_GAP=gap, EEW_PREF_SIZE=size)
                g3.append(c)
    b3 = sweep(g3, "stage3", topn=8)
    print("FINAL", b3, "score", score(b3))


if __name__ == "__main__":
    main()
