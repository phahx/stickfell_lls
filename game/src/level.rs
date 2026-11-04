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
    /// A bed of spikes (rect `a` +- `size`): hurts on touch, more on a hard landing.
    Spikes,
    /// A spinning blade of radius `size.x`, sliding between `a` and `b`.
    Saw,
    /// A vat of acid (rect `a` +- `size`): burns and slows.
    Acid,
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

impl HazardDef {
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

fn acid(cx: f32, cy: f32, hx: f32, hy: f32) -> HazardDef {
    HazardDef { kind: HazardKind::Acid, a: v(cx, cy), b: v(cx, cy), size: v(hx, hy), speed: 0.0 }
}

#[allow(dead_code)] // no current map has saws
fn saw(a: Vec2, b: Vec2, radius: f32, speed: f32) -> HazardDef {
    HazardDef { kind: HazardKind::Saw, a, b, size: v(radius, radius), speed }
}

/// The arenas online matches rotate through (the training yard comes after them).
pub const ARENAS: usize = 4;
/// Index of the practice yard in `levels()`.
pub const TRAINING: usize = 4;
/// Index of the art test level (a practice yard built from the level kit).
pub const ART_TEST: usize = 5;

pub fn levels() -> Vec<LevelDef> {
    vec![foundry(), rooftops(), keep(), brickworks(), training_yard(), art_test()]
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
        bg: 0x5a6070,
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
        bg: 0x1a2030,
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
        bg: 0x161a26,
        ambient: [0.15, 0.14, 0.17],
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

fn training_yard() -> LevelDef {
    let boxes = vec![
        b(-13.0, -1.0, 43.0, 1.0, STONE),
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
        hazards: vec![],
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
        bg: 0x3a4152,
        ambient: [0.3, 0.31, 0.34],
    }
}

/// One of the kit's 9-slice sets: 1 m fill tiles, 0.25 m edges and corners.
struct NineSlice {
    fill: &'static str,
    alt: &'static str,
    top: &'static str,
    bottom: &'static str,
    left: &'static str,
    right: &'static str,
    tl: &'static str,
    tr: &'static str,
    bl: &'static str,
    br: &'static str,
}

const STONE_SET: NineSlice = NineSlice {
    fill: "stone_fill",
    alt: "stone_fill_alt",
    top: "stone_edge_top",
    bottom: "stone_edge_bottom",
    left: "stone_edge_left",
    right: "stone_edge_right",
    tl: "stone_corner_tl",
    tr: "stone_corner_tr",
    bl: "stone_corner_bl",
    br: "stone_corner_br",
};

const STEEL_SET: NineSlice = NineSlice {
    fill: "steel_fill",
    alt: "steel_fill_alt",
    top: "steel_edge_top",
    bottom: "steel_edge_bottom",
    left: "steel_edge_left",
    right: "steel_edge_right",
    tl: "steel_corner_tl",
    tr: "steel_corner_tr",
    bl: "steel_corner_bl",
    br: "steel_corner_br",
};

/// Lays out level art from the kit's pieces, in metres (the kit is 256 px
/// per metre). Tiles repeat from the bottom-left; the last row and column
/// are cut short rather than squashed.
struct Kit {
    art: Vec<ArtDef>,
    layer: ArtLayer,
}

impl Kit {
    fn put(&mut self, piece: &'static str, min: Vec2, size: Vec2, crop: [f32; 4]) {
        self.art.push(ArtDef { piece, min, size, crop, layer: self.layer });
    }

    fn piece(&mut self, piece: &'static str, min: Vec2, size: Vec2) {
        self.put(piece, min, size, [0.0, 0.0, 1.0, 1.0]);
    }

    /// `tile` repeated from x0 to x1 with its bottom at `y`.
    fn row(&mut self, piece: &'static str, x0: f32, x1: f32, y: f32, tile: Vec2) {
        let mut x = x0;
        while x < x1 - 1e-4 {
            let w = (x1 - x).min(tile.x);
            self.put(piece, v(x, y), v(w, tile.y), [0.0, 0.0, w / tile.x, 1.0]);
            x += tile.x;
        }
    }

    /// `tile` repeated up from y0 to y1 with its left side at `x`.
    fn column(&mut self, piece: &'static str, x: f32, y0: f32, y1: f32, tile: Vec2) {
        let mut y = y0;
        while y < y1 - 1e-4 {
            let h = (y1 - y).min(tile.y);
            self.put(piece, v(x, y), v(tile.x, h), [0.0, 1.0 - h / tile.y, 1.0, 1.0]);
            y += tile.y;
        }
    }

    /// Square tiles over min..max, an `alt` one now and then to break up
    /// the repeats.
    fn fill(&mut self, piece: &'static str, alt: &'static str, min: Vec2, max: Vec2, tile: f32) {
        let (mut j, mut y) = (0, min.y);
        while y < max.y - 1e-4 {
            let h = (max.y - y).min(tile);
            let (mut i, mut x) = (0, min.x);
            while x < max.x - 1e-4 {
                let w = (max.x - x).min(tile);
                let pick = if (i * 7 + j * 3) % 5 == 2 { alt } else { piece };
                self.put(pick, v(x, y), v(w, h), [0.0, 1.0 - h / tile, w / tile, 1.0]);
                x += tile;
                i += 1;
            }
            y += tile;
            j += 1;
        }
    }

    /// A solid block over min..max from a 9-slice set.
    fn block(&mut self, set: &NineSlice, min: Vec2, max: Vec2) {
        let e = 0.25;
        let c = v(e, e);
        self.piece(set.bl, min, c);
        self.piece(set.br, v(max.x - e, min.y), c);
        self.piece(set.tl, v(min.x, max.y - e), c);
        self.piece(set.tr, v(max.x - e, max.y - e), c);
        self.row(set.bottom, min.x + e, max.x - e, min.y, v(1.0, e));
        self.row(set.top, min.x + e, max.x - e, max.y - e, v(1.0, e));
        self.column(set.left, min.x, min.y + e, max.y - e, v(e, 1.0));
        self.column(set.right, max.x - e, min.y + e, max.y - e, v(e, 1.0));
        self.fill(set.fill, set.alt, min + c, max - c, 1.0);
    }

    /// A 0.5 m thick wooden ledge from x0 to x1 with its top at `top`.
    fn ledge(&mut self, x0: f32, x1: f32, top: f32) {
        let y = top - 0.5;
        self.piece("platform_cap_left", v(x0, y), v(0.25, 0.5));
        self.row("platform_tile", x0 + 0.25, x1 - 0.25, y, v(1.0, 0.5));
        self.piece("platform_cap_right", v(x1 - 0.25, y), v(0.25, 0.5));
    }

    /// A stone column (0.625 m wide) centred on `cx`, from y0 up to y1.
    fn pillar(&mut self, cx: f32, y0: f32, y1: f32) {
        let x = cx - 0.3125;
        self.piece("pillar_base", v(x, y0), v(0.625, 0.25));
        self.column("pillar_tile", x, y0 + 0.25, y1 - 0.25, v(0.625, 1.0));
        self.piece("pillar_cap", v(x, y1 - 0.25), v(0.625, 0.25));
    }
}

/// A practice yard built from the HD level kit, for trying the art in game:
/// steel towers at either end, stone floors either side of a sunken acid
/// pit with a ledge over it, columns, crates and a brick back wall.
fn art_test() -> LevelDef {
    // (a box from its corners)
    let r = |x0: f32, y0: f32, x1: f32, y1: f32, color: u32| b((x0 + x1) * 0.5, (y0 + y1) * 0.5, (x1 - x0) * 0.5, (y1 - y0) * 0.5, color);
    let boxes = vec![
        r(-15.0, -4.5, -14.0, 12.5, STONE),
        r(14.0, -4.5, 15.0, 12.5, STONE),
        r(-14.0, -4.5, -11.0, 4.0, STEEL),
        r(11.0, -4.5, 14.0, 4.0, STEEL),
        r(-11.0, -4.5, -2.5, 0.0, STONE),
        r(2.5, -4.5, 11.0, 0.0, STONE),
        r(-2.5, -4.5, 2.5, -3.0, STONE),
        // the ledge over the pit, and one off each tower
        r(-1.75, 2.0, 1.75, 2.5, WOODC),
        r(-11.0, 3.5, -7.0, 4.0, WOODC),
        r(7.0, 3.5, 11.0, 4.0, WOODC),
        // columns and crates
        r(-5.3125, 0.0, -4.6875, 3.0, STONE),
        r(4.6875, 0.0, 5.3125, 3.0, STONE),
        r(-9.5, 0.0, -8.5, 1.0, WOODC),
        r(-9.25, 1.0, -8.75, 1.5, WOODC),
        r(6.5, 0.0, 7.5, 1.0, WOODC),
    ];

    let mut k = Kit { art: Vec::new(), layer: ArtLayer::Back };
    // the back wall: 2 m brick tiles, with lit windows in three of them
    let (mut y, mut j) = (-4.0f32, 0);
    while y < 12.5 {
        let mut x = -15.0f32;
        while x < 15.0 {
            let window = j == 5 && [-9.0, -1.0, 7.0].contains(&x);
            k.piece(if window { "bg_window" } else { "bg_brick" }, v(x, y), v(2.0, 2.0));
            x += 2.0;
        }
        y += 2.0;
        j += 1;
    }
    k.piece("bg_support", v(-4.5, 0.0), v(2.0, 2.0));
    k.piece("bg_support", v(2.5, 0.0), v(2.0, 2.0));

    k.layer = ArtLayer::Solid;
    k.block(&STONE_SET, v(-15.0, -4.5), v(-14.0, 12.5));
    k.block(&STONE_SET, v(14.0, -4.5), v(15.0, 12.5));
    k.block(&STEEL_SET, v(-14.0, -4.5), v(-11.0, 4.0));
    k.block(&STEEL_SET, v(11.0, -4.5), v(14.0, 4.0));
    k.row("trim_hazard", -14.0, -11.0, 2.0, v(1.0, 0.25));
    k.row("trim_hazard", 11.0, 14.0, 2.0, v(1.0, 0.25));
    k.block(&STONE_SET, v(-11.0, -4.5), v(-2.5, 0.0));
    k.block(&STONE_SET, v(2.5, -4.5), v(11.0, 0.0));
    k.block(&STONE_SET, v(-2.5, -4.5), v(2.5, -3.0));
    k.ledge(-1.75, 1.75, 2.5);
    k.ledge(-11.0, -7.0, 4.0);
    k.ledge(7.0, 11.0, 4.0);
    k.row("railing_tile", -11.0, -7.0, 4.0, v(1.0, 0.25));
    k.row("railing_tile", 7.0, 11.0, 4.0, v(1.0, 0.25));
    k.pillar(-5.0, 0.0, 3.0);
    k.pillar(5.0, 0.0, 3.0);
    k.piece("crate_1m", v(-9.5, 0.0), v(1.0, 1.0));
    k.piece("crate_05m", v(-9.25, 1.0), v(0.5, 0.5));
    k.piece("crate_1m", v(6.5, 0.0), v(1.0, 1.0));
    // (the barrel's art stands 14 px up from the bottom of its canvas)
    k.piece("barrel", v(-7.7, -14.0 / 256.0), v(0.75, 1.0));
    k.row("spike_strip", 9.0, 11.0, 0.0, v(1.0, 0.375));

    // acid in front, so whoever falls in sinks into it
    k.layer = ArtLayer::Front;
    k.row("acid_body", -2.5, 2.5, -3.0, v(1.0, 1.0));
    k.row("acid_surface", -2.5, 2.5, -2.0, v(1.0, 0.25));

    LevelDef {
        name: "Art Test",
        boxes,
        lamps: vec![],
        ropes: vec![rope(0.0, 12.0, 5.5)],
        hazards: vec![spikes(10.0, 0.19, 1.0, 0.19), acid(0.0, -2.375, 2.5, 0.625)],
        balls: vec![],
        lights: vec![
            light(0.0, 14.0, 40.0, 0xfff2e0, 0.25),
            light(-8.0, 7.0, 7.0, 0xffcf70, 0.35),
            light(0.0, 7.0, 7.0, 0xffcf70, 0.35),
            light(8.0, 7.0, 7.0, 0xffcf70, 0.35),
        ],
        decor: vec![],
        art: k.art,
        spawns: vec![v(-6.5, 1.5), v(3.5, 1.5), v(-3.5, 1.5), v(8.2, 1.5)],
        // the rack: one of each weapon along both floors
        weapon_spawns: [-8.1, -6.3, -5.9, -4.1, -3.3, 3.3, 4.1, 5.9, 8.1, 9.5].iter().map(|&x| v(x, 0.4)).collect(),
        min: v(-15.0, -4.5),
        max: v(15.0, 12.5),
        kill_y: -10.0,
        training: true,
        bg: 0x1d1a24,
        ambient: [0.62, 0.6, 0.64],
    }
}

/// A four-player arena built from the HD level kit (about 46 × 20 m): a
/// steel tower and spikes on the left, a sunken bowl with a steel stage in
/// the middle under a platform and two ropes, an acid pit with a bridge and a
/// taller tower on the right, and walkways up high under a low stone ceiling.
fn brickworks() -> LevelDef {
    let r = |x0: f32, y0: f32, x1: f32, y1: f32, color: u32| b((x0 + x1) * 0.5, (y0 + y1) * 0.5, (x1 - x0) * 0.5, (y1 - y0) * 0.5, color);
    let bottom = -5.5;
    let boxes = vec![
        // walls and ceiling
        r(-23.0, bottom, -22.0, 14.5, STONE),
        r(22.0, bottom, 23.0, 14.5, STONE),
        r(-22.0, 13.5, 22.0, 14.5, STONE),
        // left: tower, floor, its ledge, crates, a column
        r(-22.0, bottom, -17.0, 6.0, STEEL),
        r(-17.0, bottom, -7.0, 0.0, STONE),
        r(-17.0, 5.5, -13.0, 6.0, WOODC),
        r(-12.0, 0.0, -11.0, 1.0, WOODC),
        r(-11.0, 0.0, -10.5, 0.5, WOODC),
        r(-8.3125, 0.0, -7.6875, 3.5, STONE),
        // middle: the bowl, its stage, the platform over it, the walkways
        r(-7.0, bottom, 7.0, -1.5, STONE),
        r(-2.5, -1.5, 2.5, 0.5, STEEL),
        r(-3.5, 3.5, 3.5, 4.0, WOODC),
        r(-13.0, 7.5, -6.0, 8.0, WOODC),
        r(6.0, 7.5, 12.0, 8.0, WOODC),
        // right: floor, column, crate, the acid pit and its bridge, the tower
        r(7.0, bottom, 12.0, 0.0, STONE),
        r(10.1875, 0.0, 10.8125, 3.5, STONE),
        r(8.0, 0.0, 9.0, 1.0, WOODC),
        r(12.0, bottom, 16.0, -3.0, STONE),
        r(12.0, 1.5, 16.0, 2.0, WOODC),
        r(16.0, bottom, 17.0, 0.0, STONE),
        r(17.0, bottom, 22.0, 8.0, STEEL),
        r(13.0, 7.5, 17.0, 8.0, WOODC),
    ];

    let mut k = Kit { art: Vec::new(), layer: ArtLayer::Back };
    // the back wall: 2 m brick tiles, lit windows along the upper level
    let (mut y, mut j) = (bottom, 0);
    while y < 14.5 {
        let mut x = -23.0f32;
        while x < 23.0 {
            let window = j == 8 && [-17.0, -9.0, -1.0, 7.0, 15.0].contains(&x);
            k.piece(if window { "bg_window" } else { "bg_brick" }, v(x, y), v(2.0, 2.0));
            x += 2.0;
        }
        y += 2.0;
        j += 1;
    }
    for x in [-6.5, 4.5] {
        k.piece("bg_support", v(x, -1.5), v(2.0, 2.0));
    }
    k.piece("bg_support", v(-15.0, 6.0), v(2.0, 2.0));
    k.piece("bg_support", v(13.5, 8.0), v(2.0, 2.0));

    k.layer = ArtLayer::Solid;
    k.block(&STONE_SET, v(-23.0, bottom), v(-22.0, 14.5));
    k.block(&STONE_SET, v(22.0, bottom), v(23.0, 14.5));
    k.block(&STONE_SET, v(-22.0, 13.5), v(22.0, 14.5));
    k.block(&STEEL_SET, v(-22.0, bottom), v(-17.0, 6.0));
    k.row("trim_hazard", -22.0, -17.0, 3.0, v(1.0, 0.25));
    k.block(&STONE_SET, v(-17.0, bottom), v(-7.0, 0.0));
    k.ledge(-17.0, -13.0, 6.0);
    k.row("railing_tile", -17.0, -13.0, 6.0, v(1.0, 0.25));
    k.row("spike_strip", -16.0, -14.0, 0.0, v(1.0, 0.375));
    k.piece("crate_1m", v(-12.0, 0.0), v(1.0, 1.0));
    k.piece("crate_05m", v(-11.0, 0.0), v(0.5, 0.5));
    k.piece("barrel", v(-10.2, -14.0 / 256.0), v(0.75, 1.0));
    k.pillar(-8.0, 0.0, 3.5);

    k.block(&STONE_SET, v(-7.0, bottom), v(7.0, -1.5));
    k.block(&STEEL_SET, v(-2.5, -1.5), v(2.5, 0.5));
    k.ledge(-3.5, 3.5, 4.0);
    k.row("railing_tile", -3.5, 3.5, 4.0, v(1.0, 0.25));
    k.ledge(-13.0, -6.0, 8.0);
    k.ledge(6.0, 12.0, 8.0);
    k.row("railing_tile", -13.0, -6.0, 8.0, v(1.0, 0.25));
    k.row("railing_tile", 6.0, 12.0, 8.0, v(1.0, 0.25));

    k.block(&STONE_SET, v(7.0, bottom), v(12.0, 0.0));
    k.pillar(10.5, 0.0, 3.5);
    k.piece("crate_1m", v(8.0, 0.0), v(1.0, 1.0));
    k.piece("barrel", v(11.1, -14.0 / 256.0), v(0.75, 1.0));
    k.block(&STONE_SET, v(12.0, bottom), v(16.0, -3.0));
    k.ledge(12.0, 16.0, 2.0);
    k.block(&STONE_SET, v(16.0, bottom), v(17.0, 0.0));
    k.block(&STEEL_SET, v(17.0, bottom), v(22.0, 8.0));
    k.row("trim_hazard", 17.0, 22.0, 4.0, v(1.0, 0.25));
    k.ledge(13.0, 17.0, 8.0);
    k.row("railing_tile", 13.0, 17.0, 8.0, v(1.0, 0.25));

    // acid in front, so whoever falls in sinks into it
    k.layer = ArtLayer::Front;
    k.row("acid_body", 12.0, 16.0, -3.0, v(1.0, 1.0));
    k.row("acid_surface", 12.0, 16.0, -2.0, v(1.0, 0.25));

    LevelDef {
        name: "Brickworks",
        boxes,
        lamps: vec![],
        ropes: vec![rope(-4.5, 13.5, 5.0), rope(4.5, 13.5, 5.0)],
        hazards: vec![spikes(-15.0, 0.19, 1.0, 0.19), acid(14.0, -2.375, 2.0, 0.625)],
        balls: vec![],
        lights: vec![
            light(0.0, 13.0, 40.0, 0xfff2e0, 0.3),
            light(-16.0, 11.5, 8.0, 0xffcf70, 0.4),
            light(-8.0, 11.5, 8.0, 0xffcf70, 0.4),
            light(0.0, 11.5, 8.0, 0xffcf70, 0.4),
            light(8.0, 11.5, 8.0, 0xffcf70, 0.4),
            light(16.0, 11.5, 8.0, 0xffcf70, 0.4),
        ],
        decor: vec![],
        art: k.art,
        spawns: vec![v(-13.0, 1.5), v(9.6, 1.5), v(-4.5, 0.0), v(4.5, 0.0)],
        weapon_spawns: vec![
            v(-19.5, 6.4),
            v(-15.0, 6.4),
            v(-9.4, 0.4),
            v(-5.0, -1.1),
            v(0.0, 0.9),
            v(5.0, -1.1),
            v(0.0, 4.4),
            v(-9.5, 8.4),
            v(9.0, 8.4),
            v(14.0, 2.4),
            v(15.0, 8.4),
            v(19.5, 8.4),
        ],
        min: v(-23.0, bottom),
        max: v(23.0, 14.5),
        kill_y: -12.0,
        training: false,
        bg: 0x1d1a24,
        ambient: [0.62, 0.6, 0.64],
    }
}
