//! Scripted "feel" checks: drive a fighter with fixed inputs in a flat
//! sandbox and measure what a player would notice (jitter, drift, speed,
//! knockdowns, weapons misbehaving). Run with `--nocapture` to see numbers.

use super::*;
use crate::fighter::{self, HEAD, NECK, SWING_FRAMES};
use crate::weapon::WEAPONS;

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

#[test]
fn walk_stop_turn_jump() {
    let mut s = sandbox(&[]);
    for _ in 0..60 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let mut max_tilt: f32 = 0.0;
    let mut downs = 0;
    for _ in 0..90 {
        step(&mut s, RIGHT, Vec2::new(0.6, 0.0));
        max_tilt = max_tilt.max(tilt(&s));
        downs += (s.fighters[0].down > 0) as u32;
    }
    let vx = s.fighters[0].body_vel(&s.world).x;
    for _ in 0..40 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        downs += (s.fighters[0].down > 0) as u32;
    }
    let vstop = s.fighters[0].body_vel(&s.world).x;
    for _ in 0..60 {
        step(&mut s, LEFT, Vec2::new(-0.6, 0.0));
        max_tilt = max_tilt.max(tilt(&s));
        downs += (s.fighters[0].down > 0) as u32;
    }
    let vleft = s.fighters[0].body_vel(&s.world).x;
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(-0.6, 0.0));
    }
    let y0 = s.fighters[0].pos(&s.world, PELVIS).y;
    let mut peak = y0;
    for i in 0..70 {
        step(&mut s, if i < 30 { JUMP } else { 0 }, Vec2::new(0.6, 0.0));
        peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
        downs += (s.fighters[0].down > 0) as u32;
    }
    eprintln!("walk: vx {vx:.2} stop {vstop:.2} left {vleft:.2} max_tilt {max_tilt:.2} jump {:.2} downs {downs}", peak - y0);
    assert!(vx > 5.0 && vleft < -5.0, "should run at full speed");
    assert!(vstop.abs() < 1.0, "should stop");
    assert!(max_tilt < 0.6, "too wobbly while running: {max_tilt}");
    assert!(peak - y0 > 2.0, "jump too low");
    assert_eq!(downs, 0, "fell over while just moving");
}

#[test]
fn no_automatic_climbing_and_wall_jumps() {
    // a knee-high crate (0.5 m): walking into it never lifts you onto it
    let mut s = sandbox(&[(Vec2::new(1.2, 0.0), Vec2::new(3.0, 0.5))]);
    let mut max_y: f32 = 0.0;
    for i in 0..90 {
        step(&mut s, if i > 20 { RIGHT } else { 0 }, Vec2::new(0.6, 0.0));
        max_y = max_y.max(s.fighters[0].pos(&s.world, PELVIS).y);
    }
    eprintln!("walk into a 0.5 m crate: highest pelvis {max_y:.2}");
    assert!(max_y < 1.25, "got hoisted onto a crate without jumping");

    // a chimney between two walls 2.4 m apart: wall jumps climb it
    let walls = [(Vec2::new(-2.4, 0.0), Vec2::new(-1.2, 12.0)), (Vec2::new(1.2, 0.0), Vec2::new(2.4, 12.0))];
    let mut s = sandbox(&walls);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let mut peak: f32 = 0.0;
    let mut dir = RIGHT;
    let mut used = 0;
    for k in 0..140 {
        let f = &s.fighters[0];
        let wall = f.wall_beside(&s.world);
        let mut b = dir;
        if k == 2 {
            b |= JUMP;
        }
        // jump off a wall once we're against it on the way up/down
        if wall.is_some() && !f.grounded && f.jump_cd == 0 && k % 2 == 0 {
            b |= JUMP;
            dir = if dir == RIGHT { LEFT } else { RIGHT };
        }
        let before = s.fighters[0].wall_jumps;
        step(&mut s, b, Vec2::new(0.6, 0.0));
        if s.fighters[0].wall_jumps > before {
            used += 1;
        }
        peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
    }
    eprintln!("chimney: {used} wall jumps, pelvis peaked at {peak:.2}");
    assert_eq!(used, 2, "two wall jumps before landing");
    assert!(peak > 4.5, "wall jumps should carry you higher than a plain jump");
}

#[test]
fn held_weapons_are_steady_and_follow_the_mouse() {
    for def in 0..WEAPONS.len() as u8 {
        let name = WEAPONS[def as usize].name;
        let mut s = sandbox(&[]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let wi = give(&mut s, def);
        let w = s.weapons[wi].clone();
        let rest: Vec<Vec2> = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].pos).collect();
        // hold still
        for _ in 0..60 {
            step(&mut s, 0, Vec2::new(0.6, 0.2));
        }
        let mut still_max: f32 = 0.0;
        let mut still_mean = 0.0;
        for _ in 0..60 {
            step(&mut s, 0, Vec2::new(0.6, 0.2));
            let (mx, mean) = weapon_speeds(&s, wi);
            still_max = still_max.max(mx);
            still_mean += mean / 60.0;
        }
        // slow circle (about 1 turn per second): how well does the weapon follow?
        let mut err_sum = 0.0;
        let mut max_v: f32 = 0.0;
        let mut downs = 0;
        let (d0, d1) = w.def().aim;
        for t in 0..120 {
            let a = t as f32 * core::f32::consts::TAU / 60.0;
            let aim = Vec2::new(dmath::cos(a), dmath::sin(a)) * 0.6;
            step(&mut s, 0, aim);
            let bone = s.world.particles[w.particle(d1) as usize].pos - s.world.particles[w.particle(d0) as usize].pos;
            if t >= 30 {
                err_sum += dmath::angle_diff(dmath::angle(bone), a).abs() / 90.0;
            }
            max_v = max_v.max(weapon_speeds(&s, wi).0);
            downs += (s.fighters[0].down > 0) as u32;
        }
        // click attacks
        let mut tip_peak: f32 = 0.0;
        for k in 0..4 {
            for i in 0..SWING_FRAMES as u32 + 6 {
                step(&mut s, if i == 0 { ATTACK } else { 0 }, Vec2::new(0.6, if k % 2 == 0 { 0.0 } else { 0.3 }));
                tip_peak = tip_peak.max(s.world.particles[w.particle(d1) as usize].vel.length());
                downs += (s.fighters[0].down > 0) as u32;
            }
        }
        let shape = if w.def().rigid { shape_error(&s, wi, &rest) } else { 0.0 };
        let health = s.fighters[0].health;
        eprintln!(
            "{name:12} still max {still_max:5.2} mean {still_mean:5.2} | circle lag {err_sum:4.2} rad, max v {max_v:5.1} | swing tip {tip_peak:5.1} m/s | shape err {shape:.3} | downs {downs} hp {health}"
        );
        assert_eq!(health, MAX_HP, "{name}: hurt yourself");
        // chains/fish swing like pendulums; rigid weapons must be dead still
        let limit = if w.def().rigid { 0.25 } else { 1.5 };
        assert!(still_mean < limit, "{name}: jitters while held still");
        assert!(max_v < 40.0, "{name}: weapon speed spiked");
        assert!(shape < 0.03, "{name}: rigid weapon bent");
        assert!(err_sum < 0.9, "{name}: doesn't follow the mouse");
        assert!(tip_peak > 8.0, "{name}: click attack too weak");
    }
}

const MAX_HP: f32 = crate::fighter::MAX_HEALTH;

#[test]
fn dropped_and_picked_weapons_behave() {
    for def in 0..WEAPONS.len() as u8 {
        let name = WEAPONS[def as usize].name;
        let mut s = sandbox(&[]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        // pick up from a little way off (about a metre): must not launch anything
        let me = s.fighters[0].pos(&s.world, PELVIS);
        s.spawn_weapon(def, Vec2::new(me.x + 1.05, 0.3));
        let wi = s.weapons.len() - 1;
        for _ in 0..40 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        step(&mut s, PICKUP, Vec2::new(0.6, 0.0));
        let near = s.weapons[wi].distance_to(&s.world, s.fighters[0].pos(&s.world, HAND_A));
        assert_eq!(s.fighters[0].weapon, Some(wi as u8), "{name}: pickup from 1 m failed (hand dist {near:.2}, weapon at {:?})", s.world.particles[s.weapons[wi].base as usize].pos);
        let mut pick_v: f32 = 0.0;
        for _ in 0..15 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
            pick_v = pick_v.max(weapon_speeds(&s, wi).0);
        }
        // drop it and let it settle
        for _ in 0..20 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        s.release(0, false);
        for _ in 0..150 {
            step(&mut s, 0, Vec2::new(-0.6, 0.0));
        }
        let (rest_max, _) = weapon_speeds(&s, wi);
        let w = &s.weapons[wi];
        let low = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].pos.y - s.world.particles[w.particle(i) as usize].radius).fold(f32::MAX, f32::min);
        eprintln!("{name:12} pickup spike {pick_v:5.2} m/s | settled speed {rest_max:5.3} | lowest point {low:+.3}");
        assert!(pick_v < 12.0, "{name}: pickup yanked the weapon");
        assert!(rest_max < 0.15, "{name}: dropped weapon keeps jittering");
        assert!(low > -0.04, "{name}: sank into the floor");
        assert_eq!(s.fighters[0].health, MAX_HP, "{name}: dropped weapon hurt its owner");
    }
}

#[test]
fn sword_fight_stays_sane() {
    let mut s = sandbox(&[]);
    // bring fighter 1 next to fighter 0
    let off = Vec2::new(-28.4, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..30 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
    }
    let h0 = s.fighters[0].pos(&s.world, HAND_A);
    let h1 = s.fighters[1].pos(&s.world, HAND_A);
    s.spawn_weapon(0, h0);
    s.spawn_weapon(1, h1);
    s.step(&[input::encode(PICKUP, Vec2::new(0.6, 0.0)), input::encode(PICKUP, Vec2::new(-0.6, 0.0))]);
    let mut max_v: f32 = 0.0;
    let mut hits = 0;
    let mut worst = (0.0f32, 0usize, 0u32);
    for t in 0..600u32 {
        let a = if t % 30 == 0 { ATTACK } else { 0 };
        let b = if t % 37 == 5 { ATTACK } else { 0 };
        let wob = dmath::sin(t as f32 * 0.07) * 0.4;
        s.step(&[input::encode(a, Vec2::new(0.6, wob)), input::encode(b, Vec2::new(-0.6, -wob))]);
        hits += s.hits.len();
        for (i, p) in s.world.particles.iter().enumerate() {
            assert!(p.pos.is_finite(), "NaN");
            if p.inv_mass > 0.0 && p.vel.length() > worst.0 {
                worst = (p.vel.length(), i, t);
            }
            if p.inv_mass > 0.0 {
                max_v = max_v.max(p.vel.length());
            }
        }
    }
    eprintln!("duel worst particle {worst:?}; fighters at {} and {}, weapons at {} and {}", s.fighters[0].base, s.fighters[1].base, s.weapons[0].base, s.weapons[1].base);
    let hp = (s.fighters[0].health, s.fighters[1].health);
    eprintln!("duel end: pelvis {:?} {:?}, weapons held {:?} {:?}", s.fighters[0].pos(&s.world, PELVIS), s.fighters[1].pos(&s.world, PELVIS), s.fighters[0].weapon, s.fighters[1].weapon);
    eprintln!("duel: max particle speed {max_v:.1}, hits {hits}, hp {hp:?}");
    assert!(max_v < 60.0);
    let _ = HEAD;
}

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

#[test]
fn stabs_impale_and_swings_fling_them_off() {
    for (def, button, gap) in [(0u8, ALT, 2.1), (2u8, ATTACK, 2.5), (2u8, ALT, 2.5)] {
        let name = WEAPONS[def as usize].name;
        let (mut s, wi) = duel_setup(def, gap);
        let aim = Vec2::new(0.6, -0.05);
        let mut impaled_at = None;
        for t in 0..40 {
            s.step(&[input::encode(if t == 0 { button } else { 0 }, aim), 0]);
            if impaled_at.is_none() && s.weapons[wi].victim == Some(1) {
                impaled_at = Some(t);
            }
        }
        let hp = s.fighters[1].health;
        let tip = s.world.particles[s.weapons[wi].particle(WEAPONS[def as usize].impale.unwrap().2) as usize].pos;
        eprintln!("{name:6} tip ends at x {:.2}, victim torso x {:.2}", tip.x, s.fighters[1].pos(&s.world, NECK).x);
        eprintln!("{name:6} stab: impaled at frame {impaled_at:?}, victim hp {hp:.0}, down {}", s.fighters[1].down);
        assert!(impaled_at.is_some(), "{name}: stab didn't impale");
        assert!(s.fighters[1].down > 0, "{name}: victim should be limp");
        // holding still keeps them on the blade
        for k in 0..60 {
            s.step(&[input::encode(0, aim), 0]);
            if std::env::var("TRACE").is_ok() && k % 5 == 0 {
                let sk = s.world.skewers[s.weapons[wi].skewer as usize];
                eprintln!("  k{k} victim {:?} severed {:b} gone {:b} sk a {} b {} u {:.2} en {}", s.weapons[wi].victim, s.fighters[1].severed, s.fighters[1].limbs_gone, sk.a - s.fighters[1].base, sk.b - s.fighters[1].base, sk.u, sk.enabled);
            }
        }
        let u = s.world.skewers[s.weapons[wi].skewer as usize].u;
        eprintln!("{name:6} after 1 s held level: still on {}, sits at u={u:.2}", s.weapons[wi].victim.is_some());
        assert_eq!(s.weapons[wi].victim, Some(1), "{name}: slid off while held level");
        // a hard slash flings them off
        let mut off_at = None;
        let mut fling: f32 = 0.0;
        for t in 0..60 {
            let a = if t == 0 { ATTACK } else { 0 };
            // sweep the aim up and over to whip the blade
            let ang = -0.05 + (t as f32 * 0.25).min(2.6);
            s.step(&[input::encode(a, Vec2::new(dmath::cos(ang), dmath::sin(ang)) * 0.6), 0]);
            if off_at.is_none() && s.weapons[wi].victim.is_none() {
                off_at = Some(t);
            }
            if off_at.is_some() {
                fling = fling.max(victim_speed(&s));
            }
        }
        eprintln!("{name:6} fling: came off at frame {off_at:?}, victim speed {fling:.1} m/s, attacker hp {}", s.fighters[0].health);
        assert!(off_at.is_some(), "{name}: couldn't fling the victim off");
        assert!(fling > 3.0, "{name}: victim should fly off");
        assert_eq!(s.fighters[0].health, MAX_HP);
    }
}

#[test]
fn sword_slash_does_not_impale() {
    let (mut s, wi) = duel_setup(0, 2.1);
    let mut hit = false;
    for t in 0..40 {
        s.step(&[input::encode(if t == 0 { ATTACK } else { 0 }, Vec2::new(0.6, 0.0)), 0]);
        hit |= s.fighters[1].health < MAX_HP;
        assert!(s.weapons[wi].victim.is_none(), "a slash shouldn't impale");
    }
    eprintln!("slash: landed a hit {hit}, victim hp {:.0}", s.fighters[1].health);
}

#[test]
fn crouch_and_slide() {
    let mut s = sandbox(&[]);
    for _ in 0..40 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let stand = s.fighters[0].pos(&s.world, HEAD).y;
    let mut downs = 0;
    for _ in 0..40 {
        step(&mut s, CROUCH, Vec2::new(0.6, 0.0));
        downs += (s.fighters[0].down > 0) as u32;
    }
    let low = s.fighters[0].pos(&s.world, HEAD).y;
    {
        let f = &s.fighters[0];
        let p = |q| f.pos(&s.world, q);
        eprintln!("crouched: pelvis {:.2} neck {:.2} head {:.2} kneeA {:.2} kneeB {:.2} tilt {:.2}", p(PELVIS).y, p(NECK).y, p(HEAD).y, p(fighter::KNEE_A).y, p(fighter::KNEE_B).y, tilt(&s));
    }
    let x0 = s.fighters[0].pos(&s.world, PELVIS).x;
    for _ in 0..60 {
        step(&mut s, CROUCH | RIGHT, Vec2::new(0.6, 0.0));
        downs += (s.fighters[0].down > 0) as u32;
    }
    let walk = (s.fighters[0].pos(&s.world, PELVIS).x - x0) / 1.0;
    // sprint, then crouch into a slide
    for _ in 0..30 {
        step(&mut s, RIGHT, Vec2::new(0.6, 0.0));
    }
    let x1 = s.fighters[0].pos(&s.world, PELVIS).x;
    let mut slid_frames = 0;
    for _ in 0..60 {
        step(&mut s, CROUCH, Vec2::new(0.6, 0.0));
        slid_frames += (s.fighters[0].slide > 0) as u32;
        downs += (s.fighters[0].down > 0) as u32;
    }
    let slide = s.fighters[0].pos(&s.world, PELVIS).x - x1;
    eprintln!("crouch: head {stand:.2} -> {low:.2}, duck-walk {walk:.2} m/s, slide {slide:.2} m over {slid_frames} frames, downs {downs}");
    assert!(stand - low > 0.7, "crouch should get right down low (head {low:.2})");
    assert!(walk > 3.0, "crouched you should still scuttle along quickly ({walk:.2})");
    assert!(slid_frames > 15 && slide > 2.0, "crouching at a sprint should slide");
    assert_eq!(downs, 0);
}

