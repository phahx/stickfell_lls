//! Scripted "feel" checks: drive a fighter with fixed inputs in a flat
//! sandbox and measure what a player would notice (jitter, drift, speed,
//! knockdowns, weapons misbehaving). Run with `--nocapture` to see numbers.

use super::*;
use crate::fighter::{self, HEAD, NECK, SWING_FRAMES};
use crate::weapon::WEAPONS;

mod liquids;
mod finishers;
mod hazards;
mod climbing;
mod bare_hands;
mod weapons;
mod movement;
mod arenas;

/// Flat floor from x=-40..40 at y=0 plus optional boxes; fighter 0 at x=0,
/// fighter 1 parked far away (x=30) so it doesn't interfere.
fn sandbox(boxes: &[(Vec2, Vec2)]) -> GameState {
    let mut w = World::new(WorldConfig::default());
    w.add_static(Obb::from_min_max(Vec2::new(-40.0, -2.0), Vec2::new(40.0, 0.0)).with_friction(0.9));
    for &(min, max) in boxes {
        w.add_static(Obb::from_min_max(min, max).with_friction(0.9));
    }
    let fighters = vec![
        Fighter::spawn(&mut w, 0, Vec2::new(0.0, 0.02), 1.0),
        Fighter::spawn(&mut w, 1, Vec2::new(30.0, 0.02), -1.0),
    ];
    let mut s = GameState::new(2, 0, 1);
    s.spike_pins = GameState::make_spike_pins(&mut w, 2);
    s.world = w;
    s.fighters = fighters;
    s.weapons.clear();
    s.lamps.clear();
    s.ropes.clear();
    s.balls.clear();
    s.hazards.clear();
    s.kill_y = -20.0;
    s.min = Vec2::new(-40.0, -10.0);
    s.max = Vec2::new(40.0, 20.0);
    s.next_drop = u32::MAX;
    s
}

fn step(s: &mut GameState, buttons: u16, aim: Vec2) {
    s.step(&[input::encode(buttons, aim), 0]);
}

fn give(s: &mut GameState, def: u8) -> usize {
    let hand = s.fighters[0].pos(&s.world, HAND_A);
    s.spawn_weapon(def, hand + Vec2::new(0.1, 0.0));
    let wi = s.weapons.len() - 1;
    step(s, PICKUP, Vec2::new(0.6, 0.0));
    assert_eq!(s.fighters[0].weapon, Some(wi as u8), "pickup failed");
    wi
}

fn tilt(s: &GameState) -> f32 {
    let f = &s.fighters[0];
    let d = f.pos(&s.world, NECK) - f.pos(&s.world, PELVIS);
    dmath::angle_diff(dmath::angle(d), core::f32::consts::FRAC_PI_2).abs()
}

fn weapon_speeds(s: &GameState, wi: usize) -> (f32, f32) {
    let w = &s.weapons[wi];
    let v: Vec<f32> = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].vel.length()).collect();
    (v.iter().copied().fold(0.0, f32::max), v.iter().sum::<f32>() / v.len() as f32)
}

/// Largest change of any pairwise particle distance vs. the given reference.
fn shape_error(s: &GameState, wi: usize, rest: &[Vec2]) -> f32 {
    let w = &s.weapons[wi];
    let pos: Vec<Vec2> = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].pos).collect();
    let mut e: f32 = 0.0;
    for i in 0..pos.len() {
        for j in i + 1..pos.len() {
            e = e.max(((pos[i] - pos[j]).length() - (rest[i] - rest[j]).length()).abs());
        }
    }
    e
}

const MAX_HP: f32 = crate::fighter::MAX_HEALTH;

/// Fighter 0 armed with `def`, fighter 1 standing `gap` metres in front.
fn duel_setup(def: u8, gap: f32) -> (GameState, usize) {
    let mut s = sandbox(&[]);
    let off = Vec2::new(gap - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..30 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
    }
    let wi = give(&mut s, def);
    for _ in 0..20 {
        s.step(&[input::encode(0, Vec2::new(0.6, -0.05)), input::encode(0, Vec2::new(-0.6, 0.0))]);
    }
    // (a long weapon at rest may brush the target while setting up)
    s.fighters[1].health = MAX_HP;
    s.fighters[1].down = 0;
    (s, wi)
}

fn victim_speed(s: &GameState) -> f32 {
    s.fighters[1].body_vel(&s.world).length()
}

/// Both fighters armed with swords, `gap` apart, facing each other.
fn sword_duel(gap: f32) -> (GameState, usize, usize) {
    let (mut s, w0) = duel_setup(0, gap);
    let hand = s.fighters[1].pos(&s.world, HAND_A);
    s.spawn_weapon(0, hand + Vec2::new(-0.1, 0.0));
    let w1 = s.weapons.len() - 1;
    s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(PICKUP, Vec2::new(-0.6, 0.0))]);
    assert_eq!(s.fighters[1].weapon, Some(w1 as u8));
    for _ in 0..20 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
    }
    (s, w0, w1)
}

