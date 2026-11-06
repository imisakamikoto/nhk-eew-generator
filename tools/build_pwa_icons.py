"""生成 PWA 图标：深蓝底 + 黄色 EEW + 红色横线 + 日文副标题。

输出到 assets/web/，由 src/server.rs 用 include_bytes! 内嵌。
"""

import os
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONT = os.path.join(ROOT, "assets", "fonts", "udshingo_heavy.otf")
OUT = os.path.join(ROOT, "assets", "web")

BG_TOP = (10, 17, 30)
BG_BOT = (26, 40, 66)
YELLOW = (247, 252, 95)
RED = (214, 40, 74)
WHITE = (245, 249, 255)


def gradient(size):
    im = Image.new("RGB", (1, size))
    px = im.load()
    for y in range(size):
        t = y / max(1, size - 1)
        px[0, y] = tuple(int(BG_TOP[i] + (BG_BOT[i] - BG_TOP[i]) * t) for i in range(3))
    return im.resize((size, size), Image.Resampling.BILINEAR)


def draw_icon(size, maskable=False, radius_ratio=0.22):
    im = gradient(size).convert("RGBA")
    d = ImageDraw.Draw(im)

    # 内容区（maskable 需留 20% 安全边距）
    scale = 0.78 if maskable else 1.0
    content = size * scale
    off = (size - content) / 2

    # 边框
    if not maskable:
        inset = size * 0.045
        w = max(2, int(size * 0.022))
        d.rounded_rectangle(
            [inset, inset, size - inset - 1, size - inset - 1],
            radius=int(size * radius_ratio), outline=YELLOW, width=w,
        )

    # 主字 EEW
    txt = "EEW"
    fsize = int(content * 0.36)
    font = ImageFont.truetype(FONT, fsize)
    while d.textlength(txt, font=font) > content * 0.78 and fsize > 8:
        fsize -= max(1, fsize // 40)
        font = ImageFont.truetype(FONT, fsize)
    cx = off + content / 2
    d.text((cx, off + content * 0.37), txt, font=font, fill=YELLOW, anchor="mm")

    # 红色横线
    bar_w = content * 0.54
    bar_h = max(2, content * 0.032)
    bar_y = off + content * 0.585
    d.rounded_rectangle(
        [cx - bar_w / 2, bar_y, cx + bar_w / 2, bar_y + bar_h],
        radius=bar_h / 2, fill=RED,
    )

    # 日文副标题
    sub = "緊急地震速報"
    ssize = int(content * 0.105)
    sfont = ImageFont.truetype(FONT, ssize)
    while d.textlength(sub, font=sfont) > content * 0.78 and ssize > 6:
        ssize -= 1
        sfont = ImageFont.truetype(FONT, ssize)
    d.text((cx, off + content * 0.70), sub, font=sfont, fill=WHITE, anchor="mm")

    return im


def main():
    os.makedirs(OUT, exist_ok=True)
    jobs = [
        ("icon-192.png", 192, False),
        ("icon-512.png", 512, False),
        ("icon-maskable-512.png", 512, True),
        ("apple-touch-icon.png", 180, False),
        ("favicon.png", 64, False),
    ]
    for name, size, maskable in jobs:
        im = draw_icon(size, maskable)
        im.convert("RGB").save(os.path.join(OUT, name), optimize=True)
        print("saved", name, size)


if __name__ == "__main__":
    main()
