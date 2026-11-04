//! Stickfell's launcher (players run it as Stickfell.exe). The first time, it installs itself for this Windows
//! account (see `install`: default %LOCALAPPDATA%\Programs\Stickfell, with shortcuts and an Apps entry); a copy run
//! from anywhere else opens the installed one. It keeps itself and the game up to date from the update service,
//! then starts the game: it asks for a tester key once (kept encrypted for this Windows account), updates itself if
//! there's a newer launcher, then installs or updates the game in Game\ in the install folder. It looks like a
//! game launcher: its own title bar (Play and Patch notes tabs), the key art, what's new, and a Play button docked
//! bottom right that shows the update's progress. Offline, it plays the build that's installed.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art;
mod install;
mod secret;
mod ui;
mod update;
mod view;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use stickfell_app::{AppConfig, CursorIcon, Game, Input, KeyCode, Mouse, WindowAction};
use stickfell_render::{Frame, Vec2};
use update::{Channel, Manifest, Problem};

/// How long a note under the Play button stays up (s).
const NOTICE_SECS: f32 = 5.0;

/// Where things stand (the update work runs on its own thread and moves this along).
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// First run: where to install, whether to put a shortcut on the desktop, an old game folder to move in, and
    /// why the last try failed.
    Setup { root: PathBuf, desktop: bool, legacy: Option<PathBuf>, error: Option<String> },
    /// `--uninstall` (from Windows' Apps list): asking, then done.
    Uninstall { root: PathBuf, done: bool, error: Option<String> },
    /// Waiting for a tester key (and why the last one didn't work).
    Key { error: Option<String> },
    Busy { what: String, progress: Option<Progress> },
    /// The game can start: the installed build, any warning, and whether the service was out of reach.
    Ready { build: Option<u32>, warning: Option<String>, offline: bool },
    Failed { message: String, can_play: bool },
    /// Closing: the new launcher or the game has started.
    Done,
}

/// A download under way: bytes so far, of the total, and bytes a second.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub speed: f64,
}

/// What the update thread and the window share.
#[derive(Default)]
pub struct Shared {
    pub stage: Option<Stage>,
    /// Every published build, newest first (None until it's fetched; empty if it couldn't be).
    pub history: Option<Vec<Manifest>>,
    /// The installed build's manifest (its notes, for What's new).
    pub installed: Option<Manifest>,
    /// A short note under the Play button (e.g. after checking the game files), and when it was posted.
    pub notice: Option<(String, Instant)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tab {
    Play,
    Notes,
}

/// Where the options menu opened from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Menu {
    Dock,
    Account,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Tab(Tab),
    Play,
    Options(Menu),
    OpenFolder,
    CheckFiles,
    ChangeKey,
    Paste,
    Activate,
    KeyField,
    Retry,
    Minimize,
    Close,
    Build(usize),
    ChangeFolder,
    ToggleDesktop,
    Install,
    Uninstall,
}

impl Action {
    fn in_menu(self) -> bool {
        matches!(self, Action::OpenFolder | Action::CheckFiles | Action::ChangeKey)
    }
}

pub struct Launcher {
    shared: Arc<Mutex<Shared>>,
    dir: PathBuf,
    /// The tester key being typed, and the one in use.
    typed: String,
    key: Option<String>,
    tab: Tab,
    menu: Option<Menu>,
    /// The build shown on the Patch notes tab (index into the history).
    selected: usize,
    art: art::KeyArt,
    mouse: Vec2,
    view: ui::View,
    hits: Vec<ui::Hit<Action>>,
    window: Option<WindowAction>,
    cursor: CursorIcon,
    quit: bool,
}

impl Launcher {
    fn new(dir: PathBuf) -> Self {
        let key = secret::load();
        let mut l = Self::with(dir, Stage::Busy { what: "Checking for updates".into(), progress: None });
        l.shared.lock().unwrap().installed = update::load_notes(&l.dir);
        match key {
            Some(k) => l.start(k, false),
            None => l.set(Stage::Key { error: None }),
        }
        l
    }

