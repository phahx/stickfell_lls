//! A particle fluid: liquid as a volume of drops that flows, sloshes, pours
//! and splashes, pushed aside by (and pushing on) everything solid.
//!
//! Each step: gravity and viscosity on the drops' velocities, a predicted
//! move, then "double density relaxation" (Clavet, Beaudoin & Poulin 2005)
//! pushes crowded drops apart and pulls sparse ones together, which is what
//! makes it hold its volume, level out and have a surface; then the drops
//! are pushed out of the level's boxes and out of every capsule in the
//! [`World`], the capsules taking some of the push back.
//!
//! Deterministic like the rest of the crate: drops are visited in index
//! order through a grid rebuilt by counting sort, no hash maps.

use crate::collide::point_segment;
use crate::world::World;
use glam::Vec2;
use rayon::prelude::*;

/// The drops, for working on several at once from different threads. Only
/// ever used on drops no other thread is touching (see `Fluid::colours`).
struct Shared(*mut Drop);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// # Safety
    /// No other thread may be using drop `i` at the same time.
    #[allow(clippy::mut_from_ref)]
    unsafe fn at(&self, i: usize) -> &mut Drop {
        unsafe { &mut *self.0.add(i) }
    }
}

/// From how many drops the work is spread over threads, by default (the
/// result is the same either way).
const PARALLEL_FROM: usize = 1500;

/// How hard sparse drops pull back together, as a share of `pressure`.
const COHESION: f32 = 0.04;

/// The middle of a [`Fluid::sample`] that doesn't count (share of its reach).
const SAMPLE_HOLE: f32 = 0.4;

/// How one kind of liquid behaves (all per drop).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidKind {
    /// How hard crowded drops push apart (and sparse ones pull together),
    /// and how hard drops very close push apart (keeps them from clumping).
    pub pressure: f32,
    pub near_pressure: f32,
    /// Resistance to drops moving apart or together: linear (thick, like
    /// lava) and quadratic (just enough to calm water).
    pub viscosity: f32,
    pub viscosity_sq: f32,
    /// Mass, against the bodies it shoves about (and that shove it).
    pub mass: f32,
    /// Grip on the level's surfaces (0: slides freely).
    pub friction: f32,
    /// How heavy it is against other liquids (water 1): a lighter one rises
    /// through a heavier one and sits on top of it.
    pub density: f32,
    /// How fast dye evens out between neighbouring drops of it (share of
    /// the difference passed on per frame, for drops right on top of each
    /// other; 0: it stays where it's put).
    pub dye_spread: f32,
}

/// One drop of liquid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drop {
    pub pos: Vec2,
    pub prev: Vec2,
    pub vel: Vec2,
    /// Which [`FluidKind`].
    pub kind: u8,
    /// How much of something mixed in (blood, say) it carries: 0 clear, 1
    /// all of it. Spreads to neighbouring drops of the same kind.
    pub dye: f32,
}

/// What's around a point: how much of each kind of liquid (1 = as packed as
/// liquid at rest), and the liquid's average velocity there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FluidSample {
    pub amount: [f32; 4],
    /// How much of each kind's amount is dyed.
    pub dyed: [f32; 4],
    pub vel: Vec2,
}

impl FluidSample {
    pub fn total(&self) -> f32 {
        self.amount.iter().sum()
    }
}

/// Drops binned into square cells (counting sort, so the order is fixed).
#[derive(Clone, Debug, Default)]
struct Grid {
    origin: Vec2,
    cell: f32,
    w: i32,
    h: i32,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    fn build(&mut self, drops: &[Drop], cell: f32) {
        self.cell = cell;
        if drops.is_empty() {
            self.w = 0;
            self.h = 0;
            self.start.clear();
            self.items.clear();
            return;
        }
        let (mut lo, mut hi) = (drops[0].pos, drops[0].pos);
        for d in drops {
            lo = lo.min(d.pos);
            hi = hi.max(d.pos);
        }
        // (a runaway drop mustn't make the grid enormous: coarser cells then)
        let span = hi - lo;
        let mut c = cell;
        while (span.x / c) * (span.y / c) > 400_000.0 {
            c *= 2.0;
        }
        self.cell = c;
        self.origin = lo - Vec2::splat(c);
        self.w = (span.x / c) as i32 + 3;
        self.h = (span.y / c) as i32 + 3;
        let n = (self.w * self.h) as usize;
        self.start.clear();
        self.start.resize(n + 1, 0);
        for d in drops {
            let k = self.key(d.pos);
            self.start[k + 1] += 1;
        }
        for k in 0..n {
            self.start[k + 1] += self.start[k];
        }
        self.items.clear();
        self.items.resize(drops.len(), 0);
        let mut fill = self.start.clone();
        for (i, d) in drops.iter().enumerate() {
            let k = self.key(d.pos);
            self.items[fill[k] as usize] = i as u32;
            fill[k] += 1;
        }
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        let q = (p - self.origin) / self.cell;
        ((q.x as i32).clamp(0, self.w - 1), (q.y as i32).clamp(0, self.h - 1))
    }

    fn key(&self, p: Vec2) -> usize {
        let (x, y) = self.cell_of(p);
        (y * self.w + x) as usize
    }

