//! The launcher's screens, drawn in the 1280 x 720 design space (see `ui`).

use stickfell_render::{Color, Font, Vec2};

use crate::ui::{self, c, col, Ui};
use crate::update::{self, Manifest};
use crate::{Action, Menu, Stage, Tab};

/// Everything a frame shows.
pub struct State<'a> {
    pub stage: &'a Stage,
    pub history: Option<&'a [Manifest]>,
    pub installed: Option<&'a Manifest>,
    pub notice: Option<&'a str>,
    pub tab: Tab,
    pub menu: Option<Menu>,
    pub selected: usize,
    pub typed: &'a str,
    pub key: Option<&'a str>,
    pub art: bool,
    pub time: f32,
}

impl State<'_> {
    fn offline(&self) -> bool {
        matches!(self.stage, Stage::Ready { offline: true, .. } | Stage::Failed { .. })
    }
    fn can_play(&self) -> bool {
        matches!(self.stage, Stage::Ready { .. } | Stage::Failed { can_play: true, .. })
    }
    fn build(&self) -> Option<u32> {
        match self.stage {
            Stage::Ready { build, .. } => *build,
            _ => self.installed.map(|m| m.build),
        }
    }
}

type U<'a, 'b> = Ui<'a, Action>;

const BODY: Font = Font::Body;
const TITLE: Font = Font::Title;

pub fn draw(ui: &mut U, s: &State) {
    ui.fill(col(c::BG));
    if let Stage::Key { error } = s.stage {
        return key_screen(ui, s, error.as_deref());
    }
    match s.tab {
        Tab::Play => play_tab(ui, s),
        Tab::Notes => notes_tab(ui, s),
    }
    title_bar(ui, s, true);
    if let Some(m) = s.menu {
        options_menu(ui, s, m);
    }
}

// ---- title bar ----

fn title_bar(ui: &mut U, s: &State, full: bool) {
    ui.rect(0.0, 0.0, ui::W, 48.0, 0.0, col(c::BAR));
    ui.rect(0.0, 47.0, ui::W, 1.0, 0.0, col(c::LINE));
    let logo = ui.text(TITLE, 20.0, 24.0, 22.0, "STICKFELL", col(c::RED));
    if full {
        let mut x = 20.0 + logo + 28.0;
        for (tab, label) in [(Tab::Play, "Play"), (Tab::Notes, "Patch notes")] {
            let w = ui.text_w(BODY, 15.0, label) + 28.0;
            let hover = ui.hit(x, 0.0, w, 48.0, Action::Tab(tab));
            let active = s.tab == tab;
            let color = if active || hover { c::TEXT } else { c::MUTED };
            ui.text(BODY, x + 14.0, 25.0, 15.0, label, col(color));
            if active {
                ui.rect(x, 45.0, w, 3.0, 0.0, col(c::RED));
            }
            x += w + 4.0;
        }
        if let Some(key) = s.key {
            let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            let label = format!("Tester · {tail}");
            let w = 12.0 + 8.0 + 8.0 + ui.text_w(BODY, 13.0, &label) + 12.0;
            let x = ui::W - 96.0 - 28.0 - w;
            let hover = ui.hit(x, 9.0, w, 30.0, Action::Options(Menu::Account));
            let open = s.menu == Some(Menu::Account);
            ui.rect(x, 9.0, w, 30.0, 15.0, col(if hover || open { c::RAISED } else { c::CARD }));
            ui.circle(Vec2::new(x + 16.0, 24.0), 4.0, col(if s.offline() { c::FAINT } else { c::GREEN }));
            ui.text(BODY, x + 28.0, 24.0, 13.0, &label, col(c::SOFT));
        }
    }
    let (min_x, close_x) = (ui::W - 96.0, ui::W - 48.0);
    let hover = ui.hit(min_x, 0.0, 48.0, 48.0, Action::Minimize);
    if hover {
        ui.rect(min_x, 0.0, 48.0, 47.0, 0.0, col(c::RAISED));
    }
    ui::icon_minimise(ui, min_x + 24.0, 24.0, col(if hover { c::TEXT } else { c::MUTED }));
    let hover = ui.hit(close_x, 0.0, 48.0, 48.0, Action::Close);
    if hover {
        ui.rect(close_x, 0.0, 48.0, 47.0, 0.0, col(c::RED));
    }
    ui::icon_close(ui, close_x + 24.0, 24.0, col(if hover { 0xffffff } else { c::MUTED }));
}

