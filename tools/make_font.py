# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow>=10", "numpy>=1.26", "scipy>=1.11", "fonttools>=4.40"]
# ///
"""Bakes the game's two fonts into one signed-distance-field atlas.

    uv run tools/make_font.py [--title FONT.ttf] [--body FONT.ttf]

Defaults (GGBotNet, CC0, from S:/asset-art/fonts): FreeCheese for titles and
banners, Jupiteroid for everything else.

Writes crates/stickfell_render/assets/font-sdf.png (one channel: 0.5 is the
glyph edge, more is inside) and crates/stickfell_render/src/font_data.rs
(advances, boxes and atlas rects). Sizes are in cap heights, so text drawn
with `scale` has capitals `7 * scale` px tall, like the old pixel font.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont
from scipy.ndimage import distance_transform_edt

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_PNG = os.path.join(HERE, "..", "crates", "stickfell_render", "assets", "font-sdf.png")
OUT_RS = os.path.join(HERE, "..", "crates", "stickfell_render", "src", "font_data.rs")
DEFAULTS = {"title": "FreeCheese-Regular.ttf", "body": "Jupiteroid-Regular.ttf"}

HI = 512  # render size (px per em) for the high-res glyph
DOWN = 8  # atlas = HI / DOWN px per em
SPREAD = 6  # atlas px of distance on each side of the edge
ATLAS_W = 1024
CHARS = [chr(c) for c in range(33, 127)]


def find_ttf(path):
    if os.path.exists(path):
        return path
    for root, _, files in os.walk("S:/asset-art/fonts"):
        if os.path.basename(path) in files:
            return os.path.join(root, os.path.basename(path))
    sys.exit(f"font not found: {path}")


def cap_height(font):
    _, top, _, bottom = font.getbbox("H", anchor="ls")
    return bottom - top


def sdf_for(font, ch):
    """Signed distance (atlas px) for one glyph, plus its box in hi-res px from the pen at the baseline."""
    x0, y0, x1, y1 = font.getbbox(ch, anchor="ls")
    if x1 <= x0 or y1 <= y0:
        return None, None
    pad = SPREAD * DOWN + DOWN
    # snap the padded box to whole atlas pixels so every sample lines up
    bx0 = (x0 - pad) // DOWN * DOWN
    by0 = (y0 - pad) // DOWN * DOWN
    bx1 = -(-(x1 + pad) // DOWN) * DOWN
    by1 = -(-(y1 + pad) // DOWN) * DOWN
    img = Image.new("L", (bx1 - bx0, by1 - by0), 0)
    d = ImageDraw.Draw(img)
    d.fontmode = "1"
    d.text((-bx0, -by0), ch, font=font, fill=255, anchor="ls")
    inside = np.array(img) > 127
    dist = distance_transform_edt(~inside) - distance_transform_edt(inside)
    # average each DOWN x DOWN block -> distance at the atlas pixel's centre
    h, w = dist.shape
    small = dist.reshape(h // DOWN, DOWN, w // DOWN, DOWN).mean(axis=(1, 3)) / DOWN
    value = np.clip(0.5 - small / (2 * SPREAD), 0.0, 1.0)
    return (value * 255 + 0.5).astype(np.uint8), (bx0, by0, bx1, by1)


def bake(path, packer):
    """Renders one font's glyphs into the shared atlas; returns its table."""
    font = ImageFont.truetype(path, HI)
    cmap = TTFont(path).getBestCmap()
    cap = cap_height(font)
    glyphs = []
    for ch in CHARS:
        if ord(ch) not in cmap:  # missing from the font: drawn as a space
            glyphs.append((font.getlength(" ") / cap, None))
            continue
        tile, box = sdf_for(font, ch)
        adv = font.getlength(ch) / cap
        if tile is None:
            glyphs.append((adv, None))
            continue
        glyphs.append((adv, ([v / cap for v in box], packer.add(tile))))
    return {"name": os.path.basename(path), "space": font.getlength(" ") / cap, "glyphs": glyphs, "cap": cap}


class Packer:
    """Shelf packing: tiles left to right, a new row when one is full."""

    def __init__(self):
        self.x = self.y = self.row_h = 0
        self.tiles = []

    def add(self, tile):
        th, tw = tile.shape
        if self.x + tw > ATLAS_W:
            self.x, self.y, self.row_h = 0, self.y + self.row_h + 1, 0
        rect = (self.x, self.y, tw, th)
        self.tiles.append((self.x, self.y, tile))
        self.x += tw + 1
        self.row_h = max(self.row_h, th)
        return rect

    def image(self):
        h = self.y + self.row_h
        atlas = np.zeros((h, ATLAS_W), np.uint8)
        for tx, ty, tile in self.tiles:
            atlas[ty : ty + tile.shape[0], tx : tx + tile.shape[1]] = tile
        return atlas


def table(name, face):
    f = lambda v: f"{v:.4f}"
    lines = [
        f"/// {face['name']}",
        f"pub static {name}: Face = Face {{",
        f"    space: {f(face['space'])},",
        "    glyphs: [",
    ]
    for ch, (adv, rest) in zip(CHARS, face["glyphs"]):
        if rest is None:
            lines.append(f"        Glyph {{ advance: {f(adv)}, min: [0.0, 0.0], size: [0.0, 0.0], uv: [0.0, 0.0, 0.0, 0.0] }}, // {ch!r}")
            continue
        (bx0, by0, bx1, by1), (ax, ay, aw, ah) = rest
        lines.append(
            f"        Glyph {{ advance: {f(adv)}, min: [{f(bx0)}, {f(by0)}], size: [{f(bx1 - bx0)}, {f(by1 - by0)}], "
            f"uv: [{ax}.0, {ay}.0, {aw}.0, {ah}.0] }}, // {ch!r}"
        )
    lines += ["    ],", "};", ""]
    return lines


def main():
    args = dict(DEFAULTS)
    argv = sys.argv[1:]
    while argv:
        flag = argv.pop(0)
        if flag not in ("--title", "--body") or not argv:
            sys.exit(__doc__)
        args[flag[2:]] = argv.pop(0)
    packer = Packer()
    faces = {k: bake(find_ttf(v), packer) for k, v in args.items()}
    atlas = packer.image()
    os.makedirs(os.path.dirname(OUT_PNG), exist_ok=True)
    Image.fromarray(atlas, "L").save(OUT_PNG, optimize=True)

    lines = [
        "// Generated by tools/make_font.py; don't edit by hand.",
        "#![allow(dead_code)]",
        f"// Title: {faces['title']['name']}, body: {faces['body']['name']}. Units are cap heights (1.0 = height of 'H').",
        "",
        "use crate::font::{Face, Glyph};",
        "",
        f"pub const ATLAS_SIZE: [f32; 2] = [{ATLAS_W}.0, {atlas.shape[0]}.0];",
        "/// Distance-field spread on each side of the edge, in atlas pixels.",
        f"pub const SPREAD_TEXELS: f32 = {SPREAD}.0;",
        "",
    ]
    lines += table("TITLE", faces["title"]) + table("BODY", faces["body"])
    with open(OUT_RS, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(lines))
    print(f"title {faces['title']['name']}, body {faces['body']['name']}: atlas {ATLAS_W}x{atlas.shape[0]}")


if __name__ == "__main__":
    main()
