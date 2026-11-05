//! Arena definitions (medium-sized; the camera follows the player).
//! Geometry and spawn points feed the simulation; colors and lights are
//! render-only.

use stickfell_physics::Vec2;

#[derive(Clone, Copy, Debug)]
pub struct BoxDef {
    pub center: Vec2,
    pub half: Vec2,
    pub angle: f32,
    pub color: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct LampDef {
    pub anchor: Vec2,
    pub length: f32,
    pub color: u32,
    pub intensity: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LightDef {
    pub pos: Vec2,
    pub radius: f32,
    pub color: u32,
    pub intensity: f32,
    /// (direction, half angle) for spot lights.
    pub spot: Option<(Vec2, f32)>,
    /// Draw a glowing fixture at the light position.
    pub fixture: bool,
    pub flicker: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DecorDef {
    pub center: Vec2,
    pub half: Vec2,
    pub color: u32,
    pub glow: f32,
    pub corner: f32,
    /// Drawn in front of the level geometry (windows, signs).
    pub front: bool,
}

/// A hanging rope to grab (Shift) and swing on.
#[derive(Clone, Copy, Debug)]
pub struct RopeDef {
    pub anchor: Vec2,
    pub length: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HazardKind {
    /// A bed of spikes (rect `a` +- `size`): a base plate and a row of teeth
    /// (see [`HazardDef::teeth`]), each solid; touching one hurts, landing on
    /// its point far more.
    Spikes,
    /// A spinning blade of radius `size.x`, sliding between `a` and `b`.
    Saw,
    /// Liquids (rect `a` +- `size`, its top the surface): see
    /// [`crate::liquid`] for how each looks and behaves.
    Acid,
    Water,
    Lava,
}

#[derive(Clone, Copy, Debug)]
pub struct HazardDef {
    pub kind: HazardKind,
    pub a: Vec2,
    pub b: Vec2,
    pub size: Vec2,
    /// Saws: how fast they slide along their track (radians of the back-and-forth per frame).
    pub speed: f32,
}

/// A spike's collision box carries this tag (so it isn't drawn, grabbed or
/// stuck into as level geometry).
pub const SPIKE_TAG: u32 = 1;
/// A spike bed's base plate carries this one (an impaled body still lies on it).
pub const SPIKE_PLATE_TAG: u32 = 2;
/// Is this static part of a spike bed?
pub fn is_spike(tag: u32) -> bool {
    tag == SPIKE_TAG || tag == SPIKE_PLATE_TAG
}
/// Teeth along a spike bed, as the kit's spike strip has them (8 to a metre).
pub const SPIKE_SPACING: f32 = 0.125;
/// A tooth's point and the top of the base plate, as shares of the bed's
/// height (measured off the kit's spike strip).
const SPIKE_TIP: f32 = 0.896;
const SPIKE_PLATE: f32 = 0.167;
/// How wide a tooth's solid core is (half, m): things rest on its point.
pub const SPIKE_CORE: f32 = 0.012;

/// One spike: from the middle of its base on the plate up to its point, and
/// half its width at the base (it narrows to nothing at the point).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tooth {
    pub base: Vec2,
    pub tip: Vec2,
    pub half_width: f32,
}

impl Tooth {
    /// Whether a body part of radius `r` at `p` touches this spike, and how
    /// far up it (0 at the base .. 1 at the point).
    pub fn touch(&self, p: Vec2, r: f32) -> Option<f32> {
        let up = self.tip.y - self.base.y;
        if up <= 0.0 || p.y > self.tip.y + r || p.y < self.base.y - r {
            return None;
        }
        let t = ((p.y - self.base.y) / up).clamp(0.0, 1.0);
        ((p.x - self.tip.x).abs() <= self.half_width * (1.0 - t) + r).then_some(t)
    }
}

impl HazardDef {
    /// A spike bed's teeth, evenly along it, the first half a gap in.
    pub fn teeth(&self) -> Vec<Tooth> {
        if self.kind != HazardKind::Spikes {
            return Vec::new();
        }
        let (bottom, h) = (self.a.y - self.size.y, self.size.y * 2.0);
        let n = ((self.size.x * 2.0) / SPIKE_SPACING).round().max(1.0) as usize;
        let left = self.a.x - self.size.x + SPIKE_SPACING * 0.5;
        (0..n)
            .map(|i| {
                let x = left + i as f32 * SPIKE_SPACING;
                Tooth { base: Vec2::new(x, bottom + h * SPIKE_PLATE), tip: Vec2::new(x, bottom + h * SPIKE_TIP), half_width: SPIKE_SPACING * 0.5 }
            })
            .collect()
    }

    /// A spike bed's base plate (centre, half size).
    pub fn plate(&self) -> (Vec2, Vec2) {
        let (bottom, h) = (self.a.y - self.size.y, self.size.y * 2.0);
        (Vec2::new(self.a.x, bottom + h * SPIKE_PLATE * 0.5), Vec2::new(self.size.x, h * SPIKE_PLATE * 0.5))
    }

    /// Where a saw's hub is at `frame` (others never move).
    pub fn center(&self, frame: u32) -> Vec2 {
        if self.a == self.b {
            return self.a;
        }
        let t = 0.5 + 0.5 * stickfell_physics::dmath::sin(frame as f32 * self.speed);
        self.a + (self.b - self.a) * t
    }
}

/// A heavy ball on a chain that keeps itself swinging (wrecking ball, log).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallDef {
    pub anchor: Vec2,
    pub length: f32,
    pub radius: f32,
    pub mass: f32,
    /// Speed it's kept swinging at, at the bottom of the arc.
    pub swing: f32,
    pub color: u32,
}

/// Which pass a piece of level art is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtLayer {
    /// Behind everything (the back wall, windows).
    Back,
    /// The level itself, drawn instead of its boxes.
    Solid,
    /// In front of the fighters (they sink into it).
    Front,
}

/// One placed piece of level art: a kit piece covering `min..min + size`
/// (metres), showing the `crop` part of its texture (u0, v0, u1, v1 as
/// fractions of it, v from the top).
#[derive(Clone, Copy, Debug)]
pub struct ArtDef {
    pub piece: &'static str,
    pub min: Vec2,
    pub size: Vec2,
    pub crop: [f32; 4],
    pub layer: ArtLayer,
}

/// A raft: a plank floating on water, where it's moored (its middle), its
/// half size, and the water it floats on (its bottom and still surface).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RaftDef {
    pub x: f32,
    pub half: Vec2,
    pub sea: (f32, f32),
}