// ---- Play ----

fn key_art(ui: &mut U, s: &State) {
    if s.art {
        ui.sprite(370.0, 44.0, 980.0, 551.0, [0.0, 0.0, 1920.0, 1080.0], Color::WHITE);
    }
}

fn play_tab(ui: &mut U, s: &State) {
    key_art(ui, s);
    ui.fade(0.0, 460.0, ui::W, 260.0, col(c::BG), 0.7);
    let banner = banner_text(s);
    if let Some(text) = &banner {
        offline_banner(ui, text);
    }
    let top = if banner.is_some() { 150.0 } else { 132.0 };
    ui.text(TITLE, 56.0, top + 50.0, 104.0, "STICKFELL", col(c::RED));
    ui.text(TITLE, 58.0, top + 122.0, 28.0, "LAST LIMB STANDING", col(c::SOFT));
    let chip = "Closed testing";
    let w = 24.0 + ui.text_w(BODY, 13.0, chip);
    ui.rect(56.0, top + 160.0, w, 28.0, 14.0, col(c::RAISED));
    ui.text(BODY, 68.0, top + 174.0, 13.0, chip, col(c::SOFT));
    whats_new(ui, s);
    dock(ui, s);
}

/// What the amber bar under the title bar says, if anything's wrong.
fn banner_text(s: &State) -> Option<String> {
    match s.stage {
        Stage::Ready { offline: true, .. } => Some("The update service can't be reached, so updates are paused. You can still play the installed build.".into()),
        Stage::Failed { message, .. } => Some(message.clone()),
        _ => None,
    }
}

fn offline_banner(ui: &mut U, text: &str) {
    ui.rect(0.0, 48.0, ui::W, 48.0, 0.0, col(c::AMBER_BG));
    ui.rect(0.0, 95.0, ui::W, 1.0, 0.0, col(c::AMBER_LINE));
    ui::icon_warning(ui, 24.0, 63.0, 18.0, col(c::AMBER_ICON));
    let label = "Retry";
    let bw = 32.0 + ui.text_w(BODY, 14.0, label);
    let bx = ui::W - 20.0 - bw;
    ui.paragraph(BODY, 54.0, 72.0, 14.0, 18.0, bx - 54.0 - 24.0, 1, text, col(c::AMBER));
    let hover = ui.hit(bx, 56.0, bw, 32.0, Action::Retry);
    ui.rect(bx, 56.0, bw, 32.0, 6.0, col(0x6b5520));
    ui.rect(bx + 1.0, 57.0, bw - 2.0, 30.0, 5.0, col(if hover { 0x3a2e14 } else { c::AMBER_BG }));
    ui.text(BODY, bx + 16.0, 72.0, 14.0, label, col(c::AMBER));
}

