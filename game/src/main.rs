//! Stickfell: Last Limb Standing — an online physics ragdoll brawler.
//!
//! ```text
//! stickfell [--relay HOST:PORT]     play online (host a room or join one by code);
//!                                  without --relay, reads relay.txt next to the exe
//!
//! dev tools:
//! stickfell --host [--autostart 2] | --join ABCD   skip the menu
//! (--ai N adds N AI fighters, --teams N plays N-a-side, to the modes below)
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
mod liquid;
mod lobby;
mod loading;
mod sfx;
mod sim;
mod steam;
mod stepper;
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
    /// Host, in the room: the next mode (free for all, 2v2, ...) or AI setting.
    CycleMode,
    CycleAi,
    Leave,
    Quit,
    Practice,
    /// Practice on the level built from the art kit.
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

/// The network link a match plays over (it goes with the session to the
/// sim thread).
type Link = Box<dyn Transport + Send>;

/// Whether the lobby's AI button works (off for now: AI fighters need more
/// work; `--ai` and the tests still use them).
const AI_IN_LOBBIES: bool = false;

/// Nobody on the other end: a host alone in the room with AI fighters plays
/// the match right here (the relay only starts rooms of two or more).
struct Alone;

impl Transport for Alone {
    fn send_to(&mut self, _player: usize, _data: &[u8]) {}
    fn recv(&mut self) -> Option<Vec<u8>> {
        None
    }
}
type Session = P2PSession<GameState, Link>;

struct Match {
    perf: Perf,
    /// The session (rollback and all), stepped on the sim thread.
    sim: stepper::Stepper<Session, NetReport>,
    /// What it last reported: the newest state (drawn, heard), and the
    /// particle positions before it (render interpolation), when it came in.
    game: GameState,
    prev: Vec<Vec2>,
    arrived: std::time::Instant,
    net: NetReport,
    local: usize,
    cam: Vec2,
    cam_ready: bool,
    /// Camera view height (zooms in for blade locks).
    cam_h: f32,
    log: Vec<String>,
    ended: Option<u32>,
}

/// How a match's session stands after a tick on the sim thread.
#[derive(Default)]
struct NetReport {
    /// The newest state, if the tick moved it on.
    game: Option<GameState>,
    /// Whether it stepped (it doesn't while it waits for the others), and
    /// how long that took.
    stepped: Option<u64>,
    events: Vec<SessionEvent>,
    disconnected: Vec<bool>,
    frames_ahead: f32,
    syncing: bool,
    frame: i32,
    final_frame: i32,
    stats: stickfell_net::NetStats,
}

impl Match {
    /// One tick of the session, on the sim thread: hear from the others,
    /// and step with `local` (unless it's too far ahead of them).
    fn tick_job(local: u32) -> impl FnOnce(&mut Session) -> NetReport + Send {
        move |s| {
            s.poll();
            let events = s.drain_events();
            // Small leads are evened out by ticking a bit slower (see
            // `tick_rate`); only a big lead skips a tick outright.
            let (mut game, mut stepped) = (None, None);
            if s.frames_ahead() < 3.0 || !s.should_wait() {
                let t0 = std::time::Instant::now();
                if matches!(s.advance(local), stickfell_net::AdvanceResult::Advanced) {
                    game = Some(s.game().clone());
                }
                stepped = Some(t0.elapsed().as_micros() as u64);
            }
            // (the players on the other end: the AI fighters after them are
            // played in the sim, nobody's connection)
            let n = s.game().humans;
            NetReport {
                game,
                stepped,
                events,
                disconnected: (0..n).map(|p| s.is_disconnected(p)).collect(),
                frames_ahead: s.frames_ahead(),
                syncing: s.state() == SessionState::Syncing,
                frame: s.frame(),
                final_frame: s.final_frame(),
                stats: s.stats,
            }
        }
    }

    /// Takes in whatever the sim thread has finished: the new states (and
    /// their sounds), what happened on the network.
    fn take(&mut self, audio: &mut Option<audio::Audio>) {
        for mut r in self.sim.finished() {
            for ev in std::mem::take(&mut r.events) {
                let msg = match ev {
                    SessionEvent::Connected { player } => format!("P{} connected", player + 1),
                    SessionEvent::Disconnected { player } => format!("P{} left", player + 1),
                    SessionEvent::Desync { frame, .. } => format!("Desync at frame {frame}"),
                };
                eprintln!("[stickfell] {msg}");
                self.log.push(msg);
            }
            if let Some(us) = r.stepped {
                self.perf.add_tick(us);
            }
            if let Some(g) = r.game.take() {
                self.prev = draw::snapshot(&self.game);
                self.game = g;
                self.arrived = std::time::Instant::now();
                // the sounds of the last few frames not heard yet: a frame
                // re-run by a rollback can have new ones (someone's shot)
                if let Some(a) = audio {
                    a.play_new(&self.game.sfx_log, self.cam, &speakers(&self.game));
                    a.surroundings(&surroundings(&self.game, self.local, self.cam), self.cam);
                }
            }
            // voice lines wait until their frame is final for everyone (so
            // everyone hears the same ones): the newest state's frame count
            // is ahead of the session's by a fixed offset
            if let (Some(_), Some(a)) = (r.stepped, audio.as_mut()) {
                let through = r.final_frame + 1 + (self.game.frame as i32 - r.frame);
                a.speak(&self.game.voice_log, through.max(0) as u32, self.cam);
            }
            self.net = r;
        }
    }
}

