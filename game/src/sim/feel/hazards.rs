//! Spikes, saws and the practice dummies.

use super::*;

#[test]
fn hazards_hurt() {
    use crate::level::{HazardDef, HazardKind};
    // (spikes have their own: only their points really hurt, see landing_on_the_points_...)
    for (kind, name) in [(HazardKind::Acid, "acid"), (HazardKind::Saw, "saw")] {
        let mut s = sandbox(&[]);
        let (a, size) = match kind {
            HazardKind::Saw => (Vec2::new(1.5, 1.0), Vec2::splat(0.9)),
            _ => (Vec2::new(2.0, 0.3), Vec2::new(1.0, 0.3)),
        };
        s.add_hazard(HazardDef { kind, a, b: a, size, speed: 0.0 });
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
fn spikes_only_hurt_where_the_teeth_are() {
    // standing right up against the bed's edge: inside the old damage box, clear of every tooth
    let mut s = spike_yard();
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let first = s.hazards[0].teeth()[0];
    let me = s.fighters[0].pos(&s.world, PELVIS).x;
    teleport(&mut s, 0, Vec2::new(first.tip.x - first.half_width - 0.3 - me, 0.0));
    for _ in 0..90 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let hp = s.fighters[0].health;
    // how close any part came to the first tooth's shape (its nearest edge at that height)
    let gap = s.fighters[0].particle_ids().map(|id| {
        let p = s.world.particles[id as usize];
        if p.pos.y > first.tip.y {
            return (p.pos - first.tip).length() - p.radius;
        }
        let t = ((p.pos.y - first.base.y) / (first.tip.y - first.base.y)).clamp(0.0, 1.0);
        (p.pos.x - first.tip.x).abs() - first.half_width * (1.0 - t) - p.radius
    }).fold(f32::MAX, f32::min);
    eprintln!("beside the bed (closest part {gap:.2} m clear of the first tooth): hp {hp:.0}");
    assert!(gap < 0.3, "the test should have them close to the teeth ({gap:.2})");
    assert_eq!(hp, MAX_HP, "nothing touching a tooth, nothing hurt");
}

#[test]
fn mashing_works_you_off_the_spike_before_you_bleed_out() {
    let mut s = drop_flat_on_spikes(1.0);
    for _ in 0..40 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    assert!(s.fighters[0].impaled > 0);
    let mut freed = None;
    for k in 0..300 {
        // alternating keys, as fast as anyone could (a fresh press every frame)
        let b = if k % 2 == 0 { LEFT } else { RIGHT };
        step(&mut s, b, Vec2::new(0.6, 0.0));
        if s.fighters[0].impaled == 0 {
            freed = Some(k);
            break;
        }
    }
    let k = freed.expect("mashing should get them off the spike");
    eprintln!("struggled free after {k} frames, hp {:.0}", s.fighters[0].health);
    // at a human rate (8 presses a second) it takes longer, but well inside the bleed-out
    let mut h = drop_flat_on_spikes(1.0);
    for _ in 0..40 {
        step(&mut h, 0, Vec2::new(0.6, 0.0));
    }
    let human = (0..600).find(|k| {
        let b = if k % 15 == 0 { LEFT } else if k % 15 == 7 { RIGHT } else { 0 };
        step(&mut h, b, Vec2::new(0.6, 0.0));
        h.fighters[0].impaled == 0
    });
    eprintln!("at 8 presses a second: free after {human:?} frames, hp {:.0}", h.fighters[0].health);
    assert!(human.is_some_and(|k| k < 60 * 3) && h.fighters[0].alive, "a human mash gets off in time");
    assert!(s.fighters[0].alive, "and alive");
    assert!(k > 15, "not instantly ({k})");
    // and they don't land straight back on it
    for _ in 0..90 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    assert_eq!(s.fighters[0].impaled, 0, "not run straight back through");
}

#[test]
fn left_on_the_spike_you_bleed_out_and_stay_there() {
    let mut s = drop_flat_on_spikes(1.0);
    let mut died = None;
    for k in 0..60 * 20 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if !s.fighters[0].alive {
            died = Some(k);
            break;
        }
    }
    let k = died.expect("left on it, they bleed out");
    for _ in 0..60 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    eprintln!("bled out on the spike after {:.1} s", k as f32 / 60.0);
    assert!(k > 60 * 4, "it takes a while, time enough to struggle ({k})");
    assert!(s.fighters[0].impaled > 0, "the body stays on it");
}

#[test]
fn coming_down_hard_head_or_heart_first_kills_on_the_spot() {
    let mut s = drop_flat_on_spikes(4.0);
    let mut on = None;
    for k in 0..90 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if on.is_none() && s.fighters[0].impaled > 0 {
            on = Some(k);
        }
    }
    let k = on.expect("run through");
    let f = &s.fighters[0];
    eprintln!("4 m flat onto the points: run through on frame {k} by part {}, alive {}", f.impale_part, f.alive);
    assert!(!f.alive, "a hard fall onto the points through the head or heart is fatal at once");
    assert!(f.impaled > 0, "and they stay on it");
}

