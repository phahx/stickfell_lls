//! Static collision shapes and geometric queries.

use glam::Vec2;

use crate::dmath;

/// Oriented box used for static level geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub center: Vec2,
    pub half: Vec2,
    /// (cos, sin) of the rotation.
    pub rot: Vec2,
    pub friction: f32,
    /// Free-form user tag (material, gameplay meaning, ...).
    pub tag: u32,
}

/// Result of a ray cast against static geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub t: f32,
    pub point: Vec2,
    pub normal: Vec2,
    /// Index of the static shape that was hit.
    pub index: usize,
}

impl Obb {
    pub fn new(center: Vec2, half: Vec2, angle: f32) -> Self {
        let (s, c) = dmath::sin_cos(angle);
        Self { center, half, rot: Vec2::new(c, s), friction: 0.8, tag: 0 }
    }

    /// Axis-aligned box from min/max corners.
    pub fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self::new((min + max) * 0.5, (max - min) * 0.5, 0.0)
    }

    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    pub fn with_tag(mut self, tag: u32) -> Self {
        self.tag = tag;
        self
    }

    #[inline]
    pub fn axis_x(&self) -> Vec2 {
        self.rot
    }

    #[inline]
    pub fn axis_y(&self) -> Vec2 {
        self.rot.perp()
    }

    #[inline]
    pub fn to_local(&self, p: Vec2) -> Vec2 {
        let d = p - self.center;
        Vec2::new(d.dot(self.axis_x()), d.dot(self.axis_y()))
    }

    #[inline]
    pub fn dir_to_world(&self, v: Vec2) -> Vec2 {
        self.axis_x() * v.x + self.axis_y() * v.y
    }

    #[inline]
    pub fn dir_to_local(&self, v: Vec2) -> Vec2 {
        Vec2::new(v.dot(self.axis_x()), v.dot(self.axis_y()))
    }

    /// Corners in counter-clockwise order.
    pub fn corners(&self) -> [Vec2; 4] {
        let ax = self.axis_x() * self.half.x;
        let ay = self.axis_y() * self.half.y;
        [
            self.center - ax - ay,
            self.center + ax - ay,
            self.center + ax + ay,
            self.center - ax + ay,
        ]
    }

    /// Conservative bounding radius from the center.
    #[inline]
    pub fn bound_radius(&self) -> f32 {
        self.half.length()
    }

    /// Circle overlap test. Returns the push-out normal (world space, pointing
    /// out of the box) and penetration depth.
    pub fn collide_circle(&self, p: Vec2, r: f32) -> Option<(Vec2, f32)> {
        let d = p - self.center;
        let reach = self.bound_radius() + r;
        if d.length_squared() > reach * reach {
            return None;
        }
        let l = self.to_local(p);
        let cl = l.clamp(-self.half, self.half);
        if cl == l {
            // Center is inside: exit through the nearest face.
            let dx = self.half.x - l.x.abs();
            let dy = self.half.y - l.y.abs();
            let (n, pen) = if dx < dy {
                (Vec2::new(if l.x >= 0.0 { 1.0 } else { -1.0 }, 0.0), dx + r)
            } else {
                (Vec2::new(0.0, if l.y >= 0.0 { 1.0 } else { -1.0 }), dy + r)
            };
            return Some((self.dir_to_world(n), pen));
        }
        let delta = l - cl;
        let d2 = delta.length_squared();
        if d2 >= r * r {
            return None;
        }
        let dist = d2.sqrt();
        Some((self.dir_to_world(delta / dist), r - dist))
    }

    /// Slab ray cast. `dir` need not be normalized; `t` is in units of `dir`.
    pub fn raycast(&self, origin: Vec2, dir: Vec2, max_t: f32) -> Option<(f32, Vec2)> {
        let o = self.to_local(origin);
        let d = self.dir_to_local(dir);
        let mut tmin = 0.0f32;
        let mut tmax = max_t;
        let mut normal = -dir;
        for axis in 0..2 {
            let (oa, da, h) = if axis == 0 { (o.x, d.x, self.half.x) } else { (o.y, d.y, self.half.y) };
            if da.abs() < 1e-9 {
                if oa.abs() > h {
                    return None;
                }
                continue;
            }
            let inv = 1.0 / da;
            let mut t1 = (-h - oa) * inv;
            let mut t2 = (h - oa) * inv;
            let sign = if da > 0.0 { -1.0 } else { 1.0 };
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
            }
            if t1 > tmin {
                tmin = t1;
                let n_local = if axis == 0 { Vec2::new(sign, 0.0) } else { Vec2::new(0.0, sign) };
                normal = self.dir_to_world(n_local);
            }
            tmax = tmax.min(t2);
            if tmin > tmax {
                return None;
            }
        }
        Some((tmin, normal))
    }
}

/// Closest points between segments p1-q1 and p2-q2 (Ericson, RTCD 5.1.9).
/// Returns (s, t, c1, c2) where c1 = p1 + (q1-p1)*s and c2 = p2 + (q2-p2)*t.
pub fn closest_segments(p1: Vec2, q1: Vec2, p2: Vec2, q2: Vec2) -> (f32, f32, Vec2, Vec2) {
    const EPS: f32 = 1e-10;
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= EPS && e <= EPS {
        s = 0.0;
        t = 0.0;
    } else if a <= EPS {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom != 0.0 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    (s, t, p1 + d1 * s, p2 + d2 * t)
}

/// Distance from point `p` to segment a-b, and the segment parameter.
pub fn point_segment(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 > 1e-12 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((a + ab * t - p).length(), t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_vs_box() {
        let b = Obb::from_min_max(Vec2::new(-1.0, -1.0), Vec2::new(1.0, 0.0));
        let (n, pen) = b.collide_circle(Vec2::new(0.0, 0.1), 0.2).unwrap();
        assert!((n - Vec2::Y).length() < 1e-6);
        assert!((pen - 0.1).abs() < 1e-6);
        assert!(b.collide_circle(Vec2::new(0.0, 0.3), 0.2).is_none());
    }

    #[test]
    fn ray_hits_top_face() {
        let b = Obb::from_min_max(Vec2::new(-1.0, -1.0), Vec2::new(1.0, 0.0));
        let (t, n) = b.raycast(Vec2::new(0.2, 2.0), Vec2::NEG_Y, 10.0).unwrap();
        assert!((t - 2.0).abs() < 1e-6);
        assert!((n - Vec2::Y).length() < 1e-6);
    }
}
