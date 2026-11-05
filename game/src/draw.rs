//! Presentation: turns a `GameState` into a lit `Frame`. Render-only code may
//! use platform math freely; nothing here feeds back into the simulation.

use stickfell_physics::Vec2;
use std::sync::Arc;

use stickfell_physics::World;
use stickfell_render::{Camera2D, Color, Font, Frame, Light, Occluder, SpriteAtlas, StaticScene};

use crate::fighter::{self, Fighter, HAND_A, HEAD, HEAD_R, LIMB_R, MAX_HEALTH, NECK, PELVIS, S_FORE_A, S_FORE_B, S_HEAD, S_SHIN_A, S_SHIN_B, S_THIGH_A, S_THIGH_B, S_TORSO, S_UPPER_A, S_UPPER_B};
use crate::level::{ArtLayer, HazardKind, LevelDef};
use crate::sim::{CLASH_KEY_NAMES, CLASH_WIN, ClashStage, GameState, Phase};
use crate::weapon::{Kind, Weapon};

/// Each player's colour (every one for themselves), by slot.
pub const PLAYER_COLORS: [u32; 8] = [0xe8484f, 0x3f8cff, 0xffc93c, 0x4cd16a, 0xb06cff, 0xff8a2a, 0x3fd8d8, 0xff6ac1];
/// In a team match: the red team's shades and the blue team's (each
/// fighter on a team a shade of its colour, so they can still be told apart).
pub const TEAM_COLORS: [[u32; 4]; 2] = [[0xe8484f, 0xff8a5a, 0xc8306a, 0xff6a6a], [0x3f8cff, 0x3fc8e8, 0x6a6cff, 0x7ab0ff]];
/// The teams' names, as the round's winner is called.
pub const TEAM_NAMES: [&str; 2] = ["RED TEAM", "BLUE TEAM"];

/// The colour fighter `id` is drawn and labelled in.
pub fn fighter_color(s: &GameState, id: usize) -> u32 {
    match s.team(id) {
        Some(t) => TEAM_COLORS[t as usize][(id / 2) % 4],
        None => PLAYER_COLORS[id % PLAYER_COLORS.len()],
    }
}

#[derive(Default)]
pub struct Hud {
    pub lines: Vec<(String, Color)>,
    pub banner: Option<(String, Color)>,
    /// Screen-space cursor (drawn as the corners of a small ring).
    pub mouse: Option<Vec2>,
    /// Index of the local player's fighter.
    pub local: Option<usize>,
    /// Draw the world only (menu background).
    pub world_only: bool,
    /// Camera view height in metres (0 = the normal `CAMERA_HEIGHT`).
    pub zoom: f32,
    /// Who's saying a voice line right now (per fighter): what kind, and how
    /// open their mouth is.
    pub talk: [Option<(crate::sfx::Sfx, f32)>; crate::sim::MAX_PLAYERS],
    /// Character art to draw the fighters with (None: the game's own, from
    /// `character`; the character lab swaps in a kit it's trying out).
    pub skin: Option<Arc<Skin>>,
    /// Draw the fighters as plain capsules instead (the lab's comparison).
    pub capsules: bool,
}

/// Character art drawn over the ragdoll: the game's own (`character::skin`),
/// or a kit the character lab (`labs/character_lab`) is trying out.
#[derive(Clone)]
pub struct Skin {
    pub atlas: Arc<SpriteAtlas>,
    pub head: SkinPart,
    pub torso: SkinPart,
    pub upper_arm: SkinPart,
    pub forearm: SkinPart,
    pub thigh: SkinPart,
    pub shin: SkinPart,
    /// Draw the game's faces on the head.
    pub faces: bool,
    /// A dark outline behind the body, like the capsule fighters have.
    pub outline: bool,
    /// Keep the art's own thickness instead of matching the hitboxes.
    pub art_width: bool,
    /// Weapon art in the same atlas, by weapon name.
    pub weapons: Vec<(&'static str, SkinWeapon)>,
    /// Weapons drawn from moving pieces (each piece's rect in this atlas).
    pub rigs: Vec<SkinRig>,
}

/// A rigged weapon as this skin draws it: the rig, and where each of its
/// pieces is in the atlas.
#[derive(Clone)]
pub struct SkinRig {
    pub rig: &'static crate::character::WeaponRig,
    pub uvs: Vec<[f32; 4]>,
    /// The rope tile's rect in this atlas.
    pub rope: Option<[f32; 4]>,
}

/// A rope drawn from its tile: repeated along `path`, `thick` metres across
/// (the last tile of each stretch cut short).
fn draw_rope_tiled(frame: &mut Frame, path: &[Vec2], uv: [f32; 4], thick: f32) {
    let tile = thick * uv[2] / uv[3];
    for seg in path.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let len = (b - a).length();
        if len < 1e-4 {
            continue;
        }
        let dir = (b - a) / len;
        let mut d = 0.0;
        while d < len - 1e-4 {
            let piece = (len - d).min(tile);
            let frac = piece / tile;
            let center = a + dir * (d + piece * 0.5);
            frame.sprite(center, Vec2::new(piece * 0.5, thick * 0.5), dir, [uv[0], uv[1], uv[2] * frac, uv[3]], Color::WHITE);
            d += tile;
        }
    }
}

/// Steam: its grid handed to the renderer, drawn as one volume (see
/// stickfell_render's shaders/vapour.wgsl).
fn draw_steam(s: &GameState, frame: &mut Frame, _t: f32) {
    let (origin, w, h, thick) = s.steam.grid();
    if thick.iter().all(|&t| t < 0.002) {
        return;
    }
    frame.vapour = Some(stickfell_render::VapourField {
        origin,
        cell: crate::steam::CELL,
        w: w as u32,
        h: h as u32,
        thick: thick.to_vec(),
        density: STEAM_DENSITY,
        max_opacity: STEAM_MAX_OPACITY,
    });
}

/// How dense steam looks (opacity 1 - e^(-this x thickness)), and the most
/// opaque it gets (still see-through: fighters stay readable).
const STEAM_DENSITY: f32 = 1.3;
const STEAM_MAX_OPACITY: f32 = 0.72;

/// The flood trap: a lever on its floor, a hatch in the ceiling whose two
/// doors swing down when it's thrown (and back up when it's thrown back).
fn draw_trap(s: &GameState, frame: &mut Frame) {
    let Some(def) = s.trap.def else { return };
    let thrown = s.trap.lever(s.round_frame);
    let open = s.trap.hatch(s.round_frame);
    // the lever: a base, the handle, a red knob
    let iron = Color::hex(0x2c2f36);
    frame.rect(def.lever + Vec2::new(0.0, 0.06), Vec2::new(0.16, 0.06), Vec2::X, 0.02, iron);
    let tip = def.handle(thrown);
    frame.capsule(def.lever + Vec2::new(0.0, 0.08), tip, 0.035, Color::hex(0x8a8f99));
    frame.circle(tip, 0.07, Color::hex(0xc8302a));
    frame.occluder(Occluder::Segment { a: def.lever, b: tip });
    // the hatch: a dark opening, two doors hinged at its sides swinging down
    let half = def.hatch_half;
    let open_ease = open * open * (3.0 - 2.0 * open);
    frame.rect(def.hatch + Vec2::new(0.0, 0.08), Vec2::new(half, 0.08), Vec2::X, 0.0, Color::hex(0x0b0d12));
    for side in [-1.0f32, 1.0] {
        let hinge = def.hatch + Vec2::new(side * half, -0.03);
        // closed: lying flat across its half; open: hanging straight down
        let ang = open_ease * core::f32::consts::FRAC_PI_2;
        let dir = Vec2::new(-side * ang.cos(), -ang.sin());
        let mid = hinge + dir * (half * 0.5);
        frame.rect(mid, Vec2::new(half * 0.5, 0.04), dir * -side, 0.01, Color::hex(0x4b5262));
        frame.occluder(Occluder::Segment { a: hinge, b: hinge + dir * half });
    }
}

/// The liquids: every drop a blob (they run together into the body of
/// liquid in the shader, see stickfell_render's fluid_*.wgsl), foamier the
/// faster it's going; and the glowing ones light the room around them.
fn draw_liquids(s: &GameState, frame: &mut Frame, t: f32) {
    let drops = &s.fluid.drops;
    if drops.is_empty() {
        return;
    }
    for (k, l) in crate::liquid::LIQUIDS.iter().enumerate() {
        frame.fluid_looks[k] = l.look();
    }
    for d in drops {
        let churn = ((d.vel.length() - 2.0) / 4.0).clamp(0.0, 1.0);
        frame.fluid_drop(d.pos, d.vel, crate::liquid::DROP_BLOB, d.kind, churn, d.dye);
    }
    // light from glowing liquid: one over each patch of it (a coarse grid)
    let cell = 3.0;
    let mut patches: Vec<(i32, i32, u8, Vec2, f32, f32)> = Vec::new();
    for d in drops {
        // (cooled lava gives no light)
        if crate::liquid::LIQUIDS[d.kind as usize].light.is_none() || d.dye >= crate::liquid::COOLED_LAVA {
            continue;
        }
        let key = ((d.pos.x / cell).floor() as i32, (d.pos.y / cell).floor() as i32);
        match patches.iter_mut().find(|p| p.0 == key.0 && p.1 == key.1 && p.2 == d.kind) {
            Some(p) => {
                p.3 += d.pos;
                p.4 += 1.0;
                p.5 = p.5.max(d.pos.y);
            }
            None => patches.push((key.0, key.1, d.kind, d.pos, 1.0, d.pos.y)),
        }
    }
    for (k, (_, _, kind, sum, n, top)) in patches.iter().enumerate() {
        if *n < 4.0 {
            continue;
        }
        let Some((color, strength)) = crate::liquid::LIQUIDS[*kind as usize].light else { continue };
        let c = *sum / *n;
        let flicker = 1.0 + 0.06 * (t * 3.1 + k as f32 * 1.7).sin();
        let size = (*n / 60.0).sqrt().clamp(0.4, 1.0);
        frame.light(Light::point(Vec2::new(c.x, top + 0.4), cell * 1.6 + 1.0, Color::hex(color).scale(strength * size * flicker)).no_shadows());
    }
}