#[test]
fn spike_impaling_is_deterministic_under_rollback() {
    let mut st = stickfell_net::SyncTestSession::new(drop_flat_on_spikes(1.0), 8);
    for k in 0..200 {
        let b = if k > 60 { if k % 2 == 0 { LEFT } else { RIGHT } } else { 0 };
        st.advance(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]).unwrap();
    }
}

#[test]
#[ignore]
fn trace_spike_drop() {
    let mut s = spike_yard();
    for _ in 0..30 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(2.0 - me.x, 2.0));
    for k in 0..60 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        let f = &s.fighters[0];
        let low = f.particle_ids().map(|id| s.world.particles[id as usize].pos.y).fold(f32::MAX, f32::min);
        let foot = f.pos(&s.world, fighter::FOOT_A);
        eprintln!("k{k:2} lowest {low:.2} foot ({:.3},{:.2}) fall {:.2} hp {:.0} cd {}", foot.x, foot.y, s.fall[f.p(fighter::FOOT_A) as usize], f.health, s.hazard_cd[0]);
    }
}

#[test]
#[ignore]
fn screenshot_spike_landing() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    for (level, name) in [(2usize, "keep"), (crate::level::TRAINING, "yard")] {
        let mut s = GameState::new(1, level, 7);
        for _ in 0..20 {
            s.step(&[0]);
        }
        let bed = s.hazards.iter().find(|h| h.kind == crate::level::HazardKind::Spikes).copied().unwrap();
        let me = s.fighters[0].pos(&s.world, PELVIS);
        teleport(&mut s, 0, Vec2::new(bed.a.x + 0.3 - me.x, bed.a.y + 2.5 - me.y));
        for _ in 0..70 {
            s.step(&[0]);
        }
        let mut c = CloseUp { game: s.clone(), at: bed.a + Vec2::new(0.3, 0.6) };
        stickfell_app::capture_headless(&mut c, 0, (640, 480), format!("{dir}/spikes_{name}.png").as_ref()).unwrap();
        eprintln!("{name}: hp {:.0}", s.fighters[0].health);
    }
}

