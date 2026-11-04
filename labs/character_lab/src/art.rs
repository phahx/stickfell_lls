//! Loads a character kit (rig JSON + one PNG per body part) into a sprite
//! atlas and a [`Skin`] for the game's drawing code.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::Value;
use stickfell_physics::Vec2;
use stickfell_render::SpriteAtlas;

use crate::body::{PartSize, Proportions};
use crate::draw::{Skin, SkinPart, SkinRig, SkinWeapon};

pub const RIG_FILE: &str = "stickfigure-rig.json";
const PARTS: [&str; 6] = ["head", "torso", "upper_arm", "forearm", "thigh", "shin"];
/// Space around each part in the atlas, so smaller mips don't bleed together.
const GAP: u32 = 32;
const MIP_LEVELS: usize = 6;

/// A loaded kit: the skin (with every option off) and the files it came from.
pub struct Kit {
    pub skin: Skin,
    pub files: Vec<PathBuf>,
    /// The art's body proportions, scaled to the game's leg length.
    pub proportions: Proportions,
}

/// Newest modification time among `files` (to notice the art changing).
pub fn stamp(files: &[PathBuf]) -> Option<SystemTime> {
    files.iter().filter_map(|f| fs::metadata(f).and_then(|m| m.modified()).ok()).max()
}

pub fn load(dir: &Path) -> Result<Kit, String> {
    let rig_path = dir.join(RIG_FILE);
    let text = fs::read_to_string(&rig_path).map_err(|e| format!("{}: {e}", rig_path.display()))?;
    let rig: Value = serde_json::from_str(&text).map_err(|e| format!("{RIG_FILE}: {e}"))?;
    let mut files = vec![rig_path];
    let mut images = Vec::new();
    let mut metas = Vec::new();
    for name in PARTS {
        let part = &rig["parts"][name];
        let file = part["file"].as_str().ok_or(format!("{RIG_FILE}: part {name} has no file"))?;
        let path = inside(dir, file)?;
        images.push(read_png(&path)?);
        files.push(path);
        let pair = |key: &str| -> Result<Vec2, String> {
            let v = &part[key];
            match (v[0].as_f64(), v[1].as_f64()) {
                (Some(x), Some(y)) => Ok(Vec2::new(x as f32, y as f32)),
                _ => Err(format!("{RIG_FILE}: part {name} needs {key} as [x, y]")),
            }
        };
        let radius = part["radius"].as_f64().ok_or(format!("{RIG_FILE}: part {name} has no radius"))? as f32;
        metas.push((pair("pivot")?, pair("distal")?, radius));
    }

    // the game's weapon art, copied out of its own atlas
    let game = crate::character::skin();
    let base = &game.atlas.mips[0];
    let first_weapon = images.len();
    let rig_uvs: Vec<[f32; 4]> = game.rigs.iter().flat_map(|r| r.uvs.iter().copied().chain(r.rope)).collect();
    for uv in game.weapons.iter().map(|(_, w)| w.uv).chain(rig_uvs.iter().copied()) {
        let [x0, y0, ww, hh] = uv.map(|v| v as u32);
        let mut px = Vec::with_capacity((ww * hh * 4) as usize);
        for row in y0..y0 + hh {
            let at = ((row * game.atlas.width + x0) * 4) as usize;
            px.extend_from_slice(&base[at..at + (ww * 4) as usize]);
        }
        images.push(Image { w: ww, h: hh, px });
    }

    // one row, padded so every mip level halves evenly
    let align = 1 << (MIP_LEVELS - 1);
    let width = round_up(images.iter().map(|i| i.w + GAP).sum::<u32>() + GAP, align);
    let height = round_up(images.iter().map(|i| i.h).max().unwrap_or(1) + 2 * GAP, align);
    let mut atlas = vec![0u8; (width * height * 4) as usize];
    let mut x = GAP;
    let mut parts = Vec::new();
    let mut weapons = Vec::new();
    let mut moved_rig_uvs = Vec::new();
    for (i, img) in images.iter().enumerate() {
        for row in 0..img.h {
            let src = &img.px[(row * img.w * 4) as usize..((row + 1) * img.w * 4) as usize];
            let at = (((GAP + row) * width + x) * 4) as usize;
            atlas[at..at + src.len()].copy_from_slice(src);
        }
        let uv = [x as f32, GAP as f32, img.w as f32, img.h as f32];
        if let Some(&(pivot, distal, radius)) = metas.get(i) {
            parts.push(SkinPart { uv, pivot, distal, radius });
        } else if i - first_weapon < game.weapons.len() {
            let (name, w) = game.weapons[i - first_weapon];
            weapons.push((name, SkinWeapon { uv, ..w }));
        } else {
            moved_rig_uvs.push(uv);
        }
        x += img.w + GAP;
    }
    bleed(&mut atlas, width, height);
    let mut mips = vec![atlas];
    let (mut w, mut h) = (width, height);
    while mips.len() < MIP_LEVELS {
        mips.push(halve(mips.last().unwrap(), w, h));
        w /= 2;
        h /= 2;
    }
    let size = |p: &SkinPart| PartSize { length: (p.distal - p.pivot).length(), radius: p.radius };
    // how far the head's centre sits above the shoulders in the neutral pose
    let joint = |name: &str| rig["neutral_joints"][name].as_array().and_then(|v| Some(Vec2::new(v.first()?.as_f64()? as f32, v.get(1)?.as_f64()? as f32)));
    let neck = match (joint("head"), joint("shoulder")) {
        (Some(h), Some(s)) => (h - s).length(),
        _ => parts[0].radius,
    };
    let proportions = Proportions::from_art(size(&parts[0]), size(&parts[1]), size(&parts[2]), size(&parts[3]), size(&parts[4]), size(&parts[5]), neck);
    let skin = Skin {
        atlas: Arc::new(SpriteAtlas { width, height, mips }),
        head: parts[0],
        torso: parts[1],
        upper_arm: parts[2],
        forearm: parts[3],
        thigh: parts[4],
        shin: parts[5],
        faces: true,
        outline: false,
        art_width: false,
        weapons,
        rigs: {
            let mut left = moved_rig_uvs.into_iter();
            game.rigs
                .iter()
                .map(|r| {
                    let uvs: Vec<[f32; 4]> = left.by_ref().take(r.uvs.len()).collect();
                    let rope = r.rope.and_then(|_| left.next());
                    SkinRig { rig: r.rig, uvs, rope }
                })
                .collect()
        },
    };
    Ok(Kit { skin, files, proportions })
}

