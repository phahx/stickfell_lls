//! Character lab: tries new character art on Stickfell's real fighters.
//!
//!     cargo run --release -p stickfell_character_lab -- [KIT_FOLDER]
//!
//! It runs the game's own simulation and drawing (the source files are
//! borrowed from `game/src`, unchanged), but draws the fighters with the
//! textures from a character kit instead of capsules. The game itself is
//! untouched: nothing here ships, and the art is read from the kit folder
//! (default `S:/asset-art/stickfell-character-base`) each time it changes.
//!
//! Also: `--screenshot out.png [--ticks N] [--level I]` for a headless shot
//! with CPU fighters.
#![allow(dead_code)]

#[path = "../../../game/src/audio.rs"]
mod audio;
#[path = "../../../game/src/bot.rs"]
mod bot;
#[path = "../../../game/src/character.rs"]
mod character;
#[path = "../../../game/src/character_data.rs"]
mod character_data;
#[path = "../../../game/src/draw.rs"]
mod draw;
#[path = "../../../game/src/fighter.rs"]
mod fighter;
#[path = "../../../game/src/input.rs"]
mod input;
#[path = "../../../game/src/level.rs"]
mod level;
#[path = "../../../game/src/liquid.rs"]
mod liquid;
#[path = "../../../game/src/sfx.rs"]
mod sfx;
#[path = "../../../game/src/steam.rs"]
mod steam;
#[path = "../../../game/src/sim.rs"]
mod sim;
#[path = "../../../game/src/voice.rs"]
mod voice;
#[path = "../../../game/src/weapon.rs"]
mod weapon;

mod art;
mod body;
mod joints;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use stickfell_app::{AppConfig, Game, Input, KeyCode};
use stickfell_net::RollbackGame;
use stickfell_physics::Vec2;
use stickfell_render::{Camera2D, Color, Frame};

use draw::{CAMERA_HEIGHT, Hud, Skin};
use level::LevelDef;
use sim::GameState;

const DEFAULT_KIT: &str = "S:/asset-art/stickfell-character-base";

struct Lab {
    kit_dir: PathBuf,
    /// The loaded art (options live in `opts`), and the files to watch.
    kit: Option<art::Kit>,
    kit_stamp: Option<SystemTime>,
    opts: Opts,
    game: GameState,
    prev: Vec<Vec2>,
    level: usize,
    cpu: bool,
    levels: Vec<LevelDef>,
    cam: Vec2,
    cam_h: f32,
    cam_ready: bool,
    camera: Camera2D,
    mouse: Vec2,
    screen: Vec2,
    show_help: bool,
    audio: Option<audio::Audio>,
    /// A message along the bottom for a couple of seconds.
    note: Option<(String, u32)>,
    /// Escape pressed once: press again (within this many ticks) to quit.
    quit_armed: u32,
    quit: bool,
    /// Headless screenshots: CPU fighters everywhere.
    headless: bool,
    ticks: u32,
    /// Camera zoom (1 = the game's view; smaller is closer).
    zoom: f32,
    /// The round whose fighters have been reshaped.
    shaped_round: Option<u32>,
}

#[derive(Clone, Copy)]
struct Opts {
    skin: bool,
    faces: bool,
    outline: bool,
    art_width: bool,
    /// Fighters built with the kit's proportions (off: the original stick
    /// figure's, to compare).
    body: bool,
    /// Realistic joint limits on elbows, knees, hips and neck.
    limits: bool,
}

impl Lab {
    fn new(kit_dir: PathBuf, level: usize) -> Self {
        let mut lab = Self {
            kit_dir,
            kit: None,
            kit_stamp: None,
            opts: Opts { skin: true, faces: true, outline: false, art_width: false, body: true, limits: true },
            game: GameState::new(4, level, 0x7EA1),
            prev: Vec::new(),
            level,
            cpu: level != level::TRAINING,
            levels: level::levels(),
            cam: Vec2::ZERO,
            cam_h: CAMERA_HEIGHT,
            cam_ready: false,
            camera: Camera2D { center: Vec2::ZERO, height: CAMERA_HEIGHT },
            mouse: Vec2::ZERO,
            screen: Vec2::new(1280.0, 720.0),
            show_help: true,
            audio: None,
            note: None,
            quit_armed: 0,
            quit: false,
            headless: false,
            ticks: 0,
            zoom: 1.0,
            shaped_round: None,
        };
        lab.reload();
        lab.restart();
        lab
    }

