//! Water, acid, lava, steam and floods: swimming, drowning, burning, and bodies eaten away.

use super::*;

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

#[test]
fn lever_yourself_up_a_wall_on_the_stuck_blade() {
    let (gain, stuck, first) = lever_climb(std::env::var("TRACE").is_ok());
    eprintln!("lever climb: first lever lifts you {first:.2} m; rose {gain:.2} m, stabs stuck at {stuck:.2?}");
    assert!(first > 1.5, "sweeping the mouse down should heave you well up the stuck blade ({first:.2} m)");
    assert!(stuck.len() >= 3 && gain > 2.5, "levering up on the stuck blade and letting go should climb the wall ({gain:.2} m, {} stabs)", stuck.len());
}

#[test]
fn shoulder_throw_hauls_them_over_and_slams_them_down() {
    let mut s = pair(0.9);
    let me0 = s.fighters[0].pos(&s.world, PELVIS);
    let (mut started, mut over, mut peak) = (false, false, f32::MIN);
    for k in 0..150u32 {
        let b = shoulder_throw_input(&s, k);
        s.step(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]);
        let (me, them) = (&s.fighters[0], &s.fighters[1]);
        started |= me.shoulder > 0;
        let (neck, their) = (me.pos(&s.world, NECK), them.pos(&s.world, PELVIS));
        if me.shoulder > 0 || started && !over {
            peak = peak.max(their.y - neck.y);
            over |= (their.x - neck.x).abs() < 0.3 && their.y > neck.y + 0.4;
        }
        assert!(me.down == 0, "the thrower should stay on their feet (frame {k})");
    }
    let (me, them) = (&s.fighters[0], &s.fighters[1]);
    let (my, their) = (me.pos(&s.world, PELVIS), them.pos(&s.world, PELVIS));
    let torso = them.pos(&s.world, NECK) - their;
    eprintln!(
        "shoulder throw: peak {peak:.2} m over my neck, landed {:.2} m in front, torso ({:.2},{:.2}), hp {:.0}, I moved {:.2}",
        my.x - their.x, torso.x, torso.y, them.health, (my - me0).length()
    );
    assert!(started, "Q with your back to them should start a shoulder throw");
    assert!(over && peak > 0.5, "they should go up over your head ({peak:.2} m)");
    assert!(their.x < my.x - 0.4, "they should land in front of you, on the far side");
    assert!(them.health < MAX_HP - 8.0, "the slam should hurt ({:.0})", them.health);
    assert!(me.hold.is_none(), "you let go at the end");
    assert!((my - me0).length() < 1.0, "the thrower stays planted");
}

#[test]
fn bodies_float_in_water() {
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 1.5);
    let mut lowest = f32::MAX;
    for _ in 0..240 {
        // (limp throughout, like a body)
        s.fighters[0].down = 200;
        s.step(&[0]);
        lowest = lowest.min(s.fighters[0].pos(&s.world, PELVIS).y);
    }
    let (top, bottom) = (h.a.y + h.size.y, h.a.y - h.size.y);
    let pelvis = s.fighters[0].pos(&s.world, PELVIS).y;
    eprintln!("water: pelvis went down to {lowest:.2}, settled at {pelvis:.2} (surface {top:.2}, bottom {bottom:.2})");
    assert!(lowest > bottom + 0.3, "it shouldn't reach the bottom");
    assert!((pelvis - top).abs() < 0.35, "it floats at the surface ({pelvis:.2})");
}

#[test]
fn bodies_sink_in_acid_and_lava_and_burn() {
    use crate::level::HazardKind;
    for kind in [HazardKind::Lava] {
        let (mut s, h) = dropped_in(kind, 1.0);
        let mut died = None;
        for k in 0..300 {
            s.step(&[0]);
            if !s.fighters[0].alive {
                died = Some(k);
                break;
            }
        }
        // (a moment on, before the round starts over)
        for _ in 0..40 {
            s.step(&[0]);
        }
        let bottom = h.a.y - h.size.y;
        let pelvis = s.fighters[0].pos(&s.world, PELVIS).y;
        eprintln!("{kind:?}: died after {died:?} frames, pelvis at {pelvis:.2} (bottom {bottom:.2}), scorched {:.2}", s.fighters[0].scorch);
        assert!(pelvis < bottom + 0.45, "sunk to the bottom ({pelvis:.2})");
        assert!(died.is_some_and(|k| k < 240), "it kills");
        assert!(s.fighters[0].scorch > 0.5, "and burns");
    }
    // acid: in the sandbox, a vat to sink into
    use crate::level::HazardDef;
    let mut s = sandbox(&[]);
    s.add_hazard(HazardDef { kind: HazardKind::Acid, a: Vec2::new(3.0, 1.0), b: Vec2::new(3.0, 1.0), size: Vec2::new(1.5, 1.0), speed: 0.0 });
    lay_flat(&mut s, 0, Vec2::new(3.0, 2.8));
    let hp = s.fighters[0].health;
    for _ in 0..90 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let pelvis = s.fighters[0].pos(&s.world, PELVIS).y;
    eprintln!("acid: hp {hp:.0} -> {:.0}, pelvis at {pelvis:.2}", s.fighters[0].health);
    assert!(s.fighters[0].health < hp - 20.0, "acid eats them");
    assert!(pelvis < 0.6, "and they sink in it ({pelvis:.2})");
}

#[test]
fn you_can_swim_in_water_and_drown_if_held_under() {
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 0.5);
    s.fighters[0].down = 0;
    for _ in 0..120 {
        s.step(&[0]);
    }
    let x0 = s.fighters[0].pos(&s.world, PELVIS).x;
    // paddle right, stroking up now and then
    let mut highest = f32::MIN;
    for k in 0..40 {
        let b = RIGHT | if k % 20 == 0 { JUMP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.0))]);
        highest = highest.max(s.fighters[0].pos(&s.world, NECK).y);
    }
    let x1 = s.fighters[0].pos(&s.world, PELVIS).x;
    let top = h.a.y + h.size.y;
    eprintln!("before holding them under: hp {:.0}, breath {}", s.fighters[0].health, s.fighters[0].breath);
    eprintln!("swam from x {x0:.2} to {x1:.2}; neck up to {highest:.2} (surface {top:.2})");
    assert!(x1 > x0 + 1.0, "paddling moves them along");
    assert!(highest > top, "strokes bring them up for air");
    // held under (pushed down hard), they run out of breath and drown
    let (mut s, _) = dropped_in(crate::level::HazardKind::Water, 0.5);
    for _ in 0..60 {
        s.step(&[0]);
    }
    let mut drowned = None;
    for k in 0..60 * 40 {
        for id in s.fighters[0].particle_ids() {
            let p = &mut s.world.particles[id as usize];
            p.vel.y = p.vel.y.min(0.0) - 0.8;
            p.vel.x *= 0.9;
        }
        s.step(&[0]);
        if !s.fighters[0].alive {
            drowned = Some(k);
            break;
        }
    }
    eprintln!("held under: drowned after {drowned:?} frames (breath {})", s.fighters[0].breath);
    let k = drowned.expect("held under, they drown");
    assert!(k > crate::sim::AIR as usize, "not before their breath runs out ({k})");
    assert!(k > 60 * 18, "it takes a while ({:.1} s)", k as f32 / 60.0);
}

#[test]
fn falling_in_splashes_and_raises_the_level() {
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 3.0);
    let before = s.fluid.drops.len();
    let rest = h.a.y + h.size.y;
    let mut flung = 0;
    let mut level = f32::MIN;
    for _ in 0..100 {
        s.fighters[0].down = 200;
        s.step(&[0]);
        flung = flung.max(s.fluid.drops.iter().filter(|d| d.pos.y > rest + 0.4).count());
    }
    // and settles again with the body floating (limp) in it
    for _ in 0..360 {
        s.fighters[0].down = 200;
        s.step(&[0]);
    }
    let mut ys: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos.x - h.a.x).abs() < h.size.x).map(|d| d.pos.y).collect();
    ys.sort_by(|a, b| a.total_cmp(b));
    level = level.max(ys[ys.len() * 97 / 100]);
    let mut speeds: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos.x - h.a.x).abs() < h.size.x).map(|d| d.vel.length()).collect();
    speeds.sort_by(|a, b| a.total_cmp(b));
    let fastest = speeds[speeds.len() * 95 / 100];
    eprintln!("splash: up to {flung} drops thrown over 0.4 m above the surface; settled level {level:.2} (filled to {rest:.2}), {} of {before} drops left, 95% of the pool under {fastest:.2} m/s", s.fluid.drops.len());
    assert!(flung >= 5, "drops thrown up");
    assert_eq!(s.fluid.drops.len(), before, "none lost");
    assert!(fastest < 1.0, "settling down again");
}

#[test]
fn liquids_are_deterministic_under_rollback() {
    let (s, _) = dropped_in(crate::level::HazardKind::Water, 2.0);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for k in 0..240 {
        let b = if k > 100 { RIGHT | if k % 20 == 0 { JUMP } else { 0 } } else { 0 };
        st.advance(&[input::encode(b, Vec2::new(0.6, 0.0))]).unwrap();
    }
    let (s, _) = dropped_in(crate::level::HazardKind::Lava, 1.0);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for _ in 0..200 {
        st.advance(&[0]).unwrap();
    }
}

#[test]
#[ignore]
fn screenshot_liquids() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    use crate::level::HazardKind;
    for (kind, name, frames) in [(HazardKind::Water, "water", [8usize, 30, 150]), (HazardKind::Lava, "lava", [8, 30, 150])] {
        let (mut s, h) = dropped_in(kind, 2.5);
        let mut k = 0;
        for &at in &frames {
            while k < at {
                s.step(&[0]);
                k += 1;
            }
            let mut c = CloseUp { game: s.clone(), at: Vec2::new(h.a.x, h.a.y + h.size.y - 0.3) };
            stickfell_app::capture_headless(&mut c, 0, (800, 520), format!("{dir}/liquid_{name}_{at:03}.png").as_ref()).unwrap();
        }
    }
    // the arenas' acid
    for (level, name) in [(crate::level::TRAINING, "acid_yard"), (crate::level::GAUNTLET, "acid_gauntlet")] {
        let mut s = GameState::new(1, level, 7);
        let h = *s.hazards.iter().find(|h| h.kind == HazardKind::Acid).unwrap();
        lay_flat(&mut s, 0, Vec2::new(h.a.x, h.a.y + h.size.y + 2.0));
        for k in 1..=120 {
            s.step(&[0]);
            if k == 40 || k == 120 {
                let mut c = CloseUp { game: s.clone(), at: Vec2::new(h.a.x, h.a.y + h.size.y) };
                stickfell_app::capture_headless(&mut c, 0, (800, 520), format!("{dir}/liquid_{name}_{k}.png").as_ref()).unwrap();
            }
        }
    }
}

#[test]
#[ignore]
fn trace_float() {
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 1.5);
    for k in 0..240 {
        s.fighters[0].down = 200;
        s.step(&[0]);
        if k % 10 == 0 {
            let f = &s.fighters[0];
            let p = |slot| f.pos(&s.world, slot);
            eprintln!("    sample pelvis {:?} head {:?}", s.fluid.sample(p(PELVIS)).amount, s.fluid.sample(p(HEAD)).amount);
            eprintln!("k{k:3} pelvis ({:.2},{:.2}) neck ({:.2},{:.2}) head ({:.2},{:.2}) foot y {:.2} surface {:.2} swim {} grounded {} vel {:.2}", p(PELVIS).x, p(PELVIS).y, p(NECK).x, p(NECK).y, p(HEAD).x, p(HEAD).y, p(fighter::FOOT_A).y, liquid_top(&s, &h), f.swimming, f.grounded, f.body_vel(&s.world).y);
        }
    }
}

#[test]
#[ignore]
fn trace_acid_drop() {
    let mut s = GameState::new(1, crate::level::GAUNTLET, 7);
    let h = *s.hazards.iter().find(|h| h.kind == crate::level::HazardKind::Acid).unwrap();
    lay_flat(&mut s, 0, Vec2::new(h.a.x, h.a.y + h.size.y + 2.0));
    for k in 0..300 {
        s.step(&[0]);
        let (i, d) = s.fluid.drops.iter().enumerate().max_by(|a, b| a.1.vel.length().total_cmp(&b.1.vel.length())).unwrap();
        let low = s.fighters[0].particle_ids().map(|id| s.world.particles[id as usize].pos.y).fold(f32::MAX, f32::min);
        let high = s.fluid.drops.iter().filter(|d| d.pos.y > h.a.y + h.size.y + 0.5).count();
        let wet = s.fighters[0].wet;
        eprintln!("k{k:2} body low {low:.2} vy {:.2} wet {wet} | fastest drop #{i} at ({:.2},{:.2}) vel ({:.1},{:.1}) | {high} drops 0.5 m up", s.fighters[0].body_vel(&s.world).y, d.pos.x, d.pos.y, d.vel.x, d.vel.y);
        if [0, 30, 44, 60, 90, 150, 240, 299].contains(&k) {
            // the level: median height of the top drop in each 0.24 m column across the pit
            let mut tops = Vec::new();
            let mut x = h.a.x - h.size.x + 0.12;
            while x < h.a.x + h.size.x {
                if let Some(t) = s.fluid.drops.iter().filter(|d| (d.pos.x - x).abs() < 0.12).map(|d| d.pos.y).reduce(f32::max) {
                    tops.push(t);
                }
                x += 0.24;
            }
            tops.sort_by(|a, b| a.total_cmp(b));
            eprintln!("k{k}: median surface {:.3} m over {} columns (rect top {:.3}), mean density {:.3} of rest", tops[tops.len() / 2], tops.len(), h.a.y + h.size.y, s.fluid.mean_density());
        }
        if k == 39 {
            let mut up: Vec<(i32, i32)> = s.fluid.drops.iter().filter(|d| d.pos.y > h.a.y + h.size.y + 0.3).map(|d| ((d.pos.x * 10.0) as i32, (d.pos.y * 10.0) as i32)).collect();
            up.sort();
            eprintln!("drops 0.3+ up (x10): {up:?}");
            let total = s.fluid.drops.len();
            let under = s.fluid.drops.iter().filter(|d| d.pos.y < h.a.y + h.size.y).count();
            eprintln!("{total} drops, {under} under the old surface; x range {:.2}..{:.2}", s.fluid.drops.iter().map(|d| d.pos.x).fold(f32::MAX, f32::min), s.fluid.drops.iter().map(|d| d.pos.x).fold(f32::MIN, f32::max));
        }
    }
}