    /// Every drop in the cells overlapping the box `lo..hi`.
    fn each_in(&self, lo: Vec2, hi: Vec2, mut f: impl FnMut(usize)) {
        if self.w == 0 {
            return;
        }
        let (x0, y0) = self.cell_of(lo);
        let (x1, y1) = self.cell_of(hi);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let k = (y * self.w + x) as usize;
                for &i in &self.items[self.start[k] as usize..self.start[k + 1] as usize] {
                    f(i as usize);
                }
            }
        }
    }
}

/// The level's boxes sorted into coarse cells over the liquid, so a drop
/// only tries the boxes near it.
#[derive(Clone, Debug, Default)]
struct StaticBins {
    origin: Vec2,
    w: i32,
    h: i32,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl StaticBins {
    fn of(&self, p: Vec2) -> &[u32] {
        if self.w == 0 {
            return &[];
        }
        let q = (p - self.origin) / STATIC_CELL;
        let (x, y) = ((q.x as i32).clamp(0, self.w - 1), (q.y as i32).clamp(0, self.h - 1));
        let k = (y * self.w + x) as usize;
        &self.items[self.start[k] as usize..self.start[k + 1] as usize]
    }
}

/// How much further than reach neighbours are looked for, once a frame
/// (drops move during the frame's substeps).
const NBR_MARGIN: f32 = 1.35;
/// The coarse cells the level's boxes are sorted into (m).
const STATIC_CELL: f32 = 1.0;
/// How far a drop can have moved since the grid was built, for queries
/// against it later in the frame (m).
const STALE: f32 = 0.3;
/// How far a drop pressed between a capsule and the level slides out along
/// the gap each substep (share of its radius).
const SQUEEZE: f32 = 0.5;
/// How much of a drop's slip along a capsule it touches goes, a substep.
const SLIP: f32 = 0.1;
/// Water going through something porous: how much of their difference in
/// speed goes, a substep.
const FLOW_GRIP: f32 = 0.05;
/// How near the level a capsule's end can be and count as resting on it (m).
const RESTING: f32 = 0.005;

#[derive(Clone, Debug)]
pub struct Fluid {
    pub drops: Vec<Drop>,
    pub kinds: Vec<FluidKind>,
    /// How far apart drops sit at rest, how far they reach each other (the
    /// interaction radius), and how big each is against solid things.
    pub spacing: f32,
    pub reach: f32,
    pub radius: f32,
    /// Steps per frame, and how much of their shove the capsules feel back.
    pub substeps: u32,
    pub push_back: f32,
    /// How hard a splash throws the liquid it shoves aside (share of the
    /// speed it was hit at).
    pub splash_strength: f32,
    /// From how many drops work is spread over threads (same result either way).
    pub parallel_from: usize,
    /// The packed-drop density (worked out from `spacing` and `reach`).
    rest: f32,
    rest_near: f32,
    /// How far [`Fluid::sample`] looks (wider than a body part, so the
    /// liquid it's pushed aside still counts), and liquid at rest there.
    pub sample_reach: f32,
    rest_sample: f32,
    grid: Grid,
    /// Each drop's neighbours this substep (`nbr[nbr_start[i]..nbr_start[i + 1]]`).
    nbr_start: Vec<u32>,
    nbr: Vec<u32>,
    /// The neighbour grid's cells in 16 sets, cells in a set 4 apart each
    /// way: drops in different cells of a set share no neighbours, so they
    /// can be worked on at once (and the order never depends on threads).
    colours: Vec<Vec<u32>>,
    /// The level's boxes by coarse cell over the liquid (this frame).
    bins: StaticBins,
}

impl Fluid {
    pub fn new(kinds: Vec<FluidKind>, spacing: f32) -> Self {
        let reach = spacing * 2.0;
        let sample_reach = spacing * 3.5;
        let mut f = Self { drops: Vec::new(), kinds, spacing, reach, radius: spacing * 0.5, substeps: 4, push_back: 0.5, splash_strength: 1.1, parallel_from: PARALLEL_FROM, rest: 1.0, rest_near: 1.0, sample_reach, rest_sample: 1.0, grid: Grid::default(), nbr_start: Vec::new(), nbr: Vec::new(), colours: Vec::new(), bins: StaticBins::default() };
        // the density a drop has inside a block of them laid out at `spacing`
        let (mut rho, mut near) = (0.0, 0.0);
        for y in -3i32..=3 {
            for x in -3i32..=3 {
                if x == 0 && y == 0 {
                    continue;
                }
                let r = Vec2::new(x as f32, y as f32).length() * spacing;
                let q = r / reach;
                if q < 1.0 {
                    rho += (1.0 - q) * (1.0 - q);
                    near += (1.0 - q) * (1.0 - q) * (1.0 - q);
                }
            }
        }
        f.rest = rho;
        f.rest_near = near;
        // (a fine lattice of points standing in for liquid at rest)
        let mut sample = 0.0;
        let fine = spacing / 4.0;
        let n = (sample_reach / fine) as i32 + 1;
        for y in -n..=n {
            for x in -n..=n {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).length() * fine / sample_reach;
                sample += Self::ring(q);
            }
        }
        f.rest_sample = sample / 16.0;
        f
    }

