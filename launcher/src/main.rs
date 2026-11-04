//! Stickfell's launcher (players run it as Stickfell.exe). It keeps itself and the game up to date from the update
//! service, then starts the game: the first time it asks for a tester key (kept encrypted for this Windows account),
//! updates itself if there's a newer launcher, then installs or updates the game in game/ beside it, and shows the
//! build's notes and a Play button. Offline, it plays the build that's installed.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod secret;
mod update;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use stickfell_app::{AppConfig, Game, Input, KeyCode, Mouse};
use stickfell_render::{Color, Font, Frame, Vec2};
use update::{Channel, Problem};

/// Where things stand (the update work runs on its own thread and moves this along).
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    /// Waiting for a tester key (and why the last one didn't work).
    Key { error: Option<String> },
    Busy { what: String, progress: Option<f32> },
    Ready { build: Option<u32>, notes: Option<String>, warning: Option<String> },
    Failed { message: String, can_play: bool },
    /// Closing: the new launcher or the game has started.
    Done,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Paste,
    Submit,
    Play,
    Retry,
    ChangeKey,
}

struct Launcher {
    stage: Arc<Mutex<Stage>>,
    dir: PathBuf,
    /// The tester key being typed, and the one in use.
    typed: String,
    key: Option<String>,
    mouse: Vec2,
    buttons: Vec<(Vec2, Vec2, Action)>,
    quit: bool,
}

impl Launcher {
    fn new(dir: PathBuf) -> Self {
        let key = secret::load();
        let stage = Arc::new(Mutex::new(Stage::Busy { what: "Checking for updates".into(), progress: None }));
        let mut l = Self { stage, dir, typed: String::new(), key: key.clone(), mouse: Vec2::ZERO, buttons: Vec::new(), quit: false };
        match key {
            Some(k) => l.start(k, false),
            None => l.set(Stage::Key { error: None }),
        }
        l
    }

    fn set(&self, s: Stage) {
        *self.stage.lock().unwrap() = s;
    }

    fn stage(&self) -> Stage {
        self.stage.lock().unwrap().clone()
    }

    /// Runs the checks on their own thread (checking the key first when it's a new one).
    fn start(&mut self, key: String, new_key: bool) {
        self.key = Some(key.clone());
        let (stage, dir) = (self.stage.clone(), self.dir.clone());
        let set = move |s: Stage| *stage.lock().unwrap() = s;
        set(Stage::Busy { what: if new_key { "Checking your tester key".into() } else { "Checking for updates".into() }, progress: None });
        std::thread::spawn(move || {
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
            update_all(&dir, &key, &set);
        });
    }

    fn act(&mut self, a: Action) {
        match a {
            Action::Paste => {
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    self.typed = clean_key(&text);
                }
            }
            Action::Submit => {
                let key = clean_key(&self.typed);
                if key.len() >= 20 {
                    self.start(key, true);
                } else {
                    self.set(Stage::Key { error: Some("That's too short for a tester key (STICK-XXXXX-XXXXX-XXXXX-XXXXX).".into()) });
                }
            }
            Action::Play => {
                let exe = update::game_exe(&self.dir);
                match std::process::Command::new(&exe).current_dir(exe.parent().unwrap_or(&self.dir)).spawn() {
                    Ok(_) => self.quit = true,
                    Err(e) => self.set(Stage::Failed { message: format!("The game wouldn't start ({e})."), can_play: false }),
                }
            }
            Action::Retry => match self.key.clone() {
                Some(k) => self.start(k, false),
                None => self.set(Stage::Key { error: None }),
            },
            Action::ChangeKey => {
                secret::forget();
                self.key = None;
                self.typed.clear();
                self.set(Stage::Key { error: None });
            }
        }
    }

    fn button(&mut self, frame: &mut Frame, at: Vec2, w: f32, label: &str, action: Action, k: f32, main: bool) {
        let h = 44.0 * k;
        let min = Vec2::new(at.x - w * 0.5, at.y);
        let size = Vec2::new(w, h);
        let hover = (self.mouse - min).cmpge(Vec2::ZERO).all() && (self.mouse - min - size).cmple(Vec2::ZERO).all();
        let bg = match (hover, main) {
            (true, _) => Color::hex(0xff5a61),
            (false, true) => Color::hex(0xe8484f),
            (false, false) => Color::hex(0x1c1f27).alpha(0.92),
        };
        frame.ui_rect(min, size, 6.0 * k, bg);
        let scale = 2.4 * k;
        let tw = Frame::text_width(label, scale);
        frame.ui_text(Vec2::new(at.x - tw * 0.5, at.y + (h - 7.0 * scale) * 0.5), scale, label, Color::WHITE);
        self.buttons.push((min, size, action));
    }
}