/// What a level gives fighters to fight with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arms {
    /// A few weapons lying about to start with (a few more than there are
    /// fighters), and more dropping in every so often.
    Scattered,
    /// One weapon where each fighter starts, and nothing more.
    OneEach,
    /// Nothing: bare hands only.
    BareHands,
}

#[derive(Clone, Debug)]
pub struct LevelDef {
    pub name: &'static str,
    pub boxes: Vec<BoxDef>,
    pub lamps: Vec<LampDef>,
    pub ropes: Vec<RopeDef>,
    pub hazards: Vec<HazardDef>,
    pub balls: Vec<BallDef>,
    pub lights: Vec<LightDef>,
    pub decor: Vec<DecorDef>,
    /// Drawn level art (from the kit). A level with art draws it instead of
    /// its boxes and hazard shapes.
    pub art: Vec<ArtDef>,
    /// Fighters drop from these points onto whatever is below.
    pub spawns: Vec<Vec2>,
    pub weapon_spawns: Vec<Vec2>,
    /// Camera limits / playable area.
    pub min: Vec2,
    pub max: Vec2,
    pub kill_y: f32,
    /// Practice yard: one of every weapon on the rack, no drops, and the round
    /// only resets when the player or all the dummies are down for good.
    pub training: bool,
    /// What fighters have to fight with (not the practice yard's rack).
    pub arms: Arms,
    /// Rafts floating on the level's water.
    pub rafts: Vec<RaftDef>,
    /// A flood trap (practice only).
    pub trap: Option<TrapDef>,
    pub bg: u32,
    pub ambient: [f32; 3],
}

fn b(cx: f32, cy: f32, hx: f32, hy: f32, color: u32) -> BoxDef {
    BoxDef { center: Vec2::new(cx, cy), half: Vec2::new(hx, hy), angle: 0.0, color }
}

fn tilt(mut bx: BoxDef, angle: f32) -> BoxDef {
    bx.angle = angle;
    bx
}

fn d(cx: f32, cy: f32, hx: f32, hy: f32, color: u32, glow: f32) -> DecorDef {
    DecorDef { center: Vec2::new(cx, cy), half: Vec2::new(hx, hy), color, glow, corner: 0.04, front: false }
}

fn front(mut x: DecorDef) -> DecorDef {
    x.front = true;
    x
}

fn light(x: f32, y: f32, radius: f32, color: u32, intensity: f32) -> LightDef {
    LightDef { pos: Vec2::new(x, y), radius, color, intensity, spot: None, fixture: false, flicker: false }
}

fn lamp(x: f32, y: f32, length: f32, color: u32) -> LampDef {
    LampDef { anchor: Vec2::new(x, y), length, color, intensity: 2.2, radius: 12.0 }
}

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

fn rope(x: f32, y: f32, length: f32) -> RopeDef {
    RopeDef { anchor: v(x, y), length }
}

fn spikes(cx: f32, cy: f32, hx: f32, hy: f32) -> HazardDef {
    HazardDef { kind: HazardKind::Spikes, a: v(cx, cy), b: v(cx, cy), size: v(hx, hy), speed: 0.0 }
}

/// A pool from `x0` to `x1`, from `bottom` up to its surface at `top`.
fn pool(kind: HazardKind, x0: f32, x1: f32, bottom: f32, top: f32) -> HazardDef {
    let (a, size) = (v((x0 + x1) * 0.5, (bottom + top) * 0.5), v((x1 - x0) * 0.5, (top - bottom) * 0.5));
    HazardDef { kind, a, b: a, size, speed: 0.0 }
}

fn saw(a: Vec2, b: Vec2, radius: f32, speed: f32) -> HazardDef {
    HazardDef { kind: HazardKind::Saw, a, b, size: v(radius, radius), speed }
}

/// A lever that opens a hatch in the ceiling and lets a flood of water in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrapDef {
    /// The lever's pivot (on a floor).
    pub lever: Vec2,
    /// The middle of the hatch (on the ceiling's underside), and half its width.
    pub hatch: Vec2,
    pub hatch_half: f32,
    /// How fast the water comes in while the hatch is open (drops a
    /// second), and how much liquid the level can hold before it stops
    /// (drops in all, pools included: what the liquid can be simulated at).
    pub pour_rate: f32,
    pub max_drops: u32,
}

impl TrapDef {
    /// Where the lever's handle ends, `pulled` 0 (up and back) .. 1 (over).
    pub fn handle(&self, pulled: f32) -> Vec2 {
        let ang = 2.2 - 1.4 * pulled.clamp(0.0, 1.0);
        self.lever + stickfell_physics::dmath::dir(ang) * 0.55
    }
}

/// The arenas online matches rotate through (the training yard comes after them).
pub const ARENAS: usize = 6;
/// Index of the eight-player arena in `levels()`.
#[cfg_attr(not(test), allow(dead_code))]
pub const GAUNTLET: usize = 3;
/// Index of the rope course arena in `levels()`.
#[cfg_attr(not(test), allow(dead_code))]
pub const ROPES: usize = 4;
/// Index of the open water arena in `levels()`.
#[cfg_attr(not(test), allow(dead_code))]
pub const SEA: usize = 5;
/// Index of the practice yard in `levels()`.
pub const TRAINING: usize = 6;

/// The arena after `now` in the rotation with a spawn for each of
/// `fighters` (the big ones only, with more than four).
pub fn next_arena(now: usize, fighters: usize) -> usize {
    let lv = levels();
    (1..=ARENAS).map(|k| (now + k) % ARENAS).find(|&i| lv[i].spawns.len() >= fighters).unwrap_or(0)
}

/// The arena a match starts in (from its seed).
pub fn first_arena(seed: u32, fighters: usize) -> usize {
    next_arena((seed as usize % ARENAS + ARENAS - 1) % ARENAS, fighters)
}

pub fn levels() -> Vec<LevelDef> {
    vec![foundry(), rooftops(), keep(), gauntlet(), rope_course(), open_water(), training_yard()]
}

const STONE: u32 = 0x3a3f4b;
const STEEL: u32 = 0x4b5262;
const BRICK: u32 = 0x5a3a35;
const CASTLE: u32 = 0x4a4740;
const WOODC: u32 = 0x5e4630;

fn platform(x: f32, y: f32, half_w: f32, color: u32) -> BoxDef {
    b(x, y, half_w, 0.22, color)
}

