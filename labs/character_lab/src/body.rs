//! Reshapes the game's fighters to a character kit's proportions, so the
//! art can be felt as well as seen. Only the lab does this: the game's own
//! fighters keep their size.
//!
//! The legs keep the game's length (standing, crouching and jumping are tuned
//! to it), and everything else follows the art relative to the legs: torso,
//! arm lengths, head size, and how thick each part is.

use stickfell_physics::{Vec2, World};

use crate::fighter::{
    BONES, Body, ELBOW_A, ELBOW_B, FOOT_A, FOOT_B, Fighter, HAND_A, HAND_B, HEAD, KNEE_A, KNEE_B, NECK, PELVIS, S_FORE_A, S_FORE_B, S_HEAD, S_SHIN_A, S_SHIN_B, S_THIGH_A, S_THIGH_B, S_TORSO, S_UPPER_A,
    S_UPPER_B,
};

/// Bone lengths and radii in metres.
#[derive(Clone, Copy, Debug)]
pub struct Proportions {
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

/// Lengths and radii of one kit part in its own pixels.
#[derive(Clone, Copy, Debug)]
pub struct PartSize {
    pub length: f32,
    pub radius: f32,
}

impl Proportions {
    /// The game's original stick figure (what the poses were tuned on).
    pub const TUNED: Proportions = Proportions {
        torso: 0.6,
        upper_arm: 0.3105,
        forearm: 0.2864,
        thigh: 0.4512,
        shin: 0.4418,
        neck: 0.3007,
        head_r: 0.22,
        torso_r: 0.1,
        upper_arm_r: 0.075,
        forearm_r: 0.075,
        thigh_r: 0.075,
        shin_r: 0.075,
    };

    /// The art's proportions, scaled so its legs match the game's.
    /// `neck` is how far the head's centre sits above the shoulders (px).
    pub fn from_art(head: PartSize, torso: PartSize, upper: PartSize, fore: PartSize, thigh: PartSize, shin: PartSize, neck: f32) -> Self {
        let k = game_leg_length() / (thigh.length + shin.length).max(1.0);
        Self {
            torso: torso.length * k,
            upper_arm: upper.length * k,
            forearm: fore.length * k,
            thigh: thigh.length * k,
            shin: shin.length * k,
            neck: neck * k,
            head_r: head.radius * k,
            torso_r: torso.radius * k,
            upper_arm_r: upper.radius * k,
            forearm_r: fore.radius * k,
            thigh_r: thigh.radius * k,
            shin_r: shin.radius * k,
        }
    }
}

/// Thigh plus shin of a freshly spawned game fighter.
fn game_leg_length() -> f32 {
    let (pelvis, knee, foot) = (Vec2::new(0.0, 0.98), Vec2::new(0.1, 0.54), Vec2::new(0.06, 0.1));
    (knee - pelvis).length() + (foot - knee).length()
}

/// Rebuilds a just-spawned fighter with new proportions: joints move along
/// the directions they already point (so the pose is kept), the bones' rest
/// lengths and the hitboxes' radii change to match, and the fighter's
/// `body` measurements (which its poses scale with) are updated.
pub fn reshape(w: &mut World, f: &mut Fighter, p: &Proportions) {
    f.body = Body { arm: p.upper_arm + p.forearm, torso: p.torso, neck: p.neck, leg: p.thigh + p.shin };
    let at = |w: &World, j: u32| w.particles[(f.base + j) as usize].pos;
    let mut pos = [Vec2::ZERO; 11];
    for (j, slot) in pos.iter_mut().enumerate() {
        *slot = at(w, j as u32);
    }
    let along = |from: Vec2, to: Vec2, len: f32| from + (to - from).normalize_or(Vec2::Y) * len;
    let old = pos;
    pos[KNEE_A as usize] = along(old[PELVIS as usize], old[KNEE_A as usize], p.thigh);
    pos[FOOT_A as usize] = along(pos[KNEE_A as usize], old[FOOT_A as usize] + (pos[KNEE_A as usize] - old[KNEE_A as usize]), p.shin);
    pos[KNEE_B as usize] = along(old[PELVIS as usize], old[KNEE_B as usize], p.thigh);
    pos[FOOT_B as usize] = along(pos[KNEE_B as usize], old[FOOT_B as usize] + (pos[KNEE_B as usize] - old[KNEE_B as usize]), p.shin);
    pos[NECK as usize] = along(old[PELVIS as usize], old[NECK as usize], p.torso);
    let neck = pos[NECK as usize];
    let shift = neck - old[NECK as usize];
    pos[HEAD as usize] = along(neck, old[HEAD as usize] + shift, p.neck);
    pos[ELBOW_A as usize] = along(neck, old[ELBOW_A as usize] + shift, p.upper_arm);
    pos[HAND_A as usize] = along(pos[ELBOW_A as usize], old[HAND_A as usize] + (pos[ELBOW_A as usize] - old[ELBOW_A as usize]), p.forearm);
    pos[ELBOW_B as usize] = along(neck, old[ELBOW_B as usize] + shift, p.upper_arm);
    pos[HAND_B as usize] = along(pos[ELBOW_B as usize], old[HAND_B as usize] + (pos[ELBOW_B as usize] - old[ELBOW_B as usize]), p.forearm);
    // keep the feet on the ground where they were
    let drop = old[FOOT_A as usize].y.min(old[FOOT_B as usize].y) - pos[FOOT_A as usize].y.min(pos[FOOT_B as usize].y);
    for (j, q) in pos.iter().enumerate() {
        let pt = &mut w.particles[(f.base + j as u32) as usize];
        pt.pos = *q + Vec2::new(0.0, drop);
        pt.prev = pt.pos;
    }
    w.particles[(f.base + HEAD) as usize].radius = p.head_r;
    // bones: the nine limb/torso links in BONES order, then neck-head
    for (i, (a, b)) in BONES.iter().skip(1).enumerate() {
        let d = &mut w.distances[(f.dist_base + i as u32) as usize];
        d.rest = (pos[*a as usize] - pos[*b as usize]).length();
    }
    w.distances[(f.dist_base + 9) as usize].rest = p.neck;
    for (part, r) in [
        (S_HEAD, p.head_r),
        (S_TORSO, p.torso_r),
        (S_UPPER_A, p.upper_arm_r),
        (S_FORE_A, p.forearm_r),
        (S_UPPER_B, p.upper_arm_r),
        (S_FORE_B, p.forearm_r),
        (S_THIGH_A, p.thigh_r),
        (S_SHIN_A, p.shin_r),
        (S_THIGH_B, p.thigh_r),
        (S_SHIN_B, p.shin_r),
    ] {
        w.segments[(f.seg_base + part) as usize].radius = r;
    }
}
