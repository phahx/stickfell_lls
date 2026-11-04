//! Stick fighter: an 11-particle active ragdoll.
//!
//! It is held up by weak, damped "muscles" and a soft hover spring, so it
//! wobbles, stumbles and falls over. The weapon hand is driven by an XPBD
//! drive whose target turns toward the mouse at a capped speed (clicks play a
//! wind-up and strike); its reaction pushes back on the torso, so heavy
//! weapons pull you around. Getting hit hard knocks you down (limp ragdoll)
//! until you wobble back up. No automatic climbing: walls can be jumped off
//! (twice before landing) and ledges grabbed with the free hand.

use core::f32::consts::FRAC_PI_2;

use stickfell_physics::{Particle, Vec2, World, dmath};

use crate::input::*;

// particle slots (offset from Fighter::base)
pub const HEAD: u32 = 0;
pub const NECK: u32 = 1;
pub const PELVIS: u32 = 2;
pub const ELBOW_A: u32 = 3;
pub const HAND_A: u32 = 4;
pub const ELBOW_B: u32 = 5;
pub const HAND_B: u32 = 6;
pub const KNEE_A: u32 = 7;
pub const FOOT_A: u32 = 8;
pub const KNEE_B: u32 = 9;
pub const FOOT_B: u32 = 10;
pub const PARTICLES: u32 = 11;

// body parts = segment slots (offset from Fighter::seg_base)
pub const S_HEAD: u32 = 0;
pub const S_TORSO: u32 = 1;
pub const S_UPPER_A: u32 = 2;
pub const S_FORE_A: u32 = 3;
pub const S_UPPER_B: u32 = 4;
pub const S_FORE_B: u32 = 5;
pub const S_THIGH_A: u32 = 6;
pub const S_SHIN_A: u32 = 7;
pub const S_THIGH_B: u32 = 8;
pub const S_SHIN_B: u32 = 9;

/// Bones as (from, to) particle slots, in part order.
pub const BONES: [(u32, u32); 10] = [
    (HEAD, HEAD),
    (NECK, PELVIS),
    (NECK, ELBOW_A),
    (ELBOW_A, HAND_A),
    (NECK, ELBOW_B),
    (ELBOW_B, HAND_B),
    (PELVIS, KNEE_A),
    (KNEE_A, FOOT_A),
    (PELVIS, KNEE_B),
    (KNEE_B, FOOT_B),
];

// muscle slots: same index as the part for limbs; torso/head swapped
const M_TORSO: u32 = 0;
const M_HEAD: u32 = 1;
const MUSCLES: u32 = 10;

pub const HEAD_R: f32 = 0.22;
pub const LIMB_R: f32 = 0.075;
pub const MAX_HEALTH: f32 = 100.0;
pub const ARM_REACH: f32 = 0.64;

/// What a fighter's limbs measure (metres): the character's build (see
/// `character::BUILD`). Poses that reach for or hold things at a distance
/// were tuned on [`Body::TUNED`] and scale with these, so they fit any build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// Shoulder to hand, straight (upper arm + forearm).
    pub arm: f32,
    /// Neck to pelvis.
    pub torso: f32,
    /// Neck to the middle of the head.
    pub neck: f32,
    /// Hip to foot, straight (thigh + shin).
    pub leg: f32,
}

impl Body {
    /// The build the poses were tuned on (the original stick figure).
    pub const TUNED: Body = Body { arm: 0.5968, torso: 0.6, neck: 0.3007, leg: 0.893 };

    pub fn of(b: &crate::character::Build) -> Body {
        Body { arm: b.upper_arm + b.forearm, torso: b.torso, neck: b.neck, leg: b.thigh + b.shin }
    }

    pub fn arm_k(&self) -> f32 {
        self.arm / Body::TUNED.arm
    }
    pub fn torso_k(&self) -> f32 {
        self.torso / Body::TUNED.torso
    }
    pub fn neck_k(&self) -> f32 {
        self.neck / Body::TUNED.neck
    }
    pub fn leg_k(&self) -> f32 {
        self.leg / Body::TUNED.leg
    }
}

/// Physics substep length (must match `WorldConfig::substeps`).
const SUB_H: f32 = 1.0 / 60.0 / 8.0;
/// Frames of a click attack: wind-up, strike, recover.
pub const SWING_FRAMES: u8 = 26;
/// Wall jumps allowed before landing again.
pub const WALL_JUMPS: u8 = 2;
const STAND_HEIGHT: f32 = 0.98;
/// Tallest step the legs walk up on their own.
const MAX_STEP: f32 = 0.2;
/// Pelvis height crouched: a deep, low squat (about half your height), not
/// a polite knee bend.
const CROUCH_HEIGHT: f32 = 0.26;
const SLIDE_HEIGHT: f32 = 0.32;
/// Crouched scuttling speed, as a share of running speed.
const CROUCH_SPEED: f32 = 0.75;
/// Crouching while running at least this fast starts a slide.
const SLIDE_MIN_SPEED: f32 = 4.0;
const SLIDE_FRAMES: u8 = 40;
/// How far past full arm's length a stab reaches for (the body follows).
const STAB_EXTRA: f32 = 0.5;
const STAB_LUNGE: f32 = 7.0;
const PUNCH_LUNGE: f32 = 4.5;
const KICK_LUNGE: f32 = 6.5;
const DROPKICK_SPEED: f32 = 8.0;
/// Diving for a grab in mid-air: a lunge along the aim at this speed.
const DIVE_SPEED: f32 = 8.0;
/// Where a guarding fist sits, from the neck (x is forward).
const GUARD: Vec2 = Vec2::new(0.3, 0.02);
const RUN_SPEED: f32 = 6.0;
const JUMP_SPEED: f32 = 12.5;
const WALL_JUMP_SPEED: f32 = 11.5;
/// The second jump, in mid-air (a little weaker than one off the ground).
const DOUBLE_JUMP_SPEED: f32 = 10.5;
/// Sideways kick away from the wall.
const WALL_JUMP_PUSH: f32 = 6.5;
/// Fastest fall while pressed against a wall.
const WALL_SLIDE: f32 = 3.5;
pub const GETUP_FRAMES: u16 = 50;
/// Frames of a bare-fisted punch (a quick jab next to a weapon swing).
pub const PUNCH_FRAMES: u8 = 16;
/// Frames of a low sweeping kick and of a flying dropkick.
pub const KICK_LOW_FRAMES: u8 = 22;
pub const KICK_DROP_FRAMES: u8 = 34;
pub const KICK_LOW: u8 = 1;
pub const KICK_DROP: u8 = 2;
/// Frames of winding up before a held body is thrown.
pub const HEAVE_FRAMES: u8 = 14;
/// Frames of the flip over someone grabbed by the head in mid-air (one full
/// turn).
pub const FLIP_FRAMES: u8 = 30;
/// Frames of a shoulder throw (Q with your back to whoever you're holding):
/// up over your shoulder and slammed down in front of you.
pub const SHOULDER_FRAMES: u8 = 28;

/// One joint limit: the three particles (pivot in the middle), the two body
/// parts it needs, the allowed bend away from straight in radians for a
/// fighter facing right (counterclockwise positive), and whether it mirrors
/// when they face left.
pub struct Joint {
    pub p: [u32; 3],
    pub parts: [u32; 2],
    pub range: (f32, f32),
    pub mirrors: bool,
}

/// Elbows fold the hand forward and up (about 150 degrees, no
/// hyperextension); knees fold the foot back (about 160, enough for the deep
/// squat); hips swing the thigh forward a long way and back a little; the
/// head tilts either way.
pub const JOINTS: [Joint; 7] = [
    Joint { p: [NECK, ELBOW_A, HAND_A], parts: [S_UPPER_A, S_FORE_A], range: (-0.05, 2.6), mirrors: true },
    Joint { p: [NECK, ELBOW_B, HAND_B], parts: [S_UPPER_B, S_FORE_B], range: (-0.05, 2.6), mirrors: true },
    // (knees give a touch past straight, so a foot catching an edge bends
    // instead of levering you up onto it)
    Joint { p: [PELVIS, KNEE_A, FOOT_A], parts: [S_THIGH_A, S_SHIN_A], range: (-2.8, 0.15), mirrors: true },
    Joint { p: [PELVIS, KNEE_B, FOOT_B], parts: [S_THIGH_B, S_SHIN_B], range: (-2.8, 0.15), mirrors: true },
    Joint { p: [NECK, PELVIS, KNEE_A], parts: [S_TORSO, S_THIGH_A], range: (-0.9, 2.5), mirrors: true },
    Joint { p: [NECK, PELVIS, KNEE_B], parts: [S_TORSO, S_THIGH_B], range: (-0.9, 2.5), mirrors: true },
    Joint { p: [PELVIS, NECK, HEAD], parts: [S_TORSO, S_HEAD], range: (-0.7, 0.7), mirrors: false },
];

