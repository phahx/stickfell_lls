//! Stickfell: Last Limb Standing — an online physics ragdoll brawler.
//!
//! ```text
//! stickfell [--relay HOST:PORT]     play online (host a room or join one by code);
//!                                  without --relay, reads relay.txt next to the exe
//!
//! dev tools:
//! stickfell --host [--autostart 2] | --join ABCD   skip the menu
//! stickfell --bot                   a CPU drives your fighter (soak-testing netcode)
//! stickfell --net --player 0 --bind 0.0.0.0:7000 --peer 1.2.3.4:7001   direct-IP match (no relay)
//! stickfell --verify 3600 [--players 4] [--level 1]   headless CPU match under rollback checking
//! stickfell --screenshot shot.png [--ticks 600] [--players 4] [--level 2] [--follow 1]
//! ```

mod audio;
mod bot;
mod character;
mod character_data;
mod draw;
mod fighter;
mod input;
mod level;
mod loading;
mod sfx;
mod sim;
mod voice;
mod weapon;

use std::net::SocketAddr;
use std::process::ExitCode;

use stickfell_app::{AppConfig, Game, Input, KeyCode, Mouse};
use stickfell_net::{
    LobbyClient, LobbyState, P2PSession, RollbackGame, SessionConfig, SessionEvent, SessionState, SyncTestSession, Transport,
    UdpTransport,
};
use stickfell_physics::Vec2;
use stickfell_render::{Camera2D, Color, Font, Frame};

use crate::draw::{CAMERA_HEIGHT, Hud};
use crate::fighter::NECK;
use crate::level::{LevelDef, levels};
use crate::sim::GameState;

const DEFAULT_RELAY: &str = "127.0.0.1:7777";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Host,
    Join,
    ConfirmJoin,
    Cancel,
    Start,
    Leave,
    Quit,
    Practice,
    /// Practice on the level built from the art kit.
    ArtTest,
    /// Answer to the "are you sure?" prompt.
    ConfirmYes,
    ConfirmNo,
}

/// What an open "are you sure?" prompt would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    Quit,
    LeaveMatch,
    LeavePractice,
}

/// Rolling performance numbers for a match, logged every few seconds so lag
/// can be pinned on frame rate, simulation cost, or the network.
#[derive(Default)]
struct Perf {
    since: Option<std::time::Instant>,
    frames: u32,
    ticks: u32,
    tick_us: u64,
    tick_max_us: u64,
    draw_us: u64,
    draw_max_us: u64,
    /// Real time between drawn frames: longest gap, and how many gaps were
    /// long enough to see (over 8 ms, i.e. a missed frame at 120 Hz+).
    gap_max: f32,
    gaps: Vec<f32>,
    net: (u64, u64, u64, u64),
    line: String,
}

impl Perf {
    fn add_tick(&mut self, us: u64) {
        self.ticks += 1;
        self.tick_us += us;
        self.tick_max_us = self.tick_max_us.max(us);
    }
    fn add_draw(&mut self, us: u64, dt: f32) {
        self.gap_max = self.gap_max.max(dt);
        self.gaps.push(dt);
        self.frames += 1;
        self.draw_us += us;
        self.draw_max_us = self.draw_max_us.max(us);
    }
    /// Every 3 s: summarize, log, reset.
    fn report(&mut self, stats: &stickfell_net::NetStats, ahead: f32) {
        let now = std::time::Instant::now();
        let since = *self.since.get_or_insert(now);
        let secs = (now - since).as_secs_f32();
        if secs < 3.0 {
            return;
        }
        let net = (stats.rollbacks, stats.resimulated_frames, stats.skipped_ticks, stats.stalls);
        let d = (net.0 - self.net.0, net.1 - self.net.1, net.2 - self.net.2, net.3 - self.net.3);
        let hitches = self.gaps.iter().filter(|&&g| g > 0.008).count();
        self.line = format!(
            "FPS {:.0}  FRAME GAP MAX {:.1}MS  HITCHES {}  TICKS {:.0}/S  SIM {:.2}MS (MAX {:.1})  DRAW {:.2}MS (MAX {:.1})  ROLLBACK {:.1}F  WAITS {}  STALLS {}  AHEAD {:.1}",
            self.frames as f32 / secs,
            self.gap_max * 1000.0,
            hitches,
            self.ticks as f32 / secs,
            self.tick_us as f32 / self.ticks.max(1) as f32 / 1000.0,
            self.tick_max_us as f32 / 1000.0,
            self.draw_us as f32 / self.frames.max(1) as f32 / 1000.0,
            self.draw_max_us as f32 / 1000.0,
            d.1 as f32 / d.0.max(1) as f32,
            d.2,
            d.3,
            ahead
        );
        eprintln!("[perf] {}", self.line);
        *self = Perf { since: Some(now), net, line: std::mem::take(&mut self.line), ..Default::default() };
    }
}