// ---- bare hands: grabs, throws, escapes, punches, kicks ----------------

/// Fighter 1 standing `gap` metres in front of fighter 0, both settled.
fn pair(gap: f32) -> GameState {
    let mut s = sandbox(&[]);
    let off = Vec2::new(gap - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..30 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
    }
    s
}

fn step2(s: &mut GameState, b0: u16, aim0: Vec2, b1: u16) {
    s.step(&[input::encode(b0, aim0), input::encode(b1, Vec2::new(-0.6, 0.0))]);
}

/// Grabs fighter 1 (aiming at their neck) and returns once held.
fn grab_them(s: &mut GameState) {
    for _ in 0..30 {
        let neck = s.fighters[1].pos(&s.world, NECK) - s.fighters[0].pos(&s.world, NECK);
        step2(s, GRAB, neck, 0);
        if s.fighters[0].hold.is_some() {
            return;
        }
    }
    panic!("couldn't get hold of them");
}

/// Run at fighter 1 from `gap` m, jump, and dive for their head with grab.
fn dive_input(s: &GameState, k: u32) -> u32 {
    let me = s.fighters[0].pos(&s.world, NECK);
    let head = s.fighters[1].pos(&s.world, HEAD);
    let b = match k {
        0..=5 => RIGHT,
        6..=12 => RIGHT | JUMP,
        _ if !s.fighters[0].grounded || s.fighters[0].flip > 0 => RIGHT | GRAB,
        _ => 0,
    };
    input::encode(b, head - me)
}

/// Moves fighter `i` (every particle, keeping its velocity) so its pelvis
/// is at `to`.
fn warp(s: &mut GameState, i: usize, to: Vec2) {
    let d = to - s.fighters[i].pos(&s.world, PELVIS);
    for id in s.fighters[i].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += d;
        p.prev += d;
    }
}

/// A simple scripted player runs the practice yard's rope course from the
/// top of the chimney: pump each rope, let go swinging forward, reach for
/// the next. Returns what it reached when; `each` sees every frame.
fn cross_rope_course(mut each: impl FnMut(&GameState, u32)) -> Vec<String> {
    use crate::level::{COURSE, COURSE_FINISH};
    let mut s = GameState::new(4, crate::level::TRAINING, 1);
    // course ropes come after the yard's own rope
    let ropes: Vec<Vec<u32>> = s.ropes[1..1 + COURSE.len()].to_vec();
    warp(&mut s, 0, Vec2::new(-22.5, 11.4));
    let drive = |s: &mut GameState, b: u16, aim: Vec2| {
        let mut inputs = vec![input::encode(b, aim)];
        inputs.extend((1..4).map(|i| crate::bot::dummy_input(s, i)));
        s.step(&inputs);
    };
    for _ in 0..40 {
        drive(&mut s, 0, Vec2::new(-0.6, 0.0));
    }
    assert!(s.fighters[0].grounded, "should be standing on top of the chimney");
    // which course rope (if any) the hand is on
    let on = |s: &GameState| -> Option<usize> {
        let f = &s.fighters[0];
        if !f.grabbing || !f.grab_static {
            return None;
        }
        let b = s.world.distances[f.grab as usize].b;
        ropes.iter().position(|c| c.contains(&b))
    };
    // aim the free hand at the nearest link of rope `k` (or the platform)
    let aim_at = |s: &GameState, k: usize| -> Vec2 {
        let neck = s.fighters[0].pos(&s.world, NECK);
        let to = if k < ropes.len() {
            ropes[k].iter().map(|&p| s.world.particles[p as usize].pos).min_by(|a, b| (*a - neck).length().total_cmp(&(*b - neck).length())).unwrap() - neck
        } else {
            Vec2::new(COURSE_FINISH.x, COURSE_FINISH.y + 1.0) - neck
        };
        to.normalize_or_zero() * 1.2
    };
    let mut reached = 0; // ropes caught so far
    let (mut off_for, mut hang_t) = (0, 0);
    let mut log = Vec::new();
    for frame in 0..2400 {
        each(&s, frame);
        let f = &s.fighters[0];
        let pelvis = f.pos(&s.world, PELVIS);
        let (vx, vy) = (f.body_vel(&s.world).x, f.body_vel(&s.world).y);
        let grounded = f.grounded;
        if let Some(k) = on(&s) {
            if k + 1 > reached {
                log.push(format!("rope {} at frame {frame}", k + 1));
                reached = k + 1;
                hang_t = 0;
            }
            hang_t += 1;
            // pump; let go swinging left once this swing will carry you to the
            // next rope (or the finish), like a player judging the jump
            let push = if vx >= 0.0 { RIGHT } else { LEFT };
            let neck = s.fighters[0].pos(&s.world, NECK);
            let targets: Vec<Vec2> = if k + 1 < ropes.len() {
                ropes[k + 1].iter().map(|&p| s.world.particles[p as usize].pos).collect()
            } else {
                vec![Vec2::new(COURSE_FINISH.x, COURSE_FINISH.y + 1.0)]
            };
            let v = s.fighters[0].body_vel(&s.world);
            let carries = (1..70).any(|i| {
                let t = i as f32 / 60.0;
                let at = neck + v * t + Vec2::new(0.0, -14.0) * t * t;
                targets.iter().any(|q| (*q - at).length() < 0.75)
            });
            if hang_t > 30 && vx < -2.0 && carries {
                { let a = aim_at(&s, k + 1); drive(&mut s, JUMP | LEFT, a); }
                off_for = 0;
            } else {
                { let a = aim_at(&s, k + 1); drive(&mut s, GRAB | push, a); }
            }
            continue;
        }
        off_for += 1;
        if s.fighters[0].grabbing && s.fighters[0].grab_static {
            // caught an edge instead: pull up onto it
            { let a = aim_at(&s, reached); drive(&mut s, JUMP | LEFT, a); }
            continue;
        }
        if std::env::var("COURSE_TRACE").is_ok() && reached > 0 {
            let hand = s.fighters[0].pos(&s.world, HAND_B);
            let k = reached.min(ropes.len() - 1);
            let near = ropes[k].iter().map(|&p| (s.world.particles[p as usize].pos - hand).length()).fold(f32::MAX, f32::min);
            eprintln!("{frame}: pelvis {:.2},{:.2} v {vx:.1},{vy:.1} hand->rope{} {near:.2} grabbing {}", pelvis.x, pelvis.y, k + 1, s.fighters[0].grabbing);
        }
        if grounded && pelvis.y > COURSE_FINISH.y && (pelvis.x - COURSE_FINISH.x).abs() < 3.0 {
            log.push(format!("finish at frame {frame}"));
            break;
        }
        if pelvis.y < 4.0 {
            panic!("fell off the course at x {:.1} after {:?}", pelvis.x, log);
        }
        if grounded && reached == 0 {
            // run off the chimney top and leap at the first rope
            let b = if pelvis.x < -25.3 { JUMP | GRAB | LEFT } else { LEFT };
            { let a = aim_at(&s, 0); drive(&mut s, b, a); }
        } else if off_for < 6 {
            { let a = aim_at(&s, reached); drive(&mut s, LEFT, a); }
        } else {
            { let a = aim_at(&s, reached); drive(&mut s, GRAB | LEFT, a); }
        }
    }
    log
}

