//! # stickfell_render
//!
//! A small wgpu 2D renderer built around three ideas:
//! * everything is an anti-aliased SDF shape (circle, capsule, rounded box),
//! * the scene is lit by dynamic point/spot lights with stencil shadow volumes
//!   cast by [`Occluder`]s (boxes, segments, circles),
//! * emissive shapes glow regardless of lighting,
//! * UI text comes from a signed-distance-field font atlas (see `tools/make_font.py`),
//! * textured sprites from a [`SpriteAtlas`] are lit and shadowed like shapes.
//!
//! Fill a [`Frame`] each frame and pass it to [`Renderer::render`].

mod font;
mod font_data;
mod frame;
mod renderer;

pub use font::Font;
pub use frame::{Camera2D, Color, Frame, Light, LiquidLook, Occluder, SpriteAtlas, StaticScene, VapourField};
pub use glam::Vec2;
pub use renderer::Renderer;
pub use wgpu;

/// Distance from `p` to segment a-b and the parameter of the closest point.
pub(crate) fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 > 1e-12 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((a + ab * t - p).length(), t)
}
