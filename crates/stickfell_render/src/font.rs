//! The game's fonts, baked into one signed-distance-field atlas by
//! `tools/make_font.py` (GGBotNet, CC0): FreeCheese for titles and banners,
//! Jupiteroid for everything else. Sizes are in cap heights, so a string
//! drawn at `scale` has capitals `7 * scale` px tall.

pub use crate::font_data::SPREAD_TEXELS;
use crate::font_data::{BODY, TITLE};

/// Which of the two fonts to draw with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    /// Big, chunky lettering: the game's title, banners, announcements.
    Title,
    /// Everything else: menus, the HUD, labels, help text.
    Body,
}

/// One baked font: the width of a space and the printable ASCII glyphs from
/// '!' (33) to '~' (126).
pub struct Face {
    pub space: f32,
    pub glyphs: [Glyph; 94],
}

/// One glyph. `min`/`size` are its box (with the distance-field margin)
/// relative to the pen on the baseline, y down; `uv` is x, y, w, h in the atlas.
pub struct Glyph {
    pub advance: f32,
    pub min: [f32; 2],
    pub size: [f32; 2],
    pub uv: [f32; 4],
}

/// The atlas image (8-bit grayscale PNG).
pub static ATLAS_PNG: &[u8] = include_bytes!("../assets/font-sdf.png");

/// Extra space between letters, in cap heights.
pub const TRACKING: f32 = 0.05;

fn face(font: Font) -> &'static Face {
    match font {
        Font::Title => &TITLE,
        Font::Body => &BODY,
    }
}

/// The glyph to draw for `c`, or None for
/// spaces and characters the font doesn't have.
pub fn glyph(font: Font, c: char) -> Option<&'static Glyph> {
    let i = c as usize;
    face(font).glyphs.get(i.checked_sub(33)?).filter(|g| g.size[0] > 0.0)
}

/// How far the pen moves after `c`, in cap heights.
pub fn advance(font: Font, c: char) -> f32 {
    let f = face(font);
    let i = c as usize;
    i.checked_sub(33).and_then(|i| f.glyphs.get(i)).map_or(f.space, |g| g.advance) + TRACKING
}

/// Decodes the atlas: (width, height, one byte per pixel).
pub fn atlas() -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ATLAS_PNG)).read_info().expect("font atlas");
    let mut buf = vec![0; reader.output_buffer_size().expect("font atlas size")];
    let info = reader.next_frame(&mut buf).expect("font atlas");
    assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Grayscale, png::BitDepth::Eight), "font atlas must be 8-bit grayscale");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_data::ATLAS_SIZE;

    #[test]
    fn atlas_matches_the_glyph_tables() {
        let (w, h, px) = atlas();
        assert_eq!([w as f32, h as f32], ATLAS_SIZE);
        assert_eq!(px.len(), (w * h) as usize);
        for font in [Font::Title, Font::Body] {
            for c in ('A'..='Z').chain('a'..='z').chain('0'..='9') {
                let g = glyph(font, c).expect("letters and digits are in both fonts");
                assert!(g.uv[0] + g.uv[2] <= ATLAS_SIZE[0] && g.uv[1] + g.uv[3] <= ATLAS_SIZE[1], "{font:?} {c}");
            }
            assert!(glyph(font, ' ').is_none() && advance(font, ' ') > 0.0);
        }
    }
}
