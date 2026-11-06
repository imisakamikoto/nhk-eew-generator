"""Extended sweep for the yellow sub-headline: taller glyphs + narrower width."""
import itertools
import os
import subprocess

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "eew-generator.exe")
REF = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(np.int16)
OUT = os.path.join(ROOT, "output", "tune_sub.png")
CROP = (slice(96, 172), slice(340, 1060))


def score(env):
    e = os.environ.copy()
    e.update({k: str(v) for k, v in env.items()})
    subprocess.run([BIN, "--demo", "--out", OUT], env=e,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    g = np.array(Image.open(OUT).convert("RGB")).astype(np.int16)
    ys, xs = CROP
    return float(np.abs(REF[ys, xs] - g[ys, xs]).mean())


def main():
    print("baseline %.3f" % score({}), flush=True)
    grid = []
    for font, size, sx, gap, ol in itertools.product(
            ("bold", "debold"), (56, 57, 58), (0.66, 0.68, 0.70), (14, 18, 22), (3, 4, 5)):
        grid.append(dict(EEW_SUB_FONT=font, EEW_SUB_SIZE=size, EEW_SUB_SX=sx,
                         EEW_SUB_GAP=gap, EEW_SUB_OUTLINE=ol,
                         EEW_SUB_LEFT=352, EEW_SUB_TOP=103))
    res = sorted(((score(c), c) for c in grid), key=lambda x: x[0])
    print("=== sub extended (%d runs)" % len(res), flush=True)
    for s, c in res[:8]:
        print("   %.3f %s" % (s, c), flush=True)
    best = res[0][1]
    grid = []
    for left, top in itertools.product((351, 352, 353), (102, 103, 104)):
        grid.append(dict(best, EEW_SUB_LEFT=left, EEW_SUB_TOP=top))
    res = sorted(((score(c), c) for c in grid), key=lambda x: x[0])
    print("=== sub position", flush=True)
    for s, c in res[:5]:
        print("   %.3f %s" % (s, c), flush=True)
    print("FINAL", res[0][1], res[0][0], flush=True)


if __name__ == "__main__":
    main()