/// A spike: a stack of thin slabs narrowing to the point makes the triangle
/// (lit on one side, shaded on the other); from `from` (share up it) to the
/// point, bloody if it's through someone.
fn draw_tooth(frame: &mut Frame, t: &crate::level::Tooth, from: f32, bloody: bool) {
    let n = 10;
    let step = (t.tip.y - t.base.y) / n as f32;
    let (dark, lit) = if bloody { (Color::hex(0x7a3a3e), Color::hex(0xa8565a)) } else { (Color::hex(0x8b929d), Color::hex(0xb9bfc8)) };
    for k in 0..n {
        let mid = (k as f32 + 0.5) / n as f32;
        if mid < from {
            continue;
        }
        let hw = t.half_width * 0.92 * (1.0 - mid);
        let y = t.base.y + step * (k as f32 + 0.5);
        let half = Vec2::new(hw, step * 0.5 + 0.002);
        frame.rect(Vec2::new(t.tip.x, y), half, Vec2::X, 0.0, dark);
        frame.rect(Vec2::new(t.tip.x - hw * 0.5, y), Vec2::new(hw * 0.5, half.y), Vec2::X, 0.0, lit);
    }
}

/// Gun pixels (y down) placed in the world: `anchor` (gun pixels) lands on
/// `origin`, turned by `rot`, `scale` metres per pixel, mirrored top to
/// bottom when `fy` is -1.
#[derive(Clone, Copy)]
struct GunFrame {
    origin: Vec2,
    anchor: Vec2,
    rot: Vec2,
    scale: f32,
    fy: f32,
}

impl GunFrame {
    /// Maps art pixels `from` -> `to` onto world points `a` -> `b`.
    fn between(from: Vec2, to: Vec2, a: Vec2, b: Vec2, fy: f32) -> Option<Self> {
        let art = Vec2::new(to.x - from.x, (from.y - to.y) * fy);
        let world = b - a;
        let (al, wl) = (art.length(), world.length());
        if al < 1e-3 || wl < 1e-4 {
            return None;
        }
        let (u, v) = (art / al, world / wl);
        let rot = Vec2::new(u.x * v.x + u.y * v.y, u.x * v.y - u.y * v.x);
        Some(GunFrame { origin: a, anchor: from, rot, scale: wl / al, fy })
    }

    fn dir(&self, px: Vec2) -> Vec2 {
        let v = Vec2::new(px.x, -px.y * self.fy);
        Vec2::new(v.x * self.rot.x - v.y * self.rot.y, v.x * self.rot.y + v.y * self.rot.x) * self.scale
    }

    fn at(&self, px: Vec2) -> Vec2 {
        self.origin + self.dir(px - self.anchor)
    }

    /// A piece turned `deg` (clockwise) about its `pivot`, which sits at `at`.
    fn piece(&self, frame: &mut Frame, uv: [f32; 4], pivot: Vec2, at: Vec2, deg: f32) {
        let a = deg.to_radians();
        let turn = |v: Vec2| Vec2::new(v.x * a.cos() - v.y * a.sin(), v.x * a.sin() + v.y * a.cos());
        let size = Vec2::new(uv[2], uv[3]);
        let center = self.at(at + turn(size * 0.5 - pivot));
        let x = self.dir(turn(Vec2::X)).normalize_or(Vec2::X);
        let uv = if self.fy < 0.0 { [uv[0], uv[1] + uv[3], uv[2], -uv[3]] } else { uv };
        frame.sprite(center, size * 0.5 * self.scale, x, uv, Color::WHITE);
    }
}

/// How a rigged gun's parts sit right now (degrees, clockwise; `None` for
/// the hook when it's out of the gun).
struct RigPose {
    trigger: f32,
    needle: f32,
    drum: f32,
    prongs: [f32; 2],
    hook_in: bool,
}

impl SkinRig {
    /// The gun's own pieces, and the hook's while it's loaded.
    fn draw_gun(&self, frame: &mut Frame, g: &GunFrame, pose: &RigPose) {
        let hook = Vec2::from(self.rig.hook);
        for (p, uv) in self.rig.pieces.iter().zip(&self.uvs) {
            let (pivot, at) = (Vec2::from(p.pivot), Vec2::from(p.at));
            let hooked = p.on_hook || p.name == "hook";
            if hooked && !pose.hook_in {
                continue;
            }
            let (at, deg) = match p.name {
                "trigger" => (at, pose.trigger),
                "needle" => (at, pose.needle),
                "drum" => (at, pose.drum),
                "prong_up" => (hook + at, pose.prongs[0]),
                "prong_down" => (hook + at, pose.prongs[1]),
                _ => (at, 0.0),
            };
            g.piece(frame, *uv, pivot, at, deg);
        }
    }

    /// The hook on its own (out of the gun): its pivot on `pivot`, its tip
    /// pointing along `dir`.
    fn draw_hook(&self, frame: &mut Frame, pivot: Vec2, dir: Vec2, prongs: [f32; 2], scale: f32) -> GunFrame {
        let hook = Vec2::from(self.rig.hook);
        let fy = if dir.x < 0.0 { -1.0 } else { 1.0 };
        let g = GunFrame::between(hook, hook + Vec2::X, pivot, pivot + dir.normalize_or(Vec2::X) * scale, fy)
            .unwrap_or(GunFrame { origin: pivot, anchor: hook, rot: Vec2::X, scale, fy });
        for (p, uv) in self.rig.pieces.iter().zip(&self.uvs).filter(|(p, _)| p.on_hook || p.name == "hook") {
            let (pivot_px, at) = (Vec2::from(p.pivot), Vec2::from(p.at));
            let (at, deg) = match p.name {
                "prong_up" => (hook + at, prongs[0]),
                "prong_down" => (hook + at, prongs[1]),
                _ => (at, 0.0),
            };
            g.piece(frame, *uv, pivot_px, at, deg);
        }
        g
    }
}

/// One weapon's texture: its rect in the atlas and its grip and tip in its
/// own pixels (y down).
#[derive(Clone, Copy)]
pub struct SkinWeapon {
    pub uv: [f32; 4],
    pub grip: Vec2,
    pub tip: Vec2,
}

impl SkinWeapon {
    /// Draws it with its grip on `grip` and its tip toward `tip`, mirrored top
    /// to bottom when `flip`.
    fn place(&self, frame: &mut Frame, grip: Vec2, tip: Vec2, flip: bool) {
        let fy = if flip { -1.0 } else { 1.0 };
        // the art's grip-to-tip line (y up), and how far it reaches in the world
        let art = Vec2::new(self.tip.x - self.grip.x, (self.grip.y - self.tip.y) * fy);
        let world = tip - grip;
        let (al, wl) = (art.length(), world.length());
        if al < 1e-3 || wl < 1e-4 {
            return;
        }
        let scale = wl / al;
        // turn the art's line onto the world's
        let (a, b) = (art / al, world / wl);
        let rot = Vec2::new(a.x * b.x + a.y * b.y, a.x * b.y - a.y * b.x);
        let turn = |v: Vec2| Vec2::new(v.x * rot.x - v.y * rot.y, v.x * rot.y + v.y * rot.x);
        let size = Vec2::new(self.uv[2], self.uv[3]);
        let mid = Vec2::new(size.x * 0.5 - self.grip.x, (self.grip.y - size.y * 0.5) * fy);
        let uv = if flip { [self.uv[0], self.uv[1] + self.uv[3], self.uv[2], -self.uv[3]] } else { self.uv };
        frame.sprite(grip + turn(mid * scale), size * 0.5 * scale, rot, uv, Color::WHITE);
    }
}

pub use crate::weapon::LimbKind;

/// One body-part texture: its rect in the atlas (x, y, w, h), its proximal
/// joint (`pivot`) and distal joint in the part's own pixels (y down), and
/// its radius in pixels.
#[derive(Clone, Copy)]
pub struct SkinPart {
    pub uv: [f32; 4],
    pub pivot: Vec2,
    pub distal: Vec2,
    pub radius: f32,
}

impl SkinPart {
    /// Draws the part with its pivot at `at`, its distal end toward `dir`,
    /// scaled `sx` across and `sy` along.
    fn place(&self, frame: &mut Frame, at: Vec2, dir: Vec2, sx: f32, sy: f32, tint: Color) {
        let rot = Vec2::new(-dir.y, dir.x);
        let size = Vec2::new(self.uv[2], self.uv[3]);
        let c = size * 0.5 - self.pivot;
        let center = at + rot * c.x * sx + dir * c.y * sy;
        frame.sprite(center, Vec2::new(size.x * 0.5 * sx, size.y * 0.5 * sy), rot, self.uv, tint);
    }

    /// Stretches the part from joint `a` to joint `b`; `radius` is the
    /// hitbox's (used for the thickness unless `art_width`).
    fn span(&self, frame: &mut Frame, a: Vec2, b: Vec2, radius: f32, art_width: bool, tint: Color) {
        let len = (b - a).length();
        let joint = (self.distal - self.pivot).length().max(1.0);
        let sy = len / joint;
        let sx = if art_width { sy } else { radius / self.radius.max(1.0) };
        self.place(frame, a, (b - a).normalize_or(Vec2::NEG_Y), sx, sy, tint);
    }
}

/// A severed arm or leg from the skin's textures: its points run from the
/// hand or foot (0) to the torn end.
fn draw_skinned_limb(frame: &mut Frame, skin: &Skin, world: &World, w: &Weapon, tint: Color) {
    let pt = |i: u8| world.particles[w.particle(i) as usize].pos;
    let r = |k: u32| world.segments[(w.seg_base + k) as usize].radius;
    let kind = w.limb.unwrap_or(match w.def {
        crate::weapon::LEG => LimbKind::Leg,
        crate::weapon::ARM => LimbKind::Arm,
        _ => LimbKind::Forearm,
    });
    match kind {
        LimbKind::Arm | LimbKind::Leg => {
            let (upper, lower) = if kind == LimbKind::Arm { (&skin.upper_arm, &skin.forearm) } else { (&skin.thigh, &skin.shin) };
            upper.span(frame, pt(2), pt(1), r(1), skin.art_width, tint);
            lower.span(frame, pt(1), pt(0), r(0), skin.art_width, tint);
            frame.occluder(Occluder::Segment { a: pt(2), b: pt(1) });
        }
        LimbKind::Forearm => skin.forearm.span(frame, pt(1), pt(0), r(0), skin.art_width, tint),
        LimbKind::Shin => skin.shin.span(frame, pt(1), pt(0), r(0), skin.art_width, tint),
    }
    frame.occluder(Occluder::Segment { a: pt(1), b: pt(0) });
}

