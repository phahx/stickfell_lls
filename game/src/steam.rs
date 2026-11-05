//! Steam: what water boiling off lava turns into. A coarse grid over the
//! level holds how thick the steam is in each cell (1: a cell full of it).
//! Each frame it rises (pooling under ceilings), spreads sideways, and
//! condenses away; it never goes into the level's solid parts. Fighters in
//! thick enough steam are scalded (see `sim::update_steam`).
//!
//! Real water flashing to steam swells about 1,670 times: the yard's flood
//! meeting its lava pit would boil off enough to fill the yard eight times
//! over. This is far less of it (`STEAM_PER_DROP`), so it's survivable.

use stickfell_physics::{Vec2, World};

/// The grid's cells (m).
pub const CELL: f32 = 1.0;
/// Steam from one boiled drop of water, in cells' worth (1 = one full cell).
/// Measured over the yard's flood (with `CONDENSE` 6 s): visible steam
/// (5% opaque or more) peaks at 95 m^2 and averages 8 m^2; at 0.5 and 12 s
/// it was 207 and 26, too big.
pub const STEAM_PER_DROP: f32 = 0.15;
/// Share of a cell's steam that rises into the cell above, a second (as far
/// as there's room there).
const RISE: f32 = 3.0;
/// Share of the difference with each side neighbour that evens out, a second.
const SPREAD: f32 = 1.2;
/// The most a cell holds before steam backs up under a ceiling.
const FULL: f32 = 1.0;
/// Time for steam to thin to 1/e by condensing (s), in the air and in
/// liquid (rising through a pool, it's mostly gone into it before the top).
pub const CONDENSE: f32 = 6.0;
const IN_LIQUID: f32 = 0.3;

#[derive(Clone, Debug, Default)]
pub struct Steam {
    origin: Vec2,
    w: usize,
    h: usize,
    /// How thick, per cell (row-major, bottom row first).
    pub thick: Vec<f32>,
    /// Cells inside the level's solid parts.
    solid: Vec<bool>,
    scratch: Vec<f32>,
}