#[test]
#[ignore]
fn measure_body_displacement() {
    // the area liquid can't get into around a standing fighter, on a 5 mm grid,
    // for the drops' clearance around each capsule (0.4 and 1.0 of a drop's radius)
    let s = sandbox(&[]);
    let f = &s.fighters[0];
    let drop_r = crate::liquid::DROP_SPACING * 0.5;
    let segs: Vec<(Vec2, Vec2, f32)> = (0..fighter::BONES.len() as u32)
        .map(|k| {
            let sg = s.world.segments[(f.seg_base + k) as usize];
            (s.world.particles[sg.a as usize].pos, s.world.particles[sg.b as usize].pos, sg.radius)
        })
        .collect();
    let (lo, hi) = (Vec2::new(-1.5, -0.5), Vec2::new(1.5, 2.5));
    let step = 0.005;
    for clear in [0.0f32, 0.4, 1.0] {
        let mut n = 0u64;
        let (nx, ny) = (((hi.x - lo.x) / step) as i32, ((hi.y - lo.y) / step) as i32);
        for iy in 0..ny {
            for ix in 0..nx {
                let p = lo + Vec2::new(ix as f32 + 0.5, iy as f32 + 0.5) * step;
                if segs.iter().any(|&(a, b, r)| stickfell_physics::collide::point_segment(p, a, b).0 < r + drop_r * clear) {
                    n += 1;
                }
            }
        }
        eprintln!("clearance {clear} x drop radius ({:.3} m): body keeps {:.3} m^2 clear of liquid", drop_r * clear, n as f64 * (step * step) as f64);
    }
}

#[test]
#[ignore]
fn measure_splash() {
    for strength in [1.0f32, 1.05] {
        let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 3.0);
        s.fluid.splash_strength = strength;
        let rest = h.a.y + h.size.y;
        let (mut over4, mut over8, mut peak) = (0, 0, f32::MIN);
        for _ in 0..100 {
            s.fighters[0].down = 200;
            s.step(&[0]);
            over4 = over4.max(s.fluid.drops.iter().filter(|d| d.pos.y > rest + 0.4).count());
            over8 = over8.max(s.fluid.drops.iter().filter(|d| d.pos.y > rest + 0.8).count());
            peak = peak.max(s.fluid.drops.iter().map(|d| d.pos.y).fold(f32::MIN, f32::max));
        }
        eprintln!("strength {strength}: up to {over4} drops 0.4 m up, {over8} 0.8 m up, highest {:.2} m above the surface", peak - rest);
    }
}

#[test]
#[ignore]
fn measure_settling() {
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 3.0);
    for k in 1..=900 {
        s.fighters[0].down = 200;
        s.step(&[0]);
        if k % 100 == 0 {
            let mut v: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos.x - h.a.x).abs() < h.size.x).map(|d| d.vel.length()).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            let b = s.fighters[0].body_vel(&s.world);
            let limb = s.fighters[0].particle_ids().map(|id| (s.world.particles[id as usize].vel - b).length()).fold(0.0f32, f32::max);
            eprintln!("frame {k}: pool speed median {:.3}, 95% {:.3}, max {:.3} m/s | body vel ({:.2},{:.2}), fastest limb vs body {limb:.2}", v[v.len() / 2], v[v.len() * 95 / 100], v[v.len() - 1], b.x, b.y);
        }
    }
    // the same pool with nobody in it
    let (mut s, h) = dropped_in(crate::level::HazardKind::Water, 0.0);
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(-12.0 - me.x, 1.0 - me.y));
    for k in 1..=600 {
        s.step(&[0]);
        if k % 200 == 0 {
            let mut v: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos.x - h.a.x).abs() < h.size.x).map(|d| d.vel.length()).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            eprintln!("empty pool frame {k}: median {:.3}, 95% {:.3}, max {:.3} m/s", v[v.len() / 2], v[v.len() * 95 / 100], v[v.len() - 1]);
        }
    }
}

#[test]
#[ignore]
fn measure_floating_body() {
    // how much a limp body keeps moving in the pool, against the drops' push-back
    for push in [0.5f32] {
        let (mut s, _) = dropped_in(crate::level::HazardKind::Water, 1.0);
        s.fluid.push_back = push;
        let (mut vy, mut limb, mut ax, mut ay) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut n = 0.0;
        for k in 1..=600 {
            s.fighters[0].down = 200;
            s.step(&[0]);
            if k > 300 {
                let b = s.fighters[0].body_vel(&s.world);
                vy += b.length();
                ax += b.x.abs();
                ay += b.y.abs();
                limb += s.fighters[0].particle_ids().map(|id| (s.world.particles[id as usize].vel - b).length()).sum::<f32>() / fighter::PARTICLES as f32;
                n += 1.0;
            }
        }
        eprintln!("push_back {push}: frames 300-600, mean body speed {:.3} m/s (|vx| {:.3}, |vy| {:.3}), mean limb speed vs body {:.3} m/s, pelvis x {:.2}", vy / n, ax / n, ay / n, limb / n, s.fighters[0].pos(&s.world, PELVIS).x);
    }
}

#[test]
#[ignore]
fn measure_blood_in_water() {
    let (mut s, h) = still_pool();
    let at = Vec2::new(h.a.x, h.a.y);
    // a burst of blood right in the middle of the pool
    s.spawn_blood(at, Vec2::Y, 30, 1.0);
    let sparks = s.sparks.iter().filter(|sp| sp.blood).count();
    s.step(&[0]);
    let left = s.sparks.iter().filter(|sp| sp.blood).count();
    let total = s.fluid.total_dye();
    eprintln!("{sparks} blood sparks in the water, {left} still flying after a frame; dye in the water {total:.3} (each spark {})", crate::liquid::BLOOD_DYE);
    for k in 1..=900 {
        s.step(&[0]);
        if [60, 180, 300, 600, 900].contains(&k) {
            let (r1, n1) = stain(&s, at, 0.01);
            let (r10, n10) = stain(&s, at, 0.1);
            let peak = s.fluid.drops.iter().map(|d| d.dye).fold(0.0f32, f32::max);
            eprintln!("{:.0} s: dye {:.3} total; >=0.01 in {n1} drops out to {r1:.2} m, >=0.1 in {n10} drops out to {r10:.2} m; strongest {peak:.3}", k as f32 / 60.0, s.fluid.total_dye());
        }
    }
}

#[test]
fn blood_mixes_into_water_and_spreads_without_vanishing() {
    let (mut s, h) = still_pool();
    let at = Vec2::new(h.a.x, h.a.y);
    s.spawn_blood(at, Vec2::Y, 20, 1.0);
    s.step(&[0]);
    assert_eq!(s.sparks.iter().filter(|sp| sp.blood).count(), 0, "blood in the water is taken in by it");
    let total = s.fluid.total_dye();
    let (_, near) = stain(&s, at, 0.01);
    for _ in 0..300 {
        s.step(&[0]);
    }
    let (reach, wide) = stain(&s, at, 0.01);
    let after = s.fluid.total_dye();
    eprintln!("blood: {total:.3} dye in {near} drops, 5 s later {after:.3} in {wide} drops out to {reach:.2} m");
    assert!((after - total).abs() < 1e-3 * total, "none made or lost spreading");
    assert!(wide > near * 2 && reach > 1.0, "it spreads through the water");
    assert!(s.fluid.drops.iter().filter(|d| d.kind != crate::liquid::WATER_DROP).all(|d| d.dye == 0.0), "only water takes it");
}

#[test]
fn bleeding_into_water_is_deterministic_under_rollback() {
    let (mut s, h) = still_pool();
    s.spawn_blood(Vec2::new(h.a.x, h.a.y), Vec2::Y, 20, 1.0);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for _ in 0..120 {
        st.advance(&[0, 0, 0, 0]).unwrap();
    }
}

#[test]
#[ignore]
fn screenshot_blood_in_water() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let (mut s, h) = still_pool();
    let at = Vec2::new(h.a.x - 0.8, h.a.y + 0.4);
    for k in 1..=360 {
        // a wound pouring into the water for the first two seconds
        if k <= 120 && k % 6 == 0 {
            s.spawn_blood(at, Vec2::Y, 3, 0.5);
        }
        s.step(&[0]);
        if [60, 180, 360].contains(&k) {
            let mut c = CloseUp { game: s.clone(), at: Vec2::new(h.a.x, h.a.y + 0.3) };
            stickfell_app::capture_headless(&mut c, 0, (800, 520), format!("{dir}/blood_water_{k}.png").as_ref()).unwrap();
        }
    }
}

#[test]
#[ignore]
fn measure_flood_cost() {
    use crate::level::{HazardDef, HazardKind};
    for depth in [1.0f32, 1.5, 2.0] {
        let mut s = GameState::new(4, crate::level::TRAINING, 7);
        let before = s.fluid.drops.len();
        let a = Vec2::new(-13.0, depth * 0.5);
        s.add_hazard(HazardDef { kind: HazardKind::Water, a, b: a, size: Vec2::new(42.5, depth * 0.5), speed: 0.0 });
        let drops = s.fluid.drops.len();
        for _ in 0..120 {
            s.step(&[0, 0, 0, 0]);
        }
        let n = 300;
        let t = std::time::Instant::now();
        for _ in 0..n {
            s.step(&[0, 0, 0, 0]);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("flood {depth} m deep over the yard floor: {} more drops ({drops} in all): {ms:.3} ms per frame", drops - before);
    }
}

#[test]
fn the_lever_up_top_opens_and_shuts_the_flood_hatch() {
    let mut s = at_the_lever();
    let def = s.trap.def.unwrap();
    let before = s.fluid.drops.len();
    throw_lever(&mut s);
    assert!(s.trap.open, "pressing E by the lever throws it open");
    // pouring at its rate once the doors are open
    let secs = 3.0;
    for _ in 0..(secs * 60.0) as u32 {
        s.step(&[0, 0, 0, 0]);
    }
    let poured = s.fluid.drops.len() - before;
    let want = def.pour_rate * secs;
    eprintln!("open {secs} s: {poured} drops in (pouring {} a second; {want:.0} over {secs} s less the doors opening)", def.pour_rate);
    assert!(poured as f32 > want * 0.5 && poured as f32 <= want, "it pours while open ({poured})");
    assert!(s.trap.hatch(s.round_frame) == 1.0);
    // thrown back: it shuts, and no more comes
    throw_lever(&mut s);
    assert!(!s.trap.open, "pressing E again shuts it");
    let shut = s.fluid.drops.len();
    for _ in 0..120 {
        s.step(&[0, 0, 0, 0]);
    }
    assert_eq!(s.fluid.drops.len(), shut, "shut, no more water comes in");
    assert_eq!(s.trap.hatch(s.round_frame), 0.0, "the doors are shut");
    // and open again
    throw_lever(&mut s);
    assert!(s.trap.open);
    for _ in 0..60 {
        s.step(&[0, 0, 0, 0]);
    }
    assert!(s.fluid.drops.len() > shut, "opened again, it pours again");
}

#[test]
#[ignore]
fn measure_full_flood() {
    let caps: Vec<u32> = std::env::var("FLOOD_CAPS").ok().map_or(vec![0], |v| v.split(',').map(|c| c.parse().unwrap()).collect());
    for cap in caps {
    let mut s = at_the_lever();
    if cap > 0 {
        s.trap.def = Some(crate::level::TrapDef { max_drops: cap, ..s.trap.def.unwrap() });
    }
    let def = s.trap.def.unwrap();
    throw_lever(&mut s);
    let mut frames = 0;
    while (s.fluid.drops.len() as u32) < def.max_drops && frames < 60 * 90 {
        s.step(&[0, 0, 0, 0]);
        frames += 1;
    }
    for _ in 0..60 * 10 {
        s.step(&[0, 0, 0, 0]);
    }
    let n = 120;
    let t = std::time::Instant::now();
    for _ in 0..n {
        s.step(&[0, 0, 0, 0]);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
    let mut tops: Vec<f32> = (-50..=28)
        .step_by(4)
        .filter_map(|x| s.fluid.drops.iter().filter(|d| (d.pos.x - x as f32).abs() < 0.3 && d.kind == crate::liquid::WATER_DROP).map(|d| d.pos.y).reduce(f32::max))
        .collect();
    tops.sort_by(|a, b| a.total_cmp(b));
    eprintln!("full ({} drops, cap {}) after {:.1} s pouring; water surface across the yard: lowest {:.2}, median {:.2}, highest {:.2} m; {ms:.2} ms a frame settled", s.fluid.drops.len(), def.max_drops, frames as f32 / 60.0, tops[0], tops[tops.len() / 2], tops[tops.len() - 1]);
    assert!(s.fluid.drops.len() as u32 <= def.max_drops);
    }
}

#[test]
fn the_flood_is_deterministic_under_rollback() {
    let mut s = at_the_lever();
    for k in 0..10 {
        let b = if k == 5 { PICKUP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.2))]);
    }
    assert!(s.trap.open);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for _ in 0..150 {
        st.advance(&[0, 0, 0, 0]).unwrap();
    }
}

