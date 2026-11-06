"""Recover the map projection (affine lon/lat -> pixel) of the reference screenshot."""
import json
import os
import struct

import numpy as np
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def load_geometry(path):
    d = open(path, "rb").read()
    assert d[:5] == b"EEWG1"
    nreg, npref = struct.unpack_from("<II", d, 5)
    p = 13
    areas = []

    def read_area(with_parent):
        nonlocal p
        code, nl = struct.unpack_from("<HH", d, p); p += 4
        name = d[p:p + nl].decode("utf-8"); p += nl
        bbox = struct.unpack_from("<4f", d, p); p += 16
        (nr,) = struct.unpack_from("<I", d, p); p += 4
        rings = []
        for _ in range(nr):
            (n,) = struct.unpack_from("<I", d, p); p += 4
            pts = list(struct.unpack_from("<%df" % (2 * n), d, p)); p += 8 * n
            rings.append(list(zip(pts[0::2], pts[1::2])))
        parent = None
        if with_parent:
            (parent,) = struct.unpack_from("<H", d, p); p += 2
        return dict(code=code, name=name, bbox=bbox, rings=rings, parent=parent)

    for _ in range(nreg):
        areas.append(read_area(False))
    for _ in range(npref):
        areas.append(read_area(True))
    return areas, nreg, npref


def main():
    areas, nreg, npref = load_geometry(os.path.join(ROOT, "data", "geometry.bin"))
    prefs = areas[nreg:]
    ref = np.array(Image.open(os.path.join(ROOT, "NHK地震速报屏模板.png")).convert("RGB")).astype(int)
    MAP = (15, 13, 332, 399)  # x0,y0,x1,y1 within the 1215x411 card
    x0, y0, x1, y1 = MAP
    sub = ref[y0:y1, x0:x1]
    r, g, b = sub[:, :, 0], sub[:, :, 1], sub[:, :, 2]
    land = (r > 100) & (np.abs(r - g) < 28)
    # remove the epicenter X marker (upper-left) and the white frame
    land[:190, :115] = False
    land[:5, :] = False
    land[-8:, :] = False
    land[:, :3] = False
    land[:, -6:] = False
    target = land
    print("target land pixels", target.sum(), "of", target.size)

    def render(lon0, lat0, sx, sy):
        im = Image.new("L", (x1 - x0, y1 - y0), 0)
        dr = ImageDraw.Draw(im)
        for a in prefs:
            for ring in a["rings"]:
                pts = [((x - lon0) * sx, (lat0 - y) * sy) for x, y in ring]
                # coarse viewport cull
                if max(px for px, _ in pts) < -20 or min(px for px, _ in pts) > x1 - x0 + 20:
                    continue
                if max(py for _, py in pts) < -20 or min(py for _, py in pts) > y1 - y0 + 20:
                    continue
                dr.polygon(pts, fill=255)
        return np.array(im) > 127

    def score(lon0, lat0, sx, sy):
        m = render(lon0, lat0, sx, sy)
        inter = np.logical_and(m, target).sum()
        union = np.logical_or(m, target).sum()
        return inter / max(1, union)

    best = (0, None)
    # coarse grid
    for lon0 in np.arange(127.4, 129.4, 0.2):
        for lat0 in np.arange(35.4, 37.0, 0.2):
            for sx in np.arange(48, 62, 2.0):
                sy = sx / np.cos(np.radians(33.0))
                s = score(lon0, lat0, sx, sy)
                if s > best[0]:
                    best = (s, (lon0, lat0, sx, sy))
    print("coarse best", best)
    # local refinement
    p = list(best[1])
    step = [0.1, 0.1, 1.0, 1.0]
    for _ in range(4):
        improved = False
        for i in range(4):
            for delta in (-step[i], step[i]):
                q = p.copy(); q[i] += delta
                s = score(*q)
                if s > best[0]:
                    best = (s, tuple(q)); p = q; improved = True
        for i in range(4):
            step[i] *= 0.5
        if not improved:
            break
    print("refined best", best)
    lon0, lat0, sx, sy = best[1]
    im = Image.fromarray((render(lon0, lat0, sx, sy) * 255).astype(np.uint8))
    im.save(os.path.join(ROOT, "analysis", "fit_render.png"))
    Image.fromarray((target * 255).astype(np.uint8)).save(os.path.join(ROOT, "analysis", "fit_target.png"))
    # epicenter reference
    gj = json.load(open(os.path.join(ROOT, "震央地名.geojson"), encoding="utf-8"))
    for f in gj["features"]:
        if f["properties"]["name"] == "福岡県北西沖":
            pts = [p for poly in f["geometry"]["coordinates"] for ring in poly for p in ring]
            cx = sum(x for x, _ in pts) / len(pts)
            cy = sum(y for _, y in pts) / len(pts)
            print("epicenter centroid", cx, cy, "-> pixel", x0 + (cx - lon0) * sx, y0 + (lat0 - cy) * sy)
            break
    red = (ref[:, :, 0] > 200) & (ref[:, :, 1] < 90) & (ref[:, :, 2] < 140)
    red[:90, :] = False  # ignore pink title bar
    ys, xs = np.where(red)
    print("X red bbox", xs.min(), xs.max(), ys.min(), ys.max(), "center", (xs.min() + xs.max()) / 2, (ys.min() + ys.max()) / 2)


if __name__ == "__main__":
    main()
