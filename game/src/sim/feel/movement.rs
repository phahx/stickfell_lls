//! Getting about: walking, jumping, crouching, falling, turning, and what hits do to the body.

use super::*;

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
fn falling_flat_on_the_points_runs_you_through_and_you_stay_on_them() {
    let mut s = drop_flat_on_spikes(1.0);
    let tips = s.hazards[0].teeth()[0].tip.y;
    let mut on = None;
    for k in 0..60 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if on.is_none() && s.fighters[0].impaled > 0 {
            on = Some((k, s.fighters[0].health));
        }
    }
    let (k, hp) = on.expect("a 1 m fall flat onto the points should run them through");
    let f = &s.fighters[0];
    let sk = s.world.skewers[s.spike_pins[0][2] as usize];
    let through = s.world.particles[sk.a as usize].pos.lerp(s.world.particles[sk.b as usize].pos, sk.s);
    eprintln!("run through on frame {k} by part {}: hp {hp:.0} then; it's down the spike at {:.2} (points at {tips:.2})", f.impale_part, through.y);
    assert!(f.alive, "a 1 m fall isn't instantly fatal");
    assert!(through.y < tips - 0.05, "sunk down the spike, not resting on its point ({:.2})", through.y);
    // left there, they stay on it and keep bleeding
    let before = s.fighters[0].health;
    for _ in 0..120 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let f = &s.fighters[0];
    eprintln!("two seconds later: still on it {}, hp {:.0} -> {:.0}", f.impaled > 0, before, f.health);
    assert!(f.impaled > 0, "they stay on the spike");
    assert!(f.health < before - 10.0, "bleeding on it");
}

#[test]
fn big_falls_floor_you_and_small_ones_dont() {
    for (name, h, jump, double) in [
        ("flat, jump", 0.0, true, false),
        ("flat, double jump", 0.0, true, true),
        ("walk off 1 m", 1.0, false, false),
        ("walk off 2 m", 2.0, false, false),
        ("walk off 3.5 m", 3.5, false, false),
        ("jump off 3.5 m", 3.5, true, false),
        ("double jump off 3.5 m", 3.5, true, true),
        ("walk off 5 m", 5.0, false, false),
        ("walk off 7 m", 7.0, false, false),
        ("jump off 7 m", 7.0, true, false),
        ("walk off 8 m", 8.0, false, false),
        ("walk off 9 m", 9.0, false, false),
        ("jump off 5 m", 5.0, true, false),
        ("walk off 10 m", 10.0, false, false),
        ("walk off 14 m", 14.0, false, false),
    ] {
        let (v, down) = landing(h, jump, double);
        eprintln!("{name}: landed at {v:.1} m/s{}", if down { ", knocked down" } else { "" });
        // (jumping about, and dropping a level or two, keeps you up; a
        // real height doesn't)
        if h <= 7.0 && !(jump && h >= 5.0) {
            assert!(!down, "{name}: shouldn't knock them down ({v:.1} m/s)");
        }
        if h >= 10.0 || (jump && h >= 7.0) {
            assert!(down, "{name}: should knock them down ({v:.1} m/s)");
        }
    }
}

#[test]
fn turning_round_crouched_doesnt_fling_you() {
    // squat facing right, then: turn, turn back and forth, stand up facing
    // the other way, scuttle the other way, and sweep-kick
    let plans: [(&str, fn(u32) -> (u16, f32)); 7] = [
        ("sweep, no turn", |k| (CROUCH | if k == 20 { ATTACK } else { 0 }, 0.6)),
        ("stand up, no turn", |k| (if k < 30 { CROUCH } else { 0 }, 0.6)),
        ("turn once", |_| (CROUCH, -0.6)),
        ("turn back and forth", |k| (CROUCH, if (k / 12) % 2 == 0 { -0.6 } else { 0.6 })),
        ("turn then stand", |k| (if k < 30 { CROUCH } else { 0 }, -0.6)),
        ("turn then scuttle", |k| (CROUCH | if k > 20 { LEFT } else { 0 }, -0.6)),
        ("turn then sweep", |k| (CROUCH | if k == 20 { ATTACK } else { 0 }, -0.6)),
    ];
    for (what, plan) in plans {
        let mut s = sandbox(&[]);
        for _ in 0..40 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        for _ in 0..40 {
            step(&mut s, CROUCH, Vec2::new(0.6, 0.0));
        }
        let start = s.fighters[0].pos(&s.world, PELVIS);
        let (mut fastest, mut rose, mut downs): (f32, f32, u32) = (0.0, 0.0, 0);
        for k in 0..90 {
            let (b, x) = plan(k);
            step(&mut s, b, Vec2::new(x, 0.0));
            fastest = fastest.max(fastest_part(&s));
            rose = rose.max(s.fighters[0].pos(&s.world, PELVIS).y - start.y);
            downs += (s.fighters[0].down > 0) as u32;
        }
        let f = &s.fighters[0];
        eprintln!("{what}: fastest part {fastest:.1} m/s, pelvis rose {rose:.2} m, frames down {downs}, facing {:+} legs {:+}", f.facing, f.leg_facing);
        // (a stand-up lifts the pelvis about 0.65 m; a sweep's foot whips)
        assert!(rose < 0.9, "{what}: thrown up {rose:.2} m");
        assert!(fastest < 16.0, "{what}: something flew off at {fastest:.1} m/s");
        assert_eq!(downs, 0, "{what}: knocked over");
        if what == "turn then stand" {
            assert_eq!(f.leg_facing, f.facing, "{what}: standing, the legs turn round");
        }
    }
}

#[test]
fn only_the_one_hurt_and_who_hurt_them_feel_the_shake() {
    let mut s = GameState::new(3, 0, 1);
    for _ in 0..20 {
        s.step(&[0, 0, 0]);
    }
    let p = s.fighters[1].pos(&s.world, NECK);
    s.damage(1, S_TORSO, 30.0, p, Vec2::X, 10.0, crate::weapon::Kind::Blade, Some(2));
    eprintln!("shake after fighter 2 hits fighter 1: {:?}", &s.shake[..3]);
    assert_eq!(s.shake[0], 0.0, "a bystander's camera stays still");
    assert!(s.shake[1] > 0.0 && s.shake[2] > 0.0 && s.shake[2] < s.shake[1], "the victim's shakes, the attacker's less");
}
