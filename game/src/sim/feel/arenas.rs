//! The arenas, teams and AI fighters, and what the sim costs.

use super::*;

#[test]
#[ignore]
fn screenshot_faces() {
    use crate::sfx::Sfx;
    struct Shot {
        game: GameState,
        levels: Vec<crate::level::LevelDef>,
        talk: [Option<(Sfx, f32)>; MAX_PLAYERS],
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
            2 => s.finishers.push(Finisher { attacker: 0, victim: 1, mv: crate::weapon::Finish::Behead, t: 10, weapon: None, killed: false, wall: None, side: 1 }),
            _ => {}
        }
        let mut shot = Shot { game: s, levels: crate::level::levels(), talk: std::array::from_fn(|i| talk.get(i).copied().flatten()) };
        stickfell_app::capture_headless(&mut shot, 0, (900, 520), format!("{dir}/face_{name}.png").as_ref()).unwrap();
    }
}

#[test]
#[ignore]
fn measure_sim_cost() {
    // the real per-frame simulation cost (release build), liquids included
    for (level, players, name) in [(crate::level::TRAINING, 4usize, "training yard"), (crate::level::GAUNTLET, 2, "gauntlet"), (0, 2, "foundry")] {
        let mut s = GameState::new(players, level, 7);
        let drops = s.fluid.drops.len();
        let inputs = vec![0u32; players];
        for _ in 0..60 {
            s.step(&inputs);
        }
        let n = 600;
        let t = std::time::Instant::now();
        for _ in 0..n {
            s.step(&inputs);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("{name}: {drops} drops, {ms:.3} ms per simulated frame");
    }
}

#[test]
#[ignore]
fn hunt_bad_sound_events() {
    // every weapon, both fighters mashing attack / throw / alt as fast as the
    // keys go, aiming all over: any sound with a position or volume that isn't
    // a number (one such sample silences all audio)
    let mut bad = 0;
    for def in 0..crate::weapon::WEAPONS.len() as u8 {
        for gap in [1.2f32, 3.0] {
            let (mut s, _) = duel_setup(def, gap);
            for k in 0..900u32 {
                let ang = k as f32 * 0.37;
                let aim = Vec2::new(dmath::cos(ang), dmath::sin(ang) * 0.8) * 0.6;
                let b = [ATTACK, ATTACK | THROW, ALT, ATTACK | ALT][(k % 4) as usize] | if k % 30 < 3 { JUMP } else { 0 };
                s.step(&[input::encode(b, aim), input::encode(b, -aim)]);
                for e in &s.events {
                    if !e.pos.is_finite() || !e.volume.is_finite() {
                        bad += 1;
                        if bad <= 10 {
                            eprintln!("{} gap {gap} frame {k}: {:?} at {:?} volume {}", crate::weapon::WEAPONS[def as usize].name, e.kind, e.pos, e.volume);
                        }
                    }
                }
                if let Some((i, p)) = s.world.particles.iter().enumerate().find(|(_, p)| !p.pos.is_finite() || !p.vel.is_finite()) {
                    eprintln!("{} gap {gap} frame {k}: particle {i} not finite {:?} {:?}", crate::weapon::WEAPONS[def as usize].name, p.pos, p.vel);
                    bad += 1;
                    break;
                }
            }
        }
    }
    eprintln!("{bad} bad events/particles");
}

/// The Gauntlet with eight AI fighters at it: the whole level, then each
/// half closer (SHOT_DIR, default here).
#[test]
#[ignore]
fn screenshot_gauntlet() {
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
            crate::draw::draw_game(&self.game, &self.levels[self.game.level_index], frame, screen, self.at, &hud);
        }
    }
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    let mut s = GameState::new_match(8, 0, 0, crate::level::GAUNTLET, 11);
    for _ in 0..240 {
        s.step(&[]);
    }
    for (name, at, zoom, size) in [("all", Vec2::new(0.0, 4.5), 26.0, (2400, 700)), ("left", Vec2::new(-23.0, 4.0), 17.0, (1600, 900)), ("right", Vec2::new(23.0, 4.0), 17.0, (1600, 900))] {
        let mut shot = Shot { game: s.clone(), levels: crate::level::levels(), at, zoom };
        stickfell_app::capture_headless(&mut shot, 0, size, format!("{dir}/gauntlet_{name}.png").as_ref()).unwrap();
    }
}

