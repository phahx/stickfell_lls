//! Finishers, nooses, pins, shoulder throws and curb stomps.

use super::*;

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
        // (the victim waves their mouse about all through it)
        let caught_facing = s.fighters[1].facing;
        let mut turned = 0;
        for k in 0..320 {
            let wave = Vec2::new(if k % 24 < 12 { 0.8 } else { -0.8 }, 0.2);
            s.step(&[input::encode(0, Vec2::new(0.6, 0.0)), input::encode(0, wave)]);
            turned += (!s.finishers.is_empty() && s.fighters[1].facing != caught_facing) as u32;
            if banner.is_none() {
                banner = s.finish_banner.map(|b| b.0);
            }
            slowmo |= s.slowmo > 0;
            if banner.is_some() && s.finishers.is_empty() {
                break;
            }
        }
        assert!(slowmo, "{mv:?}: the finishing blow should go to slow motion");
        assert_eq!(turned, 0, "{mv:?}: the victim's mouse turned them round");
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
fn facing_them_q_still_heaves_them_away() {
    let mut s = pair(0.9);
    grab_them(&mut s);
    step2(&mut s, GRAB | THROW, Vec2::new(0.6, 0.35), 0);
    assert_eq!(s.fighters[0].shoulder, 0, "with them in front it's the normal throw");
    assert!(s.fighters[0].heave > 0);
}

#[test]
fn shoulder_throws_are_deterministic_under_rollback() {
    let mut st = stickfell_net::SyncTestSession::new(pair(0.9), 8);
    let mut threw = false;
    for k in 0..150u32 {
        let b = shoulder_throw_input(&st.game, k);
        st.advance(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]).unwrap();
        threw |= st.game.fighters[0].shoulder > 0;
    }
    assert!(threw, "the shoulder throw should happen under rollback too");
}

#[test]
#[ignore]
fn screenshot_shoulder_throw() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = pair(0.9);
    let (mut n, mut after) = (0, 0);
    for k in 0..150u32 {
        let b = shoulder_throw_input(&s, k);
        s.step(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]);
        let shot = if s.fighters[0].shoulder > 0 {
            n += 1;
            [1, 8, 14, 20, 26].contains(&n)
        } else if n > 0 {
            after += 1;
            [3, 30].contains(&after)
        } else {
            false
        };
        if shot {
            let name = if after > 0 { format!("after_{after}") } else { format!("{n:02}") };
            let mut sp = crate::Spectate { game: s.clone(), follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
            stickfell_app::capture_headless(&mut sp, 0, (900, 560), format!("{dir}/shoulder_{name}.png").as_ref()).unwrap();
        }
    }
}

#[test]
#[ignore]
fn trace_shoulder_throw() {
    let mut s = pair(0.9);
    let mut t = 0;
    for k in 0..150u32 {
        let b = shoulder_throw_input(&s, k);
        s.step(&[b, input::encode(0, Vec2::new(-0.6, 0.0))]);
        if s.fighters[0].shoulder > 0 || t > 0 && t < 40 {
            t += 1;
            let v = &s.fighters[1];
            let low = v.particle_ids().map(|i| s.world.particles[i as usize].pos.y).fold(f32::MAX, f32::min);
            eprintln!("t{t} sh {} vel {:?} lowest {low:.2} pelvis y {:.2} flying {} hp {:.0}", s.fighters[0].shoulder, v.body_vel(&s.world), v.pos(&s.world, PELVIS).y, v.flying, v.health);
        }
    }
}