#[test]
fn stabs_reach_far() {
    for def in [0u8, 2] {
        let mut s = sandbox(&[]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let wi = give(&mut s, def);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let tip_of = |s: &GameState| s.world.particles[s.weapons[wi].particle(WEAPONS[def as usize].impale.unwrap().2) as usize];
        let x0 = tip_of(&s).pos.x;
        let (mut far, mut fast): (f32, f32) = (x0, 0.0);
        for k in 0..30 {
            step(&mut s, if k == 0 { ALT } else { 0 }, Vec2::new(0.6, 0.0));
            far = far.max(tip_of(&s).pos.x);
            fast = fast.max(tip_of(&s).vel.x);
        }
        eprintln!("{:6} stab: tip travels {:.2} m forward, peak {fast:.1} m/s, down {}", WEAPONS[def as usize].name, far - x0, s.fighters[0].down);
        assert!(far - x0 > 1.0, "stab should reach well forward");
        assert!(fast > 11.0, "stab should be hard");
        assert_eq!(s.fighters[0].down, 0, "stabbing shouldn't knock yourself over");
    }
}

#[test]
fn blades_stick_in_the_ground_and_can_be_wrenched_out() {
    for def in [0u8, 2] {
        let name = WEAPONS[def as usize].name;
        let mut s = sandbox(&[]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let wi = give(&mut s, def);
        // the long spear is aimed shallower so its tip starts off the floor
        let down_aim = if def == 2 { Vec2::new(0.35, -0.5) } else { Vec2::new(0.35, -0.6) };
        for _ in 0..30 {
            step(&mut s, 0, down_aim);
        }
        let mut stuck_at = None;
        let (mut low, mut fast) = (f32::MAX, 0.0f32);
        for k in 0..40 {
            step(&mut s, if k == 0 { ALT } else { 0 }, down_aim);
            if stuck_at.is_none() && s.weapons[wi].embedded {
                stuck_at = Some(k);
            }
            let t = s.world.particles[s.weapons[wi].particle(WEAPONS[def as usize].impale.unwrap().2) as usize].pos.y;
            low = low.min(t);
            fast = fast.max(s.weapons[wi].tip_speed);
        }
        eprintln!("{name:6} stab: lowest tip {low:.2}, fastest along blade {fast:.1}");
        let tip = s.world.particles[s.weapons[wi].particle(WEAPONS[def as usize].impale.unwrap().2) as usize].pos;
        eprintln!("{name:6} stab into floor: stuck at {stuck_at:?}, tip y {:.2}", tip.y);
        assert!(stuck_at.is_some(), "{name}: didn't stick in the floor");
        assert!(tip.y < 0.0, "{name}: tip should be buried");
        // holding still: stays stuck, nothing explodes
        let mut max_v: f32 = 0.0;
        for _ in 0..60 {
            step(&mut s, 0, down_aim);
            max_v = max_v.max(weapon_speeds(&s, wi).0);
        }
        assert!(s.weapons[wi].embedded && max_v < 0.5, "{name}: stuck weapon moved ({max_v})");
        // let go: it stays in the ground
        step(&mut s, PICKUP, down_aim);
        assert_eq!(s.fighters[0].weapon, None);
        for _ in 0..30 {
            step(&mut s, 0, down_aim);
        }
        assert!(s.weapons[wi].embedded, "{name}: should stay stuck after letting go");
        // grab it again and wrench it out by striking
        step(&mut s, PICKUP, down_aim);
        assert_eq!(s.fighters[0].weapon, Some(wi as u8), "{name}: couldn't grab the stuck weapon");
        let mut out_at = None;
        for k in 0..120 {
            let b = if k % 30 == 0 { ATTACK } else { 0 };
            step(&mut s, b, Vec2::new(0.6, 0.4));
            if out_at.is_none() && !s.weapons[wi].embedded {
                out_at = Some(k);
            }
        }
        eprintln!("{name:6} wrenched out at {out_at:?}, holder down {}", s.fighters[0].down);
        assert!(out_at.is_some(), "{name}: couldn't wrench it out");
        assert_eq!(s.fighters[0].health, MAX_HP);
    }
    // a thrown spear sticks in a wall
    let mut s = sandbox(&[(Vec2::new(6.0, 0.0), Vec2::new(7.0, 6.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 2);
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.15));
    }
    step(&mut s, THROW, Vec2::new(0.6, 0.15));
    let mut stuck = false;
    for _ in 0..60 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        stuck |= s.weapons[wi].embedded;
    }
    eprintln!("thrown spear stuck in wall: {stuck}");
    assert!(stuck, "a thrown spear should stick in a wall");
}

#[test]
fn revolver_shoots_and_runs_dry() {
    let (mut s, wi) = duel_setup(6, 6.0);
    s.fighters[1].health = 1000.0; // a target that soaks up all six
    let aim_at = |s: &GameState| {
        let f1 = &s.fighters[1];
        (f1.pos(&s.world, NECK) + f1.pos(&s.world, PELVIS)) * 0.5 - s.fighters[0].pos(&s.world, NECK)
    };
    for _ in 0..20 {
        let a = aim_at(&s).normalize() * 0.6;
        s.step(&[input::encode(0, a), 0]);
    }
    let mut shots = 0;
    let mut hits = 0;
    for k in 0..200 {
        let a = aim_at(&s).normalize() * 0.6;
        let b = if k % 20 == 0 { ATTACK } else { 0 };
        s.step(&[input::encode(b, a), 0]);
        shots += s.events.iter().filter(|e| e.kind == crate::sfx::Sfx::Shot).count();
        hits += s.events.iter().filter(|e| e.kind == crate::sfx::Sfx::BulletHit).count();
    }
    eprintln!("revolver: {shots} shots, {hits} hits, target took {:.0}, ammo left {}, own hp {}", 1000.0 - s.fighters[1].health, s.weapons[wi].ammo, s.fighters[0].health);
    assert_eq!(shots, 6, "six shots, then empty");
    assert_eq!(s.weapons[wi].ammo, 0);
    assert!(hits >= 3, "most shots at a standing target should land");
    assert_eq!(s.fighters[0].health, MAX_HP);
    assert!(s.bullets.is_empty());
}

#[test]
fn ropes_can_be_grabbed_and_swung_on() {
    let mut s = sandbox(&[]);
    let chain = build_rope(&mut s.world, Vec2::new(0.6, 6.5), 3.6);
    s.ropes.push(chain);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.6));
    }
    // jump up at the rope reaching for it with the free hand
    let mut held_at = None;
    for k in 0..60 {
        let b = if k < 10 { JUMP | GRAB } else { GRAB };
        step(&mut s, b, Vec2::new(0.5, 0.8));
        if held_at.is_none() && s.fighters[0].grabbing {
            held_at = Some(k);
        }
    }
    let hang = s.fighters[0].pos(&s.world, PELVIS);
    eprintln!("rope: grabbed at {held_at:?}, hanging with pelvis at {hang:?}");
    assert!(held_at.is_some() && s.fighters[0].grabbing, "couldn't grab the rope");
    assert!(hang.y > 1.2, "should hang off the ground");
    // pump the swing with A/D
    let mut lo: f32 = f32::MAX;
    let mut hi: f32 = f32::MIN;
    let mut tilt: f32 = 0.0;
    for k in 0..240 {
        let vx = s.fighters[0].body_vel(&s.world).x;
        let push = if vx >= 0.0 { RIGHT } else { LEFT };
        step(&mut s, GRAB | push, Vec2::new(0.5, 0.8));
        if k > 120 {
            let x = s.fighters[0].pos(&s.world, PELVIS).x;
            lo = lo.min(x);
            hi = hi.max(x);
            let f = &s.fighters[0];
            let up = f.pos(&s.world, fighter::NECK) - f.pos(&s.world, PELVIS);
            tilt = tilt.max(dmath::angle_diff(dmath::angle(up), core::f32::consts::FRAC_PI_2).abs());
        }
    }
    eprintln!("rope: swinging between x {lo:.2} and {hi:.2}, torso tilting up to {tilt:.2} rad");
    assert!(hi - lo > 2.0, "pumping should build a real swing");
    // limp: the body swings with the rope instead of holding itself upright
    assert!(tilt > 0.6, "the body stays stiffly upright on the rope ({tilt:.2} rad)");
    assert!(s.fighters[0].grabbing);
    // leap off
    step(&mut s, GRAB | JUMP, Vec2::new(0.5, 0.8));
    assert!(!s.fighters[0].grabbing, "jump should let go");
    let max_v = s.world.particles.iter().filter(|p| p.inv_mass > 0.0).map(|p| p.vel.length()).fold(0.0f32, f32::max);
    assert!(max_v < 40.0, "nothing should explode ({max_v})");
}

#[test]
fn hazards_hurt() {
    use crate::level::{HazardDef, HazardKind};
    for (kind, name) in [(HazardKind::Spikes, "spikes"), (HazardKind::Acid, "acid"), (HazardKind::Saw, "saw")] {
        let mut s = sandbox(&[]);
        let (a, size) = match kind {
            HazardKind::Saw => (Vec2::new(1.5, 1.0), Vec2::splat(0.9)),
            _ => (Vec2::new(2.0, 0.3), Vec2::new(1.0, 0.3)),
        };
        s.hazards.push(HazardDef { kind, a, b: a, size, speed: 0.0 });
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        for _ in 0..90 {
            step(&mut s, RIGHT, Vec2::new(0.6, 0.0));
        }
        let hp = s.fighters[0].health;
        eprintln!("{name}: walked in, hp {hp:.0}, severed {:#x}", s.fighters[0].severed);
        assert!(hp < MAX_HP - 10.0, "{name} should hurt");
    }
}


#[test]
fn thrown_knife_stops_a_runner_dead_and_knocks_them_back() {
    // fighter 1 runs at fighter 0, who throws a knife into them
    let (mut s, wi) = duel_setup(7, 9.0);
    assert!(s.weapons[wi].is_knives());
    let target = |s: &GameState| {
        let f1 = &s.fighters[1];
        (f1.pos(&s.world, NECK) + f1.pos(&s.world, PELVIS)) * 0.5 - s.fighters[0].pos(&s.world, NECK)
    };
    let mut run_speed: f32 = 0.0;
    let mut thrown_at = None;
    let mut knives_thrown = 0;
    let mut hit_frame = None;
    let mut min_vx: f32 = 0.0;
    let (mut turned, mut last_ang): (f32, Option<f32>) = (0.0, None);
    for k in 0..150 {
        let a = target(&s).normalize() * 0.6;
        let close = target(&s).length() < 5.0;
        let throw = close && thrown_at.is_none();
        if throw {
            thrown_at = Some(k);
        }
        let before = s.fighters[1].health;
        s.step(&[input::encode(if throw { THROW } else { 0 }, a), input::encode(LEFT, Vec2::new(-0.6, 0.0))]);
        knives_thrown += s.events.iter().filter(|e| e.kind == crate::sfx::Sfx::Throw).count();
        let vx = s.fighters[1].body_vel(&s.world).x;
        if thrown_at.is_none() {
            run_speed = run_speed.min(vx);
        }
        if hit_frame.is_none() && s.fighters[1].health < before {
            hit_frame = Some(k);
        }
        // the flying knife tumbles end over end
        if let (Some(t), None) = (thrown_at, hit_frame) {
            if let Some(kn) = s.weapons.iter().skip(wi + 1).find(|w| w.active && w.thrown > 0) {
                let (a, b) = (s.world.particles[kn.particle(0) as usize].pos, s.world.particles[kn.particle(2) as usize].pos);
                let ang = dmath::angle(b - a);
                if let Some(prev) = last_ang {
                    turned += dmath::angle_diff(ang, prev).abs();
                }
                last_ang = Some(ang);
                let _ = t;
            }
        }
        if hit_frame.is_some_and(|h| k < h + 15) {
            min_vx = min_vx.max(vx);
        }
    }
    let f1 = &s.fighters[1];
    eprintln!(
        "knife: runner at {run_speed:.1} m/s, thrown at {thrown_at:?}, hit at {hit_frame:?}, pushed back to {min_vx:.1} m/s, down {}, hp {:.0}; knives left {}",
        f1.down, f1.health, s.weapons[wi].ammo
    );
    assert!(run_speed < -5.0, "the runner should have been sprinting");
    assert!(hit_frame.is_some(), "the knife should hit");
    assert!(min_vx > 2.0, "the hit should stop the runner and throw them back");
    assert_eq!(knives_thrown, 1);
    eprintln!("knife turned {turned:.2} rad before landing");
    assert!(turned > 1.0, "a thrown knife should spin, not fly straight ({turned:.2} rad)");
    assert_eq!(s.weapons[wi].ammo, 2, "two knives left in hand");
    assert_eq!(s.fighters[0].weapon, Some(wi as u8));
}