fn foundry() -> LevelDef {
    let mut boxes = vec![
        b(-30.6, 7.0, 0.6, 15.0, STONE),
        b(30.6, 7.0, 0.6, 15.0, STONE),
        b(0.0, 21.6, 31.0, 0.6, STONE),
        b(-22.0, -1.0, 8.0, 1.0, STONE),
        b(0.0, -1.0, 10.0, 1.0, STONE),
        b(22.0, -1.0, 8.0, 1.0, STONE),
        // crates
        b(-25.0, 0.5, 0.5, 0.5, WOODC),
        b(16.5, 0.6, 0.6, 0.6, WOODC),
        b(-3.0, 0.45, 0.45, 0.45, WOODC),
    ];
    // four tiers, 3.6 m apart: room to stand up and swing between them
    for (x, y, w) in [
        (-20.0, 3.0, 3.5),
        (20.0, 3.0, 3.5),
        (-5.5, 3.0, 2.2),
        (5.5, 3.0, 2.2),
        (-12.5, 6.6, 2.8),
        (12.5, 6.6, 2.8),
        (0.0, 6.6, 3.0),
        (-26.0, 6.6, 2.5),
        (26.0, 6.6, 2.5),
        (-19.0, 10.2, 3.0),
        (19.0, 10.2, 3.0),
        (-6.0, 10.2, 2.4),
        (6.0, 10.2, 2.4),
        (-12.5, 13.8, 2.8),
        (12.5, 13.8, 2.8),
        (0.0, 13.8, 2.6),
        (-26.0, 13.8, 2.5),
        (26.0, 13.8, 2.5),
    ] {
        boxes.push(platform(x, y, w, STEEL));
    }
    let mut decor = Vec::new();
    for i in -4..=4 {
        decor.push(d(i as f32 * 6.5, 9.5, 0.35, 12.5, 0x2c303a, 0.0));
    }
    decor.push(d(0.0, 18.4, 31.0, 0.25, 0x2c303a, 0.0));
    decor.push(d(0.0, 11.9, 31.0, 0.12, 0x2a2e37, 0.0));
    for x in [-12.0, 12.0] {
        decor.push(d(x, -2.4, 1.9, 0.6, 0xff6a1a, 1.4));
    }
    LevelDef {
        name: "Foundry",
        boxes,
        lamps: vec![
            lamp(-24.0, 21.0, 3.0, 0xffc070),
            lamp(-12.0, 21.0, 3.4, 0xffc070),
            lamp(0.0, 21.0, 2.6, 0xffc070),
            lamp(12.0, 21.0, 3.4, 0xffc070),
            lamp(24.0, 21.0, 3.0, 0xffc070),
        ],
        ropes: vec![rope(-6.0, 18.4, 6.5), rope(6.0, 18.4, 6.5)],
        hazards: vec![],
        balls: vec![],
        lights: vec![
            light(-12.0, -1.8, 8.0, 0xff5a10, 1.8),
            light(12.0, -1.8, 8.0, 0xff5a10, 1.8),
            LightDef { spot: Some((v(0.6, -0.8), 0.5)), fixture: true, ..light(-29.4, 18.5, 18.0, 0x6aa0ff, 0.9) },
            LightDef { spot: Some((v(-0.6, -0.8), 0.5)), fixture: true, ..light(29.4, 18.5, 18.0, 0x6aa0ff, 0.9) },
        ],
        decor,
        spawns: vec![v(-22.0, 1.5), v(22.0, 1.5), v(-5.5, 4.6), v(5.5, 4.6)],
        weapon_spawns: vec![
            v(-26.0, 1.5),
            v(26.0, 1.5),
            v(0.0, 1.5),
            v(-12.5, 7.6),
            v(12.5, 7.6),
            v(0.0, 7.6),
            v(-19.0, 11.2),
            v(19.0, 11.2),
            v(-6.0, 11.2),
            v(6.0, 11.2),
            v(0.0, 14.8),
        ],
        min: v(-31.0, -5.0),
        max: v(31.0, 22.0),
        kill_y: -6.0,
        art: vec![],
        training: false,
        arms: Arms::Scattered,
        rafts: vec![],
        bg: 0x5a6070,
        trap: None,
        ambient: [0.15, 0.16, 0.21],
    }
}

