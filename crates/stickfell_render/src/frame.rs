//! Per-frame draw list. Fill a [`Frame`] and hand it to the renderer.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::Vec2;

use crate::font::{self, Font};

/// Linear-space RGBA color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

impl Color {
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub const fn linear(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// From an sRGB hex value like `0xff8800`.
    pub fn hex(rgb: u32) -> Self {
        let f = |s: u32| srgb_to_linear(((rgb >> s) & 0xff) as f32 / 255.0);
        Self { r: f(16), g: f(8), b: f(0), a: 1.0 }
    }

    pub fn alpha(mut self, a: f32) -> Self {
        self.a = a;
        self
    }

    /// Multiplies RGB (use for light intensity).
    pub fn scale(self, k: f32) -> Self {
        Self { r: self.r * k, g: self.g * k, b: self.b * k, a: self.a }
    }

    pub fn lerp(self, o: Color, t: f32) -> Self {
        Self {
            r: self.r + (o.r - self.r) * t,
            g: self.g + (o.g - self.g) * t,
            b: self.b + (o.b - self.b) * t,
            a: self.a + (o.a - self.a) * t,
        }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera2D {
    pub center: Vec2,
    /// Visible world height.
    pub height: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self { center: Vec2::ZERO, height: 10.0 }
    }
}

impl Camera2D {
    /// Screen pixel -> world position.
    pub fn screen_to_world(&self, px: Vec2, screen: Vec2) -> Vec2 {
        let aspect = screen.x / screen.y.max(1.0);
        let ndc = Vec2::new(px.x / screen.x * 2.0 - 1.0, 1.0 - px.y / screen.y * 2.0);
        self.center + ndc * Vec2::new(self.height * aspect, self.height) * 0.5
    }
}

/// Point light (or spot light when `cone > -1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub pos: Vec2,
    pub radius: f32,
    /// Linear color premultiplied by intensity.
    pub color: Color,
    /// Spot direction (unit). Ignored for point lights.
    pub dir: Vec2,
    /// Cosine of the spot half-angle; -1 = omnidirectional.
    pub cone: f32,
    pub shadows: bool,
    /// How big the glowing part is (radius, m): the bigger, the softer its
    /// shadows' edges, which widen with distance from whatever casts them.
    pub size: f32,
}

impl Light {
    pub fn point(pos: Vec2, radius: f32, color: Color) -> Self {
        Self { pos, radius, color, dir: Vec2::NEG_Y, cone: -1.0, shadows: true, size: Self::SIZE }
    }

    pub fn spot(pos: Vec2, radius: f32, color: Color, dir: Vec2, half_angle: f32) -> Self {
        Self { pos, radius, color, dir: dir.normalize_or_zero(), cone: half_angle.cos(), shadows: true, size: Self::SIZE }
    }

    /// A bulb-sized source.
    pub const SIZE: f32 = 0.12;

    /// Sets how big the glowing part is (see [`Light::size`]).
    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(0.0);
        self
    }

    pub fn no_shadows(mut self) -> Self {
        self.shadows = false;
        self
    }
}

/// Shadow caster geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Occluder {
    /// Oriented box; `rot` is (cos, sin).
    Box { center: Vec2, half: Vec2, rot: Vec2 },
    Segment { a: Vec2, b: Vec2 },
    Circle { center: Vec2, radius: f32 },
}

/// A level's solid boxes, for lighting worked out once per level (baked):
/// soft contact shadows on the wall around and between them. Keep it in an
/// `Arc` that lasts: the renderer remembers bakes by that pointer.
#[derive(Clone, Debug, Default)]
pub struct StaticScene {
    /// The area to bake (anything outside stays unshadowed).
    pub min: Vec2,
    pub max: Vec2,
    /// Boxes as (centre, half size, rotation (cos, sin)).
    pub boxes: Vec<(Vec2, Vec2, Vec2)>,
}

pub(crate) const KIND_CIRCLE: f32 = 0.0;
pub(crate) const KIND_CAPSULE: f32 = 1.0;
pub(crate) const KIND_BOX: f32 = 2.0;
/// A font glyph: `b` is its atlas rect's corner and `rot` its size (texels).
pub(crate) const KIND_GLYPH: f32 = 3.0;
/// A textured quad: `b` is its atlas rect's corner, `params.zw` its size (texels).
pub(crate) const KIND_SPRITE: f32 = 4.0;
pub(crate) const KIND_RING_CORNERS: f32 = 5.0;