/// In a room, waiting for the match: the relay connection and the match
/// settings (the host's, or as heard from it).
struct Room {
    client: LobbyClient,
    sync: lobby::RoomSync,
    /// When the settings last went round (the host sends them every 250 ms).
    sent: Option<std::time::Instant>,
}

impl Room {
    fn new(client: LobbyClient) -> Self {
        let sync = if client.is_host() { lobby::RoomSync::host() } else { lobby::RoomSync::default() };
        Self { client, sync, sent: None }
    }

    /// Settings round the room: the host sends them out, everyone else
    /// answers what they got.
    fn exchange(&mut self, players: usize) {
        let hosting = self.client.is_host();
        for (from, pkt) in self.client.take_data() {
            if let Some(back) = self.sync.receive(from, &pkt, hosting) {
                self.client.send_data(0, &back);
            }
        }
        if hosting && self.sent.is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(250)) {
            for (slot, pkt) in self.sync.outgoing(players) {
                self.client.send_data(slot, &pkt);
            }
            self.sent = Some(std::time::Instant::now());
        }
    }

    /// Whether the host can start: enough players for the mode, and
    /// everyone has the settings.
    /// Two or more in the room, or one with AI fighters to play against.
    fn can_start(&self, players: usize) -> bool {
        let enough = players >= 2 || (players == 1 && self.sync.rules.bots(1) > 0);
        enough && self.sync.rules.problem(players).is_none() && self.sync.everyone_has_them(players)
    }
}

enum Screen {
    Menu { code: String, entering: bool, error: Option<String> },
    Lobby(Box<Room>),
    Match(Box<Match>),
    Practice(Box<Practice>),
}

/// Single-player practice yard: you against harmless dummies, no network.
struct Practice {
    level: usize,
    /// The yard, stepped on the sim thread; each step hands back the new
    /// state (and whether someone was moved across it: not blended).
    sim: stepper::Stepper<GameState, (GameState, bool)>,
    /// The newest state (drawn, heard), the particle positions before it
    /// (render interpolation), and when it came in.
    game: GameState,
    prev: Vec<Vec2>,
    arrived: std::time::Instant,
    /// B pressed: a dummy brought with the next step.
    dummy: bool,
    cam: Vec2,
    cam_h: f32,
    cam_ready: bool,
}

impl Practice {
    fn new(level: usize) -> Self {
        let game = GameState::new(4, level, 0x7EA1);
        let sim = stepper::Stepper::new(game.clone());
        Self { level, sim, prev: draw::snapshot(&game), game, arrived: std::time::Instant::now(), dummy: false, cam: Vec2::ZERO, cam_h: CAMERA_HEIGHT, cam_ready: false }
    }