struct Match {
    perf: Perf,
    session: P2PSession<GameState, Box<dyn Transport>>,
    local: usize,
    cam: Vec2,
    cam_ready: bool,
    /// Camera view height (zooms in for blade locks).
    cam_h: f32,
    /// Particle positions before the latest tick (render interpolation).
    prev: Vec<Vec2>,
    log: Vec<String>,
    ended: Option<u32>,
}

enum Screen {
    Menu { code: String, entering: bool, error: Option<String> },
    Lobby(Box<LobbyClient>),
    Match(Box<Match>),
    Practice(Box<Practice>),
}

/// Single-player practice yard: you against harmless dummies, no network.
struct Practice {
    level: usize,
    game: GameState,
    prev: Vec<Vec2>,
    cam: Vec2,
    cam_h: f32,
    cam_ready: bool,
}

impl Practice {
    fn new(level: usize) -> Self {
        let game = GameState::new(4, level, 0x7EA1);
        Self { level, prev: draw::snapshot(&game), game, cam: Vec2::ZERO, cam_h: CAMERA_HEIGHT, cam_ready: false }
    }
}

fn menu(error: Option<String>) -> Screen {
    Screen::Menu { code: String::new(), entering: false, error }
}

/// A CPU-vs-CPU match that plays behind the menus.
struct Demo {
    game: GameState,
    prev: Vec<Vec2>,
    cam: Vec2,
    ready: bool,
}

impl Demo {
    fn new() -> Self {
        let game = GameState::new(4, 0, 0xC0FFEE);
        Self { prev: draw::snapshot(&game), game, cam: Vec2::ZERO, ready: false }
    }
    fn tick(&mut self) {
        self.prev = draw::snapshot(&self.game);
        let inputs: Vec<u32> = (0..4).map(|p| bot::bot_input(&self.game, p)).collect();
        self.game.step(&inputs);
    }
    fn draw(&mut self, levels: &[LevelDef], frame: &mut Frame, screen: Vec2, alpha: f32, dt: f32) {
        let g = draw::interpolate(&self.game, &self.prev, alpha);
        let follow = (g.frame / 600) as usize % 4;
        let target = draw::camera_target(&g, Some(follow));
        draw::follow_camera(&mut self.cam, target, &g, screen, !self.ready, dt, CAMERA_HEIGHT);
        self.ready = true;
        let lv = &levels[g.level_index];
        draw::draw_game(&g, lv, frame, screen, self.cam, &Hud { world_only: true, ..Default::default() });
        frame.ui_rect(Vec2::ZERO, screen, 0.0, Color::BLACK.alpha(0.45));
    }
}

struct App {
    screen: Screen,
    relay: String,
    levels: Vec<LevelDef>,
    demo: Demo,
    buttons: Vec<(Vec2, Vec2, Action)>,
    quit: bool,
    show_help: bool,
    mouse: Vec2,
    camera: Camera2D,
    screen_size: Vec2,
    autostart: Option<usize>,
    bot_local: bool,
    announced: bool,
    /// Escape opens this instead of quitting or leaving straight away.
    confirm: Option<Confirm>,
    /// Sound output (None when headless or there's no audio device).
    audio: Option<audio::Audio>,
    muted: bool,
    /// The loading screen, until everything's ready (windowed runs only).
    loading: Option<Box<loading::Loading>>,
    /// Fading in from the loading screen (seconds left).
    fade: f32,
}

impl App {
    fn new(screen: Screen, relay: String) -> Self {
        Self {
            screen,
            relay,
            levels: levels(),
            demo: Demo::new(),
            buttons: Vec::new(),
            quit: false,
            show_help: true,
            mouse: Vec2::ZERO,
            camera: Camera2D { center: Vec2::ZERO, height: CAMERA_HEIGHT },
            screen_size: Vec2::new(1280.0, 720.0),
            autostart: None,
            bot_local: false,
            announced: false,
            confirm: None,
            audio: None,
            muted: false,
            loading: None,
            fade: 0.0,
        }
    }

    fn start_match(&mut self, transport: Box<dyn Transport>, num_players: usize, local: usize, seed: u32, delay: i32) {
        let game = GameState::new(num_players, seed as usize % level::ARENAS, seed);
        let mut cfg = SessionConfig::new(num_players, local);
        cfg.input_delay = delay;
        self.screen = Screen::Match(Box::new(Match {
            perf: Perf::default(),
            session: P2PSession::new(game, transport, cfg),
            local,
            cam: Vec2::ZERO,
            cam_ready: false,
            cam_h: CAMERA_HEIGHT,
            prev: Vec::new(),
            log: Vec::new(),
            ended: None,
        }));
    }