fn rooftops() -> LevelDef {
    let boxes = vec![
        b(-26.5, -4.0, 5.5, 6.0, BRICK),
        b(-14.0, -3.0, 4.0, 7.2, 0x4e3b3a),
        b(0.0, -4.0, 7.0, 5.5, BRICK),
        b(14.0, -3.0, 4.0, 7.8, 0x4e3b3a),
        b(26.5, -4.0, 5.5, 6.4, BRICK),
        tilt(b(26.5, 3.0, 4.5, 0.25, STEEL), -0.18),
        b(2.5, 2.5, 1.5, 1.0, 0x3d3a3a),  // rooftop shack
        b(-28.0, 3.0, 0.5, 1.0, BRICK),   // chimney
        b(16.5, 5.3, 0.6, 0.5, 0x555b66), // AC unit
        platform(-2.5, 5.6, 2.2, STEEL),  // water tower deck
        platform(2.5, 8.2, 1.4, STEEL),   // high perch
        platform(-14.0, 6.4, 2.4, STEEL), // billboard walkway
        platform(14.0, 7.0, 2.0, STEEL),  // antenna platform
    ];
    let mut decor = Vec::new();
    // far skyline
    let heights = [9.0, 13.0, 7.0, 15.0, 10.0, 12.0, 8.0, 14.0, 11.0, 9.0, 13.0, 7.5, 12.0, 10.0, 15.0, 9.0, 11.0];
    for (i, h) in heights.iter().enumerate() {
        let x = -40.0 + i as f32 * 5.0;
        decor.push(d(x, -8.0 + h * 0.5, 2.2, h * 0.5, 0x1d2233, 0.0));
        if i % 3 != 1 {
            decor.push(d(x - 0.6, -8.0 + h * 0.7, 0.2, 0.25, 0xffd27a, 1.0));
        }
    }
    decor.push(DecorDef { corner: 1.4, ..d(-20.0, 16.0, 1.4, 1.4, 0xdfe8ff, 1.3) }); // moon
    // water tower tank + legs
    decor.push(d(-2.5, 8.3, 1.7, 2.4, 0x3b3330, 0.0));
    decor.push(d(-3.9, 7.2, 0.08, 1.5, 0x262222, 0.0));
    decor.push(d(-1.1, 7.2, 0.08, 1.5, 0x262222, 0.0));
    // antenna
    decor.push(d(14.0, 11.0, 0.06, 4.0, 0x2a2d35, 0.0));
    decor.push(DecorDef { corner: 0.12, ..d(14.0, 15.0, 0.12, 0.12, 0xff3030, 2.5) });
    // lit windows on the buildings (in front of the walls)
    for (bx, top, w) in [(-26.5, 2.0, 5.5), (-14.0, 4.2, 4.0), (0.0, 1.5, 7.0), (14.0, 4.8, 4.0), (26.5, 2.4, 5.5)] {
        let mut x = bx - w + 1.0;
        let mut k = 0;
        while x < bx + w - 0.8 {
            for row in 0..3 {
                let lit = (k * 7 + row * 3) % 5 < 2;
                let c = if lit { 0xffcf70 } else { 0x1a1d26 };
                decor.push(front(d(x, top - 1.4 - row as f32 * 1.6, 0.3, 0.45, c, if lit { 0.9 } else { 0.0 })));
            }
            x += 1.6;
            k += 1;
        }
    }
    // neon signs
    decor.push(front(d(-14.0, 2.4, 2.0, 0.35, 0xff2e6a, 2.6)));
    decor.push(front(d(14.0, 2.9, 1.8, 0.35, 0x2ee6ff, 2.6)));
    LevelDef {
        name: "Rooftops",
        boxes,
        lamps: vec![],
        // a cable slung from the antenna to swing across the big gap
        ropes: vec![rope(7.0, 13.0, 6.5)],
        hazards: vec![],
        balls: vec![],
        lights: vec![
            light(-30.0, 30.0, 75.0, 0x8fb0ff, 1.1),
            light(-14.0, 1.6, 7.0, 0xff2e6a, 1.8),
            light(14.0, 2.1, 7.0, 0x2ee6ff, 1.6),
            LightDef { spot: Some((v(0.15, -1.0), 0.75)), fixture: true, ..light(-24.0, 7.5, 10.0, 0xffd08a, 1.8) },
            LightDef { spot: Some((v(-0.15, -1.0), 0.75)), fixture: true, ..light(24.0, 8.5, 10.0, 0xffd08a, 1.8) },
            LightDef { spot: Some((v(0.0, -1.0), 0.9)), fixture: true, ..light(-2.5, 10.9, 8.0, 0xfff0c8, 1.4) },
        ],
        decor,
        spawns: vec![v(-26.0, 4.0), v(26.0, 6.0), v(-14.0, 6.0), v(14.0, 7.0)],
        weapon_spawns: vec![
            v(-29.0, 4.0),
            v(-23.0, 4.0),
            v(-14.0, 6.0),
            v(-14.0, 8.5),
            v(-4.0, 3.0),
            v(5.0, 3.0),
            v(-2.5, 7.0),
            v(2.5, 10.0),
            v(12.0, 7.0),
            v(14.0, 9.0),
            v(24.0, 6.0),
            v(29.0, 6.0),
        ],
        min: v(-33.0, -6.5),
        max: v(33.0, 19.0),
        kill_y: -7.0,
        art: vec![],
        training: false,
        arms: Arms::Scattered,
        rafts: vec![],
        bg: 0x1a2030,
        trap: None,
        ambient: [0.09, 0.10, 0.16],
    }
}

fn keep() -> LevelDef {
    let boxes = vec![
        b(0.0, -4.0, 22.0, 4.0, CASTLE),  // courtyard
        b(-28.0, -0.4, 2.0, 7.6, CASTLE), // outer towers
        b(28.0, -0.4, 2.0, 7.6, CASTLE),
        b(-19.0, 2.5, 3.0, 2.5, CASTLE), // ramparts
        b(19.0, 2.5, 3.0, 2.5, CASTLE),
        platform(-24.0, 4.5, 2.2, WOODC), // bridges over the moat
        platform(24.0, 4.5, 2.2, WOODC),
        platform(-13.5, 3.2, 1.6, WOODC), // scaffolds (head height to walk under)
        platform(13.5, 3.2, 1.6, WOODC),
        platform(-8.0, 4.4, 1.4, WOODC),
        platform(8.0, 4.4, 1.4, WOODC),
        b(-3.4, 3.0, 0.6, 3.0, CASTLE), // gatehouse
        b(3.4, 3.0, 0.6, 3.0, CASTLE),
        b(0.0, 6.5, 4.0, 0.5, CASTLE),
        platform(-9.0, 8.2, 2.2, WOODC), // walkways
        platform(9.0, 8.2, 2.2, WOODC),
        platform(0.0, 11.0, 3.2, CASTLE), // keep top
        b(-10.0, 0.5, 0.5, 0.5, WOODC),  // crates
        b(11.0, 0.45, 0.45, 0.45, WOODC),
    ];
    let mut decor = vec![
        d(0.0, 7.0, 22.0, 7.0, 0x2a2826, 0.0), // back wall
        d(-24.0, -4.0, 2.0, 3.0, 0x14151a, 0.0), // moat walls
        d(24.0, -4.0, 2.0, 3.0, 0x14151a, 0.0),
        d(0.0, 16.0, 4.5, 4.0, 0x262422, 0.0), // keep tower
    ];
    for i in 0..11 {
        decor.push(d(-20.0 + i as f32 * 4.0, 14.4, 0.9, 0.5, 0x2a2826, 0.0)); // crenellations
    }

    for x in [-11.0, 11.0] {
        decor.push(d(x, 11.5, 0.7, 1.6, 0x7a1f1f, 0.0)); // banners
    }
    decor.push(DecorDef { corner: 1.1, ..d(16.0, 17.0, 1.1, 1.1, 0xe8e4d0, 1.2) }); // moon
    let torch = |x: f32, y: f32| LightDef { fixture: true, flicker: true, ..light(x, y, 8.5, 0xff9a40, 1.9) };
    LevelDef {
        name: "The Keep",
        boxes,
        lamps: vec![],
        ropes: vec![rope(-9.0, 15.0, 4.5), rope(9.0, 15.0, 4.5)],
        hazards: vec![spikes(-24.0, -5.4, 1.9, 0.35), spikes(24.0, -5.4, 1.9, 0.35)],
        balls: vec![],
        lights: vec![
            light(20.0, 28.0, 60.0, 0x8090c0, 0.55),
            torch(-16.3, 4.2),
            torch(16.3, 4.2),
            torch(-2.6, 4.6),
            torch(2.6, 4.6),
            torch(-28.0, 8.0),
            torch(28.0, 8.0),
            torch(-10.0, 2.0),
            torch(10.0, 2.0),
            torch(0.0, 12.4),
        ],
        decor,
        spawns: vec![v(-10.0, 1.5), v(10.0, 1.5), v(-19.0, 6.0), v(19.0, 6.0)],
        weapon_spawns: vec![
            v(-6.0, 1.5),
            v(6.0, 1.5),
            v(0.0, 1.5),
            v(-13.5, 4.3),
            v(13.5, 4.3),
            v(-19.0, 6.0),
            v(19.0, 6.0),
            v(0.0, 8.0),
            v(-9.0, 9.3),
            v(9.0, 9.3),
            v(0.0, 12.2),
            v(-28.0, 8.2),
            v(28.0, 8.2),
        ],
        min: v(-31.0, -6.0),
        max: v(31.0, 21.0),
        kill_y: -8.0,
        art: vec![],
        training: false,
        arms: Arms::Scattered,
        rafts: vec![],
        bg: 0x161a26,
        trap: None,
        ambient: [0.15, 0.14, 0.17],
    }
}

