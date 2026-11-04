//! Drawing in the launcher's design space: 1280 x 720 units, scaled to fit the window (and centred), so layouts
//! read like the design they come from. Clickable areas are noted as they're drawn, for the next tick's clicks.

use stickfell_render::{Color, Font, Frame, Vec2};

pub const W: f32 = 1280.0;
pub const H: f32 = 720.0;

/// The launcher's colours (sRGB hex).
pub mod c {
    pub const BG: u32 = 0x10121a;
    pub const BAR: u32 = 0x0b0d13;
    pub const LINE: u32 = 0x1d2130;
    pub const CARD: u32 = 0x171a24;
    pub const RAISED: u32 = 0x1d2130;
    pub const RAISED_HOVER: u32 = 0x2a2e3c;
    pub const MENU: u32 = 0x1f2331;
    pub const FIELD: u32 = 0x0f1118;
    pub const TEXT: u32 = 0xf1eee8;
    pub const SOFT: u32 = 0xc8ccd6;
    pub const MUTED: u32 = 0xa3a9b8;
    pub const FAINT: u32 = 0x8a90a0;
    pub const RED: u32 = 0xe8484f;
    pub const RED_HOVER: u32 = 0xf25a61;
    pub const ERROR: u32 = 0xff8a8f;
    pub const GREEN: u32 = 0x4cc38a;
    pub const AMBER_BG: u32 = 0x2a2212;
    pub const AMBER_LINE: u32 = 0x4a3a16;
    pub const AMBER: u32 = 0xffd28a;
    pub const AMBER_ICON: u32 = 0xffc060;
    pub const DISABLED: u32 = 0x2a2e3c;
}

pub fn col(hex: u32) -> Color {
    Color::hex(hex)
}

/// Capital height as a share of the font size (CSS px), so sizes match the design (measured against the design
/// rendered in a browser).
fn cap_share(font: Font) -> f32 {
    match font {
        Font::Title => 0.648,
        _ => 0.645,
    }
}

/// The game's fonts are ASCII only: a middle dot is drawn as a dot, an ellipsis as three full stops.
const DOT: char = '·';

fn ascii(text: &str) -> String {
    text.replace('…', "...")
}

/// A clickable area (design units) and what it does.
#[derive(Clone, Copy)]
pub struct Hit<A> {
    pub min: Vec2,
    pub size: Vec2,
    pub action: A,
    pub text: bool,
}

impl<A> Hit<A> {
    pub fn contains(&self, p: Vec2) -> bool {
        let d = p - self.min;
        d.x >= 0.0 && d.y >= 0.0 && d.x <= self.size.x && d.y <= self.size.y
    }
}

/// Screen pixels <-> design units.
#[derive(Clone, Copy, Debug, Default)]
pub struct View {
    pub k: f32,
    pub origin: Vec2,
}

impl View {
    pub fn fit(screen: Vec2) -> Self {
        let k = (screen.x / W).min(screen.y / H).max(0.01);
        Self { k, origin: (screen - Vec2::new(W, H) * k) * 0.5 }
    }
    pub fn to_design(&self, px: Vec2) -> Vec2 {
        (px - self.origin) / self.k
    }
    fn px(&self, p: Vec2) -> Vec2 {
        self.origin + p * self.k
    }
}

pub struct Ui<'a, A> {
    pub frame: &'a mut Frame,
    pub view: View,
    pub mouse: Vec2,
    pub hits: Vec<Hit<A>>,
    pub screen: Vec2,
}

impl<'a, A: Copy> Ui<'a, A> {
    pub fn new(frame: &'a mut Frame, screen: Vec2, mouse_px: Vec2) -> Self {
        let view = View::fit(screen);
        Self { frame, view, mouse: view.to_design(mouse_px), hits: Vec::new(), screen }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let k = self.view.k;
        self.frame.ui_rect(self.view.px(Vec2::new(x, y)), Vec2::new(w, h) * k, radius * k, color);
    }

    /// Fills the whole window (letterbox included).
    pub fn fill(&mut self, color: Color) {
        self.frame.ui_rect(Vec2::ZERO, self.screen, 0.0, color);
    }

    pub fn fade(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color, solid_from: f32) {
        let k = self.view.k;
        self.frame.ui_rect_fade(self.view.px(Vec2::new(x, y)), Vec2::new(w, h) * k, color, solid_from);
    }