#[test]
#[ignore]
fn measure_lever_reach() {
    let mut s = at_the_lever();
    let def = s.trap.def.unwrap();
    let handle = def.handle(0.0);
    for k in 0..10 {
        let b = if k == 5 { PICKUP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.2))]);
        let f = &s.fighters[0];
        let (a, bb) = (f.pos(&s.world, HAND_A), f.pos(&s.world, fighter::HAND_B));
        eprintln!("k{k}: pelvis {:?} hands {:?} {:?}, handle {:?}: {:.2} / {:.2} m; pressed {} down {} alive {}", f.pos(&s.world, PELVIS), a, bb, handle, (a - handle).length(), (bb - handle).length(), f.pressed(PICKUP), f.down, f.alive);
    }
}

#[test]
#[ignore]
fn measure_flood_level() {
    let mut s = at_the_lever();
    for k in 0..10 {
        let b = if k == 5 { PICKUP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.2))]);
    }
    for k in 1..=60 * 40 {
        s.step(&[0]);
        if k % (60 * 8) == 0 {
            let mut cols = String::new();
            for x in [-50.0f32, -40.0, -30.0, -18.0, -10.0, -6.0, 0.0, 6.0, 16.0, 25.0] {
                let top = s.fluid.drops.iter().filter(|d| (d.pos.x - x).abs() < 0.3).map(|d| d.pos.y).fold(f32::MIN, f32::max);
                cols += &format!(" x{x}:{top:.2}");
            }
            eprintln!("{:.0} s ({} drops, density {:.3}):{cols}", k as f32 / 60.0, s.fluid.drops.len(), s.fluid.mean_density());
        }
    }
}

#[test]
#[ignore]
fn measure_flooded_cost() {
    let mut s = at_the_lever();
    for k in 0..10 {
        let b = if k == 5 { PICKUP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.2))]);
    }
    for _ in 0..60 * 30 {
        s.step(&[0, 0, 0, 0]);
    }
    let n = 300;
    let t = std::time::Instant::now();
    for _ in 0..n {
        s.step(&[0, 0, 0, 0]);
    }
    eprintln!("flooded yard: {} drops, {:.3} ms per simulated frame", s.fluid.drops.len(), t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    let follow = 1;
    eprintln!("camera on fighter {follow} at {:?}", s.fighters[follow].pos(&s.world, PELVIS));
    let mut sp = crate::Spectate { game: s, follow, cam: Vec2::ZERO, levels: crate::level::levels() };
    stickfell_app::bench_headless(&mut sp, 1, (1920, 1080), 200).unwrap();
}

#[test]
#[ignore]
fn screenshot_flood() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = at_the_lever();
    let def = s.trap.def.unwrap();
    for k in 0..10 {
        let b = if k == 5 { PICKUP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.2))]);
    }
    let mut c = CloseUp { game: s.clone(), at: def.lever + Vec2::new(0.0, 0.8) };
    stickfell_app::capture_headless(&mut c, 0, (640, 420), format!("{dir}/flood_lever.png").as_ref()).unwrap();
    for k in 1..=60 * 30 {
        s.step(&[0, 0, 0, 0]);
        if [120, 600, 1800].contains(&k) {
            let mut sp = crate::Spectate { game: s.clone(), follow: 1, cam: Vec2::ZERO, levels: crate::level::levels() };
            stickfell_app::capture_headless(&mut sp, 0, (1100, 620), format!("{dir}/flood_{k}.png").as_ref()).unwrap();
        }
    }
}

#[test]
fn water_poured_on_lava_boils_off_and_cools_it_to_rock() {
    use crate::liquid::{LAVA_DROP, WATER_DROP};
    let mut s = GameState::new(4, crate::level::TRAINING, 7);
    let h = *s.hazards.iter().find(|h| h.kind == crate::level::HazardKind::Lava).unwrap();
    let lava = s.fluid.drops.iter().filter(|d| d.kind == LAVA_DROP).count();
    let top = h.a.y + h.size.y;
    // a stream of water onto the middle of the pit
    let mut poured = 0;
    for k in 0..600 {
        if k < 300 && k % 2 == 0 {
            for i in 0..6 {
                let x = h.a.x - 0.36 + i as f32 * 0.12;
                s.fluid.pour(Vec2::new(x, top + 1.5), Vec2::new(0.0, -3.0), WATER_DROP);
                poured += 1;
            }
        }
        s.step(&[0, 0, 0, 0]);
    }
    let water = s.fluid.drops.iter().filter(|d| d.kind == WATER_DROP && (d.pos.x - h.a.x).abs() < h.size.x + 2.0 && d.pos.y > h.a.y - h.size.y - 0.5).count();
    let lava_now = s.fluid.drops.iter().filter(|d| d.kind == LAVA_DROP).count();
    let rock = s.fluid.drops.iter().filter(|d| d.kind == LAVA_DROP && d.dye >= 1.0).count();
    let warm = s.fluid.drops.iter().filter(|d| d.kind == LAVA_DROP && d.dye > 0.0 && d.dye < 1.0).count();
    let steam = s.sparks.iter().filter(|sp| sp.steam).count();
    eprintln!("lava pit: {poured} water drops poured, {water} still there; lava {lava} -> {lava_now} drops, {rock} cooled to rock, {warm} cooling; {steam} steam puffs flying");
    assert_eq!(lava_now, lava, "lava doesn't vanish, it cools");
    assert!(water < poured, "water boils off it");
    assert!(rock > 0, "and cools it to rock");
}

#[test]
#[ignore]
fn measure_water_on_lava_layers() {
    use crate::liquid::{LAVA_DROP, WATER_DROP};
    let mut s = GameState::new(4, crate::level::TRAINING, 7);
    let h = *s.hazards.iter().find(|h| h.kind == crate::level::HazardKind::Lava).unwrap();
    let top = h.a.y + h.size.y;
    for k in 0..600 {
        if k < 300 && k % 2 == 0 {
            for i in 0..6 {
                s.fluid.pour(Vec2::new(h.a.x - 0.36 + i as f32 * 0.12, top + 1.5), Vec2::new(0.0, -3.0), WATER_DROP);
            }
        }
        s.step(&[0, 0, 0, 0]);
    }
    let mean_y = |kind: u8, rock: Option<bool>| {
        let ys: Vec<f32> = s.fluid.drops.iter().filter(|d| d.kind == kind && (d.pos.x - h.a.x).abs() < h.size.x && rock.is_none_or(|r| (d.dye >= 1.0) == r)).map(|d| d.pos.y).collect();
        (ys.iter().sum::<f32>() / ys.len().max(1) as f32, ys.len())
    };
    let (w, wn) = mean_y(WATER_DROP, None);
    let (hot, hn) = mean_y(LAVA_DROP, Some(false));
    let (rock, rn) = mean_y(LAVA_DROP, Some(true));
    eprintln!("over the pit: water mean y {w:.2} ({wn} drops), hot lava {hot:.2} ({hn}), rock {rock:.2} ({rn}); pit bottom {:.2}, top {top:.2}", h.a.y - h.size.y);
}

#[test]
#[ignore]
fn measure_wading_depth() {
    for depth in [0.3f32, 0.6, 0.9, 1.2, 1.6] {
        let s = &mut wading(depth);
        for _ in 0..120 {
            step(s, 0, Vec2::new(0.6, 0.0));
        }
        let f = &s.fighters[0];
        let pelvis = f.pos(&s.world, PELVIS);
        let at = s.fluid.sample(pelvis).total();
        let top = s.fluid.drops.iter().filter(|d| d.pos.x.abs() < 3.0).map(|d| d.pos.y).fold(f32::MIN, f32::max);
        eprintln!("water {depth} m deep (surface now {top:.2}): pelvis at {:.2} m, liquid there {at:.3}, grounded {}", pelvis.y, f.grounded);
    }
}

#[test]
fn swimmers_swim_laid_out_the_way_they_go_and_tread_water_upright() {
    // a tank 12 m wide and 2.5 m deep, the other fighter well out of it
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    let surface = 2.5;
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    let tread = torso_tilt(&s, 0);
    let swimming = s.fighters[0].swimming;
    // swim right along the pool
    let x0 = s.fighters[0].pos(&s.world, PELVIS).x;
    let mut flat = 0.0;
    for k in 0..60 {
        s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
        if k >= 30 {
            flat += torso_tilt(&s, 0) / 30.0;
        }
    }
    let x1 = s.fighters[0].pos(&s.world, PELVIS).x;
    // dive, then come back up
    let y0 = s.fighters[0].pos(&s.world, PELVIS).y;
    for _ in 0..60 {
        s.step(&[input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0]);
    }
    let deep = s.fighters[0].pos(&s.world, PELVIS).y;
    for _ in 0..60 {
        s.step(&[input::encode(JUMP, Vec2::new(0.6, 0.0)), 0]);
    }
    let up = s.fighters[0].pos(&s.world, PELVIS).y;
    eprintln!("swim: swimming {swimming}, treading tilt {tread:.2} rad; swam {:.2} m right in 1 s, mean tilt {flat:.2} rad; dived {y0:.2} -> {deep:.2}, back up to {up:.2} (surface {surface:.2})", x1 - x0);
    assert!(swimming, "in out of their depth, they swim");
    assert!(tread < 0.6, "treading water upright ({tread:.2})");
    assert!(x1 - x0 > 1.5, "swimming gets them along ({:.2} m)", x1 - x0);
    assert!(flat > 0.9, "laid out swimming ({flat:.2} rad from upright)");
    // (measured 0.45 m in the first second: the water pushes back on the head going down)
    assert!(deep < y0 - 0.35, "crouch dives ({:.2} m)", y0 - deep);
    assert!(up > deep + 0.5, "jump swims back up");
}

#[test]
fn no_shoulder_throws_waist_deep_in_water() {
    for (depth, can) in [(0.0f32, true), (1.2, false)] {
        let mut s = if depth > 0.0 { wading(depth) } else { pair(0.9) };
        for _ in 0..60 {
            s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
        }
        let (mut thrown, mut held, mut peak) = (false, false, f32::MIN);
        for k in 0..90 {
            let b = shoulder_throw_input(&s, k);
            s.step(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]);
            thrown |= s.fighters[0].shoulder > 0;
            held |= s.fighters[0].hold.is_some();
            peak = peak.max(s.fighters[1].pos(&s.world, NECK).y);
        }
        eprintln!("water {depth} m: wade {:.2}, got hold of them {held}, shoulder throw {thrown}, their neck went up to {peak:.2} m", s.fighters[0].wade);
        assert!(held, "the grab itself works ({depth} m of water)");
        assert_eq!(thrown, can, "{depth} m of water");
    }
}

#[test]
#[ignore]
fn trace_swim() {
    let (mut s, _) = dropped_in(crate::level::HazardKind::Water, 0.3);
    s.fighters[0].down = 0;
    for _ in 0..90 {
        s.step(&[0]);
    }
    for k in 0..150 {
        let b = if k < 60 { RIGHT } else if k < 105 { CROUCH } else { JUMP };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.0))]);
        if k % 8 == 0 {
            let f = &s.fighters[0];
            eprintln!("k{k:3} key {b:3}: swimming {} wade {:.2} grounded {} down {} dir {:?} tilt {:.2} pelvis {:?} vel {:?} strength {:.2}", f.swimming, f.wade, f.grounded, f.down, f.swim_dir(), torso_tilt(&s, 0), f.pos(&s.world, PELVIS), f.body_vel(&s.world), f.strength());
        }
    }
}

#[test]
#[ignore]
fn measure_standing_underwater() {
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    let mut bob = 0.0f32;
    for k in 0..240 {
        s.step(&[0, 0]);
        if k >= 120 {
            bob = bob.max(s.fighters[0].body_vel(&s.world).y.abs());
        }
        if k % 30 == 29 {
            let f = &s.fighters[0];
            let subs: Vec<String> = f.particle_ids().map(|id| format!("{:.2}", s.fluid.sample(s.world.particles[id as usize].pos).total())).collect();
            eprintln!("k{k}: pelvis {:?} grounded {} swimming {} wade {:.2} vel {:?}; liquid at each part {:?}", f.pos(&s.world, PELVIS), f.grounded, f.swimming, f.wade, f.body_vel(&s.world), subs);
            let _ = &subs;
        }
    }
    eprintln!("treading, seconds 2-4: fastest bob {bob:.2} m/s");
}

#[test]
#[ignore]
fn trace_grab_in_water() {
    let mut s = wading(1.2);
    for _ in 0..60 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
    }
    for k in 0..40 {
        let b = shoulder_throw_input(&s, k);
        s.step(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]);
        if k % 4 == 0 {
            let (a, v) = (&s.fighters[0], &s.fighters[1]);
            eprintln!("k{k}: me {:?} swimming {} wade {:.2} grabbing {} hold {:?} | them {:?} swimming {} | hand B {:?} their neck {:?}", a.pos(&s.world, PELVIS), a.swimming, a.wade, a.grabbing, a.hold, v.pos(&s.world, PELVIS), v.swimming, a.pos(&s.world, fighter::HAND_B), v.pos(&s.world, NECK));
        }
    }
}