/// Two fighters 1 m apart under a low ceiling (top at 5 m), the attacker
/// holding a grapple and the other down and badly hurt.
fn noose_setup() -> (GameState, usize) {
    let mut s = sandbox(&[(Vec2::new(-4.0, 4.6), Vec2::new(6.0, 5.0))]);
    let off = Vec2::new(1.0 - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, crate::weapon::GRAPPLE);
    s.fighters[1].health = 30.0;
    s.fighters[1].knock_down(120);
    s.fighters[1].dazed = 200;
    (s, wi)
}

/// Inputs for the attacker in the noose scenario at frame `k`: finish them,
/// wait for the knot, then fire the hook straight up.
fn noose_input(k: u32) -> u32 {
    let up = Vec2::new(0.1, 1.2);
    match k {
        0..=1 => input::encode(FINISH, Vec2::new(0.6, 0.0)),
        2..=129 => input::encode(0, up),
        130 => input::encode(ATTACK, up),
        _ => input::encode(0, up),
    }
}

/// A sword fighter at x=0 and a beaten victim at `victim_x`; with `wall`, a
/// tall wall starts at x=2.6 behind them.
fn pin_setup(wall: bool, victim_x: f32) -> (GameState, usize) {
    let boxes: &[(Vec2, Vec2)] = if wall { &[(Vec2::new(2.6, 0.0), Vec2::new(3.4, 5.0))] } else { &[] };
    let mut s = sandbox(boxes);
    warp(&mut s, 1, Vec2::new(victim_x, 0.98));
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    assert_eq!(WEAPONS[0].name, "SWORD");
    let wi = give(&mut s, 0);
    s.fighters[1].health = 30.0;
    s.fighters[1].knock_down(120);
    s.fighters[1].dazed = 200;
    (s, wi)
}

fn pin_input(k: u32) -> u32 {
    input::encode(if k < 2 { FINISH } else { 0 }, Vec2::new(0.6, 0.0))
}

/// The pin scenario with a third fighter far off, so the round goes on
/// after the kill (as in practice, or a 3-4 player match).
fn pin_setup_ongoing() -> (GameState, usize) {
    let (two, _) = pin_setup(true, 1.5);
    let mut s = GameState::new(3, 0, 1);
    s.world = two.world.clone();
    let mut fighters = two.fighters.clone();
    fighters.push(Fighter::spawn(&mut s.world, 2, Vec2::new(-30.0, 0.02), 1.0));
    s.fighters = fighters;
    s.weapons = two.weapons.clone();
    s.lamps.clear();
    s.ropes.clear();
    s.balls.clear();
    s.hazards.clear();
    s.kill_y = -20.0;
    s.min = Vec2::new(-40.0, -10.0);
    s.max = Vec2::new(40.0, 20.0);
    s.next_drop = u32::MAX;
    s.fighters[1].health = 30.0;
    s.fighters[1].knock_down(120);
    s.fighters[1].dazed = 200;
    (s, 0)
}