    fn do_action(&mut self, a: Action) {
        match a {
            Action::Quit => self.quit = true,
            Action::Practice => self.screen = Screen::Practice(Box::new(Practice::new(level::TRAINING))),
            Action::ArtTest => self.screen = Screen::Practice(Box::new(Practice::new(level::ART_TEST))),
            Action::ConfirmYes => match self.confirm.take() {
                Some(Confirm::Quit) => self.quit = true,
                Some(Confirm::LeaveMatch) => self.screen = menu(None),
                Some(Confirm::LeavePractice) => self.screen = menu(None),
                None => {}
            },
            Action::ConfirmNo => self.confirm = None,
            Action::Host => match LobbyClient::host(&self.relay, 4) {
                Ok(c) => self.screen = Screen::Lobby(Box::new(c)),
                Err(e) => self.screen = menu(Some(format!("Can't reach the relay: {e}"))),
            },
            Action::Join => {
                if let Screen::Menu { entering, error, .. } = &mut self.screen {
                    *entering = true;
                    *error = None;
                }
            }
            Action::ConfirmJoin => {
                let code = match &self.screen {
                    Screen::Menu { code, .. } if code.len() == 4 => code.clone(),
                    _ => return,
                };
                match LobbyClient::join(&self.relay, &code) {
                    Ok(c) => self.screen = Screen::Lobby(Box::new(c)),
                    Err(e) => self.screen = menu(Some(format!("Can't reach the relay: {e}"))),
                }
            }
            Action::Cancel => {
                if let Screen::Menu { entering, code, .. } = &mut self.screen {
                    *entering = false;
                    code.clear();
                }
            }
            Action::Start => {
                if let Screen::Lobby(c) = &mut self.screen {
                    c.start();
                }
            }
            Action::Leave => {
                if let Screen::Lobby(c) = &mut self.screen {
                    c.leave();
                }
                self.screen = menu(None);
            }
        }
    }

    fn button(&mut self, frame: &mut Frame, center_x: f32, y: f32, w: f32, label: &str, action: Action, k: f32, enabled: bool) {
        let h = 46.0 * k;
        let min = Vec2::new(center_x - w * 0.5, y);
        let size = Vec2::new(w, h);
        let hover = enabled && (self.mouse - min).cmpge(Vec2::ZERO).all() && (self.mouse - min - size).cmple(Vec2::ZERO).all();
        let bg = if !enabled {
            Color::hex(0x2a2d35).alpha(0.8)
        } else if hover {
            Color::hex(0xe8484f)
        } else {
            Color::hex(0x1c1f27).alpha(0.92)
        };
        frame.ui_rect(min, size, 6.0 * k, bg);
        let scale = 2.6 * k;
        let tw = Frame::text_width(label, scale);
        let tc = if enabled { Color::WHITE } else { Color::hex(0x6a6f7a) };
        frame.ui_text(Vec2::new(center_x - tw * 0.5, y + (h - 7.0 * scale) * 0.5), scale, label, tc);
        if enabled {
            self.buttons.push((min, size, action));
        }
    }
}

fn text_c(frame: &mut Frame, screen: Vec2, y: f32, scale: f32, text: &str, color: Color) {
    let w = Frame::text_width(text, scale);
    frame.ui_text(Vec2::new((screen.x - w) * 0.5, y), scale, text, color);
}

/// Centered, in the title font (the game's name, room codes, dialog titles).
fn title_c(frame: &mut Frame, screen: Vec2, y: f32, scale: f32, text: &str, color: Color) {
    let w = Frame::text_width_in(Font::Title, text, scale);
    frame.ui_text_in(Font::Title, Vec2::new((screen.x - w) * 0.5, y), scale, text, color);
}