pub const fn bit(part: u32) -> u16 {
    1 << part
}
pub const ARM_A_BITS: u16 = bit(S_UPPER_A) | bit(S_FORE_A);
pub const ARM_B_BITS: u16 = bit(S_UPPER_B) | bit(S_FORE_B);
pub const LEG_A_BITS: u16 = bit(S_THIGH_A) | bit(S_SHIN_A);
pub const LEG_B_BITS: u16 = bit(S_THIGH_B) | bit(S_SHIN_B);

/// Segment tag = (fighter id + 1) * 16 + part.
pub fn seg_tag(id: u8, part: u32) -> u32 {
    (id as u32 + 1) * 16 + part
}

pub fn decode_tag(tag: u32) -> Option<(usize, u32)> {
    (16..crate::weapon::WEAPON_TAG).contains(&tag).then(|| ((tag / 16 - 1) as usize, tag % 16))
}

/// For a severable part: the joint particle slot where it comes off, and the
/// parts that come off with it.
pub fn sever_plan(part: u32) -> Option<(u32, &'static [u32])> {
    match part {
        S_UPPER_A => Some((NECK, &[S_UPPER_A, S_FORE_A])),
        S_FORE_A => Some((ELBOW_A, &[S_FORE_A])),
        S_UPPER_B => Some((NECK, &[S_UPPER_B, S_FORE_B])),
        S_FORE_B => Some((ELBOW_B, &[S_FORE_B])),
        S_THIGH_A => Some((PELVIS, &[S_THIGH_A, S_SHIN_A])),
        S_SHIN_A => Some((KNEE_A, &[S_SHIN_A])),
        S_THIGH_B => Some((PELVIS, &[S_THIGH_B, S_SHIN_B])),
        S_SHIN_B => Some((KNEE_B, &[S_SHIN_B])),
        _ => None,
    }
}

/// Where a finisher wants the weapon hand (world angle around the shoulder,
/// distance, turn speed), and whether the free hand holds the victim up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandScript {
    pub ang: f32,
    pub reach: f32,
    pub omega: f32,
    pub hold: bool,
    /// Holding the victim (or what's left) up over your head.
    pub high: bool,
    /// Pressing the victim down onto their knees (the free hand on their neck).
    pub kneel: bool,
    /// Shoving the victim back against a wall at full stretch.
    pub shove: bool,
    /// Points the weapon this way instead of along the arm.
    pub aim: Option<f32>,
}

/// What made a wound (how it's drawn).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WoundKind {
    /// Where a bullet went in: small and neat.
    Hole,
    /// Where it came out: bigger and ragged.
    Exit,
    /// A blade or spike driven in: a slit.
    Stab,
    /// A blade dragged across: a long gash.
    Cut,
}

/// A wound, in its body part's own frame (so it moves with the part):
/// `at` is along the part and across it from its centre (metres); for a
/// slit or gash, `dir` runs from its middle to one end. Made to fit: the
/// whole wound lies on the part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wound {
    pub part: u8,
    pub kind: WoundKind,
    pub at: Vec2,
    pub dir: Vec2,
    /// Radius (metres).
    pub size: f32,
    /// The frame it was made on.
    pub born: u32,
    /// Frames left dripping blood.
    pub bleed: u16,
}

impl WoundKind {
    /// How far a hole's drawing reaches from its centre, and a slit's or
    /// gash's half-width, as multiples of its `size`.
    pub fn reach(self) -> f32 {
        match self {
            WoundKind::Hole => 1.0,
            WoundKind::Exit => 1.15,
            WoundKind::Stab => 0.5,
            WoundKind::Cut => 0.45,
        }
    }

    /// A slit's or gash's half-length, as a multiple of its `size` (0: round).
    pub fn length(self) -> f32 {
        match self {
            WoundKind::Stab => 1.3,
            WoundKind::Cut => 2.6,
            _ => 0.0,
        }
    }
}

/// How far in from a part's edge a wound must stay (of its radius), so it
/// reads as in the body, not stuck over its outline.
pub const WOUND_INSET: f32 = 0.88;

/// Most wounds a body keeps.
pub const MAX_WOUNDS: usize = 12;

/// How long a gunshot's kick takes over the gun arm.
pub const RECOIL_FRAMES: u8 = 10;

#[derive(Clone, Debug, PartialEq)]
pub struct Fighter {
    pub id: u8,
    pub base: u32,
    pub aim_base: u32,
    pub seg_base: u32,
    pub dist_base: u32,
    /// Kinematic particle used as the hold point when grabbing a ledge.
    pub anchor: u32,
    pub grip_a: u32,
    pub grip_b: u32,
    pub grab: u32,
    /// Holds the body's weight from a rope's anchor while hanging on it (so
    /// the light rope links don't have to).
    pub tether: u32,
    /// Hand drives (weapon hand A, free hand B).
    pub drive_a: u32,
    pub drive_b: u32,
    /// Where the weapon hand currently is going: angle around the shoulder
    /// and distance. Follows the mouse at a limited speed, so swings are
    /// smooth arcs instead of teleporting springs.
    pub hand_ang: f32,
    pub hand_reach: f32,
    /// Limb measurements (the character's build).
    pub body: Body,
    /// The first of this fighter's joint limits (`JOINTS`).
    pub bend_base: u32,
    /// Click attack frames left (counts down from `SWING_FRAMES`).
    pub swing: u8,
    /// The current attack is a stab (straight out) rather than a swing.
    pub swing_thrust: bool,
    /// Someone is stuck on our blade (set by the sim each frame): any attack
    /// becomes a swing that flings them off.
    pub skewering: bool,
    /// In a blade lock: where the blades are crossed (set by the sim each
    /// frame). Movement and attacks are off; the keys go to the duel.
    pub locked: Option<Vec2>,
    /// Performing a finisher: the hand follows this instead of the mouse.
    pub script: Option<HandScript>,
    /// Keys as actually pressed (`buttons` is masked while locked).
    pub raw_buttons: u16,
    pub prev_raw: u16,
    /// Slide frames left (crouch while running fast).
    pub slide: u8,
    /// Frames of being rocked by a hit: legs don't drive, no new attacks.
    pub stagger: u8,
    /// Badly hurt and seeing stars: slow and wobbly, and open to a finisher.
    pub dazed: u16,
    /// Head and spine torn out (drawn as bare vertebrae).
    pub spine_out: bool,
    /// Severed parts that have become loose limb weapons (not drawn on the
    /// body any more).
    pub limbs_gone: u16,
    /// Wall jumps used since last touching the ground (max `WALL_JUMPS`).
    pub wall_jumps: u8,
    /// The mid-air jump has been used (it comes back on landing, or hanging on).
    pub air_jumped: bool,
    /// Length of the current click attack (`SWING_FRAMES`, or `PUNCH_FRAMES`
    /// for fists).
    pub swing_len: u8,
    /// The current punch is thrown with the free hand (combos alternate).
    pub punch_b: bool,
    /// Kick frames left, and which kick (`KICK_LOW` / `KICK_DROP`).
    pub kick: u8,
    pub kick_kind: u8,
    /// Frames the gun arm is still giving way to a shot's kick.
    pub recoil: u8,
    /// Frames left wincing from being hurt (the pained face).
    pub hurt: u8,
    /// Holes, stabs and gashes, pinned to the body parts they're in (the
    /// oldest goes once there are `MAX_WOUNDS`).
    pub wounds: Vec<Wound>,
    /// Holding another fighter with the free hand: who, and by which body
    /// point (particle slot), and for how long.
    pub hold: Option<u8>,
    pub hold_slot: u8,
    pub hold_t: u16,
    /// Being held by someone (grabber, slot held); set by the sim each frame.
    pub held_by: Option<(u8, u8)>,
    /// Struggling against a hold: breaks free at 100.
    pub struggle: u8,
    /// Throw wind-up frames left (Q while holding someone).
    pub heave: u8,
    /// No grabbing until this runs out (just broken free from / thrown).
    pub grab_cd: u8,
    /// Thrown through the air: a hard landing or wall hurts (frames left,
    /// and who threw).
    pub flying: u8,
    pub flung_by: u8,
    /// Damage owed for being slammed down (a shoulder throw), dealt when the
    /// body first touches the level.
    pub slam: f32,
    /// Weight being dragged along (a held body); set by the sim each frame.
    pub carry: f32,
    /// Flipping over someone grabbed by the head in mid-air: frames left,
    /// and which way (+1 forward to the right, -1 to the left).
    pub flip: u8,
    pub flip_dir: i8,
    /// Throwing whoever you hold over your shoulder: frames left, and which
    /// way they land (+1 to the right). Their path is planned in `flip_pose`,
    /// `flip_path` and `flip_turn` (a flip and a shoulder throw never overlap).
    pub shoulder: u8,
    pub shoulder_dir: i8,
    /// Already dived for a grab on this jump (one lunge per time in the air).
    pub dove: bool,
    /// That dive was made with Shift (not the right mouse button).
    pub dove_shift: bool,
    /// Frames this (whipped) body still passes through whoever whipped it.
    pub ghost: u8,
    /// Shift pressed during the flip: the throw at the end is a whip.
    pub flip_whip: bool,
    /// Someone is close and roughly where you're aiming (set by the sim each
    /// frame): a mid-air grab dives at them instead of reaching for a ledge.
    pub prey: bool,
    /// Hanging off your own blade stuck in a wall (set by the sim each frame).
    pub hang_blade: bool,
    /// Holding a weapon that's stuck in the level (set by the sim each frame).
    pub stuck_grip: bool,
    /// Strung up by a noose, feet off the ground (set by the sim each frame).
    pub choking: bool,
    /// The flip's plan, taken when it starts: the body's shape (each
    /// particle's offset from the centre), where the centre starts, goes over
    /// and lands, and how far the body turns (radians, to end upright).
    pub flip_pose: [Vec2; PARTICLES as usize],
    pub flip_path: [Vec2; 3],
    pub flip_turn: f32,
    pub health: f32,
    pub alive: bool,
    pub facing: f32,
    pub phase: f32,
    pub grounded: bool,
    pub coyote: u8,
    pub jump_cd: u8,
    /// Knocked down (limp) frames left.
    pub down: u16,
    /// Getting-up frames left (muscles ramping back).
    pub getup: u16,
    pub air_t: u16,
    pub weapon: Option<u8>,
    pub grabbing: bool,
    pub grab_static: bool,
    pub severed: u16,
    pub buttons: u16,
    pub prev_buttons: u16,
    pub aim: Vec2,
    pub hit_flash: u8,
    pub bleed: u16,
    pub bleed_at: u32,
}