    /// A launcher that isn't installed yet: the setup screen.
    fn setup(legacy: Option<PathBuf>) -> Self {
        let mut l = Self::with(install::default_root(), Stage::Setup { root: install::default_root(), desktop: true, legacy, error: None });
        l.key = secret::load();
        l
    }

    fn with(dir: PathBuf, stage: Stage) -> Self {
        let shared = Arc::new(Mutex::new(Shared { stage: Some(stage), ..Default::default() }));
        Self {
            shared,
            dir,
            typed: String::new(),
            key: None,
            tab: Tab::Play,
            menu: None,
            selected: 0,
            art: art::KeyArt::load(),
            mouse: Vec2::ZERO,
            view: ui::View::default(),
            hits: Vec::new(),
            window: None,
            cursor: CursorIcon::Default,
            quit: false,
        }
    }

    fn set(&self, s: Stage) {
        self.shared.lock().unwrap().stage = Some(s);
    }

    fn stage(&self) -> Stage {
        self.shared.lock().unwrap().stage.clone().unwrap_or(Stage::Done)
    }

    fn notice(&self, text: &str) {
        self.shared.lock().unwrap().notice = Some((text.into(), Instant::now()));
    }

    /// Runs the checks on their own thread (checking the key first when it's a new one).
    fn start(&mut self, key: String, new_key: bool) {
        self.key = Some(key.clone());
        let (shared, dir) = (self.shared.clone(), self.dir.clone());
        let what = if new_key { "Checking your tester key" } else { "Checking for updates" };
        self.set(Stage::Busy { what: what.into(), progress: None });
        std::thread::spawn(move || {
            let set = |s: Stage| shared.lock().unwrap().stage = Some(s);
            if new_key {
                match update::check_key(&key) {
                    Ok(()) => {
                        if !secret::save(&key) {
                            eprintln!("launcher: couldn't save the tester key; it'll be asked for next time");
                        }
                    }
                    Err(Problem::Key(error)) => return set(Stage::Key { error: Some(error) }),
                    Err(p) => return set(Stage::Key { error: Some(p.to_string()) }),
                }
            }
            update_all(&dir, &key, &shared);
            let history = update::history(Channel::Game).unwrap_or_default();
            shared.lock().unwrap().history = Some(history);
        });
    }

    fn can_play(&self) -> bool {
        matches!(self.stage(), Stage::Ready { .. } | Stage::Failed { can_play: true, .. })
    }