/// A fighter's body from the skin's textures, back limbs first.
fn draw_skinned(frame: &mut Frame, skin: &Skin, w: &World, f: &Fighter, col: Color, outline: Color, now: u32) {
    let seg = |part: u32| {
        let sg = w.segments[(f.seg_base + part) as usize];
        (w.particles[sg.a as usize].pos, w.particles[sg.b as usize].pos)
    };
    let order = [
        (S_THIGH_B, &skin.thigh),
        (S_SHIN_B, &skin.shin),
        (S_UPPER_B, &skin.upper_arm),
        (S_FORE_B, &skin.forearm),
        (S_THIGH_A, &skin.thigh),
        (S_SHIN_A, &skin.shin),
        (S_TORSO, &skin.torso),
        (S_UPPER_A, &skin.upper_arm),
        (S_FORE_A, &skin.forearm),
    ];
    let shown = |part: u32| f.limbs_gone & fighter::bit(part) == 0;
    // (radii come from the hitboxes, which the lab may have resized)
    let head_r = w.segments[(f.seg_base + S_HEAD) as usize].radius;
    if skin.outline {
        for (part, _) in order {
            if shown(part) {
                let (a, b) = seg(part);
                let r = w.segments[(f.seg_base + part) as usize].radius;
                frame.capsule(a, b, r + 0.03, outline);
            }
        }
        frame.circle(f.pos(w, HEAD), head_r + 0.03, outline);
    }
    for (part, tex) in order {
        if !shown(part) {
            continue;
        }
        let (a, b) = seg(part);
        if part == S_TORSO && f.spine_out {
            // bare vertebrae, as the capsule fighters show it
            frame.capsule(a, b, 0.035, Color::hex(0xe6dccb));
            for k in 0..7 {
                frame.circle(a.lerp(b, k as f32 / 6.0), 0.055, Color::hex(0xd8ccb8));
            }
        } else {
            let r = w.segments[(f.seg_base + part) as usize].radius;
            tex.span(frame, a, b, r, skin.art_width, col);
            draw_part_wounds(frame, w, f, part, now);
            if !f.has(part) && fighter::sever_plan(part).is_some_and(|(_, ps)| ps[0] == part) {
                frame.circle(a, r * 0.9, Color::hex(0x8a0d0d));
            }
        }
        frame.occluder(Occluder::Segment { a, b });
    }
    // the head hangs off the neck, so its shading turns with it
    if f.head_popped {
        return;
    }
    let head = f.pos(w, HEAD);
    let down = (f.pos(w, NECK) - head).normalize_or(Vec2::NEG_Y);
    let k = head_r / skin.head.radius.max(1.0);
    // flattened by a stomp from above: squeezed top to bottom, spread sideways
    let across = Vec2::new(-down.y, down.x);
    let (sx, sy) = (squash_scale(across, f.head_squash), squash_scale(down, f.head_squash));
    skin.head.place(frame, head, down, k * sx, k * sy, col);
    draw_part_wounds(frame, w, f, S_HEAD, now);
}

/// How a squashed head (flattened by `q`, 0..1, from straight above) scales
/// along `axis`: shorter the more upright the axis, wider the more level.
fn squash_scale(axis: Vec2, q: f32) -> f32 {
    1.0 - q * 0.65 * axis.y * axis.y + q * 0.45 * axis.x * axis.x
}

/// A fighter's expression.
#[derive(Clone, Copy, PartialEq)]
enum Face {
    Neutral,
    /// begging, being finished, dazed or held
    Scared,
    /// gloating after a kill
    Smug,
    /// performing a finisher
    Angry,
    /// just got hit
    Pained,
}

/// Eyes, brow and mouth on a head at `head`, looking along `look` (up the
/// neck) with `side` pointing the way they face. `open` is how far the mouth
/// is open (0..1, from the voice line playing).
#[allow(clippy::too_many_arguments)]
fn draw_face(frame: &mut Frame, head: Vec2, look: Vec2, side: Vec2, face: Face, open: f32, skin: Color, ink: Color, k: f32) {
    // (`k` scales the face to the head: 1 on a head of radius HEAD_R; `rot`
    // keeps the unit direction for rotated shapes)
    let rot = side;
    let (look, side) = (look * k, side * k);
    let eye = head + side * 0.09 + look * 0.04;
    let brow = |frame: &mut Frame, back: f32, front: f32| frame.capsule(eye - side * 0.05 + look * back, eye + side * 0.05 + look * front, 0.016 * k, ink);
    match face {
        Face::Neutral => frame.circle(eye, 0.045 * k, ink),
        Face::Scared => {
            // wide eye, worried brow (raised at the front)
            frame.circle(eye, 0.064 * k, ink);
            frame.circle(eye, 0.05 * k, Color::hex(0xf4f1ea));
            frame.circle(eye + side * 0.014, 0.024 * k, ink);
            brow(frame, 0.095, 0.135);
        }
        Face::Smug => {
            // half-closed eye under a flat, lowered brow
            frame.circle(eye, 0.045 * k, ink);
            frame.rect(eye + look * 0.03, Vec2::new(0.06, 0.03) * k, rot, 0.0, skin);
            frame.capsule(eye - side * 0.05 + look * 0.002, eye + side * 0.05 + look * 0.002, 0.012 * k, ink);
            brow(frame, 0.085, 0.075);
        }
        Face::Angry => {
            frame.circle(eye, 0.045 * k, ink);
            brow(frame, 0.105, 0.05);
        }
        Face::Pained => {
            // squeezed shut: >
            frame.capsule(eye + side * 0.03, eye - side * 0.03 + look * 0.035, 0.013 * k, ink);
            frame.capsule(eye + side * 0.03, eye - side * 0.03 - look * 0.035, 0.013 * k, ink);
        }
    }
    let mouth = head + side * 0.115 - look * 0.085;
    let lips = Color::hex(0x2a0c10);
    if open > 0.06 {
        // talking: the mouth opens with the words
        let h = (0.012 + 0.046 * open) * k;
        frame.rect(mouth - look * (h / k * 0.4), Vec2::new((0.034 + 0.008 * open) * k, h), rot, h.min(0.03 * k), lips);
        return;
    }
    match face {
        Face::Smug => frame.capsule(mouth - side * 0.035 - look * 0.006, mouth + side * 0.03 + look * 0.022, 0.012 * k, ink),
        Face::Scared => frame.circle(mouth - look * 0.01, 0.022 * k, lips),
        Face::Angry | Face::Pained => frame.capsule(mouth - side * 0.035, mouth + side * 0.035, 0.012 * k, ink),
        Face::Neutral => {}
    }
}

/// View height while you're in a blade lock.
pub const CLASH_ZOOM: f32 = 6.5;

pub const CAMERA_HEIGHT: f32 = 15.0;

/// Smoothly follows `target`, clamped so the view stays inside the level.
/// Smoothly moves `cam` toward `target` (clamped to the level). `dt` is real
/// seconds since the last call, so the feel doesn't depend on frame rate.
pub fn follow_camera(cam: &mut Vec2, target: Vec2, s: &GameState, screen: Vec2, snap: bool, dt: f32, height: f32) {
    let (min, max) = s.level_bounds();
    let half = Vec2::new(height * screen.x / screen.y.max(1.0), height) * 0.5;
    let clamp_axis = |c: f32, lo: f32, hi: f32, h: f32| if hi - lo < 2.0 * h { (lo + hi) * 0.5 } else { c.clamp(lo + h, hi - h) };
    let t = Vec2::new(clamp_axis(target.x, min.x, max.x, half.x), clamp_axis(target.y, min.y, max.y, half.y));
    if snap {
        *cam = t;
    } else {
        *cam += (t - *cam) * (1.0 - (-dt * 16.0).exp());
    }
}

/// View height for the two fighters in a finisher, and at the moment it lands.
pub const FINISH_ZOOM: f32 = 4.4;
pub const FINISH_PUNCH: f32 = 3.6;

fn finisher_mid(s: &GameState, x: &crate::sim::Finisher) -> Vec2 {
    let a = s.fighters[x.attacker as usize].pos(&s.world, NECK);
    let v = s.fighters[x.victim as usize].pos(&s.world, PELVIS);
    (a + v) * 0.5 + Vec2::new(0.0, 0.2)
}

/// Where the camera should be and how far zoomed when something cinematic is
/// going on for `local`: their own blade lock or finisher (tight), or someone
/// else's finishing blow nearby in slow motion (drifts over and eases in).
pub fn cinematic(s: &GameState, local: usize) -> Option<(Vec2, f32)> {
    if let Some(c) = s.clash.filter(|c| c.side_of(local).is_some()) {
        return Some((c.point + Vec2::new(0.0, 0.25), CLASH_ZOOM));
    }
    if let Some(x) = s.finishers.iter().find(|x| x.attacker as usize == local || x.victim as usize == local) {
        return Some((finisher_mid(s, x), if s.flash > 0 { FINISH_PUNCH } else { FINISH_ZOOM }));
    }
    if s.slowmo > 0 {
        let me = camera_target(s, Some(local));
        let near = s.finishers.iter().map(|x| finisher_mid(s, x)).find(|p| (*p - me).length() < 14.0);
        if let Some(p) = near {
            return Some((me.lerp(p, 0.65), 9.0));
        }
    }
    None
}

/// Is `local` watching something cinematic close up (for bars and the flash)?
fn in_the_moment(s: &GameState, local: Option<usize>) -> bool {
    local.is_some_and(|l| s.finishers.iter().any(|x| x.attacker as usize == l || x.victim as usize == l) || (s.slowmo > 0 && cinematic(s, l).is_some()))
}

/// Where the camera should look for a given player (or a spectated one).
pub fn camera_target(s: &GameState, local: Option<usize>) -> Vec2 {
    let w = &s.world;
    let pick = local.filter(|&i| s.fighters[i].alive).or_else(|| s.fighters.iter().position(|f| f.alive)).or(local);
    match pick {
        Some(i) => {
            let f = &s.fighters[i];
            // no aim lead here: the aim is measured from the camera, so leading
            // with it makes the arm keep drifting after the mouse stops
            f.pos(w, PELVIS) + Vec2::new(0.0, 1.0)
        }
        None => Vec2::new(0.0, 4.0),
    }
}

/// Particle positions of a state (to interpolate the next one from).
pub fn snapshot(s: &GameState) -> Vec<Vec2> {
    s.world.particles.iter().map(|p| p.pos).collect()
}

/// A copy of `s` with every particle blended from `prev` by `alpha`, for
/// drawing between ticks. Teleports (new round, parked weapons) aren't blended.
pub fn interpolate(s: &GameState, prev: &[Vec2], alpha: f32) -> GameState {
    let mut g = s.clone();
    if prev.len() <= g.world.particles.len() {
        for (p, &q) in g.world.particles.iter_mut().zip(prev) {
            if (p.pos - q).length_squared() < 1.5 * 1.5 {
                p.pos = q.lerp(p.pos, alpha);
            }
        }
    }
    g
}