    /// Takes in whatever steps the sim thread has finished (and plays
    /// their sounds).
    fn take(&mut self, audio: &mut Option<audio::Audio>) {
        for (g, moved) in self.sim.finished() {
            self.prev = draw::snapshot(if moved { &g } else { &self.game });
            self.game = g;
            self.arrived = std::time::Instant::now();
            if let Some(a) = audio {
                a.play_frame(&self.game.events, self.cam, &speakers(&self.game));
                a.surroundings(&surroundings(&self.game, 0, self.cam), self.cam);
                // (no one else to wait for: every frame is final)
                a.speak(&self.game.voice_log, self.game.frame, self.cam);
            }
        }
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
        let inputs: Vec<u32> = (0..self.game.fighters.len()).map(|p| bot::bot_input(&self.game, p)).collect();
        self.game.step(&inputs);
    }
    fn draw(&mut self, levels: &[LevelDef], frame: &mut Frame, screen: Vec2, alpha: f32, dt: f32) {
        let g = draw::interpolate(&self.game, &self.prev, alpha);
        let follow = (g.frame / 600) as usize % g.fighters.len().max(1);
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
    mouse: Vec2,
    camera: Camera2D,
    screen_size: Vec2,
    autostart: Option<usize>,
    /// The settings `--host --autostart` starts with (`--ai`, `--teams`).
    autostart_rules: Option<lobby::Rules>,
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
            mouse: Vec2::ZERO,
            camera: Camera2D { center: Vec2::ZERO, height: CAMERA_HEIGHT },
            screen_size: Vec2::new(1280.0, 720.0),
            autostart: None,
            autostart_rules: None,
            bot_local: false,
            announced: false,
            confirm: None,
            audio: None,
            muted: false,
            loading: None,
            fade: 0.0,
        }
    }

    /// The tick rate (share of 60 a second) for a match this far ahead of
    /// the other players: running ahead, slow down slightly until they
    /// catch up (smoother than skipping a tick).
    fn match_rate(ahead: f32) -> f64 {
        match ahead {
            a if a > 1.0 => 0.94,
            a if a > 0.4 => 0.97,
            _ => 1.0,
        }
    }

    fn start_match(&mut self, transport: Link, num_players: usize, local: usize, seed: u32, delay: i32, rules: lobby::Rules) {
        // (the AI fighters come after the players, and are played in the sim)
        let fighters = num_players + rules.bots(num_players);
        let game = GameState::new_match(fighters, num_players, rules.teams, level::first_arena(seed, fighters), seed);
        let mut cfg = SessionConfig::new(num_players, local);
        cfg.input_delay = delay;
        let session = P2PSession::new(game.clone(), transport, cfg);
        let net = NetReport { syncing: session.state() == SessionState::Syncing, ..Default::default() };
        self.screen = Screen::Match(Box::new(Match {
            perf: Perf::default(),
            sim: stepper::Stepper::new(session),
            prev: draw::snapshot(&game),
            game,
            arrived: std::time::Instant::now(),
            net,
            local,
            cam: Vec2::ZERO,
            cam_ready: false,
            cam_h: CAMERA_HEIGHT,
            log: Vec::new(),
            ended: None,
        }));
    }

    fn do_action(&mut self, a: Action) {
        match a {
            Action::Quit => self.quit = true,
            Action::Practice => self.screen = Screen::Practice(Box::new(Practice::new(level::TRAINING))),
            Action::ConfirmYes => match self.confirm.take() {
                Some(Confirm::Quit) => self.quit = true,
                Some(Confirm::LeaveMatch) => self.screen = menu(None),
                Some(Confirm::LeavePractice) => self.screen = menu(None),
                None => {}
            },
            Action::ConfirmNo => self.confirm = None,
            Action::Host => match LobbyClient::host(&self.relay, sim::MAX_PLAYERS as u8) {
                Ok(c) => self.screen = Screen::Lobby(Box::new(Room::new(c))),
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
                    Ok(c) => self.screen = Screen::Lobby(Box::new(Room::new(c))),
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
                let mut alone = None;
                if let Screen::Lobby(r) = &mut self.screen {
                    let players = match r.client.state() {
                        LobbyState::InRoom { players, .. } => *players,
                        _ => 0,
                    };
                    if players == 1 && r.can_start(1) {
                        r.client.leave();
                        alone = Some(r.sync.rules);
                    } else {
                        r.client.start();
                    }
                }
                if let Some(rules) = alone {
                    // (any seed: nobody else has to agree on it)
                    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
                    self.start_match(Box::new(Alone), 1, 0, seed, 0, rules);
                }
            }
            Action::CycleMode | Action::CycleAi => {
                if let Screen::Lobby(r) = &mut self.screen {
                    let players = match r.client.state() {
                        LobbyState::InRoom { players, .. } => *players,
                        _ => 1,
                    };
                    let rules = if a == Action::CycleMode { r.sync.rules.next_mode() } else { r.sync.rules.next_ai(players) };
                    r.sync.set(rules);
                }
            }
            Action::Leave => {
                if let Screen::Lobby(r) = &mut self.screen {
                    r.client.leave();
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
        // (shrunk to fit, if it has to be)
        let room = w - 24.0 * k;
        let full = Frame::text_width(label, 2.6 * k);
        let scale = if full > room { 2.6 * k * room / full } else { 2.6 * k };
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
        // (the sound cutting out: F9 when it's heard, or caught by itself,
        // writes what the mix was doing to %LOCALAPPDATA%\Stickfell\audio-report.txt)
        if let Some(a) = &mut self.audio {
            a.watch(input.pressed(KeyCode::F9));
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
            Screen::Lobby(room) => {
                self.demo.tick();
                if esc {
                    action = Some(Action::Leave);
                }
                let state = room.client.poll().clone();
                let mut go = false;
                if let LobbyState::InRoom { code, players, .. } = &state {
                    if !self.announced {
                        eprintln!("[stickfell] room code {code}");
                        self.announced = true;
                    }
                    room.exchange(*players);
                    if room.client.is_host() {
                        // (--autostart's settings, once: after that they're the host's to change)
                        if let Some(r) = self.autostart_rules.take() {
                            room.sync.set(r);
                        }
                        if self.autostart.is_some_and(|n| *players >= n) && room.can_start(*players) {
                            go = true;
                        }
                    }
                }
                if go {
                    self.do_action(Action::Start);
                    return;
                }
                match state {
                    LobbyState::Started { num_players, slot, seed } => {
                        let Screen::Lobby(room) = std::mem::replace(&mut self.screen, menu(None)) else { unreachable!() };
                        // (the host made sure everyone had its settings before starting)
                        if !room.sync.heard {
                            self.screen = menu(Some("Didn't get the match settings from the host".into()));
                            return;
                        }
                        let rules = room.sync.rules;
                        let transport: Link = Box::new(room.client.into_transport());
                        self.start_match(transport, num_players, slot, seed, 2, rules);
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
                // B: a dummy right in front of you, facing you
                if input.pressed(KeyCode::KeyB) && self.confirm.is_none() {
                    p.dummy = true;
                }
                p.take(&mut self.audio);
                // the next step, worked out on the sim thread (aimed from
                // where you were last seen); with the sim thread still busy
                // with the last two, the game waits a tick
                if p.sim.ready() {
                    let g = &p.game;
                    let neck = g.fighters[0].pos(&g.world, NECK);
                    let mouse_world = self.camera.screen_to_world(self.mouse, self.screen_size);
                    let me = if self.confirm.is_some() { input::encode(0, g.fighters[0].aim) } else { input::read_local(input, mouse_world - neck) };
                    let dummy = std::mem::take(&mut p.dummy);
                    p.sim.send(move |g| {
                        if dummy {
                            g.bring_dummy(0);
                        }
                        let mut inputs = vec![me];
                        inputs.extend((1..g.num_players).map(|i| bot::dummy_input(g, i)));
                        g.step(&inputs);
                        (g.clone(), dummy)
                    });
                }
            }
            Screen::Match(m) => {
                if esc {
                    self.confirm = Some(Confirm::LeaveMatch);
                }
                let prompt_open = self.confirm.is_some();
                m.take(&mut self.audio);
                let n = m.game.humans;
                let gone = &m.net.disconnected;
                let everyone_left = n > 1 && gone.len() == n && (0..n).filter(|&p| p != m.local).all(|p| gone[p]);
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
                // the next tick, on the sim thread (aimed from where you
                // were last seen); with it still busy with the last two, this
                // tick waits
                if m.sim.ready() {
                    let g = &m.game;
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
                    m.sim.send(Match::tick_job(local));
                }
                m.perf.report(&m.net.stats, m.net.frames_ahead);
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
                // (the newest state from the sim thread, as soon as it's in;
                // blended toward over a tick from when it came)
                m.take(&mut self.audio);
                let tick = 1.0 / (60.0 * Self::match_rate(m.net.frames_ahead));
                let g = &draw::interpolate(&m.game, &m.prev, stepper::blend(m.arrived, tick));
                // blade locks, finishers, slow-mo: swoop in on the action
                let shot = draw::cinematic(g, m.local);
                let want_h = shot.map_or(CAMERA_HEIGHT, |s| s.1);
                m.cam_h += (want_h - m.cam_h) * (1.0 - (-dt * 7.0).exp());
                let target = shot.map_or_else(|| draw::camera_target(g, Some(m.local)), |s| s.0);
                draw::follow_camera(&mut m.cam, target, g, screen, !m.cam_ready, dt, m.cam_h);
                m.cam_ready = true;
                let mut hud = Hud { local: Some(m.local), mouse: Some(self.mouse), zoom: m.cam_h, talk: self.audio.as_ref().map_or([None; sim::MAX_PLAYERS], |a| a.talking()), ..Default::default() };
                if m.net.syncing {
                    hud.banner = Some(("Connecting to players...".into(), Color::WHITE));
                }
                if m.ended.is_some() {
                    hud.banner = Some(("Everyone else left".into(), Color::hex(0xffb040)));
                }
                hud.lines.push((format!("Online  frame {}", m.net.frame), Color::hex(0x9fd0ff)));
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
                p.take(&mut self.audio);
                let g = &draw::interpolate(&p.game, &p.prev, stepper::blend(p.arrived, 1.0 / 60.0));
                let shot = draw::cinematic(g, 0);
                let want_h = shot.map_or(CAMERA_HEIGHT, |s| s.1);
                p.cam_h += (want_h - p.cam_h) * (1.0 - (-dt * 7.0).exp());
                let target = shot.map_or_else(|| draw::camera_target(g, Some(0)), |s| s.0);
                draw::follow_camera(&mut p.cam, target, g, screen, !p.cam_ready, dt, p.cam_h);
                p.cam_ready = true;
                let hud = Hud { local: Some(0), mouse: Some(self.mouse), zoom: p.cam_h, talk: self.audio.as_ref().map_or([None; sim::MAX_PLAYERS], |a| a.talking()), ..Default::default() };
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
                    self.button(frame, cx, y0 + 174.0 * k, bw, "Quit", Action::Quit, k, true);
                }
                if let Some(e) = error {
                    text_c(frame, screen, y0 - 44.0 * k, 2.2 * k, e, Color::hex(0xff6060));
                }
                // (the build number comes from the release workflow; a PC build says so)
                let build = option_env!("STICKFELL_BUILD").filter(|b| !b.is_empty()).map_or("Built on a PC".to_owned(), |b| format!("Build {b}"));
                text_c(frame, screen, screen.y - 40.0 * k, 1.8 * k, &format!("{build}   Relay {}", self.relay), Color::hex(0x8a90a0));
            }
            Screen::Lobby(r) => match r.client.state() {
                LobbyState::InRoom { code, players, max, .. } => {
                    let (players, rules) = (*players, r.sync.rules);
                    text_c(frame, screen, y0 - 20.0 * k, 2.6 * k, "Room code", Color::hex(0xc8ccd6));
                    let spaced: String = code.chars().flat_map(|c| [c, ' ']).collect();
                    title_c(frame, screen, y0 + 14.0 * k, 9.0 * k, spaced.trim_end(), Color::hex(0xffd75a));
                    text_c(frame, screen, y0 + 92.0 * k, 2.6 * k, &format!("Players {players}/{max}"), Color::WHITE);
                    // everyone who'll be in it: the players, then the AI
                    // (hollow), in their colours (by team, in teams)
                    let all = players + rules.bots(players);
                    for i in 0..all {
                        let x = cx + (i as f32 - (all as f32 - 1.0) * 0.5) * 40.0 * k;
                        let col = match rules.teams {
                            0 => draw::PLAYER_COLORS[i % draw::PLAYER_COLORS.len()],
                            _ => draw::TEAM_COLORS[i % 2][(i / 2) % 4],
                        };
                        let at = Vec2::new(x, y0 + 132.0 * k);
                        frame.ui_circle(at, 12.0 * k, Color::hex(col));
                        if i >= players {
                            frame.ui_circle(at, 7.0 * k, Color::hex(0x15171c));
                        }
                    }
                    let grey = Color::hex(0x8a90a0);
                    if r.client.is_host() {
                        let mode = format!("Mode: {}", rules.mode());
                        self.button(frame, cx, y0 + 166.0 * k, bw, &mode, Action::CycleMode, k, true);
                        // (AI fighters are off in lobbies for now: `AI_IN_LOBBIES`)
                        self.button(frame, cx, y0 + 216.0 * k, bw, &rules.ai_label(players), Action::CycleAi, k, AI_IN_LOBBIES);
                        let ready = r.can_start(players);
                        self.button(frame, cx, y0 + 272.0 * k, bw, "Start match", Action::Start, k, ready);
                        let note = if players < 2 && rules.bots(1) == 0 {
                            Some(if AI_IN_LOBBIES { "Share the code, or turn on AI to play alone" } else { "Share the code - you need 2 or more players" }.to_string())
                        } else if let Some(p) = rules.problem(players) {
                            Some(p)
                        } else if !ready {
                            Some("Sending everyone the settings...".to_string())
                        } else {
                            None
                        };
                        if let Some(n) = note {
                            text_c(frame, screen, y0 + 326.0 * k, 1.8 * k, &n, grey);
                        }
                        self.button(frame, cx, y0 + 348.0 * k, bw, "Leave", Action::Leave, k, true);
                    } else {
                        let what = if r.sync.heard { format!("{}   {}", rules.mode(), rules.ai_label(players)) } else { "Getting the settings...".to_string() };
                        text_c(frame, screen, y0 + 170.0 * k, 2.2 * k, &what, Color::WHITE);
                        text_c(frame, screen, y0 + 206.0 * k, 2.4 * k, "Waiting for the host to start", Color::hex(0xc8ccd6));
                        self.button(frame, cx, y0 + 250.0 * k, bw, "Leave", Action::Leave, k, true);
                    }
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
            Screen::Match(m) => Self::match_rate(m.net.frames_ahead),
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
        let inputs: Vec<u32> = (0..self.game.humans).map(|p| bot::bot_input(&self.game, p)).collect();
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
    /// AI fighters and team size (`--ai`, `--teams`).
    rules: lobby::Rules,
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
        rules: lobby::Rules::default(),
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
            "--ai" => a.rules.ai = num(val("--ai")?)?.min(sim::MAX_PLAYERS as u32 - 1) as u8,
            "--teams" => a.rules.teams = (num(val("--teams")?)?).clamp(2, 4) as u8,
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
        let fighters = args.players + args.rules.bots(args.players);
        let mut s = SyncTestSession::new(GameState::new_match(fighters, args.players, args.rules.teams, args.level, args.seed), 7);
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
            game: GameState::new_match(args.players + args.rules.bots(args.players), args.players, args.rules.teams, args.level, args.seed),
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
            game: GameState::new_match(args.players + args.rules.bots(args.players), args.players, args.rules.teams, args.level, args.seed),
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
    app.autostart_rules = args.autostart.map(|_| args.rules);
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
        app.start_match(Box::new(transport), args.players, args.player, args.seed, args.delay, args.rules);
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
/// What's around `listener` for the sound loops: the nearest calm water and
/// hot lava, whether fighter `me` has their head under water, and everyone
/// swimming.
fn surroundings(g: &sim::GameState, me: usize, listener: stickfell_physics::Vec2) -> audio::Surroundings {
    let mut s = audio::Surroundings::default();
    let (mut water, mut lava) = (f32::MAX, f32::MAX);
    for d in &g.fluid.drops {
        let l = (d.pos - listener).length_squared();
        if d.kind == liquid::WATER_DROP && l < water {
            water = l;
            s.water = Some(d.pos);
        } else if d.kind == liquid::LAVA_DROP && d.dye < liquid::COOLED_LAVA && l < lava {
            lava = l;
            s.lava = Some(d.pos);
        }
    }
    if let Some(f) = g.fighters.get(me) {
        let head = g.fluid.sample(f.pos(&g.world, fighter::HEAD));
        s.under = f.alive && head.amount[liquid::WATER_DROP as usize] > sim::HEAD_UNDER;
    }
    for (i, f) in g.fighters.iter().enumerate().take(sim::MAX_PLAYERS) {
        if f.swimming {
            s.swimming[i] = Some(f.pos(&g.world, fighter::PELVIS));
        }
    }
    s
}

fn speakers(g: &sim::GameState) -> Vec<audio::Speaker> {
    g.fighters.iter().map(|f| audio::Speaker { mouth: f.pos(&g.world, fighter::HEAD), alive: f.alive, choking: f.choking, burning: f.burning }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loops_hear_the_pool_the_lava_and_a_head_under() {
        let mut g = sim::GameState::new(4, level::TRAINING, 7);
        for _ in 0..20 {
            g.step(&[0, 0, 0, 0]);
        }
        let (p, l) = (level::YARD_POOL, level::YARD_LAVA);
        let pool = Vec2::new((p.0 + p.1) * 0.5, p.3);
        let lava = Vec2::new((l.0 + l.1) * 0.5, l.3);
        let s = surroundings(&g, 0, pool + Vec2::new(0.0, 2.0));
        let water = s.water.expect("water heard by the pool");
        assert!((water - pool).length() < 2.5 && water.x > p.0 && water.x < p.1, "the nearest water is the pool's ({water:?})");
        let s = surroundings(&g, 0, lava + Vec2::new(0.0, 2.0));
        let hot = s.lava.expect("lava heard by the lava");
        assert!(hot.x > l.0 && hot.x < l.1, "the nearest lava is the pit's ({hot:?})");
        assert!(!s.under && s.swimming.iter().all(|w| w.is_none()), "nobody's in anything yet");
        // fighter 0 held at the bottom of the pool
        let at = Vec2::new(pool.x, p.2 + 0.5);
        for _ in 0..30 {
            let head = g.fighters[0].pos(&g.world, fighter::HEAD);
            for id in g.fighters[0].particle_ids() {
                let q = &mut g.world.particles[id as usize];
                q.pos += at - head;
                q.prev = q.pos;
                q.vel = Vec2::ZERO;
            }
            g.step(&[0, 0, 0, 0]);
        }
        assert!(surroundings(&g, 0, pool).under, "head under: the drone");
        assert!(!surroundings(&g, 1, pool).under, "someone else's head under isn't yours");
    }
}

#[cfg(test)]
mod sim_thread_tests {
    use super::*;
    use stickfell_net::RollbackGame;

    /// Steps practice the way `App::tick` does: a job a tick on the sim
    /// thread, the results taken in as they come.
    fn run(p: &mut Practice, ticks: u32, buttons: u16) {
        let mut sent = 0;
        while sent < ticks {
            p.take(&mut None);
            if p.sim.ready() {
                let aim = Vec2::new(0.6, 0.1);
                let me = input::encode(buttons, aim);
                p.sim.send(move |g| {
                    let mut inputs = vec![me];
                    inputs.extend((1..g.num_players).map(|i| bot::dummy_input(g, i)));
                    g.step(&inputs);
                    (g.clone(), false)
                });
                sent += 1;
            } else {
                // (as the game would: off drawing, not spinning on a core)
                std::thread::sleep(std::time::Duration::from_micros(500));
            }
        }
        for (g, _) in p.sim.wait() {
            p.prev = draw::snapshot(&p.game);
            p.game = g;
        }
    }

    #[test]
    fn practice_on_the_sim_thread_comes_out_the_same() {
        let mut p = Practice::new(level::TRAINING);
        let mut same = p.game.clone();
        run(&mut p, 120, input::RIGHT);
        let me = input::encode(input::RIGHT, Vec2::new(0.6, 0.1));
        for _ in 0..120 {
            let mut inputs = vec![me];
            inputs.extend((1..same.num_players).map(|i| bot::dummy_input(&same, i)));
            same.step(&inputs);
        }
        assert_eq!(p.game.frame, same.frame, "every step came back");
        assert_eq!(p.game.checksum(), same.checksum(), "the same world as stepping it here");
    }

    #[test]
    #[ignore]
    fn measure_drawing_thread_flooded() {
        // the yard flooded to the cap, then: what's left on the drawing
        // thread each tick and each drawn frame
        let mut p = Practice::new(level::TRAINING);
        let cap: Option<u32> = std::env::var("FLOOD_CAP").ok().map(|c| c.parse().unwrap());
        if let Some(c) = cap {
            p.game.trap.def = Some(level::TrapDef { max_drops: c, ..p.game.trap.def.unwrap() });
        }
        let def = p.game.trap.def.unwrap();
        p.sim.send(move |g| {
            if let Some(c) = cap {
                g.trap.def = Some(level::TrapDef { max_drops: c, ..g.trap.def.unwrap() });
            }
            g.trap.open = true;
            g.trap.toggled = g.round_frame.max(1);
            (g.clone(), false)
        });
        p.sim.wait();
        let mut frames = 0;
        while (p.game.fluid.drops.len() as u32) < def.max_drops && frames < 60 * 60 {
            run(&mut p, 60, 0);
            frames += 60;
        }
        run(&mut p, 600, 0);
        eprintln!("{} drops", p.game.fluid.drops.len());
        // the sim thread's step, for comparison
        let t = std::time::Instant::now();
        run(&mut p, 120, 0);
        eprintln!("120 steps on the sim thread: {:.2} ms a step (wall)", t.elapsed().as_secs_f64() * 1000.0 / 120.0);
        let mut g = p.game.clone();
        let t = std::time::Instant::now();
        for _ in 0..120 {
            let inputs: Vec<u32> = (0..g.num_players).map(|i| if i == 0 { 0 } else { bot::dummy_input(&g, i) }).collect();
            g.step(&inputs);
        }
        eprintln!("the same 120 stepped right here: {:.2} ms a step", t.elapsed().as_secs_f64() * 1000.0 / 120.0);
        // drawing-thread work: taking in a state (it's moved, not copied), the
        // blend copy for drawing, and the snapshot for the next blend
        let n = 200;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = draw::interpolate(&p.game, &p.prev, 0.5);
        }
        let interp = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t = std::time::Instant::now();
        for _ in 0..n {
            p.prev = draw::snapshot(&p.game);
        }
        let snap = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = p.game.clone();
        }
        let clone = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("drawing thread: blend {interp:.3} ms a drawn frame, snapshot {snap:.3} ms a tick; (the copy the sim thread sends back: {clone:.3} ms, on its side)");
    }
}

#[cfg(test)]
mod lobby_shots {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The room screen, as the host sees it and as a guest does, with the
    /// longest labels the settings have (SHOT_DIR, default here).
    #[test]
    #[ignore]
    fn screenshot_lobby() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
        let mut server = stickfell_net::RelayServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                server.poll();
            }
        });
        let mut host = LobbyClient::host(&addr, 8).unwrap();
        let code = loop {
            if let LobbyState::InRoom { code, .. } = host.poll() {
                break code.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let mut guest = LobbyClient::join(&addr, &code).unwrap();
        while !matches!(guest.poll(), LobbyState::InRoom { players: 2, .. }) {
            host.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        while !matches!(host.poll(), LobbyState::InRoom { players: 2, .. }) {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut h = Room::new(host);
        let mut g = Room::new(guest);
        for (name, rules) in [("ffa", lobby::Rules { teams: 0, ai: 6 }), ("teams", lobby::Rules { teams: 2, ai: 1 }), ("short", lobby::Rules { teams: 4, ai: 0 })] {
            h.sync.set(rules);
            g.sync.rules = rules;
            g.sync.heard = true;
            for (who, room) in [("host", &mut h), ("guest", &mut g)] {
                let taken = std::mem::replace(room, Room::new(LobbyClient::host(&addr, 8).unwrap()));
                let mut app = App::new(Screen::Lobby(Box::new(taken)), addr.clone());
                stickfell_app::capture_headless(&mut app, 1, (1280, 720), format!("{dir}/lobby_{name}_{who}.png").as_ref()).unwrap();
                let Screen::Lobby(back) = std::mem::replace(&mut app.screen, menu(None)) else { panic!("left the room") };
                *room = *back;
            }
        }
        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
    }

    fn room_of(app: &App) -> &Room {
        match &app.screen {
            Screen::Lobby(r) => r,
            _ => panic!("not in the room"),
        }
    }

    /// The host changes the mode and AI with someone in the room: the
    /// settings stay as picked (a regression: `--autostart`'s defaults were
    /// put back every frame), reach the guest, and the match can start.
    #[test]
    fn the_hosts_settings_stick_and_reach_the_room() {
        let mut server = stickfell_net::RelayServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                server.poll();
            }
        });
        let mut host = LobbyClient::host(&addr, 8).unwrap();
        let code = loop {
            if let LobbyState::InRoom { code, .. } = host.poll() {
                break code.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let mut guest = LobbyClient::join(&addr, &code).unwrap();
        while !matches!(guest.poll(), LobbyState::InRoom { players: 2, .. }) {
            host.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut h = App::new(Screen::Lobby(Box::new(Room::new(host))), addr.clone());
        let mut g = App::new(Screen::Lobby(Box::new(Room::new(guest))), addr.clone());
        // (as `--host --autostart` would, for a count never reached: its
        // settings once, then the host's)
        h.autostart = Some(3);
        h.autostart_rules = Some(lobby::Rules::default());
        let input = Input::default();
        let want = lobby::Rules { teams: 2, ai: 1 };
        let mut frames = 0;
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(5) {
            h.tick(&input);
            g.tick(&input);
            frames += 1;
            if frames == 30 {
                h.do_action(Action::CycleMode);
                h.do_action(Action::CycleAi);
            }
            if frames > 30 {
                assert_eq!(room_of(&h).sync.rules, want, "frame {frames}: the host's pick stays");
                if room_of(&g).sync.rules == want && room_of(&h).can_start(2) {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(room_of(&g).sync.heard && room_of(&g).sync.rules == want, "the guest has them ({:?})", room_of(&g).sync.rules);
        assert!(room_of(&h).can_start(2), "and the host can start");
        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
    }

    /// Two players start a match through a relay with `rules`: how long
    /// until both are past "connecting" and playing (or what it's stuck on).
    fn start_online(rules: lobby::Rules) -> Result<f32, String> {
        let mut server = stickfell_net::RelayServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                server.poll();
            }
        });
        let mut host = LobbyClient::host(&addr, 8).unwrap();
        let code = loop {
            if let LobbyState::InRoom { code, .. } = host.poll() {
                break code.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let mut guest = LobbyClient::join(&addr, &code).unwrap();
        while !matches!(guest.poll(), LobbyState::InRoom { players: 2, .. }) {
            host.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut h = App::new(Screen::Lobby(Box::new(Room::new(host))), addr.clone());
        let mut g = App::new(Screen::Lobby(Box::new(Room::new(guest))), addr.clone());
        let input = Input::default();
        let started = std::time::Instant::now();
        let mut asked = false;
        let mut result = Err("timed out".to_string());
        while started.elapsed() < std::time::Duration::from_secs(12) {
            h.tick(&input);
            g.tick(&input);
            if !asked {
                if let Screen::Lobby(r) = &mut h.screen {
                    r.sync.set(rules);
                    if r.can_start(2) {
                        h.do_action(Action::Start);
                        asked = true;
                    }
                }
            }
            let playing = |a: &App| matches!(&a.screen, Screen::Match(m) if !m.net.syncing && m.net.frame > 30);
            if playing(&h) && playing(&g) {
                result = Ok(started.elapsed().as_secs_f32());
                break;
            }
            let what = |a: &App| match &a.screen {
                Screen::Lobby(_) => "lobby".to_string(),
                Screen::Match(m) => format!("match (syncing {}, frame {})", m.net.syncing, m.net.frame),
                Screen::Menu { error, .. } => format!("menu {error:?}"),
                _ => "other".to_string(),
            };
            result = Err(format!("host: {}, guest: {}", what(&h), what(&g)));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
        result
    }

    /// A host alone in the room: with AI fighters on, Start plays the match
    /// against them right away; without, there's no starting.
    #[test]
    fn a_host_alone_can_start_against_ai_fighters() {
        let mut server = stickfell_net::RelayServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                server.poll();
            }
        });
        let mut host = LobbyClient::host(&addr, 8).unwrap();
        while !matches!(host.poll(), LobbyState::InRoom { .. }) {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut h = App::new(Screen::Lobby(Box::new(Room::new(host))), addr.clone());
        let input = Input::default();
        for _ in 0..80 {
            h.tick(&input);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!room_of(&h).can_start(1), "alone with no AI: nothing to play");
        h.do_action(Action::CycleAi);
        h.do_action(Action::CycleAi);
        let started = std::time::Instant::now();
        let mut playing = None;
        while started.elapsed() < std::time::Duration::from_secs(8) {
            h.tick(&input);
            if let Screen::Lobby(r) = &h.screen {
                if r.can_start(1) {
                    h.do_action(Action::Start);
                }
            }
            if let Screen::Match(m) = &h.screen {
                if !m.net.syncing && m.net.frame > 60 {
                    playing = Some((started.elapsed().as_secs_f32(), m.game.fighters.len(), m.game.humans));
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
        eprintln!("alone with 2 AI: playing after {playing:?} (seconds, fighters, humans)");
        assert!(playing.is_some_and(|(_, f, h)| f == 3 && h == 1), "{playing:?}");
    }

    #[test]
    fn matches_with_ai_fighters_get_going_online() {
        for rules in [lobby::Rules::default(), lobby::Rules { teams: 0, ai: 2 }, lobby::Rules { teams: 2, ai: 1 }] {
            let r = start_online(rules);
            eprintln!("{rules:?}: {r:?}");
            assert!(r.is_ok(), "{rules:?}: {r:?}");
        }
    }
}