    fn act(&mut self, a: Action) {
        if a.in_menu() {
            self.menu = None;
        }
        match a {
            Action::Tab(t) => self.tab = t,
            Action::Build(i) => self.selected = i,
            Action::Options(m) => self.menu = if self.menu == Some(m) { None } else { Some(m) },
            Action::Paste => {
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    self.typed = clean_key(&text);
                }
            }
            Action::KeyField => {}
            Action::Activate => {
                let key = clean_key(&self.typed);
                if key.len() >= 20 {
                    self.start(key, true);
                } else {
                    self.set(Stage::Key { error: Some("That key is too short. Tester keys look like STICK-XXXXX-XXXXX-XXXXX-XXXXX.".into()) });
                }
            }
            Action::Play if self.can_play() => {
                let exe = update::game_exe(&self.dir);
                match std::process::Command::new(&exe).current_dir(exe.parent().unwrap_or(&self.dir)).spawn() {
                    Ok(_) => self.quit = true,
                    Err(e) => self.set(Stage::Failed { message: format!("The game wouldn't start ({e})."), can_play: false }),
                }
            }
            Action::Play => {}
            Action::Retry => match self.key.clone() {
                Some(k) => self.start(k, false),
                None => self.set(Stage::Key { error: None }),
            },
            Action::ChangeKey => {
                secret::forget();
                self.key = None;
                self.typed.clear();
                self.tab = Tab::Play;
                self.set(Stage::Key { error: None });
            }
            Action::OpenFolder => {
                let folder = self.dir.join("game");
                let _ = std::fs::create_dir_all(&folder);
                if let Err(e) = std::process::Command::new("explorer").arg(&folder).spawn() {
                    self.notice(&format!("Couldn't open the folder ({e})."));
                }
            }
            Action::CheckFiles => self.check_files(),
            Action::Minimize => self.window = Some(WindowAction::Minimize),
            Action::Close => self.quit = true,
            Action::ChangeFolder | Action::ToggleDesktop | Action::Install => self.setup_action(a),
            Action::Uninstall => {
                if let Stage::Uninstall { root, done: false, .. } = self.stage() {
                    let error = install::uninstall(&root).err();
                    self.set(Stage::Uninstall { done: error.is_none(), root, error });
                }
            }
        }
    }

    fn setup_action(&mut self, a: Action) {
        let Stage::Setup { mut root, mut desktop, legacy, .. } = self.stage() else { return };
        let mut error = None;
        match a {
            Action::ChangeFolder => {
                if let Some(picked) = install::choose_folder(&root) {
                    root = picked;
                }
            }
            Action::ToggleDesktop => desktop = !desktop,
            _ => match install::install(&root, desktop, legacy.as_deref()) {
                Ok(exe) => match std::process::Command::new(&exe).current_dir(&root).spawn() {
                    Ok(_) => return self.quit = true,
                    Err(e) => error = Some(format!("Installed, but the launcher wouldn't start ({e}).")),
                },
                Err(e) => error = Some(e),
            },
        }
        self.set(Stage::Setup { root, desktop, legacy, error });
    }

    /// Checks the installed game against the latest signed build, and puts it right if it doesn't match.
    fn check_files(&mut self) {
        let Some(key) = self.key.clone() else { return };
        if !self.can_play() {
            return;
        }
        let (shared, dir, before) = (self.shared.clone(), self.dir.clone(), self.stage());
        self.set(Stage::Busy { what: "Checking the game files".into(), progress: None });
        std::thread::spawn(move || {
            let note = |s: &Arc<Mutex<Shared>>, stage: Stage, text: &str| {
                let mut sh = s.lock().unwrap();
                sh.stage = Some(stage);
                sh.notice = Some((text.into(), Instant::now()));
            };
            match update::latest(Channel::Game) {
                Ok(Some(m)) if update::game_is(&dir, &m) => note(&shared, before, "The game files are fine."),
                Ok(Some(m)) => {
                    let what = "Repairing the game files".to_owned();
                    let set = |s: Stage| shared.lock().unwrap().stage = Some(s);
                    match update::download(Channel::Game, &key, &m, progress_for(&what, &set)).and_then(|b| update::install_game(&dir, &b, m.build)) {
                        Ok(()) => {
                            update::save_notes(&dir, &m);
                            shared.lock().unwrap().installed = Some(m.clone());
                            note(&shared, Stage::Ready { build: Some(m.build), warning: None, offline: false }, "Repaired the game files.");
                        }
                        Err(p) => note(&shared, before, &format!("Couldn't repair the game files: {p}")),
                    }
                }
                Ok(None) => note(&shared, before, "There's no published build to check against."),
                Err(_) => note(&shared, before, "The game files can't be checked without the update service."),
            }
        });
    }
}

/// A download's progress reporter: bytes so far, the total and the speed, as a Busy stage.
fn progress_for<'a>(what: &'a str, set: &'a dyn Fn(Stage)) -> impl FnMut(u64, u64) + 'a {
    let started = Instant::now();
    move |done, total| {
        let secs = started.elapsed().as_secs_f64().max(0.05);
        set(Stage::Busy { what: what.to_owned(), progress: Some(Progress { done, total, speed: done as f64 / secs }) });
    }
}

/// A tester key as typed or pasted: letters, digits and dashes, upper case.
fn clean_key(text: &str) -> String {
    text.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect::<String>().to_ascii_uppercase()
}