/// A tester key as typed or pasted: letters, digits and dashes, upper case.
fn clean_key(text: &str) -> String {
    text.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect::<String>().to_ascii_uppercase()
}

/// The whole check: the launcher first (it restarts itself if it updated), then the game.
fn update_all(dir: &std::path::Path, key: &str, set: &dyn Fn(Stage)) {
    let installed = update::installed_build(dir);
    let offline = |why: &str| match installed {
        Some(b) => Stage::Ready { build: Some(b), notes: None, warning: Some(format!("{why} Playing the installed build.")) },
        None => Stage::Failed { message: format!("{why} The game isn't installed yet, so it needs the update service once."), can_play: false },
    };
    let mut warning = None;
    match update::latest(Channel::Launcher) {
        Ok(Some(m)) if update::launcher_build() > 0 && m.build > update::launcher_build() => {
            let what = format!("Updating the launcher to build {}", m.build);
            set(Stage::Busy { what: what.clone(), progress: Some(0.0) });
            let got = update::download(Channel::Launcher, key, &m, |a, b| set(Stage::Busy { what: what.clone(), progress: Some(a as f32 / b as f32) }));
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
        Err(Problem::Offline(_)) => return set(offline("The update service couldn't be reached.")),
        Err(p) => warning = Some(p.to_string()),
        _ => {}
    }
    match update::latest(Channel::Game) {
        Err(Problem::Offline(_)) => set(offline("The update service couldn't be reached.")),
        Err(p) => set(Stage::Failed { message: p.to_string(), can_play: installed.is_some() }),
        Ok(None) => set(match installed {
            Some(b) => Stage::Ready { build: Some(b), notes: None, warning },
            None => Stage::Failed { message: "No build of the game has been published yet.".into(), can_play: false },
        }),
        Ok(Some(m)) if update::game_is(dir, &m) => set(Stage::Ready { build: Some(m.build), notes: m.message.clone(), warning }),
        Ok(Some(m)) => {
            let what = if installed.is_some() { format!("Updating the game to build {}", m.build) } else { format!("Installing build {}", m.build) };
            set(Stage::Busy { what: what.clone(), progress: Some(0.0) });
            let got = update::download(Channel::Game, key, &m, |a, b| set(Stage::Busy { what: what.clone(), progress: Some(a as f32 / b as f32) }));
            match got.and_then(|bytes| update::install_game(dir, &bytes, m.build)) {
                Ok(()) => set(Stage::Ready { build: Some(m.build), notes: m.message.clone(), warning }),
                Err(Problem::Key(error)) => set(Stage::Key { error: Some(error) }),
                Err(p) => set(Stage::Failed { message: p.to_string(), can_play: installed.is_some() }),
            }
        }
    }
}

/// Splits text into lines that fit `width` (by words).
fn wrap(text: &str, scale: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let next = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if Frame::text_width(&next, scale) > width && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = next;
            }
        }
        lines.push(line);
    }
    lines
}