#[test]
fn b_brings_a_dummy_right_in_front_of_you() {
    let mut s = GameState::new(4, crate::level::TRAINING, 0x7EA1);
    for _ in 0..40 {
        let inputs: Vec<u32> = (0..4).map(|i| if i == 0 { input::encode(0, Vec2::new(0.6, 0.0)) } else { crate::bot::dummy_input(&s, i) }).collect();
        s.step(&inputs);
    }
    // one dummy dead, mangled and with a knife in it first: it's the one that comes back
    s.fighters[2].health = 0.0;
    s.fighters[2].alive = false;
    for round in 0..10 {
        s.bring_dummy(0);
        for _ in 0..60 {
            let inputs: Vec<u32> = (0..4).map(|i| if i == 0 { input::encode(0, Vec2::new(0.6, 0.0)) } else { crate::bot::dummy_input(&s, i) }).collect();
            s.step(&inputs);
        }
        let me = s.fighters[0].pos(&s.world, PELVIS);
        let (i, d) = (1..4).map(|i| (i, s.fighters[i].pos(&s.world, PELVIS) - me)).min_by(|a, b| a.1.length().total_cmp(&b.1.length())).unwrap();
        let f = &s.fighters[i];
        eprintln!("round {round}: me at ({:.2}, {:.2}), dummies at {:?}", me.x, me.y, (1..4).map(|k| s.fighters[k].pos(&s.world, PELVIS)).collect::<Vec<_>>());
        if round == 0 {
            assert_eq!(i, 2, "the dead dummy is the one brought back");
        }
        assert!(f.alive && f.health == MAX_HP && f.down == 0, "whole and standing (round {round})");
        assert!(d.x * s.fighters[0].facing > 0.6 && d.length() < 3.4, "a step in front ({:.2}, {:.2})", d.x, d.y);
        assert!(f.facing * s.fighters[0].facing < 0.0, "facing you");
        assert!(s.world.particles.iter().all(|p| p.pos.is_finite()), "nothing blew up");
    }
    assert!(s.hazards.iter().any(|h| h.kind == crate::level::HazardKind::Spikes), "spikes under the rope course");
}

#[test]
#[ignore]
fn trace_spike_impale() {
    let mut s = drop_flat_on_spikes(std::env::var("DROP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0));
    for k in 0..400 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        let f = &s.fighters[0];
        let sk = s.world.skewers[s.spike_pins[0][2] as usize];
        let p = |slot| f.pos(&s.world, slot);
        if k % 4 == 0 || (f.impaled > 0 && f.impaled < 12) {
            eprintln!("k{k:3} imp {:3} part {} | pelvis ({:.2},{:.2}) neck ({:.2},{:.2}) head y {:.2} | sk on {} u {:.2} umin {:.2} | hp {:.0} alive {} down {}",
                f.impaled, f.impale_part, p(PELVIS).x, p(PELVIS).y, p(NECK).x, p(NECK).y, p(HEAD).y, sk.enabled, sk.u, sk.u_min, f.health, f.alive, f.down);
        }
    }
}

#[test]
#[ignore]
fn screenshot_spike_impale() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    for (level, name) in [(crate::level::TRAINING, "yard"), (crate::level::GAUNTLET, "gauntlet")] {
        let mut s = GameState::new(1, level, 7);
        for _ in 0..20 {
            s.step(&[0]);
        }
        let bed = s.hazards.iter().find(|h| h.kind == crate::level::HazardKind::Spikes).copied().unwrap();
        let tips = bed.teeth()[0].tip.y;
        lay_flat(&mut s, 0, Vec2::new(bed.a.x, tips + 1.2));
        for _ in 0..50 {
            s.step(&[0]);
        }
        eprintln!("{name}: impaled {} part {} hp {:.0}", s.fighters[0].impaled, s.fighters[0].impale_part, s.fighters[0].health);
        let mut c = CloseUp { game: s.clone(), at: s.fighters[0].pos(&s.world, PELVIS) + Vec2::new(0.2, 0.4) };
        stickfell_app::capture_headless(&mut c, 0, (640, 480), format!("{dir}/impale_{name}.png").as_ref()).unwrap();
    }
    // the rope course's ropes
    let s = GameState::new(1, crate::level::TRAINING, 7);
    let mut c = CloseUp { game: s, at: Vec2::new(-34.5, 13.0) };
    stickfell_app::capture_headless(&mut c, 0, (640, 480), format!("{dir}/ropes_yard.png").as_ref()).unwrap();
}