/// The whole check: the launcher first (it restarts itself if it updated), then the game.
fn update_all(dir: &std::path::Path, key: &str, shared: &Arc<Mutex<Shared>>) {
    let set = |s: Stage| shared.lock().unwrap().stage = Some(s);
    let installed = update::installed_build(dir);
    let offline = || match installed {
        Some(b) => Stage::Ready { build: Some(b), warning: None, offline: true },
        None => Stage::Failed { message: "The update service can't be reached, and the game needs it once to install.".into(), can_play: false },
    };
    let mut warning = None;
    match update::latest(Channel::Launcher) {
        Ok(Some(m)) if update::launcher_build() > 0 && m.build > update::launcher_build() => {
            let what = format!("Updating the launcher to build {}", m.build);
            set(Stage::Busy { what: what.clone(), progress: Some(Progress { done: 0, total: m.size, speed: 0.0 }) });
            let got = update::download(Channel::Launcher, key, &m, progress_for(&what, &set));
            match got.and_then(|bytes| {
                let exe = std::env::current_exe().map_err(|e| Problem::Other(e.to_string()))?;
                update::swap(&exe, &bytes)?;
                std::process::Command::new(&exe).arg("--updated").spawn().map_err(|e| Problem::Other(e.to_string()))
            }) {
                Ok(_) => return set(Stage::Done),
                Err(Problem::Key(error)) => return set(Stage::Key { error: Some(error) }),
                Err(p) => warning = Some(format!("The launcher couldn't update itself: {p}")),
            }
        }
        Err(Problem::Offline(_)) => return set(offline()),
        Err(p) => warning = Some(p.to_string()),
        _ => {}
    }
    let installed_now = |m: &Manifest| {
        update::save_notes(dir, m);
        shared.lock().unwrap().installed = Some(m.clone());
    };
    match update::latest(Channel::Game) {
        Err(Problem::Offline(_)) => set(offline()),
        Err(p) => set(Stage::Failed { message: p.to_string(), can_play: installed.is_some() }),
        Ok(None) => set(match installed {
            Some(b) => Stage::Ready { build: Some(b), warning, offline: false },
            None => Stage::Failed { message: "No build of the game has been published yet.".into(), can_play: false },
        }),
        Ok(Some(m)) if update::game_is(dir, &m) => {
            installed_now(&m);
            set(Stage::Ready { build: Some(m.build), warning, offline: false });
        }
        Ok(Some(m)) => {
            let what = if installed.is_some() { format!("Updating to build {}", m.build) } else { format!("Installing build {}", m.build) };
            set(Stage::Busy { what: what.clone(), progress: Some(Progress { done: 0, total: m.size, speed: 0.0 }) });
            let got = update::download(Channel::Game, key, &m, progress_for(&what, &set));
            match got.and_then(|bytes| update::install_game(dir, &bytes, m.build)) {
                Ok(()) => {
                    installed_now(&m);
                    set(Stage::Ready { build: Some(m.build), warning, offline: false });
                }
                Err(Problem::Key(error)) => set(Stage::Key { error: Some(error) }),
                Err(p) => set(Stage::Failed { message: p.to_string(), can_play: installed.is_some() }),
            }
        }
    }
}

impl Game for Launcher {
    fn tick(&mut self, input: &Input) {
        self.mouse = input.mouse;
        let stage = self.stage();
        if input.mouse_pressed(Mouse::Left) {
            let at = self.view.to_design(input.mouse);
            let hit = self.hits.iter().rev().find(|h| h.contains(at)).map(|h| h.action);
            if self.menu.is_some() {
                match hit {
                    Some(a) if a.in_menu() => self.act(a),
                    Some(a @ Action::Options(_)) => self.act(a),
                    _ => self.menu = None,
                }
                return;
            }
            match hit {
                Some(a) => return self.act(a),
                None if at.y < 48.0 => self.window = Some(WindowAction::Drag),
                None => {}
            }
        }
        if input.pressed(KeyCode::Escape) {
            self.menu = None;
        }
        if let (Stage::Setup { .. }, true) = (&stage, input.pressed(KeyCode::Enter) || input.pressed(KeyCode::NumpadEnter)) {
            return self.act(Action::Install);
        }
        if let Stage::Key { .. } = stage {
            let ctrl = input.down(KeyCode::ControlLeft) || input.down(KeyCode::ControlRight);
            if ctrl && input.pressed(KeyCode::KeyV) {
                return self.act(Action::Paste);
            }
            if input.pressed(KeyCode::Backspace) {
                self.typed.pop();
            }
            if !ctrl {
                self.typed = clean_key(&format!("{}{}", self.typed, input.text));
            }
            if input.pressed(KeyCode::Enter) || input.pressed(KeyCode::NumpadEnter) {
                self.act(Action::Activate);
            }
        } else if (input.pressed(KeyCode::Enter) || input.pressed(KeyCode::NumpadEnter)) && self.menu.is_none() && self.tab == Tab::Play {
            self.act(Action::Play);
        }
    }