    fn reload(&mut self) {
        match art::load(&self.kit_dir) {
            Ok(kit) => {
                self.kit_stamp = art::stamp(&kit.files);
                let first = self.kit.is_none();
                self.kit = Some(kit);
                self.say("Art loaded");
                if self.opts.body && !first {
                    // new proportions need fresh bodies
                    self.restart();
                }
            }
            Err(e) => {
                // keep showing the last good art, if there was any
                eprintln!("[character lab] {e}");
                self.kit_stamp = self.kit.as_ref().and_then(|k| art::stamp(&k.files));
                self.say(&format!("Couldn't load the art: {e}"));
            }
        }
    }

    fn restart(&mut self) {
        self.game = GameState::new(4, self.level, 0x7EA1);
        self.shaped_round = None;
        self.shape();
        self.prev = draw::snapshot(&self.game);
        self.cam_ready = false;
    }

    /// Gives a new round's fighters the proportions being tried: the loaded
    /// kit's (the game spawns the baked kit's, which may be older), or the
    /// original stick figure's to compare.
    fn shape(&mut self) {
        if self.shaped_round == Some(self.game.round) {
            return;
        }
        let proportions = if self.opts.body { self.kit.as_ref().map(|k| k.proportions) } else { Some(body::Proportions::TUNED) };
        for i in 0..self.game.fighters.len() {
            let mut f = self.game.fighters[i].clone();
            if let Some(p) = &proportions {
                body::reshape(&mut self.game.world, &mut f, p);
            }
            self.game.fighters[i] = f;
        }
        self.shaped_round = Some(self.game.round);
        self.prev = draw::snapshot(&self.game);
    }

    fn say(&mut self, text: &str) {
        self.note = Some((text.to_string(), 150));
    }

    fn skin(&self) -> Option<Arc<Skin>> {
        let kit = self.kit.as_ref()?;
        let o = self.opts;
        Some(Arc::new(Skin { faces: o.faces, outline: o.outline, art_width: o.art_width, ..kit.skin.clone() }))
    }
}

impl Game for Lab {
    fn tick(&mut self, input: &Input) {
        self.mouse = input.mouse;
        if input.screen.x > 0.0 {
            self.screen = input.screen;
        }
        self.ticks += 1;
        if let Some((_, left)) = &mut self.note {
            *left = left.saturating_sub(1);
            if *left == 0 {
                self.note = None;
            }
        }
        self.quit_armed = self.quit_armed.saturating_sub(1);
        if input.pressed(KeyCode::Escape) {
            if self.quit_armed > 0 {
                self.quit = true;
            } else {
                self.quit_armed = 150;
                self.say("Press Esc again to quit the lab");
            }
        }
        let flip = |key: KeyCode, v: &mut bool| {
            if input.pressed(key) {
                *v = !*v;
            }
        };
        flip(KeyCode::Digit1, &mut self.opts.skin);
        flip(KeyCode::Digit2, &mut self.opts.faces);
        flip(KeyCode::Digit3, &mut self.opts.outline);
        flip(KeyCode::Digit4, &mut self.opts.art_width);
        if input.pressed(KeyCode::Digit6) {
            self.opts.limits = !self.opts.limits;
            self.say(if self.opts.limits { "Joint limits on" } else { "Joint limits off" });
        }
        if input.pressed(KeyCode::Digit5) {
            self.opts.body = !self.opts.body;
            self.restart();
            self.say(if self.opts.body { "Body: the art's proportions" } else { "Body: the game's proportions" });
        }
        flip(KeyCode::F1, &mut self.show_help);
        if input.pressed(KeyCode::KeyB) {
            self.cpu = !self.cpu;
            self.say(if self.cpu { "Opponents: CPU fighters" } else { "Opponents: dummies" });
        }
        if input.pressed(KeyCode::KeyL) {
            self.level = (self.level + 1) % self.levels.len();
            self.restart();
            self.say(self.levels[self.level].name);
        }
        if input.pressed(KeyCode::KeyR) {
            self.restart();
        }
        if input.pressed(KeyCode::KeyZ) {
            self.zoom = match self.zoom {
                z if z > 0.8 => 0.55,
                z if z > 0.45 => 0.3,
                _ => 1.0,
            };
        }
        // the art reloads by itself when its files change (or with F5)
        let changed = self.ticks % 30 == 0 && self.kit.as_ref().is_some_and(|k| art::stamp(&k.files) != self.kit_stamp);
        if input.pressed(KeyCode::F5) || changed || (self.kit.is_none() && self.ticks % 120 == 0) {
            self.reload();
        }

        self.game.joint_limits = self.opts.limits;
        self.prev = draw::snapshot(&self.game);
        let g = &self.game;
        let me = if self.headless {
            bot::bot_input(g, 0)
        } else {
            let neck = g.fighters[0].pos(&g.world, fighter::NECK);
            let mouse_world = self.camera.screen_to_world(self.mouse, self.screen);
            input::read_local(input, mouse_world - neck)
        };
        let mut inputs = vec![me];
        inputs.extend((1..g.num_players).map(|i| if self.cpu || self.headless { bot::bot_input(g, i) } else { bot::dummy_input(g, i) }));
        self.game.step(&inputs);
        self.shape();
        if let Some(a) = &mut self.audio {
            let speakers: Vec<audio::Speaker> = self
                .game
                .fighters
                .iter()
                .map(|f| audio::Speaker { mouth: f.pos(&self.game.world, fighter::HEAD), alive: f.alive, choking: f.choking, burning: f.burning })
                .collect();
            a.play_frame(&self.game.events, self.cam, &speakers);
            a.speak(&self.game.voice_log, self.game.frame, self.cam);
        }
    }

