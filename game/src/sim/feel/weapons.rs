//! Weapons: holding, swinging, stabbing, throwing, shooting, and blade locks.

use super::*;

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
    for level in 0..crate::level::ARENAS {
        for seed in 0..6u32 {
            let s = GameState::new(if level == crate::level::GAUNTLET { 8 } else { 4 }, level, 300 + seed);
            for w in &s.weapons {
                assert!(!w.touches_level(&s.world), "level {level} seed {seed}: {} spawned inside geometry", w.def().name);
            }
        }
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

/// Online, two players: player 0 empties a revolver (a click every 45
/// frames) while player 1 watches. The game plays each new frame's sounds as
/// it arrives (`Match::take`): how many of the shots does each of them hear,
/// with the network `latency` ticks each way?
fn shots_heard_online(latency: u32) -> (u32, u32) {
    use stickfell_net::{AdvanceResult, P2PSession, SessionConfig, SimNetConfig, SimNetwork};
    let (s, _) = duel_setup(6, 3.0);
    assert_eq!(crate::weapon::WEAPONS[6].name, "REVOLVER");
    let net = SimNetwork::new(SimNetConfig { latency, jitter: 0, loss: 0.0 }, 11);
    let mut a = P2PSession::new(s.clone(), SimNetwork::transport(&net, 0), SessionConfig::new(2, 0));
    let mut b = P2PSession::new(s, SimNetwork::transport(&net, 1), SessionConfig::new(2, 1));
    // (as the game plays them: the sounds in the newest state's log not
    // heard yet)
    let (mut log_a, mut log_b) = (Vec::new(), Vec::new());
    let shots = |g: &GameState, heard: &mut Vec<(u32, crate::sfx::SfxEvent)>| crate::audio::fresh_sounds(heard, &g.sfx_log).iter().filter(|e| e.kind == crate::sfx::Sfx::Shot).count() as u32;
    let (mut heard_a, mut heard_b) = (0, 0);
    for t in 0..700u32 {
        net.borrow_mut().tick();
        a.poll();
        b.poll();
        let fire = t > 60 && t % 45 == 0;
        let ia = input::encode(if fire { ATTACK } else { 0 }, Vec2::new(0.6, 0.0));
        let ib = input::encode(0, Vec2::new(-0.6, 0.0));
        if !a.should_wait() && matches!(a.advance(ia), AdvanceResult::Advanced) {
            heard_a += shots(a.game(), &mut log_a);
        }
        if !b.should_wait() && matches!(b.advance(ib), AdvanceResult::Advanced) {
            heard_b += shots(b.game(), &mut log_b);
        }
    }
    (heard_a, heard_b)
}

#[test]
fn everyone_hears_every_shot_online_however_late_it_arrives() {
    for latency in [0u32, 1, 2, 3, 5, 8] {
        let (a, b) = shots_heard_online(latency);
        eprintln!("latency {latency} ticks each way: the shooter hears {a} shots, the other player {b}");
        assert_eq!((a, b), (6, 6), "latency {latency}: six shots, each heard once by both");
    }
}

/// Online with a laggy, jittery network, both players running about and
/// swinging: every kind of sound as heard on each side (`fresh_sounds`, as
/// the game plays them) against what really happened (each frame's sounds
/// once it's final for everyone).
#[test]
#[ignore]
fn measure_sounds_heard_online_vs_what_happened() {
    use stickfell_net::{AdvanceResult, P2PSession, SessionConfig, SimNetConfig, SimNetwork};
    use std::collections::BTreeMap;
    let (s, _) = duel_setup(0, 2.0);
    let net = SimNetwork::new(SimNetConfig { latency: 5, jitter: 3, loss: 0.02 }, 5);
    let mut peers = [P2PSession::new(s.clone(), SimNetwork::transport(&net, 0), SessionConfig::new(2, 0)), P2PSession::new(s, SimNetwork::transport(&net, 1), SessionConfig::new(2, 1))];
    let mut heard: [Vec<(u32, crate::sfx::SfxEvent)>; 2] = Default::default();
    let mut counts: [BTreeMap<String, i32>; 2] = Default::default();
    let mut truth: BTreeMap<String, i32> = BTreeMap::new();
    let mut truth_frame = 0u32;
    for t in 0..1500u32 {
        net.borrow_mut().tick();
        for (i, p) in peers.iter_mut().enumerate() {
            p.poll();
            let dir = if (t / 50 + i as u32) % 3 == 0 { LEFT } else if (t / 50) % 3 == 1 { RIGHT } else { 0 };
            let b = dir | if (t + i as u32 * 7) % 23 == 0 { ATTACK } else { 0 } | if t % 90 == 40 { JUMP } else { 0 };
            let aim = Vec2::new(if i == 0 { 0.6 } else { -0.6 }, ((t as f32) * 0.05).sin() * 0.4);
            if !p.should_wait() && matches!(p.advance(input::encode(b, aim)), AdvanceResult::Advanced) {
                for e in crate::audio::fresh_sounds(&mut heard[i], &p.game().sfx_log) {
                    *counts[i].entry(format!("{:?}", e.kind)).or_default() += 1;
                }
            }
        }
        // what really happened: frames final for both, from peer 0's log
        let fin = peers[0].final_frame().min(peers[1].final_frame());
        if fin >= 0 {
            let g = peers[0].game();
            for (f, e) in &g.sfx_log {
                if *f > truth_frame && *f as i32 <= fin + (g.frame as i32 - peers[0].frame()) {
                    *truth.entry(format!("{:?}", e.kind)).or_default() += 1;
                }
            }
            truth_frame = truth_frame.max(g.sfx_log.iter().map(|(f, _)| *f).filter(|f| *f as i32 <= fin + (g.frame as i32 - peers[0].frame())).max().unwrap_or(0));
        }
    }
    for (kind, n) in &truth {
        eprintln!("{kind:10} happened {n:4}, heard {:4} / {:4}", counts[0].get(kind).copied().unwrap_or(0), counts[1].get(kind).copied().unwrap_or(0));
    }
    for k in counts[0].keys().chain(counts[1].keys()).filter(|k| !truth.contains_key(*k)) {
        eprintln!("{k:10} happened    0, heard {:?} / {:?}", counts[0].get(k), counts[1].get(k));
    }
}