/// What a step of the Gauntlet costs with eight AI fighters in it: as it
/// starts, and as the flood comes in (ms, the mean and the worst of each
/// second's steps), and how many drops there are by then.
#[test]
#[ignore]
fn measure_gauntlet_step() {
    let mut s = GameState::new_match(8, 0, 0, crate::level::GAUNTLET, 5);
    let second = |s: &mut GameState| {
        let (mut sum, mut worst) = (0.0f64, 0.0f64);
        for _ in 0..60 {
            let t = std::time::Instant::now();
            s.step(&[]);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            sum += ms;
            worst = worst.max(ms);
        }
        (sum / 60.0, worst)
    };
    for k in 0..3 {
        let (mean, worst) = second(&mut s);
        eprintln!("dry, second {k}: {mean:.2} ms a step, worst {worst:.2} ({} drops)", s.fluid.drops.len());
    }
    s.trap.open = true;
    s.trap.toggled = s.round_frame.max(1);
    for k in 0..40 {
        let (mean, worst) = second(&mut s);
        if k % 4 == 3 {
            eprintln!("flooding, second {k}: {mean:.2} ms a step, worst {worst:.2} ({} drops, steam {:.1})", s.fluid.drops.len(), s.steam.total());
        }
    }
}

/// A step's cost in each arena (AI fighters, no flood): mean and worst over
/// 3 s after the first, and the drops of liquid in it.
#[test]
#[ignore]
fn measure_arena_steps() {
    for level in 0..crate::level::ARENAS {
        for n in [4usize, 8] {
            let mut s = GameState::new_match(n, 0, 0, level, 5);
            s.trap.def = None;
            for _ in 0..60 {
                s.step(&[]);
            }
            let (mut sum, mut worst) = (0.0f64, 0.0f64);
            for _ in 0..180 {
                let t = std::time::Instant::now();
                s.step(&[]);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                sum += ms;
                worst = worst.max(ms);
            }
            let name = crate::level::levels()[level].name;
            eprintln!("{name}, {n} fighters: {:.2} ms a step, worst {worst:.2} ({} drops)", sum / 180.0, s.fluid.drops.len());
        }
    }
}

#[test]
fn teams_start_apart_never_hurt_each_other_and_win_together() {
    let mut s = GameState::new_match(4, 4, 2, crate::level::GAUNTLET, 3);
    let x = |s: &GameState, i: usize| s.fighters[i].pos(&s.world, PELVIS).x;
    assert!(x(&s, 0) < 0.0 && x(&s, 2) < 0.0 && x(&s, 1) > 0.0 && x(&s, 3) > 0.0, "one team each end");
    let at = s.fighters[2].pos(&s.world, PELVIS);
    let hp = s.fighters[2].health;
    s.damage(2, S_TORSO, 30.0, at, Vec2::X, 10.0, Kind::Blade, Some(0));
    assert_eq!(s.fighters[2].health, hp, "a teammate's hit doesn't hurt");
    let hp = s.fighters[1].health;
    s.damage(1, S_TORSO, 30.0, at, Vec2::X, 10.0, Kind::Blade, Some(0));
    assert!(s.fighters[1].health < hp, "the other team's does");
    assert!(s.finish_target(0) != Some(2));
    s.kill_fighter(1);
    s.kill_fighter(3);
    for _ in 0..20 {
        s.step(&[0; 4]);
    }
    assert!(matches!(s.phase, Phase::RoundOver { .. }), "over once one team's left");
    assert_eq!(s.scores, [1, 0, 1, 0], "the win counts for the whole team");
}

#[test]
fn ai_fighters_play_themselves() {
    // one player standing still, three AI
    let mut s = GameState::new_match(4, 1, 0, crate::level::GAUNTLET, 9);
    let start: Vec<Vec2> = s.fighters.iter().map(|f| f.pos(&s.world, PELVIS)).collect();
    for _ in 0..240 {
        s.step(&[0]);
    }
    let moved: Vec<f32> = s.fighters.iter().zip(&start).map(|(f, p)| (f.pos(&s.world, PELVIS) - *p).length()).collect();
    eprintln!("moved in 4 s: {moved:.1?}");
    assert!(moved[1..].iter().all(|&m| m > 2.0), "the AI get going");
}

#[test]
#[ignore]
fn screenshot_new_arenas() {
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
            crate::draw::draw_game(&self.game, &self.levels[self.game.level_index], frame, screen, self.at, &hud);
        }
    }
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
    for (level, name, at, zoom) in [(crate::level::SEA, "open_water", Vec2::new(0.0, 2.0), 22.0), (crate::level::SEA, "open_water_close", Vec2::new(-4.0, 1.0), 8.0), (crate::level::ROPES, "rope_course", Vec2::new(0.0, 9.0), 24.0)] {
        let mut s = GameState::new_match(8, 1, 0, level, 5);
        for _ in 0..240 {
            s.step(&[0]);
        }
        let mut shot = Shot { game: s, levels: crate::level::levels(), at, zoom };
        stickfell_app::capture_headless(&mut shot, 0, (1280, 720), format!("{dir}/{name}.png").as_ref()).unwrap();
    }
}