    fn draw(&mut self, frame: &mut Frame, screen: Vec2, _alpha: f32, _dt: f32) {
        let (stage, history, installed, notice) = {
            let mut sh = self.shared.lock().unwrap();
            if sh.notice.as_ref().is_some_and(|(_, t)| t.elapsed().as_secs_f32() > NOTICE_SECS) {
                sh.notice = None;
            }
            (sh.stage.clone().unwrap_or(Stage::Done), sh.history.clone(), sh.installed.clone(), sh.notice.clone().map(|n| n.0))
        };
        if stage == Stage::Done {
            self.quit = true;
        }
        if matches!(stage, Stage::Key { .. } | Stage::Setup { .. } | Stage::Uninstall { .. }) {
            self.menu = None;
        }
        let art = self.art.get();
        if let Some(a) = &art {
            frame.sprite_atlas = Some(a.clone());
        }
        let mut ui = ui::Ui::new(frame, screen, self.mouse);
        let state = view::State {
            stage: &stage,
            history: history.as_deref(),
            installed: installed.as_ref(),
            notice: notice.as_deref(),
            tab: self.tab,
            menu: self.menu,
            selected: self.selected,
            typed: &self.typed,
            key: self.key.as_deref(),
            art: art.is_some(),
            time: ui.frame.time,
        };
        view::draw(&mut ui, &state);
        self.cursor = match ui.hits.iter().rev().find(|h| h.contains(ui.mouse)) {
            Some(h) if h.text => CursorIcon::Text,
            Some(h) if !matches!(h.action, Action::Play) || self.can_play() => CursorIcon::Pointer,
            _ => CursorIcon::Default,
        };
        self.view = ui.view;
        self.hits = ui.hits;
    }

    fn wants_exit(&self) -> bool {
        self.quit
    }

    fn cursor_icon(&self) -> CursorIcon {
        self.cursor
    }

    fn window_action(&mut self) -> Option<WindowAction> {
        self.window.take()
    }
}

