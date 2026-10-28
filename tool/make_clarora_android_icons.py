#!/usr/bin/env python3
"""Generate the Clarora Android and macOS launcher icon sets.

Design matches the app's aurora-dark theme with a mint C monogram.
Run with --platform macos to update only the desktop SVG and PNG assets.

Outputs (under clarora-app/android/app/src/main/res):
- mipmap-*/ic_launcher.png          legacy square icon, rounded corners
- mipmap-*/ic_launcher_round.png    legacy circular icon
- mipmap-*/ic_launcher_foreground.png  adaptive-icon foreground (transparent)
"""

from __future__ import annotations

from pathlib import Path
import argparse
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

# macOS does not mask custom icons: the artwork needs its own rounded tile
# and transparent margin. Share vector geometry with the SVG design source,
# rather than depending on whichever system font happens to be installed.
MAC_TILE = (100, 100, 924, 924)
MAC_RADIUS = 184
MAC_BG_TOP = (25, 56, 45)
MAC_BG_BOTTOM = (13, 35, 28)
MAC_C_TOP = (113, 232, 181)
MAC_C_BOTTOM = (55, 210, 151)
MAC_C_PATH = [
    ("M", (790, 280)),
    ("L", (690, 360)),
    ("C", (645, 305, 597, 282, 535, 282)),
    ("C", (407, 282, 330, 372, 330, 512)),
    ("C", (330, 652, 407, 742, 535, 742)),
    ("C", (597, 742, 645, 719, 690, 664)),
    ("L", (790, 744)),
    ("C", (722, 831, 641, 874, 531, 874)),
    ("C", (328, 874, 197, 735, 197, 512)),
    ("C", (197, 289, 328, 150, 531, 150)),
    ("C", (641, 150, 722, 193, 790, 280)),
]

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
    """Draw the C glyph with a vertical #52E0A2→#34D399 gradient inside box."""
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


def macos_monogram() -> list[tuple[str, tuple[float, ...]]]:
    # Slight optical offset balances the open side of C within the tile.
    return [(command, tuple((value - 512) * 0.74 + (530 if i % 2 == 0 else 512)
                            for i, value in enumerate(points)))
            for command, points in MAC_C_PATH]


def make_macos_icon() -> Image.Image:
    scale = FOREGROUND_SCALE
    canvas = 1024 * scale
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    tile = tuple(value * scale for value in MAC_TILE)
    mask = Image.new("L", image.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle(tile, radius=MAC_RADIUS * scale, fill=255)
    background = Image.new("RGBA", image.size, (0, 0, 0, 0))
    side = tile[2] - tile[0]
    background.paste(vertical_gradient((side, side), MAC_BG_TOP, MAC_BG_BOTTOM), tile[:2])
    image.paste(background, (0, 0), mask)

    # A quiet rim gives the dark tile definition against a dark Dock.
    rim = Image.new("RGBA", image.size, (0, 0, 0, 0))
    ImageDraw.Draw(rim).rounded_rectangle(
        tuple(value * scale for value in (101, 101, 923, 923)),
        radius=(MAC_RADIUS - 1) * scale, outline=(134, 182, 157, 56), width=2 * scale)
    image = Image.alpha_composite(image, rim)

    outline = ImageDraw.Outline()
    for command, points in macos_monogram():
        getattr(outline, {"M": "move", "L": "line", "C": "curve"}[command])(
            *(value * scale for value in points))
    outline.close()
    glyph = Image.new("L", image.size, 0)
    ImageDraw.Draw(glyph).shape(outline, fill=255)
    top, bottom = round((150 - 512) * 0.74 + 512), round((874 - 512) * 0.74 + 512)
    accent = Image.new("RGBA", image.size, (0, 0, 0, 0))
    accent.paste(vertical_gradient((canvas, (bottom - top) * scale), MAC_C_TOP, MAC_C_BOTTOM), (0, top * scale))
    image.paste(accent, (0, 0), glyph)
    return image


def write_macos_icons() -> None:
    macos = Path(__file__).resolve().parent.parent / "clarora-app/macos/Clarora-macOS"
    color = lambda rgb: "#" + "".join(f"{value:02X}" for value in rgb)
    path = " ".join(command + " ".join(f"{value:g}" for value in points)
                    for command, points in macos_monogram()) + " Z"
    (macos / "app-icon.svg").write_text(f'''<svg width="1024" height="1024" viewBox="0 0 1024 1024" fill="none" xmlns="http://www.w3.org/2000/svg">
  <!-- Generated by tool/make_clarora_android_icons.py (macos platform). -->
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1" gradientUnits="objectBoundingBox">
      <stop stop-color="{color(MAC_BG_TOP)}"/><stop offset="1" stop-color="{color(MAC_BG_BOTTOM)}"/>
    </linearGradient>
    <linearGradient id="accent" x1="0" y1="0" x2="0" y2="1" gradientUnits="objectBoundingBox">
      <stop stop-color="{color(MAC_C_TOP)}"/><stop offset="1" stop-color="{color(MAC_C_BOTTOM)}"/>
    </linearGradient>
  </defs>
  <!-- Transparent margin and rounded artwork are part of the icon itself. -->
  <rect x="100" y="100" width="824" height="824" rx="184" fill="url(#bg)"/>
  <rect x="101" y="101" width="822" height="822" rx="183" stroke="#86B69D" stroke-opacity="0.22" stroke-width="2"/>
  <path d="{path}" fill="url(#accent)"/>
</svg>
''', encoding="utf-8")
    app_icons = macos / "Assets.xcassets/AppIcon.appiconset"
    master = make_macos_icon()
    for item in json.loads((app_icons / "Contents.json").read_text())["images"]:
        size = int(item["size"].split("x")[0]) * int(item["scale"].rstrip("x"))
        master.resize((size, size), Image.LANCZOS).save(app_icons / item["filename"])
    print("macOS rounded Clarora SVG + icons written")


def write_android_icons() -> None:
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("all", "android", "macos"), default="all")
    args = parser.parse_args()
    if args.platform in ("all", "android"):
        write_android_icons()
    if args.platform in ("all", "macos"):
        write_macos_icons()


if __name__ == "__main__":
    main()
