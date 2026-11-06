"""Extract fonts and GIS shapefiles from the supplied ZIPs into ASCII-named files."""
import os
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONTS = os.path.join(ROOT, "assets", "fonts")
GIS = os.path.join(ROOT, "data", "gis")
os.makedirs(FONTS, exist_ok=True)
os.makedirs(GIS, exist_ok=True)


def fix(name):
    try:
        return name.encode("cp437").decode("utf-8")
    except Exception:
        return name


FONT_MAP = {
    "DeBold": "udshingo_debold.otf",
    "Light": "udshingo_light.otf",
    "Regular": "udshingo_regular.otf",
    "Medium": "udshingo_medium.otf",
    "Bold": "udshingo_bold.otf",
    "Heavy": "udshingo_heavy.otf",
}

with zipfile.ZipFile(os.path.join(ROOT, "NHK字体.zip")) as z:
    for info in z.infolist():
        name = fix(info.filename)
        if name.startswith("__MACOSX") or not name.lower().endswith(".otf"):
            continue
        if "简体中文字形" in name:
            continue  # Japanese glyphs only (the UI is Japanese)
        for key, out in FONT_MAP.items():
            if key + ".otf" in name:
                path = os.path.join(FONTS, out)
                with open(path, "wb") as f:
                    f.write(z.read(info))
                print("font ->", out, len(open(path, "rb").read()))
                break

GIS_MAP = {
    "20190125_AreaForecastEEW_GIS.zip": ("area", ["shp", "shx", "dbf"]),
    "20190125_AreaForecastLocalEEW_GIS.zip": ("local", ["shp", "shx", "dbf"]),
}
for zipname, (prefix, exts) in GIS_MAP.items():
    with zipfile.ZipFile(os.path.join(ROOT, zipname)) as z:
        for info in z.infolist():
            name = fix(info.filename)
            ext = name.rsplit(".", 1)[-1].lower()
            if ext in exts:
                path = os.path.join(GIS, "%s.%s" % (prefix, ext))
                with z.open(info) as src, open(path, "wb") as dst:
                    while True:
                        chunk = src.read(1 << 20)
                        if not chunk:
                            break
                        dst.write(chunk)
                print("gis ->", path, os.path.getsize(path))
print("DONE")
