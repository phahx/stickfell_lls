//! The launcher's key art, decoded on its own thread so the window opens at once (the picture fades in when it's
//! ready).

use std::sync::{Arc, OnceLock};

use stickfell_render::SpriteAtlas;

static KEY_ART_PNG: &[u8] = include_bytes!("../assets/keyart.png");

/// Mip levels made for the picture (each half the size of the last).
const MIPS: usize = 6;

/// The key art once it's decoded (None until then, or if it couldn't be).
pub struct KeyArt(Arc<OnceLock<Option<Arc<SpriteAtlas>>>>);

impl KeyArt {
    pub fn load() -> Self {
        let cell = Arc::new(OnceLock::new());
        let fill = cell.clone();
        std::thread::spawn(move || {
            let _ = fill.set(decode().map(Arc::new));
        });
        Self(cell)
    }

    pub fn get(&self) -> Option<Arc<SpriteAtlas>> {
        self.0.get().cloned().flatten()
    }
}

fn decode() -> Option<SpriteAtlas> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(KEY_ART_PNG)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return None;
    }
    buf.truncate(info.buffer_size());
    let (mut w, mut h) = (info.width, info.height);
    let mut mips = vec![buf];
    while mips.len() < MIPS && w > 1 && h > 1 {
        mips.push(halve(mips.last()?, w, h));
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    Some(SpriteAtlas { width: info.width, height: info.height, mips })
}

/// Half the size, each pixel the alpha-weighted average of four (so see-through edges don't darken).
fn halve(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                let i = ((sy * w + sx) * 4) as usize;
                let a = px[i + 3] as u32;
                for c in 0..3 {
                    sum[c] += px[i + c] as u32 * a;
                }
                sum[3] += a;
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..3 {
                out[o + c] = if sum[3] > 0 { (sum[c] / sum[3]) as u8 } else { 0 };
            }
            out[o + 3] = (sum[3] / 4) as u8;
        }
    }
    out
}