/// The Gauntlet's pools: (left, right, bottom, surface) of each.
pub const GAUNTLET_POOL: (f32, f32, f32, f32) = (-32.0, -28.0, -3.35, -0.35);
pub const GAUNTLET_LAVA_A: (f32, f32, f32, f32) = (-26.5, -23.0, -1.75, -0.45);
pub const GAUNTLET_LAVA_B: (f32, f32, f32, f32) = (21.0, 28.0, -1.7, -0.5);
pub const GAUNTLET_WELL: (f32, f32, f32, f32) = (31.0, 34.0, -3.5, -0.6);

/// An arena for up to eight (88 x 24 m), with every hazard in it, each
/// more than one way. From the left:
///
/// - a steel tower with the flood lever on top, climbed by wall-jumping up
///   a chimney beside it (the hatch is over the open floor further in);
/// - a deep pool to dive into off a ledge, a wall, and a lava pit with a
///   long rope hung over it (the flood boils off on it);
/// - open floor with crates, and a low platform a light log swings across;
/// - a trench of spikes, with a short rope over it;
/// - the stage in the middle, a heavy wrecking ball swinging through it;
/// - a shallow trough of acid to wade or hop across on two stones;
/// - a saw rising and falling through a gap in a walkway, and the floor
///   under it;
/// - a wide lava pit with a bridge a saw runs back and forth along;
/// - a deep, narrow well of acid with a rope over it;
/// - a ledge with spikes along its middle over a saw sunk in the floor;
/// - a second steel tower.
///
/// Walkways run along above (3.5 m apart, a double jump), the highest
/// under the ceiling, and lamps hang from it.
fn gauntlet() -> LevelDef {
    let r = |x0: f32, y0: f32, x1: f32, y1: f32, color: u32| b((x0 + x1) * 0.5, (y0 + y1) * 0.5, (x1 - x0) * 0.5, (y1 - y0) * 0.5, color);
    // (a walkway: 0.44 m thick, its top at `top`)
    let walk = |x0: f32, x1: f32, top: f32| platform((x0 + x1) * 0.5, top - 0.22, (x1 - x0) * 0.5, WOODC);
    let (wet, lava_a, lava_b, well) = (GAUNTLET_POOL, GAUNTLET_LAVA_A, GAUNTLET_LAVA_B, GAUNTLET_WELL);
    let base = -8.0;
    let pit = 0x34383f;
    let boxes = vec![
        // walls and ceiling
        r(-45.0, base, -44.0, 17.0, STONE),
        r(44.0, base, 45.0, 17.0, STONE),
        r(-44.0, 16.0, 44.0, 17.0, STONE),
        // the lever's tower, and the column making a chimney beside it
        // (3 m across, open underneath to walk in)
        r(-44.0, base, -40.0, 8.0, STEEL),
        r(-37.0, 3.0, -36.0, 8.0, STEEL),
        r(-40.0, base, wet.0, 0.0, STONE),
        // the pool, the wall, the first lava pit, and the dive ledge
        r(wet.0, base, wet.1, wet.2, pit),
        r(wet.1, base, lava_a.0, 0.0, STONE),
        r(lava_a.0, base, lava_a.1, lava_a.2, pit),
        walk(-32.5, -29.0, 4.0),
        // open floor, crates, the low platform and the log's beam above it
        r(lava_a.1, base, -11.0, 0.0, STONE),
        r(-14.0, 0.0, -13.0, 1.0, WOODC),
        r(-13.0, 0.0, -12.5, 0.5, WOODC),
        walk(-19.5, -14.5, 3.5),
        walk(-20.0, -13.0, 11.0),
        // the spike trench and the walkway over it
        r(-11.0, base, -6.0, -2.5, pit),
        walk(-11.0, -6.0, 7.5),
        // the stage, and the wrecking ball's beam
        r(-6.0, base, 6.0, 0.0, STONE),
        r(-3.5, 0.0, 3.5, 1.0, STEEL),
        walk(-3.0, 3.0, 12.0),
        // the acid trough, its two stones, and the walkway over it
        r(6.0, base, 13.0, -1.2, pit),
        r(8.0, -1.2, 9.0, 0.5, STONE),
        r(10.0, -1.2, 11.0, 0.5, STONE),
        walk(6.5, 12.5, 7.5),
        // the saw's shaft between two walkways, the long one up top
        r(13.0, base, lava_b.0, 0.0, STONE),
        walk(13.0, 15.5, 4.0),
        walk(17.5, lava_b.0, 4.0),
        walk(15.0, 26.0, 11.0),
        // the second lava pit and its bridge
        r(lava_b.0, base, lava_b.1, lava_b.2, pit),
        walk(22.5, 26.5, 2.5),
        // the acid well, the walkway by it
        r(lava_b.1, base, well.0, 0.0, STONE),
        r(well.0, base, well.1, well.2, pit),
        walk(27.5, 31.0, 7.5),
        // the spiked ledge over the sunk saw, and the last tower
        r(well.1, base, 40.0, 0.0, STONE),
        walk(35.0, 39.5, 3.5),
        r(40.0, base, 44.0, 7.0, STEEL),
    ];
    // the back wall: plain, with faint uprights and a rail (nothing to
    // take the eye off the fighters)
    let mut decor = vec![d(0.0, 4.0, 44.0, 12.0, 0x2a2e38, 0.0)];
    for i in -5..=5 {
        decor.push(d(i as f32 * 8.0, 4.0, 0.25, 12.0, 0x262a33, 0.0));
    }
    decor.push(d(0.0, 9.0, 44.0, 0.1, 0x262a33, 0.0));
    // the sunk saw's slot: the floor in front of its lower half
    decor.push(front(d(37.0, -0.6, 1.0, 0.6, STONE, 0.0)));
    // a stripe on each tower
    decor.push(front(d(-42.0, 6.0, 2.0, 0.12, 0xd8a530, 0.0)));
    decor.push(front(d(42.0, 5.0, 2.0, 0.12, 0xd8a530, 0.0)));
    let lamp_at = |x: f32, len: f32| LampDef { intensity: 1.6, radius: 10.0, ..lamp(x, 16.0, len, 0xffd8a0) };
    LevelDef {
        name: "Gauntlet",
        boxes,
        lamps: vec![lamp_at(-36.5, 3.0), lamp_at(-9.0, 2.5), lamp_at(9.5, 2.5), lamp_at(36.0, 3.0)],
        // a long rope over the first lava pit, a short one over the
        // spikes, one up to the high walkway, one over the acid well
        ropes: vec![rope(-24.5, 16.0, 11.0), rope(-8.5, 7.06, 3.5), rope(13.75, 16.0, 6.0), rope(33.0, 16.0, 8.0)],
        hazards: vec![
            pool(HazardKind::Water, wet.0, wet.1, wet.2, wet.3),
            pool(HazardKind::Lava, lava_a.0, lava_a.1, lava_a.2, lava_a.3),
            spikes(-8.5, -2.5 + 0.1875, 2.5, 0.1875),
            // (the trough in three, round its stones)
            pool(HazardKind::Acid, 6.0, 8.0, -1.2, -0.45),
            pool(HazardKind::Acid, 9.0, 10.0, -1.2, -0.45),
            pool(HazardKind::Acid, 11.0, 13.0, -1.2, -0.45),
            saw(v(16.5, 1.0), v(16.5, 6.5), 0.8, 0.03),
            pool(HazardKind::Lava, lava_b.0, lava_b.1, lava_b.2, lava_b.3),
            saw(v(22.9, 3.2), v(26.1, 3.2), 0.6, 0.02),
            pool(HazardKind::Acid, well.0, well.1, well.2, well.3),
            spikes(37.0, 3.5 + 0.1875, 1.0, 0.1875),
            saw(v(37.0, -0.1), v(37.0, -0.1), 0.9, 0.0),
        ],
        // a light log over the low platform, a heavy ball through the stage
        balls: vec![
            BallDef { anchor: v(-16.5, 10.56), length: 5.2, radius: 0.45, mass: 8.0, swing: 5.0, color: 0x6b4a2e },
            BallDef { anchor: v(0.0, 11.56), length: 8.6, radius: 0.75, mass: 40.0, swing: 7.0, color: 0x767c88 },
        ],
        lights: vec![
            light(0.0, 15.0, 70.0, 0xe8ecff, 0.45),
            light(-24.5, 0.5, 7.0, 0xff5a10, 1.5),
            light(24.5, 0.5, 8.0, 0xff5a10, 1.6),
            LightDef { spot: Some((v(0.5, -0.85), 0.6)), fixture: true, ..light(-43.4, 14.5, 22.0, 0xfff4e0, 0.9) },
            LightDef { spot: Some((v(-0.5, -0.85), 0.6)), fixture: true, ..light(43.4, 14.5, 22.0, 0xfff4e0, 0.9) },
        ],
        decor,
        art: vec![],
        // (in turn, so the first few are spread out; in teams, one team
        // fills them from the left end and the other from the right)
        spawns: vec![v(-35.0, 1.5), v(19.5, 1.5), v(-8.5, 9.0), v(29.5, 9.0), v(-17.0, 1.5), v(42.0, 8.5), v(9.5, 9.0), v(-42.0, 9.5)],
        weapon_spawns: vec![
            v(-38.5, 0.5),
            v(-31.0, 4.5),
            v(-42.5, 8.5),
            v(-21.0, 0.5),
            v(-17.0, 4.0),
            v(-16.5, 11.5),
            v(-8.5, 8.0),
            v(-4.5, 0.5),
            v(4.5, 0.5),
            v(0.0, 1.5),
            v(9.5, 8.0),
            v(14.0, 4.5),
            v(19.0, 4.5),
            v(20.0, 11.5),
            v(24.5, 3.0),
            v(29.5, 0.5),
            v(29.5, 8.0),
            v(35.5, 4.0),
            v(42.0, 7.5),
        ],
        min: v(-45.0, base),
        max: v(45.0, 17.0),
        kill_y: -12.0,
        training: false,
        arms: Arms::Scattered,
        rafts: vec![],
        bg: 0x1d2028,
        // the flood: the lever on the left tower, the hatch over the open
        // floor. Online, a rollback runs up to 8 steps in one tick, so it's
        // a flash flood, not the yard's: measured with eight AI fighters
        // (`sim::feel::measure_gauntlet_step`), a step is 2.3 ms dry (2535
        // drops in the pools), 3.2-3.6 ms (worst 4.8) at this cap; at the
        // yard's 15000 it was 10 ms
        // (water boiling off on the lava pours back in while it's open)
        trap: Some(TrapDef { lever: v(-42.0, 8.0), hatch: v(-20.0, 16.0), hatch_half: 1.0, pour_rate: 600.0, max_drops: 3500 }),
        ambient: [0.2, 0.21, 0.26],
    }
}

