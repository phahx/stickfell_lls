# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow>=10", "numpy>=1.26"]
# ///
"""Bakes a character kit into the game: the part textures and the body
they describe.

    uv run tools/make_character.py [KIT_FOLDER]

Default kit: S:/asset-art/stickfell-character-base (stickfigure-rig.json plus
one PNG per part). Writes game/assets/character/atlas.png (the parts in one
row, lightest grey scaled to white so the player colour tints it exactly,
edges bled so filtering doesn't pull in dark fringes) and
game/src/character_data.rs (each part's atlas rect and joints, and the body
in metres: the legs keep the game's length, everything else follows the art).

Weapon art comes along too: every game/assets/weapons/<name>.png with a
<name>.json beside it (the "markers" from the weapon workspace's export:
"grip" and "tip" in the PNG's pixels). Weapons keep their own colours and
are trimmed to what's drawn.

A weapon made of moving pieces is a folder, game/assets/weapons/<name>/,
holding <name>.json (the rig: each piece's pivot, where it sits on the gun,
the sockets and the animation ranges) and pieces/<piece>.png.

Level art comes along as well: every game/assets/level/<piece>.png, as is
(a <piece>_glow.png is the bright part of <piece> that should bloom).
"""
import json
import os
import re
import sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_PNG = os.path.join(HERE, "..", "game", "assets", "character", "atlas.png")
OUT_RS = os.path.join(HERE, "..", "game", "src", "character_data.rs")
DEFAULT = "S:/asset-art/stickfell-character-base"
WEAPONS_DIR = os.path.join(HERE, "..", "game", "assets", "weapons")
LEVEL_DIR = os.path.join(HERE, "..", "game", "assets", "level")
MAX_WIDTH = 2048  # atlas rows wrap at this width
PARTS = ["head", "torso", "upper_arm", "forearm", "thigh", "shin"]
GAP = 32  # around each part, so smaller mips don't run together
ALIGN = 32  # atlas sides are multiples of this (clean mip halving)
# the game's legs (Fighter::spawn): pelvis (0, .98) -> knee (.1, .54) -> foot (.06, .1)
GAME_LEG = float(np.hypot(0.1, 0.44) + np.hypot(0.04, 0.44))


def inside(kit, rel):
    parts = rel.replace("\\", "/").split("/")
    if os.path.isabs(rel) or any(p in ("", ".", "..") for p in parts):
        sys.exit(f"part file {rel!r} must be a plain path inside the kit folder")
    return os.path.join(kit, *parts)


def load(path):
    img = np.array(Image.open(path).convert("RGBA")).astype(np.float32)
    opaque = img[..., 3] >= 250
    lightest = img[..., :3][opaque].max() if opaque.any() else 255.0
    img[..., :3] = np.minimum(img[..., :3] * (255.0 / max(lightest, 1.0)), 255.0)
    return img


def load_weapons():
    """(NAME, trimmed RGBA image, grip, tip) for each weapon with markers."""
    out = []
    if not os.path.isdir(WEAPONS_DIR):
        return out
    for f in sorted(os.listdir(WEAPONS_DIR)):
        stem, ext = os.path.splitext(f)
        if ext.lower() != ".png":
            continue
        if not re.fullmatch(r"[a-z0-9_-]+", stem):
            print(f"weapon {f}: name must be lowercase letters, digits, _ or -, skipped")
            continue
        meta_path = os.path.join(WEAPONS_DIR, stem + ".json")
        if not os.path.exists(meta_path):
            print(f"weapon {f}: no {stem}.json with its grip and tip markers, skipped")
            continue
        markers = json.load(open(meta_path, encoding="utf-8")).get("markers", {})
        if "grip" not in markers or "tip" not in markers:
            sys.exit(f"weapon {stem}.json needs both a grip and a tip marker")
        img = np.array(Image.open(os.path.join(WEAPONS_DIR, f)).convert("RGBA")).astype(np.float32)
        ys, xs = np.nonzero(img[..., 3] > 0)
        if len(xs) == 0:
            print(f"weapon {f}: empty, skipped")
            continue
        x0, y0 = int(xs.min()), int(ys.min())
        img = img[y0 : int(ys.max()) + 1, x0 : int(xs.max()) + 1].copy()
        grip = (float(markers["grip"][0]) - x0, float(markers["grip"][1]) - y0)
        tip = (float(markers["tip"][0]) - x0, float(markers["tip"][1]) - y0)
        name = stem.replace("_", " ").replace("-", " ").upper()
        out.append((name, img, grip, tip))
    return out