    fn draw(&mut self, frame: &mut Frame, screen: Vec2, alpha: f32, dt: f32) {
        // the crosshair follows the cursor every frame, not just every tick
        if !self.headless {
            self.mouse = frame.mouse;
        }
        let g = &draw::interpolate(&self.game, &self.prev, if self.headless { 1.0 } else { alpha });
        let shot = draw::cinematic(g, 0);
        let want_h = shot.map_or(CAMERA_HEIGHT, |s| s.1) * self.zoom;
        self.cam_h += (want_h - self.cam_h) * (1.0 - (-dt * 7.0).exp());
        let target = shot.map_or_else(|| draw::camera_target(g, Some(0)), |s| s.0);
        draw::follow_camera(&mut self.cam, target, g, screen, !self.cam_ready || self.headless, dt, self.cam_h);
        self.cam_ready = true;

        let on = |b: bool| if b { "on" } else { "off" };
        let mut hud = Hud {
            local: Some(0),
            mouse: (!self.headless).then_some(self.mouse),
            zoom: self.cam_h,
            talk: self.audio.as_ref().map_or([None; sim::MAX_PLAYERS], |a| a.talking()),
            skin: self.skin(),
            capsules: !self.opts.skin,
            ..Default::default()
        };
        let gold = Color::hex(0xffd75a);
        let grey = Color::hex(0xc0c6d4);
        let art = if self.kit.is_some() { self.kit_dir.display().to_string() } else { "no art loaded".into() };
        hud.lines.push((format!("Character lab  -  {art}"), gold));
        if self.show_help {
            let o = self.opts;
            hud.lines.push((format!("1 art: {}   2 faces: {}   3 outline: {}   4 thickness: {}   5 body: {}   6 joint limits: {}", on(o.skin), on(o.faces), on(o.outline), if o.art_width { "the art's" } else { "hitboxes" }, if o.body { "the kit's" } else { "the old stick figure's" }, on(o.limits)), grey));
            hud.lines.push((format!("L level: {}   B opponents: {}   Z zoom   R reset", self.levels[self.level].name, if self.cpu { "CPU" } else { "dummies" }), grey));
            hud.lines.push(("F5 reload the art (it also reloads when the files change)   F1 hide   Esc quit".into(), grey));
        }
        if let Some((text, _)) = &self.note {
            hud.banner = Some((text.clone(), Color::WHITE));
        }
        let lv = &self.levels[g.level_index];
        draw::draw_game(g, lv, frame, screen, self.cam, &hud);
        self.camera = frame.camera;
    }