/// A flat, well-lit practice yard: a weapon rack, a chimney to wall-jump up,
/// a ledge, a rope, three harmless dummies, and (off to the left, from the
/// top of the chimney) a rope course across to a finish platform.
/// The practice rope course, left from the top of the chimney: (x, length)
/// of each rope (hung from the ceiling), getting further apart.
pub const COURSE: [(f32, f32); 4] = [(-29.0, 7.0), (-34.5, 7.5), (-40.5, 7.0), (-46.5, 6.2)];
/// Top centre of the platform the course ends on.
pub const COURSE_FINISH: Vec2 = Vec2::new(-52.0, 10.2);

/// The practice yard's pool (water) and pit (lava), let into its floor:
/// (left, right, bottom, surface).
pub const YARD_POOL: (f32, f32, f32, f32) = (-20.6, -15.6, -2.5, -0.35);
pub const YARD_LAVA: (f32, f32, f32, f32) = (14.4, 17.6, -2.0, -0.45);

fn training_yard() -> LevelDef {
    let (p, l) = (YARD_POOL, YARD_LAVA);
    let floor = |x0: f32, x1: f32, top: f32| b((x0 + x1) * 0.5, (top - 3.0) * 0.5, (x1 - x0) * 0.5, (top + 3.0) * 0.5, STONE);
    let boxes = vec![
        floor(-56.0, p.0, 0.0),
        floor(p.0, p.1, p.2),
        floor(p.1, l.0, 0.0),
        floor(l.0, l.1, l.2),
        floor(l.1, 30.0, 0.0),
        b(-56.6, 8.0, 0.6, 10.0, STONE),
        b(30.6, 8.0, 0.6, 10.0, STONE),
        b(-13.0, 18.6, 44.2, 0.6, STONE),
        // wall-jump chimney
        b(-25.5, 5.0, 0.5, 5.0, STEEL),
        b(-21.5, 5.0, 0.5, 5.0, STEEL),
        platform(-23.5, 10.2, 2.5, STEEL),
        // ledge to grab and climb
        b(22.0, 1.2, 4.0, 1.2, WOODC),
        platform(20.0, 6.0, 3.0, STEEL),
        // the rope course's finish
        platform(COURSE_FINISH.x, COURSE_FINISH.y, 3.0, STEEL),
    ];
    let mut decor = vec![d(-13.0, 8.0, 43.0, 10.0, 0x2c3140, 0.0)];
    for i in -13..=7 {
        decor.push(d(i as f32 * 4.0, 8.0, 0.2, 10.0, 0x272b38, 0.0));
    }
    decor.push(d(0.0, 1.6, 10.2, 0.08, 0x5e4630, 0.0)); // weapon rack rail
    decor.push(d(-10.2, 0.8, 0.12, 0.8, 0x5e4630, 0.0));
    decor.push(d(10.2, 0.8, 0.12, 0.8, 0x5e4630, 0.0));
    // start and finish markers for the rope course
    decor.push(front(d(-23.5, 10.5, 2.4, 0.06, 0x7fd08a, 0.8)));
    decor.push(front(d(COURSE_FINISH.x, COURSE_FINISH.y + 0.3, 2.9, 0.06, 0xf0c85a, 0.9)));
    let mut ropes = vec![rope(12.0, 18.0, 7.0)];
    ropes.extend(COURSE.iter().map(|&(x, len)| rope(x, 18.0, len)));
    LevelDef {
        name: "Training Yard",
        boxes,
        lamps: vec![lamp(-12.0, 18.0, 2.4, 0xfff0d0), lamp(0.0, 18.0, 2.4, 0xfff0d0), lamp(12.0, 18.0, 2.4, 0xfff0d0)],
        ropes,
        // a fall off the rope course lands on spikes; a pool to swim in,
        // and a pit of lava
        hazards: vec![
            spikes(-38.0, 0.1875, 11.0, 0.1875),
            pool(HazardKind::Water, p.0, p.1, p.2, p.3),
            pool(HazardKind::Lava, l.0, l.1, l.2, l.3),
        ],
        balls: vec![],
        lights: vec![
            light(0.0, 30.0, 60.0, 0xd8e4ff, 1.2),
            LightDef { spot: Some((v(0.5, -0.85), 0.6)), fixture: true, ..light(-55.4, 16.0, 22.0, 0xfff4e0, 1.0) },
            // over the rope course
            LightDef { spot: Some((v(0.0, -1.0), 0.9)), fixture: true, ..light(-32.0, 17.7, 20.0, 0xfff4e0, 1.1) },
            LightDef { spot: Some((v(0.0, -1.0), 0.9)), fixture: true, ..light(-43.5, 17.7, 20.0, 0xfff4e0, 1.1) },
            light(-40.0, 12.0, 18.0, 0xd8e4ff, 0.6),
            LightDef { spot: Some((v(-0.5, -0.85), 0.6)), fixture: true, ..light(29.4, 16.0, 22.0, 0xfff4e0, 1.0) },
        ],
        decor,
        spawns: vec![v(-12.0, 1.5), v(4.0, 1.5), v(8.0, 1.5), v(12.0, 1.5)],
        // the rack: one of each weapon, laid out on the floor
        weapon_spawns: (0..10).map(|k| v(-9.45 + k as f32 * 2.1, 0.4)).collect(),
        min: v(-57.0, -3.0),
        max: v(31.0, 19.0),
        kill_y: -10.0,
        art: vec![],
        training: true,
        arms: Arms::Scattered,
        rafts: vec![],
        bg: 0x3a4152,
        // up on the chimney, where the rope course starts: a lever that
        // opens a hatch in the ceiling over the yard and floods it (the
        // water can reach the 43 m of floor right of the chimney: 51 m from
        // its wall to the yard's, less the 8 m ledge block, which stands
        // taller than the flood)
        // (15000 drops in all, measured: the yard right of the chimney under
        // 4.7 m of water after 25 s pouring, a step 12.7 ms with the dummies
        // about, a quarter of the sim thread's 16.7 ms spare; 18000 is 5.2 m
        // at 13.8-15.5 ms, too near it; to the ceiling would be 103000)
        trap: Some(TrapDef { lever: v(-21.6, 10.42), hatch: v(-6.0, 18.0), hatch_half: 1.0, pour_rate: 600.0, max_drops: 15000 }),
        ambient: [0.3, 0.31, 0.34],
    }
}