def load_rigs():
    """(NAME, rig json, [(piece json, RGBA image)]) for each rigged weapon."""
    out = []
    if not os.path.isdir(WEAPONS_DIR):
        return out
    for d in sorted(os.listdir(WEAPONS_DIR)):
        folder = os.path.join(WEAPONS_DIR, d)
        rig_path = os.path.join(folder, d + ".json")
        if not os.path.isdir(folder) or not os.path.exists(rig_path):
            continue
        if not re.fullmatch(r"[a-z0-9_]+", d):
            print(f"rig {d}: name must be lowercase letters, digits or _, skipped")
            continue
        rig = json.load(open(rig_path, encoding="utf-8"))
        by_name = {p["name"]: p for p in rig["pieces"]}
        pieces = []
        for name in rig["draw_order"]:
            if not re.fullmatch(r"[a-z0-9_]+", name) or name not in by_name:
                sys.exit(f"rig {d}: bad piece {name!r}")
            img = np.array(Image.open(os.path.join(folder, "pieces", name + ".png")).convert("RGBA")).astype(np.float32)
            pieces.append((by_name[name], img))
        # its rope, a tile repeated along it (the sharp @4x one if there is)
        rope = None
        tile = rig.get("rope", {}).get("tile_4x") or rig.get("rope", {}).get("tile")
        if tile:
            if not re.fullmatch(r"[a-z0-9_@.]+\.png", tile):
                sys.exit(f"rig {d}: bad rope tile {tile!r}")
            rope = np.array(Image.open(os.path.join(folder, tile)).convert("RGBA")).astype(np.float32)
        out.append((d.replace("_", " ").upper(), rig, pieces, rope))
    return out


def load_level():
    """(name, RGBA image) for each level piece, sorted by name."""
    out = []
    if not os.path.isdir(LEVEL_DIR):
        return out
    for f in sorted(os.listdir(LEVEL_DIR)):
        stem, ext = os.path.splitext(f)
        if ext.lower() != ".png":
            continue
        if not re.fullmatch(r"[a-z0-9_]+", stem):
            print(f"level piece {f}: name must be lowercase letters, digits or _, skipped")
            continue
        out.append((stem, np.array(Image.open(os.path.join(LEVEL_DIR, f)).convert("RGBA")).astype(np.float32)))
    return out