    fn wants_exit(&self) -> bool {
        self.quit
    }

    fn cursor_visible(&self) -> bool {
        false
    }

    fn cursor_captured(&self) -> bool {
        true
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut kit = PathBuf::from(DEFAULT_KIT);
    let (mut shot, mut ticks, mut level, mut zoom) = (None, 600u32, level::TRAINING, 1.0f32);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => shot = args.next(),
            "--ticks" => ticks = args.next().and_then(|v| v.parse().ok()).unwrap_or(ticks),
            "--joint-report" => {
                let n = args.next().and_then(|v| v.parse().ok()).unwrap_or(3600u32);
                joint_report(PathBuf::from(DEFAULT_KIT), n);
                return;
            }
            "--zoom" => zoom = args.next().and_then(|v| v.parse().ok()).unwrap_or(zoom),
            "--level" => level = args.next().and_then(|v| v.parse().ok()).unwrap_or(level).min(level::levels().len() - 1),
            "-h" | "--help" => {
                println!("character_lab [KIT_FOLDER] [--screenshot out.png --ticks N --level 0-3 --zoom 0.3]");
                return;
            }
            _ => kit = PathBuf::from(a),
        }
    }
    let mut lab = Lab::new(kit, level);
    if let Some(path) = shot {
        lab.headless = true;
        lab.cpu = true;
        lab.zoom = zoom;
        if let Err(e) = stickfell_app::capture_headless(&mut lab, ticks, (1280, 720), path.as_ref()) {
            eprintln!("character lab: screenshot failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    lab.audio = audio::Audio::new();
    let cfg = AppConfig { title: "Stickfell character lab".into(), ..Default::default() };
    if let Err(e) = stickfell_app::run(cfg, lab) {
        eprintln!("character lab: {e}");
        std::process::exit(1);
    }
}

/// Headless: CPU fights with joint limits off, then on, reporting how often
/// and how far joints leave their realistic range, and how much of the time
/// the fighters spend on their feet.
fn joint_report(kit: PathBuf, ticks: u32) {
    for limits in [false, true] {
        let mut lab = Lab::new(kit.clone(), level::TRAINING);
        lab.headless = true;
        lab.cpu = true;
        lab.opts.limits = limits;
        let input = Input::default();
        let (mut samples, mut bad, mut worst, mut standing, mut alive) = (0u32, 0u32, 0.0f32, 0u32, 0u32);
        let mut per = [0u32; joints::COUNT];
        let mut per_up = [0u32; joints::COUNT];
        for _ in 0..ticks {
            lab.tick(&input);
            for f in lab.game.fighters.iter().filter(|f| f.alive) {
                let up = f.down == 0 && f.grounded;
                for (k, e) in joints::each(&lab.game.world, f).iter().enumerate() {
                    if *e > 0.15 && up {
                        per_up[k] += 1;
                    }
                    if *e > 0.15 {
                        per[k] += 1;
                    }
                }
                let v = joints::worst(&lab.game.world, f);
                samples += 1;
                if v > 0.15 {
                    bad += 1;
                }
                worst = worst.max(v);
                alive += 1;
                if f.down == 0 && f.grounded {
                    standing += 1;
                }
            }
        }
        println!(
            "limits {:3}: joints past their range (by >0.15 rad) {:5.1}% of fighter-frames, worst {:4.2} rad; on their feet {:4.1}%; rounds played {}",
            if limits { "on" } else { "off" },
            100.0 * bad as f32 / samples.max(1) as f32,
            worst,
            100.0 * standing as f32 / alive.max(1) as f32,
            lab.game.round
        );
        let names = ["elbow A", "elbow B", "knee A", "knee B", "hip A", "hip B", "neck"];
        let parts: Vec<String> = names.iter().zip(per).map(|(n, c)| format!("{n} {:.1}%", 100.0 * c as f32 / samples.max(1) as f32)).collect();
        println!("    {}", parts.join(", "));
        let parts: Vec<String> = names.iter().zip(per_up).map(|(n, c)| format!("{n} {:.1}%", 100.0 * c as f32 / standing.max(1) as f32)).collect();
        println!("    while standing: {}", parts.join(", "));
    }
}