fn main() -> std::process::ExitCode {
    let flag = |f: &str| std::env::args().any(|a| a == f);
    if flag("--updated") {
        update::clean_up();
    }
    let here = match update::install_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("launcher: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let launcher = if flag("--uninstall") {
        let root = install::registered_root().unwrap_or_else(|| here.clone());
        Launcher::with(root.clone(), Stage::Uninstall { root, done: false, error: None })
    } else if flag("--portable") {
        // (developers: the old layout, the game beside this exe)
        Launcher::new(here)
    } else {
        match install::registered_root() {
            Some(root) if install::same_dir(&root, &here) => {
                install::register(&root);
                Launcher::new(root)
            }
            // a copy started from elsewhere (Downloads): open the installed one instead
            Some(root) => {
                let exe = root.join(install::EXE);
                return match std::process::Command::new(&exe).current_dir(&root).spawn() {
                    Ok(_) => std::process::ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("launcher: couldn't start {}: {e}", exe.display());
                        std::process::ExitCode::FAILURE
                    }
                };
            }
            // installed here, but Windows lost the Apps entry
            None if install::same_dir(&here, &install::default_root()) => {
                install::register(&here);
                Launcher::new(here)
            }
            None => Launcher::setup(install::legacy_game(&here)),
        }
    };
    let cfg = AppConfig {
        title: "Stickfell".into(),
        size: (1280, 720),
        window_icon: Some(include_bytes!("../../game/assets/icon/window-32.png")),
        taskbar_icon: Some(include_bytes!("../../game/assets/icon/window-64.png")),
        decorations: false,
        ..Default::default()
    };
    match stickfell_app::run(cfg, launcher) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("launcher: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod shots {
    use super::*;

    fn manifest(build: u32, message: &str) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "app": "stickfell-game", "build": build, "commit": "78f3545", "message": message,
            "sha256": "0".repeat(64), "size": 15_200_000u64, "builtAt": "2026-11-04T12:00:00Z"
        }))
        .unwrap()
    }

    /// Renders each screen: `SHOT_DIR=<dir> cargo test -p stickfell_launcher -- --ignored screens`
    #[test]
    #[ignore]
    fn screens() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
        let notes2 = "New: shoulder throw. Grab someone, turn your back on them and press Q to haul them over your shoulder. Softer, more natural shadows, and contact shadows where things meet the walls. A loading screen at startup, and the game no longer freezes for several seconds when it opens.";
        let history = vec![manifest(2, notes2), manifest(1, "First tester build of Stickfell: Last Limb Standing.")];
        let busy = Stage::Busy { what: "Updating to build 3".into(), progress: Some(Progress { done: 6_400_000, total: 15_200_000, speed: 3_100_000.0 }) };
        let ready = Stage::Ready { build: Some(2), warning: None, offline: false };
        let root = install::default_root();
        let screens: [(&str, Stage, Tab, Option<Menu>); 10] = [
            ("0-setup", Stage::Setup { root: root.clone(), desktop: true, legacy: Some(PathBuf::from("game")), error: None }, Tab::Play, None),
            ("8-uninstall", Stage::Uninstall { root: root.clone(), done: false, error: None }, Tab::Play, None),
            ("9-uninstalled", Stage::Uninstall { root: root.clone(), done: true, error: None }, Tab::Play, None),
            ("1-ready", ready.clone(), Tab::Play, None),
            ("2-updating", busy, Tab::Play, None),
            ("3-key", Stage::Key { error: Some("That key is too short. Tester keys look like STICK-XXXXX-XXXXX-XXXXX-XXXXX.".into()) }, Tab::Play, None),
            ("4-notes", ready.clone(), Tab::Notes, None),
            ("5-offline", Stage::Ready { build: Some(2), warning: None, offline: true }, Tab::Play, None),
            ("6-menu", ready.clone(), Tab::Play, Some(Menu::Dock)),
            ("7-checking", Stage::Busy { what: "Checking for updates".into(), progress: None }, Tab::Play, None),
        ];
        for (name, stage, tab, menu) in screens {
            let mut l = Launcher::with(PathBuf::new(), stage);
            l.key = Some("STICK-AB12C-DE34F-GH56J-9QX2".into());
            l.typed = "STICK-R2T5M-8Q0MV".into();
            l.tab = tab;
            l.menu = menu;
            {
                let mut sh = l.shared.lock().unwrap();
                sh.history = Some(history.clone());
                sh.installed = Some(history[0].clone());
            }
            for _ in 0..200 {
                if l.art.get().is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            stickfell_app::capture_headless(&mut l, 0, (1280, 720), format!("{dir}/launcher_{name}.png").as_ref()).unwrap();
        }
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// The whole check against the real service with this PC's saved key, installing into a scratch folder:
    /// `STICKFELL_LIVE_DIR=<dir> cargo test -p stickfell_launcher -- --ignored live_check --nocapture`
    #[test]
    #[ignore]
    fn live_check() {
        let dir = PathBuf::from(std::env::var("STICKFELL_LIVE_DIR").expect("STICKFELL_LIVE_DIR"));
        let key = secret::load().expect("no saved tester key on this PC");
        let shared = Arc::new(Mutex::new(Shared::default()));
        update_all(&dir, &key, &shared);
        eprintln!("stage: {:?}", shared.lock().unwrap().stage);
        eprintln!("history: {:?}", update::history(Channel::Game).map(|h| h.iter().map(|m| m.build).collect::<Vec<_>>()));
    }
}