/// The wounds in one body part, drawn right after the part (so whatever's
/// in front of it covers them): holes, ragged exits, slits and gashes, with
/// a trickle running down from fresh ones on the head and chest, as far as
/// the part goes.
fn draw_part_wounds(frame: &mut Frame, w: &World, f: &Fighter, part: u32, now: u32) {
    use crate::fighter::{WOUND_INSET, WoundKind};
    if !f.has(part) {
        return;
    }
    let blood = Color::hex(0xc81a1a);
    let dark = Color::hex(0x6e0909);
    let core = Color::hex(0x200303);
    let sg = w.segments[(f.seg_base + part) as usize];
    let room = sg.radius * WOUND_INSET;
    let on_part = |p: Vec2| {
        if part == S_HEAD {
            (p - f.pos(w, HEAD)).length() <= room
        } else {
            let (a, b) = (w.particles[sg.a as usize].pos, w.particles[sg.b as usize].pos);
            stickfell_physics::collide::point_segment(p, a, b).0 <= room
        }
    };
    for wd in f.wounds.iter().filter(|wd| wd.part as u32 == part) {
        let (at, run) = f.wound_at(w, wd);
        let r = wd.size;
        let age = now.saturating_sub(wd.born) as f32 / 60.0;
        if matches!(part, S_HEAD | S_TORSO) && !matches!(wd.kind, WoundKind::Cut | WoundKind::Spatter) {
            // runs down (whichever way is down) as it bleeds, stopping at the edge
            let want = (age * 0.1).min(r * 1.6 + 0.04);
            let mut run_down = 0.0;
            while run_down < want && on_part(at - Vec2::new(0.0, run_down + 0.01 + r * 0.35)) {
                run_down += 0.01;
            }
            if run_down > 0.01 {
                frame.capsule(at, at - Vec2::new(0.0, run_down), r * 0.35, blood);
            }
        }
        match wd.kind {
            WoundKind::Hole => {
                frame.circle(at, r, dark);
                frame.circle(at, r * 0.55, core);
            }
            WoundKind::Exit => {
                // torn open: a ragged ring of blood round a dark hole
                for k in 0..5 {
                    let a = k as f32 * 1.2566 + wd.born as f32 * 0.37;
                    let o = Vec2::new(a.cos(), a.sin()) * r * 0.55;
                    frame.circle(at + o, r * (0.4 + 0.067 * ((k * 3) % 4) as f32), blood);
                }
                frame.circle(at, r * 0.75, dark);
                frame.circle(at, r * 0.42, core);
            }
            WoundKind::Stab => {
                frame.capsule(at - run, at + run, r * 0.5, dark);
                frame.capsule(at - run * 0.7, at + run * 0.7, r * 0.22, core);
            }
            WoundKind::Cut => {
                frame.capsule(at - run, at + run, r * 0.45, blood);
                frame.capsule(at - run * 0.85, at + run * 0.85, r * 0.2, dark);
            }
            WoundKind::Spatter => {
                // a splash of someone else's blood: a blot and a few flecks
                frame.circle(at, r * 0.5, blood);
                for k in 0..4 {
                    let a = k as f32 * 1.7 + wd.born as f32 * 0.53;
                    let o = Vec2::new(a.cos(), a.sin()) * r * (0.55 + 0.15 * ((k * 5) % 3) as f32);
                    frame.circle(at + o, r * (0.16 + 0.07 * ((k * 7) % 3) as f32), if k % 2 == 0 { blood } else { dark });
                }
            }
        }
    }
}

/// One layer of the level's art, each piece cropped as placed, its glow
/// image (if it has one) drawn over it to bloom.
fn draw_art(frame: &mut Frame, lv: &LevelDef, layer: ArtLayer) {
    for a in lv.art.iter().filter(|a| a.layer == layer) {
        let Some(p) = crate::character::level_piece(a.piece) else { continue };
        let crop = |uv: [f32; 4]| {
            let [u0, v0, u1, v1] = a.crop;
            [uv[0] + uv[2] * u0, uv[1] + uv[3] * v0, uv[2] * (u1 - u0), uv[3] * (v1 - v0)]
        };
        let half = a.size * 0.5;
        let center = a.min + half;
        frame.sprite(center, half, Vec2::X, crop(p.uv), Color::WHITE);
        if let Some(g) = p.glow {
            frame.sprite_glow(center, half, Vec2::X, crop(g), Color::WHITE, 1.6);
        }
    }
}

fn world_to_screen(cam: &Camera2D, screen: Vec2, p: Vec2) -> Vec2 {
    let k = screen.y / cam.height;
    let d = (p - cam.center) * k;
    Vec2::new(screen.x * 0.5 + d.x, screen.y * 0.5 - d.y)
}

fn draw_weapon(frame: &mut Frame, s: &GameState, w: &Weapon, skin: Option<&Skin>) {
    let world = &s.world;
    let def = w.def();
    let pt = |i: u8| world.particles[w.particle(i) as usize].pos;
    if let (true, Some(skin)) = (w.owner > 0, skin) {
        let tint = Color::hex(fighter_color(s, (w.owner - 1) as usize)).lerp(Color::hex(0x555555), 0.35);
        draw_skinned_limb(frame, skin, world, w, tint);
        // the torn end, still bloody
        frame.circle(pt(def.points.len() as u8 - 1), 0.065, Color::hex(0x8a0d0d));
        return;
    }
    // a weapon drawn from moving pieces
    let rig = skin.and_then(|sk| sk.rigs.iter().find(|r| r.rig.name == def.name));
    if let Some(rig) = rig {
        let noose = s.nooses.iter().find(|n| !n.cut && s.weapons[n.weapon as usize].base == w.base);
        let (g0, g1) = (Vec2::from(rig.rig.grip), Vec2::from(rig.rig.muzzle));
        let fy = if w.flip { -1.0 } else { 1.0 };
        if let Some(g) = GunFrame::between(g0, g1, pt(def.grip), pt(def.muzzle), fy) {
            let out = noose.is_some_and(|n| n.hook != crate::sim::Hook::Loaded);
            // the gauge runs down as the hook goes; a spent gun reads empty
            let spent = match noose.filter(|_| out) {
                Some(n) => (n.hook_t as f32 / 20.0).min(1.0),
                None if w.ammo == 0 => 1.0,
                None => 0.0,
            };
            let pose = RigPose {
                trigger: rig.rig.trigger_pull * (w.fire_cd as f32 / 20.0).min(1.0),
                needle: rig.rig.needle_empty * spent,
                // turned by the rope paid out (clockwise: out)
                drum: noose.map_or(0.0, |n| (n.len / (rig.rig.drum_r / 420.0)).to_degrees()),
                prongs: [0.0, 0.0],
                hook_in: !out && w.ammo > 0,
            };
            rig.draw_gun(frame, &g, &pose);
        }
        for k in 0..def.segs.len() {
            let seg = world.segments[(w.seg_base + k as u32) as usize];
            frame.occluder(Occluder::Segment { a: world.particles[seg.a as usize].pos, b: world.particles[seg.b as usize].pos });
        }
        return;
    }
    // drawn art, when there is some for this weapon
    // (a bundle of throwing knives is drawn as the knife on top of it)
    let art_name = if def.name == "KNIVES" { "KNIFE" } else { def.name };
    let art = skin.and_then(|sk| sk.weapons.iter().find(|(n, _)| *n == art_name)).map(|(_, a)| *a);
    if let Some(art) = art {
        art.place(frame, pt(def.grip), pt(def.aim.1), w.flip);
        for k in 0..def.segs.len() {
            let seg = world.segments[(w.seg_base + k as u32) as usize];
            frame.occluder(Occluder::Segment { a: world.particles[seg.a as usize].pos, b: world.particles[seg.b as usize].pos });
        }
    }
    for (k, sg) in def.segs.iter().enumerate().filter(|_| art.is_none()) {
        let seg = world.segments[(w.seg_base + k as u32) as usize];
        let (a, b) = (world.particles[seg.a as usize].pos, world.particles[seg.b as usize].pos);
        if w.owner > 0 {
            // a severed limb: its owner's colour, outlined like a body
            let base = Color::hex(fighter_color(s, (w.owner - 1) as usize)).lerp(Color::hex(0x555555), 0.35);
            frame.capsule(a, b, sg.r + 0.03, Color::hex(0x0d0e12));
            frame.capsule(a, b, sg.r, base);
            frame.occluder(Occluder::Segment { a, b });
            continue;
        }
        let color = Color::hex(sg.color);
        match (def.name, k) {
            ("SLEDGEHAMMER", 1) => {
                let axis = (pt(3) - pt(1)).normalize_or(Vec2::X);
                frame.rect(pt(3), Vec2::new(0.15, 0.25), axis, 0.04, color);
                frame.occluder(Occluder::Box { center: pt(3), half: Vec2::new(0.15, 0.25), rot: axis });
                continue;
            }
            _ => {}
        }
        if seg.a == seg.b {
            frame.circle(a, sg.r, color);
            frame.occluder(Occluder::Circle { center: a, radius: sg.r * 0.9 });
        } else {
            let sheen = if sg.kind == Kind::Blade || sg.kind == Kind::Spike { 0.15 } else { 0.0 };
            frame.capsule_glow(a, b, sg.r, color, sheen);
            frame.occluder(Occluder::Segment { a, b });
        }
    }
    let axis = (pt(def.aim.1) - pt(def.aim.0)).normalize_or(Vec2::X);
    match def.name {
        "SWORD" if art.is_none() => {
            let g = pt(2);
            frame.capsule(g - axis.perp() * 0.15, g + axis.perp() * 0.15, 0.035, Color::hex(0x9a8550));
        }
        "SPEAR" => {
            frame.circle(pt(3), 0.06, Color::hex(0x9a8550));
        }
        "FLAIL" => {
            let c = pt(5);
            for k in 0..6 {
                let a = k as f32 * std::f32::consts::TAU / 6.0;
                let d = Vec2::new(a.cos(), a.sin());
                frame.capsule(c + d * 0.12, c + d * 0.25, 0.025, Color::hex(0x55585e));
            }
        }
        "FISH" => {
            let head = pt(4);
            let fwd = (pt(4) - pt(3)).normalize_or(Vec2::X);
            frame.circle(head + fwd * 0.03 + fwd.perp() * 0.04, 0.03, Color::WHITE);
            frame.circle(head + fwd * 0.04 + fwd.perp() * 0.04, 0.015, Color::BLACK);
            let tail = pt(0);
            let back = (pt(0) - pt(1)).normalize_or(-Vec2::X);
            for side in [-1.0, 1.0] {
                frame.capsule(tail, tail + back * 0.12 + back.perp() * side * 0.1, 0.03, Color::hex(0x6a93b4));
            }
        }
        "ARM" | "LEG" | "STUMP" => {
            // the torn end, still bloody
            frame.circle(pt(def.points.len() as u8 - 1), 0.065, Color::hex(0x8a0d0d));
        }
        "FRYING PAN" => {
            frame.circle(pt(2), 0.18, Color::hex(0x3d3d42));
        }
        "GRAPPLE GUN" => {
            // a hook in the muzzle, unless it's out on someone's rope
            let out = s.nooses.iter().any(|n| !n.cut && n.hook != crate::sim::Hook::Loaded && s.weapons[n.weapon as usize].base == w.base);
            if !out && w.ammo > 0 {
                draw_hook(frame, pt(2) + axis * 0.06, axis);
            }
        }
        "REVOLVER" | "PISTOL" => draw_gun_extras(frame, w, pt(1), pt(2), axis, art.is_some(), def.name),
        _ => {}
    }
}