fn whats_new(ui: &mut U, s: &State) {
    let Some(m) = s.installed.or_else(|| s.history.and_then(|h| h.first())) else { return };
    let notes = m.message.clone().unwrap_or_default();
    let lines = ui_wrap_count(ui, BODY, 15.0, 520.0, &notes).min(3);
    let card_h = 18.0 + 22.0 + if lines > 0 { 10.0 + lines as f32 * 22.5 } else { 0.0 } + 16.0;
    let (x, bottom) = (56.0, ui::H - 40.0);
    let card_y = bottom - card_h;
    let head_mid = card_y - 12.0 - 8.0;
    let offline = s.offline();
    ui.text(BODY, x, head_mid, 13.0, if offline { "INSTALLED" } else { "WHAT'S NEW" }, col(c::MUTED));
    if !offline {
        let link = "All patch notes";
        let lw = ui.text_w(BODY, 14.0, link);
        let hover = ui.hit(x + 560.0 - lw - 4.0, head_mid - 12.0, lw + 8.0, 24.0, Action::Tab(Tab::Notes));
        let color = col(if hover { 0xffffff } else { c::TEXT });
        ui.text_right(BODY, x + 560.0, head_mid, 14.0, link, color);
        ui.rect(x + 560.0 - lw, head_mid + 8.0, lw, 1.0, 0.0, color);
    }
    ui.rect(x, card_y, 560.0, card_h, 8.0, col(c::CARD));
    let title = format!("Build {}", m.build);
    let tw = ui.text(BODY, x + 20.0, card_y + 29.0, 18.0, &title, col(c::TEXT));
    if let Some(date) = m.date() {
        ui.text(BODY, x + 20.0 + tw + 12.0, card_y + 30.0, 13.0, &date, col(c::MUTED));
    }
    if lines > 0 {
        ui.paragraph(BODY, x + 20.0, card_y + 18.0 + 22.0 + 10.0 + 11.0, 15.0, 22.5, 520.0, 3, &notes, col(c::SOFT));
    }
}

fn ui_wrap_count(ui: &U, font: Font, size: f32, width: f32, text: &str) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let mut n = 0;
    for para in text.lines() {
        let mut cur = String::new();
        n += 1;
        for word in para.split_whitespace() {
            let next = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
            if ui.text_w(font, size, &next) > width && !cur.is_empty() {
                n += 1;
                cur = word.to_owned();
            } else {
                cur = next;
            }
        }
    }
    n
}

fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1e6)
}

fn time_left(p: &crate::Progress) -> String {
    if p.speed < 1.0 {
        return "working out the time left".into();
    }
    let secs = (p.total.saturating_sub(p.done)) as f64 / p.speed;
    match secs {
        s if s < 1.5 => "less than a second left".into(),
        s if s < 90.0 => format!("about {} seconds left", s.round() as u32),
        s => format!("about {} minutes left", (s / 60.0).round() as u32),
    }
}