impl Game for Launcher {
    fn tick(&mut self, input: &Input) {
        self.mouse = input.mouse;
        let clicked = input.mouse_pressed(Mouse::Left).then(|| {
            self.buttons.iter().find(|(min, size, _)| (input.mouse - *min).cmpge(Vec2::ZERO).all() && (input.mouse - *min - *size).cmple(Vec2::ZERO).all()).map(|b| b.2)
        });
        if let Some(Some(a)) = clicked {
            return self.act(a);
        }
        if let Stage::Key { .. } = self.stage() {
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
                self.act(Action::Submit);
            }
        } else if let Stage::Ready { .. } = self.stage() {
            if input.pressed(KeyCode::Enter) {
                self.act(Action::Play);
            }
        }
        if input.pressed(KeyCode::Escape) {
            self.quit = true;
        }
    }

    fn draw(&mut self, frame: &mut Frame, screen: Vec2, _alpha: f32, _dt: f32) {
        self.buttons.clear();
        frame.clear = Color::hex(0x10121a);
        let k = (screen.y / 540.0).clamp(0.6, 3.0);
        let cx = screen.x * 0.5;
        let centred = |frame: &mut Frame, font: Font, y: f32, scale: f32, text: &str, color: Color| {
            let w = Frame::text_width_in(font, text, scale);
            frame.ui_text_in(font, Vec2::new(cx - w * 0.5, y), scale, text, color);
        };
        centred(frame, Font::Title, 46.0 * k, 9.0 * k, "STICKFELL", Color::hex(0xe8484f));
        centred(frame, Font::Title, 128.0 * k, 2.6 * k, "LAST LIMB STANDING", Color::hex(0xc8ccd6));
        let grey = Color::hex(0x9aa0ad);
        let y = 200.0 * k;
        match self.stage() {
            Stage::Key { error } => {
                centred(frame, Font::Body, y, 2.4 * k, "Enter your tester key", Color::WHITE);
                let (bw, bh) = (420.0 * k, 46.0 * k);
                frame.ui_rect(Vec2::new(cx - bw * 0.5, y + 40.0 * k), Vec2::new(bw, bh), 6.0 * k, Color::hex(0x1c1f27));
                let shown = if self.typed.is_empty() { "STICK-XXXXX-XXXXX-XXXXX-XXXXX".to_owned() } else { format!("{}_", self.typed) };
                let tc = if self.typed.is_empty() { Color::hex(0x5a5f6b) } else { Color::WHITE };
                centred(frame, Font::Body, y + 40.0 * k + (bh - 7.0 * 2.4 * k) * 0.5, 2.4 * k, &shown, tc);
                if let Some(e) = error {
                    for (i, line) in wrap(&e, 1.9 * k, 560.0 * k).iter().enumerate() {
                        centred(frame, Font::Body, y + 100.0 * k + i as f32 * 20.0 * k, 1.9 * k, line, Color::hex(0xff7070));
                    }
                }
                self.button(frame, Vec2::new(cx - 110.0 * k, y + 160.0 * k), 200.0 * k, "Paste", Action::Paste, k, false);
                self.button(frame, Vec2::new(cx + 110.0 * k, y + 160.0 * k), 200.0 * k, "Continue", Action::Submit, k, true);
            }
            Stage::Busy { what, progress } => {
                centred(frame, Font::Body, y + 40.0 * k, 2.4 * k, &format!("{what}…"), Color::WHITE);
                if let Some(p) = progress {
                    let (bw, bh) = (420.0 * k, 10.0 * k);
                    let at = Vec2::new(cx - bw * 0.5, y + 90.0 * k);
                    frame.ui_rect(at, Vec2::new(bw, bh), 5.0 * k, Color::hex(0x1c1f27));
                    frame.ui_rect(at, Vec2::new((bw * p.clamp(0.0, 1.0)).max(bh), bh), 5.0 * k, Color::hex(0xe8484f));
                    centred(frame, Font::Body, y + 110.0 * k, 1.9 * k, &format!("{:.0}%", p * 100.0), grey);
                }
            }
            Stage::Ready { build, notes, warning } => {
                let title = build.map_or("Ready".to_owned(), |b| format!("Build {b}"));
                centred(frame, Font::Body, y, 2.6 * k, &title, Color::WHITE);
                let mut ly = y + 38.0 * k;
                for text in [warning.as_deref().map(|w| (w, Color::hex(0xffc060))), notes.as_deref().map(|n| (n, grey))].into_iter().flatten() {
                    for line in wrap(text.0, 1.9 * k, 600.0 * k).iter().take(5) {
                        centred(frame, Font::Body, ly, 1.9 * k, line, text.1);
                        ly += 20.0 * k;
                    }
                    ly += 8.0 * k;
                }
                self.button(frame, Vec2::new(cx, 400.0 * k), 260.0 * k, "Play", Action::Play, k, true);
                self.button(frame, Vec2::new(cx, 456.0 * k), 160.0 * k, "Change key", Action::ChangeKey, k * 0.8, false);
            }
            Stage::Failed { message, can_play } => {
                for (i, line) in wrap(&message, 2.0 * k, 600.0 * k).iter().take(4).enumerate() {
                    centred(frame, Font::Body, y + i as f32 * 22.0 * k, 2.0 * k, line, Color::hex(0xff7070));
                }
                self.button(frame, Vec2::new(cx - (if can_play { 110.0 } else { 0.0 }) * k, 400.0 * k), 200.0 * k, "Retry", Action::Retry, k, !can_play);
                if can_play {
                    self.button(frame, Vec2::new(cx + 110.0 * k, 400.0 * k), 200.0 * k, "Play anyway", Action::Play, k, true);
                }
            }
            Stage::Done => self.quit = true,
        }
        let build = match update::launcher_build() {
            0 => "Launcher (built on a PC)".to_owned(),
            n => format!("Launcher build {n}"),
        };
        frame.ui_text(Vec2::new(12.0 * k, screen.y - 24.0 * k), 1.6 * k, &build, Color::hex(0x5a5f6b));
    }

    fn wants_exit(&self) -> bool {
        self.quit
    }
}