impl Game for App {
    fn tick(&mut self, input: &Input) {
        self.mouse = input.mouse;
        if input.screen.x > 0.0 {
            self.screen_size = input.screen;
        }
        // (nothing runs until it's all loaded)
        if self.loading.is_some() {
            return;
        }
        if input.pressed(KeyCode::F1) {
            self.show_help = !self.show_help;
        }
        if input.pressed(KeyCode::KeyM) {
            self.muted = !self.muted;
            if let Some(a) = &mut self.audio {
                a.volume = if self.muted { 0.0 } else { audio::MASTER_VOLUME };
            }
        }
        let clicked = if input.mouse_pressed(Mouse::Left) {
            self.buttons
                .iter()
                .find(|(min, size, _)| {
                    let d = self.mouse - *min;
                    d.x >= 0.0 && d.y >= 0.0 && d.x <= size.x && d.y <= size.y
                })
                .map(|b| b.2)
        } else {
            None
        };
        let mut action = clicked;
        if self.confirm.is_some() {
            if input.pressed(KeyCode::Escape) {
                action = Some(Action::ConfirmNo);
            } else if input.pressed(KeyCode::Enter) {
                action = Some(Action::ConfirmYes);
            }
        }
        let esc = input.pressed(KeyCode::Escape) && self.confirm.is_none();

        match &mut self.screen {
            Screen::Menu { code, entering, .. } => {
                self.demo.tick();
                if *entering {
                    for ch in input.text.chars() {
                        if ch.is_ascii_alphabetic() && code.len() < 4 {
                            code.push(ch.to_ascii_uppercase());
                        }
                    }
                    if input.pressed(KeyCode::Backspace) {
                        code.pop();
                    }
                    if input.pressed(KeyCode::Enter) && code.len() == 4 {
                        action = Some(Action::ConfirmJoin);
                    }
                    if esc {
                        action = Some(Action::Cancel);
                    }
                } else if esc {
                    self.confirm = Some(Confirm::Quit);
                }
            }
            Screen::Lobby(client) => {
                self.demo.tick();
                if esc {
                    action = Some(Action::Leave);
                }
                let state = client.poll().clone();
                if let LobbyState::InRoom { code, players, .. } = &state {
                    if !self.announced {
                        eprintln!("[stickfell] room code {code}");
                        self.announced = true;
                    }
                    if client.is_host() && self.autostart.is_some_and(|n| *players >= n) {
                        client.start();
                    }
                }
                match state {
                    LobbyState::Started { num_players, slot, seed } => {
                        let Screen::Lobby(client) = std::mem::replace(&mut self.screen, menu(None)) else { unreachable!() };
                        let transport: Box<dyn Transport> = Box::new(client.into_transport());
                        self.start_match(transport, num_players, slot, seed, 2);
                        return;
                    }
                    LobbyState::Failed(msg) => {
                        self.screen = menu(Some(msg));
                        return;
                    }
                    _ => {}
                }
            }
            Screen::Practice(p) => {
                if esc {
                    self.confirm = Some(Confirm::LeavePractice);
                }
                if input.pressed(KeyCode::KeyR) && self.confirm.is_none() {
                    **p = Practice::new(p.level);
                }
                p.prev = draw::snapshot(&p.game);
                let g = &p.game;
                let neck = g.fighters[0].pos(&g.world, NECK);
                let mouse_world = self.camera.screen_to_world(self.mouse, self.screen_size);
                let me = if self.confirm.is_some() { input::encode(0, g.fighters[0].aim) } else { input::read_local(input, mouse_world - neck) };
                let mut inputs = vec![me];
                inputs.extend((1..g.num_players).map(|i| bot::dummy_input(g, i)));
                p.game.step(&inputs);
                if let Some(a) = &mut self.audio {
                    a.play_frame(&p.game.events, p.cam, &speakers(&p.game));
                    // (no one else to wait for: every frame is final)
                    a.speak(&p.game.voice_log, p.game.frame, p.cam);
                }
            }
            Screen::Match(m) => {
                if esc {
                    self.confirm = Some(Confirm::LeaveMatch);
                }
                let prompt_open = self.confirm.is_some();
                let s = &mut m.session;
                s.poll();
                for ev in s.drain_events() {
                    let msg = match ev {
                        SessionEvent::Connected { player } => format!("P{} connected", player + 1),
                        SessionEvent::Disconnected { player } => format!("P{} left", player + 1),
                        SessionEvent::Desync { frame, .. } => format!("Desync at frame {frame}"),
                    };
                    eprintln!("[stickfell] {msg}");
                    m.log.push(msg);
                }
                let n = s.game().num_players;
                let everyone_left = n > 1 && (0..n).filter(|&p| p != m.local).all(|p| s.is_disconnected(p));
                if everyone_left && m.ended.is_none() {
                    m.ended = Some(240);
                }
                if let Some(t) = &mut m.ended {
                    *t = t.saturating_sub(1);
                    if *t == 0 {
                        self.screen = menu(Some("Everyone else left the match".into()));
                        return;
                    }
                }
                m.prev = draw::snapshot(s.game());
                // Small leads are evened out by ticking a bit slower (see
                // `tick_rate`); only a big lead skips a tick outright.
                if s.frames_ahead() < 3.0 || !s.should_wait() {
                    let g = s.game();
                    let neck = g.fighters[m.local].pos(&g.world, NECK);
                    let mouse_world = self.camera.screen_to_world(self.mouse, self.screen_size);
                    let local = if self.bot_local {
                        bot::bot_input(g, m.local)
                    } else if prompt_open {
                        // the prompt has the mouse: stand still, don't swing at it
                        input::encode(0, g.fighters[m.local].aim)
                    } else {
                        input::read_local(input, mouse_world - neck)
                    };
                    let t0 = std::time::Instant::now();
                    let advanced = matches!(s.advance(local), stickfell_net::AdvanceResult::Advanced);
                    m.perf.add_tick(t0.elapsed().as_micros() as u64);
                    // only the newest frame's sounds: frames re-run by a
                    // rollback already made their noise the first time
                    if let (true, Some(a)) = (advanced, &mut self.audio) {
                        a.play_frame(&s.game().events, m.cam, &speakers(s.game()));
                    }
                    // voice lines wait until their frame is final for everyone
                    // (so everyone hears the same ones): the newest state's
                    // frame count is ahead of the session's by a fixed offset
                    if let Some(a) = &mut self.audio {
                        let g = s.game();
                        let through = s.final_frame() + 1 + (g.frame as i32 - s.frame());
                        a.speak(&g.voice_log, through.max(0) as u32, m.cam);
                    }
                }
                let ahead = s.frames_ahead();
                m.perf.report(&s.stats, ahead);
            }
        }
        if let Some(a) = action {
            self.do_action(a);
        }
    }