#[test]
fn heavy_hits_ragdoll_light_hits_stagger() {
    // step in and bring the sledgehammer down: they go flying. From a spread
    // of distances, since exactly where it lands (head, or the guarding
    // forearms) swings with tiny differences
    let mut flying = 0;
    let gaps: Vec<f32> = (0..13).map(|g| 1.8 + g as f32 * 0.1).collect();
    for &gap in &gaps {
        let (mut s, _) = duel_setup(1, gap);
        let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
        let mut downed = false;
        for k in 0..45 {
            let b = if k == 0 { ATTACK | RIGHT } else if k < 10 { RIGHT } else { 0 };
            s.step(&[input::encode(b, Vec2::new(0.6, 0.1)), 0]);
            downed |= s.fighters[1].down > 0;
        }
        let moved = s.fighters[1].pos(&s.world, PELVIS).x - x0;
        eprintln!("sledgehammer from {gap:.1} m: victim knocked {moved:.2} m, ragdolled {downed}, hp {:.0}", s.fighters[1].health);
        assert!(s.fighters[1].health < MAX_HP, "the hammer should connect from {gap:.1} m");
        flying += (downed && moved > 0.8) as usize;
    }
    assert!(flying * 4 >= gaps.len() * 3, "a hammer blow should send them flying ({flying} of {})", gaps.len());
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

#[test]
fn blade_lock_duel_plays_out() {
    let (mut s, _, w1) = sword_duel(2.4);
    s.clash = Some(Clash {
        fighters: [0, 1],
        point: Vec2::new(1.0, 1.5),
        score: [0, 0],
        prompt: 1,
        stage: ClashStage::Intro,
        t: 0,
        total: 0,
        lockout: [0, 0],
        last: None,
    });
    let mut wrong_tried = false;
    let mut frames = 0;
    while s.clash.is_some() && frames < 900 {
        frames += 1;
        let c = s.clash.unwrap();
        // P0 answers each prompt 8 frames in; P1 once mashes the wrong key early
        let p0 = if c.stage == ClashStage::Prompt && c.t == 8 { CLASH_KEYS[c.prompt as usize] } else { 0 };
        let p1 = if c.stage == ClashStage::Prompt && c.t == 3 && !wrong_tried {
            wrong_tried = true;
            CLASH_KEYS[(c.prompt as usize + 2) % 4]
        } else {
            0
        };
        s.step(&[input::encode(p0, Vec2::new(0.6, 0.0)), input::encode(p1, Vec2::new(-0.6, 0.0))]);
        if p1 != 0 {
            assert!(s.clash.unwrap().lockout[1] > 0, "a wrong key should lock you out");
        }
        // locked fighters don't wander off or jump from the duel keys
        let f0 = &s.fighters[0];
        assert!(f0.grounded || f0.down > 0, "W in a duel shouldn't jump");
    }
    eprintln!("duel: over after {frames} frames, banner {:?}, loser weapon {:?} down {}", s.clash_banner, s.fighters[1].weapon, s.fighters[1].down);
    assert!(s.clash.is_none(), "the duel should finish");
    assert_eq!(s.clash_banner.map(|b| b.0), Some(0), "P0 should win");
    assert_eq!(s.fighters[1].weapon, None, "the loser is disarmed");
    assert!(s.fighters[1].down > 0, "and sent flying");
    assert!(!s.weapons[w1].embedded);
}

#[test]
fn crossing_swords_can_lock() {
    let mut locked = 0;
    let mut tries = 0;
    for gap in [2.2f32, 2.4, 2.6, 2.8] {
        for (a0, a1) in [(0.0f32, 0.0f32), (0.2, -0.2), (-0.2, 0.2), (0.35, 0.35)] {
            tries += 1;
            let (mut s, _, _) = sword_duel(gap);
            for k in 0..40 {
                let b = if k == 0 { ATTACK } else { 0 };
                s.step(&[input::encode(b, Vec2::new(0.6, a0)), input::encode(b, Vec2::new(-0.6, a1))]);
                if s.clash.is_some() {
                    locked += 1;
                    break;
                }
            }
        }
    }
    eprintln!("blade lock: {locked} of {tries} mirrored sword swings locked");
    assert!(locked >= 2, "meeting swings should sometimes lock");
}

#[test]
#[ignore]
fn screenshot_clash() {
    let (mut s, _, _) = sword_duel(2.4);
    s.clash = Some(Clash {
        fighters: [0, 1],
        point: Vec2::new(1.0, 1.5),
        score: [0, 0],
        prompt: 3,
        stage: ClashStage::Intro,
        t: 0,
        total: 0,
        lockout: [0, 0],
        last: None,
    });
    let mut p0_scored = false;
    for _ in 0..140 {
        let c = s.clash.unwrap();
        let p0 = if c.stage == ClashStage::Prompt && c.t == 6 && !p0_scored {
            p0_scored = true;
            CLASH_KEYS[c.prompt as usize]
        } else {
            0
        };
        s.step(&[input::encode(p0, Vec2::new(0.6, 0.0)), 0]);
        if p0_scored && s.clash.unwrap().stage == ClashStage::Prompt && s.clash.unwrap().t > 10 {
            break;
        }
    }
    let mut sp = crate::Spectate { game: s, follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
    let out = std::env::var("CLASH_SHOT").unwrap_or_else(|_| "clash.png".into());
    stickfell_app::capture_headless(&mut sp, 0, (1280, 720), out.as_ref()).unwrap();
}

#[test]
fn blade_lock_is_deterministic_under_rollback() {
    let (mut s, _, _) = sword_duel(2.4);
    s.clash = Some(Clash {
        fighters: [0, 1],
        point: Vec2::new(1.0, 1.5),
        score: [0, 0],
        prompt: 2,
        stage: ClashStage::Intro,
        t: 0,
        total: 0,
        lockout: [0, 0],
        last: None,
    });
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for k in 0..400u32 {
        // both sides hammer keys on a fixed pattern: right, wrong, ties
        let key = |p: u32| if (k + p * 5) % 9 == 0 { CLASH_KEYS[((k / 9 + p) % 4) as usize] } else { 0 };
        st.advance(&[input::encode(key(0), Vec2::new(0.6, 0.0)), input::encode(key(1), Vec2::new(-0.6, 0.0))]).unwrap();
    }
    eprintln!("rollback duel: banner {:?}, clash {:?}", st.game.clash_banner, st.game.clash.map(|c| c.score));
}

#[test]
#[ignore]
fn diagnose_loose_weapons() {
    for level in 0..3 {
        let mut s = GameState::new(4, level, 11 + level as u32);
        let (mut fast, mut lost, mut clash_spam, mut max_v) = (0, 0, 0, 0.0f32);
        let mut first_fast = None;
        for _ in 0..7200 {
            let inputs: Vec<u32> = (0..4).map(|p| crate::bot::bot_input(&s, p)).collect();
            let active_before: Vec<bool> = s.weapons.iter().map(|w| w.active && w.thrown == 0 && w.holder.is_none()).collect();
            s.step(&inputs);
            for (wi, w) in s.weapons.iter().enumerate() {
                if w.holder.is_some() || w.thrown > 0 || !w.active {
                    if active_before.get(wi) == Some(&true) && !w.active {
                        lost += 1;
                    }
                    continue;
                }
                let v = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].vel.length()).fold(0.0f32, f32::max);
                max_v = max_v.max(v);
                if v > 20.0 {
                    fast += 1;
                    if first_fast.is_none() {
                        first_fast = Some((s.frame, w.def().name, s.world.particles[w.base as usize].pos, v));
                    }
                }
            }
            clash_spam += s.events.iter().filter(|e| e.kind == crate::sfx::Sfx::Clash).count();
        }
        eprintln!("level {level}: loose weapons >20 m/s in {fast} weapon-frames (max {max_v:.0}), lost off-map untouched {lost}, clash sounds {clash_spam}; first fast {first_fast:?}");
    }
}

#[test]
#[ignore]
fn diagnose_stuck_jitter() {
    for level in 0..3 {
        for seed in 0..4u32 {
            let mut s = GameState::new(4, level, 100 + seed);
            let mut streak: Vec<u32> = Vec::new();
            let mut worst = (0u32, "", Vec2::ZERO);
            for _ in 0..3600 {
                let inputs: Vec<u32> = (0..4).map(|p| crate::bot::bot_input(&s, p)).collect();
                s.step(&inputs);
                streak.resize(s.weapons.len(), 0);
                for (wi, w) in s.weapons.iter().enumerate() {
                    let loose = w.active && w.holder.is_none() && w.thrown == 0 && !w.embedded;
                    let mean = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].vel.length()).sum::<f32>() / w.n as f32;
                    if loose && mean > 3.0 && w.touches_level(&s.world) {
                        streak[wi] += 1;
                        if streak[wi] > worst.0 {
                            worst = (streak[wi], w.def().name, s.world.particles[w.base as usize].pos);
                        }
                    } else {
                        streak[wi] = 0;
                    }
                }
            }
            eprintln!("level {level} seed {seed}: longest wedged-jitter streak {} frames ({} at {:?})", worst.0, worst.1, worst.2);
        }
    }
}

#[test]
fn weapons_never_spawn_inside_the_level() {
    for level in 0..3 {
        for seed in 0..6u32 {
            let s = GameState::new(4, level, 300 + seed);
            for w in &s.weapons {
                assert!(!w.touches_level(&s.world), "level {level} seed {seed}: {} spawned inside geometry", w.def().name);
            }
        }
    }
}

#[test]
fn every_finisher_finishes() {
    use crate::weapon::Finish;
    let cases: [(Option<u8>, Finish); 11] = [
        (Some(0), Finish::Behead),
        (Some(2), Finish::Javelin),
        (Some(2), Finish::Lift),
        (Some(1), Finish::Smash),
        (Some(3), Finish::Spin),
        (Some(4), Finish::Slap),
        (Some(5), Finish::Bonk),
        (Some(6), Finish::Execute),
        (Some(7), Finish::Shank),
        (None, Finish::Dismember),
        (None, Finish::Spine),
    ];
    for (def, mv) in cases {
        let mut s = sandbox(&[]);
        // victim right in front, down and badly hurt
        let off = Vec2::new(1.3 - 30.0, 0.0);
        for id in s.fighters[1].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.pos += off;
            p.prev += off;
        }
        for _ in 0..20 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let wi = def.map(|d| give(&mut s, d));
        for _ in 0..10 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        // (a long weapon in hand can nudge them while it's picked up: put
        // them back in front)
        let me = s.fighters[0].pos(&s.world, PELVIS);
        let them = Vec2::new(me.x + 1.3, s.fighters[1].pos(&s.world, PELVIS).y);
        warp(&mut s, 1, them);
        s.fighters[1].health = 30.0;
        s.fighters[1].knock_down(200);
        // bare hands and the spear pick one of two moves at random: steer the
        // dice to the one under test
        if def.is_none() || def == Some(2) {
            let mut seed = 0;
            loop {
                let mut r = stickfell_physics::dmath::Rng::new(seed);
                let want = if matches!(mv, Finish::Dismember | Finish::Javelin) { 0 } else { 1 };
                if r.next_u32() % 2 == want {
                    s.rng = stickfell_physics::dmath::Rng::new(seed);
                    break;
                }
                seed += 1;
            }
        }
        assert!(s.finishable(1), "{mv:?}: victim should be finishable");
        assert_eq!(
            s.finish_target(0),
            Some(1),
            "{mv:?}: should be in range (gap {:.2} m, attacker down {} strength {:.2})",
            (s.fighters[1].pos(&s.world, PELVIS) - s.fighters[0].pos(&s.world, PELVIS)).length(),
            s.fighters[0].down,
            s.fighters[0].strength()
        );
        step(&mut s, FINISH, Vec2::new(0.6, 0.0));
        assert_eq!(s.finishers.len(), 1, "{mv:?}: F should start a finisher");
        assert_eq!(s.finishers[0].mv, mv);
        let mut banner = None;
        let mut slowmo = false;
        for _ in 0..320 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
            if banner.is_none() {
                banner = s.finish_banner.map(|b| b.0);
            }
            slowmo |= s.slowmo > 0;
            if banner.is_some() && s.finishers.is_empty() {
                break;
            }
        }
        assert!(slowmo, "{mv:?}: the finishing blow should go to slow motion");
        let v = &s.fighters[1];
        let flew = (v.pos(&s.world, PELVIS).x - 1.3).abs();
        let extra = match mv {
            Finish::Behead | Finish::Smash => format!("head off {}", !v.has(fighter::S_HEAD)),
            Finish::Javelin => format!("spear thrown {}, victim carried {flew:.1} m", s.fighters[0].weapon.is_none()),
            Finish::Lift => format!("spear still in hand {}, victim off it {}", s.fighters[0].weapon.is_some(), s.weapons[wi.unwrap()].victim.is_none()),
            Finish::Spin => format!("launched {flew:.1} m"),
            Finish::Execute => format!("rounds left {}", s.weapons[wi.unwrap()].ammo),
            Finish::Dismember => format!("limbs left {}", [fighter::S_UPPER_A, fighter::S_UPPER_B, fighter::S_THIGH_A, fighter::S_THIGH_B].iter().filter(|&&p| v.has(p)).count()),
            Finish::Spine => format!("spine out {}, head-to-legs {:.1} m", v.spine_out, (v.pos(&s.world, fighter::HEAD) - v.pos(&s.world, PELVIS)).length()),
            _ => String::new(),
        };
        eprintln!("{mv:?}: victim alive {}, banner {banner:?}, attacker hp {:.0}, done {} {extra}", v.alive, s.fighters[0].health, s.finishers.is_empty());
        assert!(!v.alive, "{mv:?}: should kill");
        assert_eq!(banner, Some(mv));
        assert!(s.finishers.is_empty(), "{mv:?}: should end");
        assert!(s.fighters[0].alive && s.fighters[0].health == MAX_HP);
        match mv {
            Finish::Behead | Finish::Smash => assert!(!v.has(fighter::S_HEAD), "{mv:?} takes the head"),
            Finish::Javelin => assert!(s.fighters[0].weapon.is_none(), "the spear was thrown"),
            Finish::Lift => assert!(s.fighters[0].weapon.is_some() && s.weapons[wi.unwrap()].victim.is_none(), "lifted on the spear, then dropped off it"),
            Finish::Spin => assert!(flew > 3.0, "the flail should launch them"),
            Finish::Execute => assert_eq!(s.weapons[wi.unwrap()].ammo, 5),
            Finish::Dismember => assert!([fighter::S_UPPER_A, fighter::S_UPPER_B, fighter::S_THIGH_A, fighter::S_THIGH_B].iter().all(|&p| !v.has(p)), "all four limbs off"),
            Finish::Spine => {
                assert!(v.spine_out, "spine torn out");
                assert!((v.pos(&s.world, fighter::HEAD) - v.pos(&s.world, PELVIS)).length() > 1.5, "head and spine pulled clear of the hips");
            }
            _ => {}
        }
        for p in &s.world.particles {
            assert!(p.pos.is_finite() && p.vel.length() < 80.0, "{mv:?}: something exploded");
        }
    }
}

#[test]
#[ignore]
fn screenshot_finishers() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let shoot = |s: GameState, name: &str| {
        let mut sp = crate::Spectate { game: s, follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
        let out = format!("{dir}/fin_{name}.png");
        stickfell_app::capture_headless(&mut sp, 0, (1280, 720), out.as_ref()).unwrap();
    };
    let setup = |def: u8| {
        let mut s = sandbox(&[]);
        let off = Vec2::new(2.0 - 30.0, 0.0);
        for id in s.fighters[1].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.pos += off;
            p.prev += off;
        }
        for _ in 0..20 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        give(&mut s, def);
        for _ in 0..10 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        s
    };
    // dazed: standing, seeing stars, skull + prompt
    let mut s = setup(0);
    let p = s.fighters[1].pos(&s.world, PELVIS);
    s.damage(1, S_TORSO, 65.0, p, Vec2::X, 4.0, Kind::Blunt, Some(0));
    for _ in 0..40 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    shoot(s, "dazed");
    // a beheading, caught at the moment it lands
    let mut s = setup(0);
    let p = s.fighters[1].pos(&s.world, PELVIS);
    s.damage(1, S_TORSO, 65.0, p, Vec2::X, 4.0, Kind::Blunt, Some(0));
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    step(&mut s, FINISH, Vec2::new(0.6, 0.0));
    for _ in 0..300 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if s.flash == 7 {
            break;
        }
    }
    shoot(s, "impact");
}

#[test]
fn practice_dummies_are_harmless_and_the_yard_resets() {
    let mut s = GameState::new(4, crate::level::TRAINING, 1);
    assert!(s.training);
    assert_eq!(s.weapons.len(), crate::weapon::RACK.len(), "one of every weapon on the rack");
    let player_x = s.fighters[0].pos(&s.world, PELVIS).x;
    for _ in 0..900 {
        let mut inputs = vec![0u32];
        inputs.extend((1..4).map(|i| crate::bot::dummy_input(&s, i)));
        s.step(&inputs);
    }
    assert_eq!(s.fighters[0].health, MAX_HP, "dummies never hurt you");
    assert!(matches!(s.phase, Phase::Fight), "practice doesn't end by itself");
    assert!(s.fighters[1..].iter().all(|f| f.alive && f.down == 0), "dummies stand around");
    // kill every dummy: the yard resets, same level
    for i in 1..4 {
        let p = s.fighters[i].pos(&s.world, PELVIS);
        s.damage(i, S_TORSO, 999.0, p, Vec2::X, 10.0, Kind::Blunt, Some(0));
    }
    let round = s.round;
    for _ in 0..240 {
        let mut inputs = vec![0u32];
        inputs.extend((1..4).map(|i| crate::bot::dummy_input(&s, i)));
        s.step(&inputs);
    }
    eprintln!("practice: round {round} -> {}, level {}, player at {:.1} (was {player_x:.1})", s.round, s.level_index, s.fighters[0].pos(&s.world, PELVIS).x);
    assert_eq!(s.round, round + 1, "all dummies down resets the yard");
    assert_eq!(s.level_index, crate::level::TRAINING, "and stays in the yard");
    assert!(s.fighters.iter().all(|f| f.alive));
}

#[test]
fn finishers_are_deterministic_under_rollback() {
    for def in [0u8, 2, 4] {
        let mut s = sandbox(&[]);
        let off = Vec2::new(1.3 - 30.0, 0.0);
        for id in s.fighters[1].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.pos += off;
            p.prev += off;
        }
        for _ in 0..20 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        give(&mut s, def);
        s.fighters[1].health = 30.0;
        s.fighters[1].knock_down(200);
        let mut st = stickfell_net::SyncTestSession::new(s, 8);
        let mut killed = false;
        for k in 0..320u32 {
            let b = if k == 3 { FINISH } else { 0 };
            st.advance(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]).unwrap();
            killed |= !st.game.fighters[1].alive;
        }
        assert!(killed, "def {def}: the finisher should land under rollback too");
    }
}