/// An RGBA image (sRGB, straight alpha) that [`Frame::sprite`] draws from,
/// with its mip chain (level 0 first, each half the size of the last).
pub struct SpriteAtlas {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<Vec<u8>>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct ShapeInstance {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub size: [f32; 2],
    pub rot: [f32; 2],
    pub color: [f32; 4],
    /// kind, emissive, corner radius, unused
    pub params: [f32; 4],
}

/// Everything to draw this frame.
pub struct Frame {
    pub camera: Camera2D,
    /// Background (albedo) clear color.
    pub clear: Color,
    /// Light every pixel receives regardless of lights.
    pub ambient: Color,
    /// Seconds, passed to shaders for animated effects.
    pub time: f32,
    /// Where the cursor is right now (physical pixels), updated every
    /// rendered frame rather than every tick, so a drawn cursor keeps up
    /// with the display.
    pub mouse: Vec2,
    /// Where the cursor has been lately, oldest first, as (position, seconds
    /// ago), ending at `mouse`. Every move the OS reported, so it holds the
    /// positions between frames too (for drawing a motion-blurred cursor).
    pub mouse_trail: Vec<(Vec2, f32)>,
    pub(crate) shapes: Vec<ShapeInstance>,
    pub(crate) ui: Vec<ShapeInstance>,
    pub(crate) lights: Vec<Light>,
    pub(crate) occluders: Vec<Occluder>,
    /// The image sprites come from. It's uploaded when a different one is
    /// set and stays between frames.
    pub sprite_atlas: Option<Arc<SpriteAtlas>>,
    /// The level being shown, for its baked contact shadows (baked the first
    /// time it's seen, so show each level once while loading).
    pub statics: Option<Arc<StaticScene>>,
    /// Whether the renderer's shaders are compiled yet (set by the app before
    /// the game draws; until then only the UI is drawn, e.g. a loading screen).
    pub shaders_ready: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new(Camera2D::default())
    }
}

impl Frame {
    pub fn new(camera: Camera2D) -> Self {
        Self {
            camera,
            clear: Color::hex(0x202430),
            ambient: Color::linear(0.25, 0.25, 0.3, 1.0),
            time: 0.0,
            mouse: Vec2::ZERO,
            mouse_trail: Vec::new(),
            shapes: Vec::new(),
            ui: Vec::new(),
            lights: Vec::new(),
            occluders: Vec::new(),
            sprite_atlas: None,
            statics: None,
            shaders_ready: true,
        }
    }

    pub fn clear_lists(&mut self) {
        self.shapes.clear();
        self.ui.clear();
        self.lights.clear();
        self.occluders.clear();
    }

    // ---- world shapes (lit) --------------------------------------------

    pub fn circle(&mut self, c: Vec2, r: f32, color: Color) {
        self.circle_glow(c, r, color, 0.0);
    }

    /// `emissive` adds `color * emissive` on top of lighting (glows in the dark).
    pub fn circle_glow(&mut self, c: Vec2, r: f32, color: Color, emissive: f32) {
        self.shapes.push(ShapeInstance {
            a: c.into(),
            b: c.into(),
            size: [r, 0.0],
            rot: [1.0, 0.0],
            color: color.to_array(),
            params: [KIND_CIRCLE, emissive, 0.0, 0.0],
        });
    }

    pub fn capsule(&mut self, a: Vec2, b: Vec2, r: f32, color: Color) {
        self.capsule_glow(a, b, r, color, 0.0);
    }

    pub fn capsule_glow(&mut self, a: Vec2, b: Vec2, r: f32, color: Color, emissive: f32) {
        self.shapes.push(ShapeInstance {
            a: a.into(),
            b: b.into(),
            size: [r, 0.0],
            rot: [1.0, 0.0],
            color: color.to_array(),
            params: [KIND_CAPSULE, emissive, 0.0, 0.0],
        });
    }

    /// Oriented rounded box; `rot` is (cos, sin).
    pub fn rect(&mut self, center: Vec2, half: Vec2, rot: Vec2, corner: f32, color: Color) {
        self.rect_glow(center, half, rot, corner, color, 0.0);
    }

    pub fn rect_glow(&mut self, center: Vec2, half: Vec2, rot: Vec2, corner: f32, color: Color, emissive: f32) {
        self.shapes.push(ShapeInstance {
            a: center.into(),
            b: center.into(),
            size: half.into(),
            rot: rot.into(),
            color: color.to_array(),
            params: [KIND_BOX, emissive, corner.min(half.x).min(half.y), 0.0],
        });
    }

    // ---- lighting -------------------------------------------------------

