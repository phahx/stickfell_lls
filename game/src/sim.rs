//! The deterministic match simulation. `GameState` is the entire state that
//! rollback snapshots; `step` is a pure function of (state, inputs).

use stickfell_net::{Fnv64, RollbackGame};
use stickfell_physics::dmath::{self, Rng};
use stickfell_physics::{Distance, Obb, Particle, Vec2, World, WorldConfig};

use crate::fighter::{self, Fighter, HAND_A, HAND_B, PELVIS, S_FORE_A, S_HEAD, S_TORSO};
use crate::input::{self, *};
use crate::level::{BallDef, HazardDef, HazardKind, LevelDef, levels};
use crate::sfx::{Sfx, SfxEvent};
use crate::weapon::{self, ARM, DROPS, Finish, GRAPPLE, KNIFE, Kind, LEG, PIN_HIT, RACK, STUMP, Weapon};
use crate::fighter::HandScript;

pub const DT: f32 = 1.0 / 60.0;
pub const MAX_PLAYERS: usize = 8;
const ROUND_OVER_FRAMES: u16 = 180;
const DROP_INTERVAL: u32 = 420;
const MAX_WEAPONS: usize = 14;
const LAMP_LINKS: u32 = 4;
/// Tip speed along the blade needed to run someone through.
const IMPALE_SPEED: f32 = 7.0;
/// Longest anyone stays on a blade (frames).
const SKEWER_FRAMES: u16 = 180;
/// Ragdoll time when impaled, and after coming off the blade.
const IMPALE_DOWN: u16 = 70;
const UNSKEWER_DOWN: u16 = 30;
/// How fast a victim slides down the blade when run through, and for how
/// many frames the blade stays slippery before it grips.
const IMPALE_GLIDE: f32 = 6.0;
const GLIDE_FRAMES: u16 = 10;
/// Frames a weapon picked up with its blade through someone passes through them.
const PICKUP_GRACE: u8 = 15;
/// Softness of the grip on a handle stuck in a wall while the hand pulls
/// itself onto it.
const EMBEDDED_GRAB: f32 = 4e-3;
/// How far behind the victim a wall can be for a sword to pin them to it.
const PIN_WALL_RANGE: f32 = 1.7;
/// Their neck's distance from the wall once shoved against it.
const PIN_GAP: f32 = 0.27;
/// The pinning sword's grip ends up this far from their chest.
const PIN_GRIP: f32 = 0.32;
/// How deep the pinning sword's tip goes into the wall (at least).
const PIN_DEPTH: f32 = 0.18;
/// Pin finisher frame the attacker lets go of their neck to flip the sword.
const PIN_LET_GO: u16 = 62;
/// Speed of the lunge that drives the pinning sword home.
const PIN_LUNGE: f32 = 5.5;
const SKEWER_GRIP: f32 = 0.45;
/// Tip speed along the blade that drives it into the floor or a wall.
const EMBED_SPEED: f32 = 9.0;
/// How far a stuck blade goes in.
const EMBED_DEPTH: f32 = 0.14;
/// Frames of hard pulling (or strikes) it takes to wrench a stuck weapon out.
const YANK_OUT: u8 = 24;
/// Extra damage for being run through.
const IMPALE_DAMAGE: f32 = 18.0;
/// How far past the hand (beyond the body's surface) a grab catches.
const GRAB_REACH: f32 = 0.3;
/// A held body thrown with Q: speed along the aim, plus some lift.
const BODY_THROW_SPEED: f32 = 12.0;
const BODY_THROW_LIFT: f32 = 3.5;
/// Struggle gained per key press while held (100 breaks free; ~1 a frame
/// on its own, so even a still victim slips out after a couple of seconds).
const STRUGGLE_PER_PRESS: u8 = 11;
/// Extra grab reach while diving head first.
const DIVE_REACH: f32 = 0.15;
/// How close someone has to be for a mid-air grab to dive at them.
const DIVE_RANGE: f32 = 3.5;
/// The flip over someone grabbed by the head: how hard they're flung on
/// at the end (along the flip, and up).
const FLIP_FLING: f32 = 7.5;
const FLIP_LIFT: f32 = 4.0;
/// A shoulder throw: how far behind you they can be to haul over, how high
/// over your neck their middle goes, where in front they're let go (just off
/// the floor), and how hard they're driven into it then (along, down).
const SHOULDER_REACH: f32 = 1.8;
const SHOULDER_OVER: f32 = 1.05;
const SHOULDER_LAND: f32 = 1.25;
const SHOULDER_SLAM: Vec2 = Vec2::new(2.0, -11.0);
const SHOULDER_DAMAGE: f32 = 18.0;
/// After a shoulder throw slams them down, how long (frames) the thrower's
/// finisher on them is a curb stomp.
const SHOULDER_COMBO: u8 = 60;
/// Frames a level's liquid is left to settle before a round (see `settled_liquid`).
const LIQUID_SETTLE: u32 = 150;
/// Spikes: how fast a body has to come down on a point to be run through on
/// it, and how hard head or heart first to die on it there and then.
const IMPALE_SPIKE_SPEED: f32 = 3.0;
const IMPALE_SPIKE_VITAL: f32 = 7.0;
/// The parts a spike runs through (not hands and feet).
const SPIKE_IMPALES: [u32; 6] = [S_HEAD, S_TORSO, fighter::S_UPPER_A, fighter::S_UPPER_B, fighter::S_THIGH_A, fighter::S_THIGH_B];
/// How far down a spike a body sinks (share of it up from the plate), the
/// damage going on, bleeding on it (per frame), and the struggle per key press.
const SPIKE_DEPTH: f32 = 0.3;
const SPIKE_IMPALE_DAMAGE: f32 = 20.0;
const SPIKE_BLEED: f32 = 0.14;
const SPIKE_STRUGGLE: u8 = 10;
/// Frames after being run through when a harder point (head or heart) can
/// still finish them, and the fewest frames between struggles that count.
const SPIKE_SETTLE: u16 = 8;
const SPIKE_BEAT: u8 = 4;
/// How fast (share of a spike a frame) a body above the point is drawn down onto it.
const SPIKE_DRAW: f32 = 0.15;
/// The curb stomp: how far past their head (away from their body) the
/// stomper stands, and how flat the first stomp leaves the head.
const STOMP_STAND: f32 = 0.42;
/// How far (m a frame) the stomper shuffles to get there.
const STOMP_SHUFFLE: f32 = 0.02;
const STOMP_SQUASH: f32 = 0.9;
/// A hit at least this hard makes the one hitting you let go.
const HOLD_BREAK_DAMAGE: f32 = 5.0;
/// Thrown bodies: losing more speed than this in a frame (hitting a wall,
/// the floor, someone) hurts.
const SLAM_SPEED: f32 = 5.0;
/// How close a sweeping leg has to pass to someone's shins to take them down.
const SWEEP_REACH: f32 = 0.3;
/// Pulling yourself up off a blade stuck in a wall (m/s upward).
const CLIMB_PULL: f32 = 10.5;
/// Thrown knives tumble end over end at this rate (rad/s, ~2.5 turns a second).
/// A gunshot's recoil (m/s): straight back, and up (the muzzle gets all of
/// it plus a quarter, the grip a quarter, so it flips about the hand).
const GUN_KICK_BACK: f32 = 2.5;
const GUN_KICK_UP: f32 = 3.5;
/// How long (frames) said voice lines stay in `GameState::voice_log`: far
/// longer than any wait for the other players' inputs.
const VOICE_LOG_FRAMES: u32 = 600;
/// How long (frames) other sounds stay in `GameState::sfx_log`: past the
/// furthest a rollback reaches back (8 frames) and then some.
const SFX_LOG_FRAMES: u32 = 16;
/// How often a choking fighter tries another choking sound (frames).
const CHOKE_EVERY: u32 = 20;
/// How long a victim winces after being hurt mid-finisher (frames).
const WINCE_FRAMES: u8 = 36;
/// How far past where it hit a thrown knife's point goes in (metres).
const KNIFE_BURIED: f32 = 0.09;
const KNIFE_SPIN: f32 = 16.0;
/// A dive flip with Ctrl held whips them away along the aim this fast, and
/// sets them spinning end over end.
const WHIP_SPEED: f32 = 30.0;
const WHIP_SPIN: f32 = 9.0;
/// Frames a whipped body passes through the one who whipped it.
const WHIP_GHOST: u8 = 18;
/// The grapple's rope: how far it pays out from a neck, how short it reels
/// in once the hook catches (neck below the hook), and how fast.
const NOOSE_SLACK: f32 = 8.5;
const HANG_LEN: f32 = 0.85;
/// Hung from a hook: how far their feet are left off the floor, and how much
/// rope hangs down when there's no floor below to judge by.
const HANG_CLEAR: f32 = 0.5;
const HANG_OVER_PIT: f32 = 2.0;
const REEL_SPEED: f32 = 1.8;
/// The fired hook: launch speed, the little it drops, and how long it flies
/// before it's spent (then it falls loose). The rope it drags them by is at
/// least this long. A loose hook's weight (kg).
const HOOK_SPEED: f32 = 24.0;
const HOOK_GRAVITY: f32 = 5.0;
const HOOK_RANGE: u16 = 48;
const DRAG_LEN: f32 = 1.4;
const HOOK_MASS: f32 = 0.6;
/// Health lost per frame hanging by the neck (40 hp lasts ~9 s).
const CHOKE_DRAIN: f32 = 0.075;
/// A blade crossing the rope at least this fast cuts it.
const CUT_SPEED: f32 = 2.5;
const ROPE_R: f32 = 0.03;
pub const TAG_LAMP_BULB: u32 = 1;
pub const TAG_BALL: u32 = 2;
/// Rope link length.
const ROPE_LINK: f32 = 0.45;

/// A wrecking ball / log on a chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Ball {
    pub def: BallDef,
    /// Particle ids from anchor to ball.
    pub chain: Vec<u32>,
}

impl Ball {
    pub fn ball(&self) -> u32 {
        *self.chain.last().unwrap()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lamp {
    pub def: usize,
    /// Particle ids from anchor to bulb.
    pub chain: Vec<u32>,
}

impl Lamp {
    pub fn bulb(&self) -> u32 {
        *self.chain.last().unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spark {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: u8,
    pub max_life: u8,
    pub blood: bool,
    /// A drop of liquid (its colour; 0: a spark or blood), or a puff of steam.
    pub tint: u32,
    pub steam: bool,
}

/// The practice yard's flood trap: its layout (if the level has one), when
/// the lever was pulled (round frame; 0: not yet), and how much water has
/// come in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trap {
    pub def: Option<crate::level::TrapDef>,
    /// The lever's thrown (the hatch open, or opening).
    pub open: bool,
    /// The round frame it was last thrown either way (0: never).
    pub toggled: u32,
    /// Drops poured in since it was last opened.
    pub poured: u32,
}

impl Trap {
    /// Frames since the lever was last thrown either way (None: never).
    pub fn since(&self, round_frame: u32) -> Option<u32> {
        (self.toggled > 0).then(|| round_frame.saturating_sub(self.toggled))
    }

    /// How far over the lever is (0 up and back, 1 thrown).
    pub fn lever(&self, round_frame: u32) -> f32 {
        let Some(t) = self.since(round_frame) else { return 0.0 };
        let moved = (t as f32 / LEVER_FRAMES as f32).min(1.0);
        if self.open { moved } else { 1.0 - moved }
    }

    /// How far open the hatch is (0 shut, 1 doors hanging straight down):
    /// it starts to open once the lever's over, and shuts as it comes back.
    pub fn hatch(&self, round_frame: u32) -> f32 {
        let Some(t) = self.since(round_frame) else { return 0.0 };
        if self.open {
            (t.saturating_sub(LEVER_FRAMES) as f32 / HATCH_FRAMES as f32).clamp(0.0, 1.0)
        } else {
            1.0 - (t as f32 / HATCH_FRAMES as f32).clamp(0.0, 1.0)
        }
    }
}

/// Frames the lever takes to throw, the hatch to swing open (or shut), and
/// before the water comes (after it's thrown open).
pub const LEVER_FRAMES: u32 = 8;
pub const HATCH_FRAMES: u32 = 30;
const POUR_AFTER: u32 = 24;
/// How far to the side of the lever you can stand and still pull it (m).
const LEVER_REACH: f32 = 1.0;

/// Swimming: from how much liquid round the pelvis they swim (waist deep;
/// measured 0.39 standing in 1.2 m of water), and swim even with their
/// feet on the bottom (chest deep: 0.56 in 1.6 m); how fast (m/s).
const SWIM_DEPTH: f32 = 0.35;
const SWIM_DEPTH_STANDING: f32 = 0.5;
const SWIM_SPEED: f32 = 2.8;
/// Once swimming, they keep swimming down to this much liquid round the
/// pelvis (laid flat at the surface it's measured 0.31..0.35); how hard
/// swimming steers them (1/s); and their buoyancy while stroking (water's
/// own 1.8 g of lift would beat any stroke: measured, they couldn't dive).
const SWIM_KEEP: f32 = 0.12;
/// A fall that ends with a body part stopped from more than this (m/s,
/// down) knocks them down: see `hard_landings`. Measured
/// (`sim::feel::big_falls_floor_you_and_small_ones_dont`): a double jump on
/// the flat lands at 12.7, a jump off a 3.5 m ledge 19.2, walking off 7 m
/// 19.1, 8 m 20.4, 9 m 20.3; jumping off 7 m 22.6, 10 m 25.0, 14 m 27.2.
const FALL_DOWN_SPEED: f32 = 20.5;
/// Frames down after such a fall, and more per m/s over it (up to the most).
const FALL_DOWN_FRAMES: u16 = 50;
const FALL_DOWN_MORE: f32 = 6.0;
const FALL_DOWN_MAX: u16 = 110;

/// Steam: from how thick (around the head) it scalds, and how hard at its
/// thickest (health a second: at least 33 s to kill, time to get out).
const SCALD_FROM: f32 = 0.3;
/// Steam off a body cooking in lava, every fifth frame (cells' worth).
const COOK_STEAM: f32 = 0.01;
const SCALD_RATE: f32 = 3.0;
/// A steam burst (water trapped under hot lava): how far it throws the
/// liquid (m) and how fast at its middle (m/s); the steam it makes (cells'
/// worth); how far it reaches fighters (m), how hard it knocks them back
/// (m/s) and hurts them at the closest; and the frames before another.
const BURST_REACH: f32 = 1.2;
const BURST_SPEED: f32 = 7.0;
const BURST_STEAM: f32 = 2.0;
const BURST_HIT: f32 = 1.6;
const BURST_PUSH: f32 = 6.0;
const BURST_DAMAGE: f32 = 15.0;
const BURST_COOLDOWN: u32 = 45;
/// How fast water has to hit hot lava to go off (m/s): measured over a
/// flood, the fastest 17 of 210 meetings (all as the front arrives, or
/// fresh lava breaks through the crust).
const BURST_IMPACT: f32 = 3.0;

/// How long the water's shove on a fighter is averaged over (s); how hard
/// (a share of their weight, sideways and down) knocks them off their feet,
/// and for how long (frames). Measured: walking in still water peaks at
/// 0.19, swimming 0.23; a 2 m dam breaking on them 0.36-0.38, standing under
/// the hatch's stream 0.55, the flood's front 0.65.
const SURGE_EASE: f32 = 0.25;
const SURGE_KNOCK: f32 = 0.3;
const SURGE_DOWN: u16 = 40;
/// Climbing out at the side: how far up over the neck the hands reach (the
/// arm's two bones, 0.38 + 0.35 m), looked for every HAUL_STEP m out to
/// HAUL_LOOK steps ahead; how far the feet clear the lip, how far onto it
/// they land (m), and how much faster than v = sqrt(2 g h) they go up to
/// make up for the water dragging at their legs (measured: at 1.0 the feet
/// came up 0.28 m short from 1 m out; at 1.2 they cleared the lip by
/// 0.25-0.31 m from 0.6-3 m out, landing standing 0.5-0.7 m onto it).
const ARM_REACH: f32 = 0.73;
const HAUL_STEP: f32 = 0.1;
const HAUL_LOOK: u32 = 10;
const HAUL_CLEAR: f32 = 0.15;
const HAUL_ONTO: f32 = 0.4;
const HAUL_LOSS: f32 = 1.2;
/// The least time a haul out takes (s).
const HAUL_TIME: f32 = 0.3;
/// Rafts: how far below the water their bottom sits, how much lower each
/// fighter on one puts it (m), how far it tips for each metre they stand
/// off its middle (rad), the most it tips, how far off its ends it reads the
/// water, its bob (m, rad/s), and how stiffly and how damped it follows.
const RAFT_DRAFT: f32 = 0.12;
const RAFT_SINK: f32 = 0.05;
const RAFT_TILT: f32 = 0.08;
const RAFT_MAX_TILT: f32 = 0.3;
const RAFT_LOOK: f32 = 0.4;
const RAFT_BOB: f32 = 0.05;
const RAFT_BOB_RATE: f32 = 1.3;
const RAFT_K: f32 = 6.0;
const RAFT_DAMP: f32 = 5.0;
/// A fluid sample this full of water is at its surface (measured over open
/// water's sea: 0.38-0.49 at the highest drop, 0.48-0.59 0.1 m under it,
/// 0.22-0.27 0.2 m over it).
const WATER_EDGE: f32 = 0.45;
/// Acid eats a part right away in this long, for as long as the part's
/// touching it (s), and lava melts one to the bone in this long. Below
/// `ROT_FROM` of it round a part (a stray drop), it isn't touching.
const ACID_EAT_TIME: f32 = 6.0;
const LAVA_MELT_TIME: f32 = 2.5;
const ROT_FROM: f32 = 0.08;
/// What a fluid sample reads with a part right in it (measured 0.39-0.44
/// in the middle of an acid vat): taken as all of it in it.
const SOAKED: f32 = 0.4;
/// The most of a torso acid eats away (out of `fighter::ROT_FULL`).
const TORSO_MOST: u16 = 52000;
/// A part lava's touched burns this long after it's out (frames); each
/// burning part costs the living FIRE_DPS health a second and chars them
/// (`scorch`) FIRE_CHAR a second (all ten parts alight); a part well alight
/// sets each part joined to it alight at FIRE_SPREAD a second; DOUSE of it
/// in water puts it out.
const FIRE_FRAMES: u16 = 360;
const FIRE_DPS: f32 = 1.2;
const FIRE_CHAR: f32 = 0.25;
const FIRE_SPREAD: f32 = 0.8;
const DOUSE: f32 = 0.25;
/// The most a camera shakes.
const SHAKE_MAX: f32 = 1.5;
/// Broken bones: the chance a hard blunt blow (more than BREAK_DAMAGE, or
/// knocked harder than BREAK_KNOCK) breaks the arm or leg bone it lands on,
/// and the same chance a fall stopped BREAK_FALL_OVER m/s past the one that
/// floors you breaks a leg.
const BREAK_CHANCE: f32 = 0.1;
const BREAK_DAMAGE: f32 = 14.0;
const BREAK_KNOCK: f32 = 16.0;
const BREAK_FALL_OVER: f32 = 3.0;
/// Steam off a part put out (cells' worth: a wisp, well short of scalding).
const DOUSE_STEAM: f32 = 0.04;
/// Pulling up onto a ledge from hanging off it: how much faster than
/// v = sqrt(2 g h) to go up, and the frames before you can grab or jump
/// again (so a held grab doesn't catch the wall on the way up).
const CLIMB_LOSS: f32 = 1.1;
const CLIMB_FRAMES: u8 = 20;
const SWIM_STEER: f32 = 6.0;
const SWIM_BUOYANCY: f32 = 1.0;
/// Treading water, how fast the bobbing dies away (1/s).
const TREAD_DAMP: f32 = 4.0;
/// Liquid round the pelvis from which a shoulder throw won't go: hip deep
/// (measured 0.18 in 0.9 m of water; knee deep, 0.6 m, is 0.0).
const THROW_WADE: f32 = 0.15;
/// A heave throw in water: what's left of it (share) once the thrower's in
/// this much liquid round the pelvis (waist deep), less in between.
const WATER_THROW_DEPTH: f32 = 0.5;
const WATER_THROW: f32 = 0.25;

/// Drowning: health lost every 8 frames once their breath is gone (with
/// AIR 600, held under they last 10 s, then 12 s more).
const DROWN_DAMAGE: f32 = 1.1;
/// Buoyancy (of g) for someone alive who can't swim (two limbs gone): they
/// go down.
const SINKING_BUOYANCY: f32 = 0.8;

/// Breath (frames) a fighter can hold under water.
pub const AIR: u16 = 600;
/// How much liquid round the head is having it under (breath held, drowning).
pub const HEAD_UNDER: f32 = 0.4;

/// A bullet in flight (simulated; hits are found by sweeping its path).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bullet {
    pub pos: Vec2,
    pub vel: Vec2,
    pub owner: u8,
    pub life: u8,
}

const BULLET_SPEED: f32 = 75.0;
const BULLET_GRAVITY: f32 = 6.0;
const BULLET_DAMAGE: f32 = 30.0;
const FIRE_COOLDOWN: u8 = 14;
/// Speed cap for weapons nobody is holding or has just thrown.
const LOOSE_MAX_SPEED: f32 = 16.0;

/// Someone down at or below this much health can be finished off.
pub const FINISH_HP: f32 = 40.0;
/// How close you have to be to start a finisher.
pub const FINISH_RANGE: f32 = 2.6;
/// Finishers play this much slower than their hand scripts are written.
const FINISH_TEMPO: f32 = 1.4;
/// How long a hit that leaves you at FINISH_HP or below dazes you (frames).
const DAZE_FRAMES: u16 = 240;
/// Real frames of slow motion around a finishing blow, and how slow.
const SLOWMO_FRAMES: u16 = 80;
const SLOWMO_SCALE: f32 = 0.3;
/// The slow motion starts this many finisher frames before the blow lands.
const SLOWMO_LEAD: u16 = 8;
/// Frames the finisher's name stays up.
const FINISH_BANNER: u16 = 130;
/// Frames after the announcer before the finisher's one-liner.
const QUIP_DELAY: u16 = 100;

/// A finisher in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finisher {
    pub attacker: u8,
    pub victim: u8,
    pub mv: Finish,
    pub t: u16,
    /// The weapon it's done with (kept track of after it's thrown).
    pub weapon: Option<u8>,
    /// The killing blow has landed.
    pub killed: bool,
    /// The wall behind the victim, for finishers that use one: where it is
    /// at their chest height when the finisher starts.
    pub wall: Option<Vec2>,
    /// A curb stomp: which way their head lay from their body when it started
    /// (+1: to the right); the stomper stands beyond it, facing back.
    pub side: i8,
}

/// Where a noose's hook is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hook {
    /// Still in the gun: the rope is a leash from the muzzle.
    Loaded,
    /// Fired, dragging them along behind it.
    Flying,
    /// Bitten into the level: the rope reels them up to it.
    Caught,
    /// Missed: the hook's loose on the end of the rope round their neck, a
    /// thing they drag about after them (it doesn't come back to the gun).
    Loose,
}

/// A grapple's rope round someone's neck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noose {
    pub weapon: u8,
    pub victim: u8,
    pub attacker: u8,
    /// Distance slot for the rope (enabled only while it's taut).
    pub tether: u32,
    /// Rope paid out right now.
    pub len: f32,
    /// Cut: the ends hang loose and it holds nothing.
    pub cut: bool,
    /// The hook has been fired and hasn't landed yet: the one on the end is
    /// a limp ragdoll and nothing hurts them.
    pub toss: bool,
    pub hook: Hook,
    /// Kinematic particle that is the hook once it's out of the gun.
    pub anchor: u32,
    pub hook_vel: Vec2,
    /// Frames since it was fired.
    pub hook_t: u16,
    /// How short it reels in once caught (hook to head): enough to leave
    /// their feet a little off the floor below, so some rope's left.
    pub hang: f32,
}

/// A raft afloat: its level def, its box in the world, and how it's riding
/// (height of its middle and how fast that's changing, tilt and its rate,
/// and where it is in its bob).
#[derive(Clone, Copy, Debug)]
pub struct Raft {
    pub def: crate::level::RaftDef,
    pub statik: u32,
    pub y: f32,
    pub vy: f32,
    pub tilt: f32,
    pub spin: f32,
    pub phase: f32,
}

/// Keys a blade lock can ask for: W A S D.
pub const CLASH_KEYS: [u16; 4] = [UP, LEFT, CROUCH, RIGHT];
pub const CLASH_KEY_NAMES: [&str; 4] = ["W", "A", "S", "D"];
/// Points to win a blade lock.
pub const CLASH_WIN: u8 = 3;
/// Blade-on-blade speed that can lock two swords together.
const CLASH_IMPACT: f32 = 7.0;
const CLASH_INTRO: u16 = 45;
const CLASH_POINT_PAUSE: u16 = 26;
/// Frames a wrong key locks you out of the current prompt.
const CLASH_LOCKOUT: u8 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClashStage {
    /// Zooming in, "CLASH!".
    Intro,
    /// A key is up: first to press it takes the point.
    Prompt,
    /// Someone just took a point.
    Point,
}

/// Two swords locked together: a quick-draw key race between their holders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clash {
    pub fighters: [u8; 2],
    /// Where the blades crossed.
    pub point: Vec2,
    pub score: [u8; 2],
    /// Index into `CLASH_KEYS`.
    pub prompt: u8,
    pub stage: ClashStage,
    /// Frames in the current stage, and overall.
    pub t: u16,
    pub total: u16,
    /// Frames each side is locked out after a wrong key.
    pub lockout: [u8; 2],
    /// Which side took the last point.
    pub last: Option<u8>,
}

impl Clash {
    /// Where the hands meet: drifts toward whoever is losing.
    pub fn lock_point(&self, s: &GameState) -> Vec2 {
        let a = s.fighters[self.fighters[0] as usize].pos(&s.world, fighter::NECK);
        let b = s.fighters[self.fighters[1] as usize].pos(&s.world, fighter::NECK);
        let mid = (a + b) * 0.5 + Vec2::new(0.0, -0.15);
        let ab = (b - a).normalize_or(Vec2::X);
        mid + ab * ((self.score[0] as f32 - self.score[1] as f32) * 0.18)
    }

    pub fn side_of(&self, fi: usize) -> Option<usize> {
        self.fighters.iter().position(|&f| f as usize == fi)
    }
}

/// A blood splat stuck to the level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decal {
    pub pos: Vec2,
    pub r: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Fight,
    RoundOver { timer: u16, winner: Option<u8> },
}

/// Something that happened this frame (for sound/screen effects).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitEvent {
    pub attacker: Option<u8>,
    pub victim: u8,
    pub point: Vec2,
    pub damage: f32,
    pub severed: bool,
    pub lethal: bool,
}

#[derive(Clone)]
pub struct GameState {
    /// The voice lines said lately, by the frame they were said on (part of
    /// the rolled-back state, so it always matches what really happened):
    /// players hear a line once its frame is final, so everyone hears the
    /// same lines in the same order.
    pub voice_log: Vec<(u32, SfxEvent)>,
    /// The other sounds of the last few frames, by frame (rolled back with
    /// the rest): online, a frame re-run once someone's late input arrives
    /// can have sounds the first run didn't (their gunshot, on the frame
    /// they clicked), and only the newest frame's own would leave those
    /// unheard (see `Audio::play_new`).
    pub sfx_log: Vec<(u32, SfxEvent)>,
    /// Realistic joint limits on every fighter (always on in the game; the
    /// character lab can switch them off to compare).
    pub joint_limits: bool,
    pub frame: u32,
    pub round_frame: u32,
    pub round: u32,
    pub num_players: usize,
    /// Fighters from this one on are played by the computer (worked out in
    /// the sim, so every player's copy does the same); none in practice.
    pub humans: usize,
    /// Team size (2 for 2v2), or 0: every one for themselves. Fighters are
    /// on alternate teams, the even ones and the odd ones.
    pub teams: u8,
    pub level_index: usize,
    pub world: World,
    pub fighters: Vec<Fighter>,
    pub weapons: Vec<Weapon>,
    pub lamps: Vec<Lamp>,
    /// Rope particle ids, anchor first.
    pub ropes: Vec<Vec<u32>>,
    pub balls: Vec<Ball>,
    pub hazards: Vec<HazardDef>,
    /// Practice yard rules (see `LevelDef::training`).
    pub training: bool,
    pub clash: Option<Clash>,
    pub finishers: Vec<Finisher>,
    pub nooses: Vec<Noose>,
    /// (move, attacker, frames left) for the finisher banner.
    pub finish_banner: Option<(Finish, u8, u16)>,
    /// Frames of shared slow motion left (a finishing blow), and of the
    /// white impact flash.
    pub slowmo: u16,
    pub flash: u8,
    /// (winner, frames left) for the "wins the clash" banner.
    pub clash_banner: Option<(u8, u16)>,
    clash_cd: u16,
    /// Per fighter: frames before a hazard can hurt them again.
    hazard_cd: Vec<u8>,
    /// How fast each particle was falling just before this frame's physics
    /// (m/s down): a landing is over by the time hazards are checked.
    fall: Vec<f32>,
    /// Per fighter: two fixed particles (a spike's base and point) and a
    /// skewer slot, for being run through on a spike bed.
    pub spike_pins: Vec<[u32; 3]>,
    /// Every liquid in the arena: a volume of drops (see `stickfell_physics::fluid`).
    pub fluid: stickfell_physics::Fluid,
    /// The flood trap, if the level has one.
    pub trap: Trap,
    /// Steam off water boiling on lava, and frames until the next steam
    /// burst can go off.
    pub steam: crate::steam::Steam,
    pub burst_cd: u32,
    /// (Tests: every water drop meeting hot lava, how far under it and how
    /// fast into it.)
    #[cfg(test)]
    pub hot_contacts: Vec<(f32, f32)>,
    pub sparks: Vec<Spark>,
    pub decals: Vec<Decal>,
    pub bullets: Vec<Bullet>,
    pub hits: Vec<HitEvent>,
    /// Sounds this frame (not part of the simulated state).
    pub events: Vec<SfxEvent>,
    pub rng: Rng,
    pub scores: Vec<u16>,
    pub phase: Phase,
    /// Camera shake, per fighter: only the player it happened to (or who
    /// did it) feels it.
    pub shake: [f32; MAX_PLAYERS],
    next_drop: u32,
    punch_cd: Vec<u8>,
    pub(crate) kill_y: f32,
    /// Rafts afloat this round.
    pub rafts: Vec<Raft>,
    min: Vec2,
    max: Vec2,
}

enum Who {
    Fighter(usize, u32),
    Weapon(usize, usize),
    Ball,
    Other,
}

