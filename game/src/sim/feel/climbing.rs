//! Climbing: wall jumps, ledges, stuck blades, ropes and the rope courses.

use super::*;

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
fn practice_rope_course_can_be_crossed() {
    let log = cross_rope_course(|_, _| {});
    eprintln!("rope course: {log:?}");
    assert!(log.last().is_some_and(|l| l.starts_with("finish")), "never made it across: {log:?}");
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
#[ignore]
fn measure_ledge_climb() {
    for h in [2.0f32, 2.5, 3.0, 3.5] {
        for let_go in [false, true] {
            let (mut ok, mut caught, mut n) = (0, 0, 0);
            for jump_at in (0..40).step_by(4) {
                for double in [None, Some(10), Some(16)] {
                    for pull in [1, 8, 20] {
                        let (o, c) = ledge_climb(h, jump_at, double, pull, let_go);
                        ok += o as u32;
                        caught += c as u32;
                        n += 1;
                    }
                }
            }
            eprintln!("ledge {h} m, {}: caught hold {caught}/{n}, climbed up {ok}/{n}", if let_go { "letting go of grab after pulling" } else { "grab held throughout" });
        }
    }
}

#[test]
#[ignore]
fn trace_ledge_climb() {
    for (h, jump_at, double) in [(2.0f32, 8u32, None), (3.0, 8, Some(10u32))] {
        let mut s = sandbox(&[(Vec2::new(1.5, 0.0), Vec2::new(24.0, h))]);
        for _ in 0..30 {
            step(&mut s, 0, Vec2::new(0.6, 0.0));
        }
        let corner = Vec2::new(1.5, h);
        let mut caught_at = None;
        eprintln!("--- ledge {h}");
        for k in 0..120u32 {
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
                caught_at = Some(k);
            }
            if caught_at.is_some_and(|c| k == c + 8) {
                b |= JUMP;
            }
            step(&mut s, b, corner + Vec2::new(0.1, 0.1) - neck);
            let f = &s.fighters[0];
            if k % 3 == 0 || b & JUMP != 0 {
                let p = f.pos(&s.world, PELVIS);
                let hb = f.pos(&s.world, fighter::HAND_B);
                let ft = f.pos(&s.world, fighter::FOOT_A);
                eprintln!("k{k:3} b{b:3} pelvis ({:+.2},{:.2}) handB ({:+.2},{:.2}) foot ({:+.2},{:.2}) vel ({:+.1},{:+.1}) grab {} static {} grounded {} down {}", p.x, p.y, hb.x, hb.y, ft.x, ft.y, f.body_vel(&s.world).x, f.body_vel(&s.world).y, f.grabbing as u8, f.grab_static as u8, f.grounded as u8, f.down);
            }
        }
    }
}

#[test]
fn broken_bones_hang_limp_and_are_no_use() {
    // a broken sword arm drops the sword and can't pick it up again
    let mut s = sandbox(&[]);
    for _ in 0..20 {
        step(&mut s, 0, Vec2::new(0.6, 0.0));
    }
    let wi = give(&mut s, 0);
    let at = s.fighters[0].pos(&s.world, fighter::ELBOW_A);
    s.break_bone(0, fighter::S_FORE_A, at);
    assert_eq!(s.fighters[0].weapon, None, "the sword's dropped");
    assert!(!s.fighters[0].arm_a_ok());
    for _ in 0..30 {
        step(&mut s, PICKUP, Vec2::new(0.6, 0.0));
    }
    assert_eq!(s.fighters[0].weapon, None, "and can't be picked up with it");
    let _ = wi;
    // a broken leg: one leg left to stand on
    s.break_bone(0, fighter::S_SHIN_B, at);
    assert_eq!(s.fighters[0].legs_left(), 1);
    // (and the head or torso can't break)
    s.break_bone(0, S_TORSO, at);
    assert_eq!(s.fighters[0].broken, fighter::bit(fighter::S_FORE_A) | fighter::bit(fighter::S_SHIN_B));
    // hard blunt blows break one now and then: a sledgehammer to the arm
    let mut breaks = 0;
    for seed in 0..200u32 {
        let mut s = sandbox(&[]);
        s.rng = stickfell_physics::dmath::Rng::new(seed * 7919 + 13);
        let p = s.fighters[1].pos(&s.world, fighter::ELBOW_A);
        s.damage(1, fighter::S_UPPER_A, 20.0, p, Vec2::X, 20.0, crate::weapon::Kind::Blunt, Some(0));
        breaks += (s.fighters[1].broken != 0) as u32;
    }
    eprintln!("heavy blunt hits to the arm: {breaks}/200 broke it");
    assert!((8..=40).contains(&breaks), "a small chance ({breaks}/200)");
}

#[test]
fn the_rope_course_gaps_can_be_swung_across_both_ways() {
    let only: Option<usize> = std::env::var("GAP_FROM").ok().and_then(|v| v.parse().ok());
    for (from, dir) in [(3usize, 1.0f32), (4, -1.0), (0, 1.0), (7, -1.0)].into_iter().filter(|(f, _)| only.is_none_or(|o| o == *f)) {
        let r = cross_rope_gap(from, dir);
        eprintln!("platform {from} going {}: {r:?}", if dir > 0.0 { "right" } else { "left" });
        assert!(r.is_ok(), "platform {from} {dir}: {r:?}");
    }
    // one weapon where each starts, and none dropping in
    let mut s = GameState::new(2, crate::level::ROPES, 3);
    assert_eq!(s.weapons.iter().filter(|w| w.active).count(), 2);
    for _ in 0..(60 * 30) {
        s.step(&[0, 0]);
    }
    assert_eq!(s.weapons.iter().filter(|w| w.active).count(), 2, "nothing more drops in");
    for f in &s.fighters {
        let p = f.pos(&s.world, PELVIS);
        let near = s.weapons.iter().any(|w| w.active && w.distance_to(&s.world, p) < 2.0);
        assert!(near, "a weapon lies by fighter {} on their platform", f.id);
    }
}

#[test]
#[ignore]
fn measure_ai_on_the_rope_course() {
    for seed in [3u32, 11, 29] {
        let mut s = GameState::new_match(4, 1, 0, crate::level::ROPES, seed);
        let mut fell = vec![None; 4];
        for k in 0..(60 * 20u32) {
            s.step(&[0]);
            for (i, f) in s.fighters.iter().enumerate() {
                if fell[i].is_none() && f.pos(&s.world, PELVIS).y < crate::level::ROPE_TOP - 4.0 {
                    fell[i] = Some(k);
                }
            }
            if s.round > 1 {
                break;
            }
        }
        eprintln!("seed {seed}: fell off (frame) {fell:?}, alive {:?}", s.fighters.iter().map(|f| f.alive).collect::<Vec<_>>());
    }
}

#[test]
#[ignore]
fn trace_ai_fall_on_the_rope_course() {
    let mut s = GameState::new_match(4, 1, 0, crate::level::ROPES, 3);
    for k in 0..95u32 {
        let b = crate::bot::bot_input(&s, 1);
        s.step(&[0]);
        let f = &s.fighters[1];
        if k < 42 {
            eprintln!("{k}: bits {:#x} pelvis ({:.2},{:.2}) v ({:.1},{:.1}) grounded {} down {} weapon {:?} grabbing {}", b & 0xff, f.pos(&s.world, PELVIS).x, f.pos(&s.world, PELVIS).y, f.body_vel(&s.world).x, f.body_vel(&s.world).y, f.grounded, f.down, f.weapon, f.grabbing);
        }
    }
}
