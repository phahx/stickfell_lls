//! Deterministic math.
//!
//! The simulation must produce bit-identical results on every peer, so it may
//! only use IEEE-754 basic operations (`+ - * /`, `sqrt`, `floor`, `abs`, ...),
//! which are exactly specified. Platform `sin`/`cos`/`atan2` come from the
//! system math library and can differ between OSes and CPUs, so the sim uses
//! these polynomial versions instead.

use core::f32::consts::{FRAC_PI_2, PI, TAU};

/// atan(x) for |x| <= 1 (Abramowitz & Stegun 4.4.49, max error ~1e-5 rad).
#[inline]
fn atan_unit(x: f32) -> f32 {
    let x2 = x * x;
    x * (0.999_866
        + x2 * (-0.330_299_5 + x2 * (0.180_141 + x2 * (-0.085_133 + x2 * 0.020_835_1))))
}

/// Deterministic `atan2(y, x)` in (-PI, PI].
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    let ax = x.abs();
    let ay = y.abs();
    if ax == 0.0 && ay == 0.0 {
        return 0.0;
    }
    let mut a = if ax >= ay {
        atan_unit(ay / ax)
    } else {
        FRAC_PI_2 - atan_unit(ax / ay)
    };
    if x < 0.0 {
        a = PI - a;
    }
    if y < 0.0 { -a } else { a }
}

/// Wraps an angle into [-PI, PI).
#[inline]
pub fn wrap_angle(a: f32) -> f32 {
    a - TAU * ((a + PI) / TAU).floor()
}

#[inline]
fn poly_sin(x: f32) -> f32 {
    let x2 = x * x;
    x * (1.0
        + x2 * (-1.0 / 6.0
            + x2 * (1.0 / 120.0
                + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0 + x2 * (-1.0 / 39_916_800.0))))))
}

#[inline]
fn poly_cos(x: f32) -> f32 {
    let x2 = x * x;
    1.0 + x2
        * (-0.5
            + x2 * (1.0 / 24.0
                + x2 * (-1.0 / 720.0
                    + x2 * (1.0 / 40_320.0
                        + x2 * (-1.0 / 3_628_800.0 + x2 * (1.0 / 479_001_600.0))))))
}

/// Deterministic `(sin(a), cos(a))`.
#[inline]
pub fn sin_cos(a: f32) -> (f32, f32) {
    let a = wrap_angle(a);
    let (x, cos_sign) = if a > FRAC_PI_2 {
        (PI - a, -1.0)
    } else if a < -FRAC_PI_2 {
        (-PI - a, -1.0)
    } else {
        (a, 1.0)
    };
    (poly_sin(x), cos_sign * poly_cos(x))
}

#[inline]
pub fn sin(a: f32) -> f32 {
    sin_cos(a).0
}

#[inline]
pub fn cos(a: f32) -> f32 {
    sin_cos(a).1
}

/// Unit vector at angle `a` (deterministic).
#[inline]
pub fn dir(a: f32) -> glam::Vec2 {
    let (s, c) = sin_cos(a);
    glam::Vec2::new(c, s)
}

/// Angle of a vector (deterministic).
#[inline]
pub fn angle(v: glam::Vec2) -> f32 {
    atan2(v.y, v.x)
}

/// Shortest signed difference `a - b`, wrapped to [-PI, PI).
#[inline]
pub fn angle_diff(a: f32, b: f32) -> f32 {
    wrap_angle(a - b)
}

/// Deterministic xorshift RNG; lives inside simulation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(pub u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [0, 1).
    #[inline]
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
    /// Uniform in [lo, hi).
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trig_matches_std_closely() {
        let mut a = -10.0f32;
        while a < 10.0 {
            let (s, c) = sin_cos(a);
            assert!((s - a.sin()).abs() < 1e-5, "sin {a}");
            assert!((c - a.cos()).abs() < 1e-5, "cos {a}");
            a += 0.0137;
        }
    }

    #[test]
    fn atan2_matches_std_closely() {
        for i in 0..720 {
            let t = i as f32 * 0.5f32.to_radians();
            for r in [0.01f32, 1.0, 37.0] {
                let (y, x) = (t.sin() * r, t.cos() * r);
                let d = wrap_angle(atan2(y, x) - y.atan2(x)).abs();
                assert!(d < 2e-5, "atan2({y},{x}) off by {d}");
            }
        }
    }
}