fn main() -> std::process::ExitCode {
    if std::env::args().any(|a| a == "--updated") {
        update::clean_up();
    }
    let dir = match update::install_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("launcher: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let cfg = AppConfig {
        title: "Stickfell".into(),
        size: (960, 540),
        window_icon: Some(include_bytes!("../../game/assets/icon/window-32.png")),
        taskbar_icon: Some(include_bytes!("../../game/assets/icon/window-64.png")),
        ..Default::default()
    };
    match stickfell_app::run(cfg, Launcher::new(dir)) {
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

    /// Renders each screen: `SHOT_DIR=<dir> cargo test -p stickfell_launcher -- --ignored screens`
    #[test]
    #[ignore]
    fn screens() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into());
        let stages = [
            ("key", Stage::Key { error: Some("That tester key isn't valid for Stickfell. Check it, or ask for a new one.".into()) }),
            ("busy", Stage::Busy { what: "Installing build 12".into(), progress: Some(0.42) }),
            ("ready", Stage::Ready { build: Some(12), notes: Some("Thrown knives stick in, the grapple leaves rope to spare, and gunshots no longer mute the sound.".into()), warning: None }),
            ("offline", Stage::Failed { message: "The update service couldn't be reached. The game isn't installed yet, so it needs the update service once.".into(), can_play: false }),
        ];
        for (name, stage) in stages {
            let mut l = Launcher { stage: Arc::new(Mutex::new(stage)), dir: PathBuf::new(), typed: "STICK-R2T5M-8Q0MV".into(), key: None, mouse: Vec2::ZERO, buttons: Vec::new(), quit: false };
            stickfell_app::capture_headless(&mut l, 0, (960, 540), format!("{dir}/launcher_{name}.png").as_ref()).unwrap();
        }
    }
}