/// `file` (from the rig) as a path inside `dir`; anything that climbs out
/// of the kit folder is refused.
fn inside(dir: &Path, file: &str) -> Result<PathBuf, String> {
    let rel = Path::new(file);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("{RIG_FILE}: part file {file:?} must be a plain path inside the kit folder"));
    }
    Ok(dir.join(rel))
}

fn round_up(v: u32, to: u32) -> u32 {
    v.div_ceil(to) * to
}

struct Image {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// Reads a PNG as straight-alpha RGBA8, with its lightest tone scaled up to
/// white so the tint comes out as the player's actual colour.
fn read_png(path: &Path) -> Result<Image, String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = fs::File::open(path).map_err(|e| bad(&e))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| bad(&e))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| bad(&"image too large"))?];
    let info = reader.next_frame(&mut buf).map_err(|e| bad(&e))?;
    buf.truncate(info.buffer_size());
    let px: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(bad(&format!("unsupported PNG colour type {other:?}"))),
    };
    let lightest = px.chunks(4).filter(|p| p[3] >= 250).map(|p| p[0].max(p[1]).max(p[2])).max().unwrap_or(255).max(1);
    let k = 255.0 / lightest as f32;
    let px = px.chunks(4).flat_map(|p| [(p[0] as f32 * k).min(255.0) as u8, (p[1] as f32 * k).min(255.0) as u8, (p[2] as f32 * k).min(255.0) as u8, p[3]]).collect();
    Ok(Image { w: info.width, h: info.height, px })
}

/// Spreads edge colours into the transparent pixels around them, so
/// filtering and mips don't pull dark fringes in from the empty background.
fn bleed(px: &mut [u8], w: u32, h: u32) {
    let (w, h) = (w as usize, h as usize);
    let mut known: Vec<bool> = px.chunks(4).map(|p| p[3] > 0).collect();
    for _ in 0..GAP {
        let mut grew = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if known[i] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0);
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if known[j] {
                        for c in 0..3 {
                            sum[c] += px[j * 4 + c] as u32;
                        }
                        n += 1;
                    }
                }
                if n > 0 {
                    grew.push((i, sum.map(|s| (s / n) as u8)));
                }
            }
        }
        if grew.is_empty() {
            break;
        }
        for (i, rgb) in grew {
            px[i * 4..i * 4 + 3].copy_from_slice(&rgb);
            known[i] = true;
        }
    }
}

/// The next mip level: each pixel the average of a 2x2 block.
fn halve(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1) as usize, (h / 2).max(1) as usize);
    let w = w as usize;
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let at = |xx: usize, yy: usize| px[(yy * w + xx) * 4 + c] as u32;
                let s = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                out[(y * nw + x) * 4 + c] = ((s + 2) / 4) as u8;
            }
        }
    }
    out
}