#[test]
#[ignore]
fn screenshot_swimming() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    let shot = |s: &GameState, name: &str| {
        let at = s.fighters[0].pos(&s.world, PELVIS) + Vec2::new(0.0, 0.2);
        let mut c = CloseUp { game: s.clone(), at };
        stickfell_app::capture_headless(&mut c, 0, (640, 420), format!("{dir}/swim_{name}.png").as_ref()).unwrap();
    };
    shot(&s, "tread");
    for k in 0..40 {
        s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
        if k == 25 || k == 32 {
            shot(&s, &format!("crawl_{k}"));
        }
    }
    for _ in 0..30 {
        s.step(&[input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0]);
    }
    shot(&s, "dive");
}

#[test]
#[ignore]
fn measure_sword_in_water() {
    for with_sword in [false, true] {
        let (mut s, h) = still_pool();
        let at = Vec2::new(h.a.x, h.a.y + h.size.y + 1.0);
        let wi = if with_sword {
            s.spawn_weapon(0, at);
            Some(s.weapons.len() - 1)
        } else {
            None
        };
        let mut last_ang = 0.0f32;
        for k in 1..=1200u32 {
            s.step(&[0]);
            if k % 120 != 0 {
                if let Some(wi) = wi {
                    let w = &s.weapons[wi];
                    let (a, b) = (s.world.particles[w.particle(0) as usize].pos, s.world.particles[w.particle(w.n - 1) as usize].pos);
                    last_ang = dmath::angle(b - a);
                }
                continue;
            }
            let (centre, sword) = match wi {
                Some(wi) => {
                    let w = &s.weapons[wi];
                    let (a, b) = (s.world.particles[w.particle(0) as usize].pos, s.world.particles[w.particle(w.n - 1) as usize].pos);
                    let spin = dmath::angle_diff(dmath::angle(b - a), last_ang) * 60.0;
                    let (vmax, vmean) = weapon_speeds(&s, wi);
                    ((a + b) * 0.5, format!("sword mid ({:+.2},{:+.2}) ang {:+.2} spin {:+.2} rad/s, speed max {vmax:.3} mean {vmean:.3}", (a + b).x * 0.5, (a + b).y * 0.5, dmath::angle(b - a), spin))
                }
                None => (Vec2::new(h.a.x, h.a.y - h.size.y + 0.3), String::from("no sword")),
            };
            let near: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos - centre).length() < 0.6).map(|d| d.vel.length()).collect();
            let far: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos.x - h.a.x).abs() < h.size.x && (d.pos - centre).length() > 1.5).map(|d| d.vel.length()).collect();
            let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
            let mx = |v: &[f32]| v.iter().copied().fold(0.0f32, f32::max);
            eprintln!("{:4.1} s: {sword} | drops within 0.6 m: {} mean {:.3} max {:.3} | far: mean {:.3} max {:.3}", k as f32 / 60.0, near.len(), mean(&near), mx(&near), mean(&far), mx(&far));
        }
    }
}

#[test]
#[ignore]
fn trace_sword_in_water() {
    let (mut s, h) = still_pool();
    s.spawn_weapon(0, Vec2::new(h.a.x, h.a.y + h.size.y + 1.0));
    let wi = s.weapons.len() - 1;
    let ids: Vec<usize> = (0..s.weapons[wi].n).map(|k| s.weapons[wi].particle(k) as usize).collect();
    let segs: Vec<usize> = (0..s.weapons[wi].nseg as usize).map(|k| s.weapons[wi].seg_base as usize + k).collect();
    eprintln!("sword particles {ids:?} segs {segs:?}");
    for &sg in &segs {
        let g = s.world.segments[sg];
        eprintln!("  seg {sg}: {}-{} radius {:.3} enabled {}", g.a, g.b, g.radius, g.enabled);
    }
    for &id in &ids {
        let p = s.world.particles[id];
        eprintln!("  particle {id}: inv_mass {:.3} radius {:.3}", p.inv_mass, p.radius);
    }
    let ke = |s: &GameState| ids.iter().map(|&i| { let p = s.world.particles[i]; if p.inv_mass > 0.0 { 0.5 / p.inv_mass * p.vel.length_squared() } else { 0.0 } }).sum::<f32>();
    for k in 1..=900u32 {
        // what each stage does to the sword's energy, this frame
        let mut a = s.clone();
        let e0 = ke(&a);
        a.update_liquids(DT);
        let e1 = ke(&a);
        a.world.step(DT);
        let e2 = ke(&a);
        a.fluid.step(DT, &mut a.world);
        let e3 = ke(&a);
        s.step(&[0]);
        if k % 30 == 0 || (k > 60 && k < 140 && k % 5 == 0) {
            let mid = (s.world.particles[ids[0]].pos + s.world.particles[ids[ids.len() - 1]].pos) * 0.5;
            let near: Vec<&stickfell_physics::Drop> = s.fluid.drops.iter().filter(|d| (d.pos - mid).length() < 0.7).collect();
            let fast = near.iter().filter(|d| d.vel.length() > 2.0).count();
            eprintln!("{:5.2}s mid ({:+.2},{:+.2}) KE {e0:.3} -> liquids {e1:.3} -> world {e2:.3} -> fluid {e3:.3} | {} drops near, {fast} over 2 m/s", k as f32 / 60.0, mid.x, mid.y, near.len());
        }
    }
    let mid = (s.world.particles[ids[0]].pos + s.world.particles[ids[ids.len() - 1]].pos) * 0.5;
    for d in s.fluid.drops.iter().filter(|d| (d.pos - mid).length() < 0.7) {
        let inside = s.world.statics.iter().any(|o| o.collide_circle(d.pos, s.fluid.radius).is_some());
        let near_seg = segs.iter().map(|&sg| { let g = s.world.segments[sg]; let (pa, pb) = (s.world.particles[g.a as usize].pos, s.world.particles[g.b as usize].pos); stickfell_physics::collide::point_segment(d.pos, pa, pb).0 - g.radius }).fold(f32::MAX, f32::min);
        eprintln!("  drop ({:+.3},{:+.3}) vel ({:+.2},{:+.2}) touching static {inside} gap to sword {near_seg:+.3}", d.pos.x, d.pos.y, d.vel.x, d.vel.y);
    }
    for &id in &ids {
        let p = s.world.particles[id];
        eprintln!("  sword particle ({:+.3},{:+.3}) vel ({:+.2},{:+.2})", p.pos.x, p.pos.y, p.vel.x, p.vel.y);
    }
}

#[test]
#[ignore]
fn measure_sword_rest_in_water() {
    for def in [0u8, 1, 2, KNIFE] {
        let (mut s, h) = still_pool();
        s.spawn_weapon(def, Vec2::new(h.a.x, h.a.y + h.size.y + 1.0));
        let wi = s.weapons.len() - 1;
        let ids: Vec<usize> = (0..s.weapons[wi].n).map(|k| s.weapons[wi].particle(k) as usize).collect();
        let (mut a_lo, mut a_hi, mut x_lo, mut x_hi, mut y_lo, mut y_hi) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        let mut reached = None;
        for k in 1..=900u32 {
            s.step(&[0]);
            let low = ids.iter().map(|&i| s.world.particles[i].pos.y - s.world.particles[i].radius).fold(f32::MAX, f32::min);
            if reached.is_none() && low < h.a.y - h.size.y + 0.05 {
                reached = Some(k);
            }
            if k > 600 {
                let (a, b) = (s.world.particles[ids[0]].pos, s.world.particles[ids[ids.len() - 1]].pos);
                let ang = dmath::angle(b - a);
                let m = (a + b) * 0.5;
                a_lo = a_lo.min(ang);
                a_hi = a_hi.max(ang);
                x_lo = x_lo.min(m.x);
                x_hi = x_hi.max(m.x);
                y_lo = y_lo.min(m.y);
                y_hi = y_hi.max(m.y);
            }
        }
        let low = ids.iter().map(|&i| s.world.particles[i].pos.y - s.world.particles[i].radius).fold(f32::MAX, f32::min);
        eprintln!("{:12}: bottom {:.2}, lowest point {low:.3}, reached the bottom at {:?} s; last 5 s: angle range {:.4} rad, mid moved x {:.4} y {:.4} m", s.weapons[wi].def().name, h.a.y - h.size.y, reached.map(|k| k as f32 / 60.0), a_hi - a_lo, x_hi - x_lo, y_hi - y_lo);
    }
}

#[test]
#[ignore]
fn trace_weapon_rest_in_water() {
    for def in [2u8, KNIFE] {
        let (mut s, h) = still_pool();
        s.spawn_weapon(def, Vec2::new(h.a.x, h.a.y + h.size.y + 1.0));
        let wi = s.weapons.len() - 1;
        let ids: Vec<usize> = (0..s.weapons[wi].n).map(|k| s.weapons[wi].particle(k) as usize).collect();
        eprintln!("{} ({} particles, mass {:.2} kg)", s.weapons[wi].def().name, ids.len(), ids.iter().map(|&i| 1.0 / s.world.particles[i].inv_mass).sum::<f32>());
        let segs: Vec<usize> = (0..s.weapons[wi].nseg as usize).map(|k| s.weapons[wi].seg_base as usize + k).collect();
        for &sg in &segs {
            let g = s.world.segments[sg];
            eprintln!("  seg {}-{} radius {:.3} (particle radii {:.3} {:.3})", g.a, g.b, g.radius, s.world.particles[g.a as usize].radius, s.world.particles[g.b as usize].radius);
        }
        for k in 1..=900u32 {
            s.step(&[0]);
            if k >= 480 && k % 15 == 0 {
                let (a, b) = (s.world.particles[ids[0]].pos, s.world.particles[ids[ids.len() - 1]].pos);
                let low = ids.iter().map(|&i| s.world.particles[i].pos.y - s.world.particles[i].radius).fold(f32::MAX, f32::min);
                let under = s.fluid.drops.iter().filter(|d| d.pos.x > a.x.min(b.x) && d.pos.x < a.x.max(b.x) && d.pos.y < low).count();
                let v = s.fluid.sample((a + b) * 0.5);
                eprintln!("  {:5.2}s ends ({:+.3},{:+.3}) ({:+.3},{:+.3}) ang {:+.3} low {low:+.3} drops under {under} liquid {:.2} vel ({:+.3},{:+.3})", k as f32 / 60.0, a.x, a.y, b.x, b.y, dmath::angle(b - a), v.total(), v.vel.x, v.vel.y);
                if k == 900 {
                    for d in s.fluid.drops.iter().filter(|d| d.pos.x > a.x.min(b.x) && d.pos.x < a.x.max(b.x) && d.pos.y < low) {
                        let gap = segs.iter().map(|&sg| { let g = s.world.segments[sg]; stickfell_physics::collide::point_segment(d.pos, s.world.particles[g.a as usize].pos, s.world.particles[g.b as usize].pos).0 - g.radius }).fold(f32::MAX, f32::min);
                        eprintln!("    drop ({:+.3},{:+.3}) gap to spear {gap:+.3} vel ({:+.2},{:+.2})", d.pos.x, d.pos.y, d.vel.x, d.vel.y);
                    }
                }
            }
        }
    }
}

#[test]
#[ignore]
fn measure_body_holes() {
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    let ids: Vec<u32> = s.fighters[0].particle_ids().collect();
    let r = s.fluid.radius;
    for g in s.world.segments.iter().filter(|g| ids.contains(&g.a) && ids.contains(&g.b)) {
        let len = (s.world.particles[g.a as usize].pos - s.world.particles[g.b as usize].pos).length();
        eprintln!("segment {}-{}: radius {:.3}, length {len:.3}; drops kept {:.3} from its axis", g.a, g.b, g.radius, g.radius + r * 0.4);
    }
    // the widest gap in the liquid across the body (rows through it, 1 cm steps)
    let pelvis = s.fighters[0].pos(&s.world, PELVIS);
    let mut widest: f32 = 0.0;
    for row in -60..=60 {
        let y = pelvis.y + row as f32 * 0.01;
        let mut xs: Vec<f32> = s.fluid.drops.iter().filter(|d| (d.pos.y - y).abs() < r).map(|d| d.pos.x).collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        for w in xs.windows(2) {
            if w[0] < pelvis.x + 0.6 && w[1] > pelvis.x - 0.6 {
                widest = widest.max(w[1] - w[0]);
            }
        }
    }
    eprintln!("widest gap between drop centres across the body (rows within {r:.2} m): {widest:.3} m; drop spacing {:.3}", s.fluid.spacing);
}

#[test]
#[ignore]
fn measure_swim_depth_hold() {
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    // under a little first, so the surface doesn't hold them up
    for _ in 0..20 {
        s.step(&[input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0]);
    }
    let mut ys = Vec::new();
    for k in 0..120 {
        s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
        if k >= 30 {
            ys.push(s.fighters[0].pos(&s.world, PELVIS).y);
        }
    }
    let drift = (ys[ys.len() - 1] - ys[0]) / (ys.len() as f32 / 60.0);
    let y0 = s.fighters[0].pos(&s.world, PELVIS).y;
    for _ in 0..60 {
        s.step(&[input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0]);
    }
    let y1 = s.fighters[0].pos(&s.world, PELVIS).y;
    for _ in 0..60 {
        s.step(&[input::encode(JUMP, Vec2::new(0.6, 0.0)), 0]);
    }
    let y2 = s.fighters[0].pos(&s.world, PELVIS).y;
    eprintln!("lift {SWIM_BUOYANCY}: swimming across drifts {drift:+.3} m/s (from y {:.2}); dive {:.2} m in 1 s, up {:.2} m in 1 s", ys[0], y0 - y1, y2 - y1);
}