    /// Fills the box `lo..hi` with drops of `kind`, packed at rest.
    pub fn fill(&mut self, lo: Vec2, hi: Vec2, kind: u8) {
        let s = self.spacing;
        let nx = ((hi.x - lo.x) / s).floor().max(1.0) as i32;
        let ny = ((hi.y - lo.y) / s).floor().max(1.0) as i32;
        let x0 = lo.x + ((hi.x - lo.x) - (nx - 1) as f32 * s) * 0.5;
        for y in 0..ny {
            for x in 0..nx {
                // (every other row a little offset: they settle better)
                let p = Vec2::new(x0 + x as f32 * s + if y % 2 == 1 { s * 0.25 } else { 0.0 }, lo.y + s * 0.5 + y as f32 * s);
                if p.x > hi.x {
                    continue;
                }
                self.drops.push(Drop { pos: p, prev: p, vel: Vec2::ZERO, kind, dye: 0.0 });
            }
        }
    }

    pub fn rest_density(&self) -> f32 {
        self.rest
    }

    /// Advances the liquid one frame (`dt` s): its own motion, the level's
    /// boxes, and every capsule in `world` (which get shoved back).
    pub fn step(&mut self, dt: f32, world: &mut World) {
        if self.drops.is_empty() || dt <= 0.0 {
            return;
        }
        let n = self.substeps.max(1);
        let h = dt / n as f32;
        self.find_neighbours();
        self.bin_statics(world);
        for _ in 0..n {
            self.substep(h, world);
        }
        self.spread_dye();
        self.grid.build(&self.drops, self.reach);
    }

    /// Dye evens out between neighbouring drops of the same kind (what one
    /// gives the other takes, so none is made or lost).
    fn spread_dye(&mut self) {
        if !self.drops.iter().any(|d| d.dye > 0.0) {
            return;
        }
        let reach = self.reach;
        let mut delta = vec![0.0f32; self.drops.len()];
        for i in 0..self.drops.len() {
            let di = self.drops[i];
            for k in self.nbr_start[i]..self.nbr_start[i + 1] {
                let j = self.nbr[k as usize] as usize;
                let dj = self.drops[j];
                if j <= i || dj.kind != di.kind || dj.dye == di.dye {
                    continue;
                }
                let q = (dj.pos - di.pos).length() / reach;
                if q >= 1.0 {
                    continue;
                }
                let flow = (dj.dye - di.dye) * self.kinds[di.kind as usize].dye_spread * (1.0 - q);
                delta[i] += flow;
                delta[j] -= flow;
            }
        }
        for (d, dd) in self.drops.iter_mut().zip(delta) {
            d.dye = (d.dye + dd).clamp(0.0, 1.0);
        }
    }

