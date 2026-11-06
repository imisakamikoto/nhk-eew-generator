"""Preprocess the JMA GIS shapefiles and epicenter-name GeoJSON into compact runtime binaries.

Output format (little-endian):
  geometry.bin:
    magic "EEWG1", u32 region_count, u32 pref_count
    for each area:
      u16 code, u16 name_len, name bytes (UTF-8)
      4 x f32 bbox (lon_min, lat_min, lon_max, lat_max)
      u32 ring_count
      for each ring: u32 point_count, then f32 lon, f32 lat pairs
      for pref areas only: u16 parent_region_index
  epicenters.bin:
    magic "EEWE1", u32 count
    for each: u16 name_len, name bytes, u32 polygon_count
      for each polygon: u32 ring_count
        for each ring: u32 point_count, then f32 lon, f32 lat pairs
"""
import json
import os
import struct

import shapefile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "data")
os.makedirs(OUT, exist_ok=True)


def rdp(points, eps):
    """Ramer-Douglas-Peucker on a closed ring (keeps first == last)."""
    if len(points) < 4:
        return points
    pts = points[:-1] if points[0] == points[-1] else points
    if len(pts) < 4:
        return points
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    ax, ay = pts[0]
    bx, by = pts[-1]
    dx, dy = bx - ax, by - ay
    den = dx * dx + dy * dy
    while stack:
        s, e = stack.pop()
        maxd, idx = -1.0, -1
        px, py = pts[s]
        qx, qy = pts[e]
        ux, uy = qx - px, qy - py
        un = ux * ux + uy * uy
        for i in range(s + 1, e):
            vx, vy = pts[i][0] - px, pts[i][1] - py
            if un == 0:
                d = (vx * vx + vy * vy) ** 0.5
            else:
                d = abs(ux * vy - uy * vx) / (un ** 0.5)
            if d > maxd:
                maxd, idx = d, i
        if maxd > eps and idx > 0:
            keep[idx] = True
            stack.append((s, idx))
            stack.append((idx, e))
    out = [pts[i] for i in range(len(pts)) if keep[i]]
    if out[0] != out[-1]:
        out.append(out[0])
    return out


def rings_from_shape(shape, eps):
    parts = list(shape.parts) + [len(shape.points)]
    rings = []
    for i in range(len(parts) - 1):
        pts = [(float(p[0]), float(p[1])) for p in shape.points[parts[i]:parts[i + 1]]]
        if len(pts) < 4:
            continue
        r = rdp(pts, eps)
        if len(r) >= 4:
            rings.append(r)
    return rings


def point_in_rings(x, y, rings):
    inside = False
    for ring in rings:
        n = len(ring)
        j = n - 1
        for i in range(n):
            xi, yi = ring[i]
            xj, yj = ring[j]
            if (yi > y) != (yj > y):
                xx = (xj - xi) * (y - yi) / (yj - yi) + xi
                if x < xx:
                    inside = not inside
            j = i
    return inside


def load_shapes(path, eps):
    r = shapefile.Reader(path)
    out = []
    for sr in r.iterShapeRecords():
        rings = rings_from_shape(sr.shape, eps)
        if not rings:
            continue
        allp = [p for ring in rings for p in ring]
        bbox = (min(p[0] for p in allp), min(p[1] for p in allp),
                max(p[0] for p in allp), max(p[1] for p in allp))
        out.append({"code": str(sr.record[0]), "name": str(sr.record[1]),
                    "rings": rings, "bbox": bbox})
    return out


def write_area(buf, a, parent=None):
    name = a["name"].encode("utf-8")
    buf += struct.pack("<HH", int(a["code"]), len(name)) + name
    buf += struct.pack("<4f", *a["bbox"])
    buf += struct.pack("<I", len(a["rings"]))
    for ring in a["rings"]:
        buf += struct.pack("<I", len(ring))
        for x, y in ring:
            buf += struct.pack("<ff", x, y)
    if parent is not None:
        buf += struct.pack("<H", parent)


def main():
    regions = load_shapes(os.path.join(ROOT, "data", "gis", "area.shp"), 0.0025)
    prefs = load_shapes(os.path.join(ROOT, "data", "gis", "local.shp"), 0.0025)
    print("regions", len(regions), [a["name"] for a in regions])
    print("prefs", len(prefs), [a["name"] for a in prefs])

    # Assign each prefecture forecast area to the regional area containing its largest ring centroid.
    parents = []
    for p in prefs:
        ring = max(p["rings"], key=len)
        cx = sum(x for x, _ in ring) / len(ring)
        cy = sum(y for _, y in ring) / len(ring)
        parent = None
        for i, reg in enumerate(regions):
            if point_in_rings(cx, cy, reg["rings"]):
                parent = i
                break
        if parent is None:
            # fall back: bbox center containment / nearest region bbox center
            bx = (p["bbox"][0] + p["bbox"][2]) / 2
            by = (p["bbox"][1] + p["bbox"][3]) / 2
            best = None
            for i, reg in enumerate(regions):
                if point_in_rings(bx, by, reg["rings"]):
                    best = i
                    break
            if best is None:
                best = min(range(len(regions)),
                           key=lambda i: ((regions[i]["bbox"][0] + regions[i]["bbox"][2]) / 2 - bx) ** 2
                                        + ((regions[i]["bbox"][1] + regions[i]["bbox"][3]) / 2 - by) ** 2)
            parent = best
        parents.append(parent)
    for p, i in zip(prefs, parents):
        print("  %-8s -> %s" % (p["name"], regions[i]["name"]))

    buf = bytearray(b"EEWG1") + struct.pack("<II", len(regions), len(prefs))
    for a in regions:
        write_area(buf, a)
    for a, par in zip(prefs, parents):
        write_area(buf, a, par)
    path = os.path.join(OUT, "geometry.bin")
    open(path, "wb").write(buf)
    print("geometry.bin", len(buf), "bytes",
          "rings", sum(len(a["rings"]) for a in regions + prefs),
          "points", sum(len(r) for a in regions + prefs for r in a["rings"]))

    # Epicenter names
    gj = json.load(open(os.path.join(ROOT, "震央地名.geojson"), encoding="utf-8"))
    ebuf = bytearray(b"EEWE1") + struct.pack("<I", len(gj["features"]))
    for f in gj["features"]:
        name = f["properties"]["name"].encode("utf-8")
        ebuf += struct.pack("<H", len(name)) + name
        polys = []
        coords = f["geometry"]["coordinates"]
        if f["geometry"]["type"] == "Polygon":
            coords = [coords]
        for poly in coords:
            rings = []
            for ring in poly:
                pts = [(float(x), float(y)) for x, y in ring]
                if len(pts) >= 4:
                    rings.append(rdp(pts, 0.0005))
            if rings:
                polys.append(rings)
        ebuf += struct.pack("<I", len(polys))
        for rings in polys:
            ebuf += struct.pack("<I", len(rings))
            for ring in rings:
                ebuf += struct.pack("<I", len(ring))
                for x, y in ring:
                    ebuf += struct.pack("<ff", x, y)
    path = os.path.join(OUT, "epicenters.bin")
    open(path, "wb").write(ebuf)
    print("epicenters.bin", len(ebuf), "bytes")
    print("DONE")


if __name__ == "__main__":
    main()