impl Fighter {
    #[inline]
    pub fn p(&self, slot: u32) -> u32 {
        self.base + slot
    }

    pub fn pos(&self, w: &World, slot: u32) -> Vec2 {
        w.particles[(self.base + slot) as usize].pos
    }

    pub fn particle_ids(&self) -> impl Iterator<Item = u32> + Clone + use<> {
        self.base..self.base + PARTICLES
    }

    pub fn has(&self, part: u32) -> bool {
        self.severed & bit(part) == 0
    }

    pub fn arm_a_ok(&self) -> bool {
        self.severed & ARM_A_BITS == 0
    }

    pub fn arm_b_ok(&self) -> bool {
        self.severed & ARM_B_BITS == 0
    }

    pub fn legs_left(&self) -> u32 {
        (self.severed & LEG_A_BITS == 0) as u32 + (self.severed & LEG_B_BITS == 0) as u32
    }

    pub fn pressed(&self, b: u16) -> bool {
        self.buttons & b != 0 && self.prev_buttons & b == 0
    }

    pub fn held(&self, b: u16) -> bool {
        self.buttons & b != 0
    }

    /// Shift grabs; right mouse grabs too while unarmed (armed, it stabs).
    pub fn wants_grab(&self) -> bool {
        self.held(GRAB) || (self.held(ALT) && self.weapon.is_none())
    }

    /// 0 = limp, 1 = full control.
    pub fn strength(&self) -> f32 {
        if !self.alive || self.down > 0 {
            return 0.0;
        }
        let ramp = if self.getup > 0 { 1.0 - self.getup as f32 / GETUP_FRAMES as f32 } else { 1.0 };
        let legs = match self.legs_left() {
            2 => 1.0,
            1 => 0.6,
            _ => 0.25,
        };
        // badly hurt fighters get wobbly; dazed ones barely stay up
        let hurt = 0.7 + 0.3 * (self.health / MAX_HEALTH).clamp(0.0, 1.0);
        let daze = if self.dazed > 0 { 0.55 } else { 1.0 };
        // held by the neck or body: hard to stand your ground
        let held = if self.held_core() { 0.6 } else { 1.0 };
        ramp * legs * hurt * daze * held
    }

    /// Held by the head, neck or body (not just a limb).
    pub fn held_core(&self) -> bool {
        self.held_by.is_some_and(|(_, slot)| matches!(slot as u32, HEAD | NECK | PELVIS))
    }

    /// Held by the weapon arm: it can't swing.
    pub fn arm_pinned(&self) -> bool {
        self.held_by.is_some_and(|(_, slot)| matches!(slot as u32, ELBOW_A | HAND_A))
    }

    /// The frame (counting down) a click attack's strike starts on.
    pub fn strike_frame(&self) -> u8 {
        self.swing_len - (self.swing_len * 3).div_ceil(10)
    }

    /// 0..1 progress of the current kick.
    pub fn kick_t(&self) -> Option<f32> {
        let len = if self.kick_kind == KICK_DROP { KICK_DROP_FRAMES } else { KICK_LOW_FRAMES };
        (self.kick > 0).then(|| 1.0 - self.kick as f32 / len as f32)
    }

    /// In the part of a kick that hurts.
    pub fn kicking(&self) -> bool {
        self.kick_t().is_some_and(|t| if self.kick_kind == KICK_DROP { t > 0.12 } else { (0.3..0.8).contains(&t) })
    }

    /// The arm segment that is punching right now (bare hands only).
    pub fn punch_part(&self) -> Option<u32> {
        (self.alive && self.weapon.is_none() && self.striking()).then_some(if self.punch_b { S_FORE_B } else { S_FORE_A })
    }

    pub fn aim_dir(&self) -> Vec2 {
        let a = self.aim;
        if a.length_squared() > 1e-4 { a.normalize() } else { Vec2::new(self.facing, 0.0) }
    }

