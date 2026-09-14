#!/usr/bin/env python3
"""Generate the Clarora Android and macOS launcher icon sets.

Design matches the app's aurora-dark theme (and the macOS app icon):
vertical gradient background #0D1512 → #1D2F26 with an "C" monogram in
#52E0A2 → #34D399.

Outputs (under clarora-app/android/app/src/main/res):
- mipmap-*/ic_launcher.png          legacy square icon, rounded corners
- mipmap-*/ic_launcher_round.png    legacy circular icon
- mipmap-*/ic_launcher_foreground.png  adaptive-icon foreground (transparent)
"""

from __future__ import annotations

from pathlib import Path
import json

from PIL import Image, ImageDraw, ImageFont

RES = Path(__file__).resolve().parent.parent / "clarora-app/android/app/src/main/res"

BG_TOP = (13, 21, 18)      # 0D1512
BG_BOTTOM = (29, 47, 38)   # 1D2F26
R_TOP = (82, 224, 162)     # 52E0A2
R_BOTTOM = (52, 211, 153)  # 34D399

SIZES = {  # density → launcher icon size (48dp grid)
    "mdpi": 48,
    "hdpi": 72,
    "xhdpi": 96,
    "xxhdpi": 144,
    "xxxhdpi": 192,
}
# Adaptive foreground uses a 108dp canvas; the glyph must stay in the
# central ~66dp safe zone so launcher masks never clip it.
FOREGROUND_SCALE = 4  # render 4× then downsample for crisp edges

FONT_CANDIDATES = [
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/System/Library/Fonts/SFNS.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "/Library/Fonts/Arial Bold.ttf",
]


def load_font(size: int) -> ImageFont.FreeTypeFont:
    for path in FONT_CANDIDATES:
        try:
            font = ImageFont.truetype(path, size)
            if font.getbbox("C")[2] > 0:
                return font
        except OSError:
            continue
    raise SystemExit("no usable system font found for the C monogram")


def vertical_gradient(size: tuple[int, int], top: tuple, bottom: tuple) -> Image.Image:
    width, height = size
    gradient = Image.new("RGB", (1, height))
    for y in range(height):
        ratio = y / max(height - 1, 1)
        color = tuple(round(top[i] + (bottom[i] - top[i]) * ratio) for i in range(3))
        gradient.putpixel((0, y), color)
    return gradient.resize((width, height))


def draw_monogram(canvas: Image.Image, box: tuple[int, int, int, int]) -> None:
    """Draw the R glyph with a vertical #52E0A2→#34D399 gradient inside box."""
    left, top, right, bottom = box
    width, height = right - left, bottom - top
    # Start from a large render then fit the glyph bbox precisely.
    render = Image.new("L", (width * 3, height * 3), 0)
    draw = ImageDraw.Draw(render)
    font = load_font(int(height * 3 * 0.9))
    draw.text((render.width // 2, render.height // 2), "C", font=font, anchor="mm", fill=255)
    glyph_bbox = render.getbbox()
    glyph = render.crop(glyph_bbox)
    glyph = glyph.resize((width, height))
    gradient = vertical_gradient((width, height), R_TOP, R_BOTTOM)
    canvas.paste(gradient, (left, top), glyph)


def make_launcher(size: int, round_shape: bool) -> Image.Image:
    scale = FOREGROUND_SCALE
    image = Image.new("RGBA", (size * scale, size * scale), (0, 0, 0, 0))
    background = vertical_gradient(image.size, BG_TOP, BG_BOTTOM).convert("RGBA")
    mask = Image.new("L", image.size, 0)
    draw = ImageDraw.Draw(mask)
    if round_shape:
        draw.ellipse((0, 0, image.width - 1, image.height - 1), fill=255)
    else:
        radius = int(image.width * 0.18)
        draw.rounded_rectangle((0, 0, image.width - 1, image.height - 1), radius=radius, fill=255)
    image.paste(background, (0, 0), mask)
    inset = int(image.width * 0.16)
    draw_monogram(image, (inset, inset, image.width - inset, image.height - inset))
    return image.resize((size, size), Image.LANCZOS)


def make_foreground(size: int) -> Image.Image:
    """Adaptive foreground: transparent canvas, glyph inside the safe zone."""
    canvas = size * FOREGROUND_SCALE
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    safe = int(canvas * 0.58)  # ≤ 66/108 visible zone, keep margin
    left = (canvas - safe) // 2
    draw_monogram(image, (left, left, left + safe, left + safe))
    return image.resize((size, size), Image.LANCZOS)


def main() -> None:
    for density, size in SIZES.items():
        directory = RES / f"mipmap-{density}"
        directory.mkdir(parents=True, exist_ok=True)
        make_launcher(size, round_shape=False).save(directory / "ic_launcher.png")
        make_launcher(size, round_shape=True).save(directory / "ic_launcher_round.png")
        make_foreground(round(size * 108 / 48)).save(directory / "ic_launcher_foreground.png")
        print(f"mipmap-{density}: {size}px launcher + {round(size * 108 / 48)}px foreground")

    anydpi = RES / "mipmap-anydpi-v26"
    anydpi.mkdir(parents=True, exist_ok=True)
    adaptive = """<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
</adaptive-icon>
"""
    (anydpi / "ic_launcher.xml").write_text(adaptive, encoding="utf-8")
    (anydpi / "ic_launcher_round.xml").write_text(adaptive, encoding="utf-8")

    drawable = RES / "drawable"
    drawable.mkdir(parents=True, exist_ok=True)
    (drawable / "ic_launcher_background.xml").write_text(
        """<?xml version="1.0" encoding="utf-8"?>
<shape xmlns:android="http://schemas.android.com/apk/res/android" android:shape="rectangle">
    <gradient android:angle="270" android:startColor="#0D1512" android:endColor="#1D2F26" />
</shape>
""",
        encoding="utf-8",
    )
    print("adaptive icon xml + gradient background written")

    # Keep the desktop launcher assets on the same Clarora monogram.
    app_icons = Path(__file__).resolve().parent.parent / "clarora-app/macos/Clarora-macOS/Assets.xcassets/AppIcon.appiconset"
    for item in json.loads((app_icons / "Contents.json").read_text())["images"]:
        size = int(item["size"].split("x")[0]) * int(item["scale"].rstrip("x"))
        icon = vertical_gradient((size * 4, size * 4), BG_TOP, BG_BOTTOM).convert("RGBA")
        inset = int(icon.width * 0.16)
        draw_monogram(icon, (inset, inset, icon.width - inset, icon.height - inset))
        icon.resize((size, size), Image.LANCZOS).save(app_icons / item["filename"])
    print("macOS Clarora icons written")


if __name__ == "__main__":
    main()