#[test]
fn they_scream_burning_in_lava_and_acid_but_not_in_water() {
    use crate::level::HazardKind;
    for (kind, burns) in [(HazardKind::Lava, true), (HazardKind::Acid, true), (HazardKind::Water, false)] {
        let mut s = if kind == HazardKind::Acid {
            // (the yard has no acid: a tank of it, them dropped in)
            let mut s = wading(1.6);
            teleport(&mut s, 1, Vec2::new(20.0, 0.0));
            s.fluid.drops.clear();
            let a = Vec2::new(0.0, 0.8);
            s.fluid.fill(a - Vec2::new(6.0, 0.8), a + Vec2::new(6.0, 0.8), crate::liquid::ACID_DROP);
            for _ in 0..30 {
                s.step(&[0, 0]);
            }
            lay_flat(&mut s, 0, Vec2::new(0.0, 2.1));
            s
        } else {
            dropped_in(kind, 0.5).0
        };
        let (mut screams, mut first, mut burning_frames, mut alive_frames) = (0, None, 0, 0);
        for k in 0..300u32 {
            s.fighters[0].down = 200;
            let inputs = vec![0; s.fighters.len()];
            s.step(&inputs);
            let n = s.events.iter().filter(|e| e.kind == Sfx::Burning && e.speaker == 0).count();
            screams += n;
            if n > 0 && first.is_none() {
                first = Some(k);
            }
            if s.fighters[0].alive {
                alive_frames += 1;
                burning_frames += s.fighters[0].burning as u32;
            } else {
                assert!(!s.fighters[0].burning, "the dead don't scream");
            }
        }
        eprintln!("{kind:?}: {screams} screams, first on frame {first:?}, burning {burning_frames} of {alive_frames} frames alive");
        if burns {
            assert!(screams > 0 && burning_frames > 0, "{kind:?}: screaming in it");
        } else {
            assert_eq!(screams, 0, "water doesn't burn");
        }
    }
}

#[test]
#[ignore]
fn measure_climbing_out_of_water() {
    let (_, h) = still_pool();
    let edge = h.a.x + h.size.x;
    let ground = s_ground(&h);
    eprintln!("pool edge x {edge:.2}, water top {:.2}, ground beside it {ground:.2}", h.a.y + h.size.y);
    let plans: [(&str, fn(u32) -> u16); 6] = [
        ("hold right", |_| RIGHT),
        ("hold right, jump once (0.5 s in)", |k| RIGHT | if k == 30 { JUMP } else { 0 }),
        ("hold right, jump at once", |k| RIGHT | if k == 0 { JUMP } else { 0 }),
        ("hold right, jump every 20 frames", |k| RIGHT | if k % 20 == 0 { JUMP } else { 0 }),
        ("hold right + jump", |_| RIGHT | JUMP),
        ("hold right + jump for 1 s, let go", |k| if k < 60 { RIGHT | JUMP } else { 0 }),
    ];
    for (name, plan) in plans {
        let (mut s, _) = swimmer_by_the_edge(1.0);
        let f = &s.fighters[0];
        let start = (f.pos(&s.world, PELVIS), f.swimming);
        let mut out = None;
        let mut best_y = f32::MIN;
        for k in 0..600u32 {
            s.step(&[input::encode(plan(k), Vec2::new(0.6, 0.0))]);
            let f = &s.fighters[0];
            let p = f.pos(&s.world, PELVIS);
            let lowest_foot = f.pos(&s.world, fighter::FOOT_A).y.min(f.pos(&s.world, fighter::FOOT_B).y);
            best_y = best_y.max(lowest_foot);
            if out.is_none() && p.x > edge + 0.2 && lowest_foot > ground - 0.1 && f.grounded {
                out = Some(k);
            }
        }
        let f = &s.fighters[0];
        let landed = s.fighters[0].pos(&s.world, PELVIS).x - edge;
        eprintln!("{name:34}: {landed:+.2} m past the edge at the end; started pelvis ({:.2},{:.2}) swimming {}; out on frame {out:?}; highest the lower foot got {best_y:+.2}; ended pelvis {:?} grounded {} swimming {}", start.0.x, start.0.y, start.1, f.pos(&s.world, PELVIS), f.grounded, f.swimming);
    }
}

#[test]
#[ignore]
fn trace_climbing_out_of_water() {
    let (mut s, h) = swimmer_by_the_edge(1.0);
    let edge = h.a.x + h.size.x;
    for k in 0..280u32 {
        let jump = k % 20 == 0;
        s.step(&[input::encode(RIGHT | if jump { JUMP } else { 0 }, Vec2::new(0.6, 0.0))]);
        let f = &s.fighters[0];
        if jump || k % 20 == 6 || k % 20 == 12 {
            let (p, hd) = (f.pos(&s.world, PELVIS), f.pos(&s.world, HEAD));
            let head_liq = s.fluid.sample(hd).total();
            eprintln!("{k:3}{} pelvis ({:+.2},{:+.2}) head ({:+.2},{:+.2}) head liquid {head_liq:.2} swimming {} wade {:.2} stroke {} grounded {} body vel ({:+.2},{:+.2}) to edge {:+.2}", if jump { "J" } else { " " }, p.x, p.y, hd.x, hd.y, f.swimming, f.wade, f.stroke, f.grounded, f.body_vel(&s.world).x, f.body_vel(&s.world).y, edge - hd.x);
        }
    }
}

#[test]
#[ignore]
fn measure_yard_volume() {
    let (s, _) = still_pool();
    let trap = s.trap.def.unwrap();
    let cell = 0.05f32;
    let (lo, hi) = (s.min - Vec2::splat(1.0), s.max + Vec2::splat(1.0));
    let (w, h) = (((hi.x - lo.x) / cell) as usize, ((hi.y - lo.y) / cell) as usize);
    let solid = |x: usize, y: usize| {
        let p = lo + Vec2::new((x as f32 + 0.5) * cell, (y as f32 + 0.5) * cell);
        s.world.statics.iter().any(|o| o.collide_circle(p, 0.001).is_some())
    };
    let mut grid = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            grid[y * w + x] = solid(x, y);
        }
    }
    // the open space the water from the hatch can get to (falls, then spreads: everything connected below the hatch)
    let start = (((trap.hatch.x - lo.x) / cell) as usize, ((trap.hatch.y - 0.3 - lo.y) / cell) as usize);
    let mut seen = vec![false; w * h];
    let mut stack = vec![start];
    while let Some((x, y)) = stack.pop() {
        if x >= w || y >= h || seen[y * w + x] || grid[y * w + x] {
            continue;
        }
        seen[y * w + x] = true;
        stack.extend([(x + 1, y), (x.wrapping_sub(1), y), (x, y + 1), (x, y.wrapping_sub(1))]);
    }
    eprintln!("level bounds {:?} .. {:?}; hatch {:?}; drop spacing {} m", s.min, s.max, trap.hatch, crate::liquid::DROP_SPACING);
    let a = cell * cell;
    let per = crate::liquid::DROP_SPACING * crate::liquid::DROP_SPACING;
    let lowest = (0..h).find(|&y| (0..w).any(|x| seen[y * w + x])).unwrap();
    let mut total = 0.0;
    for y in lowest..h {
        let row = (0..w).filter(|&x| seen[y * w + x]).count() as f32 * a;
        total += row;
        let top = lo.y + (y + 1) as f32 * cell;
        if ((top * 20.0).round() as i32) % 20 == 0 {
            eprintln!("  up to y {top:+.1}: {total:.1} m^2 = {:.0} drops", total / per);
        }
    }
    eprintln!("all the open space the hatch reaches: {total:.1} m^2 = {:.0} drops (the yard's settled pool already {} drops)", total / per, s.fluid.drops.len());
}

#[test]
#[ignore]
fn measure_fluid_cost_by_drops() {
    use crate::level::{HazardDef, HazardKind};
    for depth in [1.0f32, 2.0, 4.0, 6.0] {
        let mut s = sandbox(&[(Vec2::new(-30.5, 0.0), Vec2::new(-30.0, 9.0)), (Vec2::new(30.0, 0.0), Vec2::new(30.5, 9.0))]);
        let a = Vec2::new(0.0, depth * 0.5);
        s.add_hazard(HazardDef { kind: HazardKind::Water, a, b: a, size: Vec2::new(30.0, depth * 0.5), speed: 0.0 });
        let inputs = vec![0; s.fighters.len()];
        for _ in 0..120 {
            s.step(&inputs);
        }
        let n = 120;
        let t = std::time::Instant::now();
        for _ in 0..n {
            s.step(&inputs);
        }
        let all = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let mut w = s.world.clone();
        let mut f = s.fluid.clone();
        let t = std::time::Instant::now();
        for _ in 0..n {
            f.step(DT, &mut w);
        }
        let fl = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("{} drops ({depth} m deep, 60 m wide): {all:.2} ms a game frame, {fl:.2} ms of it the liquid", s.fluid.drops.len());
    }
}

#[test]
#[ignore]
fn trace_haul_out() {
    let (mut s, h) = swimmer_by_the_edge(1.0);
    let edge = h.a.x + h.size.x;
    for k in 0..60u32 {
        let jump = k == 30;
        let before = s.ledge_ahead(0, 1.0);
        s.step(&[input::encode(RIGHT | if jump { JUMP } else { 0 }, Vec2::new(0.6, 0.0))]);
        let f = &s.fighters[0];
        if k % 5 == 0 || (28..40).contains(&k) {
            let (p, n) = (f.pos(&s.world, PELVIS), f.pos(&s.world, NECK));
            eprintln!("{k:3}{} pelvis ({:+.2},{:+.2}) neck ({:+.2},{:+.2}) swimming {} wade {:.2} grounded {} down {} stroke {} ledge {before:?} vel ({:+.2},{:+.2}) edge {edge}", if jump { "J" } else { " " }, p.x, p.y, n.x, n.y, f.swimming, f.wade, f.grounded, f.down, f.stroke, f.body_vel(&s.world).x, f.body_vel(&s.world).y);
        }
    }
}

#[test]
#[ignore]
fn trace_going_in_down() {
    let (mut s, h) = still_pool();
    let edge = h.a.x + h.size.x;
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(edge - 1.0, h.a.y + h.size.y - 0.3) - me);
    let mut last = 0;
    for k in 0..120u32 {
        let hp = s.fighters[0].health;
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0))]);
        let f = &s.fighters[0];
        if f.down > last || k % 15 == 0 {
            let d = f.pos(&s.world, NECK) - f.pos(&s.world, PELVIS);
            eprintln!("{k:3}: down {} (was {last}) grounded {} swimming {} wade {:.2} tilt {:.2} health {:.1} -> {:.1} pelvis {:?}", f.down, f.grounded, f.swimming, f.wade, dmath::angle_diff(dmath::angle(d), core::f32::consts::FRAC_PI_2), hp, f.health, f.pos(&s.world, PELVIS));
        }
        last = f.down;
    }
}

#[test]
#[ignore]
fn trace_haul_arc() {
    let (mut s, h) = swimmer_by_the_edge(1.0);
    let edge = h.a.x + h.size.x;
    let mut hauled = None;
    for k in 0..120u32 {
        let stroke = s.fighters[0].stroke;
        s.step(&[input::encode(RIGHT | JUMP, Vec2::new(0.6, 0.0))]);
        let f = &s.fighters[0];
        if hauled.is_none() && f.stroke == 18 && stroke == 0 {
            hauled = Some(k);
        }
        if hauled.is_some_and(|h| k >= h && k < h + 45) {
            let feet = f.pos(&s.world, fighter::FOOT_A).y.min(f.pos(&s.world, fighter::FOOT_B).y);
            let fx = f.pos(&s.world, fighter::FOOT_A).x.min(f.pos(&s.world, fighter::FOOT_B).x);
            eprintln!("{k:3} pelvis ({:+.2},{:+.2}) lowest foot y {feet:+.2} rearmost foot x {:+.2} (edge {edge}) vel ({:+.2},{:+.2}) grounded {} swimming {} wade {:.2}", f.pos(&s.world, PELVIS).x, f.pos(&s.world, PELVIS).y, fx, f.body_vel(&s.world).x, f.body_vel(&s.world).y, f.grounded, f.swimming, f.wade);
        }
    }
    eprintln!("hauled on frame {hauled:?}");
}

#[test]
fn swimmers_climb_out_at_the_side_holding_toward_it_and_jump() {
    for from in [1.0f32, 3.0] {
        let (mut s, h) = swimmer_by_the_edge(from);
        let edge = h.a.x + h.size.x;
        let mut out = None;
        for k in 0..180u32 {
            // (let go once they're out: holding right, they'd walk on)
            let b = if out.is_none() && k < 120 { RIGHT | JUMP } else { 0 };
            s.step(&[input::encode(b, Vec2::new(0.6, 0.0))]);
            let f = &s.fighters[0];
            if out.is_none() && f.grounded && f.pos(&s.world, PELVIS).x > edge + 0.1 {
                out = Some(k);
            }
        }
        let f = &s.fighters[0];
        let p = f.pos(&s.world, PELVIS);
        eprintln!("{from} m out: up on the side on frame {out:?}; 1.5 s later pelvis {p:?} grounded {} swimming {}", f.grounded, f.swimming);
        assert!(out.is_some_and(|k| (k as f32) < 60.0 * (0.5 + from / 2.8)), "{from} m out: out onto the side in time ({out:?})");
        assert!(f.grounded && !f.swimming && p.x > edge && p.x < edge + 1.5, "{from} m out: standing on the side, not flung across the yard ({p:?})");
    }
}