#[test]
#[ignore]
fn count_bot_finishers() {
    let mut total = 0;
    let mut kills = 0;
    for level in 0..3 {
        let mut s = GameState::new(4, level, 50 + level as u32);
        for _ in 0..7200 {
            let inputs: Vec<u32> = (0..4).map(|p| crate::bot::bot_input(&s, p)).collect();
            let before = s.finishers.len();
            s.step(&inputs);
            if s.finishers.len() > before {
                total += 1;
            }
            kills += s.hits.iter().filter(|h| h.lethal).count();
        }
    }
    eprintln!("bots: {total} finishers started, {kills} kills in 6 minutes of 4-player matches");
}

#[test]
#[ignore]
fn screenshot_bare_hands() {
    use crate::weapon::Finish;
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    for (mv, name, stop) in [(Finish::Dismember, "dismember", 60u16), (Finish::Spine, "spine", 80)] {
        let mut s = sandbox(&[]);
        let off = Vec2::new(1.4 - 30.0, 0.0);
        for id in s.fighters[1].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.pos += off;
            p.prev += off;
        }
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        s.fighters[1].health = 30.0;
        s.fighters[1].knock_down(200);
        let mut seed = 0;
        loop {
            let mut r = stickfell_physics::dmath::Rng::new(seed);
            if (r.next_u32() % 2 == 0) == (mv == Finish::Dismember) {
                s.rng = stickfell_physics::dmath::Rng::new(seed);
                break;
            }
            seed += 1;
        }
        step(&mut s, FINISH, Vec2::new(0.6, 0.0));
        for _ in 0..400 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
            if s.finishers.first().is_some_and(|x| x.t >= stop) {
                break;
            }
        }
        let mut sp = crate::Spectate { game: s, follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
        stickfell_app::capture_headless(&mut sp, 0, (1280, 720), format!("{dir}/fin_{name}.png").as_ref()).unwrap();
    }
}

#[test]
#[ignore]
fn trace_javelin() {
    use crate::weapon::Finish;
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    for (mv, name) in [(Finish::Javelin, "javelin"), (Finish::Lift, "lift"), (Finish::Spin, "flail")] {
        let def = if mv == Finish::Spin { 3 } else { 2 };
        let mut s = sandbox(&[]);
        let off = Vec2::new(1.3 - 30.0, 0.0);
        for id in s.fighters[1].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.pos += off;
            p.prev += off;
        }
        for _ in 0..20 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let wi = give(&mut s, def);
        for _ in 0..10 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        s.fighters[1].health = 30.0;
        s.fighters[1].knock_down(200);
        if def == 2 {
            let mut seed = 0;
            loop {
                let mut r = stickfell_physics::dmath::Rng::new(seed);
                if (r.next_u32() % 2 == 0) == (mv == Finish::Javelin) {
                    s.rng = stickfell_physics::dmath::Rng::new(seed);
                    break;
                }
                seed += 1;
            }
        }
        step(&mut s, FINISH, Vec2::new(0.6, 0.0));
        let shot_at = match mv { Finish::Javelin => 148, Finish::Lift => 140, _ => 152 };
        let mut shot = None;
        for f in 0..400 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
            let Some(x) = s.finishers.first().copied() else { break };
            if name == "javelin" && (139..160).contains(&x.t) {
                let tip = s.world.particles[s.weapons[wi].particle(4) as usize];
                let chest = (s.fighters[1].pos(&s.world, NECK) + s.fighters[1].pos(&s.world, PELVIS)) * 0.5;
                eprintln!("  spear t{} tip ({:.2},{:.2}) v ({:.1},{:.1}) chest ({:.2},{:.2}) held {:?}", x.t, tip.pos.x, tip.pos.y, tip.vel.x, tip.vel.y, chest.x, chest.y, s.weapons[wi].holder);
            }
            if f % 6 == 0 || x.t == 84 {
                let (pa, pv) = (s.fighters[0].pos(&s.world, PELVIS), s.fighters[1].pos(&s.world, PELVIS));
                eprintln!("{name} t{} gap {:.2} victim up {:.2} down {} impaled {} killed {} slowmo {}", x.t, pv.x - pa.x, s.fighters[1].pos(&s.world, fighter::HEAD).y - pv.y, s.fighters[1].down, s.weapons[wi].victim == Some(1), x.killed, s.slowmo);
            }
            if shot.is_none() && x.t >= shot_at {
                shot = Some(s.clone());
            }
        }
        if let Some(g) = shot {
            let mut sp = crate::Spectate { game: g, follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
            stickfell_app::capture_headless(&mut sp, 0, (1280, 720), format!("{dir}/fin_{name}.png").as_ref()).unwrap();
        }
    }
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

#[test]
#[ignore]
fn trace_bare_hands() {
    let mut s = pair(1.0);
    grab_them(&mut s);
    eprintln!("grab slot {}", s.fighters[0].hold_slot);
    let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
    for k in 0..60 {
        step2(&mut s, GRAB | LEFT, Vec2::new(0.5, 0.0), 0);
        if k % 10 == 0 {
            eprintln!("drag k{k} victim x {:.2} me x {:.2} held {:?} str {:.2}", s.fighters[1].pos(&s.world, PELVIS).x, s.fighters[0].pos(&s.world, PELVIS).x, s.fighters[1].held_by, s.fighters[1].strength());
        }
    }
    eprintln!("dragged {:.2}", s.fighters[1].pos(&s.world, PELVIS).x - x0);
    let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
    step2(&mut s, GRAB | THROW, Vec2::new(0.6, 0.35), 0);
    for k in 0..90 {
        step2(&mut s, GRAB, Vec2::new(0.6, 0.35), 0);
        if k % 6 == 0 {
            let p = s.fighters[1].pos(&s.world, PELVIS);
            eprintln!("throw k{k} victim ({:.2},{:.2}) hp {:.0} down {} flying {} hold {:?}", p.x - x0, p.y, s.fighters[1].health, s.fighters[1].down, s.fighters[1].flying, s.fighters[0].hold);
        }
    }
    // punches
    let mut s = pair(0.9);
    for k in 0..40 {
        let b = if k == 0 || k == 9 || k == 18 { ATTACK } else { 0 };
        let neck = s.fighters[1].pos(&s.world, HEAD) - s.fighters[0].pos(&s.world, NECK);
        step2(&mut s, b, neck, 0);
        eprintln!("punch k{k} swing {} b {} hp {:.1} hits {:?} gap {:.2} handB x {:.2} handA x {:.2} their head x {:.2}", s.fighters[0].swing, s.fighters[0].punch_b, s.fighters[1].health, s.hits.iter().map(|h| h.damage).collect::<Vec<_>>(), s.fighters[1].pos(&s.world, PELVIS).x - s.fighters[0].pos(&s.world, PELVIS).x, s.fighters[0].pos(&s.world, fighter::HAND_B).x, s.fighters[0].pos(&s.world, HAND_A).x, s.fighters[1].pos(&s.world, HEAD).x);
    }
    // low kick
    let mut s = pair(1.0);
    for k in 0..40 {
        let b = CROUCH | if k == 12 { ATTACK } else { 0 };
        step2(&mut s, b, Vec2::new(0.6, -0.3), 0);
        if k > 10 && k % 3 == 0 {
            let v = &s.fighters[1];
            let near = [fighter::KNEE_A, fighter::FOOT_A, fighter::KNEE_B, fighter::FOOT_B].map(|q| v.pos(&s.world, q).x).into_iter().fold(f32::MAX, f32::min);
            let foot = s.fighters[0].pos(&s.world, fighter::FOOT_A);
            eprintln!("lowkick k{k} kick {} victim down {} hp {:.1} foot ({:.2},{:.2}) their nearest leg x {near:.2} pelvis y {:.2}", s.fighters[0].kick, s.fighters[1].down, s.fighters[1].health, foot.x, foot.y, s.fighters[0].pos(&s.world, PELVIS).y);
            let me = &s.fighters[0];
            let th = s.world.aims[(me.aim_base + fighter::S_THIGH_A) as usize];
            let ang = dmath::angle(me.pos(&s.world, fighter::KNEE_A) - me.pos(&s.world, PELVIS));
            eprintln!("   thigh target {:.2} actual {ang:.2} enabled {} comp {:.1e} grounded {} str {:.2}", th.target, th.enabled, th.compliance, me.grounded, me.strength());
            let sh = s.world.aims[(me.aim_base + fighter::S_SHIN_A) as usize];
            let sang = dmath::angle(me.pos(&s.world, fighter::FOOT_A) - me.pos(&s.world, fighter::KNEE_A));
            eprintln!("   shin target {:.2} actual {sang:.2} en {} pelvis x {:.2} footfric {:.2}", sh.target, sh.enabled, me.pos(&s.world, PELVIS).x, s.world.particles[me.p(fighter::FOOT_A) as usize].friction);
        }
    }
    // dropkick
    let mut s = pair(3.5);
    for k in 0..70 {
        let b = match k {
            0..=6 => RIGHT,
            7..=9 => RIGHT | JUMP,
            16 => ATTACK,
            _ => 0,
        };
        let to = s.fighters[1].pos(&s.world, NECK) - s.fighters[0].pos(&s.world, PELVIS);
        step2(&mut s, b, to, 0);
        if k > 14 && k % 3 == 0 {
            eprintln!("dropkick k{k} kick {} me down {} victim down {} hp {:.1} vx {:.1} gap {:.2}", s.fighters[0].kick, s.fighters[0].down, s.fighters[1].down, s.fighters[1].health, s.fighters[0].body_vel(&s.world).x, s.fighters[1].pos(&s.world, PELVIS).x - s.fighters[0].pos(&s.world, PELVIS).x);
        }
    }
    // struggle
    let mut s = pair(1.0);
    grab_them(&mut s);
    for k in 0..120 {
        let b1 = if k % 4 == 0 { LEFT } else if k % 4 == 2 { RIGHT } else { 0 };
        step2(&mut s, GRAB, Vec2::new(0.5, 0.0), b1);
        if s.fighters[0].hold.is_none() {
            eprintln!("broke free at k{k}");
            break;
        }
    }
}

#[test]
fn grabs_drag_and_throw_people() {
    let mut s = pair(0.9);
    grab_them(&mut s);
    let slot = s.fighters[0].hold_slot as u32;
    assert!(slot != fighter::FOOT_A && slot != fighter::FOOT_B && slot != fighter::KNEE_A && slot != fighter::KNEE_B, "grabbed a leg ({slot}) with the body in reach");
    let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
    for _ in 0..60 {
        step2(&mut s, GRAB | LEFT, Vec2::new(0.5, 0.0), 0);
    }
    let dragged = x0 - s.fighters[1].pos(&s.world, PELVIS).x;
    assert!(dragged > 2.0, "should drag them along: {dragged:.2}");
    assert_eq!(s.fighters[0].hold, Some(1), "lost the grip while dragging");
    let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
    step2(&mut s, GRAB | THROW, Vec2::new(0.6, 0.35), 0);
    let mut was_down = false;
    for _ in 0..90 {
        step2(&mut s, GRAB, Vec2::new(0.6, 0.35), 0);
        was_down |= s.fighters[1].down > 0;
    }
    let flew = s.fighters[1].pos(&s.world, PELVIS).x - x0;
    eprintln!("dragged {dragged:.2} thrown {flew:.2} hp {:.0}", s.fighters[1].health);
    assert!(flew > 4.0, "a throw should send them flying: {flew:.2}");
    assert!(was_down && s.fighters[1].health < MAX_HP, "a throw should floor and hurt them");
    assert!(s.fighters[0].hold.is_none());
}

#[test]
fn held_fighters_can_break_free() {
    let mut s = pair(0.9);
    grab_them(&mut s);
    let mut broke = None;
    for k in 0..60 {
        let b1 = [LEFT, 0, RIGHT, 0][k % 4];
        step2(&mut s, GRAB, Vec2::new(0.5, 0.0), b1);
        if s.fighters[0].hold.is_none() {
            broke = Some(k);
            break;
        }
    }
    let k = broke.expect("mashing should break a hold within a second");
    assert!(k > 8, "too easy to escape ({k} frames)");
    assert!(s.fighters[0].grab_cd > 0 && s.fighters[0].stagger > 0, "breaking free should rock the grabber");
    // and a solid hit makes the grabber let go
    let mut s = pair(0.9);
    grab_them(&mut s);
    let at = s.fighters[0].pos(&s.world, NECK);
    s.damage(0, S_TORSO, 8.0, at, Vec2::NEG_X, 3.0, Kind::Blunt, Some(1));
    assert!(s.fighters[0].hold.is_none(), "getting hit should make you let go");
}

#[test]
fn grabbing_the_weapon_arm_disarms() {
    let mut s = pair(0.9);
    let hand = s.fighters[1].pos(&s.world, HAND_A);
    s.spawn_weapon(0, hand + Vec2::new(-0.1, 0.0));
    let wi = s.weapons.len() - 1;
    s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(PICKUP, Vec2::new(-0.6, 0.0))]);
    assert_eq!(s.fighters[1].weapon, Some(wi as u8));
    assert!(s.fighters[0].health == MAX_HP && s.fighters[0].down == 0, "picking the sword up with its blade through me shouldn't stab me");
    // take hold of their sword hand
    let (hb, target) = (s.fighters[0].p(HAND_B), s.fighters[1].p(HAND_A));
    let (grab, grip_b) = (s.fighters[0].grab, s.fighters[0].grip_b);
    s.world.distances[grab as usize] = Distance { a: hb, b: target, rest: 0.0, compliance: 1e-4, enabled: true };
    s.world.distances[grip_b as usize].enabled = false;
    let f = &mut s.fighters[0];
    f.grabbing = true;
    f.hold = Some(1);
    f.hold_slot = HAND_A as u8;
    step2(&mut s, GRAB, Vec2::new(0.6, 0.0), 0);
    assert!(s.fighters[1].arm_pinned(), "held by the weapon arm should pin it");
    let (g, f1) = (&s.fighters[0], &s.fighters[1]);
    let dbg = format!("grabbing {} hold {:?} slot {} strength {:.2} grab enabled {} heave {}", g.grabbing, g.hold, g.hold_slot, g.strength(), s.world.distances[g.grab as usize].enabled, g.heave);
    let _ = f1;
    step2(&mut s, GRAB | THROW, Vec2::new(0.6, 0.0), 0);
    assert_eq!(s.fighters[1].weapon, None, "should have lost the sword ({dbg})");
    assert_eq!(s.fighters[0].weapon, Some(wi as u8), "the grabber should have it now");
}