/// A revolver's cylinder and hammer (when it isn't drawn from art), and
/// the muzzle flash.
fn draw_gun_extras(frame: &mut Frame, w: &Weapon, p1: Vec2, p2: Vec2, axis: Vec2, art: bool, name: &str) {
    if !art && name == "REVOLVER" {
        let up = axis.perp() * if w.flip { -1.0 } else { 1.0 };
        frame.circle(p1 + axis * 0.06, 0.06, Color::hex(0x2c2f36));
        frame.capsule(p1 - axis * 0.03, p1 - axis * 0.06 + up * 0.06, 0.018, Color::hex(0x2c2f36));
    }
    if w.flash > 0 {
        let m = p2 + axis * 0.06;
        frame.circle_glow(m, 0.09 + 0.03 * w.flash as f32, Color::hex(0xffd27a), 6.0);
        frame.light(Light::point(m, 5.0, Color::hex(0xffc060).scale(1.6)).no_shadows());
    }
}

const ROPE: u32 = 0xc9a066;

/// A grappling hook at `at`, pointing along `dir`: a shank and three claws.
/// A weapon stuck through someone into a wall: only what's outside the wall
/// is drawn (behind their body), bloodied along the blade.
fn draw_pinned(frame: &mut Frame, s: &GameState, w: &Weapon) {
    let world = &s.world;
    for (k, sg) in w.def().segs.iter().enumerate() {
        let seg = world.segments[(w.seg_base + k as u32) as usize];
        let (a, mut b) = (world.particles[seg.a as usize].pos, world.particles[seg.b as usize].pos);
        let sharp = sg.kind == Kind::Blade || sg.kind == Kind::Spike;
        if sharp {
            let len = (b - a).length();
            if let Some(h) = world.raycast(a, (b - a).normalize_or(Vec2::X), len) {
                b = h.point;
            }
        }
        let color = Color::hex(sg.color);
        frame.capsule_glow(a, b, sg.r, color, if sharp { 0.15 } else { 0.0 });
        frame.occluder(Occluder::Segment { a, b });
        if sharp {
            // blood where it went through, thickest at the wall
            for i in 1..5 {
                let t = 0.45 + i as f32 * 0.13;
                frame.circle(a.lerp(b, t.min(1.0)), sg.r * (0.55 + 0.1 * i as f32), Color::hex(0x8a0d0d).alpha(0.85));
            }
        }
    }
}

fn draw_hook(frame: &mut Frame, at: Vec2, dir: Vec2) {
    let steel = Color::hex(0xc8ccd4);
    let back = at - dir * 0.16;
    frame.capsule(back, at, 0.025, Color::hex(0x3c4048));
    frame.capsule(at, at + dir * 0.14, 0.022, steel);
    for side in [-1.0, 1.0] {
        let tip = at + dir * 0.05 + dir.perp() * (side * 0.12) - dir * 0.06;
        frame.capsule(at, tip, 0.02, steel);
    }
}

/// Grapple ropes: from the hook's eye to the noose round a neck (sagging
/// when slack), or the two loose ends once cut; a spare hook carries its
/// rope coiled.
fn draw_nooses(s: &GameState, frame: &mut Frame, skin: Option<&Skin>) {
    let w = &s.world;
    let rope_of = |wp: &Weapon| skin.and_then(|sk| sk.rigs.iter().find(|r| r.rig.name == wp.def().name)).and_then(|r| r.rope);
    let rope = Color::hex(ROPE);
    // the noose round the neck, its knot, and the end running off it (in
    // the gun's own rope where it has one)
    let collar = |frame: &mut Frame, neck: Vec2, tail: Vec2, uv: Option<[f32; 4]>| {
        let band = [neck - Vec2::new(0.085, 0.0), neck + Vec2::new(0.085, 0.0)];
        match uv {
            Some(uv) => draw_rope_tiled(frame, &band, uv, 0.05),
            None => frame.capsule(band[0], band[1], 0.032, rope),
        }
        frame.circle(neck + Vec2::new(0.0, 0.01), 0.04, rope.lerp(Color::BLACK, 0.25));
        match uv {
            Some(uv) => draw_rope_tiled(frame, &[neck, tail], uv, 0.044),
            None => frame.capsule(neck, tail, 0.022, rope),
        }
    };
    for n in &s.nooses {
        let wp = &s.weapons[n.weapon as usize];
        let out = n.hook != crate::sim::Hook::Loaded;
        let top = if out { w.particles[n.anchor as usize].pos } else { w.particles[wp.particle(wp.def().muzzle) as usize].pos };
        let neck = s.fighters[n.victim as usize].pos(w, NECK);
        // where the rope is tied on (the hook's eye, when it's drawn from a rig)
        let mut eye = top;
        if out {
            // the hook, pointing the way it flew (or away from the rope once it's stuck)
            let dir = if n.hook == crate::sim::Hook::Flying {
                n.hook_vel.normalize_or(Vec2::X)
            } else {
                (top - neck).normalize_or(Vec2::Y)
            };
            let rig = skin.and_then(|sk| sk.rigs.iter().find(|r| r.rig.name == wp.def().name));
            match rig {
                Some(rig) => {
                    // claws folded back in flight, open when it bites
                    let fold = match n.hook {
                        crate::sim::Hook::Flying => (n.hook_t as f32 / 6.0).min(1.0),
                        _ => 0.0,
                    };
                    let prongs = rig.rig.prong_fold.map(|d| d * fold);
                    let tip = Vec2::from(rig.rig.tip).x / 420.0;
                    let d = dir.normalize_or(Vec2::X);
                    let g = rig.draw_hook(frame, top - d * tip, d, prongs, 1.0 / 420.0);
                    eye = g.at(Vec2::from(rig.rig.hook) + Vec2::from(rig.rig.rope_eye));
                }
                None => draw_hook(frame, top, dir),
            }
        }
        if n.cut {
            match rope_of(wp) {
                Some(uv) => draw_rope_tiled(frame, &[eye, eye - Vec2::new(0.0, 0.7)], uv, 0.044),
                None => frame.capsule(eye, eye - Vec2::new(0.0, 0.7), 0.022, rope),
            }
            collar(frame, neck, neck - Vec2::new(0.05, 0.35), rope_of(wp));
            continue;
        }
        let mut path = s.noose_path(n);
        if let Some(first) = path.first_mut() {
            *first = eye;
        }
        match rope_of(wp) {
            Some(uv) => draw_rope_tiled(frame, &path, uv, 0.044),
            None => {
                for seg in path.windows(2) {
                    frame.capsule(seg[0], seg[1], 0.022, rope);
                }
            }
        }
        let head = s.fighters[n.victim as usize].pos(w, HEAD);
        collar(frame, neck, head, rope_of(wp));
    }
}

/// A level's solid boxes for the renderer's baked contact shadows: made once
/// per level and kept (the renderer remembers its bake by this `Arc`).
pub fn static_scene(lv: &LevelDef) -> Arc<StaticScene> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static SCENES: OnceLock<Mutex<HashMap<&'static str, Arc<StaticScene>>>> = OnceLock::new();
    let mut scenes = SCENES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    scenes
        .entry(lv.name)
        .or_insert_with(|| {
            let boxes = lv
                .boxes
                .iter()
                .map(|b| {
                    let o = stickfell_physics::Obb::new(b.center, b.half, b.angle);
                    (o.center, o.half, o.rot)
                })
                .collect();
            // (a little past the camera's limits, which the view can show)
            let pad = Vec2::splat(3.0);
            Arc::new(StaticScene { min: lv.min - pad, max: lv.max + pad, boxes })
        })
        .clone()
}