#[test]
#[ignore]
fn trace_after_haul() {
    let (mut s, h) = swimmer_by_the_edge(1.0);
    let edge = h.a.x + h.size.x;
    let mut out = None;
    for k in 0..300u32 {
        let b = if out.is_none() { RIGHT | JUMP } else { 0 };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.0))]);
        let f = &s.fighters[0];
        if out.is_none() && f.grounded && f.pos(&s.world, PELVIS).x > edge + 0.1 {
            out = Some(k);
        }
        if out.is_some() && k % 15 == 0 {
            let (fa, fb) = (f.pos(&s.world, fighter::FOOT_A), f.pos(&s.world, fighter::FOOT_B));
            eprintln!("{k:3}: pelvis {:?} feet {fa:?} {fb:?} grounded {} down {} getup {} swimming {} wade {:.2} edge {edge}", f.pos(&s.world, PELVIS), f.grounded, f.down, f.getup, f.swimming, f.wade);
        }
    }
}

#[test]
#[ignore]
fn measure_haul_landing() {
    for from in [0.6f32, 1.0, 2.0, 3.0] {
        let (mut s, h) = swimmer_by_the_edge(from);
        let edge = h.a.x + h.size.x;
        let mut out = None;
        let mut top_foot = f32::MIN;
        for k in 0..240u32 {
            let b = if out.is_none() { RIGHT | JUMP } else { 0 };
            s.step(&[input::encode(b, Vec2::new(0.6, 0.0))]);
            let f = &s.fighters[0];
            if f.stroke == 18 {
                top_foot = f32::MIN;
            }
            top_foot = top_foot.max(f.pos(&s.world, fighter::FOOT_A).y.min(f.pos(&s.world, fighter::FOOT_B).y));
            if out.is_none() && f.grounded && f.pos(&s.world, PELVIS).x > edge + 0.1 {
                out = Some(k);
            }
        }
        let f = &s.fighters[0];
        let feet = f.pos(&s.world, fighter::FOOT_A).y.min(f.pos(&s.world, fighter::FOOT_B).y);
        eprintln!("loss {HAUL_LOSS}: from {from} m: out frame {out:?}, lower foot peaked {top_foot:+.2} after the last haul, end pelvis ({:+.2},{:+.2}) lower foot {feet:+.2} (lip 0.00, edge {edge}) swimming {}", f.pos(&s.world, PELVIS).x, f.pos(&s.world, PELVIS).y, f.swimming);
    }
}

#[test]
#[ignore]
fn measure_dam() {
    for braced in [false, true] {
        let mut s = dam(braced);
        let total = s.fluid.drops.len();
        let start = s.fighters[0].pos(&s.world, PELVIS);
        let (mut downs, mut max_flow) = (0u32, 0.0f32);
        for k in 1..=300u32 {
            s.step(&[input::encode(0, Vec2::new(-0.6, 0.0)), 0]);
            let f = &s.fighters[0];
            downs += (f.down > 0) as u32;
            max_flow = max_flow.max(s.fluid.sample(f.pos(&s.world, PELVIS)).vel.length());
            if k % 60 == 0 {
                if s.round_frame < k {
                    eprintln!("  (round over at {k}: phase {:?}, alive {} {}, hp {:.0} {:.0})", s.phase, s.fighters[0].alive, s.fighters[1].alive, s.fighters[0].health, s.fighters[1].health);
                    break;
                }
                let p = f.pos(&s.world, PELVIS);
                let past = s.fluid.drops.iter().filter(|d| d.pos.x > p.x + 0.4).count();
                let top = |x0: f32, x1: f32| s.fluid.drops.iter().filter(|d| d.pos.x > x0 && d.pos.x < x1).map(|d| d.pos.y).fold(0.0f32, f32::max);
                eprintln!("porous {} grip {} braced {braced} (standing) {:.0} s: {:.0}% of the water past them; surface behind {:.2} m, in front {:.2} m; pelvis moved ({:+.2},{:+.2}); down {downs} frames; fastest flow at the pelvis {max_flow:.2} m/s", fighter::BODY_POROUS, std::env::var("GRIP").unwrap_or_default(), k as f32 / 60.0, 100.0 * past as f32 / total as f32, top(-6.0, p.x - 0.4), top(p.x + 0.4, 6.0), p.x - start.x, p.y - start.y);
            }
        }
    }
}

#[test]
#[ignore]
fn measure_surge() {
    // how hard moving water pushes (`Fighter::surge`) in the places it can:
    // per case, the peak and the 90th percentile over the frames it's wet
    let report = |name: &str, v: &mut Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f32| v.get(((v.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
        eprintln!("{name:46}: peak {:.2}, 90% {:.2}, median {:.2} ({} frames)", at(1.0), at(0.9), at(0.5), v.len());
    };
    for braced in [false, true] {
        let mut s = dam(braced);
        let mut v = Vec::new();
        for _ in 0..180 {
            s.step(&[input::encode(LEFT, Vec2::new(-0.6, 0.0)), 0]);
            v.push(s.fighters[0].surge);
        }
        report(&format!("dam break 2 m (braced {braced})"), &mut v);
    }
    // walking back and forth in still water, waist deep and chest deep
    for depth in [0.9f32, 1.3] {
        let mut s = wading(depth);
        let mut v = Vec::new();
        for k in 0..360u32 {
            let b = if (k / 90) % 2 == 0 { RIGHT } else { LEFT };
            s.step(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]);
            if k > 30 && s.fighters[0].wade > 0.05 {
                v.push(s.fighters[0].surge);
            }
        }
        report(&format!("walking in still water {depth} m deep"), &mut v);
    }
    // swimming across a pool
    let mut s = wading(2.5);
    let mut v = Vec::new();
    for k in 0..240u32 {
        let b = if (k / 60) % 2 == 0 { RIGHT } else { LEFT };
        s.step(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]);
        if k > 60 {
            v.push(s.fighters[0].surge);
        }
    }
    report("swimming back and forth", &mut v);
    // standing under the hatch as it pours, and out in the yard as it floods
    let mut s = at_the_lever();
    let def = s.trap.def.unwrap();
    throw_lever(&mut s);
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(def.hatch.x - me.x, 0.6 - me.y));
    let mut v = Vec::new();
    for _ in 0..300 {
        s.step(&[0, 0, 0, 0]);
        if s.fighters[0].wade > 0.05 {
            v.push(s.fighters[0].surge);
        }
    }
    report("under the hatch while it pours", &mut v);
    let me = s.fighters[0].pos(&s.world, PELVIS);
    teleport(&mut s, 0, Vec2::new(def.hatch.x + 8.0 - me.x, 0.6 - me.y));
    let mut v = Vec::new();
    for _ in 0..600 {
        s.step(&[0, 0, 0, 0]);
        if s.fighters[0].wade > 0.05 {
            v.push(s.fighters[0].surge);
        }
    }
    report("8 m from the hatch as the yard floods", &mut v);
}

#[test]
fn a_wall_of_water_sweeps_fighters_off_their_feet_but_wading_doesnt() {
    // 2 m of water let go against someone standing in its way (braced
    // against a step, too): they're knocked down and carried, and the water
    // gets past them
    for braced in [false, true] {
        let mut s = dam(braced);
        let x0 = s.fighters[0].pos(&s.world, PELVIS).x;
        let mut down_at = None;
        for k in 0..120u32 {
            s.step(&[0, 0]);
            if down_at.is_none() && s.fighters[0].down > 0 {
                down_at = Some(k);
            }
        }
        let x = s.fighters[0].pos(&s.world, PELVIS).x;
        let ahead = s.fluid.drops.iter().filter(|d| d.pos.x > x0 + 0.5).count() as f32 / s.fluid.drops.len() as f32;
        eprintln!("braced {braced}: knocked down on frame {down_at:?}, carried {:.2} m; after 2 s {:.0}% of the water past where they stood", x - x0, ahead * 100.0);
        assert!(down_at.is_some_and(|k| k < 60), "braced {braced}: swept off their feet");
        assert!(x - x0 > 1.0, "braced {braced}: carried along ({:.2} m)", x - x0);
        assert!(ahead > 0.3, "braced {braced}: the water isn't held back ({:.0}%)", ahead * 100.0);
    }
    // walking about in still water, and swimming, never knocks anyone down
    for (depth, what) in [(0.9f32, "wading waist deep"), (1.3, "wading chest deep"), (2.5, "swimming")] {
        let mut s = wading(depth);
        let mut most: f32 = 0.0;
        // (back and forth well clear of the tank's ends)
        let mut downs = 0;
        for k in 0..360u32 {
            let b = if (k / 45) % 2 == 0 { RIGHT } else { LEFT };
            s.step(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]);
            most = most.max(s.fighters[0].surge);
            downs += (s.fighters[0].down > 0) as u32;
        }
        eprintln!("{what}: the water shoved them at most {most:.2} of their weight; down {downs} frames; ended at x {:.2}", s.fighters[0].pos(&s.world, PELVIS).x);
        assert_eq!(downs, 0, "{what}: never knocked down");
    }
}

#[test]
#[ignore]
fn measure_wading_parts() {
    for depth in [0.9f32, 1.3] {
    let mut s = wading(depth);
    for _ in 0..90 {
        s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), 0]);
    }
    let f = &s.fighters[0];
    let names = ["head", "neck", "pelvis", "elbow a", "hand a", "elbow b", "hand b", "knee a", "foot a", "knee b", "foot b"];
    let mut line = format!("porous {}: ", fighter::BODY_POROUS);
    for (k, id) in f.particle_ids().enumerate() {
        let p = s.world.particles[id as usize];
        line += &format!("{} y {:.2} liquid {:.2}; ", names.get(k).unwrap_or(&"?"), p.pos.y, s.fluid.sample(p.pos).total());
    }
    eprintln!("{line}");
    let mut tilts = Vec::new();
    for k in 0..240u32 {
        let b = if (k / 45) % 2 == 0 { RIGHT } else { LEFT };
        let was = s.fighters[0].down;
        s.step(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]);
        let f = &s.fighters[0];
        if f.down > was + 1 {
            eprintln!("  frame {k}: knocked down for {} (surge {:.2}, swimming {}, grounded {}, wade {:.2})", f.down, f.surge, f.swimming, f.grounded, f.wade);
        }
        if k % 15 == 0 {
            let d = f.pos(&s.world, NECK) - f.pos(&s.world, PELVIS);
            tilts.push(format!("{k}:{:+.2}{}{}", dmath::angle_diff(dmath::angle(d), core::f32::consts::FRAC_PI_2), if f.down > 0 { format!("D{}", f.down) } else { String::new() }, if f.swimming { "S" } else { "" }));
        }
    }
    let speed = s.fighters[0].body_vel(&s.world).x.abs();
    eprintln!("  {depth} m: walking, tilt: {} (speed at the end {speed:.2} m/s)", tilts.join(" "));
    }
}

#[test]
#[ignore]
fn measure_flood_on_lava() {
    use crate::liquid::{COOLED_LAVA, LAVA_DROP, WATER_DROP};
    let mut s = at_the_lever();
    let count = |s: &GameState, k: u8| s.fluid.drops.iter().filter(|d| d.kind == k).count();
    let lava = |s: &GameState| {
        let l: Vec<f32> = s.fluid.drops.iter().filter(|d| d.kind == LAVA_DROP).map(|d| d.dye).collect();
        (l.len(), l.iter().filter(|&&d| d >= COOLED_LAVA).count(), l.iter().filter(|&&d| d > 0.0 && d < COOLED_LAVA).count())
    };
    let (w0, l0) = (count(&s, WATER_DROP), count(&s, LAVA_DROP));
    let (lp0, lp1, ly0, ly1) = crate::level::YARD_LAVA;
    eprintln!("before: {w0} water drops, {l0} lava drops (pit {:.2} m wide, {:.2} m deep = {:.2} m^2; a drop stands for {:.4} m^2)", lp1 - lp0, ly1 - ly0, (lp1 - lp0) * (ly1 - ly0), crate::liquid::DROP_SPACING * crate::liquid::DROP_SPACING);
    throw_lever(&mut s);
    let mut poured = 0u32;
    let mut last = s.fluid.drops.len();
    for k in 1..=60 * 90u32 {
        s.step(&[0, 0, 0, 0]);
        let now = s.fluid.drops.len();
        poured += (now as i64 - last as i64).max(0) as u32;
        last = now;
        if k % (60 * 10) == 0 {
            let (n, rock, warm) = lava(&s);
            eprintln!("{:3} s: water {} drops, lava {n} ({rock} rock, {warm} part-cooled, {} hot); level holds {}", k / 60, count(&s, WATER_DROP), n - rock - warm, s.fluid.drops.len());
        }
    }
    let w1 = count(&s, WATER_DROP);
    eprintln!("water boiled off over the 90 s: {} drops (poured in about {poured} net-of-boiling growth)", (w0 as i64 + s.trap.poured as i64 - w1 as i64));
    eprintln!("hatch poured {} drops since opened", s.trap.poured);
}

