//! The particle/constraint world.
//!
//! Small-step XPBD (Macklin et al. 2019): every frame is split into substeps,
//! each doing one predict / project / update-velocity pass. Everything is
//! stored in plain `Vec`s and iterated in a fixed order so the world can be
//! cloned for rollback snapshots and stepped deterministically.

use glam::Vec2;

use crate::collide::{Obb, RayHit, closest_segments};
use crate::dmath;

/// A point mass. Bodies (ragdolls, ropes, sticks) are built from particles
/// joined by constraints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub pos: Vec2,
    pub prev: Vec2,
    pub vel: Vec2,
    /// External force accumulated for the next step; cleared after stepping.
    pub force: Vec2,
    /// 0 = immovable (kinematic anchor).
    pub inv_mass: f32,
    pub radius: f32,
    pub friction: f32,
    pub collide_static: bool,
    /// Highest-y static contact normal touched during the last step (ZERO if none).
    pub ground: Vec2,
}

impl Particle {
    pub fn new(pos: Vec2, mass: f32, radius: f32) -> Self {
        Self {
            pos,
            prev: pos,
            vel: Vec2::ZERO,
            force: Vec2::ZERO,
            inv_mass: if mass > 0.0 { 1.0 / mass } else { 0.0 },
            radius,
            friction: 0.6,
            collide_static: true,
            ground: Vec2::ZERO,
        }
    }

    #[inline]
    pub fn mass(&self) -> f32 {
        if self.inv_mass > 0.0 { 1.0 / self.inv_mass } else { 0.0 }
    }

    #[inline]
    pub fn on_ground(&self) -> bool {
        self.ground.y > 0.5
    }
}

/// Keeps two particles at a fixed distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Distance {
    pub a: u32,
    pub b: u32,
    pub rest: f32,
    pub compliance: f32,
    /// Disabled constraints are skipped (grips, grabs, severed joints).
    pub enabled: bool,
}

/// Relative angle at pivot `b` between the bones b->a and b->c.
/// Optional hard limit range and an optional soft motor toward `target`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hinge {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub limit: bool,
    pub min: f32,
    pub max: f32,
    pub motor: bool,
    pub target: f32,
    pub compliance: f32,
}

/// Joint limit on how far bone b->c may bend away from the line of bone
/// a->b. The bend is 0 when the two bones are straight, positive turning
/// counterclockwise; it's kept within `min..=max`. Unlike [`Hinge`] the
/// angle doesn't wrap at a straight joint, so elbows and knees can sit right
/// at their limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bend {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub min: f32,
    pub max: f32,
    /// 0 is rigid; larger lets the limit give a little.
    pub compliance: f32,
    pub enabled: bool,
}

/// Soft "muscle" that drives the world-space angle of bone a->b to `target`.
///
/// Uses XPBD constraint damping (`damping` = dimensionless gamma) so a stiff
/// muscle settles instead of whipping the bone around, and limits the
/// correction per substep so it can't inject runaway angular velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub a: u32,
    pub b: u32,
    pub target: f32,
    pub compliance: f32,
    pub damping: f32,
    /// Max angle error corrected per substep (radians): caps how fast the
    /// muscle can turn the bone.
    pub max_step: f32,
    pub enabled: bool,
}

/// Soft drive pulling particle `p` toward `pos(anchor) + offset`.
///
/// Solved inside the substep loop like any other XPBD constraint, so it stays
/// stable at high stiffness (an explicit force spring at 60 Hz overshoots and
/// jitters). `reaction` (0..1) is how much of the pull is felt by the anchor,
/// and `max_step` caps the correction per substep, i.e. the drive's top speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    pub anchor: u32,
    pub p: u32,
    pub offset: Vec2,
    pub compliance: f32,
    pub damping: f32,
    pub max_step: f32,
    pub reaction: f32,
    pub enabled: bool,
}

/// Keeps a point on a body segment (`a`-`b` at parameter `s`) on the line of
/// a blade (`base`-`tip`), free to slide along it: a body run through by a
/// sword. The point can't slide back past `u_min` (the hilt); sliding off
/// the tip is left to the game (read `u`). `friction` (0..1) is how much of
/// the sliding per substep is resisted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skewer {
    pub a: u32,
    pub b: u32,
    pub s: f32,
    pub base: u32,
    pub tip: u32,
    pub u_min: f32,
    pub friction: f32,
    /// Where the body sits along the blade after the last step (0 = base, 1 = tip).
    pub u: f32,
    pub enabled: bool,
    /// The blade is fixed in place (stuck in a wall): only the body moves,
    /// so friction holds it exactly where it is on the blade.
    pub anchored: bool,
}