#[test]
fn a_finisher_right_after_a_shoulder_throw_is_a_curb_stomp() {
    let mut s = pair(0.9);
    shoulder_then_finish(&mut s, 10);
    assert_eq!(s.finishers.first().map(|x| x.mv), Some(Finish::Stomp), "a finisher within a second of the throw should be the stomp");
    let floor_pelvis = s.fighters[1].pos(&s.world, PELVIS).y;
    let (mut highest, mut squashed_alive, mut foot_gap) = (f32::MIN, false, [f32::MAX; 2]);
    for _ in 0..200 {
        s.step(&[input::encode(0, Vec2::new(-0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
        let v = &s.fighters[1];
        highest = highest.max(v.pos(&s.world, PELVIS).y - floor_pelvis);
        if let Some(x) = s.finishers.first() {
            for (i, &h) in crate::weapon::STOMP_HITS.iter().enumerate() {
                if x.t + 3 >= h && x.t <= h + 3 {
                    // on top of it: above, lined up, touching (head and heel radii apart)
                    let d = s.fighters[0].pos(&s.world, fighter::FOOT_A) - v.pos(&s.world, HEAD);
                    let touch = s.world.segments[(v.seg_base + fighter::S_HEAD) as usize].radius + s.world.particles[s.fighters[0].p(fighter::FOOT_A) as usize].radius;
                    if d.y > 0.0 && d.x.abs() < 0.15 {
                        foot_gap[i] = foot_gap[i].min(d.length() - touch);
                    }
                }
            }
            if x.t > crate::weapon::STOMP_HITS[0] && x.t < crate::weapon::STOMP_HITS[1] {
                squashed_alive |= v.alive && v.head_squash > 0.3 && !v.head_popped;
            }
        }
    }
    let (me, them) = (&s.fighters[0], &s.fighters[1]);
    let spatters = me.wounds.iter().filter(|w| w.kind == fighter::WoundKind::Spatter).count();
    eprintln!("curb stomp: victim rose at most {highest:.2} m, heel to the top of the head at the stomps {:.3} / {:.3} m, {spatters} spatters on the stomper", foot_gap[0], foot_gap[1]);
    assert!(highest < 0.15, "the victim stays on the ground ({highest:.2} m up)");
    assert!(foot_gap[0] < 0.05 && foot_gap[1] < 0.05, "the heel comes down on top of the head both times ({foot_gap:?})");
    assert!(squashed_alive, "the first stomp flattens the head and doesn't kill");
    assert!(!them.alive && them.head_popped, "the second stomp bursts the head");
    assert!(spatters >= 3, "the stomper's leg comes away bloody");
    assert!(me.alive && me.down == 0, "the stomper stays on their feet");
}

#[test]
fn a_finisher_more_than_a_second_after_the_throw_is_the_usual_one() {
    let mut s = pair(0.9);
    shoulder_then_finish(&mut s, 65);
    let mv = s.finishers.first().map(|x| x.mv);
    assert!(mv.is_some() && mv != Some(Finish::Stomp), "too late for the stomp: {mv:?}");
}

#[test]
fn curb_stomps_are_deterministic_under_rollback() {
    let mut s = pair(0.9);
    let k = shoulder_then_finish(&mut s, 10);
    // replay the same inputs from the start under the rollback checker
    let mut st = stickfell_net::SyncTestSession::new(pair(0.9), 8);
    st.game.fighters[1].health = 50.0;
    let mut replay = pair(0.9);
    replay.fighters[1].health = 50.0;
    let mut released = None;
    let mut stomped = false;
    for i in 0..k + 160 {
        let thrown = replay.fighters[0].shoulder > 0;
        let b = match released {
            Some(r) if i == r + 10 => input::encode(FINISH, Vec2::new(-0.6, 0.0)),
            Some(_) => input::encode(0, Vec2::new(-0.6, 0.0)),
            None => shoulder_throw_input(&replay, i),
        };
        let inputs = [b, input::encode(0, Vec2::new(-0.6, 0.0))];
        replay.step(&inputs);
        st.advance(&inputs).unwrap();
        if released.is_none() && thrown && replay.fighters[0].shoulder == 0 {
            released = Some(i);
        }
        stomped |= st.game.finishers.iter().any(|x| x.mv == Finish::Stomp);
    }
    assert!(stomped, "the curb stomp should happen under rollback too");
    assert!(st.game.fighters[1].head_popped);
}

#[test]
#[ignore]
fn screenshot_curb_stomp() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = pair(0.9);
    shoulder_then_finish(&mut s, 10);
    for _ in 0..140 {
        s.step(&[input::encode(0, Vec2::new(-0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
        let t = s.finishers.first().map_or(0, |x| x.t);
        if [20, 40, 50, 53, 70, 88, 93, 110].contains(&t) {
            let mut sp = crate::Spectate { game: s.clone(), follow: 0, cam: Vec2::ZERO, levels: crate::level::levels() };
            stickfell_app::capture_headless(&mut sp, 0, (900, 560), format!("{dir}/stomp_{t:03}.png").as_ref()).unwrap();
        }
    }
}

#[test]
#[ignore]
fn trace_curb_stomp() {
    let mut s = pair(0.9);
    shoulder_then_finish(&mut s, 10);
    for _ in 0..130 {
        s.step(&[input::encode(0, Vec2::new(-0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
        let Some(x) = s.finishers.first() else { break };
        if x.t % 4 == 0 || crate::weapon::STOMP_HITS.contains(&x.t) {
            let (a, v) = (&s.fighters[0], &s.fighters[1]);
            let p = |f: &Fighter, slot| f.pos(&s.world, slot);
            let tgt = s.stomp_foot(x).map(|t| t.0);
            eprintln!("    hip ({:5.2},{:4.2}) target {:?} foot vel {:?}", p(a, PELVIS).x, p(a, PELVIS).y, tgt.map(|t| (t.x * 100.0).round() / 100.0), s.world.particles[a.p(fighter::FOOT_A) as usize].vel);
            eprintln!("t{:3} me x {:5.2} facing {:2} | foot ({:5.2},{:4.2}) knee ({:5.2},{:4.2}) | their head ({:5.2},{:4.2}) pelvis x {:5.2} | grounded {} down {}",
                x.t, p(a, PELVIS).x, a.facing, p(a, fighter::FOOT_A).x, p(a, fighter::FOOT_A).y, p(a, fighter::KNEE_A).x, p(a, fighter::KNEE_A).y, p(v, HEAD).x, p(v, HEAD).y, p(v, PELVIS).x, a.grounded, a.down);
        }
    }
}

#[test]
#[ignore]
fn screenshot_curb_stomp_close() {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = pair(0.9);
    shoulder_then_finish(&mut s, 10);
    for _ in 0..140 {
        s.step(&[input::encode(0, Vec2::new(-0.6, 0.0)), input::encode(0, Vec2::new(-0.6, 0.0))]);
        let t = s.finishers.first().map_or(0, |x| x.t);
        if [44, 53, 70, 92, 94, 100, 115].contains(&t) {
            let at = s.fighters[1].pos(&s.world, fighter::NECK) + Vec2::new(0.4, 0.5);
            let mut c = CloseUp { game: s.clone(), at };
            stickfell_app::capture_headless(&mut c, 0, (640, 480), format!("{dir}/close_{t:03}.png").as_ref()).unwrap();
        }
    }
}