#[test]
#[ignore]
fn measure_steam_flood() {
    // the yard flooded onto its lava pit: the steam, the bursts, and what
    // they do to someone up by the lever and to the dummies down in it
    use crate::liquid::{LAVA_DROP, WATER_DROP};
    let mut s = at_the_lever();
    throw_lever(&mut s);
    let hp0: Vec<f32> = s.fighters.iter().map(|f| f.health).collect();
    let (mut bursts, mut peak_cells, mut peak_thick, mut boiled) = (0, 0usize, 0.0f32, 0i64);
    let mut worst_at_lever = 0.0f32;
    let (mut peak_seen, mut sum_seen) = (0usize, 0usize);
    let water0 = s.fluid.drops.iter().filter(|d| d.kind == WATER_DROP).count() as i64;
    for k in 1..=60 * 90u32 {
        let cd = s.burst_cd;
        s.step(&[0, 0, 0, 0]);
        if s.burst_cd > cd {
            bursts += 1;
        }
        let cells: Vec<f32> = s.steam.clouds(0.3).map(|c| c.1).collect();
        peak_cells = peak_cells.max(cells.len());
        // (visible: at least 5% opaque, 1 - e^-1.3t >= 0.05)
        let seen = s.steam.clouds(0.0395).count();
        peak_seen = peak_seen.max(seen);
        sum_seen += seen;
        peak_thick = peak_thick.max(cells.iter().copied().fold(0.0, f32::max));
        let head = s.fighters[0].pos(&s.world, HEAD);
        worst_at_lever = worst_at_lever.max(s.steam.at(head));
        if k % (60 * 15) == 0 {
            let lo = s.steam.clouds(0.03).map(|c| c.0.y).fold(f32::MAX, f32::min);
            let water = s.fluid.drops.iter().filter(|d| d.kind == WATER_DROP).count() as i64;
            boiled = water0 + s.trap.poured as i64 - water;
            let hot = s.fluid.drops.iter().filter(|d| d.kind == LAVA_DROP && d.dye < crate::liquid::COOLED_LAVA).count();
            eprintln!("{:3} s: steam {} cells thick enough to scald (of {} with any; lowest at y {lo:.1}); {bursts} bursts so far; {boiled} drops boiled; {hot} hot lava drops; hp {:?}", k / 60, cells.len(), s.steam.clouds(0.03).count(), s.fighters.iter().map(|f| f.health.round()).collect::<Vec<_>>());
        }
    }
    eprintln!("visible steam (>= 5% opaque): peak {peak_seen} m^2, mean {:.0} m^2 over the 90 s (per drop {}, condense {} s)", sum_seen as f32 / (60.0 * 90.0), crate::steam::STEAM_PER_DROP, crate::steam::CONDENSE);
    eprintln!("peak: {peak_cells} cells (m^2) scalding, thickest {peak_thick:.2}; at the lever (head) thickest {worst_at_lever:.2}; {bursts} bursts; {boiled} drops boiled of a real-heat budget {:.0}; hp lost {:?}", 312.0 * crate::liquid::LAVA_BOILS, hp0.iter().zip(&s.fighters).map(|(a, f)| (a - f.health).round()).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn measure_water_under_lava() {
    let mut s = at_the_lever();
    throw_lever(&mut s);
    let mut when = Vec::new();
    for k in 0..60 * 60u32 {
        let n = s.hot_contacts.len();
        s.step(&[0, 0, 0, 0]);
        if s.hot_contacts.len() > n {
            when.push(k);
        }
    }
    let c = &s.hot_contacts;
    let mut under: Vec<f32> = c.iter().map(|x| x.0).collect();
    let mut into: Vec<f32> = c.iter().map(|x| x.1).collect();
    under.sort_by(|a, b| a.total_cmp(b));
    into.sort_by(|a, b| a.total_cmp(b));
    let q = |v: &Vec<f32>, f: f32| v[((v.len() as f32 - 1.0) * f) as usize];
    eprintln!("{} water drops met hot lava in 60 s, over {} frames (first {:?}, last {:?})", c.len(), when.len(), when.first(), when.last());
    eprintln!("  how far under the lava drop (m): max {:.3}, 99% {:.3}, 90% {:.3}, median {:.3}", q(&under, 1.0), q(&under, 0.99), q(&under, 0.9), q(&under, 0.5));
    eprintln!("  how fast into it (m/s): max {:.2}, 99% {:.2}, 90% {:.2}, median {:.2}", q(&into, 1.0), q(&into, 0.99), q(&into, 0.9), q(&into, 0.5));
    for v in [2.0f32, 3.0, 4.0, 5.0] {
        let hits: Vec<u32> = c.iter().zip(0..).filter(|(x, _)| x.1 > v).map(|(_, i)| i).collect();
        eprintln!("  faster than {v} m/s: {} contacts", hits.len());
    }
}

#[test]
#[ignore]
fn measure_bursts_falling_into_flooded_lava() {
    let mut s = at_the_lever();
    throw_lever(&mut s);
    for _ in 0..60 * 40 {
        s.step(&[0, 0, 0, 0]);
    }
    let (lx0, lx1, _, ly1) = crate::level::YARD_LAVA;
    let surface = s.fluid.drops.iter().filter(|d| d.pos.x > lx0 && d.pos.x < lx1).map(|d| d.pos.y).fold(f32::MIN, f32::max);
    eprintln!("flooded: water over the pit up to y {surface:.2} (pit top {ly1:.2})");
    for (who, height) in [(1usize, 4.0f32), (2, 8.0)] {
        lay_flat(&mut s, who, Vec2::new((lx0 + lx1) * 0.5, surface + height));
        let hp = s.fighters[who].health;
        let mut bursts = Vec::new();
        for k in 0..60 * 8u32 {
            let cd = s.burst_cd;
            s.step(&[0, 0, 0, 0]);
            if s.burst_cd > cd {
                bursts.push(k);
            }
        }
        let f = &s.fighters[who];
        eprintln!("fighter {who} dropped from {height} m over the water: {} bursts at frames {bursts:?}; hp {hp:.0} -> {:.0} (alive {})", bursts.len(), f.health, f.alive);
    }
}

#[test]
fn steam_scalds_slowly_and_bursts_hurt_but_dont_kill() {
    // standing with their head in steam, thick and thin, for 5 s
    let rates: Vec<(f32, f32)> = [0.2f32, 0.6, 1.0, 3.0]
        .into_iter()
        .map(|thick| {
            let mut s = GameState::new(4, crate::level::TRAINING, 7);
            for _ in 0..30 {
                s.step(&[0, 0, 0, 0]);
            }
            let hp = s.fighters[0].health;
            for _ in 0..300 {
                // (kept topped up round their head)
                let head = s.fighters[0].pos(&s.world, HEAD);
                for dy in [-1.0f32, 0.0, 1.0] {
                    let p = head + Vec2::new(0.0, dy);
                    let now = s.steam.at(p);
                    s.steam.add(p, (thick - now).max(0.0));
                }
                s.step(&[0, 0, 0, 0]);
            }
            (thick, (hp - s.fighters[0].health) / 5.0)
        })
        .collect();
    for (thick, rate) in &rates {
        eprintln!("steam {thick} thick round the head: {rate:.2} health a second");
    }
    assert_eq!(rates[0].1, 0.0, "thin steam is harmless");
    assert!(rates.iter().all(|r| r.1 <= SCALD_RATE + 0.05), "never faster than {SCALD_RATE} a second (time to get out)");
    assert!(rates[3].1 > 2.0, "thick steam does hurt");
    // a burst going off right by them
    let mut s = GameState::new(4, crate::level::TRAINING, 7);
    for _ in 0..30 {
        s.step(&[0, 0, 0, 0]);
    }
    let (hp, at) = (s.fighters[0].health, s.fighters[0].pos(&s.world, PELVIS));
    let v0 = s.fighters[0].body_vel(&s.world);
    s.steam_burst(at + Vec2::new(-0.4, -0.3));
    let thrown = s.fighters[0].body_vel(&s.world) - v0;
    for _ in 0..60 {
        s.step(&[0, 0, 0, 0]);
    }
    let lost = hp - s.fighters[0].health;
    eprintln!("a burst 0.5 m off: {lost:.1} health, thrown at {:.2} m/s, down {}", thrown.length(), s.fighters[0].down);
    assert!(lost > 5.0 && lost <= BURST_DAMAGE + 0.01, "it hurts, a bit ({lost:.1})");
    assert!(s.fighters[0].alive && thrown.length() > 2.0, "and throws them, not kills them");
}

#[test]
#[ignore]
fn screenshot_steam() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = at_the_lever();
    throw_lever(&mut s);
    let (lx0, lx1, _, ly1) = crate::level::YARD_LAVA;
    let mut shots = 0;
    for k in 0..60 * 30u32 {
        let cd = s.burst_cd;
        s.step(&[0, 0, 0, 0]);
        let burst = s.burst_cd > cd;
        if (burst && shots < 2) || k == 60 * 12 || k == 60 * 25 {
            let mut c = Wide { game: s.clone(), at: Vec2::new((lx0 + lx1) * 0.5 - 3.0, ly1 + 5.0), height: 16.0 };
            let name = if burst { format!("steam_burst_{shots}") } else { format!("steam_{}s", k / 60) };
            stickfell_app::capture_headless(&mut c, 0, (800, 520), format!("{dir}/{name}.png").as_ref()).unwrap();
            if burst {
                shots += 1;
            }
        }
    }
}

#[test]
#[ignore]
fn measure_swim_legs() {
    // where the legs point while swimming across and diving: how far off
    // straight back along the body, how bent the knees are, how big the kick
    for (what, b) in [("across", RIGHT), ("diving", CROUCH)] {
        let mut s = wading(4.0);
        teleport(&mut s, 1, Vec2::new(20.0, 0.0));
        for _ in 0..90 {
            s.step(&[0, 0]);
        }
        let (mut off, mut bend, mut swing) = (Vec::new(), Vec::new(), Vec::new());
        for k in 0..90u32 {
            s.step(&[input::encode(b, Vec2::new(0.6, 0.0)), 0]);
            if k < 30 {
                continue;
            }
            let f = &s.fighters[0];
            let (pelvis, neck) = (f.pos(&s.world, PELVIS), f.pos(&s.world, NECK));
            let back = dmath::angle(pelvis - neck);
            for (knee, foot) in [(fighter::KNEE_A, fighter::FOOT_A), (fighter::KNEE_B, fighter::FOOT_B)] {
                let (kp, fp) = (f.pos(&s.world, knee), f.pos(&s.world, foot));
                let thigh = dmath::angle(kp - pelvis);
                let shin = dmath::angle(fp - kp);
                off.push(dmath::angle_diff(dmath::angle(fp - pelvis), back).abs());
                bend.push(dmath::angle_diff(shin, thigh).abs());
                swing.push(dmath::angle_diff(thigh, back));
            }
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let range = swing.iter().copied().fold(f32::MIN, f32::max) - swing.iter().copied().fold(f32::MAX, f32::min);
        eprintln!("{what}: legs off straight back along the body {:.2} rad (mean), knees bent {:.2} rad (mean), thighs swing over {range:.2} rad", mean(&off), mean(&bend));
    }
}

#[test]
#[ignore]
fn trace_dive_legs() {
    let mut s = wading(4.0);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    for k in 0..90u32 {
        s.step(&[input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0]);
        if k % 6 != 0 {
            continue;
        }
        let f = &s.fighters[0];
        let (pelvis, neck) = (f.pos(&s.world, PELVIS), f.pos(&s.world, NECK));
        let back = dmath::angle(pelvis - neck);
        let (kp, fp) = (f.pos(&s.world, fighter::KNEE_A), f.pos(&s.world, fighter::FOOT_A));
        eprintln!("{k:2}: back {:+.2} thigh {:+.2} shin {:+.2} | pelvis y {:.2} swimming {} grounded {} wade {:.2} facing {} vel ({:+.2},{:+.2})",
            back, dmath::angle(kp - pelvis), dmath::angle(fp - kp), pelvis.y, f.swimming, f.grounded, f.wade, f.facing, f.body_vel(&s.world).x, f.body_vel(&s.world).y);
    }
}

#[test]
#[ignore]
fn measure_swimming_without_limbs() {
    use crate::fighter::{S_FORE_A, S_THIGH_A, S_UPPER_A, S_UPPER_B};
    let cases: [(&str, &[u32]); 6] = [
        ("all four limbs", &[]),
        ("a forearm gone", &[S_FORE_A]),
        ("an arm gone", &[S_UPPER_A]),
        ("a leg gone", &[S_THIGH_A]),
        ("an arm and a leg gone", &[S_UPPER_A, S_THIGH_A]),
        ("both arms gone", &[S_UPPER_A, S_UPPER_B]),
    ];
    for (what, parts) in cases {
        let mut s = wading(4.0);
        teleport(&mut s, 1, Vec2::new(20.0, 0.0));
        for &p in parts {
            s.sever(0, p);
        }
        s.fighters[0].health = 1000.0;
        for _ in 0..60 {
            s.step(&[0, 0]);
        }
        let (x0, y0) = { let p = s.fighters[0].pos(&s.world, PELVIS); (p.x, p.y) };
        for _ in 0..60 {
            s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
        }
        let p1 = s.fighters[0].pos(&s.world, PELVIS);
        // then let go: float, tread, or go down
        for _ in 0..180 {
            s.step(&[0, 0]);
        }
        let p2 = s.fighters[0].pos(&s.world, PELVIS);
        let f = &s.fighters[0];
        eprintln!("{what:22}: power {:.2}; swam {:.2} m in 1 s (depth change {:+.2} m); then 3 s idle: depth change {:+.2} m (pelvis y {:.2}, surface 4.0); swimming {}", f.swim_power(), p1.x - x0, p1.y - y0, p2.y - p1.y, p2.y, f.swimming);
    }
}

#[test]
fn losing_limbs_makes_swimming_hard_and_two_gone_they_sink() {
    use crate::fighter::{S_THIGH_A, S_UPPER_A};
    let swim = |parts: &[u32]| {
        let mut s = wading(4.0);
        teleport(&mut s, 1, Vec2::new(20.0, 0.0));
        for &p in parts {
            s.sever(0, p);
        }
        s.fighters[0].health = 1000.0;
        for _ in 0..60 {
            s.step(&[0, 0]);
        }
        let x0 = s.fighters[0].pos(&s.world, PELVIS).x;
        for _ in 0..60 {
            s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
        }
        let x1 = s.fighters[0].pos(&s.world, PELVIS).x;
        for _ in 0..180 {
            s.step(&[0, 0]);
        }
        (x1 - x0, s.fighters[0].pos(&s.world, PELVIS).y, s.fighters[0].swimming)
    };
    let (all, all_y, _) = swim(&[]);
    let (arm, _, arm_swims) = swim(&[S_UPPER_A]);
    let (_, both_y, both_swims) = swim(&[S_UPPER_A, S_THIGH_A]);
    eprintln!("swam in 1 s: all limbs {all:.2} m, an arm gone {arm:.2} m; afloat at y {all_y:.2}; an arm and a leg gone: ended at y {both_y:.2} (the tank's 4 m deep)");
    assert!(arm_swims && arm < all * 0.8, "one limb gone, slower ({arm:.2} vs {all:.2})");
    assert!(!both_swims && both_y < 1.5 && all_y > 3.0, "two gone, they can't swim and go down");
}

#[test]
fn acid_eats_parts_away_and_lava_melts_them_to_the_bone_and_sets_them_alight() {
    use crate::level::{HazardDef, HazardKind};
    // acid: lying in a vat, everything in it goes, a part at a time
    let mut s = sandbox(&[]);
    s.add_hazard(HazardDef { kind: HazardKind::Acid, a: Vec2::new(3.0, 1.0), b: Vec2::new(3.0, 1.0), size: Vec2::new(1.5, 1.0), speed: 0.0 });
    lay_flat(&mut s, 0, Vec2::new(3.0, 2.8));
    // (the round mustn't end and start over when they die)
    s.num_players = 1;
    let mut first_off = None;
    for k in 0..(60 * 12) {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if first_off.is_none() && (2..10).any(|p| !s.fighters[0].has(p)) {
            first_off = Some(k);
        }
        if k == 120 {
            eprintln!("acid 2 s: {}", rot_report(&s));
        }
    }
    eprintln!("acid 12 s: {}", rot_report(&s));
    let loose = s.weapons.iter().filter(|w| w.limb.is_some()).count();
    let left = s.weapons.iter().filter(|w| w.limb.is_some() && w.active).count();
    eprintln!("acid: first limb off after {first_off:?} frames; loose limbs {loose}, still there {left}; head gone {}", s.fighters[0].head_popped);
    let f = &s.fighters[0];
    assert!(!f.alive);
    assert!(first_off.is_some_and(|k| k > 120), "slowly, not at once");
    assert!((2..10).all(|p| !f.has(p)), "every arm and leg eaten through");
    assert!(f.head_popped, "and the head");
    assert!(loose > 0 && left < loose, "and what came off dissolves where it lies");
    // lava: melted down to the bone, then on fire after being pulled out
    let (mut s, _) = dropped_in(HazardKind::Lava, 1.0);
    for _ in 0..90 {
        s.step(&[0]);
    }
    eprintln!("lava 1.5 s: {}", rot_report(&s));
    let f = &s.fighters[0];
    assert!(f.melted.iter().filter(|&&m| m > fighter::ROT_FULL / 2).count() >= 3, "parts melted past halfway");
    assert!(f.fire.iter().any(|&t| t > 0), "on fire");
}

#[test]
#[ignore]
fn screenshot_rot() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let full = fighter::ROT_FULL;
    // two bodies side by side: half eaten by acid / melted to the bone and
    // burning; then half melted with a burning arm / three quarters melted
    for (name, a_eat, a_melt, b_melt, b_fire) in [("rot_acid_bone", true, 0u16, full, true), ("rot_half_melted", false, full / 2, full / 4 * 3, false)] {
        let mut s = sandbox(&[]);
        s.num_players = 1;
        warp(&mut s, 1, Vec2::new(1.4, 1.0));
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        if a_eat {
            s.fighters[0].eaten = [full / 2, full / 2, full / 3, full / 3 * 2, full / 4, full / 2, full / 2, full / 4 * 3, full / 3, full / 2];
        }
        s.fighters[0].melted = [a_melt; 10];
        if !a_eat {
            s.fighters[0].fire[fighter::S_FORE_A as usize] = 300;
            s.fighters[0].fire[fighter::S_UPPER_A as usize] = 300;
        }
        s.fighters[1].melted = [b_melt; 10];
        if b_fire {
            s.fighters[1].fire = [300; 10];
        }
        for _ in 0..6 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        eprintln!("{name}: fire {:?} / {:?}, melted {:?}", s.fighters[0].fire, s.fighters[1].fire, s.fighters[1].melted);
        let mut c = CloseUp { game: s.clone(), at: Vec2::new(0.7, 1.1) };
        stickfell_app::capture_headless(&mut c, 0, (900, 560), format!("{dir}/{name}.png").as_ref()).unwrap();
    }
    // the yard's lava: in it, then pulled out and still burning
    let (mut s, h) = dropped_in(crate::level::HazardKind::Lava, 1.0);
    for _ in 0..70 {
        s.step(&[0]);
    }
    let mut c = CloseUp { game: s.clone(), at: Vec2::new(h.a.x, h.a.y + h.size.y) };
    stickfell_app::capture_headless(&mut c, 0, (900, 560), format!("{dir}/rot_lava_in.png").as_ref()).unwrap();
    // the acid vat, a few seconds in
    let mut s = sandbox(&[]);
    s.num_players = 1;
    s.add_hazard(crate::level::HazardDef { kind: crate::level::HazardKind::Acid, a: Vec2::new(3.0, 1.0), b: Vec2::new(3.0, 1.0), size: Vec2::new(1.5, 1.0), speed: 0.0 });
    lay_flat(&mut s, 0, Vec2::new(3.0, 2.8));
    for k in 1..=480 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
        if k == 240 || k == 480 {
            let mut c = CloseUp { game: s.clone(), at: Vec2::new(3.0, 1.0) };
            stickfell_app::capture_headless(&mut c, 0, (900, 560), format!("{dir}/rot_acid_{k}.png").as_ref()).unwrap();
        }
    }
}