/// After the pin, walks fighter 0 up to the sword's handle and grabs it;
/// returns the grabber's top speed over the next `frames` and the state.
fn grab_pinned_sword(frames: u32, hold: bool) -> (f32, GameState) {
    let (mut s, wi) = pin_setup_ongoing();
    for k in 0..200u32 {
        s.step(&[pin_input(k), 0, 0]);
    }
    assert!(s.weapons[wi].pinned, "set up: pinned");
    let grip = s.world.particles[s.weapons[wi].particle(1) as usize].pos;
    for _ in 0..90 {
        let me = s.fighters[0].pos(&s.world, PELVIS);
        let b = if grip.x - me.x > 0.6 { input::RIGHT } else { 0 };
        let aim = grip - s.fighters[0].pos(&s.world, NECK);
        s.step(&[input::encode(b, aim), 0, 0]);
    }
    let mut top: f32 = 0.0;
    for k in 0..frames {
        let aim = if hold { grip - s.fighters[0].pos(&s.world, NECK) } else { Vec2::new(-1.0, 0.0) };
        s.step(&[input::encode(if k == 0 { PICKUP } else { 0 }, aim), 0, 0]);
        top = top.max(s.fighters[0].body_vel(&s.world).length());
    }
    (top, s)
}

/// The lever climb: stab the wall, sweep the mouse down to lever yourself up
/// on the stuck blade, W to let go (the heave pulls it out), stab higher, repeat.
fn lever_climb(trace: bool) -> (f32, Vec<f32>, f32) {
    let mut s = sandbox(&[(Vec2::new(1.0, 0.0), Vec2::new(3.0, 12.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    let start = s.fighters[0].pos(&s.world, PELVIS).y;
    let mut peak = start;
    let mut stuck_at = Vec::new();
    let mut phase = 0; // 0 stab, 1 lever down, 2 let go
    let mut t = 0u32;
    let mut first: f32 = 0.0;
    for k in 0..900u32 {
        t += 1;
        let (b, aim) = match phase {
            0 => (if t % 6 == 1 { ALT } else { 0 }, Vec2::new(0.6, 0.35)),
            1 => {
                // sweep the aim from forward to straight down over ~half a second
                let a = (t as f32 / 30.0).min(1.0);
                (0, Vec2::new(0.6 * (1.0 - a) + 0.05, 0.35 - 1.2 * a))
            }
            _ => (JUMP, Vec2::new(0.6, 0.6)),
        };
        step(&mut s, b, aim);
        let y = s.fighters[0].pos(&s.world, PELVIS).y;
        peak = peak.max(y);
        if phase == 1 && stuck_at.len() == 1 {
            first = first.max(y - start);
        }
        if trace && k % 6 == 0 {
            eprintln!("k{k} phase {phase} t{t} pelvis {y:.2} embedded {} hang {} grounded {}", s.weapons[wi].embedded, s.fighters[0].hang_blade, s.fighters[0].grounded);
        }
        match phase {
            0 if s.weapons[wi].embedded => {
                stuck_at.push(s.world.particles[s.weapons[wi].particle(3) as usize].pos.y);
                phase = 1;
                t = 0;
            }
            0 if t > 90 => break,
            1 if t >= 40 => {
                phase = 2;
                t = 0;
            }
            2 if !s.weapons[wi].embedded || t > 20 => {
                phase = 0;
                t = 0;
            }
            _ => {}
        }
    }
    (peak - start, stuck_at, first)
}

/// Grab them, turn your back on them, Q: returns the state each frame of the
/// throw and after (fighter 0 starts on the left, so they land on the left).
fn shoulder_throw_input(s: &GameState, k: u32) -> u32 {
    let neck = s.fighters[1].pos(&s.world, NECK) - s.fighters[0].pos(&s.world, NECK);
    let (b, aim) = match k {
        _ if s.fighters[0].hold.is_none() && k < 30 => (GRAB, neck),
        _ if s.fighters[0].shoulder == 0 && s.fighters[0].facing > 0.0 && k < 60 => (GRAB, Vec2::new(-0.6, 0.35)),
        _ if s.fighters[0].shoulder == 0 && s.fighters[1].down == 0 && k < 60 => (GRAB | THROW, Vec2::new(-0.6, 0.35)),
        _ => (GRAB, Vec2::new(-0.6, 0.35)),
    };
    input::encode(b, aim)
}

/// Shoulder-throws fighter 1 (weakened, so they can be finished), then presses
/// FINISH `delay` frames after the slam. Returns once the finisher (if any)
/// has started, with the frame count so far.
fn shoulder_then_finish(s: &mut GameState, delay: u32) -> u32 {
    s.fighters[1].health = 50.0;
    let mut released = None;
    for k in 0..400u32 {
        let thrown = s.fighters[0].shoulder > 0;
        let b = match released {
            Some(r) if k == r + delay => input::encode(FINISH, Vec2::new(-0.6, 0.0)),
            Some(_) => input::encode(0, Vec2::new(-0.6, 0.0)),
            None => shoulder_throw_input(s, k),
        };
        s.step(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]);
        if released.is_none() && thrown && s.fighters[0].shoulder == 0 {
            released = Some(k);
        }
        if released.is_some_and(|r| k >= r + delay) {
            return k;
        }
    }
    panic!("the shoulder throw never happened");
}

/// A close-up of the world around a point, nothing over it (for checking art).
struct CloseUp {
    game: GameState,
    at: Vec2,
}

/// A wider view of the arena (`height` m tall) for screenshots.
struct Wide {
    game: GameState,
    at: Vec2,
    height: f32,
}

impl stickfell_app::Game for Wide {
    fn tick(&mut self, _input: &stickfell_app::Input) {}
    fn draw(&mut self, frame: &mut stickfell_render::Frame, screen: Vec2, _alpha: f32, _dt: f32) {
        let levels = crate::level::levels();
        let lv = &levels[self.game.level_index];
        crate::draw::draw_game(&self.game, lv, frame, screen, self.at, &crate::draw::Hud { world_only: true, zoom: self.height, ..Default::default() });
    }
}

impl stickfell_app::Game for CloseUp {
    fn tick(&mut self, _input: &stickfell_app::Input) {}
    fn draw(&mut self, frame: &mut stickfell_render::Frame, screen: Vec2, _alpha: f32, _dt: f32) {
        let levels = crate::level::levels();
        let lv = &levels[self.game.level_index];
        crate::draw::draw_game(&self.game, lv, frame, screen, self.at, &crate::draw::Hud { world_only: true, zoom: 2.2, ..Default::default() });
    }
}

/// One fighter in an empty sandbox with a 2 m spike bed (0.375 m tall, the
/// kit's) on the floor to the right, from x 1 to 3.
fn spike_yard() -> GameState {
    use crate::level::{HazardDef, HazardKind};
    let mut s = sandbox(&[]);
    let a = Vec2::new(2.0, 0.1875);
    s.add_hazard(HazardDef { kind: HazardKind::Spikes, a, b: a, size: Vec2::new(1.0, 0.1875), speed: 0.0 });
    s
}

fn teleport(s: &mut GameState, fi: usize, by: Vec2) {
    for id in s.fighters[fi].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += by;
        p.prev = p.pos;
        p.vel = Vec2::ZERO;
    }
}

/// Lays fighter `fi` flat on its back (head to the right), its pelvis at
/// `at`, still.
fn lay_flat(s: &mut GameState, fi: usize, at: Vec2) {
    let pelvis = s.fighters[fi].pos(&s.world, PELVIS);
    for id in s.fighters[fi].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        let d = p.pos - pelvis;
        p.pos = at + Vec2::new(d.y, -d.x * 0.3);
        p.prev = p.pos;
        p.vel = Vec2::ZERO;
    }
    // limp, so it comes down the way it was dropped
    s.fighters[fi].down = 200;
}

/// A body dropped flat from `height` m over the middle of the spike bed.
fn drop_flat_on_spikes(height: f32) -> GameState {
    let mut s = spike_yard();
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let tips = s.hazards[0].teeth()[0].tip.y;
    lay_flat(&mut s, 0, Vec2::new(1.6, tips + 0.15 + height));
    s
}

/// The practice yard with one fighter dropped (limp) into its pool or lava
/// pit from `height` m over the surface.
fn dropped_in(kind: crate::level::HazardKind, height: f32) -> (GameState, crate::level::HazardDef) {
    // (with its dummies: alone in the yard, a round is over at once)
    let mut s = GameState::new(4, crate::level::TRAINING, 7);
    for _ in 0..20 {
        s.step(&[0]);
    }
    let h = *s.hazards.iter().find(|h| h.kind == kind).unwrap();
    lay_flat(&mut s, 0, Vec2::new(h.a.x, h.a.y + h.size.y + height));
    (s, h)
}

/// The highest drop over a liquid hazard's span (its surface, roughly).
fn liquid_top(s: &GameState, h: &crate::level::HazardDef) -> f32 {
    s.fluid.drops.iter().filter(|d| (d.pos.x - h.a.x).abs() < h.size.x).map(|d| d.pos.y).fold(f32::MIN, f32::max)
}

/// The yard's pool, settled, with a fighter standing well clear of it.
fn still_pool() -> (GameState, crate::level::HazardDef) {
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 0.0);
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(-12.0 - me.x, 1.0 - me.y));
    for _ in 0..60 {
        s.step(&[0]);
    }
    (s, h)
}