impl Steam {
    /// An empty grid over `lo`..`hi`, with the level's solid parts marked.
    pub fn new(lo: Vec2, hi: Vec2, world: &World) -> Self {
        let w = (((hi.x - lo.x) / CELL).ceil() as usize).max(1);
        let h = (((hi.y - lo.y) / CELL).ceil() as usize).max(1);
        let mut solid = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let c = lo + Vec2::new((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL);
                solid[y * w + x] = world.statics.iter().any(|o| o.collide_circle(c, 0.001).is_some());
            }
        }
        Self { origin: lo, w, h, thick: vec![0.0; w * h], solid, scratch: vec![0.0; w * h] }
    }

    fn cell(&self, p: Vec2) -> Option<usize> {
        let q = (p - self.origin) / CELL;
        if q.x < 0.0 || q.y < 0.0 {
            return None;
        }
        let (x, y) = (q.x as usize, q.y as usize);
        (x < self.w && y < self.h).then(|| y * self.w + x)
    }

    /// How thick the steam is at `p`.
    pub fn at(&self, p: Vec2) -> f32 {
        self.cell(p).map_or(0.0, |k| self.thick[k])
    }

    /// Puts `amount` (cells' worth) of steam in at `p` (into the nearest open
    /// cell above, if `p` is in something solid).
    pub fn add(&mut self, p: Vec2, amount: f32) {
        let Some(mut k) = self.cell(p) else { return };
        while self.solid[k] {
            if k + self.w >= self.thick.len() {
                return;
            }
            k += self.w;
        }
        self.thick[k] += amount;
    }

    /// All of it, in cells' worth.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn total(&self) -> f32 {
        self.thick.iter().sum()
    }

    /// One frame: up, sideways, and condensing away.
    pub fn step(&mut self, dt: f32) {
        if self.thick.iter().all(|&t| t == 0.0) {
            return;
        }
        let (w, h) = (self.w, self.h);
        // up, top rows first (each cell into the one above, as there's room)
        let up = (RISE * dt).min(1.0);
        for y in (0..h - 1).rev() {
            for x in 0..w {
                let (k, a) = (y * w + x, (y + 1) * w + x);
                if self.solid[a] || self.thick[k] <= 0.0 {
                    continue;
                }
                let room = (FULL - self.thick[a]).max(0.0);
                let m = (self.thick[k] * up).min(room);
                self.thick[k] -= m;
                self.thick[a] += m;
            }
        }
        // sideways: evening out with open neighbours
        let side = (SPREAD * dt).min(0.25);
        self.scratch.copy_from_slice(&self.thick);
        for y in 0..h {
            for x in 0..w {
                let k = y * w + x;
                if self.solid[k] {
                    continue;
                }
                let mut d = 0.0;
                for n in [x.checked_sub(1), (x + 1 < w).then_some(x + 1)].into_iter().flatten() {
                    let j = y * w + n;
                    if !self.solid[j] {
                        d += (self.scratch[j] - self.scratch[k]) * side;
                    }
                }
                self.thick[k] += d;
            }
        }
        // condensing
        let keep = (-dt / CONDENSE).exp();
        for t in &mut self.thick {
            *t *= keep;
            if *t < 1e-4 {
                *t = 0.0;
            }
        }
    }

    /// Steam in liquid condenses back into it, fast (`IN_LIQUID`): `wet`
    /// says how much liquid is at a point (1 = all of it).
    pub fn condense_in(&mut self, dt: f32, wet: impl Fn(Vec2) -> f32) {
        for k in 0..self.thick.len() {
            if self.thick[k] <= 0.0 {
                continue;
            }
            let c = self.origin + Vec2::new(((k % self.w) as f32 + 0.5) * CELL, ((k / self.w) as f32 + 0.5) * CELL);
            let l = wet(c).clamp(0.0, 1.0);
            if l > 0.0 {
                self.thick[k] *= (-dt * l / IN_LIQUID).exp();
            }
        }
    }

    /// Every cell with steam in it worth drawing: its middle and thickness.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clouds(&self, min: f32) -> impl Iterator<Item = (Vec2, f32)> + '_ {
        self.thick.iter().enumerate().filter(move |&(_, &t)| t >= min).map(move |(k, &t)| {
            let (x, y) = (k % self.w, k / self.w);
            (self.origin + Vec2::new((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL), t)
        })
    }

    /// The grid: its corner, cells across and up, and thickness per cell
    /// (row-major, bottom row first).
    pub fn grid(&self) -> (Vec2, usize, usize, &[f32]) {
        (self.origin, self.w, self.h, &self.thick)
    }

    pub fn hash_into(&self, mut h: impl FnMut(u32)) {
        for &t in &self.thick {
            h(t.to_bits());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stickfell_physics::{Obb, WorldConfig};

    /// A room 10 m wide and 6 m tall: floor, walls, ceiling.
    fn room() -> Steam {
        let mut w = World::new(WorldConfig::default());
        for (a, b) in [((-6.0, -1.0), (6.0, 0.0)), ((-6.0, 6.0), (6.0, 7.0)), ((-6.0, 0.0), (-5.0, 6.0)), ((5.0, 0.0), (6.0, 6.0))] {
            w.add_static(Obb::from_min_max(Vec2::new(a.0, a.1), Vec2::new(b.0, b.1)));
        }
        Steam::new(Vec2::new(-6.0, -1.0), Vec2::new(6.0, 7.0), &w)
    }

    #[test]
    fn steam_rises_pools_under_the_ceiling_and_fades() {
        let mut s = room();
        s.add(Vec2::new(0.0, 0.5), 6.0);
        let start = s.total();
        for _ in 0..180 {
            s.step(1.0 / 60.0);
        }
        // (in the room's top row, under the ceiling, more than at the floor)
        let top = (-4..=4).map(|x| s.at(Vec2::new(x as f32 + 0.5, 5.5))).sum::<f32>();
        let floor = (-4..=4).map(|x| s.at(Vec2::new(x as f32 + 0.5, 0.5))).sum::<f32>();
        assert!(top > floor * 3.0, "it gathers up top ({top:.2} vs {floor:.2})");
        assert!(s.at(Vec2::new(0.0, 6.5)) == 0.0 && s.at(Vec2::new(-5.5, 3.0)) == 0.0, "never in the ceiling or walls");
        let left = s.total();
        let want = start * (-3.0 / CONDENSE).exp();
        assert!((left - want).abs() < want * 0.05, "condenses away at its rate ({left:.3} vs {want:.3})");
    }
}