pub fn draw_game(s: &GameState, lv: &LevelDef, frame: &mut Frame, screen: Vec2, cam_center: Vec2, hud: &Hud) {
    let t = frame.time;
    let shake = Vec2::new((t * 91.0).sin(), (t * 77.0).cos()) * s.shake * 0.12;
    let height = if hud.zoom > 0.0 { hud.zoom } else { CAMERA_HEIGHT };
    frame.camera = Camera2D { center: cam_center + shake, height };
    let skin_arc = if hud.capsules { None } else { Some(hud.skin.clone().unwrap_or_else(crate::character::skin)) };
    if let Some(skin) = &skin_arc {
        frame.sprite_atlas = Some(skin.atlas.clone());
    }
    // level art lives in the game's own atlas (not a kit the lab loaded)
    let game_atlas = crate::character::skin().atlas.clone();
    let art = !lv.art.is_empty() && skin_arc.as_ref().is_none_or(|sk| Arc::ptr_eq(&sk.atlas, &game_atlas));
    if art && skin_arc.is_none() {
        frame.sprite_atlas = Some(game_atlas);
    }
    let skin = skin_arc.as_deref();
    frame.statics = Some(static_scene(lv));
    frame.clear = Color::hex(lv.bg);
    frame.ambient = Color::linear(lv.ambient[0], lv.ambient[1], lv.ambient[2], 1.0);
    let w = &s.world;

    if art {
        draw_art(frame, lv, ArtLayer::Back);
    }
    for d in lv.decor.iter().filter(|d| !d.front) {
        frame.rect_glow(d.center, d.half, Vec2::X, d.corner, Color::hex(d.color), d.glow);
    }

    for l in &lv.lights {
        let flick = if l.flicker {
            let p = l.pos.x * 1.7 + l.pos.y;
            0.82 + 0.1 * (t * 11.0 + p).sin() + 0.08 * (t * 23.0 + p * 3.0).sin()
        } else {
            1.0
        };
        let color = Color::hex(l.color).scale(l.intensity * flick);
        let light = match l.spot {
            Some((dir, half)) => Light::spot(l.pos, l.radius * (0.95 + 0.05 * flick), color, dir, half),
            None => Light::point(l.pos, l.radius * (0.95 + 0.05 * flick), color),
        };
        // how big the source is (softer shadows the bigger): a fixture's lamp
        // head; otherwise wider the further it reaches (a glowing pit, the sky)
        let size = if l.fixture { 0.3 } else { (l.radius * 0.03).clamp(0.12, 1.5) };
        frame.light(light.size(size));
        if l.fixture {
            if l.flicker {
                frame.capsule(l.pos - Vec2::new(0.0, 0.5), l.pos - Vec2::new(0.0, 0.1), 0.05, Color::hex(0x3a2a1a));
                frame.circle_glow(l.pos + Vec2::new(0.0, 0.02 * (t * 17.0).sin()), 0.13, Color::hex(0xffb050), 4.0 * flick);
            } else {
                frame.rect(l.pos, Vec2::new(0.35, 0.12), Vec2::X, 0.05, Color::hex(0x23262e));
                frame.circle_glow(l.pos, 0.12, Color::hex(l.color), 3.0);
            }
        }
    }

    // stuck weapons go behind the level so their tips look buried
    for wpn in s.weapons.iter().filter(|w| w.active && w.embedded && !w.pinned) {
        draw_weapon(frame, s, wpn, skin);
    }

    if art {
        draw_art(frame, lv, ArtLayer::Solid);
    }
    for (i, o) in w.statics.iter().enumerate() {
        // (spike beds draw themselves, below)
        if crate::level::is_spike(o.tag) {
            continue;
        }
        if !art {
            let color = Color::hex(lv.boxes.get(i).map_or(0x3a3f4b, |b| b.color));
            frame.rect(o.center, o.half, o.rot, 0.04, color);
            let c = o.corners();
            frame.capsule(c[3], c[2], 0.035, color.lerp(Color::WHITE, 0.22));
        }
        frame.occluder(Occluder::Box { center: o.center, half: o.half, rot: o.rot });
    }

    draw_liquids(s, frame, t);
    draw_trap(s, frame);
    draw_steam(s, frame, t);

    // hazards
    for h in &s.hazards {
        match h.kind {
            HazardKind::Spikes if art => {}
            HazardKind::Spikes => {
                // the teeth as they really are (the same ones that hurt)
                let (c, half) = h.plate();
                frame.rect(c, half, Vec2::X, 0.02, Color::hex(0x3a3a3e));
                for t in h.teeth() {
                    draw_tooth(frame, &t, 0.0, false);
                    frame.occluder(Occluder::Segment { a: t.base, b: t.tip });
                }
            }
            // (drawn from the drops themselves, below)
            HazardKind::Acid | HazardKind::Water | HazardKind::Lava => {}
            HazardKind::Saw => {
                if h.a != h.b {
                    frame.capsule(h.a, h.b, 0.08, Color::hex(0x2a2d33));
                }
                let c = h.center(s.round_frame);
                let r = h.size.x;
                let spin = s.round_frame as f32 * 0.45;
                frame.circle(c, r * 0.9, Color::hex(0x9aa0aa));
                for k in 0..16 {
                    let a = spin + k as f32 * std::f32::consts::TAU / 16.0;
                    let d = Vec2::new(a.cos(), a.sin());
                    frame.capsule(c + d * r * 0.85, c + d * r + d.perp() * 0.12, 0.05, Color::hex(0xc8ccd4));
                }
                frame.circle(c, r * 0.25, Color::hex(0x3a3d44));
                frame.occluder(Occluder::Circle { center: c, radius: r * 0.9 });
            }
        }
    }

    // ropes (the rope tile, from whichever atlas is bound) and wrecking balls
    let rope_uv = match skin {
        Some(sk) => sk.rigs.iter().find_map(|r| r.rope),
        None if art => crate::character::skin().rigs.iter().find_map(|r| r.rope),
        None => None,
    };
    for chain in &s.ropes {
        let path: Vec<Vec2> = chain.iter().map(|&id| w.particles[id as usize].pos).collect();
        match rope_uv {
            Some(uv) => draw_rope_tiled(frame, &path, uv, 0.09),
            None => {
                for pair in path.windows(2) {
                    frame.capsule(pair[0], pair[1], 0.04, Color::hex(0x8a6a40));
                }
            }
        }
    }
    for b in &s.balls {
        for pair in b.chain.windows(2) {
            let (a, p) = (w.particles[pair[0] as usize].pos, w.particles[pair[1] as usize].pos);
            frame.capsule(a, p, 0.06, Color::hex(0x3a3c42));
        }
        let c = w.particles[b.ball() as usize].pos;
        frame.circle(c, b.def.radius, Color::hex(b.def.color));
        frame.circle(c + Vec2::new(-0.25, 0.25) * b.def.radius, b.def.radius * 0.25, Color::hex(b.def.color).lerp(Color::WHITE, 0.25));
        frame.occluder(Occluder::Circle { center: c, radius: b.def.radius * 0.9 });
    }

    for b in &s.bullets {
        let tail = b.pos - b.vel.normalize_or(Vec2::X) * 0.45;
        frame.capsule_glow(tail, b.pos, 0.025, Color::hex(0xffe8a0), 4.0);
    }

    for dcl in &s.decals {
        frame.circle(dcl.pos, dcl.r, Color::hex(0x5a0a0a).alpha(0.9));
    }

    for d in lv.decor.iter().filter(|d| d.front) {
        frame.rect_glow(d.center, d.half, Vec2::X, d.corner, Color::hex(d.color), d.glow);
    }
    if art {
        draw_art(frame, lv, ArtLayer::Front);
    }

    for lamp in &s.lamps {
        let def = &lv.lamps[lamp.def];
        for pair in lamp.chain.windows(2) {
            let (a, b) = (w.particles[pair[0] as usize].pos, w.particles[pair[1] as usize].pos);
            frame.capsule(a, b, 0.025, Color::hex(0x15171c));
        }
        let n = lamp.chain.len();
        let bulb = w.particles[lamp.bulb() as usize].pos;
        let above = w.particles[lamp.chain[n - 2] as usize].pos;
        let dir = (bulb - above).normalize_or(Vec2::NEG_Y);
        let rot = Vec2::new(-dir.y, dir.x).normalize_or(Vec2::X);
        frame.rect(bulb - dir * 0.05, Vec2::new(0.32, 0.12), rot, 0.06, Color::hex(0x2a2d35));
        let lc = Color::hex(def.color);
        frame.circle_glow(bulb + dir * 0.08, 0.11, lc, 4.0);
        frame.light(Light::spot(bulb + dir * 0.1, def.radius, lc.scale(def.intensity), dir, 1.05));
        frame.light(Light::point(bulb + dir * 0.1, 1.6, lc.scale(0.7)).no_shadows());
    }

    for wpn in s.weapons.iter().filter(|w| w.active && w.holder.is_none() && !w.embedded) {
        draw_weapon(frame, s, wpn, skin);
    }
    // a sword pinning someone to a wall: in front of the wall but behind
    // them, so the hilt sticks out of their chest and the blade comes out of
    // their back; cut off where it goes into the wall
    for wpn in s.weapons.iter().filter(|w| w.active && w.pinned) {
        draw_pinned(frame, s, wpn);
    }

    for f in &s.fighters {
        let base = Color::hex(fighter_color(s, f.id as usize));
        let mut col = if f.alive { base } else { base.lerp(Color::hex(0x555555), 0.6) };
        if f.scorch > 0.0 {
            col = col.lerp(Color::hex(0x2a1d16), f.scorch * 0.85);
        }
        if f.hit_flash > 0 {
            col = col.lerp(Color::WHITE, f.hit_flash as f32 / 7.0);
        }
        let outline = Color::hex(0x0d0e12);
        let blood = Color::hex(0x8a0d0d);
        let seg_pts = |part: u32| {
            let sg = w.segments[(f.seg_base + part) as usize];
            (w.particles[sg.a as usize].pos, w.particles[sg.b as usize].pos)
        };
        if let Some(skin) = skin {
            draw_skinned(frame, skin, w, f, col, outline, s.frame);
        } else {
            for pass in 0..2 {
                for part in 1..10u32 {
                    if f.limbs_gone & fighter::bit(part) != 0 {
                        continue; // lying around as a weapon now
                    }
                    let (a, b) = seg_pts(part);
                    if part == S_TORSO && f.spine_out {
                        // bare vertebrae, bloody at the torn end
                        if pass == 1 {
                            frame.capsule(a, b, 0.035, Color::hex(0xe6dccb));
                            for k in 0..7 {
                                let p = a.lerp(b, k as f32 / 6.0);
                                frame.circle(p, 0.055, Color::hex(0xd8ccb8));
                            }
                            frame.circle(b, 0.07, blood);
                        } else {
                            frame.capsule(a, b, 0.07, outline);
                        }
                        continue;
                    }
                    let r = if part == S_TORSO { 0.1 } else { LIMB_R };
                    if pass == 0 {
                        frame.capsule(a, b, r + 0.03, outline);
                    } else {
                        frame.capsule(a, b, r, col);
                        if !f.has(part) && fighter::sever_plan(part).is_some_and(|(_, ps)| ps[0] == part) {
                            frame.circle(a, r * 0.9, blood);
                        }
                        draw_part_wounds(frame, w, f, part, s.frame);
                    }
                    frame.occluder(Occluder::Segment { a, b });
                }
            }
        }
        // stumps on the body
        for part in 2..10u32 {
            if !f.has(part) {
                if let Some((joint, ps)) = fighter::sever_plan(part) {
                    if ps[0] == part {
                        frame.circle(f.pos(w, joint), 0.07, blood);
                    }
                }
            }
        }
        let head = f.pos(w, HEAD);
        // (faces and the rest are sized to the head: 1 on the original HEAD_R)
        let head_r = w.segments[(f.seg_base + S_HEAD) as usize].radius;
        let hk = head_r / HEAD_R;
        if skin.is_none() && !f.head_popped {
            if f.head_squash > 0.0 {
                // flattened: a squat rounded slab instead of a ball
                let half = Vec2::new(squash_scale(Vec2::X, f.head_squash), squash_scale(Vec2::Y, f.head_squash)) * head_r;
                frame.rect(head, half + Vec2::splat(0.03), Vec2::X, half.y + 0.03, outline);
                frame.rect(head, half, Vec2::X, half.y, col);
            } else {
                frame.circle(head, head_r + 0.03, outline);
                frame.circle(head, head_r, col);
            }
            draw_part_wounds(frame, w, f, S_HEAD, s.frame);
        }
        if !f.has(S_HEAD) {
            frame.circle(f.pos(w, NECK), 0.08, blood);
        }
        // alive, the face stays level whatever the body is doing (easier to
        // read); dead, it goes wherever the head flops
        let look = if f.alive || !f.has(S_HEAD) { Vec2::Y } else { (head - f.pos(w, NECK)).normalize_or(Vec2::Y) };
        let side = Vec2::new(look.y, -look.x) * f.facing;
        let eye = head + (side * 0.09 + look * 0.04) * hk;
        let faces = skin.is_none_or(|k| k.faces);
        if f.head_popped {
            // (nothing left to have a face)
        } else if !faces {
            // (a skin without faces: the art's head is left bare)
        } else if f.alive && f.has(S_HEAD) {
            let talk = hud.talk.get(f.id as usize).copied().flatten();
            let victim = s.finishers.iter().any(|x| x.victim == f.id);
            let performing = s.finishers.iter().any(|x| x.attacker == f.id);
            let face = match talk {
                _ if f.hurt > 0 => Face::Pained,
                Some((crate::sfx::Sfx::Plea, _)) => Face::Scared,
                _ if f.burning => Face::Pained,
                Some((crate::sfx::Sfx::Quip, _)) => Face::Smug,
                _ if victim || f.choking || f.dazed > 0 || f.held_by.is_some() => Face::Scared,
                _ if performing => Face::Angry,
                _ if f.hit_flash > 0 || f.down > 0 => Face::Pained,
                _ => Face::Neutral,
            };
            // strung up: gasping
            let gasp = if f.choking {
                0.35 + 0.3 * (t * 9.0 + f.id as f32).sin()
            } else if f.hurt > 0 {
                // a wince: mouth wide open, closing as it passes
                0.8 * f.hurt as f32 / 36.0
            } else {
                0.0
            };
            draw_face(frame, head, look, side, face, talk.map_or(gasp, |t| t.1), col, outline, hk);
        } else if f.alive {
            frame.circle(eye, 0.045 * hk, outline);
        } else {
            // x_x
            for s2 in [-1.0, 1.0] {
                let d = Vec2::new(0.04, 0.04 * s2) * hk;
                frame.capsule(eye - d, eye + d, 0.012 * hk, outline);
            }
        }
        if !f.head_popped {
            frame.occluder(Occluder::Circle { center: head, radius: head_r * 0.8 });
        }
        if f.alive && f.held_by.is_some() && f.down == 0 {
            // the struggle to break free, filling up over their head
            let at = head + Vec2::new(0.0, 0.48);
            let fill = (f.struggle as f32 / 100.0).min(1.0);
            frame.capsule(at - Vec2::new(0.32, 0.0), at + Vec2::new(0.32, 0.0), 0.05, Color::hex(0x0d0e12).alpha(0.8));
            if fill > 0.02 {
                let x0 = at - Vec2::new(0.3, 0.0);
                frame.capsule_glow(x0, x0 + Vec2::new(0.6 * fill, 0.0), 0.032, Color::hex(0xffb23a), 1.5);
            }
        }
        if f.alive && f.dazed > 0 {
            // seeing stars
            for k in 0..3 {
                let a = t * 4.0 + k as f32 * 2.094;
                let p = head + Vec2::new(a.cos() * 0.32, 0.34 + a.sin() * 0.07);
                frame.circle_glow(p, 0.045, Color::hex(0xffe066), 2.5);
            }
        }
        if let Some(wi) = f.weapon {
            if !s.weapons[wi as usize].embedded {
                draw_weapon(frame, s, &s.weapons[wi as usize], skin);
            }
            // the hand on top of the grip
            let hand_r = if skin.is_some() { w.segments[(f.seg_base + S_FORE_A) as usize].radius } else { LIMB_R };
            frame.circle(f.pos(w, HAND_A), hand_r * 1.15, col);
        }
    }

    // spikes run through bodies: the bloody point sticking out of them
    for (fi, f) in s.fighters.iter().enumerate() {
        let Some(&[pb, pt, slot]) = s.spike_pins.get(fi) else { continue };
        if f.impaled == 0 || !w.skewers[slot as usize].enabled {
            continue;
        }
        let (base, tip) = (w.particles[pb as usize].pos, w.particles[pt as usize].pos);
        let tooth = crate::level::Tooth { base, tip, half_width: crate::level::SPIKE_SPACING * 0.5 };
        let sk = w.skewers[slot as usize];
        let at = w.particles[sk.a as usize].pos.lerp(w.particles[sk.b as usize].pos, sk.s);
        let from = ((at.y - base.y) / (tip.y - base.y)).clamp(0.0, 1.0);
        draw_tooth(frame, &tooth, from, true);
    }

    draw_nooses(s, frame, skin);

    let mut spark_sum = Vec2::ZERO;
    let mut n_sparks = 0.0;
    for sp in &s.sparks {
        let k = sp.life as f32 / sp.max_life as f32;
        if sp.steam {
            // a puff, swelling as it fades
            let r = 0.05 + 0.15 * (1.0 - k);
            frame.circle(sp.pos, r, Color::hex(sp.tint).alpha(0.18 * k));
        } else if sp.tint != 0 {
            frame.circle(sp.pos, 0.016 + 0.01 * k, Color::hex(sp.tint).alpha(0.85));
        } else if sp.blood {
            frame.circle(sp.pos, 0.035 + 0.02 * k, Color::hex(0x9a0e0e));
        } else {
            let c = Color::hex(0xff3010).lerp(Color::hex(0xffe08a), k);
            frame.capsule_glow(sp.pos, sp.pos - sp.vel * 0.025, 0.028, c, 5.0 * k);
            spark_sum += sp.pos;
            n_sparks += 1.0;
        }
    }
    if n_sparks > 0.0 {
        frame.light(Light::point(spark_sum / n_sparks, 3.5, Color::hex(0xffa040).scale((n_sparks / 8.0).min(2.0))).no_shadows());
    }

    if !hud.world_only {
        draw_hud(s, lv, frame, screen, hud);
    }
}