/// How far from `at` the dye has reached (drops with at least `min` of it),
/// and how many drops that is.
fn stain(s: &GameState, at: Vec2, min: f32) -> (f32, usize) {
    let dyed: Vec<f32> = s.fluid.drops.iter().filter(|d| d.dye >= min).map(|d| (d.pos - at).length()).collect();
    (dyed.iter().copied().fold(0.0, f32::max), dyed.len())
}

/// The practice yard with you up on the chimney, by the flood lever.
fn at_the_lever() -> GameState {
    let mut s = GameState::new(4, crate::level::TRAINING, 7);
    let def = s.trap.def.expect("the yard has a flood trap");
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(def.lever.x - 0.45 - me.x, def.lever.y + 1.0 - me.y));
    for _ in 0..40 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0))]);
    }
    s
}

/// Works the lever (E) from where `at_the_lever` left you.
fn throw_lever(s: &mut GameState) {
    for k in 0..10 {
        let b = if k == 5 { PICKUP } else { 0 };
        let mut inputs = vec![0; s.fighters.len()];
        inputs[0] = input::encode(b, Vec2::new(0.6, 0.2));
        s.step(&inputs);
    }
}

/// Both fighters standing in a walled tank of water `depth` m deep (they're
/// 0.9 m apart, as for a shoulder throw).
fn wading(depth: f32) -> GameState {
    use crate::level::{HazardDef, HazardKind};
    let mut s = sandbox(&[(Vec2::new(-6.5, 0.0), Vec2::new(-6.0, 4.0)), (Vec2::new(6.0, 0.0), Vec2::new(6.5, 4.0))]);
    let off = Vec2::new(0.9 - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    let a = Vec2::new(0.0, depth * 0.5);
    s.add_hazard(HazardDef { kind: HazardKind::Water, a, b: a, size: Vec2::new(6.0, depth * 0.5), speed: 0.0 });
    s
}

/// The torso's angle from straight up (radians, 0 upright, pi/2 flat).
fn torso_tilt(s: &GameState, fi: usize) -> f32 {
    let f = &s.fighters[fi];
    let d = f.pos(&s.world, NECK) - f.pos(&s.world, PELVIS);
    stickfell_physics::dmath::angle_diff(stickfell_physics::dmath::angle(d), core::f32::consts::FRAC_PI_2).abs()
}

/// A swimmer at the surface of the yard's pool, `from` m short of its right
/// edge, facing it.
fn swimmer_by_the_edge(from: f32) -> (GameState, crate::level::HazardDef) {
    let (mut s, h) = still_pool();
    let edge = h.a.x + h.size.x;
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(edge - from, h.a.y + h.size.y - 0.3) - me);
    // (`still_pool` left them laid out flat: up and about)
    s.fighters[0].down = 0;
    for _ in 0..90 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0))]);
    }
    (s, h)
}