/// The rope course's platforms: how many, the middle of the first, how far
/// apart, half their width, and their tops; and the two ropes in each gap
/// between them, hung from the ceiling: how far in from each platform's
/// edge, and how long. (Spaced like the practice course, which is crossed
/// in `practice_rope_course_can_be_crossed`: 5.5-6 m between ropes, the
/// first 3 m out from the platform, ropes 6.2-7.5 m long off an 18 m
/// ceiling with the platform tops at 10.4.)
pub const ROPE_PLATFORMS: usize = 8;
pub const ROPE_FIRST: f32 = -59.5;
pub const ROPE_PITCH: f32 = 17.0;
pub const ROPE_HALF: f32 = 3.0;
pub const ROPE_TOP: f32 = 10.42;
pub const ROPE_IN: f32 = 2.75;
pub const ROPE_LEN: f32 = 7.0;
const ROPE_CEILING: f32 = 18.0;

/// Where platform `k` of the rope course is (the middle of its top).
pub fn rope_platform(k: usize) -> Vec2 {
    v(ROPE_FIRST + k as f32 * ROPE_PITCH, ROPE_TOP)
}

/// The rope course: eight platforms high up in a row over a drop nobody
/// comes back from, one for each fighter with a weapon on it, and nothing
/// between them but ropes hung from the ceiling, two in each gap.
fn rope_course() -> LevelDef {
    let n = ROPE_PLATFORMS;
    let left = rope_platform(0).x - ROPE_HALF - 4.0;
    let right = rope_platform(n - 1).x + ROPE_HALF + 4.0;
    let mut boxes = vec![
        // the ceiling the ropes hang from
        b(0.0, ROPE_CEILING + 0.5, right, 0.5, STONE),
    ];
    let mut ropes = Vec::new();
    for k in 0..n {
        let c = rope_platform(k);
        boxes.push(platform(c.x, c.y - 0.22, ROPE_HALF, STEEL));
        if k + 1 < n {
            let (a, z) = (c.x + ROPE_HALF + ROPE_IN, rope_platform(k + 1).x - ROPE_HALF - ROPE_IN);
            ropes.push(rope(a, ROPE_CEILING, ROPE_LEN));
            ropes.push(rope(z, ROPE_CEILING, ROPE_LEN));
        }
    }
    // the back wall: plain and dark, darker still down in the drop
    let mut decor = vec![d(0.0, 8.0, right, 11.0, 0x232733, 0.0), d(0.0, -1.0, right, 4.0, 0x15171d, 0.0)];
    for k in 0..n {
        let c = rope_platform(k);
        // a strut under each platform, down into the dark
        decor.push(d(c.x, c.y - 4.0, 0.25, 3.8, 0x2b303c, 0.0));
        // a stripe along its front
        decor.push(front(d(c.x, c.y - 0.3, ROPE_HALF - 0.1, 0.06, 0xd8a530, 0.0)));
    }
    // (in turn from the middle out, so a few fighters start a gap or two
    // apart; in teams, one team fills them from the left and the other the right)
    let order = [3usize, 4, 2, 5, 1, 6, 0, 7];
    let spawns: Vec<Vec2> = order.iter().map(|&k| rope_platform(k) + v(0.0, 1.6)).collect();
    let mut lights = vec![light(0.0, 24.0, 90.0, 0xd8e0ff, 0.55)];
    for k in (0..n).step_by(2) {
        let x = (rope_platform(k).x + rope_platform(k + 1).x) * 0.5;
        lights.push(LightDef { spot: Some((v(0.0, -1.0), 0.9)), fixture: true, ..light(x, ROPE_CEILING - 0.3, 22.0, 0xfff4e0, 0.9) });
    }
    LevelDef {
        name: "Rope Course",
        boxes,
        lamps: vec![],
        ropes,
        hazards: vec![],
        balls: vec![],
        lights,
        decor,
        art: vec![],
        weapon_spawns: spawns.clone(),
        spawns,
        min: v(left, -5.0),
        max: v(right, ROPE_CEILING + 1.0),
        kill_y: -3.0,
        training: false,
        arms: Arms::OneEach,
        rafts: vec![],
        bg: 0x161922,
        trap: None,
        ambient: [0.2, 0.21, 0.26],
    }
}