    pub fn spawn(w: &mut World, id: u8, feet: Vec2, facing: f32) -> Self {
        let base = w.particles.len() as u32;
        let f = facing;
        // the character's build, standing: each bone along the same line the
        // original stick figure's stood along
        let build = crate::character::BUILD;
        let along = |from: Vec2, dir: Vec2, len: f32| from + dir.normalize() * len;
        let pelvis = Vec2::new(0.0, 0.98);
        let neck = along(pelvis, Vec2::Y, build.torso);
        let head = along(neck, Vec2::new(0.02 * f, 0.3), build.neck);
        let elbow_a = along(neck, Vec2::new(0.08 * f, -0.3), build.upper_arm);
        let hand_a = along(elbow_a, Vec2::new(0.12 * f, -0.26), build.forearm);
        let elbow_b = along(neck, Vec2::new(-0.06 * f, -0.3), build.upper_arm);
        let hand_b = along(elbow_b, Vec2::new(0.08 * f, -0.29), build.forearm);
        let knee_a = along(pelvis, Vec2::new(0.1 * f, -0.44), build.thigh);
        let foot_a = along(knee_a, Vec2::new(-0.04 * f, -0.44), build.shin);
        let knee_b = along(pelvis, Vec2::new(-0.1 * f, -0.44), build.thigh);
        let foot_b = along(knee_b, Vec2::new(-0.04 * f, -0.44), build.shin);
        let layout: [(Vec2, f32, f32); 11] = [
            (head, 1.0, build.head_r),
            (neck, 2.0, 0.08),
            (pelvis, 2.5, 0.09),
            (elbow_a, 0.4, 0.07),
            (hand_a, 0.4, 0.08),
            (elbow_b, 0.4, 0.07),
            (hand_b, 0.4, 0.08),
            (knee_a, 0.6, 0.08),
            (foot_a, 0.6, 0.08),
            (knee_b, 0.6, 0.08),
            (foot_b, 0.6, 0.08),
        ];
        for (off, mass, r) in layout {
            let mut p = Particle::new(feet + off, mass, r);
            p.friction = 0.9;
            w.add_particle(p);
        }
        let dist_base = w.distances.len() as u32;
        for (a, b) in BONES.iter().skip(1) {
            w.add_distance(base + a, base + b, 0.0);
        }
        w.add_distance(base + NECK, base + HEAD, 0.0);
        let aim_base = w.aims.len() as u32;
        let muscle_bones: [(u32, u32); 10] = [
            (PELVIS, NECK),
            (NECK, HEAD),
            (NECK, ELBOW_A),
            (ELBOW_A, HAND_A),
            (NECK, ELBOW_B),
            (ELBOW_B, HAND_B),
            (PELVIS, KNEE_A),
            (KNEE_A, FOOT_A),
            (PELVIS, KNEE_B),
            (KNEE_B, FOOT_B),
        ];
        for (a, b) in muscle_bones {
            w.add_aim(base + a, base + b, 1e-4);
        }
        let seg_base = w.segments.len() as u32;
        let group = id as u16 + 1;
        for (part, (a, b)) in BONES.iter().enumerate() {
            let r = match part as u32 {
                S_HEAD => build.head_r,
                S_TORSO => build.torso_r,
                S_UPPER_A | S_UPPER_B => build.upper_arm_r,
                S_FORE_A | S_FORE_B => build.forearm_r,
                S_THIGH_A | S_THIGH_B => build.thigh_r,
                _ => build.shin_r,
            };
            let s = w.add_segment(base + a, base + b, r, group, seg_tag(id, part as u32));
            // torso and arms also collide along their length (legs use the joints)
            w.segments[s as usize].collide_static = matches!(part as u32, S_TORSO | S_UPPER_A | S_FORE_A | S_UPPER_B | S_FORE_B);
        }
        let mut anchor = Particle::new(Vec2::new(0.0, -1000.0), 0.0, 0.01);
        anchor.collide_static = false;
        let anchor = w.add_particle(anchor);
        let grip_a = w.add_pin_slot(0.0);
        let grip_b = w.add_pin_slot(2e-4);
        let grab = w.add_pin_slot(3e-4);
        let tether = w.add_pin_slot(0.0);
        let drive_a = w.add_drive(base + NECK, base + HAND_A);
        let drive_b = w.add_drive(base + NECK, base + HAND_B);
        // joints only bend the ways a body can (set each frame: `update_joints`)
        let bend_base = w.bends.len() as u32;
        for j in &JOINTS {
            let [a, bb, c] = j.p.map(|p| base + p);
            w.add_bend(stickfell_physics::Bend { a, b: bb, c, min: j.range.0, max: j.range.1, compliance: 0.0, enabled: false });
        }
        Fighter {
            id,
            base,
            aim_base,
            seg_base,
            dist_base,
            anchor,
            grip_a,
            grip_b,
            grab,
            tether,
            drive_a,
            drive_b,
            hand_ang: if facing > 0.0 { -0.3 } else { core::f32::consts::PI + 0.3 },
            hand_reach: 0.45,
            body: Body::of(&build),
            bend_base,
            swing: 0,
            swing_thrust: false,
            skewering: false,
            locked: None,
            script: None,
            raw_buttons: 0,
            prev_raw: 0,
            slide: 0,
            stagger: 0,
            dazed: 0,
            spine_out: false,
            limbs_gone: 0,
            wall_jumps: 0,
            air_jumped: false,
            swing_len: SWING_FRAMES,
            punch_b: false,
            kick: 0,
            recoil: 0,
            hurt: 0,
            wounds: Vec::new(),
            kick_kind: 0,
            hold: None,
            hold_slot: 0,
            hold_t: 0,
            held_by: None,
            struggle: 0,
            heave: 0,
            grab_cd: 0,
            flying: 0,
            flung_by: 0,
            slam: 0.0,
            carry: 0.0,
            flip: 0,
            flip_dir: 0,
            shoulder: 0,
            shoulder_dir: 0,
            dove: false,
            dove_shift: false,
            ghost: 0,
            flip_whip: false,
            prey: false,
            hang_blade: false,
            stuck_grip: false,
            choking: false,
            flip_pose: [Vec2::ZERO; PARTICLES as usize],
            flip_path: [Vec2::ZERO; 3],
            flip_turn: 0.0,
            health: MAX_HEALTH,
            alive: true,
            facing,
            phase: 0.0,
            grounded: false,
            coyote: 0,
            jump_cd: 0,
            down: 0,
            getup: 0,
            air_t: 0,
            weapon: None,
            grabbing: false,
            grab_static: false,
            severed: 0,
            buttons: 0,
            prev_buttons: 0,
            aim: Vec2::new(facing * 0.5, 0.0),
            hit_flash: 0,
            bleed: 0,
            bleed_at: 0,
        }
    }

    fn muscle(&self, w: &mut World, m: u32, target: f32, compliance: f32) {
        let a = &mut w.aims[(self.aim_base + m) as usize];
        a.target = target;
        a.compliance = compliance;
        // limbs turn at most ~14 rad/s: fast enough to look lively, slow
        // enough that a planted foot can't stick and then snap free
        a.max_step = if m == M_TORSO || m == M_HEAD { 0.05 } else { 0.025 };
        a.damping = 2.0;
        a.enabled = true;
    }

    pub fn relax(&self, w: &mut World, m: u32) {
        w.aims[(self.aim_base + m) as usize].enabled = false;
    }

    /// Muscle slot driving a body part.
    pub fn muscle_of(part: u32) -> u32 {
        match part {
            S_TORSO => M_TORSO,
            S_HEAD => M_HEAD,
            p => p,
        }
    }

    pub fn kill(&mut self, w: &mut World) {
        self.alive = false;
        self.kick = 0;
        for slot in [KNEE_A, FOOT_A] {
            w.particles[self.p(slot) as usize].collide_static = true;
        }
        self.health = self.health.min(0.0);
        for m in 0..MUSCLES {
            self.relax(w, m);
        }
        w.drives[self.drive_a as usize].enabled = false;
        w.drives[self.drive_b as usize].enabled = false;
    }

    /// How far through a shoulder throw (0 to 1, after this frame), eased to
    /// speed up into the slam.
    pub fn shoulder_t(&self) -> f32 {
        let u = 1.0 - (self.shoulder.max(1) - 1) as f32 / SHOULDER_FRAMES as f32;
        u * u * (1.5 - 0.5 * u)
    }

    /// The throwing hand during a shoulder throw (from the shoulder, before
    /// arm length): back over the shoulder, up over the head, down in front.
    fn shoulder_hand(&self) -> Vec2 {
        let (f, t) = (self.facing, self.shoulder_t());
        let [a, b, c] = [Vec2::new(-f * 0.4, 0.45), Vec2::new(f * 0.05, 0.85), Vec2::new(f * 0.6, -0.35)];
        a * (1.0 - t) * (1.0 - t) + b * 2.0 * t * (1.0 - t) + c * t * t
    }

    /// Knock down for at least `frames`.
    pub fn knock_down(&mut self, frames: u16) {
        self.down = self.down.max(frames);
        self.getup = 0;
        self.swing = 0;
        self.heave = 0;
        self.kick = 0;
        self.flip = 0;
        self.shoulder = 0;
    }

    /// Hanging by the neck: both hands claw at the rope above the head and
    /// the legs kick, weaker as the air runs out.
    fn choke(&mut self, w: &mut World) {
        self.down = 0;
        self.getup = 0;
        let t = self.air_t as f32;
        let fight = (self.health / 40.0).clamp(0.2, 1.0);
        // head up, body straight down below it, however hard they kick
        self.muscle(w, M_TORSO, FRAC_PI_2, 6e-5);
        self.muscle(w, M_HEAD, FRAC_PI_2, 6e-4);
        let arms = [(self.drive_a, self.arm_a_ok(), 0.0, S_UPPER_A, S_FORE_A), (self.drive_b, self.arm_b_ok(), 1.7, S_UPPER_B, S_FORE_B)];
        for (drive, ok, phase, upper, fore) in arms {
            self.relax(w, upper);
            self.relax(w, fore);
            if ok {
                let pull = dmath::sin(t * 0.3 + phase);
                let at = Vec2::new(0.05 * dmath::sin(t * 0.7 + phase), 0.3 + 0.12 * pull) * self.body.neck_k();
                self.set_drive(w, drive, at, 900.0 * fight, 0.4, 0.03, 0.3);
            } else {
                w.drives[drive as usize].enabled = false;
            }
        }
        let down = -FRAC_PI_2;
        let k = dmath::sin(t * 0.45) * 0.9 * fight;
        for (thigh, shin, side) in [(S_THIGH_A, S_SHIN_A, 1.0), (S_THIGH_B, S_SHIN_B, -1.0)] {
            if self.has(thigh) {
                self.muscle(w, thigh, down + k * side, 6e-4 / fight);
            }
            if self.has(shin) {
                self.muscle(w, shin, down + k * side * 0.5 - self.facing * 0.4, 8e-4 / fight);
            }
        }
    }