/// The top of the floor just right of the pool.
fn s_ground(h: &crate::level::HazardDef) -> f32 {
    let (s, _) = still_pool();
    let x = h.a.x + h.size.x + 0.5;
    s.world.raycast(Vec2::new(x, 5.0), Vec2::NEG_Y, 20.0).map_or(f32::NAN, |hit| hit.point.y)
}

/// A tank 12 m wide with 2 m of water held up in its left 5 m, let go
/// against fighter 0 standing at x -0.6 (braced against a step behind them,
/// or not).
fn dam(braced: bool) -> GameState {
    let mut walls = vec![(Vec2::new(-6.5, 0.0), Vec2::new(-6.0, 4.0)), (Vec2::new(6.0, 0.0), Vec2::new(6.5, 4.0))];
    if braced {
        walls.push((Vec2::new(-0.25, 0.0), Vec2::new(0.25, 0.35)));
    }
    let mut s = sandbox(&walls);
    // (the other one well out of it, on the floor: x 35)
    teleport(&mut s, 1, Vec2::new(5.0, 0.0));
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(-0.6 - me.x, 0.0));
    for _ in 0..60 {
        s.step(&[input::encode(0, Vec2::new(-0.6, 0.0)), 0]);
    }
    s.fluid.fill(Vec2::new(-6.0, 0.0), Vec2::new(-1.1, 2.0), crate::liquid::WATER_DROP);
    s
}

