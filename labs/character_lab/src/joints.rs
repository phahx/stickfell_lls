//! Measuring the joint limits (the game applies them: `fighter::JOINTS`).

use stickfell_physics::World;

use crate::fighter::{Fighter, JOINTS, S_TORSO};

pub const COUNT: usize = JOINTS.len();

/// How far (radians) each joint is outside its realistic range right now,
/// whether or not the limits are on (for comparing).
pub fn each(w: &World, f: &Fighter) -> [f32; COUNT] {
    let mut out = [0.0; COUNT];
    for (o, j) in out.iter_mut().zip(&JOINTS) {
        if (f.spine_out && j.parts.contains(&S_TORSO)) || !j.parts.iter().all(|&p| f.has(p)) {
            continue;
        }
        let [a, b, c] = j.p.map(|p| w.particles[(f.base + p) as usize].pos);
        let (d1, d2) = (b - a, c - b);
        let bend = stickfell_physics::dmath::atan2(d1.perp_dot(d2), d1.dot(d2));
        let (lo, hi) = if j.mirrors && f.facing < 0.0 { (-j.range.1, -j.range.0) } else { j.range };
        *o = (lo - bend).max(bend - hi).max(0.0);
    }
    out
}

/// The worst of [`each`].
pub fn worst(w: &World, f: &Fighter) -> f32 {
    each(w, f).into_iter().fold(0.0, f32::max)
}