fn dock(ui: &mut U, s: &State) {
    let (x, w, bottom) = (ui::W - 56.0 - 360.0, 360.0, ui::H - 40.0);
    let launcher = match update::launcher_build() {
        0 => "Launcher (built on this PC)".to_owned(),
        n => format!("Launcher build {n}"),
    };
    ui.text(BODY, x, bottom - 8.0, 12.0, &launcher, col(c::FAINT));
    let row = bottom - 16.0 - 12.0 - 76.0;
    let play_w = w - 10.0 - 76.0;
    let playable = s.can_play();
    let hover = ui.hit(x, row, play_w, 76.0, Action::Play) && playable;
    let (bg, fg) = match (playable, hover) {
        (true, true) => (c::RED_HOVER, 0xffffff),
        (true, false) => (c::RED, 0xffffff),
        _ => (c::DISABLED, c::FAINT),
    };
    ui.rect(x, row, play_w, 76.0, 8.0, col(bg));
    ui.text_centred(TITLE, x + play_w * 0.5, row + 39.0, 40.0, "PLAY", col(fg));
    let ox = x + play_w + 10.0;
    let open = s.menu == Some(Menu::Dock);
    let hover = ui.hit(ox, row, 76.0, 76.0, Action::Options(Menu::Dock));
    ui.rect(ox, row, 76.0, 76.0, 8.0, col(if hover || open { c::RAISED_HOVER } else { c::RAISED }));
    ui::icon_dots(ui, ox + 38.0, row + 38.0, 24.0, col(if open { c::TEXT } else { c::SOFT }));

    // what's happening, above the buttons
    let status_mid = row - 12.0 - 10.0;
    match s.stage {
        Stage::Busy { what, progress } => {
            let bar_y = row - 12.0 - 18.0 - 12.0 - 8.0;
            let mut title_mid = bar_y - 12.0 - 10.0;
            match progress {
                Some(p) if p.total > 0 => {
                    let f = (p.done as f32 / p.total as f32).clamp(0.0, 1.0);
                    ui.text(BODY, x, status_mid + 1.0, 13.0, &format!("{} of {} MB · {:.1} MB/s · {}", mb(p.done), mb(p.total), p.speed / 1e6, time_left(p)), col(c::MUTED));
                    ui.rect(x, bar_y, w, 8.0, 4.0, col(c::RAISED));
                    if f > 0.0 {
                        ui.rect(x, bar_y, (w * f).max(8.0), 8.0, 4.0, col(c::RED));
                    }
                    ui.text_right(BODY, x + w, title_mid, 15.0, &format!("{:.0}%", f * 100.0), col(c::TEXT));
                }
                _ => {
                    // no size yet: a sliding bar
                    title_mid = status_mid - 20.0;
                    let bar_y = status_mid - 4.0;
                    ui.rect(x, bar_y, w, 8.0, 4.0, col(c::RAISED));
                    let t = (s.time * 0.7).fract() * 1.35 - 0.35;
                    let (a, b) = ((t * w).max(0.0), ((t + 0.35) * w).min(w));
                    if b > a + 1.0 {
                        ui.rect(x + a, bar_y, b - a, 8.0, 4.0, col(c::RED));
                    }
                }
            }
            ui.text(BODY, x, title_mid, 15.0, &format!("{what}…"), col(c::TEXT));
        }
        _ => {
            let (icon, text): (u8, String) = match (s.notice, s.stage) {
                (Some(n), _) => (0, n.to_owned()),
                (None, Stage::Ready { warning: Some(wn), .. }) => (2, wn.clone()),
                (None, Stage::Ready { offline: true, build, .. }) => (1, format!("Offline · Build {} installed", build.unwrap_or(0))),
                (None, Stage::Ready { build: Some(b), .. }) => (0, format!("Up to date · Build {b}")),
                (None, Stage::Failed { can_play: true, .. }) => (2, "Updates paused".into()),
                (None, Stage::Failed { .. }) => (2, "Can't install the game yet".into()),
                _ => (0, String::new()),
            };
            let y = status_mid - 8.0;
            match icon {
                0 => ui::icon_check(ui, x, y, 16.0, col(c::GREEN)),
                1 => ui::icon_offline(ui, x, y, 16.0, col(c::FAINT)),
                _ => ui::icon_warning(ui, x, y, 16.0, col(c::AMBER_ICON)),
            }
            ui.paragraph(BODY, x + 24.0, status_mid, 14.0, 18.0, w - 24.0, 1, &text, col(c::SOFT));
        }
    }
}