    fn draw(&mut self, frame: &mut Frame, screen: Vec2, alpha: f32, dt: f32) {
        if let Some(l) = &mut self.loading {
            if l.draw(frame, screen, dt, &self.levels, &mut self.audio) {
                self.loading = None;
                self.fade = loading::FADE;
                if let Some(a) = &mut self.audio {
                    a.volume = if self.muted { 0.0 } else { audio::MASTER_VOLUME };
                }
            }
            return;
        }
        // the crosshair and button hovers follow the cursor every frame, not
        // just every tick
        self.mouse = frame.mouse;
        self.buttons.clear();
        let k = (screen.y / 720.0).clamp(0.6, 3.0);
        let cx = screen.x * 0.5;
        match &mut self.screen {
            Screen::Match(m) => {
                let t0 = std::time::Instant::now();
                let g = &draw::interpolate(m.session.game(), &m.prev, alpha);
                // blade locks, finishers, slow-mo: swoop in on the action
                let shot = draw::cinematic(g, m.local);
                let want_h = shot.map_or(CAMERA_HEIGHT, |s| s.1);
                m.cam_h += (want_h - m.cam_h) * (1.0 - (-dt * 7.0).exp());
                let target = shot.map_or_else(|| draw::camera_target(g, Some(m.local)), |s| s.0);
                draw::follow_camera(&mut m.cam, target, g, screen, !m.cam_ready, dt, m.cam_h);
                m.cam_ready = true;
                let mut hud = Hud { local: Some(m.local), mouse: Some(self.mouse), trail: frame.mouse_trail.clone(), show_help: self.show_help, zoom: m.cam_h, talk: self.audio.as_ref().map_or([None; 4], |a| a.talking()), ..Default::default() };
                if m.session.state() == SessionState::Syncing {
                    hud.banner = Some(("Connecting to players...".into(), Color::WHITE));
                }
                if m.ended.is_some() {
                    hud.banner = Some(("Everyone else left".into(), Color::hex(0xffb040)));
                }
                hud.lines.push((format!("Online  frame {}", m.session.frame()), Color::hex(0x9fd0ff)));
                if !m.perf.line.is_empty() {
                    hud.lines.push((m.perf.line.clone(), Color::hex(0x9fd0ff)));
                }
                for l in m.log.iter().rev().take(2) {
                    hud.lines.push((l.clone(), Color::hex(0xc0c0c0)));
                }
                let lv = &self.levels[g.level_index];
                draw::draw_game(g, lv, frame, screen, m.cam, &hud);
                self.camera = frame.camera;
                m.perf.add_draw(t0.elapsed().as_micros() as u64, dt);
            }
            Screen::Practice(p) => {
                let g = &draw::interpolate(&p.game, &p.prev, alpha);
                let shot = draw::cinematic(g, 0);
                let want_h = shot.map_or(CAMERA_HEIGHT, |s| s.1);
                p.cam_h += (want_h - p.cam_h) * (1.0 - (-dt * 7.0).exp());
                let target = shot.map_or_else(|| draw::camera_target(g, Some(0)), |s| s.0);
                draw::follow_camera(&mut p.cam, target, g, screen, !p.cam_ready, dt, p.cam_h);
                p.cam_ready = true;
                let mut hud = Hud { local: Some(0), mouse: Some(self.mouse), trail: frame.mouse_trail.clone(), show_help: self.show_help, zoom: p.cam_h, talk: self.audio.as_ref().map_or([None; 4], |a| a.talking()), ..Default::default() };
                hud.lines.push(("Practice  -  R resets the yard  -  Esc leaves".into(), Color::hex(0xffd75a)));
                let lv = &self.levels[g.level_index];
                draw::draw_game(g, lv, frame, screen, p.cam, &hud);
                self.camera = frame.camera;
            }
            Screen::Menu { .. } | Screen::Lobby(_) => {
                self.demo.draw(&self.levels, frame, screen, alpha, dt);
                title_c(frame, screen, screen.y * 0.12, 12.0 * k, "STICKFELL", Color::hex(0xe8484f));
                title_c(frame, screen, screen.y * 0.12 + 96.0 * k, 3.2 * k, "LAST LIMB STANDING", Color::hex(0xc8ccd6));
            }
        }

        // menu widgets (needs &mut self for buttons)
        let screen_ref = std::mem::replace(&mut self.screen, menu(None));
        let y0 = screen.y * 0.42;
        let bw = 320.0 * k;
        match &screen_ref {
            Screen::Menu { code, entering, error } => {
                if *entering {
                    text_c(frame, screen, y0, 3.0 * k, "Enter room code", Color::WHITE);
                    let mut shown = code.clone();
                    while shown.len() < 4 {
                        shown.push('_');
                    }
                    let spaced: String = shown.chars().flat_map(|c| [c, ' ']).collect();
                    title_c(frame, screen, y0 + 40.0 * k, 8.0 * k, spaced.trim_end(), Color::hex(0xffd75a));
                    self.button(frame, cx, y0 + 130.0 * k, bw, "Join", Action::ConfirmJoin, k, code.len() == 4);
                    self.button(frame, cx, y0 + 186.0 * k, bw, "Back", Action::Cancel, k, true);
                } else {
                    self.button(frame, cx, y0, bw, "Host game", Action::Host, k, true);
                    self.button(frame, cx, y0 + 58.0 * k, bw, "Join game", Action::Join, k, true);
                    self.button(frame, cx, y0 + 116.0 * k, bw, "Practice", Action::Practice, k, true);
                    self.button(frame, cx, y0 + 174.0 * k, bw, "Art test level", Action::ArtTest, k, true);
                    self.button(frame, cx, y0 + 232.0 * k, bw, "Quit", Action::Quit, k, true);
                }
                if let Some(e) = error {
                    text_c(frame, screen, y0 - 44.0 * k, 2.2 * k, e, Color::hex(0xff6060));
                }
                // (the build number comes from the release workflow; a PC build says so)
                let build = option_env!("STICKFELL_BUILD").filter(|b| !b.is_empty()).map_or("Built on a PC".to_owned(), |b| format!("Build {b}"));
                text_c(frame, screen, screen.y - 40.0 * k, 1.8 * k, &format!("{build}   Relay {}", self.relay), Color::hex(0x8a90a0));
            }
            Screen::Lobby(c) => match c.state() {
                LobbyState::InRoom { code, players, max, .. } => {
                    text_c(frame, screen, y0 - 20.0 * k, 2.6 * k, "Room code", Color::hex(0xc8ccd6));
                    let spaced: String = code.chars().flat_map(|c| [c, ' ']).collect();
                    title_c(frame, screen, y0 + 14.0 * k, 9.0 * k, spaced.trim_end(), Color::hex(0xffd75a));
                    text_c(frame, screen, y0 + 92.0 * k, 2.6 * k, &format!("Players {players}/{max}"), Color::WHITE);
                    for i in 0..*players {
                        let x = cx + (i as f32 - (*players as f32 - 1.0) * 0.5) * 40.0 * k;
                        frame.ui_circle(Vec2::new(x, y0 + 132.0 * k), 12.0 * k, Color::hex(draw::PLAYER_COLORS[i % 4]));
                    }
                    if c.is_host() {
                        self.button(frame, cx, y0 + 170.0 * k, bw, "Start match", Action::Start, k, *players >= 2);
                        if *players < 2 {
                            text_c(frame, screen, y0 + 226.0 * k, 1.8 * k, "Share the code - you need 2 or more players", Color::hex(0x8a90a0));
                        }
                    } else {
                        text_c(frame, screen, y0 + 180.0 * k, 2.4 * k, "Waiting for the host to start", Color::hex(0xc8ccd6));
                    }
                    self.button(frame, cx, y0 + 250.0 * k, bw, "Leave", Action::Leave, k, true);
                }
                _ => {
                    text_c(frame, screen, y0 + 30.0 * k, 3.0 * k, "Contacting the relay...", Color::WHITE);
                    self.button(frame, cx, y0 + 120.0 * k, bw, "Cancel", Action::Leave, k, true);
                }
            },
            Screen::Match(_) | Screen::Practice(_) => {}
        }
        self.screen = screen_ref;

        if let Some(c) = self.confirm {
            frame.ui_rect(Vec2::ZERO, screen, 0.0, Color::BLACK.alpha(0.6));
            let (title, yes) = match c {
                Confirm::Quit => ("QUIT STICKFELL?", "Quit"),
                Confirm::LeaveMatch => ("LEAVE THE MATCH?", "Leave"),
                Confirm::LeavePractice => ("LEAVE PRACTICE?", "Leave"),
            };
            let y = screen.y * 0.4;
            title_c(frame, screen, y, 4.0 * k, title, Color::WHITE);
            self.button(frame, cx, y + 70.0 * k, bw, yes, Action::ConfirmYes, k, true);
            self.button(frame, cx, y + 128.0 * k, bw, "Stay", Action::ConfirmNo, k, true);
            text_c(frame, screen, y + 192.0 * k, 1.8 * k, "Enter confirms - Esc goes back", Color::hex(0x8a90a0));
        }
        if self.fade > 0.0 {
            frame.ui_rect(Vec2::ZERO, screen, 0.0, Color::hex(0x10121a).alpha(self.fade / loading::FADE));
            self.fade = (self.fade - dt).max(0.0);
        }
    }