/// A fall: off a ledge `h` high (0: flat ground), running right, jumping as
/// they go over (and again at the top of it, `double`). How hard they hit
/// the ground (the most any part touching it was stopped, m/s) and whether
/// they went down for it.
fn landing(h: f32, jump: bool, double: bool) -> (f32, bool) {
    let boxes: Vec<(Vec2, Vec2)> = if h > 0.0 { vec![(Vec2::new(-8.0, 0.0), Vec2::new(0.0, h))] } else { vec![] };
    let mut s = sandbox(&boxes);
    let lift = s.fighters[0].pos(&s.world, PELVIS).y;
    warp(&mut s, 0, Vec2::new(-1.5, h + lift + 0.05));
    for _ in 0..40 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let (mut hardest, mut down, mut left, mut jumped) = (0.0f32, false, None, 0);
    for k in 0..200 {
        let x = s.fighters[0].pos(&s.world, PELVIS).x;
        let mut b = RIGHT;
        if jump && jumped == 0 && x > -0.4 {
            b |= JUMP;
            jumped = k;
        }
        if double && jumped > 0 && k == jumped + 22 {
            b |= JUMP;
        }
        step(&mut s, b, Vec2::new(0.6, 0.0));
        if left.is_none() && (x > 0.2 || h == 0.0 && jumped > 0) {
            left = Some(k);
        }
        if left.is_some_and(|l| k > l + 3) {
            hardest = hardest.max(s.landing_speed(0));
            down |= s.fighters[0].down > 0;
        }
    }
    (hardest, down)
}

/// Fastest any part of fighter 0 is going (m/s), but the foot a sweep
/// kicks with.
fn fastest_part(s: &GameState) -> f32 {
    let f = &s.fighters[0];
    let kicking = f.p(fighter::FOOT_A);
    f.particle_ids().filter(|&id| id != kicking).map(|id| s.world.particles[id as usize].vel.length()).fold(0.0, f32::max)
}