def pack(imgs):
    """Shelf-packs the images (tallest first), GAP apart: their rects and the
    atlas size."""
    up = lambda v, a: -(-v // a) * a
    rects = [None] * len(imgs)
    x, y, row = GAP, GAP, 0
    for i in sorted(range(len(imgs)), key=lambda i: (-imgs[i].shape[0], i)):
        h, w = imgs[i].shape[:2]
        if x + w + GAP > MAX_WIDTH and x > GAP:
            x, y, row = GAP, y + row + GAP, 0
        rects[i] = (x, y, w, h)
        x += w + GAP
        row = max(row, h)
    width = up(max(r[0] + r[2] for r in rects) + GAP, ALIGN)
    height = up(y + row + GAP, ALIGN)
    return rects, width, height


def bleed(img):
    known = img[..., 3] > 0
    rgb = img[..., :3].copy()
    for _ in range(GAP):
        if known.all():
            break
        acc = np.zeros_like(rgb)
        cnt = np.zeros(known.shape, np.float32)
        for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
            k = np.roll(known, (dy, dx), (0, 1))
            c = np.roll(rgb, (dy, dx), (0, 1))
            acc += c * k[..., None]
            cnt += k
        grow = (~known) & (cnt > 0)
        rgb[grow] = acc[grow] / cnt[grow][:, None]
        known = known | grow
    img[..., :3] = rgb
    return img


def main():
    kit = sys.argv[1] if len(sys.argv) > 1 else DEFAULT
    rig = json.load(open(os.path.join(kit, "stickfigure-rig.json"), encoding="utf-8"))
    imgs, metas = [], []
    for name in PARTS:
        p = rig["parts"][name]
        imgs.append(load(inside(kit, p["file"])))
        metas.append((p["pivot"], p["distal"], float(p["radius"])))
    weapons = load_weapons()
    imgs += [w[1] for w in weapons]
    level = load_level()
    imgs += [p[1] for p in level]
    rigs = load_rigs()
    for _, _, pieces, rope in rigs:
        imgs += [img for _, img in pieces]
        if rope is not None:
            imgs.append(rope)
    rects, width, height = pack(imgs)
    atlas = np.zeros((height, width, 4), np.float32)
    for img, (x, y, w, h) in zip(imgs, rects):
        atlas[y : y + h, x : x + w] = img
    atlas = bleed(atlas)
    os.makedirs(os.path.dirname(OUT_PNG), exist_ok=True)
    Image.fromarray(np.clip(atlas + 0.5, 0, 255).astype(np.uint8), "RGBA").save(OUT_PNG, optimize=True)

    length = lambda m: float(np.hypot(m[1][0] - m[0][0], m[1][1] - m[0][1]))
    head, torso, upper, fore, thigh, shin = metas
    joints = rig.get("neutral_joints", {})
    neck_px = float(np.hypot(joints["head"][0] - joints["shoulder"][0], joints["head"][1] - joints["shoulder"][1])) if "head" in joints and "shoulder" in joints else head[2]
    k = GAME_LEG / (length(thigh) + length(shin))
    f = lambda v: f"{v:.5f}"
    lines = [
        "// Generated by tools/make_character.py from the character kit; don't edit by hand.",
        f"// Kit: {os.path.basename(os.path.normpath(kit))}.",
        "",
        "use crate::character::{Build, LevelPiece, Part, RigPiece, WeaponArt, WeaponRig};",
        "",
        f"pub const ATLAS_SIZE: [u32; 2] = [{width}, {height}];",
        "",
    ]
    for name, (rx, ry, rw, rh), (pivot, distal, radius) in zip(PARTS, rects[: len(PARTS)], metas):
        lines.append(
            f"pub const {name.upper()}: Part = Part {{ uv: [{rx}.0, {ry}.0, {rw}.0, {rh}.0], pivot: [{f(pivot[0])}, {f(pivot[1])}], distal: [{f(distal[0])}, {f(distal[1])}], radius: {f(radius)} }};"
        )
    lines += [
        "",
        "/// The kit's body in metres (legs as long as the game's, the rest in proportion).",
        "pub const BUILD: Build = Build {",
        f"    torso: {f(length(torso) * k)},",
        f"    upper_arm: {f(length(upper) * k)},",
        f"    forearm: {f(length(fore) * k)},",
        f"    thigh: {f(length(thigh) * k)},",
        f"    shin: {f(length(shin) * k)},",
        f"    neck: {f(neck_px * k)},",
        f"    head_r: {f(head[2] * k)},",
        f"    torso_r: {f(torso[2] * k)},",
        f"    upper_arm_r: {f(upper[2] * k)},",
        f"    forearm_r: {f(fore[2] * k)},",
        f"    thigh_r: {f(thigh[2] * k)},",
        f"    shin_r: {f(shin[2] * k)},",
        "};",
        "",
        "/// Weapon art: the name of the weapon it's for, its rect in the atlas, and",
        "/// its grip and tip in its own pixels (y down).",
        "pub const WEAPONS: &[WeaponArt] = &[",
    ]
    for (name, _, grip, tip), (rx, ry, rw, rh) in zip(weapons, rects[len(PARTS) : len(PARTS) + len(weapons)]):
        lines.append(
            f'    WeaponArt {{ name: "{name}", uv: [{rx}.0, {ry}.0, {rw}.0, {rh}.0], grip: [{f(grip[0])}, {f(grip[1])}], tip: [{f(tip[0])}, {f(tip[1])}] }},'
        )
    lines += [
        "];",
        "",
        "/// Level art pieces (256 px per metre): name, rect in the atlas, and the",
        "/// rect of its glow image if it has one.",
        "pub const LEVEL_ART: &[LevelPiece] = &[",
    ]
    level_rects = dict(zip((p[0] for p in level), rects[len(PARTS) + len(weapons) : len(PARTS) + len(weapons) + len(level)]))
    uv = lambda r: f"[{r[0]}.0, {r[1]}.0, {r[2]}.0, {r[3]}.0]"
    for name, rect in level_rects.items():
        if name.endswith("_glow"):
            continue
        glow = level_rects.get(name + "_glow")
        lines.append(f'    LevelPiece {{ name: "{name}", uv: {uv(rect)}, glow: {f"Some({uv(glow)})" if glow else "None"} }},')
    lines += [
        "];",
        "",
        "/// Weapons drawn from moving pieces (gun pixels, y down, 420 px per metre):",
        "/// the pieces in drawing order, the sockets, and how far each part moves.",
        "pub const WEAPON_RIGS: &[WeaponRig] = &[",
    ]
    at = len(PARTS) + len(weapons) + len(level)
    v2 = lambda p: f"[{f(p[0])}, {f(p[1])}]"
    for name, rig, pieces, rope in rigs:
        sk, an = rig["sockets"], rig["animation"]
        hook = next(p for p, _ in pieces if p["name"] == "hook")
        lines.append(f'    WeaponRig {{')
        lines.append(f'        name: "{name}",')
        lines.append(f'        grip: {v2(sk["grip"]["gun"])},')
        lines.append(f'        muzzle: {v2(sk["muzzle"]["gun"])},')
        lines.append(f'        hook: {v2(hook["pivot_gun"])},')
        lines.append(f'        rope_eye: {v2(sk["rope_eye"]["from_hook_pivot"])},')
        lines.append(f'        tip: {v2(sk["tip"]["from_hook_pivot"])},')
        lines.append(f'        trigger_pull: {f(an["trigger_pull_deg"])},')
        lines.append(f'        needle_empty: {f(an["needle_empty_deg"])},')
        lines.append(f'        prong_fold: [{f(an["prong_fold_deg"]["prong_up"])}, {f(an["prong_fold_deg"]["prong_down"])}],')
        lines.append(f'        drum_r: {f(an["drum_rope_radius_px"])},')
        lines.append("        pieces: &[")
        for p, _ in pieces:
            x, y, w, h = rects[at]
            at += 1
            on_hook = p.get("parent") == "hook"
            place = p["pivot_in_hook"] if on_hook else p["pivot_gun"]
            lines.append(
                f'            RigPiece {{ name: "{p["name"]}", uv: [{x}.0, {y}.0, {w}.0, {h}.0], pivot: {v2(p["pivot"])}, at: {v2(place)}, on_hook: {"true" if on_hook else "false"} }},'
            )
        lines.append("        ],")
        if rope is not None:
            x, y, w, h = rects[at]
            at += 1
            lines.append(f"        rope: Some([{x}.0, {y}.0, {w}.0, {h}.0]),")
        else:
            lines.append("        rope: None,")
        lines.append("    },")
    lines += [
        "];",
        "",
    ]
    with open(OUT_RS, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(lines))
    print(f"level pieces: {sum(1 for p in level if not p[0].endswith('_glow'))}")
    for name, _, pieces, rope in rigs:
        print(f"rig {name}: {len(pieces)} pieces{', rope tile' if rope is not None else ''}")
    for name, img, _, _ in weapons:
        print(f"weapon {name}: {img.shape[1]}x{img.shape[0]}")
    print(f"atlas {width}x{height}; legs {GAME_LEG:.3f} m, torso {length(torso) * k:.3f}, arm {(length(upper) + length(fore)) * k:.3f}, head r {head[2] * k:.3f}")


if __name__ == "__main__":
    main()