/// Open water's sea: (left, right, bottom, surface). As big as the sim can
/// keep up with online: 56 x 2.2 m measured 8388 drops at 2.44 ms a step
/// still (`measure_ocean_cost`; 60 x 2.5 m jumped to 5.19 ms), against the
/// Gauntlet's 2.28 ms with eight fighters.
pub const SEA_WATER: (f32, f32, f32, f32) = (-28.0, 28.0, -2.2, 0.0);
/// Where the rafts are moored, and their half size (2.4 x 0.3 m).
pub const SEA_RAFTS: [f32; 6] = [-20.0, -12.0, -4.0, 4.0, 12.0, 20.0];
pub const RAFT_HALF: Vec2 = Vec2::new(1.2, 0.15);

/// Open water: a sea between two rocky ledges, small rafts bobbing on it,
/// and no weapons at all: bare hands, on the rafts or in the water.
fn open_water() -> LevelDef {
    let (x0, x1, bottom, surface) = SEA_WATER;
    let r = |x0: f32, y0: f32, x1: f32, y1: f32, color: u32| b((x0 + x1) * 0.5, (y0 + y1) * 0.5, (x1 - x0) * 0.5, (y1 - y0) * 0.5, color);
    let rock = 0x3d3a36;
    let sand = 0x4a4436;
    let boxes = vec![
        // the seabed, and a rocky ledge at either end to climb out onto
        r(x0, -6.0, x1, bottom, sand),
        r(x0 - 6.0, -6.0, x0, 1.2, rock),
        r(x1, -6.0, x1 + 6.0, 1.2, rock),
        // cliffs behind them
        r(x0 - 8.0, -6.0, x0 - 6.0, 12.0, rock),
        r(x1 + 6.0, -6.0, x1 + 8.0, 12.0, rock),
    ];
    let sea = (bottom, surface);
    let rafts: Vec<RaftDef> = SEA_RAFTS.iter().map(|&x| RaftDef { x, half: RAFT_HALF, sea }).collect();
    // the sky: plain, a little lighter toward the horizon; far headlands
    let decor = vec![
        d(0.0, 6.0, x1 + 8.0, 6.0, 0x1f2a3b, 0.0),
        d(0.0, 1.0, x1 + 8.0, 1.0, 0x283547, 0.0),
        d(-14.0, 0.6, 7.0, 0.6, 0x232e3d, 0.0),
        d(17.0, 0.4, 5.0, 0.4, 0x232e3d, 0.0),
    ];
    // (on the rafts from the middle out, then the ledges)
    let spawns = vec![v(-4.0, 1.6), v(4.0, 1.6), v(-12.0, 1.6), v(12.0, 1.6), v(-20.0, 1.6), v(20.0, 1.6), v(x0 - 3.0, 2.8), v(x1 + 3.0, 2.8)];
    LevelDef {
        name: "Open Water",
        boxes,
        lamps: vec![],
        ropes: vec![],
        hazards: vec![pool(HazardKind::Water, x0, x1, bottom, surface)],
        balls: vec![],
        lights: vec![light(-10.0, 30.0, 80.0, 0xdfe8ff, 0.9), light(18.0, 14.0, 30.0, 0xffe2b8, 0.4)],
        decor,
        art: vec![],
        weapon_spawns: spawns.clone(),
        spawns,
        min: v(x0 - 7.0, -5.0),
        max: v(x1 + 7.0, 12.0),
        kill_y: -9.0,
        training: false,
        arms: Arms::BareHands,
        rafts,
        bg: 0x1a2230,
        trap: None,
        ambient: [0.3, 0.32, 0.38],
    }
}