/// Shortest distance between segments a0-a1 and b0-b1.
fn seg_dist(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> f32 {
    let pt_seg = |p: Vec2, s0: Vec2, s1: Vec2| {
        let d = s1 - s0;
        let t = if d.length_squared() > 1e-8 { ((p - s0).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (p - (s0 + d * t)).length()
    };
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let (d1, d2, d3, d4) = (cross(b0, b1, a0), cross(b0, b1, a1), cross(a0, a1, b0), cross(a0, a1, b1));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0; // they cross
    }
    pt_seg(a0, b0, b1).min(pt_seg(a1, b0, b1)).min(pt_seg(b0, a0, a1)).min(pt_seg(b1, a0, a1))
}

fn who(tag: u32) -> Who {
    if let Some((w, s)) = weapon::decode_tag(tag) {
        Who::Weapon(w, s)
    } else if let Some((f, p)) = fighter::decode_tag(tag) {
        Who::Fighter(f, p)
    } else if tag == TAG_BALL {
        Who::Ball
    } else {
        Who::Other
    }
}

/// A hanging rope of light links; returns particle ids, anchor first.
pub(crate) fn build_rope(w: &mut World, anchor: Vec2, length: f32) -> Vec<u32> {
    let links = (length / ROPE_LINK).ceil().max(1.0) as u32;
    let mut prev = w.add_particle(Particle::new(anchor, 0.0, 0.03));
    let mut chain = vec![prev];
    for k in 1..=links {
        let pos = anchor - Vec2::new(0.0, length * k as f32 / links as f32);
        let mut p = Particle::new(pos, 0.3, 0.05);
        p.friction = 0.4;
        let id = w.add_particle(p);
        w.add_distance(prev, id, 0.0);
        chain.push(id);
        prev = id;
    }
    chain
}

impl GameState {
    pub fn new(num_players: usize, level_index: usize, seed: u32) -> Self {
        Self::new_match(num_players, num_players, 0, level_index, seed)
    }

    /// A match of `num_players` fighters, the first `humans` of them played
    /// by people and the rest by the computer, in teams of `teams` (0: every
    /// one for themselves).
    pub fn new_match(num_players: usize, humans: usize, teams: u8, level_index: usize, seed: u32) -> Self {
        let num_players = num_players.clamp(1, MAX_PLAYERS);
        let mut s = Self {
            joint_limits: true,
            frame: 0,
            round_frame: 0,
            round: 0,
            num_players,
            humans: humans.min(num_players),
            teams,
            level_index: level_index % levels().len(),
            world: World::default(),
            fighters: Vec::new(),
            weapons: Vec::new(),
            lamps: Vec::new(),
            ropes: Vec::new(),
            balls: Vec::new(),
            hazards: Vec::new(),
            training: false,
            clash: None,
            finishers: Vec::new(),
            nooses: Vec::new(),
            finish_banner: None,
            slowmo: 0,
            flash: 0,
            clash_banner: None,
            clash_cd: 0,
            hazard_cd: Vec::new(),
            fall: Vec::new(),
            spike_pins: Vec::new(),
            fluid: stickfell_physics::Fluid::new(crate::liquid::fluid_kinds(), crate::liquid::DROP_SPACING),
            trap: Trap::default(),
            steam: crate::steam::Steam::default(),
            burst_cd: 0,
            #[cfg(test)]
            hot_contacts: Vec::new(),
            sparks: Vec::new(),
            decals: Vec::new(),
            bullets: Vec::new(),
            hits: Vec::new(),
            events: Vec::new(),
            voice_log: Vec::new(),
            sfx_log: Vec::new(),
            rng: Rng::new(seed),
            scores: vec![0; num_players],
            phase: Phase::Fight,
            shake: [0.0; MAX_PLAYERS],
            next_drop: 0,
            rafts: Vec::new(),
            punch_cd: vec![0; num_players],
            kill_y: -6.0,
            min: Vec2::ZERO,
            max: Vec2::ZERO,
        };
        s.reset_round();
        s
    }

    fn reset_round(&mut self) {
        self.nooses.clear();
        let lv = levels().swap_remove(self.level_index);
        let mut w = World::new(WorldConfig::default());
        for bx in &lv.boxes {
            w.add_static(Obb::new(bx.center, bx.half, bx.angle).with_friction(0.9));
        }
        for h in &lv.hazards {
            Self::add_spike_statics(&mut w, h);
        }
        // rafts: a box each, floated on the water every step (`float_rafts`)
        self.rafts.clear();
        for (i, rd) in lv.rafts.iter().enumerate() {
            let y = rd.sea.1 - RAFT_DRAFT + rd.half.y;
            let statik = w.add_static(Obb::new(Vec2::new(rd.x, y), rd.half, 0.0).with_friction(0.9));
            self.rafts.push(Raft { def: *rd, statik, y, vy: 0.0, tilt: 0.0, spin: 0.0, phase: i as f32 * 1.7 });
        }
        self.lamps.clear();
        for (i, ld) in lv.lamps.iter().enumerate() {
            let mut chain = Vec::new();
            let link = ld.length / LAMP_LINKS as f32;
            let mut prev = w.add_particle(Particle::new(ld.anchor, 0.0, 0.03));
            chain.push(prev);
            for k in 1..=LAMP_LINKS {
                let last = k == LAMP_LINKS;
                let sway = if i % 2 == 0 { 0.06 } else { -0.06 };
                let pos = ld.anchor + Vec2::new(sway * k as f32, -link * k as f32);
                let mut p = Particle::new(pos, if last { 1.5 } else { 0.15 }, if last { 0.2 } else { 0.03 });
                p.collide_static = last;
                let id = w.add_particle(p);
                w.add_distance(prev, id, 0.0);
                if last {
                    w.add_segment(id, id, 0.2, 0, TAG_LAMP_BULB);
                } else {
                    w.add_segment(prev, id, 0.035, 0, 0);
                }
                chain.push(id);
                prev = id;
            }
            self.lamps.push(Lamp { def: i, chain });
        }
        self.ropes.clear();
        for rd in &lv.ropes {
            self.ropes.push(build_rope(&mut w, rd.anchor, rd.length));
        }
        self.balls.clear();
        for bd in &lv.balls {
            const LINKS: u32 = 6;
            let mut prev = w.add_particle(Particle::new(bd.anchor, 0.0, 0.03));
            let mut chain = vec![prev];
            for k in 1..=LINKS {
                let last = k == LINKS;
                let pos = bd.anchor - Vec2::new(0.0, bd.length * k as f32 / LINKS as f32);
                let mut p = Particle::new(pos, if last { bd.mass } else { 0.5 }, if last { bd.radius } else { 0.05 });
                p.collide_static = last;
                if last {
                    p.vel.x = bd.swing;
                }
                let id = w.add_particle(p);
                w.add_distance(prev, id, 0.0);
                if last {
                    w.add_segment(id, id, bd.radius, 0, TAG_BALL);
                }
                chain.push(id);
                prev = id;
            }
            self.balls.push(Ball { def: *bd, chain });
        }
        self.hazards = lv.hazards.clone();
        self.fluid = Self::settled_liquid(self.level_index, &self.hazards, &w);
        self.trap = Trap { def: lv.trap, open: false, toggled: 0, poured: 0 };
        self.steam = crate::steam::Steam::new(lv.min - Vec2::splat(2.0), lv.max + Vec2::splat(2.0), &w);
        self.burst_cd = 0;
        self.clash = None;
        self.clash_banner = None;
        self.clash_cd = 0;
        self.finishers.clear();
        self.finish_banner = None;
        self.slowmo = 0;
        self.flash = 0;
        self.hazard_cd = vec![0; self.num_players];
        self.fighters.clear();
        for id in 0..self.num_players {
            let sp = self.spawn_point(&lv, id);
            let feet = w.raycast(sp, Vec2::NEG_Y, 25.0).map_or(sp, |h| h.point + Vec2::new(0.0, 0.02));
            let facing = if sp.x <= 0.0 { 1.0 } else { -1.0 };
            self.fighters.push(Fighter::spawn(&mut w, id as u8, feet, facing));
        }
        self.spike_pins = Self::make_spike_pins(&mut w, self.num_players);
        self.world = w;
        self.weapons.clear();
        // starting weapons on distinct spawn points
        self.training = lv.training;
        if self.training {
            // the rack: one of every weapon, in order
            for (k, &def) in RACK.iter().enumerate() {
                self.spawn_loose_weapon(def, lv.weapon_spawns[k % lv.weapon_spawns.len()]);
            }
        } else if lv.arms == crate::level::Arms::OneEach {
            // one where each fighter starts
            for id in 0..self.fighters.len() {
                let def = DROPS[self.rng.next_u32() as usize % DROPS.len()];
                let at = self.spawn_point(&lv, id);
                self.spawn_loose_weapon(def, at + Vec2::new(0.6, -0.6));
            }
        } else if lv.arms == crate::level::Arms::Scattered {
            let n = (self.num_players + 3).min(lv.weapon_spawns.len());
            let mut used = Vec::new();
            while used.len() < n {
                let k = (self.rng.next_u32() as usize) % lv.weapon_spawns.len();
                if !used.contains(&k) {
                    used.push(k);
                }
            }
            for k in used {
                let def = DROPS[self.rng.next_u32() as usize % DROPS.len()];
                self.spawn_loose_weapon(def, lv.weapon_spawns[k]);
            }
        }
        self.kill_y = lv.kill_y;
        self.min = lv.min;
        self.max = lv.max;
        self.sparks.clear();
        self.decals.clear();
        self.bullets.clear();
        self.hits.clear();
        self.punch_cd = vec![0; self.num_players];
        self.phase = Phase::Fight;
        self.round_frame = 0;
        self.next_drop = self.frame + DROP_INTERVAL;
        self.round += 1;
    }

    pub(crate) fn spawn_weapon(&mut self, def: u8, pos: Vec2) {
        let idx = self.weapons.len();
        let angle = self.rng.range(-0.3, 0.3);
        let w = Weapon::spawn(&mut self.world, def, idx, pos, angle);
        self.weapons.push(w);
    }

    /// A weapon lying in the level: flat, and lifted clear of any geometry it
    /// would otherwise start inside (which would shoot it out at speed).
    fn spawn_loose_weapon(&mut self, def: u8, pos: Vec2) {
        let idx = self.weapons.len();
        let w = Weapon::spawn(&mut self.world, def, idx, pos, 0.0);
        // nearest clear spot: search outward up, sideways and diagonally
        if w.touches_level(&self.world) {
            let dirs = [Vec2::Y, Vec2::X, -Vec2::X, Vec2::new(0.7, 0.7), Vec2::new(-0.7, 0.7)];
            let mut moved = Vec2::ZERO;
            'search: for k in 1..=30 {
                for d in dirs {
                    let off = d * (k as f32 * 0.1);
                    w.shift(&mut self.world, off - moved);
                    moved = off;
                    if !w.touches_level(&self.world) {
                        break 'search;
                    }
                }
            }
        }
        self.weapons.push(w);
    }

    pub fn alive_count(&self) -> usize {
        self.fighters.iter().filter(|f| f.alive).count()
    }

    /// Which team a fighter is on (0 or 1), in a team match.
    pub fn team(&self, id: usize) -> Option<u8> {
        (self.teams > 0).then_some((id % 2) as u8)
    }

    /// Whether two fighters are on the same side (yourself included).
    pub fn allies(&self, a: usize, b: usize) -> bool {
        a == b || (self.teams > 0 && a % 2 == b % 2)
    }

    /// Teams with someone still alive.
    fn teams_alive(&self) -> usize {
        (0..2).filter(|&t| self.fighters.iter().any(|f| f.alive && f.id as usize % 2 == t)).count()
    }

    /// Where fighter `id` starts: in turn round the level's spawns, or in a
    /// team match, one team from the left end and the other from the right.
    fn spawn_point(&self, lv: &LevelDef, id: usize) -> Vec2 {
        let n = lv.spawns.len();
        if self.teams == 0 {
            return lv.spawns[id % n];
        }
        let mut by_x = lv.spawns.clone();
        by_x.sort_by(|a, b| a.x.total_cmp(&b.x));
        let k = (id / 2) % n;
        if id % 2 == 0 { by_x[k] } else { by_x[n - 1 - k] }
    }

    // ---- weapons in hand ------------------------------------------------

    fn equip(&mut self, fi: usize, wi: usize) {
        if let Some((v, _)) = self.weapons[wi].stuck {
            // yanked out of them
            let at = self.world.particles[self.weapons[wi].particle(0) as usize].pos;
            self.unstick(wi);
            self.spawn_blood(at, (self.fighters[fi].pos(&self.world, fighter::NECK) - at).normalize_or(Vec2::Y), 8, 3.0);
            self.events.push(Sfx::Unembed.at(at));
            if v as usize != fi {
                self.wince(v as usize);
            }
        }
        let f = &self.fighters[fi];
        let (hand_a, hand_b, grip_a, grip_b) = (f.p(HAND_A), f.p(HAND_B), f.grip_a, f.grip_b);
        let can_two = f.arm_b_ok() && !f.grabbing;
        let (angle, hand) = (f.weapon_angle(), self.world.particles[hand_a as usize]);
        if !self.weapons[wi].embedded {
            self.weapons[wi].place(&mut self.world, hand.pos, angle, hand.vel);
        }
        let w = &mut self.weapons[wi];
        let def = w.def();
        w.holder = Some(fi as u8);
        w.last_holder = Some(fi as u8);
        w.thrown = 0;
        w.grace = 0;
        w.set_group(&mut self.world, fi as u16 + 1);
        // a handle stuck in a wall can't come to the hand: the hand pulls
        // itself onto it (softly, firming up once it's there) instead of
        // snapping across and yanking the body after it
        let reach = if w.embedded { EMBEDDED_GRAB } else { 0.0 };
        self.world.distances[grip_a as usize] =
            Distance { a: hand_a, b: w.particle(def.grip), rest: 0.0, compliance: reach, enabled: true };
        if let (Some(g2), true) = (def.grip2, can_two) {
            self.world.distances[grip_b as usize] =
                Distance { a: hand_b, b: w.particle(g2), rest: 0.0, compliance: 2e-4, enabled: true };
        }
        // (a blade stuck in a wall can't be turned: it's only aimed once it's free)
        self.world.aims[w.aim as usize].enabled = !w.embedded;
        self.fighters[fi].weapon = Some(wi as u8);
        // picked up with the blade already through someone: it slides out of
        // them rather than counting as a stab
        if let Some(o) = (0..self.fighters.len()).find(|&o| o != fi && self.weapon_inside(wi, o)) {
            let w = self.weapons[wi].clone();
            w.set_pass_through(&mut self.world, o as u16 + 1);
            self.weapons[wi].pass_grace = PICKUP_GRACE;
        }
        let at = self.world.particles[hand_a as usize].pos;
        self.events.push(Sfx::Pickup.at(at));
    }

    /// Any part of weapon `wi` overlapping fighter `o`'s body.
    fn weapon_inside(&self, wi: usize, o: usize) -> bool {
        let w = &self.weapons[wi];
        let f = &self.fighters[o];
        if !f.alive {
            return false;
        }
        let seg = |i: u32| {
            let sg = self.world.segments[i as usize];
            (self.world.particles[sg.a as usize].pos, self.world.particles[sg.b as usize].pos, sg.radius)
        };
        (0..w.nseg as u32).any(|k| {
            let (a, b, r) = seg(w.seg_base + k);
            (0..10u32).any(|p| {
                let (c, d, q) = seg(f.seg_base + p);
                let (_, _, x, y) = stickfell_physics::collide::closest_segments(a, b, c, d);
                (x - y).length() < r + q
            })
        })
    }

    fn release(&mut self, fi: usize, throw: bool) {
        let Some(wi) = self.fighters[fi].weapon.take() else { return };
        let f = &self.fighters[fi];
        self.world.distances[f.grip_a as usize].enabled = false;
        self.world.distances[f.grip_b as usize].enabled = false;
        let dir = f.aim_dir();
        let w = &mut self.weapons[wi as usize];
        w.holder = None;
        w.grace = 12;
        self.world.aims[w.aim as usize].enabled = false;
        if throw {
            let at = self.world.particles[w.base as usize].pos;
            self.events.push(Sfx::Throw.at(at));
            w.thrown = 50;
            let grip = self.world.particles[w.particle(w.def().grip) as usize].pos;
            // spears fly point-first; knives spin end over end around their
            // middle; everything else tumbles from the grip
            let speed = w.def().throw_speed;
            if w.is_knives() {
                let spin = Self::throw_spin(w.def(), dir);
                let center = (0..w.n).map(|k| self.world.particles[w.particle(k) as usize].pos).sum::<Vec2>() / w.n as f32;
                for k in 0..w.n {
                    let p = &mut self.world.particles[w.particle(k) as usize];
                    p.vel += dir * speed + (p.pos - center).perp() * spin;
                }
            } else {
                let spin = if w.def().thrust { 0.0 } else { 7.0 };
                for k in 0..w.n {
                    let p = &mut self.world.particles[w.particle(k) as usize];
                    let along = (p.pos - grip).dot(dir);
                    p.vel += dir * speed + dir.perp() * (along * spin);
                }
            }
        }
    }

    /// Q with a bundle of knives: one flies, the rest stay in hand.
    fn throw_knife(&mut self, fi: usize, wi: usize) {
        if self.weapons[wi].ammo <= 1 || self.weapons.len() >= MAX_WEAPONS * 2 {
            self.release(fi, true);
            return;
        }
        self.weapons[wi].ammo -= 1;
        let f = &self.fighters[fi];
        let dir = f.aim_dir();
        let hand = self.world.particles[f.p(HAND_A) as usize];
        let start = hand.pos + dir * 0.2;
        self.spawn_weapon(KNIFE, start);
        let ki = self.weapons.len() - 1;
        let k = &mut self.weapons[ki];
        k.place(&mut self.world, start, dmath::angle(dir), hand.vel);
        k.last_holder = Some(fi as u8);
        k.thrown = 50;
        k.grace = 12;
        k.set_group(&mut self.world, fi as u16 + 1);
        let speed = k.def().throw_speed;
        let spin = Self::throw_spin(k.def(), dir);
        let center = (0..k.n).map(|j| self.world.particles[k.particle(j) as usize].pos).sum::<Vec2>() / k.n as f32;
        for j in 0..k.n {
            let p = &mut self.world.particles[k.particle(j) as usize];
            p.vel += dir * speed + (p.pos - center).perp() * spin;
        }
        self.events.push(Sfx::Throw.at(start).vol(0.7));
    }

    /// Severed arms and legs become loose weapons (swing them like the fish):
    /// a weapon is built where the limb is, moving as it was, and the body's
    /// own copy is put away.
    fn limbs_to_weapons(&mut self) {
        for fi in 0..self.fighters.len() {
            for (root, sub) in [(fighter::S_UPPER_A, S_FORE_A), (fighter::S_UPPER_B, fighter::S_FORE_B), (fighter::S_THIGH_A, fighter::S_SHIN_A), (fighter::S_THIGH_B, fighter::S_SHIN_B)] {
                let f = &self.fighters[fi];
                // the whole limb came off, or just its lower half
                let (parts, def): (&[u32], u8) = if !f.has(root) {
                    (&[root, sub], if root == fighter::S_UPPER_A || root == fighter::S_UPPER_B { ARM } else { LEG })
                } else if !f.has(sub) {
                    (&[sub], STUMP)
                } else {
                    continue;
                };
                if f.limbs_gone & fighter::bit(parts[0]) != 0 || self.weapons.len() >= MAX_WEAPONS * 3 {
                    continue;
                }
                // particles from the held end (hand/foot) to the torn end
                let segs: Vec<stickfell_physics::Segment> = parts.iter().map(|&p| self.world.segments[(f.seg_base + p) as usize]).collect();
                let mut ids = vec![segs[segs.len() - 1].b];
                for sg in segs.iter().rev() {
                    ids.push(sg.a);
                }
                let src: Vec<Particle> = ids.iter().map(|&i| self.world.particles[i as usize]).collect();
                let angle = dmath::angle(src[1].pos - src[0].pos);
                let wi = self.weapons.len();
                let links = self.world.distances.len();
                let mut w = Weapon::spawn(&mut self.world, def, wi, src[0].pos, angle);
                // it keeps the size it had on the body: link lengths as they
                // were, each segment as thick as the part it was (segments run
                // from the hand/foot end, so the parts in reverse)
                let n_links = w.def().links.len();
                for k in 0..n_links {
                    self.world.distances[links + k].rest = (src[k + 1].pos - src[k].pos).length();
                }
                for k in 0..(w.nseg as usize).min(segs.len()) {
                    self.world.segments[(w.seg_base + k as u32) as usize].radius = segs[segs.len() - 1 - k].radius;
                }
                let leg = matches!(root, fighter::S_THIGH_A | fighter::S_THIGH_B);
                // (what acid and lava have done to it comes off with it: the
                // least-eaten part is what's left to go)
                let body = &self.fighters[fi];
                w.eaten = parts.iter().map(|&p| body.eaten[p as usize]).min().unwrap_or(0);
                w.melted = parts.iter().map(|&p| body.melted[p as usize]).min().unwrap_or(0);
                w.fire = parts.iter().map(|&p| body.fire[p as usize]).max().unwrap_or(0);
                w.limb = Some(match (parts.len(), leg) {
                    (2, false) => crate::weapon::LimbKind::Arm,
                    (2, true) => crate::weapon::LimbKind::Leg,
                    (_, false) => crate::weapon::LimbKind::Forearm,
                    (_, true) => crate::weapon::LimbKind::Shin,
                });
                for (k, sp) in src.iter().enumerate() {
                    let p = &mut self.world.particles[w.particle(k as u8) as usize];
                    p.pos = sp.pos;
                    p.prev = sp.prev;
                    p.vel = sp.vel;
                }
                // it keeps passing through what its body passed through for a
                // moment (a blade run through them, say) before it's loose
                w.owner = fi as u8 + 1;
                w.grace = 20;
                w.set_group(&mut self.world, fi as u16 + 1);
                self.weapons.push(w);
                let (seg_base, dist_base) = (f.seg_base, f.dist_base);
                for &p in parts {
                    self.world.segments[(seg_base + p) as usize].enabled = false;
                    self.world.distances[(dist_base + p - 1) as usize].enabled = false;
                    self.fighters[fi].limbs_gone |= fighter::bit(p);
                }
                for &i in &ids {
                    self.world.park_particle(i);
                }
            }
        }
    }

    /// The stuck weapon `fi` is hanging from (holding it, feet off the ground).
    pub fn hanging_from(&self, fi: usize) -> Option<usize> {
        let f = &self.fighters[fi];
        let wi = f.weapon? as usize;
        let w = &self.weapons[wi];
        let tip = self.world.particles[w.particle(w.def().impale?.2) as usize].pos;
        // (a blade stuck in the floor isn't something to hang from)
        let below = f.pos(&self.world, PELVIS).y < tip.y - 0.2;
        (f.alive && !f.grounded && w.embedded && below).then_some(wi)
    }

    /// Angular speed (rad/s, counter-clockwise positive) a thrown knife
    /// tumbles at: forward over the top, whichever way it's thrown.
    fn throw_spin(def: &weapon::WeaponDef, dir: Vec2) -> f32 {
        if def.class != weapon::Class::Knives {
            return 0.0;
        }
        let forward = if dir.x >= 0.0 { -1.0 } else { 1.0 };
        forward * KNIFE_SPIN
    }

    fn release_grab(&mut self, fi: usize) {
        let f = &mut self.fighters[fi];
        if !f.grabbing {
            return;
        }
        f.grabbing = false;
        f.grab_static = false;
        f.hold = None;
        f.hold_t = 0;
        f.heave = 0;
        f.shoulder = 0;
        self.world.distances[f.grab as usize].enabled = false;
        self.world.distances[f.tether as usize].enabled = false;
        // put the second hand back on a two-handed weapon
        if let Some(wi) = f.weapon {
            let w = &self.weapons[wi as usize];
            if let (Some(g2), true) = (w.def().grip2, f.arm_b_ok()) {
                self.world.distances[f.grip_b as usize] =
                    Distance { a: f.p(HAND_B), b: w.particle(g2), rest: 0.0, compliance: 2e-4, enabled: true };
            }
        }
    }

    fn try_grab(&mut self, fi: usize) {
        let f = &self.fighters[fi];
        let hb = f.p(HAND_B);
        let hand = self.world.particles[hb as usize].pos;
        let diving = f.diving();
        let reach_max = GRAB_REACH + if diving { DIVE_REACH } else { 0.0 };
        // people: take the best grip in reach, not just the nearest point:
        // neck and body first, then the weapon arm, legs only as a last resort
        let mut best: Option<(f32, u32, u8)> = None;
        for o in &self.fighters {
            if o.id == f.id {
                continue;
            }
            for pid in o.particle_ids() {
                let slot = pid - o.base;
                if !o.has(Self::part_of_slot(slot)) {
                    continue;
                }
                let d = (self.world.particles[pid as usize].pos - hand).length();
                let reach = d - self.world.particles[pid as usize].radius;
                let penalty = match slot {
                    fighter::NECK | fighter::HEAD => 0.0,
                    PELVIS => 0.06,
                    HAND_A | fighter::ELBOW_A => 0.1,
                    HAND_B | fighter::ELBOW_B => 0.14,
                    _ => 0.24,
                };
                if reach < reach_max && best.is_none_or(|(bd, _, _)| reach + penalty < bd) {
                    best = Some((reach + penalty, pid, o.id));
                }
            }
        }
        // ropes and lamp chains: hang from the nearest link, weight held by a
        // tether to the anchor
        let mut rope: Option<(f32, u32, u32)> = None;
        for chain in self.ropes.iter().chain(self.lamps.iter().map(|l| &l.chain)) {
            for &pid in &chain[1..] {
                let d = (self.world.particles[pid as usize].pos - hand).length();
                if d < 0.6 && rope.is_none_or(|(bd, _, _)| d < bd) {
                    rope = Some((d, pid, chain[0]));
                }
            }
        }
        let mut held = None;
        let (target, is_static, compliance) = if let Some((_, pid, who)) = best {
            held = Some((who, pid - self.fighters[who as usize].base));
            (pid, false, 1e-4)
        } else if let Some((_, pid, anchor)) = rope {
            let rest = (self.world.particles[anchor as usize].pos - hand).length();
            self.world.distances[f.tether as usize] = Distance { a: hb, b: anchor, rest, compliance: 0.0, enabled: true };
            (pid, true, 1e-4)
        } else if self.world.statics.iter().any(|s| !crate::level::is_spike(s.tag) && s.collide_circle(hand, 0.18).is_some()) {
            let a = f.anchor as usize;
            self.world.particles[a].pos = hand;
            self.world.particles[a].prev = hand;
            (f.anchor, true, 2e-5)
        } else {
            return;
        };
        let (grab, grip_b) = (f.grab, f.grip_b);
        self.world.distances[grab as usize] = Distance { a: hb, b: target, rest: 0.0, compliance, enabled: true };
        self.world.distances[grip_b as usize].enabled = false;
        let f = &mut self.fighters[fi];
        f.grabbing = true;
        f.grab_static = is_static;
        if let Some((who, slot)) = held {
            f.hold = Some(who);
            f.hold_slot = slot as u8;
            f.hold_t = 0;
            let at = self.world.particles[target as usize].pos;
            self.events.push(Sfx::Punch.at(at).vol(0.45));
            // diving onto someone's head: over the top you go
            let by_head = matches!(slot, fighter::HEAD | fighter::NECK);
            let (v, me) = (who as usize, fi);
            if diving && by_head && !self.in_finisher(v) && self.fighters[v].alive {
                self.start_flip(me, v);
            }
        }
    }

    /// Plans a flip over `v`, whose head `fi` has just caught in a dive: the
    /// body's centre arcs up over their head and down just past them, while
    /// the body turns forward until it lands upright.
    fn start_flip(&mut self, fi: usize, v: usize) {
        let f = &self.fighters[fi];
        let dir = if f.facing >= 0.0 { 1.0 } else { -1.0 };
        let ids: Vec<u32> = f.particle_ids().collect();
        let centre = ids.iter().map(|&i| self.world.particles[i as usize].pos).sum::<Vec2>() / ids.len() as f32;
        let mut pose = [Vec2::ZERO; fighter::PARTICLES as usize];
        for (k, &i) in ids.iter().enumerate() {
            pose[k] = self.world.particles[i as usize].pos - centre;
        }
        let head = self.fighters[v].pos(&self.world, fighter::HEAD);
        let their_pelvis = self.fighters[v].pos(&self.world, PELVIS);
        let over = Vec2::new(head.x - dir * 0.1, head.y + 2.4);
        let land = Vec2::new(head.x + dir * 1.2, their_pelvis.y + 0.15);
        // forward (clockwise going right) round to upright: most of a turn
        let torso = dmath::angle(f.pos(&self.world, fighter::NECK) - f.pos(&self.world, PELVIS));
        let up = core::f32::consts::FRAC_PI_2;
        let tau = core::f32::consts::TAU;
        let mut turn = (up - torso).rem_euclid(tau); // anticlockwise way round
        if dir > 0.0 {
            turn -= tau; // clockwise instead
            if turn > -core::f32::consts::PI {
                turn -= tau;
            }
        } else if turn < core::f32::consts::PI {
            turn += tau;
        }
        let f = &mut self.fighters[fi];
        f.flip = fighter::FLIP_FRAMES;
        f.flip_dir = dir as i8;
        // dove with the right mouse button and already on Shift: that's the whip
        f.flip_whip = f.held(GRAB) && !f.dove_shift;
        f.flip_pose = pose;
        f.flip_path = [centre, over, land];
        f.flip_turn = turn;
        // stunned but still on their feet while you vault over them; they
        // go limp as they're swung
        let vf = &mut self.fighters[v];
        vf.stagger = vf.stagger.max(fighter::FLIP_FRAMES);
        self.events.push(Sfx::SwingHeavy.at(head));
    }

    /// Flips in progress: the flipper follows the planned arc and turn (the
    /// body keeps its dive shape, turned), dragging the head they hold, and
    /// with it the rest of that body, over and end over end; at the end they
    /// let go and it goes flying on.
    fn update_flips(&mut self) {
        for fi in 0..self.fighters.len() {
            let f = &self.fighters[fi];
            if f.flip == 0 {
                continue;
            }
            let (Some(v), true) = (f.hold.map(|v| v as usize), f.grabbing && f.alive) else {
                self.fighters[fi].flip = 0;
                continue;
            };
            let dir = f.flip_dir as f32;
            let left = f.flip;
            if f.pressed(GRAB) {
                self.fighters[fi].flip_whip = true;
            }
            let f = &self.fighters[fi];
            // progress after this frame
            let u = 1.0 - (left - 1) as f32 / fighter::FLIP_FRAMES as f32;
            let ease = u * u * (3.0 - 2.0 * u);
            let [p0, p1, p2] = f.flip_path;
            let centre = p0 * (1.0 - u) * (1.0 - u) + p1 * 2.0 * u * (1.0 - u) + p2 * u * u;
            let (sn, cs) = dmath::sin_cos(f.flip_turn * ease);
            let ids: Vec<u32> = f.particle_ids().collect();
            let pose = f.flip_pose;
            for (k, &i) in ids.iter().enumerate() {
                let p = &mut self.world.particles[i as usize];
                if p.inv_mass == 0.0 {
                    continue;
                }
                let o = pose[k];
                let want = centre + Vec2::new(o.x * cs - o.y * sn, o.x * sn + o.y * cs);
                p.vel = p.vel.lerp((want - p.pos) / DT, 0.6);
            }
            if left == fighter::FLIP_FRAMES / 2 {
                self.fighters[v].knock_down(fighter::FLIP_FRAMES as u16 / 2 + 50);
            }
            let f = &mut self.fighters[fi];
            f.flip -= 1;
            if f.flip == 0 {
                // let go: they keep going, end over end
                self.release_grab(fi);
                self.fighters[fi].grab_cd = 30;
                let fling = Vec2::new(dir * FLIP_FLING, FLIP_LIFT);
                for id in self.fighters[v].particle_ids() {
                    let p = &mut self.world.particles[id as usize];
                    if p.inv_mass > 0.0 {
                        p.vel += fling;
                    }
                }
                // Shift pressed during the flip: not a toss, a whip, along
                // wherever you're aiming
                if self.fighters[fi].flip_whip {
                    let aim = self.fighters[fi].aim_dir();
                    let ids: Vec<u32> = self.fighters[v].particle_ids().collect();
                    let centre = ids.iter().map(|&i| self.world.particles[i as usize].pos).sum::<Vec2>() / ids.len() as f32;
                    let spin = if aim.x >= 0.0 { -WHIP_SPIN } else { WHIP_SPIN };
                    for &id in &ids {
                        let p = &mut self.world.particles[id as usize];
                        if p.inv_mass > 0.0 {
                            p.vel = aim * WHIP_SPEED + (p.pos - centre).perp() * spin;
                        }
                    }
                    // (flung out from behind you: it goes through you, not into your back)
                    let vf = &mut self.fighters[v];
                    vf.ghost = WHIP_GHOST;
                    for k in 0..fighter::BONES.len() as u32 {
                        self.world.segments[(vf.seg_base + k) as usize].ignore_group = fi as u16 + 1;
                    }
                    self.shake_camera(fi, 1.2);
                    self.shake_camera(v, 1.2);
                    self.events.push(Sfx::HitHeavy.at(centre).vol(1.2));
                    self.events.push(Sfx::SwingHeavy.at(centre).vol(1.5));
                }
                // and you come out of it standing, not sliding
                for id in self.fighters[fi].particle_ids() {
                    let p = &mut self.world.particles[id as usize];
                    p.vel *= 0.3;
                }
                let vf = &mut self.fighters[v];
                vf.knock_down(60);
                vf.flying = 45;
                vf.flung_by = fi as u8;
                let at = self.fighters[v].pos(&self.world, PELVIS);
                self.events.push(Sfx::Throw.at(at).vol(1.3));
            }
        }
    }

    /// Q while holding someone: by the weapon arm, wrench the weapon out of
    /// their hand; otherwise start winding up to throw them.
    fn heave_or_disarm(&mut self, fi: usize) -> bool {
        let f = &self.fighters[fi];
        let Some(v) = f.hold.map(|v| v as usize) else { return false };
        let by_arm = matches!(f.hold_slot as u32, HAND_A | fighter::ELBOW_A);
        if let (true, Some(wi)) = (by_arm, self.fighters[v].weapon) {
            let wi = wi as usize;
            self.release(v, false);
            if self.weapons[wi].embedded {
                return true;
            }
            self.release_grab(fi);
            if self.fighters[fi].weapon.is_none() && self.fighters[fi].arm_a_ok() {
                self.equip(fi, wi);
            } else {
                // can't take it: knocked away instead
                let away = Vec2::new(self.fighters[fi].facing * 4.0, 3.0);
                let w = &self.weapons[wi];
                for k in 0..w.n {
                    self.world.particles[w.particle(k) as usize].vel += away;
                }
            }
            self.fighters[fi].grab_cd = 20;
            let at = self.fighters[v].pos(&self.world, HAND_A);
            self.events.push(Sfx::Clash.at(at).vol(0.6));
            return true;
        }
        if self.can_shoulder_throw(fi, v) {
            self.start_shoulder_throw(fi, v);
            return true;
        }
        if f.heave == 0 {
            self.fighters[fi].heave = fighter::HEAVE_FRAMES;
        }
        false
    }

    /// Whether `fi` can haul `v` over their shoulder: on their feet, not
    /// already throwing, with their back to `v` and `v` close behind them.
    fn can_shoulder_throw(&self, fi: usize, v: usize) -> bool {
        let (f, vf) = (&self.fighters[fi], &self.fighters[v]);
        let behind = (vf.pos(&self.world, PELVIS).x - f.pos(&self.world, PELVIS).x) * f.facing;
        let tied = self.nooses.iter().any(|n| n.victim as usize == v && !n.cut);
        // (no leverage in water: hip deep or more, either of them, and it won't go)
        let footing = f.wade < THROW_WADE && vf.wade < THROW_WADE;
        f.grounded && footing && f.heave == 0 && f.shoulder == 0 && f.down == 0 && !vf.choking && !tied && !self.in_finisher(v)
            && (-SHOULDER_REACH..-0.05).contains(&behind)
    }

    /// Plans a shoulder throw: the held body's middle arcs from behind `fi`,
    /// up over their neck and down in front of them, turning heels over head
    /// to land on its back with the head towards the thrower.
    fn start_shoulder_throw(&mut self, fi: usize, v: usize) {
        let f = &self.fighters[fi];
        let dir = f.facing.signum();
        let ids: Vec<u32> = self.fighters[v].particle_ids().collect();
        let centre = ids.iter().map(|&i| self.world.particles[i as usize].pos).sum::<Vec2>() / ids.len() as f32;
        let mut pose = [Vec2::ZERO; fighter::PARTICLES as usize];
        for (k, &i) in ids.iter().enumerate() {
            pose[k] = self.world.particles[i as usize].pos - centre;
        }
        let neck = f.pos(&self.world, fighter::NECK);
        let feet = f.pos(&self.world, fighter::FOOT_A).y.min(f.pos(&self.world, fighter::FOOT_B).y);
        let top = Vec2::new(neck.x + dir * 0.1, neck.y + SHOULDER_OVER);
        let land = Vec2::new(f.pos(&self.world, PELVIS).x + dir * SHOULDER_LAND, feet + 0.55);
        // (the curve's middle point is pulled up so the arc's top reaches `top`)
        let over = top * 2.0 - (centre + land) * 0.5;
        // turn the way they're going over (clockwise going right) to lie on
        // their back, head towards you: at least half a turn
        let vf = &self.fighters[v];
        let torso = dmath::angle(vf.pos(&self.world, fighter::NECK) - vf.pos(&self.world, PELVIS));
        let pi = core::f32::consts::PI;
        let tau = core::f32::consts::TAU;
        let mut turn = if dir > 0.0 { -(torso - (pi + 0.15)).rem_euclid(tau) } else { (-0.15 - torso).rem_euclid(tau) };
        if turn.abs() < pi {
            turn += dir * -tau;
        }
        let f = &mut self.fighters[fi];
        f.shoulder = fighter::SHOULDER_FRAMES;
        f.shoulder_dir = dir as i8;
        f.flip_pose = pose;
        f.flip_path = [centre, over, land];
        f.flip_turn = turn;
        // limp from the moment they're hauled up, and they pass over you,
        // not through your head
        // (thrown from the start: whatever they hit on the way hurts)
        let vf = &mut self.fighters[v];
        vf.knock_down(fighter::SHOULDER_FRAMES as u16 + 70);
        vf.flying = fighter::SHOULDER_FRAMES + 20;
        vf.flung_by = fi as u8;
        vf.ghost = vf.ghost.max(fighter::SHOULDER_FRAMES + 6);
        for k in 0..fighter::BONES.len() as u32 {
            self.world.segments[(vf.seg_base + k) as usize].ignore_group = fi as u16 + 1;
        }
        self.events.push(Sfx::SwingHeavy.at(neck).vol(1.2));
    }

    /// Shoulder throws in progress: the held body follows the planned arc and
    /// turn; at the end it's let go and driven into the floor.
    fn update_shoulder_throws(&mut self) {
        for fi in 0..self.fighters.len() {
            let f = &self.fighters[fi];
            if f.shoulder == 0 {
                continue;
            }
            let (Some(v), true) = (f.hold.map(|v| v as usize), f.grabbing && f.alive) else {
                self.fighters[fi].shoulder = 0;
                continue;
            };
            let (t, dir) = (f.shoulder_t(), f.shoulder_dir as f32);
            let [p0, p1, p2] = f.flip_path;
            let centre = p0 * (1.0 - t) * (1.0 - t) + p1 * 2.0 * t * (1.0 - t) + p2 * t * t;
            let (sn, cs) = dmath::sin_cos(f.flip_turn * t);
            let pose = f.flip_pose;
            let ids: Vec<u32> = self.fighters[v].particle_ids().collect();
            for (k, &i) in ids.iter().enumerate() {
                let p = &mut self.world.particles[i as usize];
                if p.inv_mass == 0.0 {
                    continue;
                }
                let o = pose[k];
                let want = centre + Vec2::new(o.x * cs - o.y * sn, o.x * sn + o.y * cs);
                p.vel = p.vel.lerp((want - p.pos) / DT, 0.6);
            }
            let f = &mut self.fighters[fi];
            f.shoulder -= 1;
            if f.shoulder > 0 {
                continue;
            }
            // let go at the bottom: slammed flat
            self.release_grab(fi);
            self.fighters[fi].grab_cd = 30;
            let slam = Vec2::new(dir * SHOULDER_SLAM.x, SHOULDER_SLAM.y);
            for &id in &ids {
                let p = &mut self.world.particles[id as usize];
                if p.inv_mass > 0.0 {
                    p.vel = slam;
                }
            }
            let vf = &mut self.fighters[v];
            vf.flying = 20;
            vf.flung_by = fi as u8;
            vf.slam = SHOULDER_DAMAGE;
            vf.shouldered = SHOULDER_COMBO;
            vf.shouldered_by = fi as u8;
            let at = self.fighters[v].pos(&self.world, PELVIS);
            self.events.push(Sfx::Throw.at(at).vol(1.3));
        }
    }

    /// The end of a heave: whoever is held goes flying along the aim.
    fn throw_held(&mut self, fi: usize) {
        let f = &self.fighters[fi];
        let Some(v) = f.hold.map(|v| v as usize) else { return };
        let dir = f.aim_dir();
        // (no footing in water: waist deep, a quarter of a throw)
        let footing = 1.0 - (f.wade / WATER_THROW_DEPTH).clamp(0.0, 1.0) * (1.0 - WATER_THROW);
        let power = (0.55 + 0.45 * f.strength().min(1.0)) * footing;
        let fling = (dir * BODY_THROW_SPEED + Vec2::new(0.0, BODY_THROW_LIFT)) * power;
        self.release_grab(fi);
        self.fighters[fi].grab_cd = 30;
        for id in self.fighters[v].particle_ids() {
            let p = &mut self.world.particles[id as usize];
            p.vel = p.vel * 0.3 + fling;
        }
        let me = self.fighters[fi].particle_ids();
        self.world.add_velocity(me, -dir * 1.2);
        let vf = &mut self.fighters[v];
        if vf.alive {
            vf.knock_down(55);
        }
        vf.flying = 50;
        vf.flung_by = fi as u8;
        let at = self.fighters[v].pos(&self.world, PELVIS);
        self.events.push(Sfx::Throw.at(at).vol(1.3));
    }

    /// Who is holding whom (for strength, prompts and escapes), and the
    /// struggle to break free: mash the movement keys.
    fn update_holds(&mut self) {
        for f in &mut self.fighters {
            f.held_by = None;
            f.carry = 0.0;
        }
        for i in 0..self.fighters.len() {
            let f = &self.fighters[i];
            let Some(v) = f.hold.map(|v| v as usize) else { continue };
            if !f.grabbing || f.grab_static {
                continue;
            }
            let slot = f.hold_slot;
            let mass: f32 = self.fighters[v].particle_ids().map(|id| self.world.particles[id as usize].mass()).sum();
            let vf = &mut self.fighters[v];
            vf.held_by = Some((i as u8, slot));
            let carry = if vf.alive && vf.down == 0 { mass } else { mass * 0.6 };
            let f = &mut self.fighters[i];
            f.carry = carry;
            f.hold_t = f.hold_t.saturating_add(1);
        }
        for v in 0..self.fighters.len() {
            let vf = &self.fighters[v];
            let Some((g, _)) = vf.held_by else {
                self.fighters[v].struggle = self.fighters[v].struggle.saturating_sub(4);
                continue;
            };
            if !vf.alive || vf.down > 0 || self.in_finisher(v) {
                continue;
            }
            let mash = [LEFT, RIGHT, JUMP, CROUCH].iter().filter(|&&b| vf.pressed(b)).count() as u8;
            let gain = 1 + mash * STRUGGLE_PER_PRESS;
            let vf = &mut self.fighters[v];
            vf.struggle = vf.struggle.saturating_add(gain);
            if vf.struggle >= 100 {
                vf.struggle = 0;
                self.break_free(v, g as usize);
            }
        }
    }

    /// Wrenched out of a hold: both stumble apart and the grabber is rocked.
    fn break_free(&mut self, v: usize, g: usize) {
        self.release_grab(g);
        let away = self.fighters[v].pos(&self.world, PELVIS) - self.fighters[g].pos(&self.world, PELVIS);
        let dir = Vec2::new(if away.x >= 0.0 { 1.0 } else { -1.0 }, 0.0);
        let vi = self.fighters[v].particle_ids();
        self.world.add_velocity(vi, dir * 3.5 + Vec2::new(0.0, 1.0));
        let gi = self.fighters[g].particle_ids();
        self.world.add_velocity(gi, -dir * 2.5);
        let gf = &mut self.fighters[g];
        gf.stagger = gf.stagger.max(28);
        gf.grab_cd = 45;
        let at = self.fighters[v].pos(&self.world, fighter::NECK);
        self.events.push(Sfx::Punch.at(at).vol(0.8));
    }

    fn actions(&mut self, fi: usize) {
        if self.fighters[fi].choking {
            return;
        }
        let f = &self.fighters[fi];
        if !f.alive {
            return;
        }
        let strength = f.strength();
        // grab with the free hand while the button is held
        if f.wants_grab() && !f.grabbing && f.arm_b_ok() && f.grab_cd == 0 && strength > 0.1 {
            self.try_grab(fi);
        }
        // Q with someone in hand throws them (or disarms them, held by the
        // weapon arm); the heave lands a few frames later
        let f = &self.fighters[fi];
        if f.hold.is_some() && f.grabbing && !self.in_finisher(fi) {
            if f.pressed(THROW) && strength > 0.3 && self.heave_or_disarm(fi) {
                return; // (that Q took the weapon; it doesn't throw it too)
            }
            let f = &mut self.fighters[fi];
            if f.heave > 0 {
                f.heave -= 1;
                if f.heave == 0 {
                    self.throw_held(fi);
                }
            }
        }
        let f = &self.fighters[fi];
        // (a finisher holds on by itself while the keys are switched off)
        if f.grabbing && !self.in_finisher(fi) && (!f.wants_grab() || !f.arm_b_ok() || (strength <= 0.0 && !f.grab_static)) {
            self.release_grab(fi);
        }
        let f = &self.fighters[fi];
        if f.grabbing && f.grab_static && f.pressed(JUMP) {
            // pull up over a ledge in reach (the way you're pushing, or
            // facing) and onto it; off a sheer wall, a hop up it; off a rope,
            // a leap (keeping the swing)
            let on_rope = self.world.distances[f.tether as usize].enabled;
            let toward = if f.held(RIGHT) != f.held(LEFT) { if f.held(RIGHT) { 1.0 } else { -1.0 } } else { f.facing };
            let lip = if on_rope { None } else { self.ledge_ahead(fi, toward) };
            self.release_grab(fi);
            let ids = self.fighters[fi].particle_ids();
            if let Some((edge, top)) = lip {
                let v = self.haul_velocity(fi, edge, top, toward, CLIMB_LOSS);
                for id in ids {
                    self.world.particles[id as usize].vel = v;
                }
                let f = &mut self.fighters[fi];
                // (no catching the wall again on the way up, and no cutting
                // the climb short by letting go of jump)
                f.grab_cd = CLIMB_FRAMES;
                f.jump_cd = CLIMB_FRAMES;
                f.facing = toward;
            } else {
                self.world.add_velocity(ids, Vec2::new(0.0, if on_rope { 6.5 } else { 9.5 }));
            }
        }
        let f = &self.fighters[fi];
        if f.grabbing && f.held(CROUCH) && self.world.distances[f.tether as usize].enabled {
            // slide down the rope
            let t = &mut self.world.distances[f.tether as usize];
            t.rest = (t.rest + 2.5 * DT).min(12.0);
        }
        if let Some(wi) = self.hanging_from(fi) {
            let f = &mut self.fighters[fi];
            // hanging off your own stuck blade counts as touching a wall
            f.wall_jumps = 0;
            if f.pressed(JUMP) && f.jump_cd == 0 && strength > 0.5 {
                f.jump_cd = 14;
                let ids = f.particle_ids();
                self.world.add_velocity(ids, Vec2::new(0.0, CLIMB_PULL));
                // the heave rips the blade out on the way up, ready to stab higher
                let mut w = self.weapons[wi].clone();
                let at = self.world.particles[w.particle(w.def().impale.map_or(0, |i| i.2)) as usize].pos;
                w.unembed(&mut self.world, Vec2::new(0.0, CLIMB_PULL * 0.6));
                self.weapons[wi] = w;
                self.spawn_sparks(at, Vec2::Y, 5, 3.0);
                self.events.push(Sfx::Unembed.at(at));
            }
        }
        let f = &self.fighters[fi];
        if f.pressed(PICKUP) && f.arm_a_ok() && strength > 0.1 {
            // in reach of the hand, or lying anywhere around your feet
            let probes = [HAND_A, PELVIS, fighter::KNEE_A, fighter::KNEE_B, fighter::FOOT_A, fighter::FOOT_B].map(|s| f.pos(&self.world, s));
            let mut best: Option<(f32, usize)> = None;
            for (wi, w) in self.weapons.iter().enumerate() {
                if !w.active || w.holder.is_some() {
                    continue;
                }
                let d = if w.embedded {
                    // a stuck weapon has to be grabbed by its handle
                    (self.world.particles[w.particle(w.def().grip) as usize].pos - probes[0]).length() + 0.3
                } else {
                    probes.iter().map(|&p| w.distance_to(&self.world, p)).fold(f32::MAX, f32::min)
                };
                if d < 1.0 && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, wi));
                }
            }
            match best {
                Some((_, wi)) => {
                    self.release(fi, false);
                    self.equip(fi, wi);
                }
                None => self.release(fi, false),
            }
        }
        let f = &self.fighters[fi];
        if f.pressed(FINISH) {
            if let Some(v) = self.finish_target(fi) {
                self.start_finisher(fi, v);
                return;
            }
        }
        let f = &self.fighters[fi];
        if let (true, Some(wi), None) = (f.pressed(THROW), f.weapon, f.hold) {
            if self.weapons[wi as usize].is_knives() {
                self.throw_knife(fi, wi as usize);
            } else {
                self.release(fi, true);
            }
        }
        let f = &self.fighters[fi];
        if let (true, Some(wi)) = (f.pressed(ATTACK) && f.strength() > 0.3, f.weapon) {
            if self.weapons[wi as usize].is_gun() {
                self.fire(fi, wi as usize);
            } else if self.weapons[wi as usize].is_grapple() {
                self.fire_hook(fi, wi as usize);
            }
        }
    }

    /// Pulls the trigger: a bullet leaves the muzzle along the barrel, and
    /// the gun kicks up and back.
    /// Which way a held gun points so its shot goes through the cursor: the
    /// line down the barrel (which sits above the hand) through the point
    /// the player is aiming at, a touch high for the bullet's drop. Close in
    /// (the gun's already at the cursor), just the way the arm points.
    fn gun_angle(&self, f: &Fighter, w: &Weapon) -> f32 {
        let d = w.def();
        let pt = |i: u8| Vec2::new(d.points[i as usize].x, d.points[i as usize].y);
        let barrel = (pt(d.aim.1) - pt(d.aim.0)).normalize_or(Vec2::X);
        // how far the barrel's line is above the grip (in the gun's own frame)
        let above = -barrel.perp_dot(pt(d.grip) - pt(d.aim.0));
        let side = if w.flip { -1.0 } else { 1.0 };
        let grip = self.world.particles[w.particle(d.grip) as usize].pos;
        let mut at = f.pos(&self.world, fighter::NECK) + f.aim;
        let dist = (at - grip).length();
        if dist < above.abs() * 2.0 + 0.25 {
            return f.weapon_angle();
        }
        let t = dist / BULLET_SPEED;
        at.y += 0.5 * BULLET_GRAVITY * t * t;
        let to = at - grip;
        let off = (side * above / to.length()).clamp(-0.9, 0.9);
        dmath::atan2(to.y, to.x) - dmath::atan2(off, (1.0 - off * off).sqrt())
    }

    /// The sound a gun makes going off.
    fn shot_sound(def: &weapon::WeaponDef) -> Sfx {
        if def.name == "PISTOL" { Sfx::PistolShot } else { Sfx::Shot }
    }

    fn fire(&mut self, fi: usize, wi: usize) {
        let w = &self.weapons[wi];
        if w.fire_cd > 0 {
            return;
        }
        let d = w.def();
        let muzzle = self.world.particles[w.particle(d.muzzle) as usize].pos;
        if w.ammo == 0 {
            self.weapons[wi].fire_cd = FIRE_COOLDOWN;
            self.events.push(Sfx::Empty.at(muzzle));
            return;
        }
        let back = self.world.particles[w.particle(d.aim.0) as usize].pos;
        let dir = (muzzle - back).normalize_or(Vec2::X);
        self.bullets.push(Bullet { pos: muzzle, vel: dir * BULLET_SPEED, owner: fi as u8, life: 60 });
        let w = &mut self.weapons[wi];
        w.ammo -= 1;
        w.fire_cd = FIRE_COOLDOWN;
        w.flash = 3;
        self.kick_gun(fi, wi, dir);
        self.spawn_sparks(muzzle, dir, 6, 6.0);
        self.events.push(Self::shot_sound(self.weapons[wi].def()).at(muzzle));
    }

    /// A shot's recoil: the gun flips up about the hand (the muzzle most), the
    /// arm is thrown up and back and gives way for a moment, and the shooter
    /// rocks back a little.
    fn kick_gun(&mut self, fi: usize, wi: usize, dir: Vec2) {
        let w = &self.weapons[wi];
        let d = w.def();
        let up = if dir.x >= 0.0 { dir.perp() } else { -dir.perp() };
        let grip = self.world.particles[w.particle(d.grip) as usize].pos;
        let len = (self.world.particles[w.particle(d.muzzle) as usize].pos - grip).length().max(0.05);
        for k in 0..w.n {
            let p = &mut self.world.particles[w.particle(k) as usize];
            let out = ((p.pos - grip).dot(dir) / len).clamp(0.0, 1.0);
            p.vel += -dir * GUN_KICK_BACK + up * (GUN_KICK_UP * (0.25 + out));
        }
        let f = &mut self.fighters[fi];
        f.recoil = fighter::RECOIL_FRAMES;
        let (hand, elbow) = (f.p(HAND_A), f.p(fighter::ELBOW_A));
        let ids = f.particle_ids();
        self.world.add_velocity([hand, elbow], -dir * GUN_KICK_BACK + up * (GUN_KICK_UP * 0.5));
        self.world.add_velocity(ids, -dir * 0.6);
    }

    /// Moves bullets along, sweeping each step for the first thing hit.
    fn update_bullets(&mut self) {
        let mut i = 0;
        while i < self.bullets.len() {
            let mut b = self.bullets[i];
            let dt = DT * self.time_scale();
            b.vel.y -= BULLET_GRAVITY * dt;
            let step = b.vel * dt;
            let mut best_t = self.world.raycast(b.pos, step, 1.0).map_or(f32::MAX, |h| h.t);
            let mut target: Option<(usize, u32)> = None;
            for (fi, f) in self.fighters.iter().enumerate() {
                if fi == b.owner as usize {
                    continue;
                }
                for part in 0..10u32 {
                    if !f.has(part) {
                        continue;
                    }
                    let sg = self.world.segments[(f.seg_base + part) as usize];
                    let (pa, pb) = (self.world.particles[sg.a as usize].pos, self.world.particles[sg.b as usize].pos);
                    let (_, t, c1, c2) = stickfell_physics::collide::closest_segments(pa, pb, b.pos, b.pos + step);
                    if (c1 - c2).length() < sg.radius + 0.03 && t < best_t {
                        best_t = t;
                        target = Some((fi, part));
                    }
                }
            }
            if best_t <= 1.0 {
                let at = b.pos + step * best_t;
                let dir = b.vel.normalize_or(Vec2::X);
                match target {
                    Some((fi, part)) => {
                        let loc = match part {
                            S_HEAD => 1.8,
                            S_TORSO => 1.0,
                            _ => 0.7,
                        };
                        self.events.push(Sfx::BulletHit.at(at));
                        self.wound(fi, part, at, fighter::WoundKind::Hole, 0.028, dir.perp());
                        if part == S_HEAD {
                            let head = self.fighters[fi].pos(&self.world, fighter::HEAD);
                            let r = self.world.segments[(self.fighters[fi].seg_base + S_HEAD) as usize].radius;
                            let out = head + dir * r * 0.8;
                            self.wound(fi, part, out, fighter::WoundKind::Exit, 0.05, dir.perp());
                            self.spawn_blood(out, dir, 10, 6.0);
                        }
                        self.damage(fi, part, BULLET_DAMAGE * loc, at, dir, 14.0, Kind::Spike, Some(b.owner));
                    }
                    None => {
                        self.spawn_sparks(at, -dir, 8, 5.0);
                        self.events.push(Sfx::Ricochet.at(at));
                    }
                }
                self.bullets.swap_remove(i);
                continue;
            }
            b.pos += step;
            b.life = b.life.saturating_sub(1);
            if b.life == 0 {
                self.bullets.swap_remove(i);
                continue;
            }
            self.bullets[i] = b;
            i += 1;
        }
    }

    // ---- damage ---------------------------------------------------------

    fn spawn_blood(&mut self, at: Vec2, dir: Vec2, n: u32, speed: f32) {
        for _ in 0..n {
            if self.sparks.len() >= 320 {
                break;
            }
            let a = dmath::angle(dir) + self.rng.range(-0.9, 0.9);
            let v = dmath::dir(a) * (speed * self.rng.range(0.3, 1.0)) + Vec2::new(0.0, 1.5);
            let life = 30 + (self.rng.next_u32() % 30) as u8;
            self.sparks.push(Spark { pos: at, vel: v, life, max_life: life, blood: true, tint: 0, steam: false });
        }
    }

    /// Leaves a wound in a body part at `at` (pulled onto the part if it's
    /// off the edge), running along `dir`.
    fn wound(&mut self, fi: usize, part: u32, at: Vec2, kind: fighter::WoundKind, size: f32, dir: Vec2) {
        use fighter::WoundKind;
        let f = &self.fighters[fi];
        if !f.has(part) {
            return;
        }
        let (c, axis) = f.part_frame(&self.world, part);
        let across = axis.perp();
        let sg = self.world.segments[(f.seg_base + part) as usize];
        // the part in its own frame: a line from -half to +half along x
        // (a point for the head), fattened by `room`
        let half = if part == S_HEAD { 0.0 } else { (self.world.particles[sg.b as usize].pos - self.world.particles[sg.a as usize].pos).length() * 0.5 };
        let room = sg.radius * fighter::WOUND_INSET;
        let core = |p: Vec2| Vec2::new(p.x.clamp(-half, half), 0.0);
        // smaller if even a centred one wouldn't fit
        let size = size.min(room / (kind.reach() + kind.length()).max(1.0));
        let width = size * kind.reach();
        let d = at - c;
        let mut local = Vec2::new(d.dot(axis), d.dot(across));
        // the centre in far enough for its round edge (or a slit's width)
        let off = local - core(local);
        if off.length() > room - width {
            local = core(local) + off.normalize_or_zero() * (room - width).max(0.0);
        }
        let dir = dir.normalize_or(Vec2::X);
        let mut run = Vec2::new(dir.dot(axis), dir.dot(across)) * size * kind.length();
        // a slit or gash shortened until both ends are on the part too
        let inside = |p: Vec2| (p - core(p)).length() <= room - width + 1e-5;
        for _ in 0..6 {
            if inside(local + run) && inside(local - run) {
                break;
            }
            run *= 0.8;
        }
        if !(inside(local + run) && inside(local - run)) {
            // still over: bring it in from the edge by its half-length (then
            // neither end can be further out than the centre was allowed)
            let len = run.length().min((room - width).max(0.0));
            run = run.normalize_or_zero() * len;
            let off = local - core(local);
            local = core(local) + off.normalize_or_zero() * off.length().min((room - width - len).max(0.0));
        }
        let bleed = match kind {
            WoundKind::Hole => 240,
            WoundKind::Exit => 420,
            WoundKind::Stab => 300,
            WoundKind::Cut => 200,
            WoundKind::Spatter => 0,
        };
        let wd = fighter::Wound { part: part as u8, kind, at: local, dir: run, size, born: self.frame, bleed };
        let f = &mut self.fighters[fi];
        if f.wounds.len() >= fighter::MAX_WOUNDS {
            f.wounds.remove(0);
        }
        f.wounds.push(wd);
    }

    /// Fresh wounds drip: a drop now and then, falling to splat on the floor.
    fn bleed(&mut self) {
        for fi in 0..self.fighters.len() {
            for k in 0..self.fighters[fi].wounds.len() {
                let wd = self.fighters[fi].wounds[k];
                if wd.bleed == 0 {
                    continue;
                }
                self.fighters[fi].wounds[k].bleed -= 1;
                // (the fresher, the faster it drips)
                let every = if wd.bleed > 150 { 7 } else { 13 };
                if (self.frame + k as u32 * 5 + fi as u32 * 3) % every != 0 || self.sparks.len() >= 320 {
                    continue;
                }
                let f = &self.fighters[fi];
                if !f.has(wd.part as u32) {
                    continue;
                }
                let (at, _) = f.wound_at(&self.world, &wd);
                let vel = Vec2::new(self.rng.range(-0.25, 0.25), -0.3);
                self.sparks.push(Spark { pos: at, vel, life: 50, max_life: 50, blood: true, tint: 0, steam: false });
            }
        }
    }

    fn spawn_sparks(&mut self, at: Vec2, dir: Vec2, n: u32, speed: f32) {
        for _ in 0..n {
            if self.sparks.len() >= 320 {
                break;
            }
            let a = dmath::angle(dir) + self.rng.range(-1.4, 1.4);
            let v = dmath::dir(a) * (speed * self.rng.range(0.35, 1.0));
            let life = 10 + (self.rng.next_u32() % 14) as u8;
            self.sparks.push(Spark { pos: at, vel: v, life, max_life: life, blood: false, tint: 0, steam: false });
        }
    }

    fn kill_fighter(&mut self, fi: usize) {
        self.release(fi, false);
        self.release_grab(fi);
        let mut f = self.fighters[fi].clone();
        f.kill(&mut self.world);
        self.fighters[fi] = f;
    }

    fn sever(&mut self, fi: usize, part: u32) -> bool {
        let f = self.fighters[fi].clone();
        if !f.has(part) {
            return false;
        }
        if part == S_HEAD {
            self.world.distances[(f.dist_base + 9) as usize].enabled = false;
            f.relax(&mut self.world, Fighter::muscle_of(S_HEAD));
            let head = f.p(fighter::HEAD) as usize;
            self.world.particles[head].vel += Vec2::new(self.rng.range(-3.0, 3.0), 7.0);
            let fm = &mut self.fighters[fi];
            fm.severed |= fighter::bit(S_HEAD);
            fm.bleed = 300;
            fm.bleed_at = f.p(fighter::NECK);
            return true;
        }
        let Some((joint_slot, parts)) = fighter::sever_plan(part) else { return false };
        if parts.iter().any(|&p| !f.has(p)) {
            return false;
        }
        let joint = f.p(joint_slot);
        let jp = self.world.particles[joint as usize];
        let mut np = Particle::new(jp.pos, 0.4, 0.07);
        np.vel = jp.vel;
        np.prev = jp.prev;
        let new_id = self.world.add_particle(np);
        let first = parts[0];
        self.world.segments[(f.seg_base + first) as usize].a = new_id;
        self.world.distances[(f.dist_base + first - 1) as usize].a = new_id;
        for &p in parts {
            f.relax(&mut self.world, Fighter::muscle_of(p));
        }
        {
            let fm = &mut self.fighters[fi];
            for &p in parts {
                fm.severed |= fighter::bit(p);
            }
            fm.bleed = 220;
            fm.bleed_at = joint;
        }
        if parts.iter().any(|&p| fighter::ARM_A_BITS & fighter::bit(p) != 0) {
            self.release(fi, false);
        }
        if parts.iter().any(|&p| fighter::ARM_B_BITS & fighter::bit(p) != 0) {
            self.release_grab(fi);
            let g = self.fighters[fi].grip_b;
            self.world.distances[g as usize].enabled = false;
        }
        true
    }

    /// Shakes fighter `fi`'s player's camera (nobody else's), up to
    /// `SHAKE_MAX`.
    fn shake_camera(&mut self, fi: usize, amount: f32) {
        if let Some(k) = self.shake.get_mut(fi) {
            *k = (*k + amount).min(SHAKE_MAX);
        }
    }

    /// Breaks the bone of arm or leg part `part` (anything else: nothing): a
    /// crack, a cry, and the limb goes limp; whatever the hand held, it lets go of.
    fn break_bone(&mut self, fi: usize, part: u32, at: Vec2) {
        let f = &self.fighters[fi];
        if !(fighter::S_UPPER_A..=fighter::S_SHIN_B).contains(&part) || !f.has(part) || f.broken & fighter::bit(part) != 0 || f.limbs_gone & fighter::bit(part) != 0 {
            return;
        }
        self.fighters[fi].broken |= fighter::bit(part);
        if fighter::ARM_A_BITS & fighter::bit(part) != 0 {
            self.release(fi, false);
        }
        if fighter::ARM_B_BITS & fighter::bit(part) != 0 {
            self.release_grab(fi);
        }
        self.events.push(Sfx::HitBlunt.at(at).vol(1.3));
        self.spawn_blood(at, Vec2::Y, 10, 3.0);
        if self.fighters[fi].alive {
            self.wince(fi);
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// On the end of a tossed grapple, before it lands: untouchable.
    pub fn tossed(&self, fi: usize) -> bool {
        self.nooses.iter().any(|n| n.toss && n.victim as usize == fi)
    }

    fn damage(&mut self, fi: usize, part: u32, dmg: f32, point: Vec2, dir: Vec2, knock: f32, kind: Kind, attacker: Option<u8>) {
        if self.tossed(fi) {
            return;
        }
        // (teammates can knock each other about, but not hurt each other)
        if attacker.is_some_and(|a| self.allies(a as usize, fi) && a as usize != fi) {
            return;
        }
        let was_alive = self.fighters[fi].alive;
        let k = knock.min(40.0);
        {
            let f = &mut self.fighters[fi];
            if was_alive {
                f.health -= dmg;
                f.hit_flash = 6;
                if dmg > 12.0 || k > 14.0 {
                    // a real hit: limp ragdoll, longer the harder it was
                    f.knock_down((20.0 + k * 1.6 + dmg * 0.4).min(100.0) as u16);
                } else if k > 4.0 {
                    f.stagger = f.stagger.max((6.0 + k * 1.5) as u8);
                }
                if f.health > 0.0 && f.health <= FINISH_HP {
                    // seeing stars: wide open for a finisher for a while
                    f.dazed = DAZE_FRAMES;
                }
            }
        }
        if dmg >= HOLD_BREAK_DAMAGE && self.fighters[fi].hold.is_some() && !self.in_finisher(fi) {
            self.release_grab(fi);
        }
        self.shove(fi, part, dir, k);
        let n = (dmg * 0.5).clamp(3.0, 22.0) as u32;
        self.spawn_blood(point, dir, n, 3.0 + knock * 0.25);
        let mut severed = false;
        let threshold = if part == S_HEAD { 26.0 } else { 14.0 };
        if kind == Kind::Blade && dmg >= threshold && part != S_TORSO && self.rng.f32() < 0.55 {
            severed = self.sever(fi, part);
            if severed {
                self.events.push(Sfx::Sever.at(point));
                self.spawn_blood(point, dir, 16, 7.0);
            }
        }
        // a hard enough blow (or slam) now and then breaks the bone it lands on
        if !severed && kind == Kind::Blunt && (dmg > BREAK_DAMAGE || k > BREAK_KNOCK) && self.rng.f32() < BREAK_CHANCE {
            self.break_bone(fi, part, point);
        }
        let f = &self.fighters[fi];
        let lethal = was_alive && (f.health <= 0.0 || (severed && part == S_HEAD));
        if lethal {
            self.kill_fighter(fi);
        }
        self.shake_camera(fi, dmg * 0.03);
        if let Some(a) = attacker {
            self.shake_camera(a as usize, dmg * 0.015);
        }
        self.hits.push(HitEvent { attacker, victim: fi as u8, point, damage: dmg, severed, lethal });
    }

    /// How fast the world runs this frame: 1, or slower around a finishing
    /// blow (easing back to full speed at the end).
    pub fn time_scale(&self) -> f32 {
        match self.slowmo {
            0 => 1.0,
            n if n > 20 => SLOWMO_SCALE,
            n => SLOWMO_SCALE + (1.0 - SLOWMO_SCALE) * (1.0 - n as f32 / 20.0),
        }
    }

    pub fn in_finisher(&self, fi: usize) -> bool {
        self.finishers.iter().any(|x| x.attacker as usize == fi || x.victim as usize == fi)
    }

    /// Down, badly hurt, and not already busy: can be finished off.
    pub fn finishable(&self, vi: usize) -> bool {
        let v = &self.fighters[vi];
        v.alive && !v.choking && !self.tossed(vi) && (v.down > 0 || v.dazed > 0) && v.health <= FINISH_HP && !self.in_finisher(vi) && v.impaled == 0 && !self.weapons.iter().any(|w| w.victim == Some(vi as u8)) && self.clash.is_none_or(|c| c.side_of(vi).is_none())
    }

    /// The finishable fighter `fi` could finish right now, if any.
    pub fn finish_target(&self, fi: usize) -> Option<usize> {
        let f = &self.fighters[fi];
        if !f.alive || f.down > 0 || f.strength() < 0.6 || self.in_finisher(fi) || self.clash.is_some_and(|c| c.side_of(fi).is_some()) {
            return None;
        }
        let me = f.pos(&self.world, PELVIS);
        (0..self.fighters.len())
            .filter(|&v| !self.allies(fi, v) && self.finishable(v))
            // (already tied to the hook in your hand: toss it instead)
            .filter(|&v| !self.nooses.iter().any(|n| !n.cut && n.hook == Hook::Loaded && n.victim as usize == v && Some(n.weapon) == f.weapon))
            .map(|v| (v, (self.fighters[v].pos(&self.world, PELVIS) - me).length()))
            .filter(|&(_, d)| d < FINISH_RANGE)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(v, _)| v)
    }

    fn start_finisher(&mut self, fi: usize, vi: usize) {
        // a spent grapple gun is just junk: let it go and use your hands
        if self.fighters[fi].weapon.is_some_and(|wi| self.weapons[wi as usize].is_grapple() && self.weapons[wi as usize].ammo == 0) {
            self.release(fi, false);
        }
        let mut mv = match self.fighters[fi].weapon {
            Some(wi) => self.weapons[wi as usize].def().finish,
            // bare hands: tear them apart, one way or the other
            None if self.rng.next_u32() % 2 == 0 => Finish::Dismember,
            None => Finish::Spine,
        };
        if mv == Finish::Javelin {
            // the throw needs room to back off; otherwise (or on a coin flip) run them through
            let me = self.fighters[fi].pos(&self.world, PELVIS);
            let away = if self.fighters[vi].pos(&self.world, PELVIS).x >= me.x { -1.0 } else { 1.0 };
            let clear = self.world.raycast(me, Vec2::new(away, 0.0), 3.5).is_none();
            let floor = self.world.raycast(me + Vec2::new(away * 3.0, 0.0), Vec2::NEG_Y, 2.0).is_some();
            if !(clear && floor) || self.rng.next_u32() % 2 == 1 {
                mv = Finish::Lift;
            }
        }
        // just thrown over your shoulder: stomp on their head, whatever's in your hand
        let vf = &self.fighters[vi];
        if vf.shouldered > 0 && vf.shouldered_by as usize == fi {
            mv = Finish::Stomp;
        }
        let weapon = self.fighters[fi].weapon;
        // a sword with a wall at their back: pin them to it instead
        let wall = if mv == Finish::Behead { self.wall_behind(fi, vi) } else { None };
        if wall.is_some() {
            mv = Finish::Pin;
        }
        let side = Self::head_side(&self.fighters[vi], &self.world) as i8;
        self.finishers.push(Finisher { attacker: fi as u8, victim: vi as u8, mv, t: 0, weapon, killed: false, wall, side });
        // the performance can't land a stray hit early: only the scripted strikes count
        // (and a pickup's grace running out mustn't undo that)
        if let Some(wi) = self.fighters[fi].weapon {
            self.weapons[wi as usize].pass_grace = 0;
            let w = self.weapons[wi as usize].clone();
            w.set_pass_through(&mut self.world, vi as u16 + 1);
        }
        // grab them by the neck with the free hand
        if mv.holds() && self.fighters[fi].arm_b_ok() {
            self.release_grab(fi);
            let f = &self.fighters[fi];
            let (hb, grab, grip_b) = (f.p(HAND_B), f.grab, f.grip_b);
            let neck = self.fighters[vi].p(fighter::NECK);
            self.world.distances[grab as usize] = Distance { a: hb, b: neck, rest: 0.0, compliance: 3e-4, enabled: true };
            self.world.distances[grip_b as usize].enabled = false;
            let f = &mut self.fighters[fi];
            f.grabbing = true;
            f.grab_static = false;
            f.hold = Some(vi as u8);
            f.hold_slot = fighter::NECK as u8;
        }
        let at = self.fighters[vi].pos(&self.world, fighter::NECK);
        self.events.push(Sfx::Pickup.at(at));
        self.events.push(Sfx::Plea.at(at).line(0, vi as u8));
    }

    /// A wall right behind the victim (on the far side from the attacker),
    /// tall enough to pin a body to: its point at the victim's chest height.
    fn wall_behind(&self, fi: usize, vi: usize) -> Option<Vec2> {
        let w = &self.world;
        let v = &self.fighters[vi];
        let side = if v.pos(w, PELVIS).x >= self.fighters[fi].pos(w, PELVIS).x { 1.0 } else { -1.0 };
        let dir = Vec2::new(side, 0.0);
        // measured from where they'd stand: they may be lying in a heap
        let feet = v.pos(w, PELVIS) - Vec2::new(0.0, 0.9);
        let chest = feet + Vec2::new(0.0, 1.25);
        let hit = w.raycast(chest, dir, PIN_WALL_RANGE)?;
        // a wall facing them, and solid from the knees to above the head
        // (not a crate they'd just be bent over)
        if hit.normal.x * side > -0.7 {
            return None;
        }
        for h in [0.55, 1.9] {
            let from = feet + Vec2::new(0.0, h);
            match w.raycast(from, dir, PIN_WALL_RANGE + 0.3) {
                Some(o) if (o.t - hit.t).abs() < 0.3 && o.normal.x * side < -0.7 => {}
                _ => return None,
            }
        }
        Some(hit.point)
    }

    /// Where the attacker's hand goes at this point of the finisher, and
    /// which way they face.
    fn finisher_script(&self, x: &Finisher) -> (HandScript, f32) {
        let (a, v) = (&self.fighters[x.attacker as usize], &self.fighters[x.victim as usize]);
        let w = &self.world;
        let neck = a.pos(w, fighter::NECK);
        let f = if v.pos(w, PELVIS).x >= a.pos(w, PELVIS).x { 1.0 } else { -1.0 };
        let torso = (v.pos(w, fighter::NECK) + v.pos(w, PELVIS)) * 0.5;
        let head = v.pos(w, fighter::HEAD);
        let to_torso = dmath::angle(torso - neck);
        let to_head = dmath::angle(head - neck);
        let up = core::f32::consts::FRAC_PI_2;
        let t = (x.t.saturating_sub(x.mv.lead_in()) as f32 / FINISH_TEMPO) as u16;
        let high = x.mv == Finish::Spine && x.t >= x.mv.hits()[0];
        let chest = torso + Vec2::new(0.0, 0.15 * v.body.torso_k());
        let to_chest = dmath::angle(chest - neck);
        let ft = x.t;
        let kneel = matches!(x.mv, Finish::Hang | Finish::Execute);
        // (reaches are for the game's arm; they stretch with the attacker's)
        let ak = a.body.arm_k();
        let shove = x.mv == Finish::Pin && ft < PIN_HIT;
        // a stomp faces back at the head from beyond it
        let f = if x.mv == Finish::Stomp { -(x.side as f32) } else { f };
        let hs = |ang: f32, reach: f32, omega: f32| HandScript { ang: dmath::wrap_angle(ang), reach: reach * ak, omega, hold: x.mv.holds(), high, kneel, shove, aim: None, leg: None };
        let sc = match x.mv {
            Finish::Behead if t < 28 => hs(to_head + f * 2.2, 0.5, 18.0),
            Finish::Behead => hs(to_head - f * 0.7, 0.64, 32.0),
            // spear throw: hold them up, back off with it cocked over the shoulder, launch
            Finish::Javelin if ft < 70 => hs(up + f * 0.3, 0.5, 10.0),
            Finish::Javelin if ft < 140 => hs(to_chest + f * 2.5, 0.6, 8.0),
            Finish::Javelin => hs(to_chest, 0.64, 30.0),
            // spear lift: draw back, jam it in, raise them high, then whip it down
            Finish::Lift if ft < 70 => hs(to_chest + f * 0.8, 0.45, 10.0),
            Finish::Lift if ft < 110 => hs(to_chest, 0.22, 6.0),
            Finish::Lift if ft < 116 => hs(to_chest, 0.75, 34.0),
            Finish::Lift if ft < 154 => hs(up - f * 0.35, 0.72, 4.0),
            Finish::Lift => hs(-up + f * 0.8, 0.64, 45.0),
            Finish::Smash if t < 30 => hs(up + f * 0.9, 0.5, 16.0),
            Finish::Smash => hs(to_torso - f * 0.25, 0.64, 30.0),
            // flail: hold them up, let them stand, whirl it up, launch them
            Finish::Spin if ft < 70 => hs(to_torso + f * 0.8, 0.45, 10.0),
            // flail held ready while they sway, then whirled up, faster and faster
            Finish::Spin if ft < 100 => hs(to_torso + f * 1.3, 0.5, 6.0),
            Finish::Spin if ft < 150 => {
                let k = (ft - 100) as f32;
                hs(to_torso + f * (1.3 + k * 0.12 + k * k * 0.0045), 0.62, 40.0)
            }
            Finish::Spin => hs(to_torso - f * 0.3, 0.64, 30.0),
            Finish::Slap => {
                let swing = match t {
                    0..12 => 1.4,
                    12..22 => -0.9,
                    22..30 => 1.4,
                    30..40 => -0.9,
                    40..50 => 2.4,
                    _ => -1.1,
                };
                hs(to_head + f * swing, 0.6, 30.0)
            }
            Finish::Bonk if t < 26 => hs(up + f * 0.5, 0.55, 18.0),
            Finish::Bonk => hs(to_head, 0.64, 30.0),
            Finish::Execute => {
                // on their knees, the muzzle pressed down onto the back of the
                // head: the hand above and behind it, the gun angled down
                let gun = a.weapon.map(|wi| self.weapons[wi as usize].def()).filter(|d| d.class == weapon::Class::Gun);
                let reach = gun.map_or(0.3, |d| {
                    let p = |i: u8| Vec2::new(d.points[i as usize].x, d.points[i as usize].y);
                    let axis = (p(d.aim.1) - p(d.aim.0)).normalize_or(Vec2::X);
                    (p(d.muzzle) - p(d.grip)).dot(axis)
                });
                let head_r = w.segments[(v.seg_base + S_HEAD) as usize].radius;
                let from = Vec2::new(-f * 0.5, 0.866);
                let hand = head + from * (head_r + reach + 0.01) - neck;
                HandScript { aim: Some(dmath::angle(-from)), ..hs(dmath::angle(hand), (hand.length() / ak).min(0.64), 12.0) }
            }
            Finish::Shank => hs(to_torso, if (t / 6) % 2 == 0 { 0.3 } else { 0.75 }, 20.0),
            Finish::Dismember => {
                // reach for the next limb, then yank it away
                let hits = x.mv.hits();
                let done = hits.iter().filter(|&&h| x.t >= h).count();
                let just_ripped = hits.iter().any(|&h| x.t >= h && x.t < h + 7);
                match Self::next_limb(v, done) {
                    _ if just_ripped => hs(dmath::angle(Vec2::new(-f, 0.4)), 0.5, 30.0),
                    Some(slot) => {
                        let to = v.pos(w, slot) - neck;
                        hs(dmath::angle(to), (to.length() / ak).clamp(0.3, 0.64), 24.0)
                    }
                    None => hs(to_torso, 0.5, 20.0),
                }
            }
            Finish::Spine if x.t < x.mv.hits()[0] => hs(to_torso, 0.55, 20.0),
            Finish::Spine => hs(up, 0.64, 18.0), // the trophy held high
            // the hook kept down and back out of the way while the free hand
            // pushes them down and ties the rope on
            Finish::Hang => hs(-up - f * 0.7, 0.42, 10.0),
            // the sword held down and back behind you, well clear of them,
            // while you shove them against the wall; then you let go, step back
            // and flip it round over your shoulder (in clear air) to point at
            // their chest, cock it back, and lunge in to drive it through
            // (a negative reach puts the hand behind the neck, the blade still
            // pointing the way of the angle)
            Finish::Pin if ft < PIN_LET_GO => hs(dmath::angle(Vec2::new(-f, -0.45)), 0.42, 8.0),
            Finish::Pin if ft < PIN_LET_GO + 8 => hs(up, 0.25, 18.0),
            Finish::Pin if ft < PIN_LET_GO + 18 => hs(to_chest, 0.1, 14.0),
            Finish::Pin if ft < PIN_HIT - 6 => hs(to_chest, -0.15, 5.0),
            Finish::Pin if ft <= PIN_HIT => hs(to_chest, 0.85, 36.0),
            Finish::Pin => hs(to_chest - f * 0.5, 0.35, 6.0),
            Finish::Stomp => {
                // the heel placed by working the leg back from where it should
                // be (the knee bent forward to fit): lifted straight up beside
                // you, carried over their head, then driven straight down onto
                // it; up over it again, higher, and down again. It never sweeps
                // along the floor into them.
                let (hip, knee, foot) = (a.pos(w, PELVIS), a.pos(w, fighter::KNEE_A), a.pos(w, fighter::FOOT_A));
                let (lt, ls) = ((knee - hip).length().max(0.1), (foot - knee).length().max(0.1));
                let ik = |target: Vec2| {
                    let d = (target - hip).length().clamp(0.05, (lt + ls) * 0.999);
                    // (acos from the deterministic atan2, for rollback)
                    let cos_rule = |near: f32, far: f32| {
                        let c = ((near * near + d * d - far * far) / (2.0 * near * d)).clamp(-1.0, 1.0);
                        dmath::angle(Vec2::new(c, (1.0 - c * c).max(0.0).sqrt()))
                    };
                    let (at_hip, at_foot) = (cos_rule(lt, ls), cos_rule(ls, lt));
                    (dmath::angle(target - hip) + f * at_hip, at_hip + at_foot)
                };
                let leg = self.stomp_foot(x).map(|(target, _)| ik(target));
                HandScript { leg, ..hs(dmath::angle(Vec2::new(-f, 0.25)), 0.45, 8.0) }
            }
        };
        (sc, f)
    }

    fn update_finishers(&mut self) {
        if let Some((m, a, t)) = self.finish_banner {
            let at = self.fighters[a as usize].pos(&self.world, fighter::NECK);
            if t == FINISH_BANNER {
                self.events.push(Sfx::Announce.at(at).line(m as u8, a));
            } else if t == FINISH_BANNER - QUIP_DELAY && self.fighters[a as usize].alive {
                self.events.push(Sfx::Quip.at(at).line(m as u8, a));
            }
            self.finish_banner = (t > 1).then_some((m, a, t - 1));
        }
        let mut i = 0;
        while i < self.finishers.len() {
            let mut x = self.finishers[i];
            let (a, v) = (x.attacker as usize, x.victim as usize);
            // interrupted: the attacker got knocked down, or someone else got the kill first
            // (a thrown spear killing them on its own still counts as the finisher landing)
            let thrown = x.mv == Finish::Javelin && x.t >= x.mv.blow();
            let broken = !self.fighters[a].alive || self.fighters[a].down > 0 || (!x.killed && !thrown && !self.fighters[v].alive);
            if broken || x.t >= x.mv.length() {
                self.finishers.remove(i);
                self.release_grab(a);
                if let Some(wi) = self.fighters[a].weapon {
                    let w = self.weapons[wi as usize].clone();
                    if w.victim.is_none() {
                        w.set_pass_through(&mut self.world, 0);
                    }
                }
                continue;
            }
            // in slow motion the performance slows down with everything else
            let ticked = self.slowmo == 0 || self.frame % 3 == 0;
            if ticked {
                x.t += 1;
            }
            self.finishers[i] = x;
            let lead = if x.mv == Finish::Javelin { 2 } else { SLOWMO_LEAD };
            if x.t + lead == x.mv.blow() && self.slowmo == 0 && x.mv != Finish::Hang {
                self.slowmo = SLOWMO_FRAMES;
            }
            // moves that don't grab: step in to striking distance first
            if let (true, Some((target, speed))) = (x.mv == Finish::Stomp, self.stomp_foot(&x)) {
                // the heel carried along its path (the leg's muscles only
                // shape it: driving it with them would heave the body up)
                let p = &mut self.world.particles[self.fighters[a].p(fighter::FOOT_A) as usize];
                let mut want = (target - p.pos) * 14.0;
                if want.length() > speed {
                    want *= speed / want.length();
                }
                p.vel = p.vel.lerp(want, 0.5);
            }
            if x.mv == Finish::Stomp {
                // eased to just beyond their head (away from their body),
                // never while a stomp is coming down
                let [first, second] = weapon::STOMP_HITS;
                // (a shuffle: a little each frame, which standing still can't undo)
                if ticked && (x.t < first - 10 || (first + 8..second - 10).contains(&x.t)) {
                    let spot = self.fighters[v].pos(&self.world, fighter::HEAD).x + x.side as f32 * STOMP_STAND;
                    let dx = (spot - self.fighters[a].pos(&self.world, PELVIS).x).clamp(-STOMP_SHUFFLE, STOMP_SHUFFLE);
                    for id in self.fighters[a].particle_ids() {
                        let p = &mut self.world.particles[id as usize];
                        p.pos.x += dx;
                        p.prev.x += dx;
                    }
                }
            }
            if !x.mv.holds() && x.mv != Finish::Stomp && x.t + 12 < x.mv.hits()[0] {
                let gap = self.fighters[v].pos(&self.world, PELVIS).x - self.fighters[a].pos(&self.world, PELVIS).x;
                if gap.abs() > 1.2 {
                    let want = gap.signum() * 4.0;
                    for id in self.fighters[a].particle_ids() {
                        let p = &mut self.world.particles[id as usize];
                        p.vel.x += (want - p.vel.x) * 0.3;
                    }
                }
            }
            // the victim stays limp (hoisted when held), or - for the moves that
            // stand them up - sways on their feet, barely upright
            let standing = x.mv.stand_at().is_some_and(|s| x.t >= s) && !x.killed;
            if self.fighters[v].alive {
                let fv = &mut self.fighters[v];
                if standing {
                    fv.down = 0;
                    fv.dazed = fv.dazed.max(60);
                    let (head, neck) = (fv.p(fighter::HEAD), fv.p(fighter::NECK));
                    let g = self.world.cfg.gravity;
                    for id in [head, neck] {
                        let p = &mut self.world.particles[id as usize];
                        p.force -= g * (p.mass() * 0.5);
                    }
                } else {
                    fv.down = fv.down.max(20);
                }
            }
            if ticked && x.mv == Finish::Pin && x.t == PIN_LET_GO {
                // let go: they stay slumped back against the wall
                self.release_grab(a);
            }
            if ticked && x.mv == Finish::Pin && x.t == PIN_HIT - 6 {
                // the lunge that drives it home
                let fa = &self.fighters[a];
                let toward = if x.wall.is_some_and(|w| w.x >= fa.pos(&self.world, PELVIS).x) { 1.0 } else { -1.0 };
                let ids = fa.particle_ids();
                self.world.add_velocity(ids, Vec2::new(toward * PIN_LUNGE, 0.0));
            }
            let pressed_back = x.mv == Finish::Pin && x.wall.is_some() && x.t < PIN_HIT;
            if (x.mv.holds() && self.fighters[a].grabbing) || pressed_back {
                // hoisted up by the neck to arm's length in front of the attacker
                let fa = &self.fighters[a];
                let side = if self.fighters[v].pos(&self.world, PELVIS).x >= fa.pos(&self.world, PELVIS).x { 1.0 } else { -1.0 };
                let trophy = x.mv == Finish::Spine && x.t >= x.mv.hits()[0];
                let kneel = matches!(x.mv, Finish::Hang | Finish::Execute);
                let neck_a = fa.pos(&self.world, fighter::NECK);
                let mut hold = neck_a
                    + if trophy {
                        Vec2::new(side * 0.3, 1.0)
                    } else if kneel {
                        // down on their knees in front of you
                        Vec2::new(side * 0.55, -0.5)
                    } else {
                        Vec2::new(side * 0.6, -0.05)
                    } * fa.body.arm_k();
                if let (Finish::Pin, Some(wall)) = (x.mv, x.wall) {
                    // shoved back until their shoulders hit the wall
                    let flat = Vec2::new(wall.x - side * PIN_GAP, hold.y);
                    let k = ((x.t as f32 - 24.0) / 34.0).clamp(0.0, 1.0);
                    hold = hold.lerp(flat, k * k * (3.0 - 2.0 * k));
                }
                let g = self.world.cfg.gravity;
                let fv = &self.fighters[v];
                let head_up = Vec2::new(0.0, 0.3 * fv.body.neck_k());
                for id in fv.particle_ids() {
                    let slot = id - fv.base;
                    // once the spine is out, only head and spine are still in hand
                    if fv.spine_out && slot != fighter::NECK && slot != fighter::HEAD {
                        continue;
                    }
                    let p = &mut self.world.particles[id as usize];
                    // (kneeling, the body's weight sits on the knees)
                    p.force -= g * (p.mass() * if kneel { 0.15 } else { 0.6 });
                    if slot == fighter::NECK || slot == fighter::HEAD {
                        let target = if slot == fighter::HEAD { hold + head_up } else { hold };
                        // hauled up steadily, not snapped upright
                        let mut want = (target - p.pos) * 5.0;
                        let fast = want.length();
                        if fast > 1.8 {
                            want *= 1.8 / fast;
                        }
                        p.vel = p.vel.lerp(want, 0.25);
                    }
                }
            }
            if let (true, Some(k)) = (ticked, x.mv.hits().iter().position(|&h| h == x.t)) {
                self.finisher_hit(x, k);
                if !self.fighters[v].alive {
                    self.finishers[i].killed = true;
                }
            }
            i += 1;
        }
    }

    /// Thrown spear finishers: the kill lands the moment the spear reaches
    /// them (any hit on them, run through or not), or a while after the throw
    /// if it somehow sails past. Runs after this frame's hits are resolved.
    fn land_javelins(&mut self) {
        for i in 0..self.finishers.len() {
            let x = self.finishers[i];
            if x.mv != Finish::Javelin || x.t < x.mv.blow() || x.killed {
                continue;
            }
            let (a, v) = (x.attacker as usize, x.victim as usize);
            let them = self.fighters[v].pos(&self.world, PELVIS);
            let landed = x.weapon.is_some_and(|wi| {
                let wp = &self.weapons[wi as usize];
                let tip = wp.def().impale.map_or(0, |i| i.2);
                let chest = (self.fighters[v].pos(&self.world, fighter::NECK) + them) * 0.5;
                wp.victim == Some(v as u8) || (self.world.particles[wp.particle(tip) as usize].pos - chest).length() < 0.7
            }) || self.hits.iter().any(|h| h.victim as usize == v && h.attacker == Some(a as u8));
            if landed || !self.fighters[v].alive || x.t > x.mv.blow() + 40 {
                self.javelin_kill(x);
                self.finishers[i].killed = true;
            }
        }
    }

    /// Keys a finisher presses for its performer this frame (backing away
    /// from the victim before a spear throw, as far as there's room).
    fn finisher_walk(&self, fi: usize) -> u16 {
        let Some(x) = self.finishers.iter().find(|x| x.attacker as usize == fi) else { return 0 };
        if x.mv == Finish::Stomp {
            // (placed by a slide in update_finishers, not by running into them)
            return 0;
        }
        if let (Finish::Pin, Some(wall)) = (x.mv, x.wall) {
            let me = self.fighters[fi].pos(&self.world, PELVIS);
            let toward = if wall.x >= me.x { RIGHT } else { LEFT };
            let away = if toward == RIGHT { LEFT } else { RIGHT };
            let gap = (wall.x - me.x).abs();
            // close in to arm's length of them against the wall; after, a step back
            let near = PIN_GAP + 0.58 * self.fighters[fi].body.arm_k();
            return match x.t {
                // shove them back...
                24..PIN_LET_GO if gap > near + 0.04 => toward,
                // ...then step away to make room to flip the sword round
                t if (PIN_LET_GO..PIN_LET_GO + 16).contains(&t) && gap < near + 0.7 => away,
                t if t > PIN_HIT + 8 && t < PIN_HIT + 14 => away,
                _ => 0,
            };
        }
        if x.mv != Finish::Javelin || x.t < 76 || x.t >= 132 {
            return 0;
        }
        let (me, them) = (self.fighters[fi].pos(&self.world, PELVIS), self.fighters[x.victim as usize].pos(&self.world, PELVIS));
        let away = if them.x >= me.x { -1.0 } else { 1.0 };
        let room = self.world.raycast(me, Vec2::new(away, 0.0), 0.9).is_none();
        if (them.x - me.x).abs() < 4.4 && room {
            if away < 0.0 { LEFT } else { RIGHT }
        } else {
            0
        }
    }

    /// The grapple finisher's knot: the rope goes round their neck (they drop
    /// whatever they had), and they're left in a heap while you decide where
    /// to toss the hook.
    fn attach_noose(&mut self, x: Finisher) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        let Some(wi) = self.fighters[a].weapon.filter(|&wi| self.weapons[wi as usize].def == GRAPPLE && self.weapons[wi as usize].ammo > 0) else { return };
        // one rope loaded in the gun, and one per neck
        let loaded = |n: &Noose| n.weapon == wi && n.hook == Hook::Loaded;
        let old: Vec<Noose> = self.nooses.iter().copied().filter(|n| loaded(n) || n.victim == v as u8).collect();
        for n in old {
            self.world.distances[n.tether as usize].enabled = false;
            self.world.park_particle(n.anchor);
        }
        self.nooses.retain(|n| !loaded(n) && n.victim != v as u8);
        self.release(v, false);
        let tether = self.world.add_pin_slot(0.0);
        let mut hook = Particle::new(Vec2::ZERO, 0.0, 0.03);
        hook.collide_static = false;
        let anchor = self.world.add_particle(hook);
        self.nooses.push(Noose { weapon: wi, victim: v as u8, attacker: a as u8, tether, len: NOOSE_SLACK, cut: false, toss: false, hook: Hook::Loaded, anchor, hook_vel: Vec2::ZERO, hook_t: 0, hang: HANG_LEN });
        let fv = &mut self.fighters[v];
        fv.down = fv.down.max(110);
        fv.dazed = fv.dazed.max(DAZE_FRAMES);
        let neck = self.fighters[v].pos(&self.world, fighter::NECK);
        self.events.push(Sfx::Pickup.at(neck).vol(1.3));
        self.events.push(Sfx::Punch.at(neck).vol(0.6));
    }

    /// Where a noose's rope starts: the gun's muzzle while the hook is
    /// loaded, the hook itself once it's out.
    fn noose_top(&self, n: &Noose) -> u32 {
        let w = &self.weapons[n.weapon as usize];
        if n.hook == Hook::Loaded { w.particle(w.def().muzzle) } else { n.anchor }
    }

    /// Left click with the grapple gun: the hook flies along the aim,
    /// dragging whoever is tied to it (limp, and untouchable) along behind.
    fn fire_hook(&mut self, fi: usize, wi: usize) {
        if self.weapons[wi].fire_cd > 0 {
            return;
        }
        self.weapons[wi].fire_cd = 20;
        let w = &self.weapons[wi];
        let muzzle = self.world.particles[w.particle(w.def().muzzle) as usize].pos;
        let Some(k) = self.nooses.iter().position(|n| n.weapon as usize == wi && !n.cut && n.hook == Hook::Loaded) else {
            self.events.push(Sfx::Empty.at(muzzle));
            return;
        };
        let dir = self.fighters[fi].aim_dir();
        let head = self.fighters[self.nooses[k].victim as usize].pos(&self.world, fighter::HEAD);
        // its one hook: hit or miss, the gun's spent
        self.weapons[wi].ammo = 0;
        let n = &mut self.nooses[k];
        n.hook = Hook::Flying;
        n.hook_vel = dir * HOOK_SPEED;
        n.hook_t = 0;
        n.toss = true;
        n.len = (head - muzzle).length().max(DRAG_LEN);
        let p = &mut self.world.particles[n.anchor as usize];
        p.pos = muzzle;
        p.prev = muzzle;
        self.events.push(Sfx::Shot.at(muzzle).vol(0.5));
        self.events.push(Sfx::Throw.at(muzzle).vol(1.2));
    }

    /// The hook out of the gun: flying (until it hits something or gives
    /// up), or on its way back after a miss.
    /// A missed hook goes loose at `at`: a light thing that falls, rolls
    /// along the floor and is dragged on the rope (it's out of the gun for good).
    fn drop_hook(&mut self, n: &mut Noose, at: Vec2) {
        n.hook = Hook::Loose;
        n.toss = false;
        let p = &mut self.world.particles[n.anchor as usize];
        p.pos = at;
        p.prev = at - n.hook_vel * DT;
        p.vel = n.hook_vel;
        p.inv_mass = 1.0 / HOOK_MASS;
        p.collide_static = true;
        p.friction = 0.6;
    }

    fn move_hook(&mut self, k: usize) {
        let mut n = self.nooses[k];
        let at = self.world.particles[n.anchor as usize].pos;
        match n.hook {
            Hook::Flying => {
                n.hook_vel.y -= HOOK_GRAVITY * DT;
                let step = n.hook_vel * DT;
                let hit = self.world.raycast(at, step.normalize_or(Vec2::X), step.length());
                let to = hit.map_or(at + step, |h| h.point + h.normal * 0.02);
                n.hook_t += 1;
                if let Some(h) = hit {
                    n.toss = false;
                    // it bites if it could hang them from there; otherwise it glances off
                    let head = self.fighters[n.victim as usize].pos(&self.world, fighter::HEAD);
                    if to.y > head.y + 1.0 {
                        n.hook = Hook::Caught;
                        // reels in only until their feet are just off the floor below
                        let fv = &self.fighters[n.victim as usize];
                        let body = fv.body.neck + fv.body.torso + fv.body.leg;
                        let floor = self.world.raycast(to, Vec2::NEG_Y, 40.0).map(|f| f.point.y);
                        n.hang = floor.map_or(HANG_OVER_PIT, |y| to.y - y - body - HANG_CLEAR).max(HANG_LEN);
                        // the rope snaps taut: they stop dead and hang, head up
                        for id in self.fighters[n.victim as usize].particle_ids() {
                            self.world.particles[id as usize].vel *= 0.2;
                        }
                        self.spawn_sparks(to, h.normal, 6, 3.0);
                        self.events.push(Sfx::Embed.at(to));
                    } else {
                        // glances off: what bounced off goes loose
                        let v = n.hook_vel - h.normal * (1.4 * n.hook_vel.dot(h.normal));
                        n.hook_vel = v * 0.35;
                        self.drop_hook(&mut n, to);
                        self.events.push(Sfx::Ricochet.at(to).vol(0.6));
                        self.nooses[k] = n;
                        return;
                    }
                } else if n.hook_t >= HOOK_RANGE {
                    // spent: it falls where it is, still flying on
                    self.drop_hook(&mut n, to);
                    self.nooses[k] = n;
                    return;
                }
                let p = &mut self.world.particles[n.anchor as usize];
                p.prev = at;
                p.pos = to;
            }
            _ => {}
        }
        self.nooses[k] = n;
    }

    /// Ropes before the world moves: paid out loose while the hook is free,
    /// reeled in once it has caught, and pulling only when taut. Anyone hung
    /// clear of the ground chokes.
    fn update_nooses(&mut self) {
        for f in &mut self.fighters {
            f.choking = false;
        }
        let mut i = 0;
        while i < self.nooses.len() {
            let n = self.nooses[i];
            let w = &self.weapons[n.weapon as usize];
            // (a gun lost with the rope still on it takes the rope with it)
            if (!w.active || w.def != GRAPPLE) && n.hook == Hook::Loaded {
                self.world.distances[n.tether as usize].enabled = false;
                self.world.park_particle(n.anchor);
                self.nooses.remove(i);
                continue;
            }
            if n.cut {
                i += 1;
                continue;
            }
            self.move_hook(i);
            let mut n = self.nooses[i];
            let v = n.victim as usize;
            // (the rope pulls on the head, so the body always hangs below it)
            let (top, head) = (self.noose_top(&n), self.fighters[v].p(fighter::HEAD));
            let d = (self.world.particles[top as usize].pos - self.world.particles[head as usize].pos).length();
            n.len = match n.hook {
                // (reeled in, never let out: stops at `hang`, or where it is if that's shorter)
                Hook::Caught => {
                    let now = n.len.min(d + 0.05);
                    (now - REEL_SPEED * DT).max(n.hang.min(now))
                }
                // dragged along behind the hook on a short rope; once it's
                // loose, they drag it about on what's paid out
                Hook::Flying | Hook::Loose => n.len,
                Hook::Loaded => NOOSE_SLACK,
            };
            let taut = d >= n.len - 0.01;
            self.world.distances[n.tether as usize] = Distance { a: top, b: head, rest: n.len, compliance: 0.0, enabled: taut };
            self.nooses[i] = n;
            if n.toss && self.fighters[v].alive {
                // flung about on the end of the rope: limp until the hook lands
                let fv = &mut self.fighters[v];
                fv.down = fv.down.max(12);
            }
            let fv = &self.fighters[v];
            let touching = fv.particle_ids().any(|id| self.world.particles[id as usize].on_ground());
            if n.hook == Hook::Caught && fv.alive && d >= n.len - 0.06 && !touching {
                let fv = &mut self.fighters[v];
                fv.choking = true;
                fv.health -= CHOKE_DRAIN;
                if fv.health <= 0.0 {
                    self.choke_kill(n);
                }
            }
            i += 1;
        }
    }

    fn choke_kill(&mut self, n: Noose) {
        let v = n.victim as usize;
        let at = self.fighters[v].pos(&self.world, fighter::NECK);
        self.kill_fighter(v);
        self.fighters[v].choking = false;
        self.hits.push(HitEvent { attacker: Some(n.attacker), victim: n.victim, point: at, damage: 0.0, severed: false, lethal: true });
        self.finish_banner = Some((Finish::Hang, n.attacker, FINISH_BANNER));
        self.shake_camera(n.attacker as usize, 0.8);
        self.shake_camera(v, 0.8);
        self.events.push(Sfx::Thud.at(at).vol(0.8));
        self.events.push(Sfx::ChokeEnd.at(at).line(0, n.victim));
    }

    /// The rope as drawn: straight when taut, sagging when slack.
    pub fn noose_path(&self, n: &Noose) -> [Vec2; 9] {
        let a = self.world.particles[self.noose_top(n) as usize].pos;
        let b = self.fighters[n.victim as usize].pos(&self.world, fighter::HEAD);
        let d = (b - a).length();
        // (shown with a little slack, not all 7 m of it, lying on the floor
        // where it reaches)
        let shown = n.len.min(d + 1.2);
        let sag = if d < shown { ((shown * shown - d * d).max(0.0)).sqrt() * 0.5 } else { 0.0 };
        core::array::from_fn(|k| {
            let t = k as f32 / 8.0;
            let line = a + (b - a) * t;
            let drop = sag * 4.0 * t * (1.0 - t);
            let floor = self.world.raycast(line, Vec2::NEG_Y, drop + 0.03).map_or(drop, |h| (h.t - 0.03).max(0.0));
            line - Vec2::Y * drop.min(floor)
        })
    }

    /// A blade (held or flying) or a bullet crossing a rope fast enough cuts
    /// it: whoever was hanging drops, alive.
    fn cut_nooses(&mut self) {
        for k in 0..self.nooses.len() {
            let n = self.nooses[k];
            if n.cut {
                continue;
            }
            let path = self.noose_path(&n);
            let crosses = |a: Vec2, b: Vec2, r: f32| path.windows(2).find(|s| seg_dist(a, b, s[0], s[1]) < r + ROPE_R).map(|s| (s[0] + s[1]) * 0.5);
            let mut cut_at = None;
            for (wi, w) in self.weapons.iter().enumerate() {
                if wi == n.weapon as usize || !w.active || (w.holder.is_none() && w.thrown == 0) {
                    continue;
                }
                for (si, sg) in w.def().segs.iter().enumerate() {
                    if sg.kind != Kind::Blade {
                        continue;
                    }
                    let seg = self.world.segments[(w.seg_base + si as u32) as usize];
                    let (pa, pb) = (self.world.particles[seg.a as usize], self.world.particles[seg.b as usize]);
                    if pa.vel.length().max(pb.vel.length()) < CUT_SPEED {
                        continue;
                    }
                    if let Some(at) = crosses(pa.pos, pb.pos, sg.r) {
                        cut_at = Some(at);
                    }
                }
            }
            for b in &self.bullets {
                if let Some(at) = crosses(b.pos - b.vel * DT, b.pos, 0.0) {
                    cut_at = Some(at);
                }
            }
            let Some(at) = cut_at else { continue };
            self.nooses[k].cut = true;
            self.world.distances[n.tether as usize].enabled = false;
            let v = n.victim as usize;
            if self.fighters[v].alive {
                let fv = &mut self.fighters[v];
                fv.choking = false;
                fv.knock_down(45);
            }
            self.spawn_sparks(at, Vec2::Y, 5, 2.0);
            self.events.push(Sfx::Sever.at(at).vol(0.7));
        }
    }

    /// Lets go and leaves them standing, swaying, for the blow.
    fn stand_up(&mut self, x: Finisher) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        self.release_grab(a);
        let fv = &mut self.fighters[v];
        fv.down = 0;
        fv.getup = 0;
        fv.stagger = 0;
        fv.dazed = fv.dazed.max(DAZE_FRAMES);
    }

    /// Runs `v` through on weapon `wi`'s blade (no speed or contact needed).
    fn force_skewer(&mut self, wi: usize, v: usize, friction: f32) {
        let wp = &self.weapons[wi];
        let Some((_, base, tip)) = wp.def().impale else { return };
        let seg = self.world.segments[(self.fighters[v].seg_base + S_TORSO) as usize];
        let (slot, b_id, t_id) = (wp.skewer, wp.particle(base), wp.particle(tip));
        self.world.skewers[slot as usize] = stickfell_physics::Skewer { a: seg.a, b: seg.b, s: 0.5, base: b_id, tip: t_id, u_min: 0.08, u_max: f32::MAX, friction, u: 0.5, enabled: true, anchored: false };
        let wp = &mut self.weapons[wi];
        wp.victim = Some(v as u8);
        wp.skewer_t = GLIDE_FRAMES;
        let wp = wp.clone();
        wp.set_pass_through(&mut self.world, v as u16 + 1);
    }

    /// The spear leaves the hand on a line for their chest (aimed a little high
    /// to allow for the drop), fast and point-first.
    fn javelin_throw(&mut self, x: Finisher) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        let Some(wi) = self.fighters[a].weapon.map(|w| w as usize) else { return };
        let wp = &self.weapons[wi];
        let grip = self.world.particles[wp.particle(wp.def().grip) as usize].pos;
        let chest = (self.fighters[v].pos(&self.world, fighter::NECK) + self.fighters[v].pos(&self.world, PELVIS)) * 0.5;
        const SPEED: f32 = 18.0;
        let flight = (chest - grip).length() / SPEED;
        let aim = chest + Vec2::new(0.0, 0.5 * 28.0 * flight * flight) - grip;
        let dir = aim.normalize_or(Vec2::X);
        wp.place(&mut self.world, grip, dmath::angle(dir), Vec2::ZERO);
        self.release(a, false);
        let wp = &mut self.weapons[wi];
        wp.thrown = 60;
        wp.last_holder = Some(a as u8);
        // the shaft starts out through the thrower's own body: keep it from
        // catching on them until it's clear (slow motion makes that take a while)
        wp.grace = 60;
        let wp = wp.clone();
        wp.set_pass_through(&mut self.world, 0); // it has to actually hit them now
        for k in 0..wp.n {
            self.world.particles[wp.particle(k) as usize].vel = dir * SPEED;
        }
        self.events.push(Sfx::Throw.at(grip).vol(1.4));
    }

    /// The thrown spear reaches them: run through and carried back off their feet.
    fn javelin_kill(&mut self, x: Finisher) {
        let v = x.victim as usize;
        let chest = (self.fighters[v].pos(&self.world, fighter::NECK) + self.fighters[v].pos(&self.world, PELVIS)) * 0.5;
        let mut dir = Vec2::X;
        if let Some(wi) = x.weapon.map(|w| w as usize) {
            let vel = self.world.particles[self.weapons[wi].base as usize].vel;
            dir = vel.normalize_or(Vec2::X);
            let wp = &self.weapons[wi];
            let tip = wp.def().impale.map_or(0, |i| i.2);
            let close = (self.world.particles[wp.particle(tip) as usize].pos - chest).length() < 0.9;
            // pinned on it only if it's actually there (never yanked across the room)
            if close && wp.victim.is_none() {
                self.force_skewer(wi, v, 0.6);
            }
        }
        self.finish_banner = Some((x.mv, x.attacker, FINISH_BANNER));
        self.shake_camera(x.attacker as usize, 1.5);
        self.shake_camera(x.victim as usize, 1.5);
        self.flash = 10;
        self.events.push(Sfx::Impale.at(chest).vol(1.8));
        self.spawn_blood(chest, dir, 30, 9.0);
        self.damage(v, S_TORSO, 999.0, chest, dir, 0.0, Kind::Spike, Some(x.attacker));
        let ids = self.fighters[v].particle_ids();
        self.world.add_velocity(ids, dir * 2.5 + Vec2::new(0.0, 1.5));
    }

    /// Spear lift, part one: jammed through the chest (the kill).
    fn lift_jam(&mut self, x: Finisher) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        let Some(wi) = self.fighters[a].weapon.map(|w| w as usize) else { return };
        let chest = (self.fighters[v].pos(&self.world, fighter::NECK) + self.fighters[v].pos(&self.world, PELVIS)) * 0.5;
        self.force_skewer(wi, v, 0.95);
        self.wound(v, S_TORSO, chest, fighter::WoundKind::Stab, 0.05, Vec2::Y);
        self.finish_banner = Some((x.mv, x.attacker, FINISH_BANNER));
        self.shake_camera(x.attacker as usize, 1.5);
        self.shake_camera(x.victim as usize, 1.5);
        self.flash = 10;
        self.events.push(Sfx::Impale.at(chest).vol(1.8));
        self.spawn_blood(chest, Vec2::Y, 28, 7.0);
        let dir = (chest - self.fighters[a].pos(&self.world, fighter::NECK)).normalize_or(Vec2::X);
        self.damage(v, S_TORSO, 999.0, chest, dir, 0.0, Kind::Spike, Some(x.attacker));
    }

    /// Spear lift, part two: whipped down off the spear and slammed into the ground.
    /// The pin: the sword goes through their chest and on into the wall,
    /// and both hands let go (the sword and them stay where they are).
    fn pin_to_wall(&mut self, x: Finisher) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        let (Some(wi), Some(wall)) = (self.fighters[a].weapon.map(|w| w as usize), x.wall) else { return };
        let w = &self.world;
        let chest = (self.fighters[v].pos(w, fighter::NECK) + self.fighters[v].pos(w, PELVIS)) * 0.5;
        let side = if wall.x >= self.fighters[a].pos(w, PELVIS).x { 1.0 } else { -1.0 };
        // driven in level and a little downward, from the attacker's hand
        let axis = Vec2::new(side, -0.12).normalize();
        let to_wall = ((wall.x - chest.x) * side).max(0.05) / axis.x.abs();
        let reach = self.weapons[wi].tip_reach(&self.world);
        // the grip ends up a hand's width from their chest, or (if they're
        // not quite against it) far enough back that the tip still bites
        let grip_from_chest = PIN_GRIP.max(reach - to_wall - PIN_DEPTH).min(reach - 0.2);
        let grip = chest - axis * grip_from_chest;
        self.release(a, false);
        self.release_grab(a);
        let mut wp = self.weapons[wi].clone();
        wp.place(&mut self.world, grip, dmath::angle(axis), Vec2::ZERO);
        // sunk into the wall past the tip (drawn behind the wall, so it's buried)
        wp.embed(&mut self.world, axis, 0.0);
        self.weapons[wi] = wp;
        self.force_skewer(wi, v, 1.0);
        self.weapons[wi].pinned = true;
        self.world.skewers[self.weapons[wi].skewer as usize].anchored = true;
        self.finish_banner = Some((x.mv, x.attacker, FINISH_BANNER));
        self.shake_camera(x.attacker as usize, 1.5);
        self.shake_camera(x.victim as usize, 1.5);
        self.flash = 10;
        self.events.push(Sfx::Impale.at(chest).vol(1.8));
        self.events.push(Sfx::Embed.at(chest + axis * to_wall).vol(1.4));
        self.spawn_blood(chest, -axis, 18, 5.0);
        self.spawn_blood(chest + axis * to_wall, Vec2::new(0.0, -1.0), 14, 2.0);
        self.damage(v, S_TORSO, 999.0, chest, axis, 0.0, Kind::Spike, Some(x.attacker));
        self.wound(v, S_TORSO, chest, fighter::WoundKind::Stab, 0.05, Vec2::Y);
    }

    fn lift_drop(&mut self, x: Finisher) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        if let Some(wi) = self.fighters[a].weapon.map(|w| w as usize) {
            if self.weapons[wi].victim == Some(v as u8) {
                self.unskewer(wi);
            }
        }
        let ids = self.fighters[v].particle_ids();
        self.world.add_velocity(ids, Vec2::new(0.0, -14.0));
        let at = self.fighters[v].pos(&self.world, PELVIS);
        self.events.push(Sfx::HitHeavy.at(at).vol(1.6));
        self.events.push(Sfx::Thud.at(at).vol(1.4));
        self.shake_camera(v, 1.5);
    }

    /// The next limb a bare-handed finisher rips off (its elbow/knee), after
    /// `done` already have been; skips limbs that are already gone.
    fn next_limb(v: &Fighter, done: usize) -> Option<u32> {
        [(fighter::S_UPPER_A, fighter::ELBOW_A), (fighter::S_UPPER_B, fighter::ELBOW_B), (fighter::S_THIGH_A, fighter::KNEE_A), (fighter::S_THIGH_B, fighter::KNEE_B)]
            .into_iter()
            .filter(|(part, _)| v.has(*part))
            .nth(0)
            .filter(|_| done < 4)
            .map(|(_, slot)| slot)
    }

    /// Tears a limb off at the shoulder/hip and throws it.
    fn rip_limb(&mut self, v: usize, from: Vec2) {
        let parts = [fighter::S_UPPER_A, fighter::S_UPPER_B, fighter::S_THIGH_A, fighter::S_THIGH_B];
        let Some(&part) = parts.iter().find(|&&p| self.fighters[v].has(p)) else { return };
        if !self.sever(v, part) {
            return;
        }
        let f = &self.fighters[v];
        let Some((_, ps)) = fighter::sever_plan(part) else { return };
        let mut ids = Vec::new();
        for &p in ps {
            let sg = self.world.segments[(f.seg_base + p) as usize];
            ids.extend([sg.a, sg.b]);
        }
        let at = self.world.particles[ids[0] as usize].pos;
        let away = (at - from).normalize_or(Vec2::Y);
        for id in ids {
            self.world.particles[id as usize].vel += away * 7.0 + Vec2::new(0.0, 5.0);
        }
        self.spawn_blood(at, away, 22, 7.0);
        self.events.push(Sfx::Sever.at(at).vol(1.5));
    }

    /// Tears the head out with the spine still on it: the arms come away, the
    /// torso parts from the hips, and head + spine stay in the attacker's hand.
    fn rip_spine(&mut self, v: usize) {
        for part in [fighter::S_UPPER_A, fighter::S_UPPER_B] {
            if self.sever(v, part) {
                let f = &self.fighters[v];
                let sg = self.world.segments[(f.seg_base + part) as usize];
                self.world.particles[sg.b as usize].vel += Vec2::new(0.0, -2.0);
            }
        }
        let f = self.fighters[v].clone();
        let pel = f.p(PELVIS);
        let jp = self.world.particles[pel as usize];
        let mut np = Particle::new(jp.pos, 0.6, 0.07);
        np.vel = jp.vel;
        np.prev = jp.prev;
        let tail = self.world.add_particle(np);
        // the torso bone (neck -> pelvis) now ends at its own loose tip
        self.world.segments[(f.seg_base + S_TORSO) as usize].b = tail;
        self.world.distances[(f.dist_base + S_TORSO - 1) as usize].b = tail;
        let fm = &mut self.fighters[v];
        fm.spine_out = true;
        fm.bleed = 300;
        fm.bleed_at = pel;
        self.spawn_blood(jp.pos, Vec2::Y, 34, 8.0);
    }

    /// One strike of a finisher; the last one kills, in the weapon's own way.
    /// Hurt and still alive mid-finisher: a cry of pain cuts off whatever
    /// they were saying, and they wince for a moment.
    fn wince(&mut self, fi: usize) {
        let f = &mut self.fighters[fi];
        if !f.alive {
            return;
        }
        f.hurt = WINCE_FRAMES;
        let head = f.pos(&self.world, fighter::HEAD);
        self.events.push(Sfx::Pain.at(head).line(0, fi as u8));
    }

    /// Where a curb stomp has the stomper's heel this frame, and how fast it
    /// may get there (m/s): lifted straight up beside them, carried over the
    /// head, driven straight down onto it; up over it again, higher, and down
    /// again. None while they're stepping in, and after.
    fn stomp_foot(&self, x: &Finisher) -> Option<(Vec2, f32)> {
        let (a, v) = (&self.fighters[x.attacker as usize], &self.fighters[x.victim as usize]);
        let w = &self.world;
        let f = -(x.side as f32);
        let hip = a.pos(w, PELVIS);
        let head = v.pos(w, fighter::HEAD);
        // (on top of the head: its radius and the heel's, sunk in a little)
        let top = w.segments[(v.seg_base + S_HEAD) as usize].radius + w.particles[a.p(fighter::FOOT_A) as usize].radius;
        let hover = |h: f32| head + Vec2::new(0.0, top + h);
        let on = head + Vec2::new(0.0, top - 0.03);
        let [first, second] = weapon::STOMP_HITS;
        Some(match x.t {
            t if t < 26 => return None,
            t if t < 34 => (Vec2::new(hip.x + f * 0.08, hip.y - 0.4), 3.0),
            t if t < first - 7 => (hover(0.3), 3.0),
            t if t < first + 8 => (on, 7.0),
            t if t < first + 16 => (hover(0.2), 2.5),
            t if t < second - 7 => (hover(0.4), 2.5),
            t if t < second + 14 => (on, 8.0),
            _ => return None,
        })
    }

    /// Which way their head lies from their body (+1: to the right).
    fn head_side(v: &Fighter, w: &World) -> f32 {
        if v.pos(w, fighter::HEAD).x >= v.pos(w, PELVIS).x { 1.0 } else { -1.0 }
    }

    /// A curb stomp landing: the first flattens the head, the last bursts it.
    /// Either way the stomper's leg comes away bloody.
    fn stomp(&mut self, x: Finisher, last: bool) {
        let (a, v) = (x.attacker as usize, x.victim as usize);
        let head = self.fighters[v].pos(&self.world, fighter::HEAD);
        let foot = self.fighters[a].pos(&self.world, fighter::FOOT_A);
        let att = Some(x.attacker);
        // the stomper's foot and shin take some of it
        let (knee, hip) = (self.fighters[a].pos(&self.world, fighter::KNEE_A), self.fighters[a].pos(&self.world, PELVIS));
        let spots: &[(u32, Vec2)] = if last {
            &[(fighter::S_SHIN_A, foot), (fighter::S_SHIN_A, foot.lerp(knee, 0.5)), (fighter::S_SHIN_A, knee), (fighter::S_THIGH_A, knee.lerp(hip, 0.4)), (S_TORSO, hip + Vec2::new(0.0, 0.18))]
        } else {
            &[(fighter::S_SHIN_A, foot), (fighter::S_SHIN_A, foot.lerp(knee, 0.4))]
        };
        for &(part, at) in spots {
            let jitter = Vec2::new(self.rng.range(-0.04, 0.04), self.rng.range(-0.04, 0.04));
            let size = self.rng.range(0.03, 0.045);
            self.wound(a, part, at + jitter, fighter::WoundKind::Spatter, size, Vec2::X);
        }
        self.events.push(Sfx::Thud.at(head).vol(1.6));
        if !last {
            self.fighters[v].head_squash = self.fighters[v].head_squash.max(STOMP_SQUASH);
            self.events.push(Sfx::HitHeavy.at(head).vol(1.3));
            self.spawn_blood(head, Vec2::new(0.0, 1.0), 10, 3.5);
            let alive = self.fighters[v].health;
            self.damage(v, S_HEAD, 4.0, head, Vec2::NEG_Y, 0.0, Kind::Blunt, att);
            // the first one never kills
            self.fighters[v].health = self.fighters[v].health.max(alive.min(1.0));
            self.wince(v);
            self.shake_camera(x.attacker as usize, 0.7);
            self.shake_camera(v, 0.7);
            return;
        }
        self.finish_banner = Some((x.mv, x.attacker, FINISH_BANNER));
        self.shake_camera(x.attacker as usize, 1.5);
        self.shake_camera(x.victim as usize, 1.5);
        self.flash = 10;
        // flattened, then gone: nothing flies off, it bursts where it lies
        let fv = self.fighters[v].clone();
        self.sever(v, S_HEAD);
        let hid = fv.p(fighter::HEAD) as usize;
        let hp = &mut self.world.particles[hid];
        hp.vel = Vec2::ZERO;
        hp.prev = hp.pos;
        hp.radius = 0.01;
        self.world.segments[(fv.seg_base + S_HEAD) as usize].radius = 0.005;
        {
            let fm = &mut self.fighters[v];
            fm.head_squash = 1.0;
            fm.head_popped = true;
        }
        for k in 0..9 {
            let ang = core::f32::consts::PI * (k as f32 + 0.5) / 9.0;
            let (s, c) = dmath::sin_cos(ang);
            let speed = self.rng.range(5.0, 9.0);
            self.spawn_blood(head, Vec2::new(c, s * 0.6 + 0.2), 7, speed);
        }
        self.spawn_blood(head, Vec2::Y, 18, 3.0);
        self.events.push(Sfx::Sever.at(head).vol(1.6));
        self.events.push(Sfx::HitHeavy.at(head).vol(1.8));
        let neck = self.fighters[v].pos(&self.world, fighter::NECK);
        self.damage(v, S_TORSO, 999.0, neck, Vec2::NEG_Y, 0.0, Kind::Blunt, att);
    }

    fn finisher_hit(&mut self, x: Finisher, k: usize) {
        let hits = x.mv.hits();
        let last = k + 1 == hits.len();
        if x.mv.stand_at() == Some(hits[k]) {
            self.stand_up(x);
            return;
        }
        match (x.mv, k) {
            (Finish::Stomp, _) => return self.stomp(x, last),
            (Finish::Javelin, 1) => return self.javelin_throw(x),
            (Finish::Lift, 1) => return self.lift_jam(x),
            (Finish::Lift, 2) => return self.lift_drop(x),
            (Finish::Hang, _) => return self.attach_noose(x),
            (Finish::Pin, _) => return self.pin_to_wall(x),
            _ => {}
        }
        let (a, v) = (x.attacker as usize, x.victim as usize);
        let w = &self.world;
        let head = self.fighters[v].pos(w, fighter::HEAD);
        let torso = (self.fighters[v].pos(w, fighter::NECK) + self.fighters[v].pos(w, PELVIS)) * 0.5;
        let f = if torso.x >= self.fighters[a].pos(w, PELVIS).x { 1.0 } else { -1.0 };
        let att = Some(x.attacker);
        if !last && x.mv == Finish::Dismember {
            // one more limb gone (they're still alive for now)
            let from = self.fighters[a].pos(w, fighter::NECK);
            self.rip_limb(v, from);
            self.shake_camera(a, 0.6);
            self.shake_camera(v, 0.6);
            self.wince(v);
            return;
        }
        if !last {
            // the build-up: slaps and stabs that hurt but don't finish
            let (sound, part, at) = match x.mv {
                Finish::Slap => (Sfx::HitBlunt, S_HEAD, head),
                // (each stab somewhere new on the torso)
                _ => (Sfx::HitBlade, S_TORSO, torso + Vec2::new(0.0, self.rng.range(-0.18, 0.18))),
            };
            self.events.push(sound.at(at).vol(1.2));
            if part == S_TORSO {
                self.wound(v, S_TORSO, at - Vec2::new(f * 0.08, 0.0), fighter::WoundKind::Stab, 0.04, Vec2::Y);
            }
            self.spawn_blood(at, Vec2::new(f, 0.3), 8, 4.0);
            let alive = self.fighters[v].health;
            self.damage(v, part, 3.0, at, Vec2::new(f, 0.0), 6.0, Kind::Blunt, att);
            // never let the build-up do the killing
            self.fighters[v].health = self.fighters[v].health.max(alive.min(1.0));
            self.wince(v);
            return;
        }
        self.finish_banner = Some((x.mv, x.attacker, FINISH_BANNER));
        self.shake_camera(x.attacker as usize, 1.5);
        self.shake_camera(x.victim as usize, 1.5);
        self.flash = 10;
        let kill = |s: &mut Self, part: u32, at: Vec2, dir: Vec2, knock: f32, kind: Kind| {
            s.damage(v, part, 999.0, at, dir, knock, kind, att);
        };
        match x.mv {
            Finish::Behead | Finish::Smash => {
                self.sever(v, S_HEAD);
                let hid = self.fighters[v].p(fighter::HEAD) as usize;
                let pop = if x.mv == Finish::Smash { Vec2::new(f * 3.0, 9.0) } else { Vec2::new(f * 6.0, 5.0) };
                self.world.particles[hid].vel += pop;
                self.spawn_blood(head, Vec2::Y, 30, 8.0);
                self.events.push(if x.mv == Finish::Smash { Sfx::HitHeavy } else { Sfx::Sever }.at(head).vol(1.6));
                let dir = if x.mv == Finish::Smash { Vec2::NEG_Y } else { Vec2::new(f, 0.0) };
                kill(self, S_TORSO, torso, dir, 30.0, Kind::Blunt);
            }
            Finish::Javelin | Finish::Lift | Finish::Hang | Finish::Pin | Finish::Stomp => {}
            Finish::Spin => {
                self.events.push(Sfx::HitHeavy.at(torso).vol(1.8));
                self.release_grab(a);
                let dir = Vec2::new(f, 0.75).normalize();
                kill(self, S_TORSO, torso, dir, 40.0, Kind::Blunt);
                // standing square in its path: the whole body goes flying
                let ids = self.fighters[v].particle_ids();
                self.world.add_velocity(ids, dir * 9.0);
                self.spawn_blood(torso, dir, 24, 8.0);
            }
            Finish::Slap => {
                self.release_grab(a);
                self.events.push(Sfx::HitHeavy.at(head).vol(1.4));
                kill(self, S_HEAD, head, Vec2::new(f, 0.5).normalize(), 40.0, Kind::Blunt);
            }
            Finish::Bonk => {
                self.release_grab(a);
                self.events.push(Sfx::Clash.at(head).vol(1.8));
                kill(self, S_HEAD, head, Vec2::new(f * 0.3, 1.0).normalize(), 40.0, Kind::Blunt);
                let hid = self.fighters[v].p(fighter::HEAD) as usize;
                self.world.particles[hid].vel += Vec2::new(-f * 8.0, 0.0); // spin
            }
            Finish::Execute => {
                let loaded = self.fighters[a].weapon.is_some_and(|wi| self.weapons[wi as usize].ammo > 0);
                if let (true, Some(wi)) = (loaded, self.fighters[a].weapon) {
                    let wp = &mut self.weapons[wi as usize];
                    wp.ammo -= 1;
                    wp.flash = 4;
                    let d = wp.def();
                    let (m, b) = (self.world.particles[wp.particle(d.muzzle) as usize].pos, self.world.particles[wp.particle(d.aim.0) as usize].pos);
                    self.kick_gun(a, wi as usize, (m - b).normalize_or(Vec2::X));
                    self.events.push(Self::shot_sound(d).at(head).vol(1.3));
                    // through the head along the barrel: a hole where the
                    // muzzle was pressed, a ragged one out the far side
                    let through = (head - m).normalize_or(Vec2::new(f, 0.0));
                    let r = self.world.segments[(self.fighters[v].seg_base + S_HEAD) as usize].radius;
                    self.wound(v, S_HEAD, head - through * r * 0.8, fighter::WoundKind::Hole, 0.032, through.perp());
                    let out = head + through * r * 0.8;
                    self.wound(v, S_HEAD, out, fighter::WoundKind::Exit, 0.065, through.perp());
                    self.spawn_blood(out, through, 32, 10.0);
                } else {
                    self.events.push(Sfx::HitHeavy.at(head));
                }
                self.release_grab(a);
                kill(self, S_HEAD, head, Vec2::new(f, 0.1).normalize(), 22.0, Kind::Spike);
            }
            Finish::Shank => {
                self.release_grab(a);
                self.wound(v, S_TORSO, torso - Vec2::new(f * 0.08, 0.0), fighter::WoundKind::Stab, 0.045, Vec2::Y);
                self.events.push(Sfx::Impale.at(torso).vol(1.4));
                kill(self, S_TORSO, torso, Vec2::new(f, 0.0), 14.0, Kind::Spike);
            }
            Finish::Dismember => {
                let from = self.fighters[a].pos(&self.world, fighter::NECK);
                self.rip_limb(v, from);
                self.events.push(Sfx::HitHeavy.at(torso).vol(1.4));
                self.release_grab(a);
                kill(self, S_TORSO, torso, Vec2::new(f, -0.3).normalize(), 10.0, Kind::Blunt);
            }
            Finish::Spine => {
                self.rip_spine(v);
                // the trophy is carried, not hung off the grip (which would
                // haul the attacker up into the air after it)
                let grab = self.fighters[a].grab;
                self.world.distances[grab as usize].enabled = false;
                self.events.push(Sfx::Sever.at(torso).vol(1.8));
                self.events.push(Sfx::HitHeavy.at(torso).vol(1.2));
                // kept hold of: the head and spine go up as a trophy
                kill(self, S_HEAD, head, Vec2::Y, 0.0, Kind::Blunt);
            }
        }
    }

    /// Two blades meeting hard enough, crossed, with at least one of them
    /// mid-strike, lock together into a duel.
    fn try_clash(&mut self, w1: usize, s1: usize, w2: usize, s2: usize, point: Vec2, impact: f32) {
        if self.clash.is_some() || self.clash_cd > 0 || impact < CLASH_IMPACT || !matches!(self.phase, Phase::Fight) {
            return;
        }
        let (a, b) = (&self.weapons[w1], &self.weapons[w2]);
        let (Some(ha), Some(hb)) = (a.holder, b.holder) else { return };
        if ha == hb || a.def().segs[s1].kind != Kind::Blade || b.def().segs[s2].kind != Kind::Blade || a.victim.is_some() || b.victim.is_some() {
            return;
        }
        let (fa, fb) = (&self.fighters[ha as usize], &self.fighters[hb as usize]);
        let ready = |f: &Fighter| f.alive && f.down == 0 && !f.skewering;
        if !ready(fa) || !ready(fb) || !(fa.striking() || fb.striking()) {
            return;
        }
        // the blades have to actually cross, not slide along each other
        let axis = |w: &Weapon| {
            let (p, q) = w.def().aim;
            (self.world.particles[w.particle(q) as usize].pos - self.world.particles[w.particle(p) as usize].pos).normalize_or(Vec2::X)
        };
        if axis(a).perp_dot(axis(b)).abs() < 0.45 {
            return;
        }
        let prompt = (self.rng.next_u32() % 4) as u8;
        self.clash = Some(Clash {
            fighters: [ha, hb],
            point,
            score: [0, 0],
            prompt,
            stage: ClashStage::Intro,
            t: 0,
            total: 0,
            lockout: [0, 0],
            last: None,
        });
        // the blades rest against each other without shoving
        let (a, b) = (self.weapons[w1].clone(), self.weapons[w2].clone());
        a.set_pass_through(&mut self.world, hb as u16 + 1);
        b.set_pass_through(&mut self.world, ha as u16 + 1);
        self.spawn_sparks(point, Vec2::Y, 24, 9.0);
        self.events.push(Sfx::Clash.at(point).vol(1.8));
    }

    /// Runs the duel: prompts, points, and the finish.
    fn update_clash(&mut self) {
        self.clash_cd = self.clash_cd.saturating_sub(1);
        if let Some((w, t)) = self.clash_banner {
            self.clash_banner = (t > 1).then_some((w, t - 1));
        }
        let Some(mut c) = self.clash else { return };
        let [a, b] = c.fighters.map(|f| f as usize);
        let fine = |f: &Fighter| f.alive && f.down == 0 && f.weapon.is_some();
        if !fine(&self.fighters[a]) || !fine(&self.fighters[b]) || c.total > 720 {
            self.end_clash();
            return;
        }
        c.t += 1;
        c.total += 1;
        for l in &mut c.lockout {
            *l = l.saturating_sub(1);
        }
        let new_prompt = |rng: &mut Rng, old: u8| (old + 1 + (rng.next_u32() % 3) as u8) % 4;
        match c.stage {
            ClashStage::Intro => {
                if c.t >= CLASH_INTRO {
                    c.stage = ClashStage::Prompt;
                    c.t = 0;
                }
            }
            ClashStage::Prompt => {
                let want = CLASH_KEYS[c.prompt as usize];
                let mut right = [false; 2];
                for (side, fi) in [a, b].into_iter().enumerate() {
                    if c.lockout[side] > 0 {
                        continue;
                    }
                    let f = &self.fighters[fi];
                    let hit_any = CLASH_KEYS.iter().any(|&k| f.pressed_raw(k));
                    if !hit_any {
                        continue;
                    }
                    let wrong = CLASH_KEYS.iter().any(|&k| k != want && f.pressed_raw(k));
                    if f.pressed_raw(want) && !wrong {
                        right[side] = true;
                    } else {
                        c.lockout[side] = CLASH_LOCKOUT;
                        let at = f.pos(&self.world, fighter::NECK);
                        self.events.push(Sfx::Empty.at(at));
                    }
                }
                match right {
                    [true, false] | [false, true] => {
                        let side = if right[0] { 0 } else { 1 };
                        c.score[side] += 1;
                        c.last = Some(side as u8);
                        c.stage = ClashStage::Point;
                        c.t = 0;
                        self.spawn_sparks(c.point, Vec2::Y, 14, 7.0);
                        self.events.push(Sfx::Clash.at(c.point).vol(1.2));
                    }
                    // a dead heat, or nobody got it in time: next key
                    [true, true] => {
                        c.prompt = new_prompt(&mut self.rng, c.prompt);
                        c.t = 0;
                    }
                    [false, false] if c.t > 150 => {
                        c.prompt = new_prompt(&mut self.rng, c.prompt);
                        c.t = 0;
                    }
                    _ => {}
                }
            }
            ClashStage::Point => {
                if c.t >= CLASH_POINT_PAUSE {
                    if c.score.iter().any(|&s| s >= CLASH_WIN) {
                        let win = if c.score[0] >= CLASH_WIN { 0 } else { 1 };
                        self.clash = Some(c);
                        self.finish_clash(win);
                        return;
                    }
                    c.stage = ClashStage::Prompt;
                    c.prompt = new_prompt(&mut self.rng, c.prompt);
                    c.t = 0;
                }
            }
        }
        // the crossed blades grind back and forth
        c.point = c.lock_point(self);
        self.clash = Some(c);
    }

    /// The winner throws the loser's blade aside: disarmed and sent flying.
    fn finish_clash(&mut self, win: usize) {
        let Some(c) = self.clash else { return };
        let (w, l) = (c.fighters[win] as usize, c.fighters[1 - win] as usize);
        self.end_clash();
        let dir = (self.fighters[l].pos(&self.world, PELVIS) - self.fighters[w].pos(&self.world, PELVIS)).normalize_or(Vec2::X);
        let lost = self.fighters[l].weapon;
        self.release(l, false);
        if let Some(wi) = lost {
            let wpn = &self.weapons[wi as usize];
            for k in 0..wpn.n {
                self.world.particles[wpn.particle(k) as usize].vel += dir * 8.0 + Vec2::new(0.0, 7.0);
            }
        }
        self.shove(l, S_TORSO, dir, 32.0);
        self.fighters[l].knock_down(80);
        self.clash_banner = Some((w as u8, 110));
        let at = self.fighters[l].pos(&self.world, fighter::NECK);
        self.events.push(Sfx::HitHeavy.at(at).vol(1.5));
        self.events.push(Sfx::Throw.at(at));
    }

    fn end_clash(&mut self) {
        let Some(c) = self.clash.take() else { return };
        for fi in c.fighters {
            if let Some(wi) = self.fighters[fi as usize].weapon {
                let w = self.weapons[wi as usize].clone();
                if w.victim.is_none() {
                    w.set_pass_through(&mut self.world, 0);
                }
            }
        }
        for f in &mut self.fighters {
            f.locked = None;
        }
        self.clash_cd = 240;
    }

    /// What a hit does to the body: whatever was carrying it into the blow
    /// stops dead, then it's thrown back, upper body first so it topples, and
    /// the struck part gets an extra jolt.
    fn shove(&mut self, fi: usize, part: u32, dir: Vec2, k: f32) {
        if k <= 0.0 {
            return;
        }
        let f = &self.fighters[fi];
        let stop = (k / 18.0).min(1.0);
        let push = (k * 0.26).min(8.5);
        for id in f.particle_ids() {
            let slot = id - f.base;
            let weight = match slot {
                fighter::HEAD | fighter::NECK => 1.25,
                fighter::ELBOW_A | HAND_A | fighter::ELBOW_B | HAND_B => 1.0,
                PELVIS => 0.8,
                fighter::KNEE_A | fighter::KNEE_B => 0.45,
                _ => 0.3,
            };
            let p = &mut self.world.particles[id as usize];
            if p.inv_mass == 0.0 {
                continue;
            }
            let into = p.vel.dot(dir);
            if into < 0.0 {
                p.vel -= dir * (into * stop);
            }
            p.vel += (dir * push + Vec2::Y * (push * 0.22)) * weight;
        }
        let seg = self.world.segments[(f.seg_base + part) as usize];
        self.world.add_velocity([seg.a, seg.b], dir * (k * 0.2).min(6.0));
    }

    fn weapon_hit(&mut self, wi: usize, seg: usize, fi: usize, part: u32, point: Vec2, impact: f32, dir: Vec2) {
        let w = &self.weapons[wi];
        if !w.active || w.holder == Some(fi as u8) || w.hit_cd[fi] > 0 {
            return;
        }
        // a grapple never hurts the one tied to it
        if w.def == GRAPPLE && self.nooses.iter().any(|n| !n.cut && n.weapon as usize == wi && n.victim as usize == fi) {
            return;
        }
        // only weapons in someone's hand or freshly thrown hurt, and never
        // the person who is holding or threw them
        let attacker = match w.holder {
            Some(h) => h,
            None if w.thrown > 0 => match w.last_holder {
                Some(h) => h,
                None => return,
            },
            None => return,
        };
        if attacker as usize == fi {
            return;
        }
        let attacker = Some(attacker);
        let threshold = 3.0;
        if impact < threshold || !self.fighters[fi].has(part) {
            return;
        }
        let def = w.def();
        // a thrown knife spins too fast to tell which end met them first: it
        // counts as point first (a stab, at a blade's strength), and stays in
        let point_first = w.holder.is_none() && w.def == KNIFE;
        let kind = if point_first { Kind::Spike } else { def.segs[seg].kind };
        let mult = match kind {
            _ if point_first => 1.0,
            Kind::Blade => 1.0,
            Kind::Spike => 1.25,
            Kind::Blunt => 0.85,
            Kind::Handle => 0.3,
        };
        let loc = match part {
            S_HEAD => 1.8,
            S_TORSO => 1.0,
            _ => 0.7,
        };
        // a clean click-strike (~25 m/s at the tip) does about a third of a
        // health bar to the body; scrappy mouse-waggle hits do a few points
        let dmg = (def.damage * 0.7 * (impact - threshold) * mult * loc).min(90.0);
        if dmg < 1.0 {
            return;
        }
        self.weapons[wi].hit_cd[fi] = 12;
        let impaled = self.try_impale(wi, seg, fi, part, point);
        // a clean run-through slides in instead of batting them away
        // something flying through the air lands with everything it's got
        let thrown = self.weapons[wi].holder.is_none();
        // something flying knocks you the way it was flying, not whichever way
        // a tumbling handle happened to glance off
        let dir = if thrown {
            let w = &self.weapons[wi];
            let n = w.def().points.len() as u8;
            let vel = (0..n).fold(Vec2::ZERO, |v, i| v + self.world.particles[w.particle(i) as usize].vel);
            let flight = vel.normalize_or_zero();
            if flight == Vec2::ZERO { dir } else { (flight + Vec2::Y * 0.25).normalize() }
        } else {
            dir
        };
        let knock = if impaled {
            0.0
        } else {
            impact * def.knock * if thrown { 1.6 } else { 1.0 }
        };
        let dmg = if impaled { dmg + IMPALE_DAMAGE } else { dmg };
        let sound = match kind {
            _ if impaled => Sfx::Impale,
            Kind::Blade | Kind::Spike => Sfx::HitBlade,
            Kind::Blunt if dmg > 22.0 => Sfx::HitHeavy,
            _ => Sfx::HitBlunt,
        };
        self.events.push(sound.at(point).vol((0.55 + dmg / 40.0).min(1.4)));
        let swing = self.world.particles[self.weapons[wi].particle(self.weapons[wi].def().aim.1) as usize].vel;
        match kind {
            _ if impaled => self.wound(fi, part, point, fighter::WoundKind::Stab, 0.045, dir.perp()),
            Kind::Blade if dmg >= 6.0 => self.wound(fi, part, point, fighter::WoundKind::Cut, (0.02 + dmg * 0.0015).min(0.04), swing),
            Kind::Spike if dmg >= 4.0 => self.wound(fi, part, point, fighter::WoundKind::Stab, 0.035, dir.perp()),
            _ => {}
        }
        self.damage(fi, part, dmg, point, dir, knock, kind, attacker);
        if point_first {
            self.stick_in(wi, fi, part, point);
        }
        if impaled {
            self.fighters[fi].knock_down(IMPALE_DOWN);
            self.spawn_blood(point, dir, 12, 5.0);
        }
    }

    /// A fast stab landing with the tip runs the victim through: the blade
    /// stops colliding with them and they're held on its line, free to slide.
    fn try_impale(&mut self, wi: usize, seg: usize, fi: usize, part: u32, point: Vec2) -> bool {
        let w = &self.weapons[wi];
        let Some((iseg, base, tip)) = w.def().impale else { return false };
        if !w.def().skewer || seg != iseg as usize || w.victim.is_some() || self.tossed(fi) || self.weapons.iter().any(|o| o.victim == Some(fi as u8)) {
            return false;
        }
        let (pb, pt) = (self.world.particles[w.particle(base) as usize], self.world.particles[w.particle(tip) as usize]);
        let axis = (pt.pos - pb.pos).normalize_or_zero();
        if pt.vel.dot(axis).max(w.tip_speed) < IMPALE_SPEED || (point - pt.pos).length() > 0.4 {
            return false;
        }
        // run the body through, not just a dangling arm in front of it
        let f = &self.fighters[fi];
        let seg_of = |p: u32| self.world.segments[(f.seg_base + p) as usize];
        let near = |p: u32| {
            let sg = seg_of(p);
            let (a, b) = (self.world.particles[sg.a as usize].pos, self.world.particles[sg.b as usize].pos);
            stickfell_physics::collide::point_segment(pt.pos, a, b)
        };
        let part = if f.has(S_TORSO) && near(S_TORSO).0 < 0.45 {
            S_TORSO
        } else if f.has(S_HEAD) && near(S_HEAD).0 < 0.4 {
            S_HEAD
        } else {
            part
        };
        let vseg = seg_of(part);
        let (va, vb) = (self.world.particles[vseg.a as usize].pos, self.world.particles[vseg.b as usize].pos);
        let s = if vseg.a == vseg.b { 0.0 } else { stickfell_physics::collide::point_segment(point, va, vb).1 };
        let (slot, base_id, tip_id) = (w.skewer, w.particle(base), w.particle(tip));
        self.world.skewers[slot as usize] = stickfell_physics::Skewer {
            u_max: f32::MAX,
            a: vseg.a,
            b: vseg.b,
            s,
            base: base_id,
            tip: tip_id,
            u_min: 0.08,
            friction: 0.02,
            u: 1.0,
            enabled: true,
            anchored: false,
        };
        let w = &mut self.weapons[wi];
        w.victim = Some(fi as u8);
        w.skewer_t = 0;
        w.pass_grace = 0;
        let w = w.clone();
        w.set_pass_through(&mut self.world, fi as u16 + 1);
        // the blade glides on through them
        let ids = self.fighters[fi].particle_ids();
        self.world.add_velocity(ids, -axis * IMPALE_GLIDE);
        true
    }

    /// A swinging ball smashing into someone.
    fn ball_hit(&mut self, fi: usize, part: u32, point: Vec2, impact: f32, dir: Vec2) {
        if impact < 4.0 || self.hazard_cd[fi] > 0 || !self.fighters[fi].alive || !self.fighters[fi].has(part) {
            return;
        }
        self.hazard_cd[fi] = 20;
        let dmg = ((impact - 4.0) * 3.0).min(60.0);
        self.events.push(Sfx::HitHeavy.at(point).vol(1.4));
        self.damage(fi, part, dmg, point, dir, impact * 2.2, Kind::Blunt, None);
    }

    /// Keeps wrecking balls swinging: a nudge along the arc near the bottom
    /// whenever they've slowed below their set speed.
    fn drive_balls(&mut self) {
        for b in &self.balls {
            let anchor = self.world.particles[b.chain[0] as usize].pos;
            let p = &mut self.world.particles[b.ball() as usize];
            let r = p.pos - anchor;
            if r.y > -0.6 * b.def.length {
                continue;
            }
            let t = Vec2::new(-r.y, r.x).normalize_or_zero();
            let vt = p.vel.dot(t);
            let sign = if vt.abs() > 0.5 { vt.signum() } else { b.def.swing.signum() };
            if vt.abs() < b.def.swing.abs() {
                p.vel += t * (sign * 0.3);
            }
        }
    }

    /// Body part a particle belongs to (for hazards that touch particles).
    fn part_of_slot(slot: u32) -> u32 {
        match slot {
            fighter::HEAD => S_HEAD,
            fighter::NECK | fighter::PELVIS => S_TORSO,
            fighter::ELBOW_A => fighter::S_UPPER_A,
            fighter::HAND_A => S_FORE_A,
            fighter::ELBOW_B => fighter::S_UPPER_B,
            fighter::HAND_B => fighter::S_FORE_B,
            fighter::KNEE_A => fighter::S_THIGH_A,
            fighter::FOOT_A => fighter::S_SHIN_A,
            fighter::KNEE_B => fighter::S_THIGH_B,
            _ => fighter::S_SHIN_B,
        }
    }

    /// A spike bed's solid parts: its base plate and a thin core up each
    /// tooth to its point (bodies rest on the points; the damage comes from
    /// touching a tooth's real shape, see `update_hazards`).
    fn add_spike_statics(w: &mut World, h: &HazardDef) {
        if h.kind != HazardKind::Spikes {
            return;
        }
        let (c, half) = h.plate();
        w.add_static(Obb::new(c, half, 0.0).with_friction(0.9).with_tag(crate::level::SPIKE_PLATE_TAG));
        for t in h.teeth() {
            let mid = (t.base + t.tip) * 0.5;
            let half = Vec2::new(crate::level::SPIKE_CORE, (t.tip.y - t.base.y) * 0.5);
            w.add_static(Obb::new(mid, half, 0.0).with_friction(0.6).with_tag(crate::level::SPIKE_TAG));
        }
    }

    /// Practice: brings a dummy back whole (a dead one if there is one, else
    /// the one furthest away), standing a step in front of fighter `me`,
    /// facing them. Everything still tied to its old body lets go first, and
    /// the old body is taken out of play.
    pub fn bring_dummy(&mut self, me: usize) {
        let here = self.fighters[me].pos(&self.world, PELVIS);
        let score = |s: &Self, i: usize| (if s.fighters[i].alive { 0.0 } else { 1e6 }) + (s.fighters[i].pos(&s.world, PELVIS) - here).length();
        let Some(i) = (0..self.fighters.len()).filter(|&i| i != me).max_by(|&a, &b| score(self, a).total_cmp(&score(self, b))) else { return };
        // let go of everything tied to it
        self.release(i, false);
        self.release_grab(i);
        for k in 0..self.fighters.len() {
            if self.fighters[k].hold == Some(i as u8) {
                self.release_grab(k);
            }
        }
        let mut j = 0;
        while j < self.finishers.len() {
            let x = self.finishers[j];
            if x.attacker as usize == i || x.victim as usize == i {
                self.finishers.remove(j);
                self.release_grab(x.attacker as usize);
            } else {
                j += 1;
            }
        }
        for n in self.nooses.iter().filter(|n| n.victim as usize == i) {
            self.world.distances[n.tether as usize].enabled = false;
        }
        self.nooses.retain(|n| n.victim as usize != i);
        for wi in 0..self.weapons.len() {
            if self.weapons[wi].victim == Some(i as u8) {
                self.unskewer(wi);
            }
            if self.weapons[wi].stuck.is_some_and(|(f, _)| f as usize == i) {
                self.unstick(wi);
            }
        }
        if self.clash.is_some_and(|c| c.side_of(i).is_some()) {
            self.clash = None;
        }
        self.unimpale(i);
        // the old body out of play
        let old = self.fighters[i].clone();
        for id in old.particle_ids().chain([old.anchor]) {
            self.world.park_particle(id);
        }
        for k in 0..fighter::BONES.len() as u32 {
            self.world.segments[(old.seg_base + k) as usize].enabled = false;
        }
        for d in [old.grab, old.tether, old.grip_a, old.grip_b] {
            self.world.distances[d as usize].enabled = false;
        }
        // a fresh one a step in front, on whatever's under it
        let facing = self.fighters[me].facing;
        // a step ahead, or further on if someone's standing there already
        let reach = self.world.raycast(here, Vec2::new(facing, 0.0), 3.4).map_or(3.4, |h| (h.t - 0.45).max(0.5));
        let clear = |x: f32| (0..self.fighters.len()).filter(|&k| k != i && k != me).all(|k| (self.fighters[k].pos(&self.world, PELVIS).x - x).abs() > 0.7);
        let ahead = [1.4f32, 2.2, 3.0].into_iter().map(|a| a.min(reach)).find(|&a| clear(here.x + facing * a)).unwrap_or(1.4f32.min(reach));
        let spot = here + Vec2::new(facing * ahead, 1.0);
        let feet = self.world.raycast(spot, Vec2::NEG_Y, 25.0).map_or(spot, |h| h.point + Vec2::new(0.0, 0.02));
        self.fighters[i] = Fighter::spawn(&mut self.world, i as u8, feet, -facing);
        self.hazard_cd[i] = 0;
        self.punch_cd[i] = 0;
        let at = self.fighters[i].pos(&self.world, fighter::NECK);
        self.events.push(Sfx::Pickup.at(at));
    }

    /// Run through on a spike: the body slides down it and stays on it (the
    /// tooth's line through the part it went in by) until they struggle off
    /// the point or bleed out. Head or heart first and hard enough, they die
    /// on it at once.
    fn spike_impale(&mut self, fi: usize, part: u32, s: f32, tooth: crate::level::Tooth, down: f32) {
        let Some(&[pb, pt, slot]) = self.spike_pins.get(fi) else { return };
        for (id, at) in [(pb, tooth.base), (pt, tooth.tip)] {
            let p = &mut self.world.particles[id as usize];
            p.pos = at;
            p.prev = at;
            p.vel = Vec2::ZERO;
        }
        let f = self.fighters[fi].clone();
        let sg = self.world.segments[(f.seg_base + part) as usize];
        let at = self.world.particles[sg.a as usize].pos.lerp(self.world.particles[sg.b as usize].pos, s);
        let u = ((at.y - tooth.base.y) / (tooth.tip.y - tooth.base.y)).max(SPIKE_DEPTH);
        // (if it's still above the point, it's drawn down onto it over a few frames)
        self.world.skewers[slot as usize] = stickfell_physics::Skewer { a: sg.a, b: sg.b, s, base: pb, tip: pt, u_min: SPIKE_DEPTH, u_max: u, friction: 0.05, u, enabled: true, anchored: true };
        // through the other teeth too, down onto the plate
        self.set_spike_skip(fi, crate::level::SPIKE_TAG);
        for id in f.particle_ids() {
            self.world.particles[id as usize].vel.x *= 0.3;
        }
        let fm = &mut self.fighters[fi];
        fm.impaled = 1;
        fm.impale_part = part as u8;
        fm.impale_free = 0;
        self.hazard_cd[fi] = 25;
        let point = Vec2::new(tooth.tip.x, at.y);
        self.events.push(Sfx::Impale.at(point).vol(1.3));
        self.events.push(Sfx::Spikes.at(point));
        self.wound(fi, part, point, fighter::WoundKind::Stab, 0.045, Vec2::Y);
        self.spawn_blood(point, Vec2::Y, 14, 3.5);
        let vital = part == S_HEAD || (part == S_TORSO && s >= 0.5);
        if vital && down >= IMPALE_SPIKE_VITAL {
            self.spawn_blood(point, Vec2::Y, 16, 5.0);
            self.shake_camera(fi, 0.8);
            self.damage(fi, part, 999.0, point, Vec2::Y, 0.0, Kind::Spike, None);
        } else {
            self.damage(fi, part, SPIKE_IMPALE_DAMAGE + down * 2.0, point, Vec2::Y, 0.0, Kind::Spike, None);
        }
    }

    /// Each fighter's spike pins (see `spike_pins`), out of the way till needed.
    pub(crate) fn make_spike_pins(w: &mut World, n: usize) -> Vec<[u32; 3]> {
        (0..n)
            .map(|_| {
                let mut pin = Particle::new(Vec2::new(0.0, -1000.0), 0.0, 0.01);
                pin.collide_static = false;
                [w.add_particle(pin), w.add_particle(pin), w.add_skewer_slot()]
            })
            .collect()
    }

    /// A fighter's body passing through (or not) a kind of static.
    fn set_spike_skip(&mut self, fi: usize, tag: u32) {
        let f = &self.fighters[fi];
        for id in f.particle_ids() {
            self.world.particles[id as usize].skip_tag = tag;
        }
        for k in 0..fighter::BONES.len() as u32 {
            self.world.segments[(f.seg_base + k) as usize].skip_tag = tag;
        }
    }

    /// Off the spike (struggled free, or taken out of play).
    fn unimpale(&mut self, fi: usize) {
        if self.fighters[fi].impaled == 0 {
            return;
        }
        if let Some(&[_, _, slot]) = self.spike_pins.get(fi) {
            self.world.skewers[slot as usize].enabled = false;
        }
        self.set_spike_skip(fi, 0);
        let f = &mut self.fighters[fi];
        f.impaled = 0;
        f.impale_free = 0;
    }

    /// The impaled: bleeding, held on the spike, struggling up it (mash the
    /// movement keys) and off the point.
    fn update_impaled(&mut self) {
        for fi in 0..self.fighters.len() {
            let f = &mut self.fighters[fi];
            f.impale_grace = f.impale_grace.saturating_sub(1);
            if f.impaled == 0 {
                continue;
            }
            f.impaled = f.impaled.saturating_add(1);
            let Some(&[_, _, slot]) = self.spike_pins.get(fi) else { continue };
            let (t, alive) = (f.impaled, f.alive);
            // (anything still above the point comes down onto it)
            {
                let sk = &mut self.world.skewers[slot as usize];
                sk.u_max = (sk.u_max - SPIKE_DRAW).max(sk.u_min).max(if alive { SPIKE_DEPTH } else { 0.0 });
                if alive {
                    sk.u_max = sk.u_max.max(1.02);
                }
            }
            if !alive {
                // the dead stay on it
                self.world.skewers[slot as usize].friction = 1.0;
                continue;
            }
            f.down = f.down.max(8);
            // (a struggle counts at most every few frames, however fast the keys go)
            f.impale_beat = f.impale_beat.saturating_sub(1);
            let pressed = [LEFT, RIGHT, JUMP, CROUCH].iter().any(|&b| f.pressed(b));
            let mash = (pressed && f.impale_beat == 0) as u8;
            if mash > 0 {
                f.impale_beat = SPIKE_BEAT;
            }
            f.impale_free = if mash > 0 {
                f.impale_free.saturating_add(mash * SPIKE_STRUGGLE)
            } else if t % 3 == 0 {
                f.impale_free.saturating_sub(1)
            } else {
                f.impale_free
            };
            let free = f.impale_free;
            // hauled up the spike as they struggle, sliding back down as they tire
            let sk = &mut self.world.skewers[slot as usize];
            sk.u_min = SPIKE_DEPTH + (1.02 - SPIKE_DEPTH) * (free.min(100) as f32 / 100.0);
            sk.friction = 0.1;
            let sk = *sk;
            let at = self.world.particles[sk.a as usize].pos.lerp(self.world.particles[sk.b as usize].pos, sk.s);
            if mash > 0 {
                // thrashing about on it
                let ids: Vec<u32> = self.fighters[fi].particle_ids().collect();
                for id in ids {
                    let kick = Vec2::new(self.rng.range(-0.8, 0.8), self.rng.range(0.0, 0.6));
                    self.world.particles[id as usize].vel += kick;
                }
                if self.rng.range(0.0, 1.0) < 0.4 {
                    self.spawn_blood(at, Vec2::Y, 2, 2.0);
                }
            }
            if free >= 100 {
                // off the point: up and away
                self.unimpale(fi);
                let ids = self.fighters[fi].particle_ids();
                self.world.add_velocity(ids, Vec2::new(0.0, 2.4));
                let f = &mut self.fighters[fi];
                f.impale_grace = 90;
                f.down = f.down.max(20);
                self.hazard_cd[fi] = 40;
                self.events.push(Sfx::Pickup.at(at).vol(0.7));
                continue;
            }
            if t % 7 == 0 {
                self.spawn_blood(at, Vec2::NEG_Y, 2, 1.5);
            }
            let part = self.fighters[fi].impale_part as u32;
            if t > 10 {
                self.damage(fi, part, SPIKE_BLEED, at, Vec2::Y, 0.0, Kind::Spike, None);
            }
        }
    }

    /// A level's liquids, poured in and left to settle before anyone's
    /// there (worked out once per level: it depends on nothing else).
    fn settled_liquid(level: usize, hazards: &[HazardDef], w: &World) -> stickfell_physics::Fluid {
        static SETTLED: std::sync::Mutex<Vec<(usize, stickfell_physics::Fluid)>> = std::sync::Mutex::new(Vec::new());
        if let Some((_, f)) = SETTLED.lock().unwrap().iter().find(|(l, _)| *l == level) {
            return f.clone();
        }
        let mut fluid = stickfell_physics::Fluid::new(crate::liquid::fluid_kinds(), crate::liquid::DROP_SPACING);
        for h in hazards {
            if let Some(kind) = crate::liquid::drop_kind(h.kind) {
                fluid.fill(h.a - h.size, h.a + h.size, kind);
            }
        }
        if !fluid.drops.is_empty() {
            let mut still = World::new(WorldConfig::default());
            still.statics = w.statics.clone();
            fluid.settle(&mut still, LIQUID_SETTLE);
        }
        SETTLED.lock().unwrap().push((level, fluid.clone()));
        fluid
    }

    /// Water touching hot lava boils off in a puff of steam, and cools the
    /// lava it touched (enough of it, and that lava's rock).
    fn water_on_lava(&mut self) {
        use crate::liquid::{COOLED_LAVA, DROP_SPACING, LAVA_COOLING, LAVA_DROP, WATER_DROP};
        self.burst_cd = self.burst_cd.saturating_sub(1);
        let pairs = self.fluid.touching(WATER_DROP, LAVA_DROP, DROP_SPACING * 1.2);
        if pairs.is_empty() {
            return;
        }
        let mut gone = Vec::with_capacity(pairs.len());
        let mut burst: Option<Vec2> = None;
        for &(w, l) in pairs.iter() {
            let at = self.fluid.drops[w].pos;
            let (lava_at, heat) = (self.fluid.drops[l].pos, 1.0 - self.fluid.drops[l].dye);
            // the lava's heat boils the water off (with less than a drop's
            // worth left, only maybe: on average LAVA_BOILS drops a lava drop)
            let boils = heat >= LAVA_COOLING || self.rng.range(0.0, 1.0) < heat / LAVA_COOLING;
            #[cfg(test)]
            if heat > 1.0 - COOLED_LAVA {
                let into = (self.fluid.drops[w].vel - self.fluid.drops[l].vel).dot((lava_at - at).normalize_or_zero());
                self.hot_contacts.push((lava_at.y - at.y, into));
            }
            // water getting under hot lava, or slamming into it, flashes to
            // steam and blows it apart
            let into = (self.fluid.drops[w].vel - self.fluid.drops[l].vel).dot((lava_at - at).normalize_or_zero());
            if burst.is_none() && heat > 1.0 - COOLED_LAVA && (at.y < lava_at.y || into > BURST_IMPACT) {
                burst = Some(at);
            }
            let lava = &mut self.fluid.drops[l];
            lava.dye = (lava.dye + LAVA_COOLING).min(1.0);
            if !boils {
                continue;
            }
            gone.push(w);
            self.steam.add(at, crate::steam::STEAM_PER_DROP);
        }
        if self.frame % 16 == 0 {
            let at = self.fluid.drops[pairs[0].0].pos;
            self.events.push(Sfx::Splash.at(at).vol(0.4));
        }
        self.fluid.remove(&gone);
        if let (Some(at), 0) = (burst, self.burst_cd) {
            self.steam_burst(at);
        }
    }

    /// Water trapped under hot lava flashing to steam: the liquid round it
    /// thrown out, a cloud of steam, and anyone close knocked back and
    /// scalded (much less than the real thing).
    fn steam_burst(&mut self, at: Vec2) {
        self.burst_cd = BURST_COOLDOWN;
        for d in &mut self.fluid.drops {
            let off = d.pos - at;
            let l = off.length();
            if l < BURST_REACH {
                let dir = (off.normalize_or(Vec2::Y) + Vec2::new(0.0, 0.6)).normalize();
                d.vel += dir * (BURST_SPEED * (1.0 - l / BURST_REACH));
            }
        }
        self.steam.add(at, BURST_STEAM);
        for fi in 0..self.fighters.len() {
            let f = &self.fighters[fi];
            if !f.alive {
                continue;
            }
            let (near, part) = [(PELVIS, S_TORSO), (fighter::NECK, S_TORSO), (fighter::HEAD, S_HEAD), (fighter::KNEE_A, fighter::S_THIGH_A), (fighter::KNEE_B, fighter::S_THIGH_B)]
                .into_iter()
                .map(|(slot, part)| ((f.pos(&self.world, slot) - at).length(), part))
                .fold((f32::MAX, S_TORSO), |a, b| if b.0 < a.0 { b } else { a });
            if near >= BURST_HIT {
                continue;
            }
            let k = 1.0 - near / BURST_HIT;
            let dir = (f.pos(&self.world, PELVIS) - at).normalize_or(Vec2::Y);
            let ids = f.particle_ids();
            self.world.add_velocity(ids, (dir + Vec2::new(0.0, 0.5)).normalize() * (BURST_PUSH * k));
            self.damage(fi, part, BURST_DAMAGE * k, at, dir, 0.0, Kind::Blunt, None);
            let f = &mut self.fighters[fi];
            f.scorch = (f.scorch + 0.3 * k).min(1.0);
            if k > 0.5 {
                f.knock_down(40);
            }
        }
        for _ in 0..14 {
            if self.sparks.len() >= 320 {
                break;
            }
            let vel = Vec2::new(self.rng.range(-3.0, 3.0), self.rng.range(1.5, 6.0));
            self.sparks.push(Spark { pos: at, vel, life: 26, max_life: 26, blood: false, tint: 0, steam: false });
        }
        self.events.push(Sfx::HitHeavy.at(at).vol(1.5));
        self.events.push(Sfx::Splash.at(at).vol(1.4));
    }

    /// Acid and lava at work, part by part, on bodies (living or dead) and on
    /// loose limbs: acid eats a part away for as long as it's touching it
    /// until it's gone, an arm or leg dropping off
    /// and the rest of it dissolving where it lies; lava melts the flesh off
    /// down to the bone, and sets the part on fire. Fire burns on after
    /// they're out (hurting the living, charring them), spreads along the
    /// body, and goes out in water.
    fn eat_and_burn(&mut self, dt: f32) {
        use crate::liquid::{ACID_DROP, LAVA_DROP, WATER_DROP};
        let full = fighter::ROT_FULL as f32;
        let empty = self.fluid.drops.is_empty();
        // (touching it at all, it's at work at full rate)
        let gain = |time: f32, _amount: f32| (full * dt / time) as u32;
        let mut sizzle: Vec<Vec2> = Vec::new();
        for fi in 0..self.fighters.len() {
            let f = &self.fighters[fi];
            let on_fire = f.fire.iter().any(|&t| t > 0);
            if empty && !on_fire {
                continue;
            }
            let seg_base = f.seg_base;
            let mut eat_through: Vec<u32> = Vec::new();
            for part in 0..10u32 {
                let f = &self.fighters[fi];
                // (gone, or off and lying about as a limb of its own)
                if !f.has(part) || f.limbs_gone & fighter::bit(part) != 0 || (part == S_HEAD && f.head_popped) {
                    continue;
                }
                let sg = self.world.segments[(seg_base + part) as usize];
                let (a, b) = (self.world.particles[sg.a as usize].pos, self.world.particles[sg.b as usize].pos);
                let wet = if empty { [0.0; 3] } else { self.soak([a, b, (a + b) * 0.5]) };
                let fm = &mut self.fighters[fi];
                let k = part as usize;
                if wet[ACID_DROP as usize] > ROT_FROM {
                    let e = fm.eaten[k] as u32 + gain(ACID_EAT_TIME, wet[ACID_DROP as usize]);
                    // (a torso is never all gone: there's always a ribcage left)
                    let most = if part == S_TORSO { TORSO_MOST } else { fighter::ROT_FULL };
                    fm.eaten[k] = e.min(most as u32) as u16;
                    if fm.eaten[k] >= fighter::ROT_FULL {
                        eat_through.push(part);
                    }
                }
                if wet[LAVA_DROP as usize] > ROT_FROM {
                    let m = fm.melted[k] as u32 + gain(LAVA_MELT_TIME, wet[LAVA_DROP as usize]);
                    fm.melted[k] = m.min(fighter::ROT_FULL as u32) as u16;
                    fm.fire[k] = FIRE_FRAMES;
                } else if fm.fire[k] > 0 && wet[WATER_DROP as usize] > DOUSE {
                    fm.fire[k] = 0;
                    sizzle.push((a + b) * 0.5);
                } else if fm.fire[k] > 0 {
                    fm.fire[k] -= 1;
                }
            }
            // fire spreads along the body, to the parts joined to a part
            // that's well alight
            for k in 0..10usize {
                if self.fighters[fi].fire[k] <= FIRE_FRAMES / 3 {
                    continue;
                }
                for n in fighter::part_neighbours(k as u32) {
                    let f = &self.fighters[fi];
                    let n = *n as usize;
                    if f.fire[n] == 0 && f.has(n as u32) && f.limbs_gone & fighter::bit(n as u32) == 0 && self.rng.range(0.0, 1.0) < FIRE_SPREAD * dt {
                        self.fighters[fi].fire[n] = FIRE_FRAMES * 2 / 3;
                    }
                }
            }
            // eaten right through: an arm or leg drops off (and goes on
            // dissolving where it lies); a head is just gone
            for part in eat_through {
                if part == S_HEAD {
                    let at = self.fighters[fi].pos(&self.world, fighter::HEAD);
                    self.fighters[fi].head_popped = true;
                    if self.fighters[fi].alive {
                        self.damage(fi, S_HEAD, fighter::MAX_HEALTH * 2.0, at, Vec2::Y, 0.0, Kind::Blunt, None);
                    }
                } else if self.sever(fi, part) {
                    let joint = fighter::sever_plan(part).map_or(PELVIS, |(j, _)| j);
                    let at = self.fighters[fi].pos(&self.world, joint);
                    self.events.push(Sfx::Sever.at(at).vol(0.5));
                } else if self.fighters[fi].has(part) && fighter::sever_plan(part).is_some() {
                    // (an upper arm or thigh whose lower half has already
                    // come off: nothing to drop, it's just gone)
                    let f = &self.fighters[fi];
                    let (seg, dist) = ((f.seg_base + part) as usize, (f.dist_base + part - 1) as usize);
                    let (joint, end) = (self.world.segments[seg].a, self.world.segments[seg].b);
                    self.world.segments[seg].enabled = false;
                    self.world.distances[dist].enabled = false;
                    self.world.park_particle(end);
                    let fm = &mut self.fighters[fi];
                    fm.severed |= fighter::bit(part);
                    fm.limbs_gone |= fighter::bit(part);
                    fm.bleed = 220;
                    fm.bleed_at = joint;
                    if part == fighter::S_UPPER_B {
                        self.release_grab(fi);
                    }
                }
            }
            // on fire: it hurts, it chars, and they scream
            let f = &self.fighters[fi];
            let lit = f.fire.iter().filter(|&&t| t > 0).count();
            if lit > 0 {
                let (alive, screaming) = (f.alive, f.burning);
                let head = f.pos(&self.world, fighter::HEAD);
                let fm = &mut self.fighters[fi];
                fm.scorch = (fm.scorch + FIRE_CHAR * lit as f32 / 10.0 * dt).min(1.0);
                if alive {
                    fm.burning = true;
                    if !screaming && (self.frame + fi as u32 * 7) % CHOKE_EVERY == 0 {
                        self.events.push(Sfx::Burning.at(head).line(0, fi as u8));
                    }
                    if self.hazard_cd[fi] == 0 {
                        self.hazard_cd[fi] = 8;
                        self.damage(fi, S_TORSO, FIRE_DPS * lit as f32 * 8.0 / 60.0, head, Vec2::Y, 0.0, Kind::Blunt, None);
                    }
                }
            }
        }
        // loose limbs: the same, the whole limb at once
        for wi in 0..self.weapons.len() {
            let w = &self.weapons[wi];
            if !w.active || w.limb.is_none() || (empty && w.fire == 0) {
                continue;
            }
            let (a, b) = (self.world.particles[w.particle(0) as usize].pos, self.world.particles[w.particle(w.n - 1) as usize].pos);
            let wet = if empty { [0.0; 3] } else { self.soak([a, b, (a + b) * 0.5]) };
            let w = &mut self.weapons[wi];
            if wet[ACID_DROP as usize] > ROT_FROM {
                w.eaten = (w.eaten as u32 + gain(ACID_EAT_TIME, wet[ACID_DROP as usize])).min(fighter::ROT_FULL as u32) as u16;
            }
            if wet[LAVA_DROP as usize] > ROT_FROM {
                w.melted = (w.melted as u32 + gain(LAVA_MELT_TIME, wet[LAVA_DROP as usize])).min(fighter::ROT_FULL as u32) as u16;
                w.fire = FIRE_FRAMES;
            } else if w.fire > 0 && wet[WATER_DROP as usize] > DOUSE {
                w.fire = 0;
                sizzle.push((a + b) * 0.5);
            } else {
                w.fire = w.fire.saturating_sub(1);
            }
            if w.eaten >= fighter::ROT_FULL {
                // (dropped first if someone's swinging it)
                if let Some(h) = w.holder {
                    self.release(h as usize, false);
                }
                let mut w = self.weapons[wi].clone();
                w.park(&mut self.world);
                self.weapons[wi] = w;
            }
        }
        // put out: a hiss, and a wisp of steam
        for at in sizzle {
            self.events.push(Sfx::Splash.at(at).vol(0.4));
            self.steam.add(at + Vec2::new(0.0, 0.3), DOUSE_STEAM);
        }
    }

    /// Rafts ride the water: each settles to the water level just off its
    /// ends (as waves and splashes move it), sits a little lower for each
    /// fighter standing on it and tips toward where they stand, and bobs.
    fn float_rafts(&mut self, dt: f32) {
        for ri in 0..self.rafts.len() {
            let r = self.rafts[ri];
            let (x, half) = (r.def.x, r.def.half);
            let side = half.x + RAFT_LOOK;
            let (lw, rw) = (self.water_level(x - side, r.def.sea), self.water_level(x + side, r.def.sea));
            let top = r.y + half.y;
            let (mut load, mut moment) = (0.0f32, 0.0f32);
            for f in &self.fighters {
                let p = f.pos(&self.world, PELVIS);
                if (p.x - x).abs() < half.x + 0.3 && p.y > top && p.y < top + 1.6 {
                    load += 1.0;
                    moment += (p.x - x).clamp(-half.x, half.x);
                }
            }
            let bob = RAFT_BOB * dmath::sin(self.frame as f32 * DT * RAFT_BOB_RATE + r.phase);
            let want_y = (lw + rw) * 0.5 - RAFT_DRAFT + half.y - load * RAFT_SINK + bob;
            let want_tilt = ((rw - lw) / (2.0 * side) - moment * RAFT_TILT).clamp(-RAFT_MAX_TILT, RAFT_MAX_TILT);
            let k = (RAFT_K * dt).min(1.0);
            let damp = (RAFT_DAMP * dt).min(1.0);
            let raft = &mut self.rafts[ri];
            raft.vy += (want_y - raft.y) * RAFT_K * k - raft.vy * damp;
            raft.y += raft.vy * dt;
            raft.spin += (want_tilt - raft.tilt) * RAFT_K * k - raft.spin * damp;
            raft.tilt = (raft.tilt + raft.spin * dt).clamp(-RAFT_MAX_TILT, RAFT_MAX_TILT);
            let o = &mut self.world.statics[raft.statik as usize];
            o.center = Vec2::new(x, raft.y);
            o.rot = dmath::dir(raft.tilt);
        }
    }

    /// The water's level at `x`, between `sea`'s bottom and a metre over its
    /// still surface (its bottom if there's none there).
    fn water_level(&self, x: f32, sea: (f32, f32)) -> f32 {
        let (mut lo, mut hi) = (sea.0, sea.1 + 1.0);
        for _ in 0..8 {
            let mid = (lo + hi) * 0.5;
            if self.fluid.sample(Vec2::new(x, mid)).amount[crate::liquid::WATER_DROP as usize] > WATER_EDGE {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// How much water, acid and live lava are around a body part (its ends
    /// and middle; cooled lava doesn't count), 0..1 each.
    fn soak(&self, pts: [Vec2; 3]) -> [f32; 3] {
        use crate::liquid::{ACID_DROP, LAVA_DROP, WATER_DROP};
        let mut a = [0.0f32; 3];
        for p in pts {
            let smp = self.fluid.sample(p);
            a[WATER_DROP as usize] += smp.amount[WATER_DROP as usize];
            a[ACID_DROP as usize] += smp.amount[ACID_DROP as usize];
            a[LAVA_DROP as usize] += (smp.amount[LAVA_DROP as usize] - smp.dyed[LAVA_DROP as usize]).max(0.0);
        }
        a.map(|x| (x / 3.0 / SOAKED).min(1.0))
    }

    /// Steam: it rises, spreads and condenses; anyone breathing it in thick
    /// enough is scalded, slowly (time to get out of it).
    fn update_steam(&mut self, dt: f32) {
        self.steam.step(dt);
        let fluid = &self.fluid;
        self.steam.condense_in(dt, |p| fluid.sample(p).total());
        for fi in 0..self.fighters.len() {
            let f = &self.fighters[fi];
            if !f.alive {
                continue;
            }
            let head = f.pos(&self.world, fighter::HEAD);
            let thick = self.steam.at(head).max(self.steam.at(f.pos(&self.world, fighter::NECK)));
            if thick <= SCALD_FROM {
                continue;
            }
            let rate = SCALD_RATE * ((thick - SCALD_FROM) / (1.0 - SCALD_FROM)).min(1.0);
            {
                let f = &mut self.fighters[fi];
                f.scorch = (f.scorch + rate / 100.0 * dt).min(1.0);
                // (crying out in the worst of it)
                if rate > SCALD_RATE * 0.5 {
                    f.burning = true;
                }
            }
            if (self.frame + fi as u32 * 3) % 8 == 0 {
                self.damage(fi, S_TORSO, rate * 8.0 * dt, head, Vec2::Y, 0.0, Kind::Blunt, None);
            }
            let f = &self.fighters[fi];
            if f.burning && f.alive && (self.frame + fi as u32 * 7) % CHOKE_EVERY == 0 {
                self.events.push(Sfx::Burning.at(head).line(0, fi as u8));
            }
        }
    }

    /// The flood trap: a hand on the lever (E or Shift) throws it; the
    /// hatch swings open and the water pours in until the flood's all in.
    fn update_trap(&mut self) {
        let Some(def) = self.trap.def else { return };
        let since = self.trap.since(self.round_frame);
        // (standing by it, on its floor; once it's done moving) throws it
        // the other way: open, or shut again
        let settled = since.is_none_or(|t| t >= LEVER_FRAMES);
        let puller = (0..self.fighters.len()).find(|&i| {
            let f = &self.fighters[i];
            let off = f.pos(&self.world, PELVIS) - def.lever;
            f.alive && f.down == 0 && (f.pressed(PICKUP) || f.pressed(GRAB)) && off.x.abs() < LEVER_REACH && off.y > 0.3 && off.y < 1.6
        });
        if settled && puller.is_some() {
            self.trap.open = !self.trap.open;
            self.trap.toggled = self.round_frame.max(1);
            if self.trap.open {
                self.trap.poured = 0;
            }
            self.events.push(Sfx::Thud.at(def.lever).vol(1.2));
            return;
        }
        let Some(since) = since else { return };
        // the doors: banging open once the lever's over, shut as they close
        if (self.trap.open && since == LEVER_FRAMES) || (!self.trap.open && since == HATCH_FRAMES) {
            self.events.push(Sfx::HitHeavy.at(def.hatch).vol(1.4));
        }
        if !self.trap.open || since < POUR_AFTER || self.fluid.drops.len() as u32 >= def.max_drops {
            return;
        }
        // as much as should be in by now, across the opening, falling (no
        // more than the level can hold)
        let room = def.max_drops - self.fluid.drops.len() as u32;
        let due = ((since - POUR_AFTER + 1) as f32 * def.pour_rate / 60.0) as u32;
        let due = due.min(self.trap.poured + room);
        // (scattered across the opening and through the frame's fall, so it
        // comes down as a column, not in rows)
        let fall = 5.0;
        while self.trap.poured < due {
            let x = def.hatch.x + self.rng.range(-def.hatch_half, def.hatch_half) * 0.9;
            let y = def.hatch.y - crate::liquid::DROP_SPACING * 0.75 - self.rng.range(0.0, fall / 60.0);
            self.fluid.pour(Vec2::new(x, y), Vec2::new(0.0, -fall), crate::liquid::WATER_DROP);
            self.trap.poured += 1;
        }
        if since % 20 == 0 {
            // the roar of it coming down
            let floor = self.world.raycast(def.hatch - Vec2::new(0.0, 0.5), Vec2::NEG_Y, 30.0).map_or(def.hatch, |h| h.point);
            self.events.push(Sfx::Splash.at(floor).vol(0.35));
        }
    }

    /// A fighter's momentum (all their parts).
    fn momentum(&self, f: &Fighter) -> Vec2 {
        f.particle_ids().map(|i| { let p = &self.world.particles[i as usize]; p.vel * p.mass() }).sum()
    }

    /// The lip of a side fighter `fi` could haul themselves out onto, going
    /// `toward` (+1 right, -1 left): the first floor out of the liquid
    /// ahead of them, no higher than their hands reach over their head and
    /// above their waist (its edge x and its top).
    /// The speed that hauls fighter `fi` up over the lip at (`edge`, `top`)
    /// and onto it, going `toward` (+1 right): up past it with the feet clear
    /// of it, and over it by the time they're at the top (v = sqrt(2 g h),
    /// t = v / g), `loss` times faster up for whatever drags at them.
    fn haul_velocity(&self, fi: usize, edge: f32, top: f32, toward: f32, loss: f32) -> Vec2 {
        let g = -self.world.cfg.gravity.y;
        let f = &self.fighters[fi];
        let feet = f.pos(&self.world, fighter::FOOT_A).y.min(f.pos(&self.world, fighter::FOOT_B).y);
        let rise = (top + HAUL_CLEAR - feet).max(0.0);
        let vy = (2.0 * g * rise).sqrt() * loss;
        // (however little there is to climb, they take HAUL_TIME getting
        // over it, not an instant)
        let t = (vy / g).max(HAUL_TIME);
        let across = ((edge - f.pos(&self.world, PELVIS).x) * toward).max(0.0) + HAUL_ONTO;
        Vec2::new(toward * across / t, vy)
    }

    fn ledge_ahead(&self, fi: usize, toward: f32) -> Option<(f32, f32)> {
        let f = &self.fighters[fi];
        let (neck, pelvis) = (f.pos(&self.world, fighter::NECK), f.pos(&self.world, PELVIS));
        let from_y = neck.y + ARM_REACH;
        for k in 1..=HAUL_LOOK {
            let x = neck.x + toward * k as f32 * HAUL_STEP;
            let Some(hit) = self.world.raycast(Vec2::new(x, from_y), Vec2::NEG_Y, from_y - (pelvis.y - 0.3)) else { continue };
            if hit.t <= 0.0 || hit.normal.y < 0.7 {
                continue;
            }
            // (a floor under the liquid isn't a way out)
            if self.fluid.sample(hit.point + Vec2::new(0.0, 0.15)).total() > 0.3 {
                continue;
            }
            return Some((x - toward * HAUL_STEP * 0.5, hit.point.y));
        }
        None
    }

    /// What the liquid does to whatever is in it (the liquid itself is
    /// `self.fluid`: it's pushed aside and splashes by itself). How much of
    /// a body part is in how much of which liquid comes from the drops
    /// around it: water holds a body up (and lets the living swim, and
    /// drowns them if they stay under); acid and lava let it sink, and eat it.
    fn update_liquids(&mut self, dt: f32) {
        // (who was swimming: they keep at it until they're well out of it)
        let was_swimming: Vec<bool> = self.fighters.iter().map(|f| f.swimming).collect();
        let was_burning: Vec<bool> = self.fighters.iter().map(|f| f.burning).collect();
        for f in &mut self.fighters {
            f.stroke = f.stroke.saturating_sub(1);
            f.swimming = false;
            f.burning = false;
        }
        if self.fluid.drops.is_empty() {
            for f in &mut self.fighters {
                f.breath = (f.breath + 3).min(AIR);
                f.wet = false;
                f.wade = 0.0;
                f.surge = 0.0;
            }
            return;
        }
        for fi in 0..self.fighters.len() {
            let pelvis = self.fighters[fi].pos(&self.world, PELVIS);
            self.fighters[fi].wade = self.fluid.sample(pelvis).total();
        }
        let g = -self.world.cfg.gravity.y;
        let liquids = crate::liquid::LIQUIDS;
        let active: Vec<bool> = self.fighters.iter().zip(&was_swimming).map(|(f, &was)| {
            let held = f.held(LEFT) || f.held(RIGHT) || f.held(JUMP) || f.held(CROUCH);
            was && held && f.alive && f.down == 0
        }).collect();
        // (alive and struggling but without the limbs left to swim: they go
        // down, rather than float like the dead)
        let sinking: Vec<bool> = self.fighters.iter().map(|f| f.alive && f.down == 0 && f.swim_power() <= 0.0).collect();
        // (how much liquid a part is in, and which: buoyancy, drag and harm blended by kind)
        let blend = |amount: [f32; 4]| {
            let tot: f32 = amount[..3].iter().sum();
            let k = |f: &dyn Fn(&crate::liquid::Liquid) -> f32| (0..3).map(|i| f(liquids[i]) * amount[i]).sum::<f32>() / tot.max(1e-5);
            (tot.min(1.0), k(&|l| l.buoyancy), k(&|l| l.drag))
        };
        for fi in 0..self.fighters.len() {
            let ids = self.fighters[fi].particle_ids();
            let base = self.fighters[fi].base;
            let mut by_kind = [0.0f32; 3];
            let mut wet = 0u32;
            let mut splash: Option<(Vec2, f32)> = None;
            let mut highest: Option<Vec2> = None;
            // (in control of themselves, on their feet or swimming, the
            // water's drag is taken as a whole: dragging at their legs alone
            // would tip them over, or brake every turn and kick; limp, it
            // tumbles them)
            let upright = { let f = &self.fighters[fi]; f.alive && f.down == 0 };
            let (mut drag_dp, mut mass) = (Vec2::ZERO, 0.0f32);
            for id in ids.clone() {
                let p = &self.world.particles[id as usize];
                if p.inv_mass > 0.0 {
                    mass += p.mass();
                }
            }
            for id in ids.clone() {
                let p = self.world.particles[id as usize];
                if p.inv_mass == 0.0 {
                    continue;
                }
                let smp = self.fluid.sample(p.pos);
                let (sub, buoy, drag) = blend(smp.amount);
                if sub < 0.08 {
                    continue;
                }
                wet += 1;
                for k in 0..3 {
                    by_kind[k] += smp.amount[k];
                }
                // (lava cooled to rock is just a weight on them)
                by_kind[crate::liquid::LAVA_DROP as usize] -= smp.dyed[crate::liquid::LAVA_DROP as usize];
                if highest.is_none_or(|h| p.pos.y > h.y) {
                    highest = Some(p.pos);
                }
                let q = &mut self.world.particles[id as usize];
                // (stroking, a swimmer holds their depth: weightless in it,
                // going where they swim)
                if sinking[fi] {
                    q.vel.y += g * SINKING_BUOYANCY * sub * dt;
                } else if active[fi] {
                    // (stroking holds them up as well as their limbs let it)
                    let p = self.fighters[fi].swim_power();
                    q.vel.y += g * (SINKING_BUOYANCY + (SWIM_BUOYANCY - SINKING_BUOYANCY) * p) * dt;
                } else {
                    q.vel.y += g * buoy * sub * dt;
                }
                // dragged along with the liquid around it
                let dv = (smp.vel - q.vel) * (drag * sub * dt).min(1.0);
                if upright {
                    drag_dp += dv * q.mass();
                } else {
                    q.vel += dv;
                }
                // the body going in fast: a splash (the drops fly by themselves)
                let _ = base;
                if !self.fighters[fi].wet && p.vel.y < -3.5 && splash.is_none_or(|(_, v)| p.vel.y < v) {
                    splash = Some((p.pos, p.vel.y));
                }
            }
            if upright && mass > 0.0 {
                let dv = drag_dp / mass;
                for id in ids.clone() {
                    let q = &mut self.world.particles[id as usize];
                    if q.inv_mass > 0.0 {
                        q.vel += dv;
                    }
                }
            }
            if let Some((at, v)) = splash {
                self.events.push(Sfx::Splash.at(at).vol((-v / 8.0).clamp(0.3, 1.4)));
                let (mut x0, mut x1, mut low) = (f32::MAX, f32::MIN, f32::MAX);
                for id in ids.clone() {
                    let q = self.world.particles[id as usize].pos;
                    x0 = x0.min(q.x);
                    x1 = x1.max(q.x);
                    low = low.min(q.y);
                }
                self.fluid.splash(x0, x1, low, -v);
            }
            self.fighters[fi].wet = wet > 0;
            // which liquid they're mostly in
            let kind = (0..3).max_by(|&a, &b| by_kind[a].total_cmp(&by_kind[b])).unwrap_or(0);
            let liq = liquids[kind];
            // breath: held under, run out, drown
            let f = &self.fighters[fi];
            let head = self.world.particles[f.p(fighter::HEAD) as usize];
            let at_head = self.fluid.sample(head.pos);
            let head_under = f.has(S_HEAD) && at_head.amount[..3].iter().zip(liquids).any(|(a, l)| l.drowns && *a > HEAD_UNDER);
            if f.alive && head_under {
                let f = &mut self.fighters[fi];
                f.breath = f.breath.saturating_sub(1);
                if f.breath == 0 && (self.frame + fi as u32) % 8 == 0 {
                    self.events.push(Sfx::Choke.at(head.pos).line(0, fi as u8));
                    self.damage(fi, S_HEAD, DROWN_DAMAGE, head.pos, Vec2::Y, 0.0, Kind::Blunt, None);
                }
                if (self.frame + fi as u32 * 5) % 20 == 0 && self.sparks.len() < 320 {
                    self.sparks.push(Spark { pos: head.pos, vel: Vec2::new(0.0, 0.6), life: 40, max_life: 40, blood: false, tint: liq.foam.0, steam: true });
                }
            } else {
                let f = &mut self.fighters[fi];
                f.breath = (f.breath + 3).min(AIR);
            }
            if wet == 0 {
                continue;
            }
            // in acid or lava, alive: screaming (straight away going in, then
            // again each time they've finished)
            if liq.damage > 0.0 && by_kind[kind] > 0.0 && self.fighters[fi].alive {
                self.fighters[fi].burning = true;
                if !was_burning[fi] || (self.frame + fi as u32 * 7) % CHOKE_EVERY == 0 {
                    self.events.push(Sfx::Burning.at(head.pos).line(0, fi as u8));
                }
            }
            let share = wet as f32 / fighter::PARTICLES as f32;
            {
                let f = &mut self.fighters[fi];
                f.scorch = (f.scorch + liq.scorch * share * dt).min(1.0);
            }
            // steam off whatever's cooking in it (into the steam itself: a
            // wisp, nowhere near enough to scald)
            if liq.steams && (self.frame + fi as u32 * 3) % 5 == 0 {
                let at = highest.unwrap_or(self.fighters[fi].pos(&self.world, PELVIS)) + Vec2::new(0.0, 0.3);
                self.steam.add(at, COOK_STEAM);
            }
            // eaten away, the more of them in it the faster
            if liq.damage > 0.0 && self.fighters[fi].alive && self.hazard_cd[fi] == 0 {
                self.hazard_cd[fi] = 8;
                let at = highest.unwrap_or(self.fighters[fi].pos(&self.world, PELVIS));
                let dmg = liq.damage * 8.0 / 60.0 * (0.5 + 0.5 * share);
                self.damage(fi, S_TORSO, dmg, at, Vec2::Y, 0.0, Kind::Blunt, None);
            }
            if !self.fighters[fi].alive {
                self.fighters[fi].burning = false;
            }
            // swimming (waist deep, or out of their depth): the keys steer
            // them through it; at the surface, jump leaps up out of it
            let f = &self.fighters[fi];
            let pelvis = f.pos(&self.world, PELVIS);
            let deep = if was_swimming[fi] { f.wade > SWIM_KEEP && (!f.grounded || f.wade > SWIM_DEPTH) } else { f.wade > SWIM_DEPTH && (!f.grounded || f.wade > SWIM_DEPTH_STANDING) };
            if !f.alive || liq.swim <= 0.0 || !deep || f.impaled > 0 || f.down > 0 || f.swim_power() <= 0.0 {
                continue;
            }
            self.fighters[fi].swimming = true;
            let f = &self.fighters[fi];
            let dir = f.swim_dir();
            let head_out = self.fluid.sample(f.pos(&self.world, fighter::HEAD)).total() < 0.3;
            // at the side, jump hauls them up out onto it (the way they're
            // pushing, or facing); held, they swim up until they can reach it
            let toward = if f.held(RIGHT) != f.held(LEFT) { if f.held(RIGHT) { 1.0 } else { -1.0 } } else { f.facing };
            // (a hand to haul with)
            let haul = if f.held(JUMP) && f.stroke == 0 && (f.arm_a_ok() || f.arm_b_ok()) { self.ledge_ahead(fi, toward) } else { None };
            let leap = f.pressed(JUMP) && f.stroke == 0 && head_out && haul.is_none();
            let body = f.body_vel(&self.world);
            let want = dir * SWIM_SPEED * liq.swim * f.swim_power();
            // (just leapt or hauled: the stroke doesn't fight it)
            let free = f.stroke == 0;
            let haul_vel = haul.map(|(edge, top)| self.haul_velocity(fi, edge, top, toward, HAUL_LOSS));
            // (stroking steers the body as a whole: pulling each part toward
            // the same speed would hold them stiff, every turn and kick braked)
            let steer = (want - body) * (SWIM_STEER * dt).min(1.0);
            for id in ids.clone() {
                let q = &mut self.world.particles[id as usize];
                if let Some(v) = haul_vel {
                    q.vel = v;
                    continue;
                }
                if !free {
                } else if dir != Vec2::ZERO {
                    // (along the way they're swimming, against what the water does)
                    q.vel += steer;
                } else {
                    // treading water: limbs settle with the body instead of
                    // flailing, and the bobbing calms down
                    q.vel += (body - q.vel) * 0.15;
                    q.vel.y *= 1.0 - (TREAD_DAMP * dt).min(1.0);
                }
                if leap {
                    q.vel.y = q.vel.y.max(0.0) + 4.5 * liq.swim;
                }
            }
            if leap || haul.is_some() {
                let f = &mut self.fighters[fi];
                f.stroke = 18;
                if haul.is_some() {
                    f.facing = toward;
                }
                self.events.push(Sfx::Splash.at(pelvis).vol(0.35));
            }}
        // loose weapons: metal and wood sink, slowly in water
        for wi in 0..self.weapons.len() {
            let w = &self.weapons[wi];
            if !w.active || w.holder.is_some() {
                continue;
            }
            for k in 0..w.n {
                let id = w.particle(k) as usize;
                let p = self.world.particles[id];
                if p.inv_mass == 0.0 {
                    continue;
                }
                let smp = self.fluid.sample(p.pos);
                let (sub, _, drag) = blend(smp.amount);
                if sub < 0.08 {
                    continue;
                }
                // (no lift of its own: the drops pressing on it from below
                // already hold it up as much as the water it pushes aside)
                let q = &mut self.world.particles[id];
                q.vel += (smp.vel - q.vel) * (drag * sub * dt).min(1.0);
            }
        }
    }

    /// Adds a hazard to the arena as it stands (its solid parts too).
    #[allow(dead_code)] // (tests build arenas this way)
    pub fn add_hazard(&mut self, h: HazardDef) {
        Self::add_spike_statics(&mut self.world, &h);
        if let Some(kind) = crate::liquid::drop_kind(h.kind) {
            self.fluid.fill(h.a - h.size, h.a + h.size, kind);
        }
        self.hazards.push(h);
    }

    /// Is `p` in or near a spike bed, acid vat or saw? (Bots steer by this.)
    pub fn hazard_near(&self, p: Vec2, margin: f32) -> bool {
        self.hazards.iter().any(|h| match h.kind {
            HazardKind::Saw => (p - h.center(self.round_frame)).length() < h.size.x + margin,
            _ => {
                let d = (p - h.a).abs();
                d.x < h.size.x + margin && d.y < h.size.y + margin
            }
        })
    }

    fn update_hazards(&mut self) {
        for cd in &mut self.hazard_cd {
            *cd = cd.saturating_sub(1);
        }
        for hi in 0..self.hazards.len() {
            let h = self.hazards[hi];
            for fi in 0..self.fighters.len() {
                // (a spike can still run someone through while it's cooling off from a graze)
                if (self.hazard_cd[fi] > 0 && h.kind != HazardKind::Spikes) || !self.fighters[fi].alive {
                    continue;
                }
                let f = &self.fighters[fi];
                let cooling = self.hazard_cd[fi] > 0;
                match h.kind {
                    HazardKind::Spikes if f.impaled > SPIKE_SETTLE => {}
                    HazardKind::Spikes => {
                        // only a tooth's real shape hurts: brushing its side a
                        // little, coming down on its point a lot, taking one
                        // square and hard enough runs them through
                        let teeth = h.teeth();
                        // (just run through one spike: only another point through the head or heart matters)
                        let settling = f.impaled > 0;
                        let mut hit: Option<(u32, Vec2, f32)> = None;
                        // (part, how far along it, which tooth, how hard, and how bad: fatal, then body before limb)
                        let mut through: Option<(u32, f32, usize, f32, (bool, bool))> = None;
                        for part in 0..10u32 {
                            if !f.has(part) || (settling && part != S_HEAD && part != S_TORSO) {
                                continue;
                            }
                            let sg = self.world.segments[(f.seg_base + part) as usize];
                            let (pa, pb) = (self.world.particles[sg.a as usize], self.world.particles[sg.b as usize]);
                            // (as fast as they were coming down before the points stopped them)
                            let fall = |id: u32| self.fall.get(id as usize).copied().unwrap_or(0.0);
                            let (fa, fb) = (fall(sg.a), fall(sg.b));
                            for k in 0..5 {
                                let along = k as f32 / 4.0;
                                let q = pa.pos.lerp(pb.pos, along);
                                if (q.x - h.a.x).abs() > h.size.x + sg.radius + 0.1 {
                                    continue;
                                }
                                // (the joints' own collision can be fatter than the bone's)
                                let r = sg.radius.max(pa.radius + (pb.radius - pa.radius) * along) + 0.01;
                                for (ti, tooth) in teeth.iter().enumerate() {
                                    let Some(t) = tooth.touch(q, r) else { continue };
                                    let down = (-(pa.vel.y + (pb.vel.y - pa.vel.y) * along)).max(fa + (fb - fa) * along);
                                    // coming down hard: on the point, the full force; down
                                    // the side of a tooth, most of it; just brushing one, a graze
                                    let point_first = t > 0.6 && down > 0.5;
                                    let hard = if down > 1.5 { 8.0 + down * 3.0 } else { 4.0 };
                                    let dmg = (if point_first { hard } else { hard * 0.7 }).min(45.0);
                                    if hit.is_none_or(|(_, _, d)| dmg > d) {
                                        hit = Some((part, Vec2::new(tooth.tip.x, q.y), dmg));
                                    }
                                    if point_first && down >= IMPALE_SPIKE_SPEED && SPIKE_IMPALES.contains(&part) && f.impale_grace == 0 {
                                        let vital = part == S_HEAD || (part == S_TORSO && along >= 0.5);
                                        let rank = (vital && down >= IMPALE_SPIKE_VITAL, part == S_HEAD || part == S_TORSO);
                                        if through.is_none_or(|x| rank > x.4) {
                                            through = Some((part, along, ti, down, rank));
                                        }
                                    }
                                }
                            }
                        }
                        if settling {
                            if let Some((part, _, ti, _, (true, _))) = through {
                                let at = teeth[ti].tip - Vec2::new(0.0, 0.05);
                                self.wound(fi, part, at, fighter::WoundKind::Stab, 0.045, Vec2::Y);
                                self.spawn_blood(at, Vec2::Y, 24, 5.0);
                                self.events.push(Sfx::Impale.at(at).vol(1.3));
                                self.shake_camera(fi, 0.8);
                                self.damage(fi, part, 999.0, at, Vec2::Y, 0.0, Kind::Spike, None);
                            }
                            continue;
                        }
                        if let Some((part, along, ti, down, _)) = through {
                            self.spike_impale(fi, part, along, teeth[ti], down);
                            continue;
                        }
                        let Some((part, at, dmg)) = hit.filter(|_| !cooling) else { continue };
                        self.hazard_cd[fi] = 25;
                        self.events.push(Sfx::Spikes.at(at));
                        if dmg > 10.0 {
                            self.wound(fi, part, at, fighter::WoundKind::Stab, 0.03, Vec2::Y);
                            self.spawn_blood(at, Vec2::Y, 6, 3.0);
                        }
                        self.damage(fi, part, dmg, at, Vec2::Y, if dmg > 10.0 { 6.0 } else { 2.0 }, Kind::Spike, None);
                    }
                    // (what liquids do is in `update_liquids`)
                    HazardKind::Acid | HazardKind::Water | HazardKind::Lava => {}
                    HazardKind::Saw => {
                        let c = h.center(self.round_frame);
                        let mut hit = None;
                        for part in 0..10u32 {
                            if !f.has(part) {
                                continue;
                            }
                            let sg = self.world.segments[(f.seg_base + part) as usize];
                            let (pa, pb) = (self.world.particles[sg.a as usize].pos, self.world.particles[sg.b as usize].pos);
                            let (dist, t) = stickfell_physics::collide::point_segment(c, pa, pb);
                            if dist < h.size.x + sg.radius {
                                hit = Some((part, pa + (pb - pa) * t));
                                break;
                            }
                        }
                        let Some((part, at)) = hit else { continue };
                        self.hazard_cd[fi] = 14;
                        let dir = (at - c).normalize_or(Vec2::Y);
                        self.spawn_sparks(at, dir, 10, 7.0);
                        self.events.push(Sfx::Saw.at(at));
                        self.damage(fi, part, 24.0, at, dir, 20.0, Kind::Blade, None);
                    }
                }
            }
        }
    }

    /// Sticks fast point-first blades into the level, and wrenches stuck ones
    /// out when their holder pulls or strikes hard enough for long enough.
    fn update_embed(&mut self, wi: usize) {
        if let Some(h) = self.weapons[wi].holder {
            // a grip taken softly (on a stuck handle) firms up over a few
            // frames: the hand pulls itself on, nothing snaps
            let grip = &mut self.world.distances[self.fighters[h as usize].grip_a as usize];
            if grip.compliance > 0.0 {
                grip.compliance = if grip.compliance < 1e-5 { 0.0 } else { grip.compliance * 0.75 };
            }
        }
        let w = &self.weapons[wi];
        let Some((_, base, tip)) = w.def().impale else { return };
        let (pb, pt) = (self.world.particles[w.particle(base) as usize], self.world.particles[w.particle(tip) as usize]);
        let axis = (pt.pos - pb.pos).normalize_or_zero();
        if !w.embedded {
            // a stab with the point already against a wall drives it in too
            let point_blank = w.holder.is_some_and(|h| {
                let f = &self.fighters[h as usize];
                f.swing_thrust && f.striking()
            });
            if (w.tip_speed < EMBED_SPEED && !point_blank) || (w.holder.is_none() && w.thrown == 0) {
                return;
            }
            // tip at (or within reach of) the level, pointing into it: a fast
            // stab carries it the rest of the way in
            let reach = 0.16;
            let hit = self.world.statics.iter().filter(|o| !crate::level::is_spike(o.tag)).filter_map(|o| o.collide_circle(pt.pos, pt.radius + reach)).find(|(n, _)| axis.dot(*n) < -0.55);
            if let Some((n, pen)) = hit {
                let gap = (reach - pen).max(0.0) / (-axis.dot(n));
                let mut w = self.weapons[wi].clone();
                w.embed(&mut self.world, axis, gap + EMBED_DEPTH);
                w.lever = w.holder.is_some();
                // the stab that drove it in stops dead (and doesn't count as pulling)
                if let Some(h) = w.holder {
                    self.fighters[h as usize].swing = 0;
                }
                self.weapons[wi] = w;
                self.spawn_sparks(pt.pos, -axis, 8, 4.0);
                self.events.push(Sfx::Embed.at(pt.pos));
            }
            return;
        }
        // stuck: pulling hard on the handle (the hand can't reach where it's
        // being driven) or striking at it builds up until it pops out
        let mut pull = 0;
        if let Some(h) = w.holder {
            let f = &self.fighters[h as usize];
            let d = self.world.drives[f.drive_a as usize];
            let hand = self.world.particles[f.p(HAND_A) as usize].pos;
            let neck = f.pos(&self.world, fighter::NECK);
            let strain = (hand - (neck + d.offset)).length();
            if f.striking() {
                pull = 3;
            } else if strain > 0.3 && f.strength() > 0.5 && self.hanging_from(h as usize).is_none() {
                pull = 1;
            }
        }
        let w = &mut self.weapons[wi];
        w.yank = if pull > 0 { w.yank.saturating_add(pull) } else { w.yank.saturating_sub(1) };
        if w.yank >= YANK_OUT {
            // a body pinned on it slides off as it comes out of the wall
            if self.weapons[wi].pinned {
                self.unskewer(wi);
            }
            let mut w = self.weapons[wi].clone();
            w.unembed(&mut self.world, -axis * 3.0);
            self.weapons[wi] = w;
            self.spawn_sparks(pt.pos, -axis, 6, 3.0);
            self.events.push(Sfx::Unembed.at(pt.pos));
        }
    }

    /// A lopsided weapon (a gun) turned upside down in someone's hand is
    /// mirrored across its aim line instead, rest shape and all, so the
    /// grip and the aim stay exactly where they were.
    fn keep_upright(&mut self) {
        for wi in 0..self.weapons.len() {
            let w = &self.weapons[wi];
            let def = w.def();
            let Some(rg) = w.rigid else { continue };
            let Some(h) = w.holder.filter(|_| w.active && !w.embedded) else { continue };
            if !def.lopsided() {
                // nothing to move: only its drawing turns over with its holder
                let left = self.fighters[h as usize].facing < 0.0;
                self.weapons[wi].flip = left;
                continue;
            }
            let pos = |i: u8| self.world.particles[w.particle(i) as usize].pos;
            let axis = (pos(def.aim.1) - pos(def.aim.0)).normalize_or_zero();
            let up = axis.perp() * if w.flip { -1.0 } else { 1.0 };
            // (a margin either side of straight up, so aiming overhead doesn't
            // flip it back and forth)
            if axis == Vec2::ZERO || up.y > -0.25 {
                continue;
            }
            let g = pos(def.grip);
            let mirror = |p: Vec2| g + axis * (2.0 * (p - g).dot(axis)) - (p - g);
            for i in 0..w.n {
                let p = &mut self.world.particles[w.particle(i) as usize];
                p.pos = mirror(p.pos);
                p.prev = mirror(p.prev);
            }
            let r = self.world.rigids[rg as usize];
            for q in &mut self.world.rigid_rest[r.rest as usize..(r.rest + r.count) as usize] {
                q.y = -q.y;
            }
            self.weapons[wi].flip = !self.weapons[wi].flip;
        }
    }

    /// A thrown knife that hit blade first stays in: laid along the way it
    /// was flying with its point buried a little past where it hit, carried
    /// on that body part until someone pulls it out.
    fn stick_in(&mut self, wi: usize, fi: usize, part: u32, point: Vec2) {
        let w = &self.weapons[wi];
        let def = w.def();
        let n = w.n as usize;
        if n > 6 || !self.fighters[fi].has(part) {
            return;
        }
        let pos = |i: usize| self.world.particles[w.particle(i as u8) as usize].pos;
        let vel = (0..n).fold(Vec2::ZERO, |v, i| v + self.world.particles[w.particle(i as u8) as usize].vel);
        let dir = vel.normalize_or(Vec2::X);
        // its points as they are, measured from its tip along and across it
        let tip = def.impale.map_or(def.aim.1, |i| i.2) as usize;
        let along = (pos(tip) - pos(def.grip as usize)).normalize_or(Vec2::X);
        let buried = point + dir * KNIFE_BURIED;
        let f = &self.fighters[fi];
        let (c, axis) = f.part_frame(&self.world, part);
        let across = axis.perp();
        let mut local = [Vec2::ZERO; 6];
        for (i, l) in local.iter_mut().enumerate().take(n) {
            let d = pos(i) - pos(tip);
            let p = buried + dir * d.dot(along) + dir.perp() * d.dot(along.perp());
            *l = Vec2::new((p - c).dot(axis), (p - c).dot(across));
        }
        let w = &mut self.weapons[wi];
        w.stuck = Some((fi as u8, part as u8));
        w.stuck_at = local;
        w.thrown = 0;
        for k in 0..w.nseg as u32 {
            self.world.segments[(w.seg_base + k) as usize].enabled = false;
        }
        for i in 0..w.n {
            let p = &mut self.world.particles[w.particle(i) as usize];
            p.collide_static = false;
            p.vel = Vec2::ZERO;
        }
        self.events.push(Sfx::Embed.at(point).vol(0.8));
        self.carry_stuck();
    }

    /// Knives stuck in bodies go wherever those parts go (and drop out if
    /// the part comes off).
    fn carry_stuck(&mut self) {
        for wi in 0..self.weapons.len() {
            let Some((fi, part)) = self.weapons[wi].stuck else { continue };
            let f = &self.fighters[fi as usize];
            if !f.has(part as u32) || f.limbs_gone & fighter::bit(part as u32) != 0 {
                self.unstick(wi);
                continue;
            }
            let (c, axis) = f.part_frame(&self.world, part as u32);
            let across = axis.perp();
            let w = &self.weapons[wi];
            for i in 0..w.n {
                let l = w.stuck_at[i as usize];
                let p = &mut self.world.particles[w.particle(i) as usize];
                p.pos = c + axis * l.x + across * l.y;
                p.prev = p.pos;
                p.vel = Vec2::ZERO;
            }
        }
    }

    /// Out of the body: an ordinary loose weapon again.
    fn unstick(&mut self, wi: usize) {
        let w = &mut self.weapons[wi];
        if w.stuck.take().is_none() {
            return;
        }
        for k in 0..w.nseg as u32 {
            self.world.segments[(w.seg_base + k) as usize].enabled = true;
        }
        for i in 0..w.n {
            self.world.particles[w.particle(i) as usize].collide_static = true;
        }
    }

    /// Takes whoever is on this weapon's blade off it.
    fn unskewer(&mut self, wi: usize) {
        let Some(v) = self.weapons[wi].victim.take() else { return };
        self.world.skewers[self.weapons[wi].skewer as usize].enabled = false;
        self.weapons[wi].pass_grace = 15;
        let f = &mut self.fighters[v as usize];
        f.down = f.down.max(UNSKEWER_DOWN);
    }

    fn resolve_hits(&mut self) {
        for ci in 0..self.world.contacts.len() {
            let c = self.world.contacts[ci];
            let ta = self.world.segments[c.seg_a as usize].tag;
            let tb = self.world.segments[c.seg_b as usize].tag;
            match (who(ta), who(tb)) {
                // normal points from b to a
                (Who::Weapon(w, s), Who::Fighter(f, p)) => self.weapon_hit(w, s, f, p, c.point, c.impact, -c.normal),
                (Who::Fighter(f, p), Who::Weapon(w, s)) => self.weapon_hit(w, s, f, p, c.point, c.impact, c.normal),
                (Who::Ball, Who::Fighter(f, p)) => self.ball_hit(f, p, c.point, c.impact, -c.normal),
                (Who::Fighter(f, p), Who::Ball) => self.ball_hit(f, p, c.point, c.impact, c.normal),
                (Who::Weapon(w1, s1), Who::Weapon(w2, s2)) if c.impact > 5.0 => {
                    self.try_clash(w1, s1, w2, s2, c.point, c.impact);
                    // only clang when someone is actually swinging or throwing:
                    // loose weapons settling against each other stay quiet
                    let live = |w: &Weapon| w.holder.is_some() || w.thrown > 0;
                    if live(&self.weapons[w1]) || live(&self.weapons[w2]) {
                        let n = (c.impact * 0.8) as u32;
                        self.spawn_sparks(c.point, c.normal, n.min(16), c.impact * 0.6);
                        self.events.push(Sfx::Clash.at(c.point).vol((c.impact / 12.0).min(1.3)));
                    }
                }
                (Who::Fighter(a, pa), Who::Fighter(b, pb)) => {
                    for (att, apart, vic, vpart, dir) in [(a, pa, b, pb, -c.normal), (b, pb, a, pa, c.normal)] {
                        if self.punch_cd[att] > 0 || !self.fighters[vic].has(vpart) || self.fighters[att].hold == Some(vic as u8) && !self.fighters[att].striking() {
                            continue;
                        }
                        let f = &self.fighters[att];
                        let kick_leg = matches!(apart, fighter::S_SHIN_A | fighter::S_THIGH_A)
                            || (f.kick_kind == fighter::KICK_DROP && matches!(apart, fighter::S_SHIN_B | fighter::S_THIGH_B));
                        if f.alive && f.kicking() && kick_leg && c.impact > 2.0 {
                            self.kick_hit(att, vic, vpart, c.point, c.impact);
                        } else if f.punch_part() == Some(apart) && c.impact > 2.5 {
                            self.punch_cd[att] = 6;
                            let head = vpart == S_HEAD;
                            let dmg = (2.0 + (c.impact - 2.5) * 1.2) * if head { 1.35 } else { 1.0 };
                            if dmg >= 1.0 {
                                self.events.push(Sfx::Punch.at(c.point).vol((0.6 + dmg / 18.0).min(1.4)));
                                self.damage(vic, vpart, dmg, c.point, dir, c.impact * 0.8, Kind::Blunt, Some(att as u8));
                                if head {
                                    // the head snaps back
                                    let hid = self.fighters[vic].p(fighter::HEAD) as usize;
                                    self.world.particles[hid].vel += dir * (c.impact * 0.2).min(2.5);
                                }
                            }
                        } else if self.fighters[att].flying > 0 && self.fighters[att].flung_by as usize != vic && c.impact > 6.0 && self.fighters[vic].alive {
                            // a thrown body bowls over whoever it lands on
                            self.punch_cd[att] = 20;
                            let by = Some(self.fighters[att].flung_by);
                            self.events.push(Sfx::HitHeavy.at(c.point).vol(1.1));
                            self.damage(vic, vpart, 4.0 + c.impact * 0.5, c.point, dir, c.impact * 1.6, Kind::Blunt, by);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// A thrown body that stops dead (a wall, the floor) takes the hit.
    fn slam_thrown(&mut self, before: &[Option<Vec2>]) {
        // slammed down: the hit lands as soon as they touch anything solid
        for i in 0..self.fighters.len() {
            let f = &self.fighters[i];
            if f.slam <= 0.0 || !f.particle_ids().any(|id| self.world.particles[id as usize].on_ground()) {
                continue;
            }
            let (dmg, at, by) = (f.slam, f.pos(&self.world, fighter::NECK), Some(f.flung_by));
            let f = &mut self.fighters[i];
            f.slam = 0.0;
            f.flying = 0;
            self.events.push(Sfx::Thud.at(at).vol(1.5));
            self.shake_camera(i, 0.7);
            if let Some(b) = by {
                self.shake_camera(b as usize, 0.35);
            }
            if self.fighters[i].alive {
                self.damage(i, S_TORSO, dmg, at, Vec2::NEG_Y, 0.0, Kind::Blunt, by);
            }
        }
        for (i, v0) in before.iter().enumerate() {
            let Some(v0) = *v0 else { continue };
            let f = &self.fighters[i];
            let v1 = f.body_vel(&self.world);
            let lost = (v0 - v1).length();
            if lost < SLAM_SPEED {
                continue;
            }
            let at = f.pos(&self.world, fighter::NECK);
            let by = Some(f.flung_by);
            self.fighters[i].flying = 0;
            self.events.push(Sfx::Thud.at(at).vol((lost / 10.0).min(1.6)));
            self.shake_camera(i, 0.4);
            if self.fighters[i].alive {
                let dmg = (lost - SLAM_SPEED) * 2.2 + 5.0;
                self.damage(i, S_TORSO, dmg, at, (v1 - v0).normalize_or(Vec2::Y), 0.0, Kind::Blunt, by);
            }
        }
    }

    /// A long fall ending on something solid: whatever hit first was
    /// stopped from falling fast, and the legs can't take it, so they go
    /// down in a heap (thrown fighters have their own slam).
    fn hard_landings(&mut self) {
        for i in 0..self.fighters.len() {
            let f = &self.fighters[i];
            if !f.alive || f.down > 0 || f.flying > 0 || f.impaled > 0 || f.held_by.is_some() {
                continue;
            }
            let stopped = self.landing_speed(i);
            if stopped < FALL_DOWN_SPEED {
                continue;
            }
            let at = f.pos(&self.world, PELVIS);
            // (coming down really hard, now and then a leg goes)
            if stopped >= FALL_DOWN_SPEED + BREAK_FALL_OVER && self.rng.f32() < BREAK_CHANCE {
                let leg = [fighter::S_SHIN_A, fighter::S_SHIN_B, fighter::S_THIGH_A, fighter::S_THIGH_B][(self.rng.next_u32() % 4) as usize];
                let at = self.fighters[i].pos(&self.world, fighter::BONES[leg as usize].1);
                self.break_bone(i, leg, at);
            }
            let frames = FALL_DOWN_FRAMES + ((stopped - FALL_DOWN_SPEED) * FALL_DOWN_MORE) as u16;
            self.fighters[i].knock_down(frames.min(FALL_DOWN_MAX));
            self.events.push(Sfx::Thud.at(at).vol((stopped / 15.0).min(1.6)));
            self.shake_camera(i, 0.3);
        }
    }

    /// How hard fighter `i` landed this frame: the most any part of them
    /// touching the ground was stopped from falling (m/s).
    fn landing_speed(&self, i: usize) -> f32 {
        self.fighters[i]
            .particle_ids()
            .filter(|&id| self.world.particles[id as usize].on_ground())
            // (only falling speed taken off: a part shoved up off the floor
            // hasn't landed any harder for it)
            .map(|id| {
                let fall = self.fall.get(id as usize).copied().unwrap_or(0.0);
                fall.min(fall + self.world.particles[id as usize].vel.y)
            })
            .fold(0.0f32, f32::max)
    }

    /// Sweeps don't need a perfect capsule hit: the sweeping foot or shin
    /// passing near someone's feet or shins while it's swinging takes them down.
    fn sweep_check(&mut self) {
        for a in 0..self.fighters.len() {
            let f = &self.fighters[a];
            if !(f.alive && f.kick_kind == fighter::KICK_LOW && f.kicking()) || self.punch_cd[a] > 0 {
                continue;
            }
            let (foot, knee) = (f.pos(&self.world, fighter::FOOT_A), f.pos(&self.world, fighter::KNEE_A));
            let speed = self.world.particles[f.p(fighter::FOOT_A) as usize].vel.length();
            for v in 0..self.fighters.len() {
                let o = &self.fighters[v];
                if v == a || !o.alive || o.down > 0 {
                    continue;
                }
                let hit = [(fighter::KNEE_A, fighter::FOOT_A, fighter::S_SHIN_A), (fighter::KNEE_B, fighter::FOOT_B, fighter::S_SHIN_B)].into_iter().find_map(|(k, ft, part)| {
                    if !o.has(part) {
                        return None;
                    }
                    let (k, ft) = (o.pos(&self.world, k), o.pos(&self.world, ft));
                    // closest approach of our shin (knee..foot) to theirs
                    let d = seg_dist(knee, foot, k, ft);
                    (d < SWEEP_REACH).then_some((part, (foot + ft) * 0.5))
                });
                if let Some((part, at)) = hit {
                    self.kick_hit(a, v, part, at, speed.max(4.0));
                    break;
                }
            }
        }
    }

    /// A kick connecting: a low one takes their legs out, a dropkick sends
    /// them flying.
    fn kick_hit(&mut self, att: usize, vic: usize, vpart: u32, point: Vec2, impact: f32) {
        let f = &self.fighters[att];
        // (a sweep goes the way the legs face: see `Fighter::leg_facing`)
        let fwd = Vec2::new(f.leg_facing, 0.0);
        let drop = f.kick_kind == fighter::KICK_DROP;
        self.punch_cd[att] = f.kick;
        if drop {
            let speed = f.body_vel(&self.world).length();
            let dmg = 8.0 + impact * 0.5 + speed * 0.5;
            self.events.push(Sfx::HitHeavy.at(point).vol(1.4));
            self.damage(vic, vpart, dmg, point, (fwd + Vec2::new(0.0, 0.25)).normalize(), 26.0 + speed, Kind::Blunt, Some(att as u8));
            self.fighters[vic].flying = 30;
            self.fighters[vic].flung_by = att as u8;
            // and you bounce off them
            let me = self.fighters[att].particle_ids();
            self.world.add_velocity(me, -fwd * 2.5 + Vec2::new(0.0, 1.5));
            return;
        }
        let legs = matches!(vpart, fighter::S_THIGH_A | fighter::S_SHIN_A | fighter::S_THIGH_B | fighter::S_SHIN_B);
        self.events.push(Sfx::Punch.at(point).vol(1.1));
        self.damage(vic, vpart, 3.0 + impact * 0.4, point, fwd, impact * 1.2, Kind::Blunt, Some(att as u8));
        if legs && self.fighters[vic].alive && self.fighters[vic].grounded {
            // swept: feet out from under them, down they go
            let v = &self.fighters[vic];
            let feet = [fighter::KNEE_A, fighter::FOOT_A, fighter::KNEE_B, fighter::FOOT_B].map(|s| v.p(s));
            let top = [fighter::HEAD, fighter::NECK].map(|s| v.p(s));
            self.world.add_velocity(feet, fwd * 5.0 + Vec2::new(0.0, 2.5));
            self.world.add_velocity(top, -fwd * 2.0);
            self.fighters[vic].knock_down(45);
        }
    }

    fn upkeep_weapons(&mut self) {
        for wi in 0..self.weapons.len() {
            if !self.weapons[wi].active {
                continue;
            }
            let w = &mut self.weapons[wi];
            for cd in &mut w.hit_cd {
                *cd = cd.saturating_sub(1);
            }
            w.thrown = w.thrown.saturating_sub(1);
            w.fire_cd = w.fire_cd.saturating_sub(1);
            if w.holder.is_none() && w.thrown == 0 && !w.embedded {
                // nothing loose on the floor should ever be moving this fast
                for i in 0..w.n {
                    let p = &mut self.world.particles[w.particle(i) as usize];
                    let v = p.vel.length();
                    if v > LOOSE_MAX_SPEED {
                        p.vel *= LOOSE_MAX_SPEED / v;
                    }
                }
            }
            w.flash = w.flash.saturating_sub(1);
            if w.victim.is_none() && w.pass_grace > 0 {
                w.pass_grace -= 1;
                if w.pass_grace == 0 {
                    let w = self.weapons[wi].clone();
                    w.set_pass_through(&mut self.world, 0);
                }
            }
            let w = &mut self.weapons[wi];
            if let Some(v) = w.victim {
                w.skewer_t += 1;
                let (t, u) = (w.skewer_t, self.world.skewers[w.skewer as usize].u);
                // slippery while gliding in, and while the holder swings to fling them off
                let flinging = w.holder.is_some_and(|h| self.fighters[h as usize].striking());
                // (pinned to a wall through the chest, they stay right where they are)
                self.world.skewers[w.skewer as usize].anchored = w.pinned && w.embedded;
                self.world.skewers[w.skewer as usize].friction = if w.pinned {
                    1.0
                } else if t < GLIDE_FRAMES || flinging {
                    0.02
                } else {
                    SKEWER_GRIP
                };
                if flinging && t >= GLIDE_FRAMES {
                    // the swing throws them out along the blade, toward the tip
                    let sk = self.world.skewers[w.skewer as usize];
                    let axis = (self.world.particles[sk.tip as usize].pos - self.world.particles[sk.base as usize].pos).normalize_or_zero();
                    let ids = self.fighters[v as usize].particle_ids();
                    self.world.add_velocity(ids, axis * 1.3);
                }
                // slid off the tip (flung, or the blade pointed down), or time's up
                // (pinned to the floor by a planted spear, they stay up as a trophy)
                let planted = self.weapons[wi].embedded;
                if (u > 1.02 && t > GLIDE_FRAMES) || (t > SKEWER_FRAMES && !planted) {
                    self.unskewer(wi);
                } else {
                    let f = &mut self.fighters[v as usize];
                    f.down = f.down.max(8); // limp while run through
                    if t % 7 == 0 {
                        let sk = self.world.skewers[self.weapons[wi].skewer as usize];
                        let at = self.world.particles[sk.a as usize].pos.lerp(self.world.particles[sk.b as usize].pos, sk.s);
                        self.spawn_blood(at, Vec2::NEG_Y, 2, 1.5);
                    }
                }
            }
            self.update_embed(wi);
            let w = &mut self.weapons[wi];
            if w.grace > 0 {
                w.grace -= 1;
                if w.grace == 0 && w.holder.is_none() {
                    let w = self.weapons[wi].clone();
                    w.set_group(&mut self.world, 0);
                }
            }
            let w = &self.weapons[wi];
            let p = self.world.particles[w.base as usize].pos;
            if p.y < self.kill_y - 3.0 || p.x < self.min.x - 8.0 || p.x > self.max.x + 8.0 {
                if let Some(h) = w.holder {
                    self.release(h as usize, false);
                }
                let mut w = self.weapons[wi].clone();
                w.park(&mut self.world);
                self.weapons[wi] = w;
            }
        }
    }

    fn update_fx(&mut self) {
        for i in 0..self.sparks.len() {
            let mut s = self.sparks[i];
            let dt = DT * self.time_scale();
            // blood landing in liquid mixes into it (water takes the colour;
            // acid and lava just swallow it)
            if s.blood && !self.fluid.drops.is_empty() && self.fluid.sample(s.pos).total() > 0.3 {
                self.fluid.dye(s.pos, crate::liquid::BLOOD_DYE, crate::liquid::WATER_DROP);
                s.life = 0;
                self.sparks[i] = s;
                continue;
            }
            if s.steam {
                // rising, slowing, spreading
                s.vel.y += 2.0 * dt;
                s.vel *= 0.95;
            } else {
                s.vel.y -= 22.0 * dt;
                s.vel *= 0.99;
            }
            let step = s.vel * dt;
            if s.blood {
                if let Some(hit) = self.world.raycast(s.pos, step, 1.0) {
                    let r = 0.04 + (s.life % 5) as f32 * 0.018;
                    self.decals.push(Decal { pos: hit.point, r });
                    s.life = 0;
                }
            }
            s.pos += step;
            s.life = s.life.saturating_sub(1);
            self.sparks[i] = s;
        }
        self.sparks.retain(|s| s.life > 0);
        if self.decals.len() > 220 {
            let cut = self.decals.len() - 220;
            self.decals.drain(..cut);
        }
    }

    pub fn level_bounds(&self) -> (Vec2, Vec2) {
        (self.min, self.max)
    }
}

impl GameState {
    /// One frame of the game (`step` also keeps the voice log).
    fn simulate(&mut self, inputs: &[u32]) {
        self.frame += 1;
        self.round_frame += 1;
        self.hits.clear();
        self.events.clear();
        // strung up: a choking sound now and then (each plays if they've
        // finished the last)
        for (i, f) in self.fighters.iter().enumerate() {
            if f.alive && f.choking && (self.frame + i as u32 * 7) % CHOKE_EVERY == 0 {
                self.events.push(Sfx::Choke.at(f.pos(&self.world, fighter::HEAD)).line(0, i as u8));
            }
        }
        let fighting = matches!(self.phase, Phase::Fight);

        // the computer's fighters decide from this frame as it stands (the
        // same on every player's copy)
        let ai: Vec<u32> = (self.humans..self.fighters.len()).map(|i| crate::bot::bot_input(self, i)).collect();
        // a finisher can walk its performer: backing off before a spear throw
        let walk: Vec<u16> = (0..self.fighters.len()).map(|i| self.finisher_walk(i)).collect();
        for i in 0..self.fighters.len() {
            let given = if i >= self.humans { ai[i - self.humans] } else { inputs.get(i).copied().unwrap_or(0) };
            let raw = if fighting { given } else { 0 };
            let (b, aim) = input::decode(raw);
            let locked = self.clash.is_some_and(|c| c.side_of(i).is_some()) || self.in_finisher(i);
            let victim = self.finishers.iter().any(|x| x.victim as usize == i);
            let f = &mut self.fighters[i];
            f.prev_raw = f.raw_buttons;
            f.raw_buttons = b;
            f.prev_buttons = f.buttons;
            // in a blade lock the keys belong to the duel, not the body
            f.buttons = if locked { walk[i] } else { b };
            // (being finished, your mouse doesn't turn you round either: you
            // stay as you were caught)
            if !victim {
                f.aim = aim;
            }
            self.punch_cd[i] = self.punch_cd[i].saturating_sub(1);
        }
        self.update_holds();
        self.update_clash();
        self.update_finishers();
        for i in 0..self.fighters.len() {
            self.actions(i);
        }
        self.update_nooses();
        // tell the locked pair where to hold their blades
        let lock = self.clash.map(|c| (c, c.lock_point(self)));
        for (i, f) in self.fighters.iter_mut().enumerate() {
            f.locked = lock.and_then(|(c, p)| c.side_of(i).map(|_| p));
        }
        let scale = self.time_scale();
        let scripts: Vec<Option<(HandScript, f32)>> = (0..self.fighters.len())
            .map(|i| self.finishers.iter().find(|x| x.attacker as usize == i).map(|x| self.finisher_script(x)))
            .collect();
        for i in 0..self.fighters.len() {
            let mut f = self.fighters[i].clone();
            f.script = scripts[i].map(|(sc, _)| sc);
            if let Some((_, facing)) = scripts[i] {
                f.facing = facing;
            }
            let (mut load, mut two_handed, mut thrust) = (0.0, false, false);
            f.skewering = f.weapon.is_some_and(|wi| self.weapons[wi as usize].victim.is_some());
            // anyone to dive at? close, alive, and roughly where you're aiming
            let neck = f.pos(&self.world, fighter::NECK);
            let aim = f.aim_dir();
            f.prey = self.fighters.iter().any(|o| {
                let to = o.pos(&self.world, fighter::HEAD) - neck;
                o.id != f.id && o.alive && to.length() < DIVE_RANGE && to.normalize_or_zero().dot(aim) > 0.5
            });
            f.hang_blade = self.hanging_from(i).is_some();
            f.stuck_grip = f.weapon.is_some_and(|wi| self.weapons[wi as usize].embedded);
            if let Some(wi) = f.weapon {
                let w = &self.weapons[wi as usize];
                load = w.mass(&self.world) * 0.6;
                // (a body on a blade stuck in a wall hangs off the wall, not your arm)
                if let (Some(v), false) = (w.victim, w.embedded) {
                    // a body on the blade is heavy
                    load += self.fighters[v as usize].particle_ids().map(|i| self.world.particles[i as usize].mass()).sum::<f32>() * 0.35;
                }
                two_handed = self.world.distances[f.grip_b as usize].enabled;
                thrust = w.def().thrust;
            }
            let gun = f.weapon.is_some_and(|wi| self.weapons[wi as usize].is_gun() || self.weapons[wi as usize].is_grapple());
            let (step_before, air_before, slid) = (dmath::sin(f.phase), f.air_t, f.slide > 0);
            f.control(&mut self.world, DT * scale, load, two_handed, thrust, gun);
            if f.alive && f.slide > 0 && !slid {
                self.events.push(Sfx::Slide.at(f.pos(&self.world, PELVIS)));
            }
            // sounds: the strike of an attack, footfalls, hard landings
            if f.alive && f.swing > 0 && f.swing == f.strike_frame() {
                let at = f.pos(&self.world, HAND_A);
                let kind = if f.swing_thrust {
                    Sfx::Stab
                } else if load > 2.0 {
                    Sfx::SwingHeavy
                } else {
                    Sfx::Swing
                };
                self.events.push(kind.at(at));
            }
            if f.alive && f.grounded {
                let step_now = dmath::sin(f.phase);
                if step_before * step_now < 0.0 && self.world.particles[f.p(PELVIS) as usize].vel.x.abs() > 1.0 {
                    let foot = if step_now > 0.0 { fighter::FOOT_A } else { fighter::FOOT_B };
                    self.events.push(Sfx::Footstep.at(f.pos(&self.world, foot)));
                }
                if air_before > 18 {
                    let at = f.pos(&self.world, fighter::FOOT_A);
                    self.events.push(Sfx::Land.at(at).vol((air_before as f32 / 30.0).min(1.5)));
                }
            }
            if let Some(wi) = f.weapon {
                // two-handers: if the weapon has swung badly out of line with
                // where you're aiming, the rear hand lets go so it can't hold it
                // backwards, and takes it again once it's back in line
                let w = &self.weapons[wi as usize];
                if let (Some(g2), true) = (w.def().grip2, f.arm_b_ok() && !f.grabbing && !w.embedded) {
                    let (a0, a1) = w.def().aim;
                    let bone = self.world.particles[w.particle(a1) as usize].pos - self.world.particles[w.particle(a0) as usize].pos;
                    let off = dmath::angle_diff(dmath::angle(bone), f.weapon_angle()).abs();
                    let slot = &mut self.world.distances[f.grip_b as usize];
                    if slot.enabled && off > 1.4 {
                        slot.enabled = false;
                    } else if !slot.enabled && off < 0.5 {
                        *slot = Distance { a: f.p(HAND_B), b: w.particle(g2), rest: 0.0, compliance: 2e-4, enabled: true };
                    }
                }
                // the weapon points where the hand is going, turning at a capped rate
                let w = &self.weapons[wi as usize];
                let strength = f.strength();
                let striking = f.striking();
                let target = if w.is_gun() && f.script.is_none() && f.locked.is_none() { self.gun_angle(&f, w) } else { f.weapon_angle() };
                let a = &mut self.world.aims[w.aim as usize];
                // (a blade you stuck in the wall yourself is a lever to heave
                // yourself up on; one you grabbed while it was stuck isn't turned)
                a.enabled = f.alive && f.arm_a_ok() && strength > 0.05 && (!w.embedded || w.lever);
                a.target = target;
                a.compliance = if striking { 1e-6 } else { 4e-6 } / strength.max(0.1);
                a.damping = 1.0;
                a.max_step = if striking { 0.08 } else { 0.06 };
                if f.recoil > 0 {
                    // the kick wins for a moment, then the wrist brings it back
                    let r = f.recoil as f32 / fighter::RECOIL_FRAMES as f32;
                    a.enabled &= r < 0.5;
                    a.compliance *= 1.0 + 40.0 * r;
                    a.max_step *= 1.0 - 0.85 * r;
                }
            }
            self.fighters[i] = f;
        }

        for w in &mut self.weapons {
            if let (true, Some((_, base, tip))) = (w.active, w.def().impale) {
                let (pb, pt) = (self.world.particles[w.particle(base) as usize], self.world.particles[w.particle(tip) as usize]);
                w.tip_speed = pt.vel.dot((pt.pos - pb.pos).normalize_or_zero());
            }
        }
        self.update_flips();
        self.update_shoulder_throws();
        for f in &mut self.fighters {
            if f.ghost > 0 {
                f.ghost -= 1;
                if f.ghost == 0 {
                    for k in 0..fighter::BONES.len() as u32 {
                        self.world.segments[(f.seg_base + k) as usize].ignore_group = 0;
                    }
                }
            }
        }
        self.drive_balls();
        let flying: Vec<Option<Vec2>> = self.fighters.iter().map(|f| (f.flying > 0).then(|| f.body_vel(&self.world))).collect();
        // joint limits face the way each fighter faces this frame
        for f in &self.fighters {
            f.update_joints(&mut self.world, self.joint_limits);
        }
        self.keep_upright();
        self.update_trap();
        self.float_rafts(DT * scale);
        self.update_liquids(DT * scale);
        self.fall = self.world.particles.iter().map(|p| -p.vel.y).collect();
        self.world.step(DT * scale);
        self.hard_landings();
        // the liquid flows, and is shoved about by (and shoves) everything in
        // it: how hard it shoved each fighter (sideways and down: up is it
        // holding them up), as a share of their weight, eased over SURGE_EASE
        // (not the water holding back their own going: that's only drag)
        let before: Vec<(Vec2, Vec2)> = self.fighters.iter().map(|f| (self.momentum(f), f.body_vel(&self.world))).collect();
        let vels: Vec<Vec<Vec2>> = self.fighters.iter().map(|f| f.particle_ids().map(|i| self.world.particles[i as usize].vel).collect()).collect();
        self.fluid.step(DT * scale, &mut self.world);
        // on their feet, they take it as a whole (the water dragging at their
        // legs alone doesn't tip them over: a shove too hard for them knocks
        // them down, below); down, it tumbles them as it likes
        for (fi, v0) in vels.iter().enumerate() {
            let f = &self.fighters[fi];
            if !f.alive || f.down > 0 {
                continue;
            }
            let ids: Vec<usize> = f.particle_ids().map(|i| i as usize).collect();
            let (mut dp, mut m) = (Vec2::ZERO, 0.0);
            for (k, &i) in ids.iter().enumerate() {
                let p = &self.world.particles[i];
                dp += (p.vel - v0[k]) * p.mass();
                m += p.mass();
            }
            let dv = dp / m.max(1e-6);
            for (k, &i) in ids.iter().enumerate() {
                if self.world.particles[i].inv_mass > 0.0 {
                    self.world.particles[i].vel = v0[k] + dv;
                }
            }
        }
        let g = -self.world.cfg.gravity.y;
        for (fi, (p0, going)) in before.into_iter().enumerate() {
            let f = &self.fighters[fi];
            let mut push = (self.momentum(f) - p0) / (DT * scale);
            let way = going.normalize_or_zero();
            push -= way * push.dot(way).min(0.0);
            let weight = f.particle_ids().map(|i| self.world.particles[i as usize].mass()).sum::<f32>() * g;
            let shove = Vec2::new(push.x, push.y.min(0.0)).length() / weight.max(1e-3);
            let k = (DT * scale / SURGE_EASE).min(1.0);
            let f = &mut self.fighters[fi];
            f.surge += (shove - f.surge) * k;
            // too much of it, on their feet: swept off them (not as they
            // find their feet coming out of a swim: the shove's averaged
            // over SURGE_EASE, and some of it is still the swim's)
            f.dry = if f.swimming { 0 } else { f.dry.saturating_add(1) };
            let settled = f.dry as f32 * DT * scale >= SURGE_EASE;
            if f.surge > SURGE_KNOCK && settled && f.alive && f.grounded && f.down == 0 && f.impaled == 0 {
                f.knock_down(SURGE_DOWN);
                let at = f.pos(&self.world, PELVIS);
                self.events.push(Sfx::Splash.at(at).vol(0.6));
            }
        }
        self.fluid.keep_within(Vec2::new(self.min.x - 4.0, self.kill_y), Vec2::new(self.max.x + 4.0, self.max.y + 20.0));
        self.water_on_lava();
        self.update_steam(DT * scale);
        self.eat_and_burn(DT * scale);
        self.carry_stuck();
        self.resolve_hits();
        self.slam_thrown(&flying);
        self.sweep_check();
        self.limbs_to_weapons();
        self.land_javelins();
        self.cut_nooses();
        self.update_bullets();
        self.update_hazards();
        self.update_impaled();
        self.upkeep_weapons();

        for i in 0..self.fighters.len() {
            let f = &self.fighters[i];
            let p = f.pos(&self.world, PELVIS);
            if f.alive && (p.y < self.kill_y || p.x < self.min.x - 4.0 || p.x > self.max.x + 4.0) {
                self.kill_fighter(i);
                self.shake_camera(i, 0.4);
            }
            let f = &mut self.fighters[i];
            if f.bleed > 0 {
                f.bleed -= 1;
                if f.bleed % 3 == 0 {
                    let at = self.world.particles[f.bleed_at as usize].pos;
                    let k = f.bleed as f32 / 200.0;
                    self.spawn_blood(at, Vec2::Y, 1, 2.0 + 3.0 * k);
                }
            }
        }
        self.bleed();
        self.update_fx();

        // weapon drops
        if fighting && !self.training && self.frame >= self.next_drop {
            self.next_drop = self.frame + DROP_INTERVAL;
            let lv = levels().swap_remove(self.level_index);
            // (only where weapons lie about to start with)
            if lv.arms == crate::level::Arms::Scattered && self.weapons.iter().filter(|w| w.active).count() < MAX_WEAPONS {
                let n = lv.weapon_spawns.len();
                let start = (self.rng.next_u32() as usize) % n;
                let def = DROPS[self.rng.next_u32() as usize % DROPS.len()];
                // the first spawn point with nothing already lying on it
                let free = (0..n).map(|i| lv.weapon_spawns[(start + i) % n]).find(|&p| {
                    !self.weapons.iter().any(|w| w.active && w.distance_to(&self.world, p) < 1.4)
                });
                if let Some(p) = free {
                    let pos = p + Vec2::new(0.0, 0.6);
                    self.spawn_loose_weapon(def, pos);
                    self.spawn_sparks(pos, Vec2::Y, 10, 4.0);
                }
            }
        }
        for k in &mut self.shake {
            *k *= 0.86;
        }
        self.slowmo = self.slowmo.saturating_sub(1);
        self.flash = self.flash.saturating_sub(1);

        match self.phase {
            Phase::Fight => {
                let alive = self.alive_count();
                let over = if self.training {
                    // practice: over when you die or every dummy is down for good
                    !self.fighters[0].alive || self.fighters[1..].iter().all(|f| !f.alive)
                } else if self.teams > 0 {
                    self.teams_alive() <= 1
                } else if self.num_players >= 2 {
                    alive <= 1
                } else {
                    alive == 0
                };
                if over && self.round_frame > 10 {
                    let winner = if self.num_players >= 2 {
                        self.fighters.iter().find(|f| f.alive).map(|f| f.id)
                    } else {
                        None
                    };
                    // (a team's win counts for everyone on it)
                    for i in 0..self.fighters.len() {
                        if winner.is_some_and(|w| self.allies(w as usize, i)) {
                            self.scores[i] += 1;
                        }
                    }
                    self.phase = Phase::RoundOver { timer: ROUND_OVER_FRAMES, winner };
                }
            }
            Phase::RoundOver { timer, winner } => {
                if timer <= 1 {
                    if !self.training {
                        self.level_index = crate::level::next_arena(self.level_index, self.fighters.len());
                    }
                    self.reset_round();
                } else {
                    self.phase = Phase::RoundOver { timer: timer - 1, winner };
                }
            }
        }
    }
}

impl RollbackGame for GameState {
    type Input = u32;

    fn step(&mut self, inputs: &[u32]) {
        self.simulate(inputs);
        // this frame's voice lines go in the log, tagged with this frame
        let said = self.frame;
        self.voice_log.extend(self.events.iter().filter(|e| e.kind.is_voice()).map(|e| (said, *e)));
        self.voice_log.retain(|(f, _)| f + VOICE_LOG_FRAMES > said);
        self.sfx_log.extend(self.events.iter().filter(|e| !e.kind.is_voice()).map(|e| (said, *e)));
        self.sfx_log.retain(|(f, _)| f + SFX_LOG_FRAMES > said);
    }

    fn checksum(&self) -> u64 {
        let mut h = Fnv64::default();
        h.write_u32(self.frame);
        h.write_u32(self.round_frame);
        h.write_u32(self.level_index as u32);
        h.write_u32(self.humans as u32);
        h.write_u32(self.teams as u32);
        h.write_u32(self.rng.0);
        h.write_u32(self.next_drop);
        for r in &self.rafts {
            for x in [r.y, r.vy, r.tilt, r.spin] {
                h.write_f32(x);
            }
        }
        self.world.hash_into(|v| h.write_u32(v));
        for a in &self.world.aims {
            h.write_f32(a.target);
            h.write_f32(a.compliance);
            h.write_u8(a.enabled as u8);
        }
        for d in &self.world.distances {
            h.write_u32(d.a);
            h.write_u32(d.b);
            h.write_u8(d.enabled as u8);
        }
        for k in &self.world.skewers {
            h.write_u32(k.a);
            h.write_f32(k.s);
            h.write_f32(k.u);
            h.write_u8(k.enabled as u8);
        }
        for s in &self.world.segments {
            h.write_u32(s.a);
            h.write_u32(s.ignore_group as u32);
            h.write_u32(s.group as u32);
            h.write_u8(s.enabled as u8);
        }
        for f in &self.fighters {
            h.write_f32(f.health);
            h.write_f32(f.facing);
            h.write_f32(f.leg_facing);
            h.write_f32(f.phase);
            h.write_f32(f.hand_ang);
            h.write_f32(f.hand_reach);
            h.write_u32(f.swing as u32 | (f.wall_jumps as u32) << 8 | (f.slide as u32) << 16 | (f.stagger as u32) << 24);
            h.write_u8(f.air_jumped as u8);
            h.write_u8(f.recoil);
            h.write_u8(f.hurt);
            h.write_u32(f.severed as u32);
            h.write_u32(f.down as u32 | (f.getup as u32) << 16);
            h.write_u32(f.bleed as u32);
            h.write_u32(f.weapon.map_or(255, |w| w as u32));
            for b in [f.alive as u8, f.grounded as u8, f.coyote, f.jump_cd, f.grabbing as u8] {
                h.write_u8(b);
            }
            h.write_u32(f.buttons as u32 | (f.prev_buttons as u32) << 16);
            h.write_u32(f.swing_len as u32 | (f.punch_b as u32) << 8 | (f.kick as u32) << 16 | (f.kick_kind as u32) << 24);
            h.write_u32(f.hold.map_or(255, |v| v as u32) | (f.hold_slot as u32) << 8 | (f.struggle as u32) << 16 | (f.heave as u32) << 24);
            h.write_u32(f.limbs_gone as u32 | (f.flip as u32) << 16 | (f.flip_dir as u8 as u32) << 24);
            h.write_u8(f.shoulder);
            h.write_u8(f.shouldered);
            h.write_u8(f.shouldered_by);
            h.write_f32(f.head_squash);
            h.write_u8(f.head_popped as u8);
            h.write_u32(f.impaled as u32 | (f.impale_part as u32) << 16 | (f.impale_free as u32) << 24);
            h.write_u8(f.impale_grace);
            h.write_u8(f.impale_beat);
            h.write_u32(f.breath as u32 | (f.stroke as u32) << 16 | (f.swimming as u32) << 24 | (f.wet as u32) << 25);
            h.write_f32(f.scorch);
            h.write_u32(f.broken as u32);
            for k in 0..10 {
                h.write_u32(f.eaten[k] as u32 | (f.melted[k] as u32) << 16);
                h.write_u32(f.fire[k] as u32);
            }
            h.write_u32(f.burning as u32);
            h.write_f32(f.surge);
            h.write_u32(f.dry as u32);
            h.write_f32(f.wade);
            h.write_f32(f.swim_phase);
            h.write_f32(f.slam);
            h.write_u8(f.shoulder_dir as u8);
            h.write_u8(f.dove as u8 | (f.prey as u8) << 1 | (f.hang_blade as u8) << 2 | (f.choking as u8) << 3 | (f.dove_shift as u8) << 4 | (f.flip_whip as u8) << 5);
            h.write_u8(f.ghost);
            h.write_f32(f.flip_turn);
            for p in f.flip_path {
                h.write_f32(p.x);
                h.write_f32(p.y);
            }
            h.write_u32(f.grab_cd as u32 | (f.flying as u32) << 8 | (f.flung_by as u32) << 16 | (f.hold_t as u32 & 0xff) << 24);
            h.write_u32(f.raw_buttons as u32 | (f.prev_raw as u32) << 16);
        }
        for w in &self.weapons {
            h.write_u32(w.holder.map_or(255, |x| x as u32));
            h.write_u8(w.thrown);
            h.write_u8(w.grace);
            h.write_u8(w.active as u8);
            h.write_f32(w.tip_speed);
            h.write_u32(w.embedded as u32 | (w.pinned as u32) << 1 | (w.flip as u32) << 2 | (w.yank as u32) << 8 | (w.ammo as u32) << 16 | (w.fire_cd as u32) << 24);
            h.write_u32(w.victim.map_or(255, |v| v as u32) | (w.skewer_t as u32) << 8 | (w.pass_grace as u32) << 24);
            h.write_u32(w.stuck.map_or(0xffff, |(f, p)| f as u32 | (p as u32) << 8));
        }
        for b in &self.bullets {
            h.write_f32(b.pos.x);
            h.write_f32(b.pos.y);
            h.write_f32(b.vel.x);
            h.write_f32(b.vel.y);
            h.write_u32(b.owner as u32 | (b.life as u32) << 8);
        }
        self.fluid.hash_into(|v| h.write_u32(v));
        h.write_u32(self.trap.open as u32);
        h.write_u32(self.trap.toggled);
        h.write_u32(self.trap.poured);
        self.steam.hash_into(|v| h.write_u32(v));
        h.write_u32(self.burst_cd);
        for s in &self.sparks {
            h.write_f32(s.pos.x);
            h.write_f32(s.pos.y);
            h.write_u8(s.life);
        }
        for d in &self.decals {
            h.write_f32(d.pos.x);
            h.write_f32(d.pos.y);
        }
        for cd in &self.hazard_cd {
            h.write_u8(*cd);
        }
        h.write_u32(self.clash_cd as u32);
        h.write_u32(self.slowmo as u32 | (self.flash as u32) << 16);
        for f in &self.fighters {
            h.write_u32(f.dazed as u32);
        }
        for x in &self.finishers {
            h.write_u32(x.attacker as u32 | (x.victim as u32) << 8 | (x.mv as u32) << 16 | (x.killed as u32) << 24);
            h.write_u8(x.side as u8);
            h.write_u32(x.t as u32);
            if let Some(wall) = x.wall {
                h.write_f32(wall.x);
                h.write_f32(wall.y);
            }
        }
        for n in &self.nooses {
            h.write_u32(n.weapon as u32 | (n.victim as u32) << 8 | (n.attacker as u32) << 16 | (n.cut as u32) << 24 | (n.toss as u32) << 25);
            h.write_u32(n.tether);
            h.write_f32(n.len);
            h.write_f32(n.hang);
            h.write_u32(n.hook as u32 | (n.hook_t as u32) << 8);
            h.write_u32(n.anchor);
            h.write_f32(n.hook_vel.x);
            h.write_f32(n.hook_vel.y);
        }
        if let Some(c) = self.clash {
            h.write_u32(c.fighters[0] as u32 | (c.fighters[1] as u32) << 8 | (c.score[0] as u32) << 16 | (c.score[1] as u32) << 24);
            h.write_u32(c.prompt as u32 | (c.stage as u32) << 8 | (c.lockout[0] as u32) << 16 | (c.lockout[1] as u32) << 24);
            h.write_u32(c.t as u32 | (c.total as u32) << 16);
            h.write_f32(c.point.x);
            h.write_f32(c.point.y);
        }
        for (sc, cd) in self.scores.iter().zip(&self.punch_cd) {
            h.write_u32(*sc as u32);
            h.write_u8(*cd);
        }
        match self.phase {
            Phase::Fight => h.write_u8(0),
            Phase::RoundOver { timer, winner } => {
                h.write_u8(1);
                h.write_u32(timer as u32);
                h.write_u8(winner.map_or(255, |w| w));
            }
        }
        for k in self.shake {
            h.write_f32(k);
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fighter::{HEAD, NECK};

    fn idle(s: &mut GameState, frames: u32) {
        let n = s.num_players;
        for _ in 0..frames {
            s.step(&vec![0; n]);
        }
    }

    #[test]
    fn idle_fighters_stand() {
        let mut s = GameState::new(2, 0, 1);
        idle(&mut s, 240);
        for f in &s.fighters {
            assert!(f.alive);
            let head = f.pos(&s.world, HEAD);
            let pelvis = f.pos(&s.world, PELVIS);
            assert!(head.y - pelvis.y > 0.7, "fighter {} slumped", f.id);
        }
    }

    #[test]
    fn heavy_hit_knocks_down_and_fighter_gets_up() {
        let mut s = GameState::new(2, 0, 1);
        idle(&mut s, 60);
        let p = s.fighters[0].pos(&s.world, NECK);
        s.damage(0, S_TORSO, 20.0, p, Vec2::X, 20.0, Kind::Blunt, None);
        assert!(s.fighters[0].down > 0);
        idle(&mut s, 200);
        let f = &s.fighters[0];
        assert!(f.alive && f.down == 0);
        let head = f.pos(&s.world, HEAD);
        let pelvis = f.pos(&s.world, PELVIS);
        assert!(head.y - pelvis.y > 0.6, "didn't get back up: head {head:?} pelvis {pelvis:?}");
    }

    #[test]
    fn pickup_swing_and_throw() {
        let mut s = GameState::new(2, 0, 3);
        idle(&mut s, 30);
        // drop a sword right at fighter 0's hand
        let hand = s.fighters[0].pos(&s.world, HAND_A);
        s.spawn_weapon(0, hand + Vec2::new(0.2, 0.3));
        let wi = s.weapons.len() - 1;
        idle(&mut s, 10);
        s.step(&[input::encode(PICKUP, Vec2::new(0.6, 0.0)), 0]);
        assert_eq!(s.fighters[0].weapon, Some(wi as u8));
        // swing in a circle with power held
        for t in 0..60 {
            let a = t as f32 * 0.3;
            s.step(&[input::encode(ATTACK, Vec2::new(a.cos(), a.sin()) * 0.6), 0]);
        }
        // (the circling can stick the tip in the girder overhead; this test
        // is about throwing, so pull it free)
        if s.weapons[wi].embedded {
            let mut w = s.weapons[wi].clone();
            w.unembed(&mut s.world, Vec2::ZERO);
            s.weapons[wi] = w;
        }
        // (and let the swing settle: a throw from mid-swing goes where the
        // swing was heading)
        idle(&mut s, 30);
        assert_eq!(s.fighters[0].weapon, Some(wi as u8));
        s.step(&[input::encode(THROW, Vec2::new(1.0, 0.3)), 0]);
        assert_eq!(s.fighters[0].weapon, None);
        let v = s.world.particles[s.weapons[wi].base as usize].vel;
        assert!(v.x > 5.0, "thrown weapon should fly: {v:?}");
    }

    #[test]
    fn sword_can_sever_and_kill() {
        let mut s = GameState::new(2, 0, 5);
        idle(&mut s, 30);
        let p = s.fighters[1].pos(&s.world, HEAD);
        // repeated big blade hits eventually sever something (65% chance each)
        for _ in 0..8 {
            if !s.fighters[1].alive {
                break;
            }
            s.damage(1, fighter::S_UPPER_B, 30.0, p, Vec2::X, 10.0, Kind::Blade, Some(0));
        }
        assert!(s.fighters[1].severed != 0 || !s.fighters[1].alive);
        idle(&mut s, 120);
        assert!(!s.decals.is_empty(), "blood should splat on the floor");
    }

    #[test]
    fn rollback_resimulation_is_deterministic() {
        let mut st = stickfell_net::SyncTestSession::new(GameState::new(4, 2, 9), 8);
        for _ in 0..900 {
            let inputs: Vec<u32> = (0..4).map(|p| crate::bot::bot_input(&st.game, p)).collect();
            st.advance(&inputs).unwrap();
        }
    }
}

#[cfg(test)]
mod feel;