    fn wants_exit(&self) -> bool {
        self.quit
    }

    fn cursor_visible(&self) -> bool {
        !matches!(self.screen, Screen::Match(_) | Screen::Practice(_)) || self.confirm.is_some()
    }

    /// In a fight the mouse stays inside the window (menus and the "leave?"
    /// prompt let it go).
    fn cursor_captured(&self) -> bool {
        !self.cursor_visible()
    }

    fn tick_rate(&self) -> f64 {
        match &self.screen {
            // running ahead of the other players: slow down slightly until
            // they catch up (smoother than skipping a tick)
            Screen::Match(m) => match m.session.frames_ahead() {
                a if a > 1.0 => 0.94,
                a if a > 0.4 => 0.97,
                _ => 1.0,
            },
            _ => 1.0,
        }
    }
}

/// Headless spectator used for screenshots.
struct Spectate {
    game: GameState,
    follow: usize,
    cam: Vec2,
    levels: Vec<LevelDef>,
}

impl Game for Spectate {
    fn tick(&mut self, _input: &Input) {
        let inputs: Vec<u32> = (0..self.game.num_players).map(|p| bot::bot_input(&self.game, p)).collect();
        self.game.step(&inputs);
    }
    fn draw(&mut self, frame: &mut Frame, screen: Vec2, _alpha: f32, _dt: f32) {
        let (target, h) = draw::cinematic(&self.game, self.follow).unwrap_or_else(|| (draw::camera_target(&self.game, Some(self.follow)), CAMERA_HEIGHT));
        draw::follow_camera(&mut self.cam, target, &self.game, screen, true, 1.0, h);
        let lv = &self.levels[self.game.level_index];
        draw::draw_game(&self.game, lv, frame, screen, self.cam, &Hud { local: Some(self.follow), zoom: h, ..Default::default() });
    }
}

