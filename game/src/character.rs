//! The fighters' character: the art baked in by `tools/make_character.py`
//! (one texture per body part) and the body it describes.

use std::sync::{Arc, OnceLock};

use stickfell_physics::Vec2;
use stickfell_render::SpriteAtlas;

use crate::character_data as data;
use crate::draw::{Skin, SkinPart, SkinRig, SkinWeapon};

/// One part's texture: its rect in the atlas (x, y, w, h) and, in its own
/// pixels (y down), the proximal joint, the distal joint and the radius.
pub struct Part {
    pub uv: [f32; 4],
    pub pivot: [f32; 2],
    pub distal: [f32; 2],
    pub radius: f32,
}

/// A weapon's texture: which weapon (its `WeaponDef` name), its rect in the
/// atlas (x, y, w, h), and where its grip and tip are in its own pixels (y
/// down). It's drawn so those land on the weapon's grip and aim-tip points.
pub struct WeaponArt {
    pub name: &'static str,
    pub uv: [f32; 4],
    pub grip: [f32; 2],
    pub tip: [f32; 2],
}

/// One piece of a weapon drawn from moving parts (gun pixels, y down): its
/// rect in the atlas, the point it turns about (in its own pixels), and
/// where that point sits: on the gun, or (`on_hook`) off the hook's pivot.
#[derive(Clone, Copy, Debug)]
pub struct RigPiece {
    pub name: &'static str,
    pub uv: [f32; 4],
    pub pivot: [f32; 2],
    pub at: [f32; 2],
    pub on_hook: bool,
}

/// A weapon drawn from moving pieces: which weapon, its pieces in drawing
/// order, its sockets (gun pixels; the hook's rope eye and tip measured from
/// the hook's pivot), and how far each part moves (degrees, clockwise).
#[derive(Debug)]
pub struct WeaponRig {
    pub name: &'static str,
    pub grip: [f32; 2],
    pub muzzle: [f32; 2],
    pub hook: [f32; 2],
    pub rope_eye: [f32; 2],
    pub tip: [f32; 2],
    pub trigger_pull: f32,
    pub needle_empty: f32,
    pub prong_fold: [f32; 2],
    pub drum_r: f32,
    pub pieces: &'static [RigPiece],
    /// Its rope's tile (repeated along the rope), if it has its own.
    pub rope: Option<[f32; 4]>,
}

/// A level art piece's texture (256 px per metre): its name, its rect in the
/// atlas, and its glow image's rect (the part that blooms), if any.
pub struct LevelPiece {
    pub name: &'static str,
    pub uv: [f32; 4],
    pub glow: Option<[f32; 4]>,
}

/// The level art piece called `name`, if it's been baked in.
pub fn level_piece(name: &str) -> Option<&'static LevelPiece> {
    data::LEVEL_ART.iter().find(|p| p.name == name)
}

/// Bone lengths and radii in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Build {
    pub torso: f32,
    pub upper_arm: f32,
    pub forearm: f32,
    pub thigh: f32,
    pub shin: f32,
    /// Neck to the middle of the head.
    pub neck: f32,
    pub head_r: f32,
    pub torso_r: f32,
    pub upper_arm_r: f32,
    pub forearm_r: f32,
    pub thigh_r: f32,
    pub shin_r: f32,
}

/// Every fighter's body.
pub const BUILD: Build = data::BUILD;

/// Mip levels made for the atlas (its sides divide by 2^(n-1)).
const MIPS: usize = 6;

static ATLAS_PNG: &[u8] = include_bytes!("../assets/character/atlas.png");

/// The character art as the fighters are drawn with it (faces on).
pub fn skin() -> Arc<Skin> {
    static SKIN: OnceLock<Arc<Skin>> = OnceLock::new();
    SKIN.get_or_init(|| {
        let part = |p: &Part| SkinPart { uv: p.uv, pivot: Vec2::from(p.pivot), distal: Vec2::from(p.distal), radius: p.radius };
        Arc::new(Skin {
            atlas: Arc::new(atlas()),
            head: part(&data::HEAD),
            torso: part(&data::TORSO),
            upper_arm: part(&data::UPPER_ARM),
            forearm: part(&data::FOREARM),
            thigh: part(&data::THIGH),
            shin: part(&data::SHIN),
            faces: true,
            outline: false,
            art_width: false,
            weapons: data::WEAPONS.iter().map(|a| (a.name, SkinWeapon { uv: a.uv, grip: Vec2::from(a.grip), tip: Vec2::from(a.tip) })).collect(),
            rigs: data::WEAPON_RIGS.iter().map(|r| SkinRig { rig: r, uvs: r.pieces.iter().map(|p| p.uv).collect(), rope: r.rope }).collect(),
        })
    })
    .clone()
}

fn atlas() -> SpriteAtlas {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ATLAS_PNG)).read_info().expect("character atlas");
    let mut buf = vec![0; reader.output_buffer_size().expect("character atlas size")];
    let info = reader.next_frame(&mut buf).expect("character atlas");
    assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight), "character atlas must be 8-bit RGBA");
    buf.truncate(info.buffer_size());
    assert_eq!([info.width, info.height], data::ATLAS_SIZE, "character_data.rs is out of date with the atlas");
    let (mut w, mut h) = (info.width, info.height);
    let mut mips = vec![buf];
    while mips.len() < MIPS {
        mips.push(halve(mips.last().unwrap(), w, h));
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    SpriteAtlas { width: info.width, height: info.height, mips }
}

/// The next mip level: each pixel the average of a 2x2 block.
pub fn halve(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1) as usize, (h / 2).max(1) as usize);
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let at = |xx: usize, yy: usize| px[(yy.min(h - 1) * w + xx.min(w - 1)) * 4 + c] as u32;
                let s = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                out[(y * nw + x) * 4 + c] = ((s + 2) / 4) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_baked_character_loads() {
        let skin = super::skin();
        assert_eq!(skin.atlas.mips.len(), super::MIPS);
        let b = super::BUILD;
        assert!(b.torso > 0.3 && b.upper_arm + b.forearm > 0.5 && b.head_r > 0.1, "{b:?}");
    }
}
