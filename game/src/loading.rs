//! The loading screen shown when the game starts. Behind it, the game:
//! compiles the renderer's shaders, decodes the character and level art and
//! every sound (all on other threads, so the window stays responsive), then
//! shows each level once, hidden under the screen, a level a frame: that bakes
//! the level's contact shadows and runs every shader the level uses, so none
//! of it costs a hitch the first time it's seen in play.

use std::thread::JoinHandle;

use stickfell_physics::Vec2;
use stickfell_render::{Color, Font, Frame};

use crate::audio::{Audio, Sounds};
use crate::draw::{self, Hud};
use crate::level::LevelDef;
use crate::sim::GameState;

/// The bar never fills faster than this (fraction a second), so a quick load
/// still reads as a bar filling rather than a flash.
const FILL_RATE: f32 = 2.5;
/// How long the full bar stays up, and the fade into the game after (s).
const HOLD: f32 = 0.2;
pub const FADE: f32 = 0.35;

pub struct Loading {
    art: Option<JoinHandle<()>>,
    sounds: Option<JoinHandle<Sounds>>,
    /// Levels shown so far.
    levels_done: usize,
    shown: f32,
    held: f32,
}

impl Loading {
    pub fn start() -> Self {
        let art = std::thread::spawn(|| {
            crate::character::skin();
        });
        let sounds = std::thread::spawn(Sounds::decode);
        Self { art: Some(art), sounds: Some(sounds), levels_done: 0, shown: 0.0, held: 0.0 }
    }

    /// Takes the next loading step, draws the loading screen over it, and says
    /// whether it's all done (the game can start).
    pub fn draw(&mut self, frame: &mut Frame, screen: Vec2, dt: f32, levels: &[LevelDef], audio: &mut Option<Audio>) -> bool {
        if self.art.as_ref().is_some_and(|t| t.is_finished()) {
            if let Some(t) = self.art.take() {
                let _ = t.join();
            }
        }
        if self.sounds.as_ref().is_some_and(|t| t.is_finished()) {
            if let Some(Ok(sounds)) = self.sounds.take().map(|t| t.join()) {
                *audio = Audio::open(sounds);
            }
        }
        let art_ready = self.art.is_none();
        let shaders = frame.shaders_ready;
        let levels_left = art_ready && shaders && self.levels_done < levels.len();
        if levels_left {
            // this level, drawn under the loading screen
            let lv = &levels[self.levels_done];
            let game = GameState::new(4, self.levels_done, 1);
            let centre = (lv.min + lv.max) * 0.5;
            draw::draw_game(&game, lv, frame, screen, centre, &Hud { world_only: true, ..Default::default() });
            self.levels_done += 1;
        }
        let steps = 3 + levels.len();
        let done = shaders as usize + art_ready as usize + self.sounds.is_none() as usize + self.levels_done;
        self.shown = (self.shown + FILL_RATE * dt).min(done as f32 / steps as f32);
        if done == steps && self.shown >= 1.0 {
            self.held += dt;
        }
        let status = match () {
            _ if !shaders => "Compiling shaders",
            _ if !art_ready => "Loading art",
            _ if levels_left => "Lighting the levels",
            _ if self.sounds.is_some() => "Loading sounds",
            _ => "Ready",
        };
        self.draw_screen(frame, screen, status);
        done == steps && self.held >= HOLD
    }

    fn draw_screen(&self, frame: &mut Frame, screen: Vec2, status: &str) {
        let k = (screen.y / 720.0).clamp(0.6, 3.0);
        frame.ui_rect(Vec2::ZERO, screen, 0.0, Color::hex(0x10121a));
        let title = |frame: &mut Frame, y: f32, scale: f32, text: &str, color: Color| {
            let w = Frame::text_width_in(Font::Title, text, scale);
            frame.ui_text_in(Font::Title, Vec2::new((screen.x - w) * 0.5, y), scale, text, color);
        };
        title(frame, screen.y * 0.3, 12.0 * k, "STICKFELL", Color::hex(0xe8484f));
        title(frame, screen.y * 0.3 + 96.0 * k, 3.2 * k, "LAST LIMB STANDING", Color::hex(0xc8ccd6));
        let (bw, bh) = (420.0 * k, 10.0 * k);
        let at = Vec2::new((screen.x - bw) * 0.5, screen.y * 0.66);
        frame.ui_rect(at, Vec2::new(bw, bh), bh * 0.5, Color::hex(0x23262f));
        if self.shown > 0.0 {
            frame.ui_rect(at, Vec2::new((bw * self.shown).max(bh), bh), bh * 0.5, Color::hex(0xe8484f));
        }
        let scale = 1.6 * k;
        let w = Frame::text_width(status, scale);
        frame.ui_text(Vec2::new((screen.x - w) * 0.5, at.y + bh + 16.0 * k), scale, status, Color::hex(0x8a90a0));
    }
}