struct Args {
    relay: String,
    players: usize,
    level: usize,
    net: bool,
    player: usize,
    bind: Option<SocketAddr>,
    peers: Vec<SocketAddr>,
    delay: i32,
    screenshot: Option<String>,
    /// Render benchmark: frames to time.
    bench: Option<u32>,
    time_startup: bool,
    size: (u32, u32),
    ticks: u32,
    verify: Option<u32>,
    follow: usize,
    seed: u32,
    host: bool,
    join: Option<String>,
    autostart: Option<usize>,
    bot: bool,
    menu_shot: bool,
    loading_shot: bool,
}

/// Relay address from `relay.txt` next to the exe (first line that isn't
/// blank or a `#` comment), so a shared build needs no command line. A bare
/// host gets the default port.
fn relay_from_file() -> Option<String> {
    let path = std::env::current_exe().ok()?.with_file_name("relay.txt");
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
    if line.len() > 255 {
        return None;
    }
    Some(if line.contains(':') { line.to_string() } else { format!("{line}:{}", stickfell_net::DEFAULT_RELAY_PORT) })
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        relay: relay_from_file().unwrap_or_else(|| DEFAULT_RELAY.into()),
        players: 2,
        level: 0,
        net: false,
        player: 0,
        bind: None,
        peers: Vec::new(),
        delay: 2,
        screenshot: None,
        bench: None,
        time_startup: false,
        size: (1280, 720),
        ticks: 600,
        verify: None,
        follow: 0,
        seed: 0x5EED,
        host: false,
        join: None,
        autostart: None,
        bot: false,
        menu_shot: false,
        loading_shot: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        let num = |s: String| s.parse::<u32>().map_err(|e| format!("bad number {s}: {e}"));
        match arg.as_str() {
            "--relay" => a.relay = val("--relay")?,
            "--players" => a.players = num(val("--players")?)? as usize,
            "--level" => a.level = num(val("--level")?)? as usize,
            "--seed" => a.seed = num(val("--seed")?)?,
            "--net" => a.net = true,
            "--player" => a.player = num(val("--player")?)? as usize,
            "--bind" => a.bind = Some(val("--bind")?.parse().map_err(|e| format!("bad --bind: {e}"))?),
            "--peer" => a.peers.push(val("--peer")?.parse().map_err(|e| format!("bad --peer: {e}"))?),
            "--delay" => a.delay = num(val("--delay")?)? as i32,
            "--screenshot" => a.screenshot = Some(val("--screenshot")?),
            "--bench-render" => a.bench = Some(num(val("--bench-render")?)?),
            "--time-startup" => a.time_startup = true,
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or(format!("bad --size {v} (want WxH)"))?;
                a.size = (num(w.into())?.clamp(64, 8192), num(h.into())?.clamp(64, 8192));
            }
            "--ticks" => a.ticks = num(val("--ticks")?)?,
            "--verify" => a.verify = Some(num(val("--verify")?)?),
            "--follow" => a.follow = num(val("--follow")?)? as usize,
            "--host" => a.host = true,
            "--join" => a.join = Some(val("--join")?),
            "--autostart" => a.autostart = Some(num(val("--autostart")?)? as usize),
            "--bot" => a.bot = true,
            "--menu" => a.menu_shot = true,
            "--loading" => a.loading_shot = true,
            "-h" | "--help" => {
                let doc: Vec<&str> = include_str!("main.rs").lines().take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!")).collect();
                println!("{}", doc.join("\n"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other} (try --help)")),
        }
    }
    a.players = a.players.clamp(1, sim::MAX_PLAYERS);
    if a.net {
        a.players = a.players.max(a.peers.len() + 1);
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("stickfell: {e}");
            return ExitCode::from(2);
        }
    };

    if let Some(frames) = args.verify {
        let mut s = SyncTestSession::new(GameState::new(args.players, args.level, args.seed), 7);
        for _ in 0..frames {
            let inputs: Vec<u32> = (0..args.players).map(|p| bot::bot_input(&s.game, p)).collect();
            if let Err(e) = s.advance(&inputs) {
                eprintln!("stickfell: {e}");
                return ExitCode::FAILURE;
            }
        }
        let g = &s.game;
        let severed: u32 = g.fighters.iter().map(|f| f.severed.count_ones()).sum();
        println!(
            "verify ok: {frames} frames, round {}, scores {:?}, weapons {}, decals {}, severed parts now {severed}, checksum {:016x}",
            g.round,
            g.scores,
            g.weapons.iter().filter(|w| w.active).count(),
            g.decals.len(),
            g.checksum()
        );
        return ExitCode::SUCCESS;
    }

    if let (Some(path), true) = (&args.screenshot, args.menu_shot) {
        let mut app = App::new(menu(None), args.relay.clone());
        if let Some(code) = &args.join {
            app.screen = Screen::Menu { code: code.to_uppercase(), entering: true, error: None };
        }
        if args.loading_shot {
            app.loading = Some(Box::new(loading::Loading::start()));
        }
        return match stickfell_app::capture_headless(&mut app, args.ticks, (1280, 720), path.as_ref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("stickfell: screenshot failed: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if args.time_startup {
        let t = std::time::Instant::now();
        let sound = audio::Audio::new();
        println!("audio {:.1} ms ({})", t.elapsed().as_secs_f64() * 1e3, if sound.is_some() { "device open" } else { "no device" });
        drop(sound);
        let t = std::time::Instant::now();
        let mut app = App::new(menu(None), args.relay.clone());
        println!("game state {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        return match stickfell_app::time_startup(&mut app, args.size, 8) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("stickfell: startup timing failed: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(n) = args.bench {
        let mut sp = Spectate {
            game: GameState::new(args.players, args.level, args.seed),
            follow: args.follow.min(args.players - 1),
            cam: Vec2::ZERO,
            levels: levels(),
        };
        return match stickfell_app::bench_headless(&mut sp, args.ticks, args.size, n) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("stickfell: bench failed: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(path) = &args.screenshot {
        let mut sp = Spectate {
            game: GameState::new(args.players, args.level, args.seed),
            follow: args.follow.min(args.players - 1),
            cam: Vec2::ZERO,
            levels: levels(),
        };
        return match stickfell_app::capture_headless(&mut sp, args.ticks, args.size, path.as_ref()) {
            Ok(()) => {
                println!("wrote {path}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("stickfell: screenshot failed: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let mut app = App::new(menu(None), args.relay.clone());
    app.loading = Some(Box::new(loading::Loading::start()));
    app.autostart = args.autostart;
    app.bot_local = args.bot;
    if args.host {
        app.do_action(Action::Host);
    } else if let Some(code) = &args.join {
        app.screen = Screen::Menu { code: code.to_uppercase(), entering: true, error: None };
        app.do_action(Action::ConfirmJoin);
    }
    if args.net {
        if args.player >= args.players {
            eprintln!("stickfell: --player must be < number of players");
            return ExitCode::from(2);
        }
        let bind = args.bind.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 7000 + args.player as u16)));
        let mut it = args.peers.iter();
        let peers = (0..args.players).map(|p| if p == args.player { None } else { it.next().copied() }).collect();
        let transport = match UdpTransport::bind(bind, peers) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("stickfell: could not bind {bind}: {e}");
                return ExitCode::FAILURE;
            }
        };
        eprintln!("[stickfell] direct online as P{} on {bind}", args.player + 1);
        app.start_match(Box::new(transport), args.players, args.player, args.seed, args.delay);
    }
    let cfg = AppConfig {
        title: "Stickfell: Last Limb Standing".into(),
        window_icon: Some(include_bytes!("../assets/icon/window-32.png")),
        taskbar_icon: Some(include_bytes!("../assets/icon/window-64.png")),
        ..Default::default()
    };
    match stickfell_app::run(cfg, app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("stickfell: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Where each fighter's voice comes from, and whether they're still alive to
/// use it (for steering voice lines).
fn speakers(g: &sim::GameState) -> Vec<audio::Speaker> {
    g.fighters.iter().map(|f| audio::Speaker { mouth: f.pos(&g.world, fighter::HEAD), alive: f.alive, choking: f.choking }).collect()
}
