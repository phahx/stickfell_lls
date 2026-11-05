//! Bare hands: punches, kicks, grabs, throws, dives and flips.

use super::*;

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
fn bots_brawl_bare_handed() {
    // no weapons around: bots have to use fists, feet and grabs
    let (mut grabs, mut throws, mut kicks, mut punches, mut escapes, mut shoulders) = (0, 0, 0, 0, 0, 0);
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
            let before: Vec<(Option<u8>, u8, u8, u8)> = s.fighters.iter().map(|f| (f.hold, f.kick, f.flying, f.shoulder)).collect();
            s.step(&inputs);
            for (f, (hold, kick, flying, shoulder)) in s.fighters.iter().zip(before) {
                shoulders += (shoulder == 0 && f.shoulder > 0) as u32;
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
    eprintln!("bots (3 min, bare hands): {punches} punches, {kicks} kicks, {grabs} grabs, {throws} throws ({shoulders} over the shoulder), {escapes} escapes");
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
#[ignore]
fn trace_dive_forces() {
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    let ids: Vec<usize> = s.fighters[0].particle_ids().map(|i| i as usize).collect();
    let mass: f32 = ids.iter().map(|&i| { let w = s.world.particles[i].inv_mass; if w > 0.0 { 1.0 / w } else { 0.0 } }).sum();
    let mom = |s: &GameState| ids.iter().map(|&i| { let p = s.world.particles[i]; if p.inv_mass > 0.0 { p.vel.y / p.inv_mass } else { 0.0 } }).sum::<f32>();
    eprintln!("body mass {mass:.2} kg, weight {:.1} N", mass * 9.81);
    let (mut liq, mut wld, mut fl) = (0.0, 0.0, 0.0);
    for k in 0..60 {
        let inp = [input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0];
        let mut a = s.clone();
        let m0 = mom(&a);
        a.update_liquids(DT);
        let m1 = mom(&a);
        a.world.step(DT);
        let m2 = mom(&a);
        a.fluid.step(DT, &mut a.world);
        let m3 = mom(&a);
        s.step(&inp);
        if k >= 10 {
            liq += (m1 - m0) * 60.0 / 50.0;
            wld += (m2 - m1) * 60.0 / 50.0;
            fl += (m3 - m2) * 60.0 / 50.0;
        }
    }
    eprintln!("mean vertical force while diving: liquids pass {liq:+.1} N, world (gravity, joints) {wld:+.1} N, drops {fl:+.1} N; pelvis y {:.2}", s.fighters[0].pos(&s.world, PELVIS).y);
}

#[test]
#[ignore]
fn trace_dive() {
    let mut s = wading(2.5);
    teleport(&mut s, 1, Vec2::new(20.0, 0.0));
    for _ in 0..90 {
        s.step(&[0, 0]);
    }
    for _ in 0..60 {
        s.step(&[input::encode(RIGHT, Vec2::new(0.6, 0.0)), 0]);
    }
    for k in 0..60 {
        s.step(&[input::encode(CROUCH, Vec2::new(0.6, 0.0)), 0]);
        if k % 5 == 4 {
            let f = &s.fighters[0];
            let (p, h) = (f.pos(&s.world, PELVIS), f.pos(&s.world, HEAD));
            let d = f.pos(&s.world, NECK) - p;
            let b = f.body_vel(&s.world);
            eprintln!("  {k:2}: pelvis {:.2} head {:.2} torso ang {:+.2} body vel ({:+.2},{:+.2}) swimming {} wade {:.2} grounded {} down {}", p.y, h.y, dmath::angle(d), b.x, b.y, f.swimming, f.wade, f.grounded, f.down);
        }
    }
}
