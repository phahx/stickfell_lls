# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow>=10"]
# ///
"""Builds the app icon from the cartoon artwork.

    uv run tools/make_icon.py

Reads game/assets/icon/icon-source.png (any size, transparent background),
trims it to the artwork, centres it on a square and writes:
  game/assets/icon/stickfell.ico   every Windows size, for the exe
  game/assets/icon/window-32.png   title bar icon
  game/assets/icon/window-64.png   taskbar icon
"""
import io
import os
import struct

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
DIR = os.path.join(HERE, "..", "game", "assets", "icon")
SIZES = [256, 128, 64, 48, 32, 24, 16]
MARGIN = 0.03  # empty border around the artwork, as a fraction of the side


def square(src):
    art = src.crop(src.getchannel("A").getbbox())
    side = round(max(art.size) / (1 - 2 * MARGIN))
    out = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    out.paste(art, ((side - art.width) // 2, (side - art.height) // 2))
    return out


def png_bytes(img):
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def write_ico(path, images):
    entries, blobs = [], []
    offset = 6 + 16 * len(images)
    for img in images:
        data = png_bytes(img)
        w = 0 if img.width >= 256 else img.width
        entries.append(struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(data), offset))
        blobs.append(data)
        offset += len(data)
    with open(path, "wb") as f:
        f.write(struct.pack("<HHH", 0, 1, len(images)))
        f.writelines(entries)
        f.writelines(blobs)


def main():
    src = square(Image.open(os.path.join(DIR, "icon-source.png")).convert("RGBA"))
    # (Pillow resamples RGBA with premultiplied alpha, so edges stay clean)
    images = {s: src.resize((s, s), Image.Resampling.LANCZOS) for s in SIZES}
    write_ico(os.path.join(DIR, "stickfell.ico"), [images[s] for s in SIZES])
    images[32].save(os.path.join(DIR, "window-32.png"))
    images[64].save(os.path.join(DIR, "window-64.png"))
    print(f"artwork {src.size[0]}px square, wrote stickfell.ico ({', '.join(map(str, SIZES))}) and window icons")


if __name__ == "__main__":
    main()