/// Dev tool: an ocean's cost. Seas of different sizes (a basin `w` m wide,
/// `depth` m of water): how many drops, and what a sim step costs settled.
#[test]
#[ignore]
fn measure_ocean_cost() {
    use crate::level::{HazardDef, HazardKind};
    for (w, depth) in [(40.0f32, 2.5f32), (48.0, 2.4), (50.0, 2.5), (56.0, 2.2)] {
        let mut s = sandbox(&[(Vec2::new(-w * 0.5 - 1.0, -depth), Vec2::new(-w * 0.5, 2.0)), (Vec2::new(w * 0.5, -depth), Vec2::new(w * 0.5 + 1.0, 2.0))]);
        // (the basin: the sandbox floor dug out)
        s.world.statics.retain(|o| o.collide_circle(Vec2::new(0.0, -1.0), 0.001).is_none());
        s.world.add_static(stickfell_physics::Obb::from_min_max(Vec2::new(-w * 0.5 - 1.0, -depth - 2.0), Vec2::new(w * 0.5 + 1.0, -depth)).with_friction(0.9));
        s.add_hazard(HazardDef { kind: HazardKind::Water, a: Vec2::new(0.0, -depth * 0.5), b: Vec2::new(0.0, -depth * 0.5), size: Vec2::new(w * 0.5, depth * 0.5), speed: 0.0 });
        let drops = s.fluid.drops.len();
        for _ in 0..120 {
            s.step(&[0, 0]);
        }
        let t = std::time::Instant::now();
        let n = 240;
        for _ in 0..n {
            s.step(&[0, 0]);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("sea {w} m x {depth} m ({} m^2): {drops} drops, {ms:.2} ms a step", w * depth);
    }
}

#[test]
fn open_water_has_rafts_that_float_and_carry_people_and_no_weapons() {
    let mut s = GameState::new(2, crate::level::SEA, 5);
    for _ in 0..180 {
        s.step(&[0, 0]);
    }
    assert!(s.weapons.iter().all(|w| !w.active), "bare hands only");
    let (_, _, _, surface) = crate::level::SEA_WATER;
    for (i, r) in s.rafts.iter().enumerate() {
        let top = r.y + r.def.half.y;
        eprintln!("raft {i} at x {}: top {top:.2} (water {surface}), tilt {:.3}", r.def.x, r.tilt);
        assert!((top - surface).abs() < 0.35, "raft {i} rides at the surface ({top:.2})");
    }
    for f in &s.fighters {
        let p = f.pos(&s.world, PELVIS);
        eprintln!("fighter {}: pelvis ({:.2},{:.2}) grounded {} swimming {}", f.id, p.x, p.y, f.grounded, f.swimming);
        assert!(f.grounded && !f.swimming && p.y > surface + 0.5, "standing on a raft");
    }
    // the one with someone on it sits lower than the empty ones
    let loaded = s.rafts.iter().find(|r| (r.def.x - s.fighters[0].pos(&s.world, PELVIS).x).abs() < 1.5).unwrap().y;
    let empty = s.rafts.iter().filter(|r| r.def.x.abs() > 10.0).map(|r| r.y).fold(f32::MIN, f32::max);
    eprintln!("loaded raft {loaded:.3} m, empty {empty:.3} m");
    assert!(loaded < empty - 0.02);
    // walk off the end into the sea: swimming
    for _ in 0..120 {
        s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
    }
    let f = &s.fighters[0];
    eprintln!("after walking off: pelvis {:?}, swimming {}", f.pos(&s.world, PELVIS), f.swimming);
    assert!(f.swimming, "in the water");
}

#[test]
#[ignore]
fn measure_open_water_cost() {
    let fighters: usize = std::env::var("SEA_FIGHTERS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let mut s = GameState::new_match(fighters, 2.min(fighters), 0, crate::level::SEA, 5);
    if std::env::var("SEA_NO_RAFTS").is_ok() {
        s.rafts.clear();
    }
    let drops = s.fluid.drops.len();
    for _ in 0..120 {
        s.step(&[0, 0]);
    }
    let (mut worst, mut sum) = (0.0f64, 0.0f64);
    let n = 60 * 20;
    for _ in 0..n {
        let t = std::time::Instant::now();
        s.step(&[0, 0]);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);
        sum += ms;
    }
    let swimming = s.fighters.iter().filter(|f| f.swimming).count();
    eprintln!("open water, {fighters} fighters: {drops} drops, {:.2} ms a step on average over 20 s, worst {worst:.2}; {swimming} swimming at the end, alive {}", sum / n as f64, s.fighters.iter().filter(|f| f.alive).count());
}

#[test]
#[ignore]
fn measure_water_level_reading() {
    let s = GameState::new(2, crate::level::SEA, 5);
    for x in [-24.0f32, -8.0, 0.0, 16.0] {
        let top = s.fluid.drops.iter().filter(|d| (d.pos.x - x).abs() < 0.3).map(|d| d.pos.y).fold(f32::MIN, f32::max);
        let read = s.water_level(x, (-2.2, 0.0));
        let amounts: Vec<String> = [-0.4f32, -0.2, -0.1, 0.0, 0.1, 0.2, 0.3].iter().map(|dy| format!("{:.2}", s.fluid.sample(Vec2::new(x, top + dy)).amount[0])).collect();
        eprintln!("x {x}: highest drop {top:.3}, reading {read:.3}; amounts from 0.4 under the top drop to 0.3 over: {}", amounts.join(" "));
    }
}

/// Rafts, AI fighters brawling in the sea, acid eating and lava burning
/// bodies (and breaking bones on the way), and the rope course: all the same
/// re-simulated from any of the last 8 frames.
#[test]
fn new_arenas_rot_and_fire_are_deterministic_under_rollback() {
    let mut st = stickfell_net::SyncTestSession::new(GameState::new_match(6, 1, 0, crate::level::SEA, 5), 8);
    for k in 0..240 {
        let b = if k > 30 && k < 120 { RIGHT } else if k % 20 == 0 { ATTACK } else { 0 };
        st.advance(&[input::encode(b, Vec2::new(0.6, 0.0))]).unwrap();
    }
    let mut st = stickfell_net::SyncTestSession::new(GameState::new_match(4, 1, 0, crate::level::ROPES, 3), 8);
    for _ in 0..200 {
        st.advance(&[0]).unwrap();
    }
    // a body dropped in the yard's lava: melting, then burning
    let (s, _) = dropped_in(crate::level::HazardKind::Lava, 1.0);
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for _ in 0..150 {
        st.advance(&[0, 0, 0, 0]).unwrap();
    }
    // and in acid, eaten through
    let mut s = sandbox(&[]);
    s.num_players = 1;
    s.add_hazard(crate::level::HazardDef { kind: crate::level::HazardKind::Acid, a: Vec2::new(3.0, 1.0), b: Vec2::new(3.0, 1.0), size: Vec2::new(1.5, 1.0), speed: 0.0 });
    lay_flat(&mut s, 0, Vec2::new(3.0, 2.8));
    for _ in 0..360 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let mut st = stickfell_net::SyncTestSession::new(s, 8);
    for _ in 0..120 {
        st.advance(&[0, 0]).unwrap();
    }
}