/// Skulls over anyone who can be finished, the "F FINISH" prompt when you're
/// close enough, and the finisher's name when it lands.
fn draw_finishers(s: &GameState, frame: &mut Frame, screen: Vec2, hud: &Hud) {
    let k = (screen.y / 720.0).clamp(0.6, 3.0);
    // cinematic bars: full for the pair in a finisher, thin for everyone in slow-mo
    let close = in_the_moment(s, hud.local);
    let bar = if close {
        screen.y * 0.11
    } else if s.slowmo > 0 {
        screen.y * 0.05
    } else {
        0.0
    };
    if bar > 0.0 {
        frame.ui_rect(Vec2::ZERO, Vec2::new(screen.x, bar), 0.0, Color::BLACK.alpha(0.9));
        frame.ui_rect(Vec2::new(0.0, screen.y - bar), Vec2::new(screen.x, bar), 0.0, Color::BLACK.alpha(0.9));
    }
    if close && s.flash > 0 {
        frame.ui_rect(Vec2::ZERO, screen, 0.0, Color::WHITE.alpha(s.flash as f32 / 10.0 * 0.35));
    }
    let cam = frame.camera;
    let t = frame.time;
    let mine = hud.local.and_then(|l| s.finish_target(l));
    for (vi, v) in s.fighters.iter().enumerate() {
        if !s.finishable(vi) {
            continue;
        }
        let p = world_to_screen(&cam, screen, v.pos(&s.world, HEAD) + Vec2::new(0.0, 0.75));
        let r = 11.0 * k * (1.0 + 0.15 * (t * 8.0).sin());
        frame.ui_circle(p, r, Color::hex(0xe8484f));
        frame.ui_rect(Vec2::new(p.x - r * 0.45, p.y + r * 0.55), Vec2::new(r * 0.9, r * 0.6), 2.0 * k, Color::hex(0xe8484f));
        for side in [-1.0, 1.0] {
            frame.ui_circle(Vec2::new(p.x + side * r * 0.38, p.y - r * 0.05), r * 0.24, Color::hex(0x15171c));
        }
        if mine == Some(vi) {
            let text = "F  Finish";
            let tw = Frame::text_width(text, 2.2 * k);
            frame.ui_rect(Vec2::new(p.x - tw * 0.5 - 8.0 * k, p.y - 44.0 * k), Vec2::new(tw + 16.0 * k, 26.0 * k), 5.0 * k, Color::BLACK.alpha(0.7));
            frame.ui_text(Vec2::new(p.x - tw * 0.5, p.y - 38.0 * k), 2.2 * k, text, Color::hex(0xffd75a));
        }
    }
    if let Some((mv, a, left)) = s.finish_banner {
        let alpha = (left as f32 / 20.0).min(1.0);
        let grow = 1.0 + ((100 - left.min(100)) as f32 / 8.0).min(1.0) * 0.3;
        centered_title(frame, screen, screen.y * 0.22, 6.0 * k * grow, mv.title(), Color::hex(fighter_color(s, a as usize)).alpha(alpha));
    }
}

/// The blade-lock duel: letterbox bars, the key to press, and both scores.
fn draw_clash(s: &GameState, frame: &mut Frame, screen: Vec2, hud: &Hud) {
    let k = (screen.y / 720.0).clamp(0.6, 3.0);
    if let Some((w, left)) = s.clash_banner {
        let a = (left as f32 / 20.0).min(1.0);
        let text = if hud.local == Some(w as usize) { "YOU WIN THE CLASH!".to_string() } else { format!("P{} WINS THE CLASH!", w + 1) };
        centered_title(frame, screen, screen.y * 0.2, 5.0 * k, &text, Color::hex(fighter_color(s, w as usize)).alpha(a));
    }
    let Some(c) = s.clash else { return };
    let me = hud.local.and_then(|l| c.side_of(l));
    let names = c.fighters.map(|f| format!("P{}", f + 1));
    if me.is_none() {
        // everyone else just gets a heads-up
        let text = format!("{} vs {}  {} - {}", names[0], names[1], c.score[0], c.score[1]);
        centered_text(frame, screen, screen.y * 0.12, 2.6 * k, &text, Color::hex(0xffd75a));
        return;
    }
    let bar = screen.y * 0.11;
    frame.ui_rect(Vec2::ZERO, Vec2::new(screen.x, bar), 0.0, Color::BLACK.alpha(0.85));
    frame.ui_rect(Vec2::new(0.0, screen.y - bar), Vec2::new(screen.x, bar), 0.0, Color::BLACK.alpha(0.85));
    let t = frame.time;
    // scores: pips on each side, in each fighter's color
    for side in 0..2 {
        let col = Color::hex(fighter_color(s, c.fighters[side] as usize));
        let x0 = if side == 0 { screen.x * 0.28 } else { screen.x * 0.72 };
        let label = if Some(side) == me { "You" } else { names[side].as_str() };
        let tw = Frame::text_width(label, 2.4 * k);
        frame.ui_text(Vec2::new(x0 - tw * 0.5, screen.y - bar + 14.0 * k), 2.4 * k, label, col);
        for i in 0..CLASH_WIN {
            let x = x0 + (i as f32 - (CLASH_WIN as f32 - 1.0) * 0.5) * 30.0 * k;
            let on = i < c.score[side];
            let pulse = if on && c.stage == ClashStage::Point && c.last == Some(side as u8) && i + 1 == c.score[side] { 1.0 + 0.4 * (t * 18.0).sin().abs() } else { 1.0 };
            frame.ui_circle(Vec2::new(x, screen.y - bar * 0.35), 9.0 * k * pulse, if on { col } else { Color::WHITE.alpha(0.2) });
        }
    }
    let cx = screen.x * 0.5;
    let cy = screen.y - bar - 64.0 * k;
    match c.stage {
        ClashStage::Intro => {
            let grow = (c.t as f32 / 12.0).min(1.0);
            centered_title(frame, screen, screen.y * 0.18, (4.0 + 5.0 * grow) * k, "CLASH!", Color::hex(0xffd75a));
            centered_text(frame, screen, cy, 2.4 * k, "Hit the key first", Color::WHITE.alpha(0.85));
        }
        ClashStage::Prompt => {
            let locked_out = me.is_some_and(|m| c.lockout[m] > 0);
            let size = 92.0 * k * (1.0 + 0.06 * (t * 10.0).sin());
            let bg = if locked_out { Color::hex(0x3a3d44) } else { Color::hex(0xffd75a) };
            frame.ui_rect(Vec2::new(cx - size * 0.5, cy - size * 0.5), Vec2::splat(size), 14.0 * k, bg);
            let key = CLASH_KEY_NAMES[c.prompt as usize];
            let ks = 9.0 * k;
            let kw = Frame::text_width(key, ks);
            frame.ui_text(Vec2::new(cx - kw * 0.5, cy - 3.5 * ks), ks, key, if locked_out { Color::hex(0x7a7f8a) } else { Color::hex(0x15171c) });
            if locked_out {
                centered_text(frame, screen, cy + size * 0.6, 2.4 * k, "Wrong key!", Color::hex(0xff6060));
            }
        }
        ClashStage::Point => {
            if let Some(side) = c.last {
                let text = if Some(side as usize) == me { "POINT!".to_string() } else { format!("{} SCORES", names[side as usize]) };
                centered_title(frame, screen, cy - 20.0 * k, 4.0 * k, &text, Color::hex(fighter_color(s, c.fighters[side as usize] as usize)));
            }
        }
    }
}