#[test]
fn fists_combo_and_kicks_land() {
    // a one-two-three: fast jabs, alternating hands
    let mut s = pair(0.9);
    let mut used_b = false;
    let mut landed = 0;
    for k in 0..40 {
        let b = if k == 0 || k == 9 || k == 18 { ATTACK } else { 0 };
        let head = s.fighters[1].pos(&s.world, HEAD) - s.fighters[0].pos(&s.world, NECK);
        step2(&mut s, b, head, 0);
        used_b |= s.fighters[0].punch_b;
        landed += s.hits.iter().filter(|h| h.victim == 1).count();
        if k == 0 {
            assert_eq!(s.fighters[0].swing_len, fighter::PUNCH_FRAMES);
        }
    }
    let dealt = MAX_HP - s.fighters[1].health;
    eprintln!("combo: {landed} landed, {dealt:.1} damage");
    assert!(used_b, "the follow-up should come from the other hand");
    assert!(landed >= 2 && dealt > 10.0, "combo too weak: {landed} hits, {dealt:.1}");
    assert_eq!(s.fighters[0].health, MAX_HP, "punching hurt the puncher");

    // crouch + click: a low sweep takes their legs out, from a fair way off
    for gap in [0.8, 1.2, 1.5] {
        let mut s = pair(gap);
        let mut swept = false;
        for k in 0..40 {
            step2(&mut s, CROUCH | if k == 20 { ATTACK } else { 0 }, Vec2::new(0.6, -0.3), 0);
            swept |= s.fighters[1].down > 0;
            if std::env::var("TRACE").is_ok() && k >= 18 {
                let (f, o) = (&s.fighters[0], &s.fighters[1]);
                let near = [fighter::KNEE_A, fighter::FOOT_A, fighter::KNEE_B, fighter::FOOT_B].map(|q| o.pos(&s.world, q).x).into_iter().fold(f32::MAX, f32::min);
                let (ft, kn) = (f.pos(&s.world, fighter::FOOT_A), f.pos(&s.world, fighter::KNEE_A));
                eprintln!("  gap {gap} k{k} kick {} pelvis ({:.2},{:.2}) knee ({:.2},{:.2}) foot ({:.2},{:.2}) their nearest leg x {near:.2} hands y {:.2} {:.2} elbows y {:.2} {:.2} vel {:.1} grounded {} crouch_ok", f.kick, f.pos(&s.world, PELVIS).x, f.pos(&s.world, PELVIS).y, kn.x, kn.y, ft.x, ft.y, f.pos(&s.world, fighter::HAND_A).y, f.pos(&s.world, fighter::HAND_B).y, f.pos(&s.world, fighter::ELBOW_A).y, f.pos(&s.world, fighter::ELBOW_B).y, f.body_vel(&s.world).x, f.grounded);
            }
        }
        eprintln!("sweep from {gap} m: {swept}");
        assert!(swept, "low sweep should take their legs out from {gap} m");
    }
    {
        // but it's no ranged attack
        let mut s = pair(2.3);
        for k in 0..40 {
            step2(&mut s, CROUCH | if k == 20 { ATTACK } else { 0 }, Vec2::new(0.6, -0.3), 0);
            assert_eq!(s.fighters[1].down, 0, "swept someone 2.3 m away");
        }
        assert_eq!(s.fighters[0].down, 0, "a sweep shouldn't floor the sweeper");
    }

    // click in the air: a dropkick floors them, and you
    let mut s = pair(3.5);
    let (mut floored, mut me_down) = (false, false);
    for k in 0..70 {
        let b = match k {
            0..=6 => RIGHT,
            7..=9 => RIGHT | JUMP,
            16 => ATTACK,
            _ => 0,
        };
        let to = s.fighters[1].pos(&s.world, NECK) - s.fighters[0].pos(&s.world, PELVIS);
        step2(&mut s, b, to, 0);
        floored |= s.fighters[1].down > 0;
        me_down |= s.fighters[0].down > 0;
    }
    eprintln!("dropkick: victim hp {:.1}", s.fighters[1].health);
    assert!(floored && s.fighters[1].health < MAX_HP - 10.0, "dropkick should flatten them");
    assert!(me_down, "a dropkick should land the kicker on their back");
    // standing kick-free: a jump click is a kick, not a punch
    assert_eq!(s.fighters[0].health, MAX_HP);
}

#[test]
fn bare_hands_are_deterministic_under_rollback() {
    let s = pair(0.9);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for k in 0..360u32 {
        let (b0, aim) = match k {
            0..=40 => (GRAB, Vec2::new(0.7, 0.2)),
            41..=80 => (GRAB | LEFT, Vec2::new(0.5, 0.0)),
            81 => (GRAB | THROW, Vec2::new(0.6, 0.4)),
            82..=120 => (GRAB, Vec2::new(0.6, 0.4)),
            121..=200 => (RIGHT | if k % 9 == 0 { ATTACK } else { 0 }, Vec2::new(0.6, 0.3)),
            201..=260 => (CROUCH | if k % 20 == 0 { ATTACK } else { 0 }, Vec2::new(0.6, -0.3)),
            261..=275 => (RIGHT | JUMP, Vec2::new(0.6, 0.0)),
            280 => (ATTACK, Vec2::new(0.6, -0.2)),
            _ => (0, Vec2::new(0.6, 0.0)),
        };
        let b1 = if (30..60).contains(&k) && k % 4 == 0 { RIGHT } else { 0 };
        st.advance(&[input::encode(b0, aim), input::encode(b1, Vec2::new(-0.6, 0.0))]).unwrap();
    }
}