    /// Points each joint limit the way the fighter faces, and switches off
    /// any whose body parts have been cut off (or all of them, if not `on`).
    pub fn update_joints(&self, w: &mut World, on: bool) {
        for (k, j) in JOINTS.iter().enumerate() {
            let b = &mut w.bends[(self.bend_base + k as u32) as usize];
            let (lo, hi) = j.range;
            (b.min, b.max) = if j.mirrors && self.facing < 0.0 { (-hi, -lo) } else { (lo, hi) };
            let torso_whole = !(self.spine_out && j.parts.contains(&S_TORSO));
            b.enabled = on && torso_whole && j.parts.iter().all(|&p| self.has(p));
        }
    }

    /// Hanging from something (a rope, a ledge, a stuck blade) with the feet
    /// off the ground: the body goes limp so it swings freely.
    pub fn hanging(&self) -> bool {
        !self.grounded && ((self.grabbing && self.grab_static) || self.hang_blade)
    }

    /// Reaching out head first for a grab in mid-air.
    pub fn diving(&self) -> bool {
        !self.grounded && self.coyote == 0 && self.air_t > 3 && self.prey && self.wants_grab() && !self.grabbing && self.flip == 0 && self.arm_b_ok()
    }

    /// 0..1 progress of the click attack, if one is running.
    pub fn swing_t(&self) -> Option<f32> {
        (self.swing > 0).then(|| 1.0 - self.swing as f32 / self.swing_len.max(1) as f32)
    }

    /// In the fast, damaging part of a click attack.
    pub fn striking(&self) -> bool {
        self.swing_t().is_some_and(|t| (0.3..0.75).contains(&t))
    }

    /// Angle the held weapon should point along.
    pub fn weapon_angle(&self) -> f32 {
        match (self.locked, self.script.and_then(|sc| sc.aim)) {
            // blades raised and crossed in an X over the lock
            (Some(_), _) => dmath::wrap_angle(self.hand_ang + self.facing * 0.9),
            (None, Some(aim)) => aim,
            (None, None) => self.hand_ang,
        }
    }

    /// A body part's frame: its centre, and the way it runs (for the head,
    /// from its centre down toward the neck).
    pub fn part_frame(&self, w: &World, part: u32) -> (Vec2, Vec2) {
        if part == S_HEAD {
            let head = self.pos(w, HEAD);
            return (head, (self.pos(w, NECK) - head).normalize_or(Vec2::NEG_Y));
        }
        let sg = w.segments[(self.seg_base + part) as usize];
        let (a, b) = (w.particles[sg.a as usize].pos, w.particles[sg.b as usize].pos);
        ((a + b) * 0.5, (b - a).normalize_or(Vec2::X))
    }

    /// Where a wound is in the world right now, and the way it runs.
    pub fn wound_at(&self, w: &World, wd: &Wound) -> (Vec2, Vec2) {
        let (c, axis) = self.part_frame(w, wd.part as u32);
        let across = axis.perp();
        (c + axis * wd.at.x + across * wd.at.y, axis * wd.dir.x + across * wd.dir.y)
    }

    /// A key went down this frame (ignores the lock's masking).
    pub fn pressed_raw(&self, b: u16) -> bool {
        self.raw_buttons & b != 0 && self.prev_raw & b == 0
    }

    /// Points a hand drive at `offset` from the neck. `k` is the spring
    /// stiffness (N/m) and `mass` the mass it moves, used for ~critical damping.
    /// `reaction` must stay 1 (equal and opposite on the body): anything less
    /// lets a hand straining at an unreachable target push the fighter along.
    #[allow(clippy::too_many_arguments)]
    fn set_drive(&self, w: &mut World, drive: u32, offset: Vec2, k: f32, mass: f32, max_step: f32, reaction: f32) {
        let compliance = 1.0 / k;
        let c = 1.6 * (k * mass).sqrt();
        let d = &mut w.drives[drive as usize];
        d.offset = offset;
        d.compliance = compliance;
        d.damping = c * compliance / SUB_H;
        d.max_step = max_step;
        d.reaction = reaction;
        d.enabled = true;
    }

    /// Outward normal of a wall right beside the body (either side), if any.
    pub fn wall_beside(&self, w: &World) -> Option<Vec2> {
        let reach = 0.5;
        for from in [self.pos(w, PELVIS), self.pos(w, NECK)] {
            for side in [-1.0f32, 1.0] {
                if let Some(h) = w.raycast(from, Vec2::new(side, 0.0), reach) {
                    if h.t > 0.0 && h.normal.x.abs() > 0.7 {
                        return Some(h.normal);
                    }
                }
            }
        }
        None
    }

    /// Mass-weighted velocity of head, neck and pelvis.
    pub fn body_vel(&self, w: &World) -> Vec2 {
        let mut v = Vec2::ZERO;
        let mut m = 0.0;
        for slot in [HEAD, NECK, PELVIS] {
            let p = &w.particles[self.p(slot) as usize];
            v += p.vel * p.mass();
            m += p.mass();
        }
        v / m
    }

    /// Ground under the body: the nearest walkable surface below the pelvis
    /// or either foot (so standing at a ledge edge still counts).
    fn ground_probe(&self, w: &World) -> Option<f32> {
        let pelvis = self.pos(w, PELVIS);
        // only surfaces the feet could simply step onto count: anything taller
        // has to be climbed on purpose (jump), never hoisted onto automatically
        // (a foot skimming through the floor in a sweep doesn't count)
        let feet_y = [FOOT_A, FOOT_B]
            .iter()
            .filter(|&&q| w.particles[self.p(q) as usize].collide_static)
            .map(|&q| self.pos(w, q).y)
            .fold(f32::MAX, f32::min)
            .min(self.pos(w, PELVIS).y)
            - 0.08;
        let mut best: Option<f32> = None;
        for x in [pelvis.x, self.pos(w, FOOT_A).x, self.pos(w, FOOT_B).x] {
            let x = pelvis.x + (x - pelvis.x).clamp(-0.3, 0.3);
            if let Some(h) = w.raycast(Vec2::new(x, pelvis.y), Vec2::NEG_Y, 1.6 * self.body.leg_k()) {
                let too_high = h.point.y > feet_y + MAX_STEP;
                if h.normal.y > 0.5 && h.t > 0.0 && !too_high && best.is_none_or(|b| h.t < b) {
                    best = Some(h.t);
                }
            }
        }
        best
    }