/// A scripted player climbing onto a ledge `h` high (a block from x = 1.5
/// on) with bare hands: run at it, jump at `jump_at` (and double jump
/// `double` frames later, if any), reach for the top corner holding grab,
/// and press jump `pull` frames after catching hold, still holding right
/// and grab (a player keeps the keys down). Did they end up on top, and
/// did they catch hold?
fn ledge_climb(h: f32, jump_at: u32, double: Option<u32>, pull: u32, let_go: bool) -> (bool, bool) {
    let mut s = sandbox(&[(Vec2::new(1.5, 0.0), Vec2::new(24.0, h))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let corner = Vec2::new(1.5, h);
    let (mut caught, mut caught_at) = (false, None);
    for k in 0..240u32 {
        let f = &s.fighters[0];
        let neck = f.pos(&s.world, NECK);
        let mut b = RIGHT;
        if k == jump_at || double.is_some_and(|d| k == jump_at + d) {
            b |= JUMP;
        }
        if k > jump_at + 3 {
            b |= GRAB;
        }
        if f.grabbing && f.grab_static && caught_at.is_none() {
            caught = true;
            caught_at = Some(k);
        }
        if let Some(c) = caught_at {
            if k == c + pull {
                b |= JUMP;
            }
            if let_go && k > c + pull {
                b &= !GRAB;
            }
        }
        step(&mut s, b, corner + Vec2::new(0.1, 0.1) - neck);
    }
    let f = &s.fighters[0];
    let p = f.pos(&s.world, PELVIS);
    (p.y > h + 0.3 && p.x > 1.6 && f.down == 0, caught)
}

/// A body in an acid vat (sandbox) or the yard's lava pit: how each part
/// fares over `secs`, then (lava) a while out of it.
fn rot_report(s: &GameState) -> String {
    let f = &s.fighters[0];
    let full = fighter::ROT_FULL as f32;
    let names = ["head", "torso", "upA", "foreA", "upB", "foreB", "thighA", "shinA", "thighB", "shinB"];
    (0..10).map(|k| format!("{} e{:.2} m{:.2} f{}{}", names[k], f.eaten[k] as f32 / full, f.melted[k] as f32 / full, f.fire[k], if f.has(k as u32) { "" } else { " OFF" })).collect::<Vec<_>>().join(" | ")
}

/// The rope course arena: a player on platform `from` swings across the gap
/// to the next platform `dir` (+1 right, -1 left) on its two ropes, the way
/// the practice course is crossed: leap at the first rope, pump, let go when
/// the swing will carry them to the next rope (or the platform), and pull
/// up onto the edge if that's what they catch. Frames taken, or why not.
fn cross_rope_gap(from: usize, dir: f32) -> Result<u32, String> {
    use crate::level::{ROPE_HALF, rope_platform};
    let mut s = GameState::new(2, crate::level::ROPES, 3);
    let to = if dir > 0.0 { from + 1 } else { from - 1 };
    let gap = from.min(to);
    // (the level's ropes in order: two in each gap)
    let ropes: Vec<Vec<u32>> = if dir > 0.0 { vec![s.ropes[gap * 2].clone(), s.ropes[gap * 2 + 1].clone()] } else { vec![s.ropes[gap * 2 + 1].clone(), s.ropes[gap * 2].clone()] };
    let start = rope_platform(from);
    let goal = rope_platform(to);
    warp(&mut s, 0, start + Vec2::new(0.0, 1.0));
    // (the other fighter idles on a platform far off)
    warp(&mut s, 1, rope_platform(if from < 4 { 7 } else { 0 }) + Vec2::new(0.0, 1.0));
    let (towards, back) = if dir > 0.0 { (RIGHT, LEFT) } else { (LEFT, RIGHT) };
    let drive = |s: &mut GameState, b: u16, aim: Vec2| s.step(&[input::encode(b, aim), 0]);
    for _ in 0..40 {
        drive(&mut s, 0, Vec2::new(0.6 * dir, 0.0));
    }
    if !s.fighters[0].grounded {
        return Err("not standing on the platform to start".into());
    }
    let on = |s: &GameState| -> Option<usize> {
        let f = &s.fighters[0];
        if !f.grabbing || !f.grab_static {
            return None;
        }
        let b = s.world.distances[f.grab as usize].b;
        ropes.iter().position(|c| c.contains(&b))
    };
    let aim_at = |s: &GameState, k: usize| -> Vec2 {
        let neck = s.fighters[0].pos(&s.world, NECK);
        let to = if k < ropes.len() {
            ropes[k].iter().map(|&p| s.world.particles[p as usize].pos).min_by(|a, b| (*a - neck).length().total_cmp(&(*b - neck).length())).unwrap() - neck
        } else {
            goal + Vec2::new(0.0, 1.0) - neck
        };
        to.normalize_or_zero() * 1.2
    };
    let (mut reached, mut off_for, mut hang_t) = (0, 0, 0);
    for frame in 0..1500u32 {
        let f = &s.fighters[0];
        let pelvis = f.pos(&s.world, PELVIS);
        let vx = f.body_vel(&s.world).x * dir;
        if let Some(k) = on(&s) {
            if k + 1 > reached {
                reached = k + 1;
                hang_t = 0;
            }
            hang_t += 1;
            let push = if vx >= 0.0 { towards } else { back };
            let neck = s.fighters[0].pos(&s.world, NECK);
            let targets: Vec<Vec2> = if k + 1 < ropes.len() { ropes[k + 1].iter().map(|&p| s.world.particles[p as usize].pos).collect() } else { vec![goal + Vec2::new(0.0, 1.0)] };
            // (letting go of a rope leaps up off it at 6.5 m/s: see `actions`)
            let v = s.fighters[0].body_vel(&s.world) + Vec2::new(0.0, 6.5);
            // (to the next rope: the arc passes by it; to the platform: it
            // comes down on it, not over it, as a player judges the leap)
            let carries = if k + 1 < ropes.len() {
                (1..70).any(|i| {
                    let t = i as f32 / 60.0;
                    let at = neck + v * t + Vec2::new(0.0, -14.0) * t * t;
                    targets.iter().any(|q| (*q - at).length() < 0.75)
                })
            } else {
                let arc = |t: f32| pelvis + v * t + Vec2::new(0.0, -14.0) * t * t;
                let land = (1..120).map(|i| arc(i as f32 / 60.0)).find(|at| at.y < goal.y + 1.0);
                land.is_some_and(|at| (at.x - goal.x).abs() < ROPE_HALF - 1.0)
            };
            let a = aim_at(&s, k + 1);
            if std::env::var("GAP_TRACE").is_ok() && k == 1 {
                eprintln!("{frame}: on rope {k} hang {hang_t} pelvis ({:.2},{:.2}) v ({:.1},{:.1}) carries {carries}", pelvis.x, pelvis.y, v.x, v.y);
            }
            if hang_t > 30 && vx > 2.0 && carries {
                drive(&mut s, JUMP | towards, a);
                off_for = 0;
            } else {
                drive(&mut s, GRAB | push, a);
            }
            continue;
        }
        off_for += 1;
        if std::env::var("GAP_TRACE").is_ok() && frame % 3 == 0 {
            let f = &s.fighters[0];
            eprintln!("{frame}: pelvis ({:.2},{:.2}) v ({:.1},{:.1}) reached {reached} grabbing {} static {} grounded {} down {}", pelvis.x, pelvis.y, f.body_vel(&s.world).x, f.body_vel(&s.world).y, f.grabbing, f.grab_static, f.grounded, f.down);
        }
        if s.fighters[0].grabbing && s.fighters[0].grab_static {
            let a = aim_at(&s, reached);
            drive(&mut s, JUMP | towards, a);
            continue;
        }
        let grounded = s.fighters[0].grounded;
        if grounded && (pelvis.x - goal.x).abs() < ROPE_HALF && pelvis.y > goal.y {
            return Ok(frame);
        }
        if pelvis.y < goal.y - 3.0 {
            return Err(format!("fell, at x {:.1}, having caught {reached} of the ropes", pelvis.x));
        }
        let a = aim_at(&s, reached);
        if grounded && reached == 0 {
            // run off the edge and leap at the first rope
            let past = (pelvis.x - start.x) * dir > ROPE_HALF - 0.6;
            drive(&mut s, if past { JUMP | GRAB | towards } else { towards }, a);
        } else if off_for < 6 {
            drive(&mut s, towards, a);
        } else {
            drive(&mut s, GRAB | towards, a);
        }
    }
    Err(format!("ran out of time, having caught {reached} ropes"))
}