fn options_menu(ui: &mut U, s: &State, from: Menu) {
    let (w, h) = (280.0, 6.0 + 132.0 + 13.0 + 34.0 + 6.0);
    let (x, y) = match from {
        Menu::Dock => (ui::W - 56.0 - w, ui::H - 152.0 - h),
        Menu::Account => (ui::W - 96.0 - 28.0 - w, 56.0),
    };
    ui.shadow(x, y + 10.0, w, h, 10.0, 36.0, Color::BLACK.alpha(0.55));
    ui.rect(x, y, w, h, 10.0, col(c::MENU));
    let items: [(Action, &str, fn(&mut U, f32, f32, f32, Color)); 3] = [
        (Action::OpenFolder, "Open game folder", ui::icon_folder),
        (Action::CheckFiles, "Check game files", ui::icon_shield),
        (Action::ChangeKey, "Change tester key", ui::icon_key),
    ];
    for (i, (action, label, icon)) in items.into_iter().enumerate() {
        let iy = y + 6.0 + i as f32 * 44.0;
        let enabled = action != Action::CheckFiles || s.can_play();
        let hover = ui.hit(x + 6.0, iy, w - 12.0, 44.0, action) && enabled;
        if hover {
            ui.rect(x + 6.0, iy, w - 12.0, 44.0, 6.0, col(c::RAISED_HOVER));
        }
        let color = col(if enabled { c::TEXT } else { c::FAINT });
        icon(ui, x + 18.0, iy + 13.0, 18.0, color);
        ui.text(BODY, x + 48.0, iy + 22.0, 15.0, label, color);
    }
    let sep = y + 6.0 + 132.0 + 6.0;
    ui.rect(x + 10.0, sep, w - 20.0, 1.0, 0.0, col(0x2e3344));
    let game = s.build().map_or("Game not installed".to_owned(), |b| format!("Game build {b}"));
    let launcher = match update::launcher_build() {
        0 => "launcher built on this PC".to_owned(),
        n => format!("Launcher build {n}"),
    };
    ui.text(BODY, x + 18.0, sep + 1.0 + 17.0, 12.0, &format!("{game} · {launcher}"), col(c::FAINT));
}

// ---- Patch notes ----

fn notes_tab(ui: &mut U, s: &State) {
    ui.rect(299.0, 48.0, 1.0, ui::H - 48.0, 0.0, col(c::LINE));
    ui.text(BODY, 28.0, 88.0, 13.0, "BUILDS", col(c::MUTED));
    let Some(history) = s.history else {
        ui.text(BODY, 364.0, 110.0, 17.0, "Loading patch notes…", col(c::MUTED));
        return;
    };
    if history.is_empty() {
        ui.paragraph(BODY, 364.0, 110.0, 17.0, 27.0, 740.0, 3, "No patch notes yet. They come from the update service, so they show once it can be reached.", col(c::MUTED));
        return;
    }
    let installed = s.build();
    let selected = s.selected.min(history.len() - 1);
    for (i, m) in history.iter().enumerate().take(9) {
        let y = 108.0 + i as f32 * 68.0;
        let hover = ui.hit(16.0, y, 268.0, 64.0, Action::Build(i));
        if i == selected {
            ui.rect(16.0, y, 268.0, 64.0, 6.0, col(c::RAISED));
        } else if hover {
            ui.rect(16.0, y, 268.0, 64.0, 6.0, col(c::CARD));
        }
        ui.text(BODY, 28.0, y + 22.0, 16.0, &format!("Build {}", m.build), col(c::TEXT));
        if installed == Some(m.build) {
            ui.text_right(BODY, 272.0, y + 22.0, 12.0, "Installed", col(c::GREEN));
        }
        if let Some(d) = m.date() {
            ui.text(BODY, 28.0, y + 44.0, 13.0, &d, col(c::MUTED));
        }
    }
    let m = &history[selected];
    ui.text(TITLE, 364.0, 110.0, 44.0, &format!("BUILD {}", m.build), col(c::TEXT));
    let sub = match m.date() {
        Some(d) => format!("{d} · {} MB", mb(m.size)),
        None => format!("{} MB", mb(m.size)),
    };
    ui.text(BODY, 364.0, 150.0, 14.0, &sub, col(c::MUTED));
    let notes = m.message.as_deref().unwrap_or("No notes for this build.");
    ui.paragraph(BODY, 364.0, 196.0, 17.0, 27.0, 740.0, 18, notes, col(0xd6d9e1));
}

// ---- first run: the tester key ----