    /// One frame of body control (before `World::step`). `load` is the mass
    /// of the held weapon (0 if none), `two_handed` if both hands are on it,
    /// `thrust` if its attack is a stab instead of a swing.
    pub fn control(&mut self, w: &mut World, dt: f32, load: f32, two_handed: bool, thrust: bool, gun: bool) {
        self.hit_flash = self.hit_flash.saturating_sub(1);
        if !self.alive {
            return;
        }
        self.jump_cd = self.jump_cd.saturating_sub(1);
        self.recoil = self.recoil.saturating_sub(1);
        self.hurt = self.hurt.saturating_sub(1);
        self.dazed = self.dazed.saturating_sub(1);
        self.grab_cd = self.grab_cd.saturating_sub(1);
        self.flying = self.flying.saturating_sub(1);
        if self.down > 0 {
            self.down -= 1;
            if self.down == 0 {
                self.getup = GETUP_FRAMES;
            }
        } else {
            self.getup = self.getup.saturating_sub(1);
        }

        let pelvis = self.pos(w, PELVIS);
        let neck = self.pos(w, NECK);
        // upper-body average velocity: a single particle's velocity is too
        // noisy to steer by (legs and contacts jostle it within the frame)
        let vel = self.body_vel(w);
        let tilt = dmath::angle_diff(dmath::angle(neck - pelvis), FRAC_PI_2);

        // ---- ground ----
        let probe = self.ground_probe(w);
        let feet_down = [FOOT_A, FOOT_B, KNEE_A, KNEE_B]
            .iter()
            .any(|&s| w.particles[self.p(s) as usize].on_ground());
        let ray_ground = probe.is_some_and(|t| t < STAND_HEIGHT * self.body.leg_k() + 0.45);
        self.grounded = (ray_ground || feet_down) && self.jump_cd < 6;
        if self.grounded {
            self.coyote = 6;
            self.air_t = 0;
        } else {
            self.coyote = self.coyote.saturating_sub(1);
            self.air_t = self.air_t.saturating_add(1);
        }
        // strung up by the neck: claw at the rope and kick, nothing else
        if self.choking {
            self.choke(w);
            return;
        }
        // toppled over: go limp for a moment, then get up
        // (mid-flip, upside down is the point)
        if self.down == 0 && self.getup == 0 && self.grounded && tilt.abs() > 1.25 && self.flip == 0 {
            self.knock_down(24);
        }

        let strength = self.strength();
        let dir_x = self.held(RIGHT) as i32 as f32 - self.held(LEFT) as i32 as f32;
        if let Some(lock) = self.locked {
            self.facing = if lock.x >= pelvis.x { 1.0 } else { -1.0 };
            self.swing = 0;
        } else if self.flip > 0 {
            self.facing = self.flip_dir as f32;
        } else if self.shoulder > 0 {
            self.facing = self.shoulder_dir as f32;
        } else if self.script.is_none() && self.swing == 0 && strength > 0.2 && self.aim.x.abs() > 0.15 {
            // (a finisher keeps you facing your victim, whatever the mouse does)
            self.facing = self.aim.x.signum();
        }
        let f = self.facing;
        // crouch while running fast: slide feet first until it runs out
        if self.pressed(CROUCH) && self.grounded && self.slide == 0 && vel.x.abs() > SLIDE_MIN_SPEED && strength > 0.6 {
            self.slide = SLIDE_FRAMES;
        }
        if self.slide > 0 {
            self.slide -= 1;
            if !self.held(CROUCH) || !self.grounded || vel.x.abs() < 1.5 || strength < 0.6 {
                self.slide = 0;
            }
        }
        let sliding = self.slide > 0;
        let crouch = self.held(CROUCH) && self.grounded;

        // feet: slide while running (or sliding), plant while idle
        let foot_friction = if sliding {
            0.03
        } else if dir_x != 0.0 {
            0.15
        } else {
            0.7
        };
        for slot in [FOOT_A, FOOT_B] {
            w.particles[self.p(slot) as usize].friction = foot_friction;
        }
        // the kicking leg skims over the floor instead of levering the body
        // up off it
        let skim = self.kick > 0 && self.kick_kind == KICK_LOW && self.has(S_THIGH_A) && self.has(S_SHIN_A);
        for slot in [KNEE_A, FOOT_A] {
            w.particles[self.p(slot) as usize].collide_static = !skim;
        }

        // ---- locomotion ----
        // (a stab's lunge coasts: no braking until the strike is over)
        let lunging = self.swing_thrust && self.striking() && self.weapon.is_some();
        self.stagger = self.stagger.saturating_sub(1);
        // swinging in place plants the feet: a heavy swing twists the body
        // but doesn't drag it across the floor
        let planted = self.swing > 0 && !self.swing_thrust && self.grounded && dir_x == 0.0 && strength > 0.5;
        if planted {
            for id in self.particle_ids() {
                w.particles[id as usize].vel.x *= 0.8;
            }
        }
        if self.stagger > 0 {
            // rocked back on your heels: no running against it
        } else if sliding {
            // coast, slowly scrubbing off speed
            for id in self.particle_ids() {
                w.particles[id as usize].vel.x *= 0.988;
            }
        } else if self.flip > 0 {
            // the flip carries you; no running mid-air somersault
        } else if self.shoulder > 0 {
            // planted for the throw
            for id in self.particle_ids() {
                w.particles[id as usize].vel.x *= 0.8;
            }
        } else if !lunging && (strength > 0.15 || !self.grounded) {
            let haul = 1.0 / (1.0 + self.carry * 0.045);
            let speed = if crouch { RUN_SPEED * CROUCH_SPEED } else { RUN_SPEED } * haul;
            let accel = if self.grounded { 55.0 * strength } else { 22.0 };
            let accel = if self.held_by.is_some() && dir_x == 0.0 { accel * 0.3 } else { accel };
            let dv = (dir_x * speed - vel.x).clamp(-accel * dt, accel * dt);
            // in the air, holding a direction only ever adds speed that way:
            // flying off a rope (or a throw) faster than you run isn't braked
            let dv = if !self.grounded && dv * dir_x < 0.0 { 0.0 } else { dv };
            if dir_x != 0.0 || self.grounded {
                let ids = [HEAD, NECK, PELVIS, ELBOW_A, HAND_A, ELBOW_B, HAND_B].map(|s| self.p(s));
                w.add_velocity(ids, Vec2::new(dv, 0.0));
            }
        }

        // hover spring + partial gravity compensation, scaled by strength
        if let (true, Some(t)) = (self.grounded && self.jump_cd == 0 && strength > 0.0 && self.flip == 0, probe) {
            let height = if sliding {
                SLIDE_HEIGHT
            } else if crouch {
                CROUCH_HEIGHT
            } else {
                STAND_HEIGHT
            } * self.body.leg_k();
            let err = height - t;
            let ay = ((err * 170.0 - vel.y * 15.0) * strength).clamp(-50.0, 70.0);
            let g = -w.cfg.gravity.y * strength;
            for id in self.particle_ids() {
                let slot = id - self.base;
                if slot == FOOT_A || slot == FOOT_B {
                    continue;
                }
                let p = &mut w.particles[id as usize];
                let mut a = g;
                if slot == PELVIS || slot == NECK {
                    a += ay;
                }
                p.force.y += a * p.mass();
            }
        }

        // ---- jumping ----
        if self.grounded {
            self.wall_jumps = 0;
        }
        if self.grounded || (self.grabbing && self.grab_static) || self.hang_blade {
            self.air_jumped = false;
        }
        if self.pressed(JUMP) && self.jump_cd == 0 && self.coyote > 0 && strength > 0.6 {
            let speed = JUMP_SPEED * strength.max(0.75);
            for id in self.particle_ids() {
                let p = &mut w.particles[id as usize];
                p.vel.y = p.vel.y.max(0.0) + speed;
                p.vel.x += dir_x * 1.5;
            }
            self.jump_cd = 14;
            self.coyote = 0;
            self.grounded = false;
        } else if !self.grounded && self.coyote == 0 && self.down == 0 && strength > 0.5 {
            // airborne next to a wall: jump off it (twice before landing), and
            // slide down it slowly while pushing into it
            let wall = self.wall_beside(w);
            let wall_jump = wall.is_some() && self.wall_jumps < WALL_JUMPS;
            // a jump in mid-air: once, until you land (not while jump is
            // letting go of a rope or heaving up off a blade)
            let free = !self.grabbing && !self.hang_blade && self.flip == 0 && self.shoulder == 0 && self.held_by.is_none();
            if self.pressed(JUMP) && self.jump_cd == 0 && !wall_jump && !self.air_jumped && free {
                for id in self.particle_ids() {
                    let p = &mut w.particles[id as usize];
                    p.vel.y = p.vel.y.max(0.0) * 0.3 + DOUBLE_JUMP_SPEED;
                    p.vel.x += dir_x * 1.5;
                }
                self.air_jumped = true;
                self.jump_cd = 12;
            }
            if let Some(n) = wall {
                if self.pressed(JUMP) && self.jump_cd == 0 && self.wall_jumps < WALL_JUMPS {
                    for id in self.particle_ids() {
                        let p = &mut w.particles[id as usize];
                        p.vel = Vec2::new(n.x * WALL_JUMP_PUSH, p.vel.y.max(0.0) * 0.3 + WALL_JUMP_SPEED);
                    }
                    self.wall_jumps += 1;
                    self.jump_cd = 12;
                    self.facing = n.x.signum();
                } else if dir_x * n.x < 0.0 && vel.y < -WALL_SLIDE {
                    for id in self.particle_ids() {
                        let p = &mut w.particles[id as usize];
                        p.vel.y = p.vel.y.max(-WALL_SLIDE);
                    }
                }
            }
        }
        if !self.held(JUMP) && vel.y > 0.0 && !self.grounded && self.jump_cd < 8 && !self.hanging() {
            for id in self.particle_ids() {
                let p = &mut w.particles[id as usize];
                p.force.y -= 18.0 * p.mass();
            }
        }

        // ---- diving for a grab ----
        if self.grounded {
            self.dove = false;
        }
        let grab_pressed = self.pressed(GRAB) || (self.pressed(ALT) && self.weapon.is_none());
        if grab_pressed && !self.dove && self.diving() && strength > 0.4 {
            self.dove = true;
            self.dove_shift = self.pressed(GRAB);
            // head first at whatever you're reaching for (never straight back)
            let a = self.aim_dir();
            let d = Vec2::new(f * a.x.abs().max(0.35), a.y.clamp(-0.95, 0.6)).normalize();
            let speed = vel.dot(d).max(DIVE_SPEED);
            for id in self.particle_ids() {
                let p = &mut w.particles[id as usize];
                p.vel = d * speed;
            }
        }

        // ---- kicks (bare feet) ----
        // crouch + click: a low sweep; click in mid-air: a flying dropkick
        let free = self.weapon.is_none() && self.script.is_none() && self.locked.is_none() && self.stagger == 0;
        if free && self.pressed(ATTACK) && self.kick == 0 && self.swing == 0 && self.heave == 0 && self.shoulder == 0 && strength > 0.4 {
            if crouch && !sliding && self.legs_left() == 2 {
                self.kick = KICK_LOW_FRAMES;
                self.kick_kind = KICK_LOW;
            } else if !self.grounded && self.coyote == 0 && self.air_t > 3 && self.legs_left() == 2 {
                self.kick = KICK_DROP_FRAMES;
                self.kick_kind = KICK_DROP;
                // launch feet first along the aim
                let a = self.aim_dir();
                let d = Vec2::new(f * a.x.abs().max(0.5), a.y.clamp(-0.8, 0.5)).normalize();
                let speed = (vel.dot(d)).max(0.0).max(DROPKICK_SPEED);
                for id in self.particle_ids() {
                    let p = &mut w.particles[id as usize];
                    p.vel = d * speed;
                }
            }
        }
        if self.kick > 0 {
            self.kick -= 1;
            if self.kick_kind == KICK_LOW && self.kick == KICK_LOW_FRAMES - 5 && self.grounded {
                // step into the kick
                w.add_velocity(self.particle_ids(), Vec2::new(f * KICK_LUNGE, 0.0));
            }
            // a dropkick always ends flat on your back
            let landed = self.kick_kind == KICK_DROP && self.grounded && self.kick_t().is_some_and(|t| t > 0.3);
            if self.kick_kind == KICK_DROP && (self.kick == 0 || landed) {
                self.knock_down(30);
            }
        }

        // ---- muscles ----
        self.swing = self.swing.saturating_sub(1);
        if strength <= 0.02 || self.flip > 0 {
            for m in 0..MUSCLES {
                self.relax(w, m);
            }
            w.drives[self.drive_a as usize].enabled = false;
            w.drives[self.drive_b as usize].enabled = false;
            return;
        }
        let weak = 1.0 / strength.max(0.05);
        // hanging on: a dead weight under the hand, free to swing (until
        // you attack from up there)
        let limp = self.hanging() && self.swing == 0 && self.kick == 0 && self.script.is_none();
        let stabbing = self.swing_thrust && self.striking();
        let slide_dir = if vel.x >= 0.0 { 1.0 } else { -1.0 };
        let lean = if sliding {
            slide_dir * 0.35 // lean back into the slide
        } else {
            let l = (-vel.x * 0.035).clamp(-0.35, 0.35) - if crouch { f * 0.75 } else { 0.0 } - if stabbing { f * 0.5 } else { 0.0 };
            // (never so far over that it counts as falling)
            l.clamp(-1.0, 1.0)
        };
        // kicking leans back away from the leg
        let lean = match (self.kick > 0, self.kick_kind) {
            (true, KICK_DROP) => f * 1.0,
            (true, _) => lean + f * 0.45,
            _ => lean,
        };
        // diving for a grab: head first, reaching out
        let lean = if self.diving() { -f * 1.2 } else { lean };
        if limp {
            self.relax(w, M_TORSO);
            self.relax(w, M_HEAD);
        } else {
            self.muscle(w, M_TORSO, FRAC_PI_2 + lean, 3.5e-5 * weak);
            self.muscle(w, M_HEAD, FRAC_PI_2 + lean * 0.5, 7e-4 * weak);
        }

        let down = -FRAC_PI_2;
        let airborne = self.coyote == 0;
        let moving = self.grounded && vel.x.abs() > 0.6;
        if moving {
            self.phase = dmath::wrap_angle(self.phase + vel.x.abs() * dt * 2.6);
        } else if self.grounded {
            self.phase *= 0.85;
        }
        let (s, c) = dmath::sin_cos(self.phase);
        let (thigh_a, bend_a, thigh_b, bend_b) = if sliding {
            // feet first, legs nearly straight
            let f = slide_dir;
            (down + f * 1.25, 0.15, down + f * 0.9, 0.5)
        } else if airborne {
            let k = dmath::sin(self.air_t as f32 * 0.3) * 0.5; // bicycle kicks
            (down + f * (0.4 + k), 0.9, down - f * (0.2 + k), 0.6)
        } else if crouch && moving {
            // a quick low scuttle
            (down + f * (1.3 + 0.3 * s), 2.5 + 0.2 * c, down + f * (1.3 - 0.3 * s), 2.5 - 0.2 * c)
        } else if crouch {
            (down + f * 1.45, 2.7, down + f * 1.05, 2.5)
        } else if moving {
            (down + f * 0.6 * s, 0.25 + 0.75 * (-c).max(0.0), down - f * 0.6 * s, 0.25 + 0.75 * c.max(0.0))
        } else {
            (down + f * 0.2, 0.12, down - f * 0.14, 0.08)
        };
        let (thigh_a, bend_a, thigh_b, bend_b) = if self.diving() {
            // legs trailing out behind
            (down - f * 0.7, 0.3, down - f * 0.9, 0.5)
        } else {
            (thigh_a, bend_a, thigh_b, bend_b)
        };
        let (thigh_a, bend_a, thigh_b, bend_b) = match (self.kick_t(), self.kick_kind) {
            // both feet straight out in front
            (Some(_), KICK_DROP) => (down + f * 1.55, 0.05, down + f * 1.35, 0.15),
            // a sweep from the squat: leg tucked, then scythed out straight
            // along the floor at ankle height
            (Some(t), _) if t < 0.25 => (down + f * 0.7, 2.0, thigh_b, bend_b),
            (Some(t), _) if t < 0.8 => (down + f * 1.42, 0.0, thigh_b, bend_b),
            _ => (thigh_a, bend_a, thigh_b, bend_b),
        };
        let leg_tilt = if self.kick > 0 { 0.0 } else { tilt * 0.5 };
        // legs: soft on the ground, softer still in the air (they trail and
        // flop); a kick tenses them
        let leg_c = if self.kick > 0 && self.kick_kind == KICK_DROP {
            5e-5
        } else if airborne {
            3.5e-4
        } else if crouch && self.kick == 0 {
            // squatting legs give way and fold under the body
            9e-4
        } else {
            2.4e-4
        } * weak;
        for (thigh, shin, ang, bend) in [(S_THIGH_A, S_SHIN_A, thigh_a, bend_a), (S_THIGH_B, S_SHIN_B, thigh_b, bend_b)] {
            if limp {
                self.relax(w, thigh);
                self.relax(w, shin);
                continue;
            }
            if self.has(thigh) {
                self.muscle(w, thigh, ang + leg_tilt, leg_c);
            }
            if self.has(shin) {
                self.muscle(w, shin, ang + leg_tilt - f * bend, leg_c);
            }
            if self.kick > 0 {
                // a kick snaps out far faster than a step
                w.aims[(self.aim_base + thigh) as usize].max_step = 0.07;
                w.aims[(self.aim_base + shin) as usize].max_step = 0.09;
            }
        }

        // ---- weapon hand (A) ----
        // The hand target turns toward the mouse at a capped rate; a click
        // plays wind-up -> strike -> recover (pull back -> stab for thrusts
        // and fists).
        let fists = self.weapon.is_none();
        let stab = self.pressed(ALT) && !fists;
        let swing_click = self.pressed(ATTACK) && !gun && self.kick == 0;
        // fists: click again as a punch lands to throw the other hand (one-two)
        let combo = fists && self.swing_t().is_some_and(|t| t >= 0.55) && self.arm_b_ok() && !self.grabbing;
        let ready = self.swing == 0 || combo;
        if (swing_click || stab) && ready && self.stagger == 0 && self.heave == 0 && self.shoulder == 0 && self.arm_a_ok() && !self.arm_pinned() && strength > 0.4 {
            self.swing_len = if fists { PUNCH_FRAMES } else { SWING_FRAMES };
            self.swing = self.swing_len;
            self.swing_thrust = (thrust || fists || stab) && !self.skewering;
            // fists take turns, left, right, left (the other hand comes back to guard)
            self.punch_b = fists && self.arm_b_ok() && !self.grabbing && !self.punch_b;
        }
        let punching_b = fists && self.punch_b && self.swing > 0;
        let aim_ang = dmath::angle(self.aim_dir());
        let heavy = 1.0 / (1.0 + load * 0.1);
        let mut want_ang = aim_ang;
        // (reaches scale with the arm: full stretch is ARM_REACH on the game's build)
        let ak = self.body.arm_k();
        let full = ARM_REACH * ak;
        let mut want_reach = self.aim.length().clamp(0.3 * ak, full);
        if let Some(lock) = self.locked {
            // hands pressed in toward the crossed blades
            let to = lock - neck;
            want_ang = dmath::angle(to);
            want_reach = to.length().clamp(0.3 * ak, full);
        }
        let (mut omega, mut w2, mut spin, mut reach_rate) = (24.0, 900.0, 0.0, 5.0);
        if let Some(sc) = self.script {
            want_ang = sc.ang;
            want_reach = sc.reach;
            omega = sc.omega;
            w2 = 2600.0;
            reach_rate = 14.0;
            self.swing = 0;
        }
        if let Some(t) = self.swing_t() {
            if self.swing_thrust {
                want_reach = if t < 0.3 {
                    if fists { 0.3 * ak } else { 0.15 * ak }
                } else if t < 0.75 {
                    full + STAB_EXTRA
                } else {
                    want_reach
                };
                omega = 10.0;
                reach_rate = if t < 0.3 { 6.0 } else { 24.0 };
            } else {
                let (off, om) = if t < 0.3 {
                    (2.1, 20.0)
                } else if t < 0.75 {
                    (-1.3, 30.0)
                } else {
                    (0.0, 24.0)
                };
                want_ang = aim_ang + f * off;
                omega = om;
                spin = if t < 0.3 {
                    f
                } else if t < 0.75 {
                    -f
                } else {
                    0.0
                };
            }
            if self.striking() {
                w2 = 2600.0;
            }
        }
        // stabs lunge: a step into the thrust as it fires (a punch, a little one)
        // (in mid-air, half a lunge: enough to reach a wall you're next to)
        if self.swing_thrust && self.swing == self.strike_frame() && (self.grounded || !fists) {
            let lunge = if fists { PUNCH_LUNGE } else { STAB_LUNGE } * if self.grounded { 1.0 } else { 0.5 };
            w.add_velocity(self.particle_ids(), Vec2::new(dmath::dir(aim_ang).x * lunge, 0.0));
        }
        let mut diff = dmath::angle_diff(want_ang, self.hand_ang);
        if spin != 0.0 && diff * spin < -0.2 {
            diff += spin * core::f32::consts::TAU; // swing the long way round, as told
        }
        let step = omega * heavy * dt;
        self.hand_ang = dmath::wrap_angle(self.hand_ang + diff.clamp(-step, step));
        self.hand_reach += (want_reach - self.hand_reach).clamp(-reach_rate * dt, reach_rate * dt);
        self.relax(w, S_FORE_A);
        // hanging: the hand on the blade just holds on, a bare fist hangs
        // loose, a weapon is held out lightly (it doesn't stiffen the body)
        let arm_limp = limp && (fists || self.hang_blade);
        let w2 = if limp { w2 * 0.35 } else { w2 };
        if self.arm_a_ok() && !arm_limp {
            let m = 0.4 + load;
            // (the other fist is punching: this one comes back up to guard)
            let offset = if punching_b { GUARD * Vec2::new(f, 1.0) * ak } else { dmath::dir(self.hand_ang) * self.hand_reach };
            // the handle of a stuck weapon: pulling away from it sideways (to
            // work it loose) strains gently instead of hauling you after it;
            // with it above you the arm hauls you up onto it (climbing)
            let hand = w.particles[self.p(HAND_A) as usize].pos;
            let strain = hand - (neck + offset);
            let yanking = self.stuck_grip && self.grounded && strain.y < strain.x.abs();
            let (k, step) = if yanking { (0.3, 0.006) } else { (1.0, 0.03) };
            // (a shot's kick: the arm gives, then brings the gun back down)
            let k = k * (1.0 - 0.85 * self.recoil as f32 / RECOIL_FRAMES as f32);
            self.set_drive(w, self.drive_a, offset, w2 * m * strength.max(0.3) * k, m, step, 1.0);
            if self.swing_thrust && self.swing_t().is_some_and(|t| t < 0.3) {
                // drawing back for a stab: let the elbow fold
                self.relax(w, S_UPPER_A);
            } else {
                // gentle elbow preference so the arm doesn't fold backwards
                self.muscle(w, S_UPPER_A, self.hand_ang - f * 0.35, 1.2e-3 * weak);
            }
        } else {
            w.drives[self.drive_a as usize].enabled = false;
            self.relax(w, S_UPPER_A);
        }

        // ---- free hand (B): reach when grabbing, hold the 2nd grip, or dangle ----
        w.drives[self.drive_b as usize].enabled = false;
        let holding_victim = self.script.is_some_and(|sc| sc.hold) && self.grabbing;
        if holding_victim && self.arm_b_ok() {
            // a finisher: hold them up by the neck at arm's length
            let at = match self.script {
                Some(sc) if sc.high => Vec2::new(f * 0.25, 0.85),
                Some(sc) if sc.kneel => Vec2::new(f * 0.48, -0.45),
                Some(sc) if sc.shove => Vec2::new(f * 0.62, -0.05),
                _ => Vec2::new(f * 0.5, 0.0),
            } * ak;
            self.set_drive(w, self.drive_b, at, 900.0 * 2.0, 2.0, 0.03, 1.0);
            self.relax(w, S_UPPER_B);
            self.relax(w, S_FORE_B);
        } else if !self.arm_b_ok() {
            self.relax(w, S_UPPER_B);
            self.relax(w, S_FORE_B);
        } else if self.grabbing && self.hold.is_some() {
            // holding someone: drag them where the mouse is; a throw winds
            // back over the shoulder, then hurls them along the aim
            let k = 1700.0 * strength.max(0.35);
            let at = match self.heave {
                _ if self.shoulder > 0 => self.shoulder_hand() * ak,
                0 => dmath::dir(aim_ang) * self.aim.length().clamp(0.3 * ak, full),
                h if h > HEAVE_FRAMES / 3 => Vec2::new(-f * 0.45, 0.4) * ak,
                _ => self.aim_dir() * full,
            };
            let snap = if self.shoulder > 0 || (self.heave > 0 && self.heave <= HEAVE_FRAMES / 3) { 0.08 } else { 0.04 };
            self.set_drive(w, self.drive_b, at, k, 2.0, snap, 1.0);
            self.relax(w, S_UPPER_B);
            self.relax(w, S_FORE_B);
        } else if punching_b {
            let offset = dmath::dir(self.hand_ang) * self.hand_reach;
            self.set_drive(w, self.drive_b, offset, w2 * 0.4 * strength.max(0.3), 0.4, 0.03, 1.0);
            self.muscle(w, S_UPPER_B, self.hand_ang - f * 0.35, 1.2e-3 * weak);
        } else if self.wants_grab() && !self.grab_static {
            let reach = self.aim.length().clamp(0.3 * ak, full) * 0.95;
            self.set_drive(w, self.drive_b, dmath::dir(aim_ang) * reach, 700.0 * 0.4 * strength.max(0.3), 0.4, 0.03, 1.0);
            self.relax(w, S_UPPER_B);
            self.relax(w, S_FORE_B);
        } else if two_handed || self.grabbing {
            self.relax(w, S_UPPER_B);
            self.relax(w, S_FORE_B);
        } else if self.weapon.is_none() && self.grounded && self.kick == 0 {
            // bare-handed: the free fist stays up in a guard
            self.set_drive(w, self.drive_b, GUARD * Vec2::new(f, 1.0) * ak, 500.0 * strength, 0.4, 0.03, 1.0);
            self.relax(w, S_UPPER_B);
            self.relax(w, S_FORE_B);
        } else if self.air_t > 25 {
            // flail while falling
            let a = self.air_t as f32 * 0.45;
            self.muscle(w, S_UPPER_B, a, 8e-4);
            self.muscle(w, S_FORE_B, a + 0.8, 1.2e-3);
        } else {
            let swing = if moving { 0.6 * s } else { 0.0 };
            self.muscle(w, S_UPPER_B, down - f * 0.15 + f * swing + tilt, 1e-3 * weak);
            self.muscle(w, S_FORE_B, down + f * 0.45 + f * swing + tilt, 1.5e-3 * weak);
        }
    }
}