fn centered_text(frame: &mut Frame, screen: Vec2, y: f32, scale: f32, text: &str, color: Color) {
    centered_in(Font::Body, frame, screen, y, scale, text, color);
}

/// Big announcements (finishers, rounds, the clash) in the title font.
fn centered_title(frame: &mut Frame, screen: Vec2, y: f32, scale: f32, text: &str, color: Color) {
    centered_in(Font::Title, frame, screen, y, scale, text, color);
}

fn centered_in(font: Font, frame: &mut Frame, screen: Vec2, y: f32, scale: f32, text: &str, color: Color) {
    let w = Frame::text_width_in(font, text, scale);
    let x = (screen.x - w) * 0.5;
    frame.ui_text_in(font, Vec2::new(x + scale * 0.6, y + scale * 0.6), scale, text, Color::BLACK.alpha(0.6));
    frame.ui_text_in(font, Vec2::new(x, y), scale, text, color);
}

fn draw_hud(s: &GameState, lv: &LevelDef, frame: &mut Frame, screen: Vec2, hud: &Hud) {
    draw_clash(s, frame, screen, hud);
    draw_finishers(s, frame, screen, hud);
    let k = (screen.y / 720.0).clamp(0.6, 3.0);
    let pad = 16.0 * k;
    // (as wide as there's room for, eight across)
    let n = s.fighters.len().max(1) as f32;
    let panel_w = (200.0 * k).min((screen.x - pad * 2.0 - (n - 1.0) * 10.0 * k) / n);
    let cam = frame.camera;
    for f in &s.fighters {
        let i = f.id as usize;
        let x = pad + i as f32 * (panel_w + 10.0 * k);
        let y = pad;
        let col = Color::hex(fighter_color(s, i));
        frame.ui_rect(Vec2::new(x, y), Vec2::new(panel_w, 46.0 * k), 6.0 * k, Color::BLACK.alpha(0.45));
        frame.ui_rect(Vec2::new(x + 8.0 * k, y + 8.0 * k), Vec2::new(14.0 * k, 14.0 * k), 3.0 * k, col);
        let bot = if i >= s.humans { " AI" } else { "" };
        let name = if hud.local == Some(i) { format!("P{} You", i + 1) } else { format!("P{}{bot}", i + 1) };
        frame.ui_text(Vec2::new(x + 30.0 * k, y + 8.5 * k), 2.0 * k, &name, Color::WHITE);
        for wi in 0..s.scores[i].min(8) {
            frame.ui_circle(Vec2::new(x + panel_w - 14.0 * k - wi as f32 * 13.0 * k, y + 15.0 * k), 4.5 * k, Color::hex(0xffd75a));
        }
        let hp = (f.health / MAX_HEALTH).clamp(0.0, 1.0);
        let bar = Vec2::new(panel_w - 16.0 * k, 8.0 * k);
        frame.ui_rect(Vec2::new(x + 8.0 * k, y + 30.0 * k), bar, 3.0 * k, Color::hex(0x1a1a1a));
        if hp > 0.0 {
            frame.ui_rect(Vec2::new(x + 8.0 * k, y + 30.0 * k), Vec2::new(bar.x * hp, bar.y), 3.0 * k, col);
        }
    }

    // name tags over other fighters + local pickup hints
    for f in &s.fighters {
        if !f.alive {
            continue;
        }
        let p = world_to_screen(&cam, screen, f.pos(&s.world, HEAD) + Vec2::new(0.0, 0.55));
        let col = Color::hex(fighter_color(s, f.id as usize));
        let tag = if hud.local == Some(f.id as usize) {
            "You".to_string()
        } else if f.id as usize >= s.humans {
            format!("P{} AI", f.id + 1)
        } else {
            format!("P{}", f.id + 1)
        };
        let w = Frame::text_width(&tag, 1.6 * k);
        frame.ui_text(Vec2::new(p.x - w * 0.5, p.y), 1.6 * k, &tag, col);
    }
    if let Some(li) = hud.local {
        let f = &s.fighters[li];
        if f.alive && f.arm_a_ok() {
            let hand = f.pos(&s.world, HAND_A);
            let near = s
                .weapons
                .iter()
                .filter(|w| w.active && w.holder.is_none())
                .map(|w| (w.distance_to(&s.world, hand), w))
                .filter(|(d, _)| *d < 1.3)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, w)) = near {
                let p = world_to_screen(&cam, screen, s.world.particles[w.base as usize].pos + Vec2::new(0.0, 0.7));
                let text = format!("E  {}", w.def().label());
                let tw = Frame::text_width(&text, 1.8 * k);
                frame.ui_rect(Vec2::new(p.x - tw * 0.5 - 6.0 * k, p.y - 5.0 * k), Vec2::new(tw + 12.0 * k, 22.0 * k), 4.0 * k, Color::BLACK.alpha(0.55));
                frame.ui_text(Vec2::new(p.x - tw * 0.5, p.y), 1.8 * k, &text, Color::WHITE);
            }
        }
    }

    if s.round_frame < 90 && matches!(s.phase, Phase::Fight) {
        let text = if s.round_frame < 50 { format!("ROUND {}", s.round) } else { "FIGHT!".to_string() };
        centered_title(frame, screen, screen.y * 0.28, 7.0 * k, &text, Color::WHITE);
        centered_text(frame, screen, screen.y * 0.28 + 64.0 * k, 3.0 * k, lv.name, Color::hex(0xb8c0d0));
    }
    if let Phase::RoundOver { winner, .. } = s.phase {
        let (text, col) = match winner {
            Some(w) if s.teams > 0 && hud.local.is_some_and(|me| s.allies(me, w as usize)) => ("YOUR TEAM WINS".to_string(), Color::hex(fighter_color(s, w as usize))),
            Some(w) if s.teams > 0 => (format!("{} WINS", TEAM_NAMES[w as usize % 2]), Color::hex(TEAM_COLORS[w as usize % 2][0])),
            Some(w) if hud.local == Some(w as usize) => ("YOU WIN THE ROUND".to_string(), Color::hex(fighter_color(s, w as usize))),
            Some(w) => (format!("PLAYER {} WINS", w + 1), Color::hex(fighter_color(s, w as usize))),
            None => ("NO SURVIVORS".to_string(), Color::WHITE),
        };
        centered_title(frame, screen, screen.y * 0.3, 6.0 * k, &text, col);
    }
    if let Some((text, col)) = &hud.banner {
        centered_text(frame, screen, screen.y * 0.5, 4.0 * k, text, *col);
    }

    let mut y = screen.y - pad - 10.0 * k;
    for (line, col) in hud.lines.iter().rev() {
        frame.ui_text(Vec2::new(pad, y), 1.6 * k, line, *col);
        y -= 14.0 * k;
    }
    if let Some(m) = hud.mouse {
        // a small, faint ring (just its corners): your arm and weapon already
        // show where you're aiming, so it only stands out once the cursor is
        // past arm's reach (where moving it doesn't move the arm any more)
        let r = 6.0 * k;
        let line = 1.6 * k;
        let arc = 50f32.to_radians();
        let me = hud.local.map(|i| &s.fighters[i]).filter(|f| f.alive);
        let alpha = me.map_or(0.5, |f| {
            let cam = frame.camera;
            let px = screen.y / cam.height;
            let neck = world_to_screen(&cam, screen, f.pos(&s.world, crate::fighter::NECK));
            let reach = crate::fighter::ARM_REACH * f.body.arm_k() * px;
            let over = ((m.distance(neck) - reach) / (0.6 * px)).clamp(0.0, 1.0);
            0.2 + 0.65 * over
        });
        let ring = |frame: &mut Frame, p: Vec2, a: f32| {
            frame.ui_ring_corners(p, r, line + 2.0 * k, arc + 0.08, Color::hex(0x10131a).alpha(a * 0.45));
            frame.ui_ring_corners(p, r, line, arc, Color::WHITE.alpha(a));
        };
        ring(frame, m, alpha);
        // rounds left, as pips under the ring
        let gun = hud.local.and_then(|i| s.fighters[i].weapon).map(|wi| &s.weapons[wi as usize]).filter(|w| w.is_gun() || (w.is_knives() && w.def().ammo > 1));
        if let Some(w) = gun {
            let n = w.def().ammo;
            for i in 0..n {
                let x = m.x + (i as f32 - (n as f32 - 1.0) * 0.5) * 7.0 * k;
                let col = if i < w.ammo { Color::hex(0xffd27a) } else { Color::WHITE.alpha(0.25) };
                frame.ui_circle(Vec2::new(x, m.y + r + 8.0 * k), 2.2 * k, col);
            }
        }
    }
}