    pub fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, color: Color) {
        let k = self.view.k;
        self.frame.ui_shadow(self.view.px(Vec2::new(x, y)), Vec2::new(w, h) * k, radius * k, blur * k, color);
    }

    pub fn line(&mut self, a: Vec2, b: Vec2, width: f32, color: Color) {
        let k = self.view.k;
        self.frame.ui_line(self.view.px(a), self.view.px(b), width * k, color);
    }

    pub fn circle(&mut self, c: Vec2, r: f32, color: Color) {
        let k = self.view.k;
        self.frame.ui_circle(self.view.px(c), r * k, color);
    }

    pub fn sprite(&mut self, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], tint: Color) {
        let k = self.view.k;
        self.frame.ui_sprite(self.view.px(Vec2::new(x, y)), Vec2::new(w, h) * k, uv, tint);
    }

    pub fn text_w(&self, font: Font, size: f32, text: &str) -> f32 {
        let scale = size * cap_share(font) / 7.0;
        let parts: Vec<String> = ascii(text).split(DOT).map(str::to_owned).collect();
        parts.iter().map(|p| Frame::text_width_in(font, p, scale)).sum::<f32>() + (parts.len() - 1) as f32 * size * 0.45
    }

    /// Text in `font` at `size` (CSS px), its capitals centred on `mid`; returns its width.
    pub fn text(&mut self, font: Font, x: f32, mid: f32, size: f32, text: &str, color: Color) -> f32 {
        let cap = size * cap_share(font);
        let k = self.view.k;
        let mut at = x;
        for (i, part) in ascii(text).split(DOT).enumerate() {
            if i > 0 {
                self.circle(Vec2::new(at + size * 0.225, mid), size * 0.075, color);
                at += size * 0.45;
            }
            self.frame.ui_text_in(font, self.view.px(Vec2::new(at, mid - cap * 0.5)), cap * k / 7.0, part, color);
            at += self.text_w(font, size, part);
        }
        at - x
    }

    /// Like [`Ui::text`], ending at `right`.
    pub fn text_right(&mut self, font: Font, right: f32, mid: f32, size: f32, text: &str, color: Color) {
        let w = self.text_w(font, size, text);
        self.text(font, right - w, mid, size, text, color);
    }

    /// Like [`Ui::text`], centred on `cx`.
    pub fn text_centred(&mut self, font: Font, cx: f32, mid: f32, size: f32, text: &str, color: Color) {
        let w = self.text_w(font, size, text);
        self.text(font, cx - w * 0.5, mid, size, text, color);
    }

    /// Wrapped text, `line` apart, at most `max_lines` (the last one ends in an ellipsis if it's cut short).
    #[allow(clippy::too_many_arguments)]
    pub fn paragraph(&mut self, font: Font, x: f32, first_mid: f32, size: f32, line: f32, width: f32, max_lines: usize, text: &str, color: Color) -> f32 {
        let mut lines = self.wrap(font, size, width, text);
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            let last = lines.last_mut().unwrap();
            while !last.is_empty() && self.text_w(font, size, &format!("{last}…")) > width {
                last.pop();
            }
            *last = format!("{}…", last.trim_end());
        }
        for (i, l) in lines.iter().enumerate() {
            self.text(font, x, first_mid + i as f32 * line, size, l, color);
        }
        lines.len() as f32 * line
    }

    fn wrap(&self, font: Font, size: f32, width: f32, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        for para in text.lines() {
            let mut cur = String::new();
            for word in para.split_whitespace() {
                let next = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
                if self.text_w(font, size, &next) > width && !cur.is_empty() {
                    lines.push(std::mem::replace(&mut cur, word.to_owned()));
                } else {
                    cur = next;
                }
            }
            lines.push(cur);
        }
        lines
    }

    pub fn hovered(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let d = self.mouse - Vec2::new(x, y);
        d.x >= 0.0 && d.y >= 0.0 && d.x <= w && d.y <= h
    }

    /// Notes a clickable area; says whether the mouse is over it.
    pub fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, action: A) -> bool {
        self.hits.push(Hit { min: Vec2::new(x, y), size: Vec2::new(w, h), action, text: false });
        self.hovered(x, y, w, h)
    }

    /// Notes a text field (an I-beam pointer over it).
    pub fn hit_text(&mut self, x: f32, y: f32, w: f32, h: f32, action: A) {
        self.hits.push(Hit { min: Vec2::new(x, y), size: Vec2::new(w, h), action, text: true });
    }
}

// ---- icons (drawn in a box at x, y of the given size) ----

pub fn icon_check<A: Copy>(ui: &mut Ui<A>, x: f32, y: f32, s: f32, color: Color) {
    let c = Vec2::new(x + s * 0.5, y + s * 0.5);
    let r = s * 0.44;
    ring(ui, c, r, s * 0.09, color);
    ui.line(c + Vec2::new(-0.2, 0.02) * s, c + Vec2::new(-0.07, 0.15) * s, s * 0.1, color);
    ui.line(c + Vec2::new(-0.07, 0.15) * s, c + Vec2::new(0.2, -0.14) * s, s * 0.1, color);
}