fn key_screen(ui: &mut U, s: &State, error: Option<&str>) {
    key_art(ui, s);
    ui.rect(0.0, 0.0, ui::W, ui::H, 0.0, Color::hex(0x0a0b10).alpha(0.78));
    title_bar(ui, s, false);
    let (x, w, pad) = (390.0, 500.0, 40.0);
    let inner = w - pad * 2.0;
    let intro = "Stickfell is in closed testing. Enter the tester key you were sent to install the game.";
    let foot = "Your key is saved on this PC, encrypted for your Windows account.";
    let n_intro = ui_wrap_count(ui, BODY, 15.0, inner, intro) as f32;
    let n_err = error.map_or(0.0, |e| ui_wrap_count(ui, BODY, 14.0, inner, e) as f32);
    let n_foot = ui_wrap_count(ui, BODY, 13.0, inner, foot) as f32;
    let h = 36.0 + 34.0 + 8.0 + n_intro * 22.5 + 18.0 + 16.0 + 8.0 + 48.0 + if n_err > 0.0 { 8.0 + n_err * 20.0 } else { 0.0 } + 18.0 + 52.0 + 18.0 + n_foot * 19.5 + 32.0;
    let y0 = 360.0 - h * 0.46;
    ui.shadow(x, y0 + 16.0, w, h, 12.0, 48.0, Color::BLACK.alpha(0.5));
    ui.rect(x, y0, w, h, 12.0, col(c::CARD));
    let lx = x + pad;
    let mut y = y0 + 36.0;
    ui.text(TITLE, lx, y + 17.0, 34.0, "ACTIVATE STICKFELL", col(c::TEXT));
    y += 34.0 + 8.0;
    ui.paragraph(BODY, lx, y + 11.0, 15.0, 22.5, inner, 4, intro, col(c::SOFT));
    y += n_intro * 22.5 + 18.0;
    ui.text(BODY, lx, y + 8.0, 13.0, "TESTER KEY", col(c::MUTED));
    y += 16.0 + 8.0;
    let paste = "Paste";
    let pw = 36.0 + ui.text_w(BODY, 15.0, paste);
    let fw = inner - 8.0 - pw;
    ui.hit_text(lx, y, fw, 48.0, Action::KeyField);
    ui.rect(lx, y, fw, 48.0, 6.0, col(if error.is_some() { c::RED } else { 0x2e3344 }));
    ui.rect(lx + 1.0, y + 1.0, fw - 2.0, 46.0, 5.0, col(c::FIELD));
    if s.typed.is_empty() {
        ui.text(BODY, lx + 14.0, y + 24.0, 16.0, "STICK-XXXXX-XXXXX-XXXXX-XXXXX", col(0x5a5f6b));
    } else {
        let tw = ui.text(BODY, lx + 14.0, y + 24.0, 16.0, s.typed, col(c::TEXT));
        if (s.time * 1.8).fract() < 0.55 {
            ui.rect(lx + 14.0 + tw + 2.0, y + 14.0, 2.0, 20.0, 1.0, col(c::TEXT));
        }
    }
    let hover = ui.hit(lx + fw + 8.0, y, pw, 48.0, Action::Paste);
    ui.rect(lx + fw + 8.0, y, pw, 48.0, 6.0, col(if hover { 0x30364a } else { 0x252a38 }));
    ui.text(BODY, lx + fw + 8.0 + 18.0, y + 24.0, 15.0, paste, col(c::TEXT));
    y += 48.0;
    if let Some(e) = error {
        y += 8.0;
        ui.paragraph(BODY, lx, y + 10.0, 14.0, 20.0, inner, 3, e, col(c::ERROR));
        y += n_err * 20.0;
    }
    y += 18.0;
    let hover = ui.hit(lx, y, inner, 52.0, Action::Activate);
    ui.rect(lx, y, inner, 52.0, 8.0, col(if hover { c::RED_HOVER } else { c::RED }));
    ui.text_centred(BODY, lx + inner * 0.5, y + 26.0, 20.0, "Activate", Color::WHITE);
    y += 52.0 + 18.0;
    ui.paragraph(BODY, lx, y + 9.0, 13.0, 19.5, inner, 3, foot, col(c::FAINT));
}