#[test]
fn stab_a_wall_and_climb_it() {
    // a tall wall right in front: jump, stab it, hang, pull up, stab higher
    let mut s = sandbox(&[(Vec2::new(1.0, 0.0), Vec2::new(3.0, 9.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.1));
    }
    let tipi = WEAPONS[0].impale.unwrap().2;
    let tip_y = |s: &GameState| s.world.particles[s.weapons[wi].particle(tipi) as usize].pos.y;
    let mut stuck_heights = Vec::new();
    let mut hung = 0;
    let mut phase = 0; // 0 jump+stab, 1 hang, 2 pull up
    let mut t = 0;
    let mut peak: f32 = 0.0;
    for k in 0..400 {
        t += 1;
        let b = match phase {
            0 if t < 4 => JUMP | RIGHT,
            0 if t == 12 => ALT | RIGHT,
            0 => RIGHT,
            1 if t >= 30 => JUMP,
            _ => 0,
        };
        step(&mut s, b, Vec2::new(0.6, 0.15));
        peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
        if phase == 0 && std::env::var("TRACE").is_ok() {
            let tp = s.world.particles[s.weapons[wi].particle(tipi) as usize].pos;
            eprintln!("k{k} t{t} tip ({:.2},{:.2}) speed {:.1} swing {} pelvis ({:.2},{:.2}) grounded {}", tp.x, tp.y, s.weapons[wi].tip_speed, s.fighters[0].swing, s.fighters[0].pos(&s.world, PELVIS).x, s.fighters[0].pos(&s.world, PELVIS).y, s.fighters[0].grounded);
        }
        match phase {
            0 if s.weapons[wi].embedded => {
                stuck_heights.push(tip_y(&s));
                phase = 1;
                t = 0;
            }
            0 if t > 50 => panic!("stab #{} into the wall didn't stick (k{k}, tip speed {:.1})", stuck_heights.len() + 1, s.weapons[wi].tip_speed),
            1 if !s.weapons[wi].embedded => {
                phase = 0;
                t = 4; // already in the air: just stab again
                if stuck_heights.len() >= 3 {
                    break;
                }
            }
            1 => hung += s.hanging_from(0).is_some() as u32,
            _ => {}
        }
    }
    eprintln!("climb: stuck at {stuck_heights:?}, hung {hung} frames, pelvis peak {peak:.2}");
    assert!(stuck_heights.len() >= 3, "should get three stabs in");
    assert!(hung > 40, "should hang from the stuck blade, not tear it out");
    assert!(stuck_heights[2] > stuck_heights[0] + 1.0, "each stab should be higher up the wall");
    assert_eq!(s.fighters[0].weapon, Some(wi as u8), "kept the sword the whole way up");
}

#[test]
fn severed_limbs_can_be_picked_up_and_swung() {
    let mut s = pair(1.6);
    // fighter 1 loses an arm, then a shin
    let elbow = s.fighters[1].pos(&s.world, fighter::ELBOW_B);
    assert!(s.sever(1, fighter::S_UPPER_B));
    assert!(s.sever(1, fighter::S_SHIN_A));
    step2(&mut s, 0, Vec2::new(0.6, 0.0), 0);
    let limbs: Vec<usize> = (0..s.weapons.len()).filter(|&i| s.weapons[i].owner == 2).collect();
    assert_eq!(limbs.len(), 2, "an arm and a shin should be lying around");
    let arm = limbs.iter().copied().find(|&i| s.weapons[i].def == crate::weapon::ARM).expect("whole arm");
    assert!(limbs.iter().any(|&i| s.weapons[i].def == crate::weapon::STUMP), "shin");
    let near = (s.world.particles[s.weapons[arm].particle(1) as usize].pos - elbow).length();
    assert!(near < 0.3, "the arm weapon should be where the arm was ({near:.2})");
    let f1 = &s.fighters[1];
    assert!(f1.limbs_gone & fighter::bit(fighter::S_UPPER_B) != 0 && !s.world.segments[(f1.seg_base + fighter::S_UPPER_B) as usize].enabled);
    // let it settle, walk over, pick it up, and smack them with it
    for _ in 0..40 {
        step2(&mut s, 0, Vec2::new(0.6, 0.0), 0);
    }
    let at = s.world.particles[s.weapons[arm].base as usize].pos;
    let hand = s.fighters[0].pos(&s.world, HAND_A);
    let off = hand - at + Vec2::new(0.05, 0.0);
    let w = s.weapons[arm].clone();
    w.shift(&mut s.world, off);
    step2(&mut s, PICKUP, Vec2::new(0.6, 0.0), 0);
    assert_eq!(s.fighters[0].weapon, Some(arm as u8), "couldn't pick up the arm");
    let hp = s.fighters[1].health;
    for k in 0..60 {
        let head = s.fighters[1].pos(&s.world, HEAD) - s.fighters[0].pos(&s.world, NECK);
        step2(&mut s, if k % 30 == 0 { ATTACK } else { 0 }, head, 0);
    }
    eprintln!("arm smack: {:.1} damage", hp - s.fighters[1].health);
    assert!(s.fighters[1].health < hp, "should be able to hit them with their own arm");
    assert_eq!(s.fighters[0].health, MAX_HP);
}

#[test]
fn bots_brawl_bare_handed() {
    // no weapons around: bots have to use fists, feet and grabs
    let (mut grabs, mut throws, mut kicks, mut punches, mut escapes) = (0, 0, 0, 0, 0);
    for level in 0..3 {
        let mut s = GameState::new(4, level, 70 + level as u32);
        s.next_drop = u32::MAX;
        for w in 0..s.weapons.len() {
            let mut wp = s.weapons[w].clone();
            wp.park(&mut s.world);
            s.weapons[w] = wp;
        }
        for _ in 0..3600 {
            let inputs: Vec<u32> = (0..4).map(|p| crate::bot::bot_input(&s, p)).collect();
            let before: Vec<(Option<u8>, u8, u8)> = s.fighters.iter().map(|f| (f.hold, f.kick, f.flying)).collect();
            s.step(&inputs);
            for (f, (hold, kick, flying)) in s.fighters.iter().zip(before) {
                grabs += (hold.is_none() && f.hold.is_some()) as u32;
                throws += (flying == 0 && f.flying >= 45) as u32;
                // (the cooldown has already ticked once by the end of the frame)
                escapes += (hold.is_some() && f.hold.is_none() && f.grab_cd == 44) as u32;
                kicks += (kick == 0 && f.kick > 0) as u32;
                punches += (f.swing > 0 && f.swing == f.swing_len) as u32;
            }
            for f in &s.fighters {
                for id in f.particle_ids() {
                    let p = s.world.particles[id as usize].pos;
                    assert!(p.is_finite(), "physics blew up");
                }
            }
        }
    }
    eprintln!("bots (3 min, bare hands): {punches} punches, {kicks} kicks, {grabs} grabs, {throws} throws, {escapes} escapes");
    // (throws are the rarest: 3 to 8 a run across seeds, so the bar is just
    // that they happen)
    assert!(punches > 25 && kicks > 5 && grabs > 5 && throws >= 2, "bots should use the whole moveset");
}

#[test]
#[ignore]
fn screenshot_bare_hand_moves() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let shot = |s: &GameState, name: &str| {
        let mut sp = crate::Spectate { game: s.clone(), follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
        stickfell_app::capture_headless(&mut sp, 0, (1280, 720), format!("{dir}/bh_{name}.png").as_ref()).unwrap();
    };
    // held, struggling
    let mut s = pair(0.9);
    grab_them(&mut s);
    for k in 0..12 {
        step2(&mut s, GRAB, Vec2::new(0.55, 0.15), if k % 4 == 0 { LEFT } else { 0 });
    }
    shot(&s, "grab");
    // mid-throw
    step2(&mut s, GRAB | THROW, Vec2::new(0.6, 0.35), 0);
    for _ in 0..22 {
        step2(&mut s, GRAB, Vec2::new(0.6, 0.35), 0);
    }
    shot(&s, "throw");
    // limbs on the floor
    let mut s = pair(1.6);
    s.sever(1, fighter::S_UPPER_B);
    s.sever(1, fighter::S_THIGH_A);
    for _ in 0..50 {
        step2(&mut s, 0, Vec2::new(0.6, 0.0), 0);
    }
    shot(&s, "limbs");
    // hanging from a sword in a wall
    let mut s = sandbox(&[(Vec2::new(1.0, 0.0), Vec2::new(3.0, 9.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.1));
    }
    let mut stuck_for = 0;
    for k in 1..120 {
        let b = match k {
            1..=3 => JUMP | RIGHT,
            12 => ALT | RIGHT,
            _ => RIGHT,
        };
        step(&mut s, b, Vec2::new(0.6, 0.15));
        stuck_for += s.weapons[wi].embedded as u32;
        if stuck_for > 25 {
            break;
        }
    }
    eprintln!("hang shot: embedded {} hanging {:?}", s.weapons[wi].embedded, s.hanging_from(0));
    shot(&s, "hang");
}

#[test]
fn finishers_get_voice_lines() {
    use crate::sfx::Sfx;
    for def in [0u8, 1, 3] {
        let mut s = sandbox(&[]);
        let off = Vec2::new(1.3 - 30.0, 0.0);
        for id in s.fighters[1].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.pos += off;
            p.prev += off;
        }
        for _ in 0..20 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        give(&mut s, def);
        s.fighters[1].health = 30.0;
        s.fighters[1].knock_down(200);
        let mv = WEAPONS[def as usize].finish;
        let mut said = Vec::new();
        for k in 0..500 {
            step(&mut s, if k == 0 { FINISH } else { 0 }, Vec2::new(0.6, 0.0));
            for e in &s.events {
                if matches!(e.kind, Sfx::Plea | Sfx::Announce | Sfx::Quip) {
                    said.push((e.kind, e.variant, e.speaker));
                }
            }
        }
        eprintln!("{}: {said:?}", WEAPONS[def as usize].name);
        assert_eq!(said, vec![(Sfx::Plea, 0, 1), (Sfx::Announce, mv as u8, 0), (Sfx::Quip, mv as u8, 0)], "{} voice lines", WEAPONS[def as usize].name);
    }
}

#[test]
#[ignore]
fn screenshot_faces() {
    use crate::sfx::Sfx;
    struct Shot {
        game: GameState,
        levels: Vec<crate::level::LevelDef>,
        talk: [Option<(Sfx, f32)>; 4],
    }
    impl stickfell_app::Game for Shot {
        fn tick(&mut self, _input: &stickfell_app::Input) {}
        fn draw(&mut self, frame: &mut stickfell_render::Frame, screen: Vec2, _alpha: f32, _dt: f32) {
            let g = &self.game;
            let mid = (g.fighters[0].pos(&g.world, HEAD) + g.fighters[1].pos(&g.world, HEAD)) * 0.5;
            let hud = crate::draw::Hud { zoom: 2.6, talk: self.talk, world_only: true, ..Default::default() };
            crate::draw::draw_game(g, &self.levels[0], frame, screen, mid, &hud);
        }
    }
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let base = pair(1.2);
    let shots: [(&str, [Option<(Sfx, f32)>; 4], u8); 4] = [
        ("smug_vs_scared", [Some((Sfx::Quip, 0.0)), Some((Sfx::Plea, 0.8)), None, None], 0),
        ("smug_talking_vs_pained", [Some((Sfx::Quip, 0.7)), None, None, None], 1),
        ("angry_vs_scared", [None, None, None, None], 2),
        ("neutral", [None, None, None, None], 3),
    ];
    for (name, talk, kind) in shots {
        let mut s = base.clone();
        match kind {
            1 => s.fighters[1].hit_flash = 6,
            2 => s.finishers.push(Finisher { attacker: 0, victim: 1, mv: crate::weapon::Finish::Behead, t: 10, weapon: None, killed: false, wall: None }),
            _ => {}
        }
        let mut shot = Shot { game: s, levels: crate::level::levels(), talk };
        stickfell_app::capture_headless(&mut shot, 0, (900, 520), format!("{dir}/face_{name}.png").as_ref()).unwrap();
    }
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

#[test]
fn dive_onto_a_head_and_flip_them() {
    let mut caught = 0;
    for gap in [2.0f32, 2.4] {
        let mut s = pair(gap);
        let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
        let (mut turned, mut spun, mut last, mut last_v) = (0.0f32, 0.0f32, None::<f32>, None::<f32>);
        let (mut flipped, mut slot, mut landed_down) = (false, None, false);
        let mut peak = 0.0f32;
        for k in 0..160 {
            let b = dive_input(&s, k);
            s.step(&[b, 0]);
            let ang = |s: &GameState, i: usize| dmath::angle(s.fighters[i].pos(&s.world, NECK) - s.fighters[i].pos(&s.world, PELVIS));
            if s.fighters[0].flip > 0 {
                flipped = true;
                slot = slot.or(Some(s.fighters[0].hold_slot as u32));
                let a = ang(&s, 0);
                if let Some(l) = last {
                    turned += dmath::angle_diff(a, l);
                }
                last = Some(a);
            } else if flipped && s.fighters[0].grounded {
                landed_down |= s.fighters[0].down > 0;
            }
            if flipped {
                let a = ang(&s, 1);
                if let Some(l) = last_v {
                    spun += dmath::angle_diff(a, l);
                    // (how far over they went: landing can roll them back)
                    peak = if spun.abs() > peak.abs() { spun } else { peak };
                }
                last_v = Some(a);
            }
        }
        let thrown = s.fighters[1].pos(&s.world, PELVIS).x - x0;
        eprintln!("dive from {gap} m: flipped {flipped} by {slot:?}, turned {turned:.2} rad, they spun {peak:.2} rad and flew {thrown:.2} m, landed down {landed_down}");
        if !flipped {
            continue;
        }
        caught += 1;
        assert!(matches!(slot, Some(fighter::HEAD | fighter::NECK)), "flip without a head grab");
        assert!(turned < -4.0, "a forward flip, most of a turn ({turned:.2} rad)");
        assert!(peak.abs() > 3.0, "they should go end over end ({peak:.2} rad)");
        assert!(thrown > 3.0, "and get flung on ({thrown:.2} m)");
        assert!(!landed_down, "the flipper should land on their feet");
    }
    assert!(caught > 0, "diving at their head should catch it");
}

#[test]
fn dive_flips_are_deterministic_under_rollback() {
    let mut st = stickfell_net::SyncTestSession::new(pair(2.2), 8);
    let mut flipped = false;
    for k in 0..200u32 {
        let b = dive_input(&st.game, k);
        st.advance(&[b, 0]).unwrap();
        flipped |= st.game.fighters[0].flip > 0;
    }
    assert!(flipped, "the flip should happen under rollback too");
}

#[test]
#[ignore]
fn screenshot_dive_flip() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = pair(2.2);
    let mut n = 0;
    for k in 0..200 {
        let b = dive_input(&s, k);
        s.step(&[b, 0]);
        if s.fighters[0].flip > 0 {
            n += 1;
            if [6, 14, 24].contains(&n) {
                let mut sp = crate::Spectate { game: s.clone(), follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
                stickfell_app::capture_headless(&mut sp, 0, (1100, 620), format!("{dir}/flip_{n}.png").as_ref()).unwrap();
            }
        }
    }
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

#[test]
fn practice_rope_course_can_be_crossed() {
    let log = cross_rope_course(|_, _| {});
    eprintln!("rope course: {log:?}");
    assert!(log.last().is_some_and(|l| l.starts_with("finish")), "never made it across: {log:?}");
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

#[test]
#[ignore]
fn screenshot_rope_course() {
    struct Shot {
        game: GameState,
        levels: Vec<crate::level::LevelDef>,
        at: Vec2,
        zoom: f32,
    }
    impl stickfell_app::Game for Shot {
        fn tick(&mut self, _input: &stickfell_app::Input) {}
        fn draw(&mut self, frame: &mut stickfell_render::Frame, screen: Vec2, _alpha: f32, _dt: f32) {
            let hud = crate::draw::Hud { zoom: self.zoom, world_only: true, ..Default::default() };
            crate::draw::draw_game(&self.game, &self.levels[crate::level::TRAINING], frame, screen, self.at, &hud);
        }
    }
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    cross_rope_course(|s, frame| {
        let (at, zoom) = match frame {
            0 => (Vec2::new(-38.0, 9.0), 20.0),
            100 | 300 | 480 => (s.fighters[0].pos(&s.world, PELVIS), 9.0),
            _ => return,
        };
        let mut shot = Shot { game: s.clone(), levels: crate::level::levels(), at, zoom };
        stickfell_app::capture_headless(&mut shot, 0, (1100, 620), format!("{dir}/course_{frame}.png").as_ref()).unwrap();
    });
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

#[test]
fn grapple_noose_hangs_and_chokes() {
    let (mut s, wi) = noose_setup();
    let mut rope_was = f32::MAX;
    let mut tied_at = None;
    let (mut caught_at, mut choking_at) = (None, None);
    let hp0 = s.fighters[1].health;
    let (mut kneel_neck, mut tossed, mut head_margin) = (Vec::new(), 0, f32::MAX);
    for k in 0..1200u32 {
        s.step(&[noose_input(k), 0]);
        let v = &s.fighters[1];
        let (head, neck, pelvis) = (v.pos(&s.world, HEAD), v.pos(&s.world, NECK), v.pos(&s.world, PELVIS));
        if (60..90).contains(&k) {
            kneel_neck.push(neck.y);
        }
        if k == 90 {
            assert_eq!(s.fighters[1].health, hp0, "the tie-up never hurts them");
        }
        if s.tossed(1) {
            tossed += 1;
            assert!(v.down > 0, "limp while the hook flies");
            assert_eq!(s.fighters[0].weapon, Some(wi as u8), "the gun stays in hand");
            let before = v.health;
            s.damage(1, S_TORSO, 30.0, pelvis, Vec2::X, 10.0, Kind::Blade, Some(0));
            assert_eq!(s.fighters[1].health, before, "nothing hurts them while the hook is in the air");
        }
        if s.fighters[1].choking {
            head_margin = head_margin.min((head.y - neck.y).min(neck.y - pelvis.y));
        }
        if tied_at.is_none() && !s.nooses.is_empty() {
            tied_at = Some(k);
            assert_eq!(s.fighters[0].weapon, Some(wi as u8), "still holding the hook after tying the knot");
        }
        if caught_at.is_none() && s.nooses.first().is_some_and(|n| n.hook == Hook::Caught) {
            caught_at = Some(k);
        }
        if let Some(n) = s.nooses.first().filter(|n| n.hook == Hook::Caught) {
            assert!(n.len <= rope_was + 1e-4, "the rope got longer after the hook caught ({rope_was:.2} -> {:.2})", n.len);
            rope_was = n.len;
        }
        if choking_at.is_none() && s.fighters[1].choking {
            choking_at = Some(k);
        }
        if !s.fighters[1].alive {
            let neck = s.fighters[1].pos(&s.world, NECK);
            let (lo, hi) = kneel_neck.iter().fold((f32::MAX, f32::MIN), |(l, h), &y| (l.min(y), h.max(y)));
            eprintln!("noose: tied {tied_at:?}, kneeling neck {lo:.2}..{hi:.2}, tossed {tossed} frames, hook caught {caught_at:?}, choking from {choking_at:?}, head above by >= {head_margin:.2}, dead at {k} hanging with neck at {neck:?}");
            assert!(lo > 0.6 && hi < 1.25, "on their knees while the rope goes on ({lo:.2}..{hi:.2})");
            assert!(tossed > 0, "the toss should be a protected ragdoll");
            assert!(head_margin > 0.0, "head always above the body while hanging ({head_margin:.2})");
            assert!(matches!(s.finish_banner, Some((Finish::Hang, 0, _))), "the hanging counts as the finisher");
            assert!(s.events.iter().any(|e| e.kind == Sfx::ChokeEnd && e.speaker == 1), "a last gasp as they go");
            // left hanging with their feet a little off the floor, rope to spare
            let feet = s.fighters[1].pos(&s.world, fighter::FOOT_A).y.min(s.fighters[1].pos(&s.world, fighter::FOOT_B).y);
            eprintln!("hung with feet {feet:.2} m up, {:.2} m of rope", s.nooses[0].len);
            assert!(feet > 0.2, "they died hanging, not on the floor ({feet:.2})");
            assert!(s.nooses[0].len > 1.5, "not reeled all the way up to the hook");
            let secs = (k - choking_at.unwrap()) as f32 / 60.0;
            assert!((5.0..14.0).contains(&secs), "a slow choke, not instant ({secs:.1} s)");
            return;
        }
    }
    panic!("never died: tied {tied_at:?}, hook caught {caught_at:?}, choking from {choking_at:?}, hp {:.1}, neck {:?}", s.fighters[1].health, s.fighters[1].pos(&s.world, NECK));
}

#[test]
fn cutting_the_noose_saves_them() {
    let (mut s, _) = noose_setup();
    let mut k = 0;
    while !s.fighters[1].choking {
        s.step(&[noose_input(k), 0]);
        k += 1;
        assert!(k < 600, "never got strung up");
    }
    for _ in 0..60 {
        s.step(&[noose_input(k), 0]);
        k += 1;
    }
    let hp = s.fighters[1].health;
    // a knife thrown through the rope
    let path = s.noose_path(&s.nooses[0]);
    let mid = (path[2] + path[6]) * 0.5;
    s.spawn_weapon(KNIFE, mid - Vec2::new(1.2, 0.0));
    let kw = s.weapons.len() - 1;
    for i in 0..s.weapons[kw].n {
        s.world.particles[s.weapons[kw].particle(i) as usize].vel = Vec2::new(20.0, 0.0);
    }
    s.weapons[kw].thrown = 50;
    let mut cut = false;
    for _ in 0..20 {
        s.step(&[noose_input(k), 0]);
        k += 1;
        cut |= s.nooses[0].cut;
    }
    assert!(cut, "the knife should cut the rope");
    // (the drop to the floor can knock a little off; after that, nothing)
    for _ in 0..60 {
        s.step(&[noose_input(k), 0]);
        k += 1;
    }
    let landed = s.fighters[1].health;
    for _ in 0..120 {
        s.step(&[noose_input(k), 0]);
        k += 1;
    }
    let v = &s.fighters[1];
    eprintln!("cut down: hp {hp:.1} -> {landed:.1} on landing -> {:.1}, pelvis {:?}, alive {}", v.health, v.pos(&s.world, PELVIS), v.alive);
    assert!(v.alive && !v.choking, "saved");
    assert!(v.pos(&s.world, PELVIS).y < 1.2, "they drop to the floor");
    assert!(landed > hp - 5.0, "the fall is just a bump");
    assert!(v.health > landed - 0.2, "and they've stopped choking");
}

#[test]
fn nooses_are_deterministic_under_rollback() {
    let (s, _) = noose_setup();
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    let mut hung = false;
    for k in 0..400u32 {
        st.advance(&[noose_input(k), 0]).unwrap();
        hung |= st.game.fighters[1].choking;
    }
    assert!(hung, "the hanging should happen under rollback too");
}

#[test]
fn shift_whips_a_dive_flip() {
    // (whip, aim during the flip): forward, forward and down through your
    // own body, and up
    let runs = [(false, Vec2::new(1.0, 0.25)), (true, Vec2::new(1.0, 0.25)), (true, Vec2::new(1.0, -0.15)), (true, Vec2::new(0.5, 0.8))];
    let mut out = Vec::new();
    for (whip, whip_aim) in runs {
        let mut s = pair(2.2);
        let (mut released, mut top_speed, mut x_at) = (None, 0.0f32, 0.0);
        let (mut me_speed, mut me_down) = (0.0f32, false);
        for k in 0..200u32 {
            let flipping = s.fighters[0].flip > 0;
            // dive with the right mouse button (bare hands); during the flip,
            // aim and press Shift (or not)
            let (bits, aim) = input::decode(dive_input(&s, k));
            let bits = if bits & GRAB != 0 { (bits & !GRAB) | ALT } else { bits };
            let b = if flipping {
                input::encode(bits | if whip { GRAB } else { 0 }, whip_aim)
            } else {
                input::encode(bits, aim)
            };
            s.step(&[b, 0]);
            if released.is_none() && flipping && s.fighters[0].flip == 0 {
                released = Some(k);
                x_at = s.fighters[1].pos(&s.world, PELVIS).x;
            }
            if let Some(r) = released {
                top_speed = top_speed.max(s.fighters[1].body_vel(&s.world).length());
                // the thrower must not get hit by their own throw
                if k > r + 1 && k < r + 40 {
                    me_speed = me_speed.max(s.fighters[0].body_vel(&s.world).length());
                    me_down |= s.fighters[0].down > 0 || s.fighters[0].hit_flash > 0;
                }
            }
        }
        assert!(released.is_some(), "no flip happened (whip {whip})");
        let flew = s.fighters[1].pos(&s.world, PELVIS).x - x_at;
        eprintln!("whip {whip} aim {whip_aim}: top speed {top_speed:.1} m/s, flew {flew:.1} m; thrower after: speed {me_speed:.1}, hit/down {me_down}");
        if whip {
            assert!(!me_down && me_speed < 4.5, "the whip hit the thrower (aim {whip_aim}: speed {me_speed:.1}, down {me_down})");
        }
        out.push((top_speed, flew));
    }
    assert!(out[1].0 > 18.0 && out[1].0 > out[0].0 * 1.6, "Shift should WHIP them ({:.1} vs {:.1} m/s)", out[1].0, out[0].0);
    assert!(out[1].1 > out[0].1 + 4.0, "and send them much further ({:.1} vs {:.1} m)", out[1].1, out[0].1);
}

#[test]
#[ignore]
fn screenshot_noose() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let (mut s, _) = noose_setup();
    for k in 0..420u32 {
        s.step(&[noose_input(k), 0]);
        if [110, 132, 134, 150, 260].contains(&k) {
            let mut sp = crate::Spectate { game: s.clone(), follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
            stickfell_app::capture_headless(&mut sp, 0, (1100, 620), format!("{dir}/noose_{k}.png").as_ref()).unwrap();
        }
    }
}

#[test]
fn a_missed_grapple_shot_stays_missed() {
    let (mut s, wi) = noose_setup();
    // tie them up (same as the hanging), then fire level along the floor:
    // nothing up there to bite
    let side = Vec2::new(1.2, 0.05);
    for k in 0..130u32 {
        s.step(&[noose_input(k), 0]);
    }
    let x0 = s.fighters[1].pos(&s.world, PELVIS).x;
    s.step(&[input::encode(ATTACK, side), 0]);
    assert_eq!(s.nooses[0].hook, Hook::Flying, "the hook should fire");
    assert_eq!(s.weapons[wi].ammo, 0, "one hook: the gun's spent once it's fired");
    let (mut furthest, mut loose_at) = (0.0f32, None);
    for k in 0..240u32 {
        s.step(&[input::encode(0, side), 0]);
        furthest = furthest.max(s.fighters[1].pos(&s.world, PELVIS).x - x0);
        assert_ne!(s.nooses[0].hook, Hook::Loaded, "a missed hook never comes back to the gun");
        if loose_at.is_none() && s.nooses[0].hook == Hook::Loose {
            loose_at = Some(k);
        }
        assert!(!s.fighters[1].choking, "no hanging from a miss");
    }
    let hook = s.world.particles[s.nooses[0].anchor as usize].pos;
    eprintln!("missed shot: dragged {furthest:.1} m, loose after {loose_at:?} frames, lying at {hook:?}");
    assert!(furthest > 2.0, "the shot should drag them along behind it");
    assert!(loose_at.is_some(), "and then the hook's loose");
    assert!(hook.y > -0.1 && hook.y < 0.5, "lying on the floor, not floating ({hook:?})");
    assert!(s.fighters[1].alive && s.fighters[0].weapon == Some(wi as u8));
    // the gun's empty now: clicking does nothing
    s.step(&[input::encode(ATTACK, Vec2::new(0.1, 1.2)), 0]);
    assert_eq!(s.nooses[0].hook, Hook::Loose, "nothing left to fire");
    // and they drag it along wherever they go
    let v = &mut s.fighters[1];
    v.down = 0;
    v.dazed = 0;
    let before = s.world.particles[s.nooses[0].anchor as usize].pos;
    for _ in 0..240 {
        s.step(&[0, input::encode(input::LEFT, Vec2::new(-0.6, 0.0))]);
    }
    let after = s.world.particles[s.nooses[0].anchor as usize].pos;
    let neck = s.fighters[1].pos(&s.world, NECK);
    eprintln!("dragged the hook from {before:?} to {after:?} (neck at {neck:?}, rope {:.1} m)", s.nooses[0].len);
    assert!(after.x < before.x - 1.0, "the hook comes along behind them");
    assert!((after - neck).length() <= s.nooses[0].len + 0.3, "on the end of its rope");
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

#[test]
fn a_sword_pins_them_to_the_wall_behind_them() {
    let (mut s, wi) = pin_setup(true, 1.5);
    s.step(&[pin_input(0), 0]);
    assert_eq!(s.finishers.first().map(|x| x.mv), Some(Finish::Pin), "a wall at their back should make it the pin");
    let (mut closest_neck, mut killed_at) = (f32::MAX, None);
    let round = s.round;
    for k in 1..280u32 {
        s.step(&[pin_input(k), 0]);
        let v = &s.fighters[1];
        if k < crate::weapon::PIN_HIT as u32 {
            closest_neck = closest_neck.min(2.6 - v.pos(&s.world, NECK).x);
        }
        if killed_at.is_none() && !v.alive {
            killed_at = Some(k);
            let w = &s.weapons[wi];
            assert!(w.embedded, "the sword should be stuck in the wall");
            assert_eq!(w.victim, Some(1), "and through them");
            assert!(w.holder.is_none() && s.fighters[0].weapon.is_none(), "the attacker lets go of it");
            assert!(!s.fighters[0].grabbing, "and of them");
        }
    }
    assert_eq!(s.round, round, "(still the same round: the body's still there)");
    let v = &s.fighters[1];
    let (chest, pelvis) = ((v.pos(&s.world, NECK) + v.pos(&s.world, PELVIS)) * 0.5, v.pos(&s.world, PELVIS));
    let w = &s.weapons[wi];
    eprintln!(
        "pin: neck came within {closest_neck:.2} m of the wall, killed at frame {killed_at:?}; ~3 s later chest {:.2} m from the wall, pelvis {:.2} m up, still on the blade: {}",
        2.6 - chest.x,
        pelvis.y,
        w.victim == Some(1)
    );
    assert!(closest_neck < 0.45, "they should be shoved back against the wall ({closest_neck:.2} m)");
    assert!(killed_at.is_some_and(|k| k >= crate::weapon::PIN_HIT as u32), "the thrust is the kill");
    assert!(w.embedded && w.victim == Some(1), "they stay pinned (the sword stays in the wall)");
    assert!((2.6 - chest.x) < 0.6, "pinned against the wall, not slid off ({:.2} m)", 2.6 - chest.x);
    assert!(pelvis.y > 0.45, "hanging on the blade, not dumped on the floor ({:.2} m)", pelvis.y);
    assert!(s.fighters[0].alive && s.fighters[0].down == 0, "the attacker walks away on their feet");
}

#[test]
fn no_wall_no_pin() {
    // open floor, or a wall too far behind them: the sword's usual beheading
    for (wall, x) in [(false, 1.5), (true, 0.6)] {
        let (mut s, _) = pin_setup(wall, x);
        s.step(&[pin_input(0), 0]);
        assert_eq!(s.finishers.first().map(|x| x.mv), Some(Finish::Behead), "wall {wall}, victim at {x}");
    }
}

#[test]
fn pins_are_deterministic_under_rollback() {
    let (s, wi) = pin_setup(true, 1.5);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for k in 0..240u32 {
        st.advance(&[pin_input(k), 0]).unwrap();
    }
    assert!(!st.game.fighters[1].alive && st.game.weapons[wi].embedded, "the pin should happen under rollback too");
}

#[test]
#[ignore]
fn screenshot_pin() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let (mut s, _) = pin_setup(true, 1.5);
    for k in 0..400u32 {
        s.step(&[pin_input(k), 0]);
        if [45, 72, 80, 90, 104, 140].contains(&k) {
            let mut sp = crate::Spectate { game: s.clone(), follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
            stickfell_app::capture_headless(&mut sp, 0, (1100, 620), format!("{dir}/pin_{k}.png").as_ref()).unwrap();
        }
    }
}

#[test]
fn the_pin_flips_the_sword_clear_of_them_before_the_thrust() {
    // until the thrust, no part of the sword crosses the victim's body
    let (mut s, wi) = pin_setup(true, 1.5);
    let mut worst: f32 = f32::MAX;
    for k in 0..200u32 {
        s.step(&[pin_input(k), 0]);
        let Some(x) = s.finishers.first() else { break };
        if x.t < 24 || x.t >= crate::weapon::PIN_HIT - 6 {
            continue;
        }
        let w = &s.weapons[wi];
        let v = &s.fighters[1];
        let (grip, tip) = (s.world.particles[w.particle(0) as usize].pos, s.world.particles[w.particle(3) as usize].pos);
        for part in [fighter::S_HEAD, fighter::S_TORSO] {
            let sg = s.world.segments[(v.seg_base + part) as usize];
            let (a, b) = (s.world.particles[sg.a as usize].pos, s.world.particles[sg.b as usize].pos);
            let (_, _, c1, c2) = stickfell_physics::collide::closest_segments(grip, tip, a, b);
            let d = (c1 - c2).length();
            worst = worst.min(d - sg.radius - 0.045);
        }
    }
    eprintln!("pin: the sword's closest pass to them before the thrust: {worst:.2} m clear");
    assert!(worst > 0.0, "the sword cut through them before the thrust ({worst:.2} m)");
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

#[test]
fn grabbing_a_pinned_sword_doesnt_fling_you() {
    let (top, s) = grab_pinned_sword(30, true);
    eprintln!("grabbing the pinned sword: top speed {top:.1} m/s, holding it {:?}", s.fighters[0].weapon);
    assert_eq!(s.fighters[0].weapon, Some(0), "you can grab its handle");
    let grip = s.world.distances[s.fighters[0].grip_a as usize];
    assert_eq!(grip.compliance, 0.0, "the grip firms up");
    assert!(top < 4.0, "grabbing it flung you at {top:.1} m/s");
}

#[test]
fn a_pinned_sword_can_be_pulled_out() {
    // grab it and pull away from the wall: it comes out (with them on it)
    let (top, s) = grab_pinned_sword(240, false);
    let w = &s.weapons[0];
    eprintln!("pulling the pinned sword: top speed {top:.1} m/s, embedded {} pinned {}", w.embedded, w.pinned);
    assert!(!w.embedded && !w.pinned, "pulling should work it free");
    assert!(top < 7.0, "pulling it out flung you at {top:.1} m/s");
}

#[test]
fn bare_fists_take_turns() {
    let mut s = sandbox(&[]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let mut hands = Vec::new();
    for _ in 0..4 {
        step(&mut s, ATTACK, Vec2::new(0.6, 0.0));
        hands.push(s.fighters[0].punch_b);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
    }
    assert_eq!(hands, vec![true, false, true, false], "each punch is thrown with the other fist");
}

#[test]
fn stab_a_wall_from_the_floor_hang_and_climb() {
    // standing at a wall: stab it high, hang off it, and climb (jump pulls you up)
    let mut s = sandbox(&[(Vec2::new(1.0, 0.0), Vec2::new(3.0, 9.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    let aim = Vec2::new(0.6, 0.55);
    let mut stuck_at = None;
    for k in 0..60u32 {
        step(&mut s, if k == 20 { ALT } else { 0 }, aim);
        if stuck_at.is_none() && s.weapons[wi].embedded {
            stuck_at = Some(k);
        }
    }
    assert!(stuck_at.is_some(), "a high stab from the floor should stick in the wall");
    let start = s.fighters[0].pos(&s.world, PELVIS).y;
    let mut peak: f32 = start;
    let mut climbs = 0;
    for k in 0..240u32 {
        let jump = k % 40 == 30;
        step(&mut s, if jump { JUMP } else { 0 }, aim);
        peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
        if jump && !s.weapons[wi].embedded {
            climbs += 1;
        }
        // stab again higher whenever the blade came out on the way up
        if !s.weapons[wi].embedded && k % 40 == 34 {
            step(&mut s, ALT, aim);
        }
    }
    eprintln!("wall climb from the floor: stuck at frame {stuck_at:?}, pelvis {start:.2} -> peak {peak:.2}, heaves {climbs}");
    assert!(peak > start + 1.2, "jumping off the stuck blade should climb ({start:.2} -> {peak:.2})");
}

#[test]
fn jump_and_grab_a_stuck_sword_to_hang_and_climb() {
    // a sword already stuck high in a wall: jump at it, grab it (E), hang, climb
    let mut s = sandbox(&[(Vec2::new(1.0, 0.0), Vec2::new(3.0, 9.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    let mut stuck = false;
    // (a steep stab from right up against a wall can glance off instead)
    for k in 0..60u32 {
        step(&mut s, if k == 20 { ALT } else { 0 }, Vec2::new(0.6, 0.5));
        stuck |= s.weapons[wi].embedded;
    }
    assert!(stuck, "set up: the sword sticks in the wall");
    // let go of it and step back from the wall
    step(&mut s, PICKUP, Vec2::new(0.6, 0.0));
    assert!(s.fighters[0].weapon.is_none(), "set up: let go");
    for _ in 0..40 {
        step(&mut s, input::LEFT, Vec2::new(-0.6, 0.0));
    }
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.3));
    }
    let grip = s.world.particles[s.weapons[wi].particle(1) as usize].pos;
    let (mut held_at, mut low_after, mut peak) = (None, f32::MAX, 0.0f32);
    for k in 0..200u32 {
        let neck = s.fighters[0].pos(&s.world, NECK);
        let aim = grip - neck;
        let b = match k {
            0..=3 => JUMP | input::RIGHT,
            _ if held_at.is_none() => if k % 2 == 0 { PICKUP | input::RIGHT } else { input::RIGHT },
            _ if k % 40 == 0 => JUMP,
            _ => 0,
        };
        step(&mut s, b, aim);
        if held_at.is_none() && s.fighters[0].weapon == Some(wi as u8) {
            held_at = Some(k);
        }
        if let Some(h) = held_at {
            if k < h + 30 {
                low_after = low_after.min(s.fighters[0].pos(&s.world, PELVIS).y);
            }
        }
        peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
    }
    eprintln!("jump-grab a stuck sword: grabbed at {held_at:?}, grip at {:.2}, pelvis sagged to {low_after:.2} after, peak {peak:.2}", grip.y);
    assert!(held_at.is_some(), "should catch the handle");
    assert!(low_after > grip.y - 1.6, "should hang off it, not slide down ({low_after:.2})");
    assert!(peak > grip.y, "should climb up past it ({peak:.2})");
}

#[test]
fn a_blade_stuck_overhead_hauls_you_up_onto_it() {
    // stab the wall above your head from the floor and hold on: the arm pulls
    // you up off the floor to hang from it (ready to climb)
    let mut s = sandbox(&[(Vec2::new(1.0, 0.0), Vec2::new(3.0, 9.0))]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    let aim = Vec2::new(0.6, 0.55);
    for k in 0..60u32 {
        step(&mut s, if k == 20 { ALT } else { 0 }, aim);
        if s.weapons[wi].embedded {
            break;
        }
    }
    assert!(s.weapons[wi].embedded, "set up: stuck above");
    let start = s.fighters[0].pos(&s.world, PELVIS).y;
    let mut peak = start;
    let mut hung = 0;
    for _ in 0..60 {
        step(&mut s, 0, aim);
        peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
        hung += s.fighters[0].hang_blade as u32;
    }
    eprintln!("overhead blade: pelvis {start:.2} -> {peak:.2}, hanging {hung} frames");
    assert!(peak > start + 0.2 || hung > 10, "holding a blade stuck overhead should haul you up onto it");
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

#[test]
fn lever_yourself_up_a_wall_on_the_stuck_blade() {
    let (gain, stuck, first) = lever_climb(std::env::var("TRACE").is_ok());
    eprintln!("lever climb: first lever lifts you {first:.2} m; rose {gain:.2} m, stabs stuck at {stuck:.2?}");
    assert!(first > 1.5, "sweeping the mouse down should heave you well up the stuck blade ({first:.2} m)");
    assert!(stuck.len() >= 3 && gain > 2.5, "levering up on the stuck blade and letting go should climb the wall ({gain:.2} m, {} stabs)", stuck.len());
}

#[test]
fn running_with_a_weapon_doesnt_drag_it_on_the_ground() {
    // run with each weapon held at a normal aim:
    // nothing scrapes the floor and nothing snags you
    let mut bad = Vec::new();
    for def in [0u8, 1, 2, 3, 4, 5, 6, 7, crate::weapon::GRAPPLE] {
        let name = WEAPONS[def as usize].name;
        let mut s = sandbox(&[]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let wi = give(&mut s, def);
        let (mut lowest, mut slowest, mut fastest) = (f32::MAX, f32::MAX, 0.0f32);
        for k in 0..150u32 {
            step(&mut s, input::RIGHT, Vec2::new(0.6, 0.0));
            let w = &s.weapons[wi];
            let low = (0..w.n).map(|i| s.world.particles[w.particle(i) as usize].pos.y).fold(f32::MAX, f32::min);
            if k > 20 {
                lowest = lowest.min(low);
                let v = s.fighters[0].body_vel(&s.world).x;
                slowest = slowest.min(v);
                fastest = fastest.max(v);
            }
        }
        eprintln!("{name:12} running: lowest point {lowest:.2} m, speed {slowest:.1}..{fastest:.1} m/s");
        if lowest <= 0.15 || slowest <= 4.5 {
            bad.push(format!("{name} (lowest {lowest:.2} m, slowest {slowest:.1} m/s)"));
        }
    }
    assert!(bad.is_empty(), "dragged or snagged while running: {bad:?}");
}

#[test]
fn double_jump_once_in_mid_air() {
    let peak_with = |presses: &[u32]| -> f32 {
        let mut s = sandbox(&[]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let start = s.fighters[0].pos(&s.world, PELVIS).y;
        let mut peak = start;
        for k in 0..90u32 {
            step(&mut s, if presses.contains(&k) { JUMP } else { 0 }, Vec2::new(0.6, 0.0));
            peak = peak.max(s.fighters[0].pos(&s.world, PELVIS).y);
        }
        peak - start
    };
    let single = peak_with(&[0]);
    let double = peak_with(&[0, 20]);
    let triple = peak_with(&[0, 20, 40]);
    eprintln!("jump {single:.2} m, double jump {double:.2} m, a third press {triple:.2} m");
    assert!(double > single + 1.0, "the second jump should go higher ({single:.2} -> {double:.2})");
    assert!((triple - double).abs() < 0.2, "only one jump in mid-air ({double:.2} vs {triple:.2})");
}

#[test]
fn weapon_art_matches_the_weapons() {
    // drawn weapons are fitted at the character's scale: the art's grip-to-tip
    // must be as long as the weapon's grip-to-aim-tip, or it's drawn stretched
    for art in crate::character_data::WEAPONS {
        let def = crate::weapon::WEAPONS.iter().find(|d| d.name == art.name).unwrap_or_else(|| panic!("art for an unknown weapon {}", art.name));
        let p = |i: u8| Vec2::new(def.points[i as usize].x, def.points[i as usize].y);
        let physics = (p(def.aim.1) - p(def.grip)).length();
        let drawn = Vec2::new(art.tip[0] - art.grip[0], art.tip[1] - art.grip[1]).length() / 420.0;
        eprintln!("{}: physics {physics:.3} m, art {drawn:.3} m", art.name);
        assert!((drawn / physics - 1.0).abs() < 0.03, "{} art and physics disagree", art.name);
    }
    // weapons drawn from pieces: grip to muzzle
    for rig in crate::character_data::WEAPON_RIGS {
        let def = crate::weapon::WEAPONS.iter().find(|d| d.name == rig.name).unwrap_or_else(|| panic!("rig for an unknown weapon {}", rig.name));
        let p = |i: u8| Vec2::new(def.points[i as usize].x, def.points[i as usize].y);
        let physics = (p(def.muzzle) - p(def.grip)).length();
        let drawn = Vec2::new(rig.muzzle[0] - rig.grip[0], rig.muzzle[1] - rig.grip[1]).length() / 420.0;
        eprintln!("{} (rig): physics {physics:.3} m, art {drawn:.3} m", rig.name);
        assert!((drawn / physics - 1.0).abs() < 0.03, "{} rig and physics disagree", rig.name);
    }
}

#[test]
fn guns_are_never_held_upside_down() {
    let def = crate::weapon::WEAPONS.iter().position(|d| d.name == "REVOLVER").unwrap() as u8;
    let mut s = sandbox(&[]);
    let wi = give(&mut s, def);
    for aim in [Vec2::new(0.6, 0.1), Vec2::new(-0.6, 0.1), Vec2::new(0.6, -0.2), Vec2::new(-0.6, -0.2), Vec2::new(0.6, 0.1)] {
        for _ in 0..50 {
            s.step(&[input::encode(0, aim), 0]);
        }
        let w = &s.weapons[wi];
        let pos = |i: u8| s.world.particles[w.particle(i) as usize].pos;
        // the grip hangs below the barrel, whichever way it points
        let axis = (pos(2) - pos(1)).normalize();
        let below = (pos(0) - pos(1)).perp_dot(axis) * axis.x.signum();
        eprintln!("aim {aim:?}: flipped {}, grip {below:.3} m below the barrel line", w.flip);
        assert!(below > 0.03, "held upside down aiming {aim:?}");
        assert_eq!(s.fighters[0].weapon, Some(wi as u8), "still in hand");
    }
}


#[test]
fn revolver_kicks_up_and_settles() {
    let def = crate::weapon::WEAPONS.iter().position(|d| d.name == "REVOLVER").unwrap() as u8;
    for aim in [Vec2::new(0.6, 0.05), Vec2::new(-0.6, 0.05)] {
        let mut s = sandbox(&[]);
        let wi = give(&mut s, def);
        for _ in 0..50 {
            s.step(&[input::encode(0, aim), 0]);
        }
        // the barrel's angle above level, either way it faces
        let ang = |s: &GameState| {
            let w = &s.weapons[wi];
            let d = s.world.particles[w.particle(2) as usize].pos - s.world.particles[w.particle(1) as usize].pos;
            dmath::angle(Vec2::new(d.x.abs(), d.y)).to_degrees()
        };
        let a0 = ang(&s);
        let (mut peak, mut after): (f32, f32) = (0.0, 0.0);
        for k in 0..30 {
            s.step(&[input::encode(if k == 0 { ATTACK } else { 0 }, aim), 0]);
            peak = peak.max(ang(&s) - a0);
            if k >= 14 {
                after = after.max((ang(&s) - a0).abs());
            }
        }
        eprintln!("revolver recoil aiming {:+.1}: kicks up {peak:.0} deg, within {after:.1} deg after 15 frames", aim.x);
        assert!((20.0..50.0).contains(&peak), "a shot should kick the gun up, not flip it over ({peak:.0} deg)");
        assert!(after < 5.0, "and the arm brings it back down");
        assert_eq!(s.fighters[0].weapon, Some(wi as u8));
    }
}

#[test]
fn execution_presses_the_muzzle_to_the_head() {
    let def = crate::weapon::WEAPONS.iter().position(|d| d.name == "REVOLVER").unwrap() as u8;
    let mut s = sandbox(&[]);
    let off = Vec2::new(2.0 - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, def);
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let p = s.fighters[1].pos(&s.world, PELVIS);
    s.damage(1, S_TORSO, 65.0, p, Vec2::X, 4.0, Kind::Blunt, Some(0));
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    step(&mut s, FINISH, Vec2::new(0.6, 0.0));
    assert!(!s.finishers.is_empty(), "the execution should start");
    let head_r = s.world.segments[(s.fighters[1].seg_base + S_HEAD) as usize].radius;
    let hit = crate::weapon::Finish::Execute.hits()[0];
    let (mut near, mut far): (f32, f32) = (f32::MAX, f32::MIN);
    while s.finishers.first().is_some_and(|x| x.t < hit) {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if s.finishers.first().is_some_and(|x| x.t >= 40) {
            let muzzle = s.world.particles[s.weapons[wi].particle(2) as usize].pos;
            let gap = (muzzle - s.fighters[1].pos(&s.world, crate::fighter::HEAD)).length() - head_r;
            near = near.min(gap);
            far = far.max(gap);
        }
    }
    eprintln!("execution: muzzle {near:.2}..{far:.2} m off the skin; victim kneeling, pelvis {:.2} m up", s.fighters[1].pos(&s.world, PELVIS).y);
    assert!(near > -0.01, "the muzzle went into their head");
    assert!(far < 0.08, "the muzzle should be against their head");
}

#[test]
fn voice_lines_are_logged_by_the_frame_they_were_said_on() {
    // an execution: the announcer and the executioner have their say, and
    // it's all in the log (part of the state, so it rolls back with it)
    let def = crate::weapon::WEAPONS.iter().position(|d| d.name == "REVOLVER").unwrap() as u8;
    let mut s = sandbox(&[]);
    let off = Vec2::new(2.0 - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    give(&mut s, def);
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let p = s.fighters[1].pos(&s.world, PELVIS);
    s.damage(1, S_TORSO, 65.0, p, Vec2::X, 4.0, Kind::Blunt, Some(0));
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let before = s.clone();
    let mut said_on = Vec::new();
    // what a player hears: after every frame, the lines from frames since the
    // last one heard, up to this one (as `Audio::speak` takes them)
    let (mut heard, mut spoken) = (s.frame, Vec::new());
    for k in 0..200 {
        step(&mut s, if k == 0 { FINISH } else { 0 }, Vec2::new(0.6, 0.0));
        let f = s.frame;
        said_on.extend(s.events.iter().filter(|e| e.kind.is_voice()).map(|e| (f, e.kind)));
        spoken.extend(s.voice_log.iter().filter(|(lf, _)| *lf > heard && *lf <= f).map(|(lf, e)| (*lf, e.kind)));
        heard = f;
    }
    assert_eq!(spoken, said_on, "every line heard, on the frame it was said");
    let logged: Vec<(u32, Sfx)> = s.voice_log.iter().map(|(f, e)| (*f, e.kind)).filter(|(f, _)| *f > before.frame).collect();
    eprintln!("voice lines: {logged:?}");
    assert!(logged.iter().any(|(_, k)| *k == Sfx::Announce), "the announcer should call it");
    assert_eq!(logged, said_on, "every line logged against the frame it was said on");
    // the same frames again from the snapshot: the same log (it's deterministic)
    let mut again = before;
    for k in 0..200 {
        step(&mut again, if k == 0 { FINISH } else { 0 }, Vec2::new(0.6, 0.0));
    }
    let replay: Vec<(u32, Sfx)> = again.voice_log.iter().map(|(f, e)| (*f, e.kind)).filter(|(f, _)| logged.first().is_some_and(|l| *f >= l.0)).collect();
    assert_eq!(replay, logged);
}

#[test]
fn hurt_mid_finisher_they_wince_and_cry_out() {
    // shanked: every stab before the last one hurts, and they show it
    let mut s = sandbox(&[]);
    let off = Vec2::new(2.0 - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    give(&mut s, crate::weapon::KNIFE);
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let p = s.fighters[1].pos(&s.world, PELVIS);
    s.damage(1, S_TORSO, 65.0, p, Vec2::X, 4.0, Kind::Blunt, Some(0));
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    step(&mut s, FINISH, Vec2::new(0.6, 0.0));
    assert!(s.finishers.first().is_some_and(|x| x.mv == crate::weapon::Finish::Shank), "set up: a shanking");
    let (mut cries, mut winced, mut died_at) = (0, 0, None);
    for k in 0..200 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        cries += s.events.iter().filter(|e| e.kind == Sfx::Pain && e.speaker == 1).count();
        if s.fighters[1].alive {
            winced += (s.fighters[1].hurt > 0) as u32;
        } else if died_at.is_none() {
            died_at = Some(k);
        }
    }
    eprintln!("shanked: {cries} cries of pain, wincing {winced} frames, died at {died_at:?}");
    let stabs = crate::weapon::Finish::Shank.hits().len() - 1;
    assert_eq!(cries, stabs, "a cry for every stab that doesn't kill");
    assert!(winced > 30, "and a pained face while it hurts");
    assert!(died_at.is_some(), "the last one kills");
}

#[test]
fn executions_leave_a_hole_through_the_head() {
    let def = crate::weapon::WEAPONS.iter().position(|d| d.name == "REVOLVER").unwrap() as u8;
    let mut s = sandbox(&[]);
    let off = Vec2::new(2.0 - 30.0, 0.0);
    for id in s.fighters[1].particle_ids() {
        let p = &mut s.world.particles[id as usize];
        p.pos += off;
        p.prev += off;
    }
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    give(&mut s, def);
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let p = s.fighters[1].pos(&s.world, PELVIS);
    s.damage(1, S_TORSO, 65.0, p, Vec2::X, 4.0, Kind::Blunt, Some(0));
    for _ in 0..10 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let decals = s.decals.len();
    step(&mut s, FINISH, Vec2::new(0.6, 0.0));
    // (a second after the shot, before the next round)
    let mut shot = None;
    for k in 0..300 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if !s.fighters[1].alive && shot.is_none() {
            shot = Some(k);
        }
        if shot.is_some_and(|t| k >= t + 60) {
            break;
        }
    }
    let v = &s.fighters[1];
    assert!(shot.is_some() && !v.alive, "executed");
    let kinds: Vec<_> = v.wounds.iter().map(|w| (w.part as u32, w.kind)).collect();
    eprintln!("execution wounds: {kinds:?}, {} new splats", s.decals.len().saturating_sub(decals));
    use crate::fighter::WoundKind;
    assert!(kinds.contains(&(S_HEAD, WoundKind::Hole)) && kinds.contains(&(S_HEAD, WoundKind::Exit)), "in one side, out the other");
    // the wounds stay on the head as the body falls about
    let head = v.pos(&s.world, HEAD);
    let r = s.world.segments[(v.seg_base + S_HEAD) as usize].radius;
    for wd in &v.wounds {
        let (at, _) = v.wound_at(&s.world, wd);
        assert!((at - head).length() <= r + 1e-3, "wound off the head");
    }
    // and in and out are on opposite sides of it
    let (a, _) = v.wound_at(&s.world, &v.wounds[0]);
    let (b, _) = v.wound_at(&s.world, &v.wounds[1]);
    assert!((a - head).dot(b - head) < 0.0);
    assert!(s.decals.len() > decals, "and they bleed onto the floor");
}

#[test]
fn wounds_lie_on_the_body_not_over_its_edge() {
    use crate::fighter::{WOUND_INSET, WoundKind};
    // every kind, aimed all over (and off) every part: each one's whole
    // drawing has to end up on the part it's in
    let mut s = sandbox(&[]);
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let mut rng = stickfell_physics::dmath::Rng::new(7);
    let mut checked = 0;
    for part in 0..10u32 {
        for kind in [WoundKind::Hole, WoundKind::Exit, WoundKind::Stab, WoundKind::Cut] {
            for _ in 0..25 {
                s.fighters[1].wounds.clear();
                let (c, _) = s.fighters[1].part_frame(&s.world, part);
                let at = c + Vec2::new(rng.range(-0.6, 0.6), rng.range(-0.6, 0.6));
                let dir = Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                let size = rng.range(0.02, 0.08);
                s.wound(1, part, at, kind, size, dir);
                let f = &s.fighters[1];
                let wd = f.wounds[0];
                let (p, run) = f.wound_at(&s.world, &wd);
                let sg = s.world.segments[(f.seg_base + part) as usize];
                let room = sg.radius * WOUND_INSET + 1e-3;
                let dist = |q: Vec2| {
                    if part == S_HEAD {
                        (q - f.pos(&s.world, HEAD)).length()
                    } else {
                        stickfell_physics::collide::point_segment(q, s.world.particles[sg.a as usize].pos, s.world.particles[sg.b as usize].pos).0
                    }
                };
                let edge = wd.size * kind.reach();
                for q in [p, p + run, p - run] {
                    assert!(dist(q) + edge <= room, "{kind:?} on part {part} hangs off it: {:.3} + {edge:.3} > {room:.3}", dist(q));
                }
                checked += 1;
            }
        }
    }
    eprintln!("{checked} wounds, all on the body");
}

#[test]
fn guns_shoot_where_youre_aiming() {
    // the shot goes through the point under the reticle, near or far, up or
    // down, either way you face
    let mut worst: f32 = 0.0;
    for name in ["REVOLVER", "PISTOL"] {
        let def = crate::weapon::WEAPONS.iter().position(|d| d.name == name).unwrap() as u8;
        for &(dx, dy) in &[(2.5f32, 0.4f32), (5.0, 1.5), (9.0, -0.6), (12.0, 3.0), (-4.0, 0.8), (-8.0, 2.5)] {
            let mut s = sandbox(&[]);
            let wi = give(&mut s, def);
            let neck = s.fighters[0].pos(&s.world, NECK);
            let target = neck + Vec2::new(dx, dy);
            for _ in 0..70 {
                let neck = s.fighters[0].pos(&s.world, NECK);
                s.step(&[input::encode(0, target - neck), 0]);
            }
            let neck = s.fighters[0].pos(&s.world, NECK);
            s.step(&[input::encode(ATTACK, target - neck), 0]);
            assert_eq!(s.weapons[wi].ammo, s.weapons[wi].def().ammo - 1, "{name}: fired");
            let mut closest = f32::MAX;
            for _ in 0..40 {
                if let Some(b) = s.bullets.first() {
                    let before = b.pos;
                    let neck = s.fighters[0].pos(&s.world, NECK);
                    s.step(&[input::encode(0, target - neck), 0]);
                    let after = s.bullets.first().map_or(before, |b| b.pos);
                    closest = closest.min(stickfell_physics::collide::point_segment(target, before, after).0);
                } else {
                    break;
                }
            }
            eprintln!("{name} at ({dx}, {dy}): passes {closest:.3} m from the aim point");
            worst = worst.max(closest);
        }
    }
    assert!(worst < 0.08, "shots should go through the reticle (worst {worst:.3} m off)");
}

#[test]
fn thrown_knives_stick_in_where_they_land() {
    // knives thrown at someone from a spread of distances stay in where they
    // hit (a spinning knife counts as point first), and go where that part goes
    let (mut stuck_n, mut throws) = (0, 0);
    for g in 0..10 {
        let gap = 2.4 + g as f32 * 0.3;
        let (mut s, _) = duel_setup(7, gap);
        let torso = |s: &GameState| (s.fighters[1].pos(&s.world, NECK) + s.fighters[1].pos(&s.world, PELVIS)) * 0.5;
        let aim = (torso(&s) - s.fighters[0].pos(&s.world, NECK)).normalize() * 0.6;
        s.step(&[input::encode(THROW, aim), 0]);
        throws += 1;
        let mut stuck = None;
        for _ in 0..60 {
            s.step(&[input::encode(0, aim), 0]);
            if let Some(wi) = s.weapons.iter().position(|w| w.stuck.is_some_and(|(f, _)| f == 1)) {
                stuck = Some(wi);
                break;
            }
        }
        let Some(wi) = stuck else { continue };
        stuck_n += 1;
        let (_, part) = s.weapons[wi].stuck.unwrap();
        // shove them about: the knife stays put in them
        let ids = s.fighters[1].particle_ids();
        s.world.add_velocity(ids, Vec2::new(4.0, 3.0));
        let offset = |s: &GameState| {
            let (c, axis) = s.fighters[1].part_frame(&s.world, part as u32);
            let d = s.world.particles[s.weapons[wi].particle(0) as usize].pos - c;
            Vec2::new(d.dot(axis), d.dot(axis.perp()))
        };
        let before = offset(&s);
        for _ in 0..40 {
            s.step(&[input::encode(0, aim), 0]);
        }
        assert!((offset(&s) - before).length() < 0.01, "a stuck knife moves with the part it's in");
        assert!(s.weapons[wi].stuck.is_some(), "and stays in");
        // the one it's in pulls it out
        s.fighters[1].down = 0;
        let hand = s.fighters[1].pos(&s.world, HAND_A);
        let knife = s.world.particles[s.weapons[wi].particle(1) as usize].pos;
        let pull = (knife - s.fighters[1].pos(&s.world, NECK)).clamp_length_max(0.6);
        let _ = hand;
        for _ in 0..30 {
            s.step(&[0, input::encode(PICKUP, pull)]);
            if s.fighters[1].weapon == Some(wi as u8) {
                break;
            }
            s.step(&[0, input::encode(0, pull)]);
        }
        if s.fighters[1].weapon == Some(wi as u8) {
            assert!(s.weapons[wi].stuck.is_none(), "picked up means pulled out");
        }
    }
    eprintln!("thrown knives: {stuck_n} of {throws} stuck in");
    assert!(stuck_n * 3 >= throws * 2, "thrown knives should stick in ({stuck_n} of {throws})");
}