pub fn icon_offline<A: Copy>(ui: &mut Ui<A>, x: f32, y: f32, s: f32, color: Color) {
    let c = Vec2::new(x + s * 0.5, y + s * 0.5);
    ring(ui, c, s * 0.44, s * 0.09, color);
    ui.line(c - Vec2::new(0.19 * s, 0.0), c + Vec2::new(0.19 * s, 0.0), s * 0.1, color);
}

pub fn icon_warning<A: Copy>(ui: &mut Ui<A>, x: f32, y: f32, s: f32, color: Color) {
    let (top, l, r) = (Vec2::new(x + s * 0.5, y + s * 0.1), Vec2::new(x + s * 0.06, y + s * 0.86), Vec2::new(x + s * 0.94, y + s * 0.86));
    let w = s * 0.085;
    ui.line(top, l, w, color);
    ui.line(l, r, w, color);
    ui.line(r, top, w, color);
    ui.line(Vec2::new(x + s * 0.5, y + s * 0.38), Vec2::new(x + s * 0.5, y + s * 0.6), w * 1.1, color);
    ui.circle(Vec2::new(x + s * 0.5, y + s * 0.73), w * 0.65, color);
}

pub fn icon_dots<A: Copy>(ui: &mut Ui<A>, cx: f32, cy: f32, s: f32, color: Color) {
    for dx in [-0.29, 0.0, 0.29] {
        ui.circle(Vec2::new(cx + dx * s, cy), s * 0.085, color);
    }
}

pub fn icon_minimise<A: Copy>(ui: &mut Ui<A>, cx: f32, cy: f32, color: Color) {
    ui.line(Vec2::new(cx - 5.5, cy), Vec2::new(cx + 5.5, cy), 1.4, color);
}

pub fn icon_close<A: Copy>(ui: &mut Ui<A>, cx: f32, cy: f32, color: Color) {
    ui.line(Vec2::new(cx - 5.0, cy - 5.0), Vec2::new(cx + 5.0, cy + 5.0), 1.4, color);
    ui.line(Vec2::new(cx + 5.0, cy - 5.0), Vec2::new(cx - 5.0, cy + 5.0), 1.4, color);
}

pub fn icon_folder<A: Copy>(ui: &mut Ui<A>, x: f32, y: f32, s: f32, color: Color) {
    let p = |u: f32, v: f32| Vec2::new(x + u * s, y + v * s);
    let w = s * 0.08;
    for (a, b) in [((0.12, 0.3), (0.12, 0.8)), ((0.12, 0.8), (0.88, 0.8)), ((0.88, 0.8), (0.88, 0.38)), ((0.88, 0.38), (0.5, 0.38)), ((0.5, 0.38), (0.42, 0.24)), ((0.42, 0.24), (0.12, 0.24)), ((0.12, 0.24), (0.12, 0.3))] {
        ui.line(p(a.0, a.1), p(b.0, b.1), w, color);
    }
}

pub fn icon_shield<A: Copy>(ui: &mut Ui<A>, x: f32, y: f32, s: f32, color: Color) {
    let p = |u: f32, v: f32| Vec2::new(x + u * s, y + v * s);
    let w = s * 0.08;
    let pts = [(0.5, 0.1), (0.84, 0.24), (0.84, 0.48), (0.72, 0.72), (0.5, 0.9), (0.28, 0.72), (0.16, 0.48), (0.16, 0.24), (0.5, 0.1)];
    for pair in pts.windows(2) {
        ui.line(p(pair[0].0, pair[0].1), p(pair[1].0, pair[1].1), w, color);
    }
    ui.line(p(0.35, 0.5), p(0.46, 0.61), w, color);
    ui.line(p(0.46, 0.61), p(0.66, 0.4), w, color);
}

pub fn icon_key<A: Copy>(ui: &mut Ui<A>, x: f32, y: f32, s: f32, color: Color) {
    let p = |u: f32, v: f32| Vec2::new(x + u * s, y + v * s);
    let w = s * 0.08;
    ring(ui, p(0.32, 0.5), s * 0.17, w, color);
    ui.line(p(0.5, 0.5), p(0.9, 0.5), w, color);
    ui.line(p(0.75, 0.5), p(0.75, 0.65), w, color);
    ui.line(p(0.88, 0.5), p(0.88, 0.62), w, color);
}

/// A circle outline (as short lines).
fn ring<A: Copy>(ui: &mut Ui<A>, c: Vec2, r: f32, width: f32, color: Color) {
    let n = 24;
    for i in 0..n {
        let (a0, a1) = (i as f32 / n as f32 * std::f32::consts::TAU, (i + 1) as f32 / n as f32 * std::f32::consts::TAU);
        ui.line(c + Vec2::new(a0.cos(), a0.sin()) * r, c + Vec2::new(a1.cos(), a1.sin()) * r, width, color);
    }
}