/// A rigid body made of the particles `start..start+count`, kept in its rest
/// shape by 2D shape matching (best-fit rotation about the mass centroid).
/// Rest offsets live in `World::rigid_rest[rest..rest+count]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigid {
    pub start: u32,
    pub count: u32,
    pub rest: u32,
    pub enabled: bool,
}

/// Max overlap a dynamic contact resolves per substep (metres), about 14 m/s
/// of separation speed at 8 substeps.
pub const MAX_DEPENETRATION: f32 = 0.03;

/// Default max angle error a muscle corrects per substep (radians).
pub const AIM_MAX_STEP: f32 = 0.12;

/// Capsule between two particles used for dynamic-vs-dynamic collision.
/// A segment with `a == b` is a circle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: u32,
    pub b: u32,
    pub radius: f32,
    /// Segments sharing a non-zero group never collide with each other.
    pub group: u16,
    pub enabled: bool,
    /// Never collide with segments of this group (0 = none). Lets a blade
    /// pass through the one body it is stuck in.
    pub ignore_group: u16,
    /// Also collide the whole capsule (not just its end particles) with
    /// static geometry, so long blades and limbs can't slip through corners.
    pub collide_static: bool,
    pub tag: u32,
}

/// A dynamic segment-vs-segment contact recorded during the last step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub seg_a: u32,
    pub seg_b: u32,
    pub point: Vec2,
    /// Points from b towards a.
    pub normal: Vec2,
    /// Approach speed along the normal at first touch (>= 0).
    pub impact: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldConfig {
    pub gravity: Vec2,
    pub substeps: u32,
    /// Linear drag per second.
    pub drag: f32,
    pub max_speed: f32,
    /// Friction multiplier for steep static contacts (normal.y < 0.5), so
    /// bodies pressed against walls slide down instead of sticking.
    pub wall_friction: f32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self { gravity: Vec2::new(0.0, -28.0), substeps: 8, drag: 0.15, max_speed: 70.0, wall_friction: 0.15 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct World {
    pub cfg: WorldConfig,
    pub particles: Vec<Particle>,
    pub distances: Vec<Distance>,
    pub hinges: Vec<Hinge>,
    pub bends: Vec<Bend>,
    pub aims: Vec<Aim>,
    pub drives: Vec<Drive>,
    pub skewers: Vec<Skewer>,
    pub rigids: Vec<Rigid>,
    pub rigid_rest: Vec<Vec2>,
    pub segments: Vec<Segment>,
    pub statics: Vec<Obb>,
    pub contacts: Vec<Contact>,
    /// Per-substep segment bounds (broadphase scratch, not simulation state).
    seg_bounds: Vec<[f32; 4]>,
}

#[inline]
fn grad_angle(u: Vec2) -> Vec2 {
    // d(angle of u)/du = perp(u) / |u|^2
    let l2 = u.length_squared().max(1e-8);
    u.perp() / l2
}

impl World {
    pub fn new(cfg: WorldConfig) -> Self {
        Self { cfg, ..Default::default() }
    }

    // ---- building -------------------------------------------------------

    pub fn add_particle(&mut self, p: Particle) -> u32 {
        self.particles.push(p);
        (self.particles.len() - 1) as u32
    }

    /// Rigid distance using the particles' current separation.
    pub fn add_distance(&mut self, a: u32, b: u32, compliance: f32) -> u32 {
        let rest = (self.particles[a as usize].pos - self.particles[b as usize].pos).length();
        self.distances.push(Distance { a, b, rest, compliance, enabled: true });
        (self.distances.len() - 1) as u32
    }

    /// Adds a disabled zero-length constraint slot to be pointed at particles
    /// later (grips, grabs). Enable it by setting `a`, `b` and `enabled`.
    pub fn add_pin_slot(&mut self, compliance: f32) -> u32 {
        self.distances.push(Distance { a: 0, b: 0, rest: 0.0, compliance, enabled: false });
        (self.distances.len() - 1) as u32
    }

    /// Freezes a particle out of the simulation (no motion, no collisions).
    pub fn park_particle(&mut self, i: u32) {
        let p = &mut self.particles[i as usize];
        p.inv_mass = 0.0;
        p.collide_static = false;
        p.vel = Vec2::ZERO;
        p.pos = Vec2::new(0.0, -1000.0);
        p.prev = p.pos;
    }

    pub fn add_hinge(&mut self, h: Hinge) -> u32 {
        self.hinges.push(h);
        (self.hinges.len() - 1) as u32
    }

    pub fn add_bend(&mut self, b: Bend) -> u32 {
        self.bends.push(b);
        (self.bends.len() - 1) as u32
    }

    pub fn add_aim(&mut self, a: u32, b: u32, compliance: f32) -> u32 {
        let target = dmath::angle(self.particles[b as usize].pos - self.particles[a as usize].pos);
        self.aims.push(Aim { a, b, target, compliance, damping: 1.0, max_step: AIM_MAX_STEP, enabled: true });
        (self.aims.len() - 1) as u32
    }

    /// Adds a disabled drive of `p` toward `anchor`; configure it each frame.
    pub fn add_drive(&mut self, anchor: u32, p: u32) -> u32 {
        self.drives.push(Drive {
            anchor,
            p,
            offset: Vec2::ZERO,
            compliance: 1e-3,
            damping: 1.0,
            max_step: 0.05,
            reaction: 0.0,
            enabled: false,
        });
        (self.drives.len() - 1) as u32
    }

    /// Adds a disabled skewer slot; fill it in and enable it when something gets run through.
    pub fn add_skewer_slot(&mut self) -> u32 {
        self.skewers.push(Skewer { a: 0, b: 0, s: 0.0, base: 0, tip: 0, u_min: 0.0, friction: 0.0, u: 0.0, enabled: false, anchored: false });
        (self.skewers.len() - 1) as u32
    }

    /// Makes particles `start..start+count` one rigid body in their current shape.
    pub fn add_rigid(&mut self, start: u32, count: u32) -> u32 {
        let ids = start..start + count;
        let c = self.center_of_mass(ids.clone());
        let rest = self.rigid_rest.len() as u32;
        for i in ids {
            self.rigid_rest.push(self.particles[i as usize].pos - c);
        }
        self.rigids.push(Rigid { start, count, rest, enabled: true });
        (self.rigids.len() - 1) as u32
    }

    pub fn add_segment(&mut self, a: u32, b: u32, radius: f32, group: u16, tag: u32) -> u32 {
        self.segments.push(Segment { a, b, radius, group, ignore_group: 0, enabled: true, collide_static: false, tag });
        (self.segments.len() - 1) as u32
    }

    pub fn add_static(&mut self, o: Obb) -> u32 {
        self.statics.push(o);
        (self.statics.len() - 1) as u32
    }

    // ---- queries --------------------------------------------------------

    /// Nearest static hit along `origin + dir * t`, t in [0, max_t].
    pub fn raycast(&self, origin: Vec2, dir: Vec2, max_t: f32) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        for (i, s) in self.statics.iter().enumerate() {
            let limit = best.map_or(max_t, |b| b.t);
            if let Some((t, n)) = s.raycast(origin, dir, limit) {
                if best.is_none_or(|b| t < b.t) {
                    best = Some(RayHit { t, point: origin + dir * t, normal: n, index: i });
                }
            }
        }
        best
    }

    /// Velocity of a point on a segment at parameter `s`.
    pub fn segment_point_vel(&self, seg: u32, s: f32) -> Vec2 {
        let sg = &self.segments[seg as usize];
        let va = self.particles[sg.a as usize].vel;
        let vb = self.particles[sg.b as usize].vel;
        va * (1.0 - s) + vb * s
    }

    // ---- stepping -------------------------------------------------------

    pub fn step(&mut self, dt: f32) {
        let n = self.cfg.substeps.max(1);
        let h = dt / n as f32;
        self.contacts.clear();
        for p in &mut self.particles {
            p.ground = Vec2::ZERO;
        }
        let damp = (1.0 - self.cfg.drag * h).max(0.0);
        let max_v2 = self.cfg.max_speed * self.cfg.max_speed;
        for _ in 0..n {
            for p in &mut self.particles {
                p.prev = p.pos;
                if p.inv_mass == 0.0 {
                    continue;
                }
                p.vel += (self.cfg.gravity + p.force * p.inv_mass) * h;
                p.vel *= damp;
                let v2 = p.vel.length_squared();
                if v2 > max_v2 {
                    p.vel *= self.cfg.max_speed / v2.sqrt();
                }
                p.pos += p.vel * h;
            }
            self.solve_distances(h);
            self.solve_hinges(h);
            self.solve_aims(h);
            self.solve_drives(h);
            self.solve_bends(h);
            self.solve_skewers();
            self.solve_rigids();
            self.solve_segments();
            self.solve_statics();
            self.solve_segment_statics();
            let inv_h = 1.0 / h;
            for p in &mut self.particles {
                if p.inv_mass != 0.0 {
                    p.vel = (p.pos - p.prev) * inv_h;
                }
            }
        }
        for p in &mut self.particles {
            p.force = Vec2::ZERO;
        }
    }

    fn solve_distances(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for c in &self.distances {
            if !c.enabled {
                continue;
            }
            let (ia, ib) = (c.a as usize, c.b as usize);
            let (pa, pb) = (self.particles[ia], self.particles[ib]);
            let w = pa.inv_mass + pb.inv_mass;
            if w == 0.0 {
                continue;
            }
            let d = pa.pos - pb.pos;
            let len = d.length();
            if len < 1e-9 {
                continue;
            }
            let n = d / len;
            let cval = len - c.rest;
            let dl = -cval / (w + c.compliance * inv_h2);
            self.particles[ia].pos += n * (dl * pa.inv_mass);
            self.particles[ib].pos -= n * (dl * pb.inv_mass);
        }
    }

    /// Generic 3-particle angular solve: C with gradients ga, gb, gc.
    fn apply3(&mut self, ids: [usize; 3], g: [Vec2; 3], c: f32, alpha_t: f32) {
        let mut denom = alpha_t;
        for k in 0..3 {
            denom += self.particles[ids[k]].inv_mass * g[k].length_squared();
        }
        if denom < 1e-12 {
            return;
        }
        let dl = -c / denom;
        for k in 0..3 {
            let w = self.particles[ids[k]].inv_mass;
            self.particles[ids[k]].pos += g[k] * (w * dl);
        }
    }

    fn solve_hinges(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for i in 0..self.hinges.len() {
            let hg = self.hinges[i];
            let (ia, ib, ic) = (hg.a as usize, hg.b as usize, hg.c as usize);
            let pb = self.particles[ib].pos;
            let u = self.particles[ia].pos - pb;
            let v = self.particles[ic].pos - pb;
            let theta = dmath::atan2(u.perp_dot(v), u.dot(v));
            let ga = -grad_angle(u);
            let gc = grad_angle(v);
            let gb = -(ga + gc);
            if hg.limit {
                let err = if theta < hg.min {
                    theta - hg.min
                } else if theta > hg.max {
                    theta - hg.max
                } else {
                    0.0
                };
                if err != 0.0 {
                    self.apply3([ia, ib, ic], [ga, gb, gc], err.clamp(-0.5, 0.5), 0.0);
                }
            }
            if hg.motor {
                let err = dmath::angle_diff(theta, hg.target).clamp(-0.5, 0.5);
                self.apply3([ia, ib, ic], [ga, gb, gc], err, hg.compliance * inv_h2);
            }
        }
    }

    fn solve_bends(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for i in 0..self.bends.len() {
            let bd = self.bends[i];
            if !bd.enabled {
                continue;
            }
            let (ia, ib, ic) = (bd.a as usize, bd.b as usize, bd.c as usize);
            let pb = self.particles[ib].pos;
            let u = self.particles[ia].pos - pb;
            let v = self.particles[ic].pos - pb;
            // bone a->b is -u: the bend from it to v, 0 when straight
            let bend = dmath::atan2(-u.perp_dot(v), -u.dot(v));
            let err = if bend < bd.min {
                bend - bd.min
            } else if bend > bd.max {
                bend - bd.max
            } else {
                continue;
            };
            // (the bend differs from the hinge angle by a constant, so the gradients are the same)
            let ga = -grad_angle(u);
            let gc = grad_angle(v);
            let gb = -(ga + gc);
            self.apply3([ia, ib, ic], [ga, gb, gc], err.clamp(-0.5, 0.5), bd.compliance * inv_h2);
        }
    }

    fn solve_aims(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for i in 0..self.aims.len() {
            let am = self.aims[i];
            if !am.enabled {
                continue;
            }
            let (ia, ib) = (am.a as usize, am.b as usize);
            let (pa, pb) = (self.particles[ia], self.particles[ib]);
            let (wa, wb) = (pa.inv_mass, pb.inv_mass);
            let u = pb.pos - pa.pos;
            let phi = dmath::angle(u);
            let err = dmath::angle_diff(phi, am.target).clamp(-am.max_step, am.max_step);
            // angular change of the bone during this substep (for damping)
            let dphi = dmath::angle_diff(phi, dmath::angle(pb.prev - pa.prev));
            let gb = grad_angle(u);
            let gamma = am.damping;
            let denom = (1.0 + gamma) * (wa + wb) * gb.length_squared() + am.compliance * inv_h2;
            if denom < 1e-12 {
                continue;
            }
            let dl = -(err + gamma * dphi) / denom;
            self.particles[ib].pos += gb * (wb * dl);
            self.particles[ia].pos -= gb * (wa * dl);
        }
    }

    fn solve_drives(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for i in 0..self.drives.len() {
            let d = self.drives[i];
            if !d.enabled {
                continue;
            }
            let (ia, ip) = (d.anchor as usize, d.p as usize);
            let (pa, pp) = (self.particles[ia], self.particles[ip]);
            let wa = pa.inv_mass * d.reaction;
            let w = pp.inv_mass + wa;
            if w == 0.0 {
                continue;
            }
            let err = pp.pos - (pa.pos + d.offset);
            // relative motion during this substep (for damping)
            let rel = (pp.pos - pp.prev) - (pa.pos - pa.prev);
            let gamma = d.damping;
            let mut corr = -(err + rel * gamma) / ((1.0 + gamma) * w + d.compliance * inv_h2);
            let cl = corr.length() * pp.inv_mass;
            if cl > d.max_step {
                corr *= d.max_step / cl;
            }
            self.particles[ip].pos += corr * pp.inv_mass;
            self.particles[ia].pos -= corr * wa;
        }
    }

    fn solve_skewers(&mut self) {
        for i in 0..self.skewers.len() {
            let sk = self.skewers[i];
            if !sk.enabled {
                continue;
            }
            let ids = [sk.a as usize, sk.b as usize, sk.base as usize, sk.tip as usize];
            let mut ps = ids.map(|k| self.particles[k]);
            if sk.anchored {
                ps[2].inv_mass = 0.0;
                ps[3].inv_mass = 0.0;
            }
            let point = |q: [Vec2; 4]| q[0] * (1.0 - sk.s) + q[1] * sk.s;
            let p = point(ps.map(|q| q.pos));
            let (w0, w1) = (ps[2].pos, ps[3].pos);
            let ax = w1 - w0;
            let l2 = ax.length_squared();
            if l2 < 1e-8 {
                continue;
            }
            let u = (p - w0).dot(ax) / l2;
            self.skewers[i].u = u;
            let uc = u.max(sk.u_min);
            let ks = [1.0 - sk.s, sk.s, -(1.0 - uc), -uc];
            let denom: f32 = (0..4).map(|k| ps[k].inv_mass * ks[k] * ks[k]).sum();
            if denom < 1e-12 {
                continue;
            }
            // pull the body point onto the blade line
            let d = p - (w0 + ax * uc);
            let dist = d.length();
            if dist > 1e-7 {
                let n = d / dist;
                let dl = -dist / denom;
                for k in 0..4 {
                    self.particles[ids[k]].pos += n * (dl * ps[k].inv_mass * ks[k]);
                }
            }
            // friction against sliding along the blade
            let axn = ax / l2.sqrt();
            let now = ids.map(|k| self.particles[k].pos);
            let prev = ps.map(|q| q.prev);
            let moved = (point(now) - point(prev)) - ((now[2] - prev[2]) * (1.0 - uc) + (now[3] - prev[3]) * uc);
            let slip = moved.dot(axn);
            let dl = -slip * sk.friction / denom;
            for k in 0..4 {
                self.particles[ids[k]].pos += axn * (dl * ps[k].inv_mass * ks[k]);
            }
        }
    }

    fn solve_rigids(&mut self) {
        for r in 0..self.rigids.len() {
            let rg = self.rigids[r];
            if !rg.enabled {
                continue;
            }
            let (s, n) = (rg.start as usize, rg.count as usize);
            let mut m = 0.0;
            let mut c = Vec2::ZERO;
            for p in &self.particles[s..s + n] {
                let pm = p.mass();
                c += p.pos * pm;
                m += pm;
            }
            if m <= 0.0 {
                continue;
            }
            c /= m;
            let (mut dot, mut cross) = (0.0, 0.0);
            for k in 0..n {
                let p = &self.particles[s + k];
                let q = self.rigid_rest[rg.rest as usize + k];
                let d = p.pos - c;
                let pm = p.mass();
                dot += q.dot(d) * pm;
                cross += q.perp_dot(d) * pm;
            }
            let (sn, cs) = dmath::sin_cos(dmath::atan2(cross, dot));
            for k in 0..n {
                let q = self.rigid_rest[rg.rest as usize + k];
                let p = &mut self.particles[s + k];
                if p.inv_mass > 0.0 {
                    p.pos = c + Vec2::new(q.x * cs - q.y * sn, q.x * sn + q.y * cs);
                }
            }
        }
    }

    fn solve_segments(&mut self) {
        let ns = self.segments.len();
        // broadphase: AABB of every segment, inflated by its radius
        self.seg_bounds.clear();
        for sg in &self.segments {
            let (a, b) = (self.particles[sg.a as usize].pos, self.particles[sg.b as usize].pos);
            let r = sg.radius;
            self.seg_bounds.push([a.x.min(b.x) - r, a.y.min(b.y) - r, a.x.max(b.x) + r, a.y.max(b.y) + r]);
        }
        for i in 0..ns {
            let si = self.segments[i];
            if !si.enabled {
                continue;
            }
            let bi = self.seg_bounds[i];
            for j in (i + 1)..ns {
                let bj = self.seg_bounds[j];
                if bi[0] > bj[2] || bj[0] > bi[2] || bi[1] > bj[3] || bj[1] > bi[3] {
                    continue;
                }
                let sj = self.segments[j];
                if !sj.enabled || (si.group != 0 && si.group == sj.group) {
                    continue;
                }
                if (si.ignore_group != 0 && si.ignore_group == sj.group) || (sj.ignore_group != 0 && sj.ignore_group == si.group) {
                    continue;
                }
                if si.a == sj.a || si.a == sj.b || si.b == sj.a || si.b == sj.b {
                    continue;
                }
                let (a1, b1) = (self.particles[si.a as usize], self.particles[si.b as usize]);
                let (a2, b2) = (self.particles[sj.a as usize], self.particles[sj.b as usize]);
                let r = si.radius + sj.radius;
                let (s, t, c1, c2) = closest_segments(a1.pos, b1.pos, a2.pos, b2.pos);
                let d = c1 - c2;
                let d2 = d.length_squared();
                if d2 >= r * r || d2 < 1e-12 {
                    continue;
                }
                let dist = d2.sqrt();
                let n = d / dist;
                let cval = dist - r;
                let one_seg_i = si.a == si.b;
                let one_seg_j = sj.a == sj.b;
                // gradients for the 4 particle slots
                let (wi_a, wi_b) = if one_seg_i { (1.0, 0.0) } else { (1.0 - s, s) };
                let (wj_a, wj_b) = if one_seg_j { (1.0, 0.0) } else { (1.0 - t, t) };
                let denom = a1.inv_mass * wi_a * wi_a
                    + b1.inv_mass * wi_b * wi_b
                    + a2.inv_mass * wj_a * wj_a
                    + b2.inv_mass * wj_b * wj_b;
                if denom < 1e-12 {
                    continue;
                }
                // approach speed (velocities from the previous substep)
                let v1 = a1.vel * wi_a + b1.vel * wi_b;
                let v2 = a2.vel * wj_a + b2.vel * wj_b;
                let impact = (-(v1 - v2).dot(n)).max(0.0);
                // resolve deep overlaps over several substeps instead of all at
                // once, so a bad overlap separates at a sane speed instead of
                // launching things
                let dl = -cval.max(-MAX_DEPENETRATION) / denom;
                self.particles[si.a as usize].pos += n * (a1.inv_mass * wi_a * dl);
                if !one_seg_i {
                    self.particles[si.b as usize].pos += n * (b1.inv_mass * wi_b * dl);
                }
                self.particles[sj.a as usize].pos -= n * (a2.inv_mass * wj_a * dl);
                if !one_seg_j {
                    self.particles[sj.b as usize].pos -= n * (b2.inv_mass * wj_b * dl);
                }
                self.record_contact(i as u32, j as u32, (c1 + c2) * 0.5, n, impact);
            }
        }
    }

    fn record_contact(&mut self, a: u32, b: u32, point: Vec2, normal: Vec2, impact: f32) {
        for c in &mut self.contacts {
            if c.seg_a == a && c.seg_b == b {
                if impact > c.impact {
                    c.impact = impact;
                    c.point = point;
                    c.normal = normal;
                }
                return;
            }
        }
        self.contacts.push(Contact { seg_a: a, seg_b: b, point, normal, impact });
    }

    fn solve_statics(&mut self) {
        let wall_friction = self.cfg.wall_friction;
        for p in &mut self.particles {
            if p.inv_mass == 0.0 || !p.collide_static {
                continue;
            }
            for s in &self.statics {
                let Some((n, pen)) = s.collide_circle(p.pos, p.radius) else {
                    continue;
                };
                p.pos += n * pen;
                // Positional Coulomb friction against the tangential motion of this substep.
                let disp = p.pos - p.prev;
                let tang = disp - n * disp.dot(n);
                let tl = tang.length();
                if tl > 1e-9 {
                    let mut mu = (p.friction * s.friction).sqrt();
                    if n.y < 0.5 {
                        mu *= wall_friction;
                    }
                    let max_fric = mu * pen;
                    if tl <= max_fric {
                        p.pos -= tang;
                    } else {
                        p.pos -= tang * (max_fric / tl);
                    }
                }
                if n.y > p.ground.y {
                    p.ground = n;
                }
            }
        }
    }

    /// Capsule-vs-box for segments flagged `collide_static`. Candidate points
    /// along the capsule (ends, middle, and the spots nearest the box's
    /// corners and center) are tested and the deepest one is pushed out,
    /// split between the two end particles like a lever.
    fn solve_segment_statics(&mut self) {
        let wall_friction = self.cfg.wall_friction;
        for si in 0..self.segments.len() {
            let sg = self.segments[si];
            if !sg.enabled || !sg.collide_static || sg.a == sg.b {
                continue;
            }
            let (ia, ib) = (sg.a as usize, sg.b as usize);
            let r = sg.radius;
            for oi in 0..self.statics.len() {
                let (pa, pb) = (self.particles[ia], self.particles[ib]);
                let (wa0, wb0) = (pa.inv_mass, pb.inv_mass);
                if wa0 + wb0 == 0.0 {
                    break;
                }
                let o = self.statics[oi];
                let (d, _) = crate::collide::point_segment(o.center, pa.pos, pb.pos);
                if d > o.bound_radius() + r {
                    continue;
                }
                let ab = pb.pos - pa.pos;
                let l2 = ab.length_squared().max(1e-12);
                let mut ts = [0.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0];
                let corners = o.corners();
                for (k, cpt) in corners.iter().chain(core::iter::once(&o.center)).enumerate() {
                    ts[3 + k] = ((*cpt - pa.pos).dot(ab) / l2).clamp(0.0, 1.0);
                }
                let mut best: Option<(f32, Vec2, f32)> = None;
                for &t in &ts {
                    if let Some((n, pen)) = o.collide_circle(pa.pos + ab * t, r) {
                        if best.is_none_or(|(_, _, bp)| pen > bp) {
                            best = Some((t, n, pen));
                        }
                    }
                }
                let Some((t, n, pen)) = best else { continue };
                let (ka, kb) = (1.0 - t, t);
                let denom = wa0 * ka * ka + wb0 * kb * kb;
                if denom < 1e-12 {
                    continue;
                }
                let dl = pen / denom;
                self.particles[ia].pos += n * (dl * wa0 * ka);
                self.particles[ib].pos += n * (dl * wb0 * kb);
                // friction against this substep's tangential motion of the contact point
                let (pa, pb) = (self.particles[ia], self.particles[ib]);
                let disp = (pa.pos - pa.prev) * ka + (pb.pos - pb.prev) * kb;
                let tang = disp - n * disp.dot(n);
                let tl = tang.length();
                if tl > 1e-9 {
                    let mut mu = ((pa.friction * ka + pb.friction * kb) * o.friction).sqrt();
                    if n.y < 0.5 {
                        mu *= wall_friction;
                    }
                    let k = (mu * pen / tl).min(1.0) / denom;
                    self.particles[ia].pos -= tang * (k * wa0 * ka);
                    self.particles[ib].pos -= tang * (k * wb0 * kb);
                }
                let gi = if t < 0.5 { ia } else { ib };
                if n.y > self.particles[gi].ground.y {
                    self.particles[gi].ground = n;
                }
            }
        }
    }

    // ---- helpers for game code -----------------------------------------

    /// Adds a velocity change to a set of particles.
    pub fn add_velocity(&mut self, ids: impl IntoIterator<Item = u32>, dv: Vec2) {
        for i in ids {
            let p = &mut self.particles[i as usize];
            if p.inv_mass > 0.0 {
                p.vel += dv;
            }
        }
    }

    /// Mass-weighted center of a set of particles.
    pub fn center_of_mass(&self, ids: impl IntoIterator<Item = u32>) -> Vec2 {
        let mut sum = Vec2::ZERO;
        let mut m = 0.0;
        for i in ids {
            let p = &self.particles[i as usize];
            let pm = p.mass();
            sum += p.pos * pm;
            m += pm;
        }
        if m > 0.0 { sum / m } else { Vec2::ZERO }
    }

    /// Feeds the dynamic state into a hasher (for desync detection).
    pub fn hash_into(&self, mut write: impl FnMut(u32)) {
        for p in &self.particles {
            write(p.pos.x.to_bits());
            write(p.pos.y.to_bits());
            write(p.vel.x.to_bits());
            write(p.vel.y.to_bits());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_world() -> World {
        let mut w = World::new(WorldConfig::default());
        w.add_static(Obb::from_min_max(Vec2::new(-10.0, -1.0), Vec2::new(10.0, 0.0)));
        let mut prev = w.add_particle(Particle::new(Vec2::new(0.0, 3.0), 1.0, 0.1));
        for i in 1..6 {
            let p = w.add_particle(Particle::new(Vec2::new(0.3 * i as f32, 3.0), 1.0, 0.1));
            w.add_distance(prev, p, 0.0);
            w.add_segment(prev, p, 0.1, 0, 0);
            prev = p;
        }
        w
    }

    #[test]
    fn chain_falls_and_rests_on_floor() {
        let mut w = chain_world();
        for _ in 0..240 {
            w.step(1.0 / 60.0);
        }
        for p in &w.particles {
            assert!(p.pos.y > 0.05 && p.pos.y < 0.2, "particle at {:?}", p.pos);
            assert!(p.vel.length() < 0.5);
        }
        // constraints held
        for d in &w.distances {
            let l = (w.particles[d.a as usize].pos - w.particles[d.b as usize].pos).length();
            assert!((l - d.rest).abs() < 0.01);
        }
    }

    #[test]
    fn stepping_is_deterministic_across_clones() {
        let mut a = chain_world();
        for _ in 0..30 {
            a.step(1.0 / 60.0);
        }
        let mut b = a.clone();
        for _ in 0..90 {
            a.step(1.0 / 60.0);
            b.step(1.0 / 60.0);
        }
        assert_eq!(a.particles, b.particles);
    }
}

#[cfg(test)]
mod aim_momentum {
    use super::*;
    #[test]
    fn aim_conserves_momentum() {
        let mut w = World::new(WorldConfig { gravity: Vec2::ZERO, drag: 0.0, ..Default::default() });
        let a = w.add_particle(Particle::new(Vec2::new(0.0, 1.0), 2.5, 0.1));
        let b = w.add_particle(Particle::new(Vec2::new(0.3, 0.6), 0.6, 0.1));
        w.add_distance(a, b, 0.0);
        let m = w.add_aim(a, b, 1e-5);
        w.aims[m as usize].target = 2.0;
        for p in &mut w.particles { p.vel = Vec2::new(0.0, 10.0); }
        let mom = |w: &World| w.particles.iter().map(|p| p.vel * p.mass()).sum::<Vec2>();
        let before = mom(&w);
        w.step(1.0 / 60.0);
        let after = mom(&w);
        println!("before {before:?} after {after:?}");
        assert!((before - after).length() < 1e-2);
    }
}

#[cfg(test)]
mod bend_limits {
    use super::*;

    fn bend_of(w: &World) -> f32 {
        let (a, b, c) = (w.particles[0].pos, w.particles[1].pos, w.particles[2].pos);
        let (d1, d2) = (b - a, c - b);
        dmath::atan2(d1.perp_dot(d2), d1.dot(d2))
    }

    #[test]
    fn a_bend_past_its_limit_is_pushed_back_and_straight_is_fine() {
        let mut w = World::new(WorldConfig { gravity: Vec2::ZERO, ..Default::default() });
        for p in [Vec2::new(0.0, 1.0), Vec2::ZERO, Vec2::new(1.0, 0.0)] {
            w.add_particle(Particle::new(p, 1.0, 0.05));
        }
        w.add_distance(0, 1, 0.0);
        w.add_distance(1, 2, 0.0);
        assert!((bend_of(&w) - core::f32::consts::FRAC_PI_2).abs() < 1e-4, "starts bent a quarter turn counterclockwise");
        w.add_bend(Bend { a: 0, b: 1, c: 2, min: -0.2, max: 0.3, compliance: 0.0, enabled: true });
        for _ in 0..30 {
            w.step(1.0 / 60.0);
        }
        let bend = bend_of(&w);
        assert!(bend <= 0.32 && bend >= -0.22, "bend {bend} should be back inside -0.2..0.3");

        // a straight joint inside the range isn't disturbed (no wrap at straight)
        let mut w2 = World::new(WorldConfig { gravity: Vec2::ZERO, ..Default::default() });
        for p in [Vec2::new(0.0, 1.0), Vec2::ZERO, Vec2::new(0.0, -1.0)] {
            w2.add_particle(Particle::new(p, 1.0, 0.05));
        }
        w2.add_distance(0, 1, 0.0);
        w2.add_distance(1, 2, 0.0);
        w2.add_bend(Bend { a: 0, b: 1, c: 2, min: -0.05, max: 2.6, compliance: 0.0, enabled: true });
        let before = w2.particles[2].pos;
        for _ in 0..30 {
            w2.step(1.0 / 60.0);
        }
        assert!((w2.particles[2].pos - before).length() < 1e-4);
    }
}