    /// Something mixing into the liquid at `p`: the nearest drop of `kind`
    /// within reach takes `amount` of dye (what it can't hold goes to the
    /// next nearest). False if there's no such liquid there.
    pub fn dye(&mut self, p: Vec2, amount: f32, kind: u8) -> bool {
        let reach = self.reach;
        let mut near: Vec<(f32, usize)> = Vec::new();
        let drops = &self.drops;
        self.grid.each_in(p - Vec2::splat(reach), p + Vec2::splat(reach), |j| {
            let d = (drops[j].pos - p).length();
            if drops[j].kind == kind && d < reach {
                near.push((d, j));
            }
        });
        if near.is_empty() {
            return false;
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut left = amount;
        for (_, j) in near {
            let d = &mut self.drops[j];
            let take = left.min(1.0 - d.dye);
            d.dye += take;
            left -= take;
            if left <= 0.0 {
                break;
            }
        }
        true
    }

    /// All the dye in the liquid (for checking).
    pub fn total_dye(&self) -> f32 {
        self.drops.iter().map(|d| d.dye).sum()
    }

    /// Who's near whom, once a frame (out past reach: they move during it).
    fn find_neighbours(&mut self) {
        let reach = self.reach * NBR_MARGIN;
        let r2 = reach * reach;
        self.grid.build(&self.drops, reach);
        // (in chunks, each on whatever thread, put back together in order)
        let (drops, grid) = (&self.drops, &self.grid);
        let find = |range: std::ops::Range<usize>| {
            let (mut counts, mut list) = (Vec::with_capacity(range.len()), Vec::new());
            for i in range {
                let pi = drops[i].pos;
                let before = list.len();
                grid.each_in(pi - Vec2::splat(reach), pi + Vec2::splat(reach), |j| {
                    if j != i && (drops[j].pos - pi).length_squared() < r2 {
                        list.push(j as u32);
                    }
                });
                counts.push((list.len() - before) as u32);
            }
            (counts, list)
        };
        const CHUNK: usize = 512;
        let n = drops.len();
        let ranges: Vec<std::ops::Range<usize>> = (0..n.div_ceil(CHUNK)).map(|c| c * CHUNK..((c + 1) * CHUNK).min(n)).collect();
        let parts: Vec<(Vec<u32>, Vec<u32>)> = if n >= self.parallel_from { ranges.into_par_iter().map(find).collect() } else { ranges.into_iter().map(find).collect() };
        self.nbr_start.clear();
        self.nbr.clear();
        let mut at = 0u32;
        for (counts, list) in parts {
            for c in counts {
                self.nbr_start.push(at);
                at += c;
            }
            self.nbr.extend_from_slice(&list);
        }
        self.nbr_start.push(self.nbr.len() as u32);
        // the cells, coloured
        self.colours.clear();
        self.colours.resize(16, Vec::new());
        let g = &self.grid;
        for y in 0..g.h {
            for x in 0..g.w {
                let k = (y * g.w + x) as usize;
                if g.start[k] != g.start[k + 1] {
                    self.colours[((y % 4) * 4 + x % 4) as usize].push(k as u32);
                }
            }
        }
    }

    /// Runs `f` on every drop: colour by colour, the cells of a colour at
    /// once (each cell's drops in order).
    fn each_coloured(&self, f: impl Fn(usize) + Sync) {
        let g = &self.grid;
        let cell = |k: u32| {
            for &i in &g.items[g.start[k as usize] as usize..g.start[k as usize + 1] as usize] {
                f(i as usize);
            }
        };
        let parallel = self.drops.len() >= self.parallel_from;
        for set in &self.colours {
            if parallel {
                set.par_iter().for_each(|&k| cell(k));
            } else {
                set.iter().for_each(|&k| cell(k));
            }
        }
    }

    fn substep(&mut self, dt: f32, world: &mut World) {
        let g = world.cfg.gravity;
        let reach = self.reach;
        let r2 = reach * reach;
        for d in &mut self.drops {
            d.vel += g * dt;
        }
        // viscosity: drops closing on (or leaving) each other are slowed toward each other's speed
        let len = self.drops.len();
        {
            let shared = Shared(self.drops.as_mut_ptr());
            let (nbr, nbr_start, kinds) = (&self.nbr, &self.nbr_start, &self.kinds);
            self.each_coloured(|i| {
                // SAFETY: drop i and its neighbours are touched by no other thread (see `colours`)
                let di = unsafe { *shared.at(i) };
                let kind = kinds[di.kind as usize];
                if kind.viscosity <= 0.0 && kind.viscosity_sq <= 0.0 {
                    return;
                }
                for k in nbr_start[i]..nbr_start[i + 1] {
                    let j = nbr[k as usize] as usize;
                    if j <= i || j >= len {
                        continue;
                    }
                    let (vi, dj) = unsafe { (shared.at(i).vel, *shared.at(j)) };
                    let r = dj.pos - di.pos;
                    let l2 = r.length_squared();
                    if l2 >= r2 || l2 < 1e-12 {
                        continue;
                    }
                    let l = l2.sqrt();
                    let q = l / reach;
                    let rn = r / l;
                    let u = (vi - dj.vel).dot(rn);
                    if u > 0.0 {
                        let imp = rn * (dt * (1.0 - q) * (kind.viscosity * u + kind.viscosity_sq * u * u)).min(u * 0.5);
                        unsafe {
                            shared.at(i).vel -= imp * 0.5;
                            shared.at(j).vel += imp * 0.5;
                        }
                    }
                }
            });
        }
        // the move
        for d in &mut self.drops {
            d.prev = d.pos;
            d.pos += d.vel * dt;
        }
        // double density relaxation
        let (rest, rest_near) = (self.rest, self.rest_near);
        let max_push = self.spacing * 0.15;
        {
            let shared = Shared(self.drops.as_mut_ptr());
            let (nbr, nbr_start, kinds) = (&self.nbr, &self.nbr_start, &self.kinds);
            self.each_coloured(|i| {
                // SAFETY: drop i and its neighbours are touched by no other thread (see `colours`)
                let di = unsafe { *shared.at(i) };
                let pi = di.pos;
                let kind = kinds[di.kind as usize];
                let (s0, s1) = (nbr_start[i] as usize, nbr_start[i + 1] as usize);
                let (mut rho, mut near) = (0.0f32, 0.0f32);
                for &j in &nbr[s0..s1] {
                    let dj = unsafe { *shared.at(j as usize) };
                    let l2 = (dj.pos - pi).length_squared();
                    if l2 < r2 {
                        // (heavier neighbours count for more: they press the
                        // lighter ones up and out of their way)
                        let m = kinds[dj.kind as usize].density / kind.density;
                        let a = 1.0 - l2.sqrt() / reach;
                        rho += a * a * m;
                        near += a * a * a * m;
                    }
                }
                // (sparse drops pull together only a little: a thin surface tension, so
                // splashes can fly apart)
                let p = (kind.pressure * (rho - rest) / rest).max(-kind.pressure * COHESION);
                // (spray and loose drops barely hold each other off: they run
                // back together and level out instead of stacking like sand)
                let packed = (rho / rest).clamp(0.0, 1.0);
                let pn = kind.near_pressure * near / rest_near * packed * packed;
                let mut dx = Vec2::ZERO;
                for &j in &nbr[s0..s1] {
                    let dj = unsafe { shared.at(j as usize) };
                    let r = dj.pos - pi;
                    let l2 = r.length_squared();
                    if l2 >= r2 || l2 < 1e-12 {
                        continue;
                    }
                    let l = l2.sqrt();
                    let a = 1.0 - l / reach;
                    // (never more than a fraction of the spacing at once: squeezed
                    // hard, drops ease apart rather than burst)
                    let push = (r / l) * (dt * dt * (p * a + pn * a * a) * reach).clamp(-max_push, max_push);
                    // (shared by weight: the heavier one moves less)
                    let (mi, mj) = (kind.density, kinds[dj.kind as usize].density);
                    dj.pos += push * (mi / (mi + mj));
                    dx -= push * (mj / (mi + mj));
                }
                unsafe { shared.at(i).pos += dx };
            });
        }
        self.collide(dt, world);
        let max = 16.0;
        for d in &mut self.drops {
            let v = (d.pos - d.prev) / dt;
            let l = v.length();
            d.vel = if l > max { v * (max / l) } else { v };
        }
    }

    /// Sorts the level's boxes into coarse cells over the liquid (with room
    /// for this frame's movement).
    fn bin_statics(&mut self, world: &World) {
        let b = &mut self.bins;
        let (mut lo, mut hi) = (self.drops[0].pos, self.drops[0].pos);
        for d in &self.drops {
            lo = lo.min(d.pos);
            hi = hi.max(d.pos);
        }
        let pad = self.radius + STALE + STATIC_CELL;
        b.origin = lo - Vec2::splat(pad);
        let span = hi - lo + Vec2::splat(2.0 * pad);
        b.w = ((span.x / STATIC_CELL) as i32 + 1).min(4096);
        b.h = ((span.y / STATIC_CELL) as i32 + 1).min(4096);
        let n = (b.w * b.h) as usize;
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); n];
        let r = self.radius + STALE;
        for (k, o) in world.statics.iter().enumerate() {
            let br = o.bound_radius() + r;
            let (a, z) = ((o.center - Vec2::splat(br) - b.origin) / STATIC_CELL, (o.center + Vec2::splat(br) - b.origin) / STATIC_CELL);
            let (x0, y0) = ((a.x.floor() as i32).max(0), (a.y.floor() as i32).max(0));
            let (x1, y1) = ((z.x.floor() as i32).min(b.w - 1), (z.y.floor() as i32).min(b.h - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    lists[(y * b.w + x) as usize].push(k as u32);
                }
            }
        }
        b.start.clear();
        b.items.clear();
        for l in &lists {
            b.start.push(b.items.len() as u32);
            b.items.extend_from_slice(l);
        }
        b.start.push(b.items.len() as u32);
    }

    /// Out of the level's boxes, and out of every capsule (which feel it).
    fn collide(&mut self, dt: f32, world: &mut World) {
        let r = self.radius;
        if self.drops.is_empty() {
            return;
        }
        let (mut lo, mut hi) = (self.drops[0].pos, self.drops[0].pos);
        for d in &self.drops {
            lo = lo.min(d.pos);
            hi = hi.max(d.pos);
        }
        let (kinds, bins) = (&self.kinds, &self.bins);
        let statics = |d: &mut Drop| {
            let fr = kinds[d.kind as usize].friction;
            for &k in bins.of(d.pos) {
                let o = &world.statics[k as usize];
                if let Some((n, pen)) = o.collide_circle(d.pos, r) {
                    d.pos += n * pen;
                    // stopped, not bounced: no speed away from it out of being pushed out
                    let moved = d.pos - d.prev;
                    let away = moved.dot(n);
                    if away > 0.0 {
                        d.prev += n * away;
                    }
                    // a little grip along the surface
                    let moved = d.pos - d.prev;
                    let tang = moved - n * moved.dot(n);
                    d.pos -= tang * fr;
                }
            }
        };
        if self.drops.len() >= self.parallel_from {
            self.drops.par_iter_mut().for_each(statics);
        } else {
            self.drops.iter_mut().for_each(statics);
        }
        // capsules: drops pushed out of them, the capsule pushed back by the
        // drop's share (the grid is from the start of the frame: looked
        // through a little wider for it)
        for si in 0..world.segments.len() {
            let sg = world.segments[si];
            if !sg.enabled || sg.radius <= 0.0 {
                continue;
            }
            let (ia, ib) = (sg.a as usize, sg.b as usize);
            let (pa, pb) = (world.particles[ia], world.particles[ib]);
            if pa.inv_mass == 0.0 && pb.inv_mass == 0.0 {
                continue;
            }
            // (a drop comes right up against a capsule: a thin limb only
            // pushes aside about as much liquid as it looks like it should)
            let reach = sg.radius + r * 0.4;
            let (blo, bhi) = (pa.pos.min(pb.pos) - Vec2::splat(reach + STALE), pa.pos.max(pb.pos) + Vec2::splat(reach + STALE));
            if bhi.x < lo.x || blo.x > hi.x || bhi.y < lo.y || blo.y > hi.y {
                continue;
            }
            // (the capsule's ends' speeds, as the drops change them one by one)
            let (mut va, mut vb) = (pa.vel, pb.vel);
            let (wa, wb) = (pa.inv_mass, if ib == ia { 0.0 } else { pb.inv_mass });
            let drops = &mut self.drops;
            let (kinds, bins, statics) = (&self.kinds, &self.bins, &world.statics);
            let push_back = self.push_back;
            // an end resting on the level can't be pushed on into it: that
            // much of the shove goes into the level (packed tight against a
            // blade lying on the floor, drops would otherwise drive it on and
            // on, faster every substep, and be flung off it)
            let resting = |p: &crate::world::Particle| {
                let mut on = [Vec2::ZERO; 2];
                let mut k = 0;
                for &o in bins.of(p.pos) {
                    if let Some((ns, _)) = statics[o as usize].collide_circle(p.pos, p.radius + RESTING) {
                        on[k] = ns;
                        k += 1;
                        if k == 2 {
                            break;
                        }
                    }
                }
                on
            };
            let (on_a, on_b) = (resting(&pa), resting(&pb));
            let hold = |v: Vec2, on: &[Vec2; 2]| {
                let mut v = v;
                for &ns in on {
                    let into = v.dot(ns);
                    if into < 0.0 {
                        v -= ns * into;
                    }
                }
                v
            };
            self.grid.each_in(blo, bhi, |i| {
                let d = &mut drops[i];
                let (dist, t) = if ia == ib { ((d.pos - pa.pos).length(), 0.0) } else { point_segment(d.pos, pa.pos, pb.pos) };
                if dist >= reach {
                    return;
                }
                let at = pa.pos.lerp(pb.pos, t);
                let n = if dist > 1e-6 { (d.pos - at) / dist } else { Vec2::Y };
                let pen = reach - dist;
                // a collision that doesn't bounce, shared by weight: the drop
                // and the capsule's surface there end up going the same speed
                // along the normal (a light blade gets shoved about by water,
                // a body hardly), and part of their slip along it goes too;
                // it can only ever take energy out, never put it in
                let w_d = 1.0 / (kinds[d.kind as usize].mass * push_back).max(1e-6);
                let w_c = (1.0 - t) * (1.0 - t) * wa + t * t * wb;
                let surf = va.lerp(vb, t);
                let vel = (d.pos - d.prev) / dt;
                let rel = vel - surf;
                let vn = rel.dot(n);
                let mut imp = Vec2::ZERO;
                if vn < 0.0 {
                    imp += n * (-vn / (w_d + w_c));
                }
                // (and a little of their slip along it: water slides past
                // whatever's in it)
                imp -= (rel - n * vn) * (SLIP / (w_d + w_c));
                // something porous stops only its share of it; the rest of
                // the water goes on through, the two pulling each other
                // toward the same speed a little (the flow's drag on it, and
                // its drag on the flow)
                let p = sg.porous;
                imp = imp * (1.0 - p) - rel * (p * FLOW_GRIP / (w_d + w_c));
                let vel = vel + imp * w_d;
                va = hold(va - imp * ((1.0 - t) * wa), &on_a);
                vb = hold(vb - imp * (t * wb), &on_b);
                // out of it (where it is: the world moves the capsule), as
                // much as it doesn't let through
                d.pos += n * (pen * (1.0 - sg.porous));
                // pressed between it and the level, it squeezes out sideways
                // along the gap (toward the capsule's nearer end) instead of
                // propping it up
                for &k in bins.of(d.pos) {
                    let Some((ns, _)) = statics[k as usize].collide_circle(d.pos, r) else { continue };
                    if ns.dot(n) < -0.3 {
                        let mut along = Vec2::new(-ns.y, ns.x);
                        let end = if ia == ib { d.pos - at } else if t < 0.5 { pa.pos - at } else { pb.pos - at };
                        if along.dot(end) < 0.0 {
                            along = -along;
                        }
                        d.pos += along * (r * SQUEEZE);
                        break;
                    }
                }
                d.prev = d.pos - vel * dt;
            });
            if wa > 0.0 {
                world.particles[ia].vel = va;
            }
            if wb > 0.0 {
                world.particles[ib].vel = vb;
            }
        }
    }

    /// How much a drop `q` (share of `sample_reach`) away counts toward a
    /// sample: nothing right at the middle (that's where the body part being
    /// asked about is, having pushed the liquid out of its way), evenly out to
    /// the edge.
    fn ring(q: f32) -> f32 {
        if q < SAMPLE_HOLE || q >= 1.0 { 0.0 } else { 1.0 - q * 0.5 }
    }

    /// The liquid around `p` (within `sample_reach`): 1 when it's all liquid
    /// at rest around it.
    pub fn sample(&self, p: Vec2) -> FluidSample {
        let mut s = FluidSample::default();
        let reach = self.sample_reach;
        let mut w_sum = 0.0;
        let drops = &self.drops;
        self.grid.each_in(p - Vec2::splat(reach), p + Vec2::splat(reach), |j| {
            let d = &drops[j];
            let a = Self::ring((d.pos - p).length() / reach);
            if a > 0.0 {
                s.amount[(d.kind as usize).min(3)] += a;
                s.dyed[(d.kind as usize).min(3)] += a * d.dye;
                s.vel += d.vel * a;
                w_sum += a;
            }
        });
        if w_sum > 0.0 {
            s.vel /= w_sum;
        }
        for a in s.amount.iter_mut().chain(s.dyed.iter_mut()) {
            *a /= self.rest_sample;
        }
        s
    }

    /// Drops of kind `a` touching (within `within`) a drop of kind `b` that
    /// isn't fully dyed: each `a` with the nearest such `b`, in order.
    pub fn touching(&self, a: u8, b: u8, within: f32) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        if !self.drops.iter().any(|d| d.kind == b) {
            return out;
        }
        for (i, d) in self.drops.iter().enumerate() {
            if d.kind != a {
                continue;
            }
            let mut best: Option<(f32, usize)> = None;
            self.grid.each_in(d.pos - Vec2::splat(within), d.pos + Vec2::splat(within), |j| {
                let o = &self.drops[j];
                let l = (o.pos - d.pos).length();
                if o.kind == b && o.dye < 1.0 && l < within && best.is_none_or(|(bl, bj)| l < bl || (l == bl && j < bj)) {
                    best = Some((l, j));
                }
            });
            if let Some((_, j)) = best {
                out.push((i, j));
            }
        }
        out
    }

    /// Takes out the drops at these indices (any order, repeats fine).
    pub fn remove(&mut self, ids: &[usize]) {
        if ids.is_empty() {
            return;
        }
        let mut gone = vec![false; self.drops.len()];
        for &i in ids {
            gone[i] = true;
        }
        let mut k = 0;
        self.drops.retain(|_| {
            k += 1;
            !gone[k - 1]
        });
        self.grid.build(&self.drops, self.reach);
    }

    /// Adds one drop of `kind` at `pos`, moving at `vel`.
    pub fn pour(&mut self, pos: Vec2, vel: Vec2, kind: u8) {
        self.drops.push(Drop { pos, prev: pos - vel / 60.0, vel, kind, dye: 0.0 });
    }

    /// Lets freshly poured liquid come to rest (`frames` 60ths of a second,
    /// heavily damped), so it starts out still.
    pub fn settle(&mut self, world: &mut World, frames: u32) {
        for _ in 0..frames {
            self.step(1.0 / 60.0, world);
            for d in &mut self.drops {
                d.vel *= 0.8;
            }
        }
    }

    /// Something `x0..x1` wide, its bottom at `low`, hitting the liquid at
    /// `speed` m/s: the liquid it shoves aside goes up and out around it
    /// (the drops at the surface just beyond its sides).
    pub fn splash(&mut self, x0: f32, x1: f32, low: f32, speed: f32) {
        let band = 0.45;
        // the surface there: the highest drop under it
        let top = self.drops.iter().filter(|d| d.pos.x > x0 - band && d.pos.x < x1 + band && d.pos.y < low + 0.15).map(|d| d.pos.y).fold(f32::MIN, f32::max);
        if top == f32::MIN {
            return;
        }
        for d in &mut self.drops {
            if d.pos.y < top - 0.3 || d.pos.y > top + 0.2 {
                continue;
            }
            let (out, side) = if d.pos.x < x0 { (x0 - d.pos.x, -1.0) } else if d.pos.x > x1 { (d.pos.x - x1, 1.0) } else { (0.0, if d.pos.x < (x0 + x1) * 0.5 { -1.0 } else { 1.0 }) };
            if out >= band {
                continue;
            }
            let k = (1.0 - out / band) * (1.0 - (top - d.pos.y).max(0.0) / 0.3);
            // (right under it, it's mostly pushed aside)
            let up = if out > 0.0 { 1.0 } else { 0.3 };
            d.vel += Vec2::new(side * 0.5, up) * (speed * self.splash_strength * k);
        }
    }

    /// How packed the drops are on average (1 = at rest), for checking.
    pub fn mean_density(&self) -> f32 {
        if self.drops.is_empty() {
            return 0.0;
        }
        let reach = self.reach;
        let mut total = 0.0;
        for (i, d) in self.drops.iter().enumerate() {
            let mut rho = 0.0;
            self.grid.each_in(d.pos - Vec2::splat(reach), d.pos + Vec2::splat(reach), |j| {
                let q = (self.drops[j].pos - d.pos).length() / reach;
                if j != i && q < 1.0 {
                    rho += (1.0 - q) * (1.0 - q);
                }
            });
            total += rho / self.rest;
        }
        total / self.drops.len() as f32
    }

    /// Drops that have left `lo..hi` are gone.
    pub fn keep_within(&mut self, lo: Vec2, hi: Vec2) {
        self.drops.retain(|d| d.pos.x >= lo.x && d.pos.x <= hi.x && d.pos.y >= lo.y && d.pos.y <= hi.y && d.pos.is_finite());
        self.grid.build(&self.drops, self.reach);
    }

    pub fn hash_into(&self, mut write: impl FnMut(u32)) {
        write(self.drops.len() as u32);
        for d in &self.drops {
            write(d.pos.x.to_bits());
            write(d.pos.y.to_bits());
            write(d.vel.x.to_bits());
            write(d.vel.y.to_bits());
            write(d.dye.to_bits());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Obb, WorldConfig};

    fn tank() -> (World, Fluid) {
        let mut w = World::new(WorldConfig::default());
        w.add_static(Obb::from_min_max(Vec2::new(-3.0, -1.0), Vec2::new(3.0, 0.0)));
        w.add_static(Obb::from_min_max(Vec2::new(-3.0, 0.0), Vec2::new(-2.0, 4.0)));
        w.add_static(Obb::from_min_max(Vec2::new(2.0, 0.0), Vec2::new(3.0, 4.0)));
        let water = FluidKind { pressure: 1.0, near_pressure: 1.0, viscosity: 0.0, viscosity_sq: 0.1, mass: 0.3, friction: 0.05, density: 1.0, dye_spread: 0.05 };
        let mut f = Fluid::new(vec![water], 0.1);
        let water = FluidKind { pressure: 2000.0, near_pressure: 4000.0, viscosity: 0.3, viscosity_sq: 0.3, ..water };
        f.kinds = vec![water];
        f.fill(Vec2::new(-2.0, 0.0), Vec2::new(2.0, 1.5), 0);
        (w, f)
    }

    #[test]
    fn a_tank_of_it_settles_and_keeps_its_level() {
        let (mut w, mut f) = tank();
        let n = f.drops.len();
        for _ in 0..240 {
            w.step(1.0 / 60.0);
            f.step(1.0 / 60.0, &mut w);
        }
        let top = f.drops.iter().map(|d| d.pos.y).fold(f32::MIN, f32::max);
        let speed = f.drops.iter().map(|d| d.vel.length()).fold(0.0f32, f32::max);
        let low = f.drops.iter().map(|d| d.pos.y).fold(f32::MAX, f32::min);
        eprintln!("{n} drops: level {top:.2} (filled to 1.5), lowest {low:.2}, fastest {speed:.2} m/s");
        assert!(f.drops.iter().all(|d| d.pos.x > -2.0 && d.pos.x < 2.0 && d.pos.y > 0.0), "all in the tank");
        assert!(top > 1.2 && top < 1.75, "holds its volume ({top:.2})");
        assert!(speed < 1.5, "and settles ({speed:.2})");
    }

    #[test]
    fn spreading_it_over_threads_changes_nothing() {
        let run = |parallel_from: usize| {
            let (mut w, mut f) = tank();
            f.parallel_from = parallel_from;
            f.drops[0].vel = Vec2::new(3.0, 5.0);
            for _ in 0..90 {
                w.step(1.0 / 60.0);
                f.step(1.0 / 60.0, &mut w);
            }
            let mut h = Vec::new();
            f.hash_into(|v| h.push(v));
            h
        };
        assert_eq!(run(usize::MAX), run(0), "one thread and many should agree to the bit");
    }

    #[test]
    fn it_is_deterministic() {
        let run = || {
            let (mut w, mut f) = tank();
            f.drops[0].vel = Vec2::new(3.0, 5.0);
            for _ in 0..60 {
                w.step(1.0 / 60.0);
                f.step(1.0 / 60.0, &mut w);
            }
            let mut h = Vec::new();
            f.hash_into(|v| h.push(v));
            h
        };
        assert_eq!(run(), run());
    }

    #[test]
    #[ignore]
    fn tune() {
        for &(p, pn, sub, visc, vsq) in &[(2000.0, 4000.0, 4, 0.3, 0.3), (2000.0, 4000.0, 3, 0.3, 0.3), (2500.0, 5000.0, 3, 0.3, 0.3)] {
            let mut w = World::new(WorldConfig::default());
            w.add_static(Obb::from_min_max(Vec2::new(-3.0, -1.0), Vec2::new(3.0, 0.0)));
            w.add_static(Obb::from_min_max(Vec2::new(-3.0, 0.0), Vec2::new(-2.5, 4.0)));
            w.add_static(Obb::from_min_max(Vec2::new(2.5, 0.0), Vec2::new(3.0, 4.0)));
            let k = FluidKind { pressure: p, near_pressure: pn, viscosity: visc, viscosity_sq: vsq, mass: 0.3, friction: 0.05, density: 1.0, dye_spread: 0.05 };
            let sp: f32 = std::env::var("SPACING").ok().and_then(|v| v.parse().ok()).unwrap_or(0.1);
            let mut f = Fluid::new(vec![k], sp);
            f.substeps = sub;
            f.fill(Vec2::new(-2.5, 0.0), Vec2::new(2.5, 2.2), 0);
            let t = std::time::Instant::now();
            let mut worst = 0.0f32;
            for i in 0..360 {
                w.step(1.0 / 60.0);
                f.step(1.0 / 60.0, &mut w);
                if i > 240 {
                    worst = worst.max(f.drops.iter().map(|d| d.vel.length()).fold(0.0f32, f32::max));
                }
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / 360.0;
            let mut ys: Vec<f32> = f.drops.iter().map(|d| d.pos.y).collect();
            ys.sort_by(|a, b| a.total_cmp(b));
            eprintln!("p {p} pn {pn} sub {sub} visc {visc}/{vsq}: level {:.2} (2.2), late max speed {worst:.2}, {:.2} ms/frame for {} drops", ys[ys.len() * 97 / 100], ms, f.drops.len());
        }
    }
}