    /// Number of world shapes and occluders queued (for profiling).
    /// A textured quad from the sprite atlas, lit and shadowed like the
    /// other shapes. `uv` is the atlas rect (x, y, w, h in pixels); its top
    /// edge ends up on the `+perp(rot)` side and its left edge on `-rot`.
    /// The texture's colour is multiplied by `tint`.
    pub fn sprite(&mut self, center: Vec2, half: Vec2, rot: Vec2, uv: [f32; 4], tint: Color) {
        self.shapes.push(ShapeInstance {
            a: center.into(),
            b: [uv[0], uv[1]],
            size: half.into(),
            rot: rot.into(),
            color: tint.to_array(),
            params: [KIND_SPRITE, 0.0, uv[2], uv[3]],
        });
    }

    /// A sprite that also glows (blooms) by `glow`.
    pub fn sprite_glow(&mut self, center: Vec2, half: Vec2, rot: Vec2, uv: [f32; 4], tint: Color, glow: f32) {
        self.sprite(center, half, rot, uv, tint);
        if let Some(s) = self.shapes.last_mut() {
            s.params[1] = glow;
        }
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.shapes.len(), self.occluders.len())
    }

    pub fn light(&mut self, l: Light) {
        self.lights.push(l);
    }

    pub fn occluder(&mut self, o: Occluder) {
        self.occluders.push(o);
    }

    // ---- UI (screen pixels, origin top-left, unlit) --------------------

    pub fn ui_rect(&mut self, min: Vec2, size: Vec2, corner: f32, color: Color) {
        let half = size * 0.5;
        self.ui.push(ShapeInstance {
            a: (min + half).into(),
            b: (min + half).into(),
            size: half.into(),
            rot: [1.0, 0.0],
            color: color.to_array(),
            params: [KIND_BOX, 0.0, corner.min(half.x).min(half.y), 0.0],
        });
    }

    pub fn ui_circle(&mut self, c: Vec2, r: f32, color: Color) {
        self.ui.push(ShapeInstance {
            a: c.into(),
            b: c.into(),
            size: [r, 0.0],
            rot: [1.0, 0.0],
            color: color.to_array(),
            params: [KIND_CIRCLE, 0.0, 0.0, 0.0],
        });
    }

    /// A ring of radius `r` kept only at its four diagonal corners: arcs
    /// `arc` radians long, lines `width` pixels thick.
    pub fn ui_ring_corners(&mut self, c: Vec2, r: f32, width: f32, arc: f32, color: Color) {
        self.ui.push(ShapeInstance {
            a: c.into(),
            b: c.into(),
            size: [r, 0.0],
            rot: [1.0, 0.0],
            color: color.to_array(),
            params: [KIND_RING_CORNERS, 0.0, width * 0.5, (arc * 0.5).clamp(0.0, std::f32::consts::FRAC_PI_4)],
        });
    }

    /// Draws text in the body font with its capitals' tops at `pos.y`.
    /// `scale` keeps the old pixel font's sizing: capitals are `7 * scale`
    /// px tall. Returns the width.
    pub fn ui_text(&mut self, pos: Vec2, scale: f32, text: &str, color: Color) -> f32 {
        self.ui_text_in(Font::Body, pos, scale, text, color)
    }

    /// [`Frame::ui_text`] in a chosen font.
    pub fn ui_text_in(&mut self, font: Font, pos: Vec2, scale: f32, text: &str, color: Color) -> f32 {
        let cap = 7.0 * scale;
        let baseline = pos.y + cap;
        let mut x = pos.x;
        for ch in text.chars() {
            if let Some(g) = font::glyph(font, ch) {
                let half = Vec2::new(g.size[0], g.size[1]) * cap * 0.5;
                let center = Vec2::new(x + g.min[0] * cap, baseline + g.min[1] * cap) + half;
                self.ui.push(ShapeInstance {
                    a: center.into(),
                    b: [g.uv[0], g.uv[1]],
                    size: half.into(),
                    rot: [g.uv[2], g.uv[3]],
                    color: color.to_array(),
                    params: [KIND_GLYPH, 0.0, font::SPREAD_TEXELS, 0.0],
                });
            }
            x += font::advance(font, ch) * cap;
        }
        Self::text_width_in(font, text, scale)
    }

    /// Width of `text` in the body font at `scale` (see [`Frame::ui_text`]).
    pub fn text_width(text: &str, scale: f32) -> f32 {
        Self::text_width_in(Font::Body, text, scale)
    }

    /// [`Frame::text_width`] in a chosen font.
    pub fn text_width_in(font: Font, text: &str, scale: f32) -> f32 {
        let w: f32 = text.chars().map(|c| font::advance(font, c)).sum();
        (w - font::TRACKING).max(0.0) * 7.0 * scale
    }
}
