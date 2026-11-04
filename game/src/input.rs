//! Player input: 9 button bits plus a quantized aim offset, packed in a u32
//! (that's what goes over the wire every frame).
//!
//! Aim is the mouse position relative to the fighter's neck, in world units,
//! quantized to `AIM_UNIT`. Within `AIM_EXACT` it's sent as it is; further
//! out the distance is squeezed (never the direction), so a far cursor still
//! fits and still says how far it is. Everything inside the sim works from
//! the decoded value, so all peers see exactly the same aim.

use stickfell_app::{Input, KeyCode, Mouse};
use stickfell_physics::Vec2;

pub const LEFT: u16 = 1;
pub const RIGHT: u16 = 2;
pub const JUMP: u16 = 4;
pub const CROUCH: u16 = 8;
/// Left mouse: attack (slash with blades and blunt weapons, stab with
/// spears, punch when unarmed).
pub const ATTACK: u16 = 16;
/// Left Shift: grab with the free hand (people, ledges). Right mouse grabs
/// too while unarmed (see `ALT`).
pub const GRAB: u16 = 32;
/// E: pick up / swap / drop.
pub const PICKUP: u16 = 64;
/// Q: throw the held weapon.
pub const THROW: u16 = 128;
/// Right mouse: stab with a held weapon (a clean tip hit impales), or grab
/// when unarmed.
pub const ALT: u16 = 256;
/// F: finish off someone who's down and badly hurt.
pub const FINISH: u16 = 512;

pub const AIM_UNIT: f32 = 0.02;
/// Aim is exact out to this far (metres): arm's reach and a good way past.
const AIM_EXACT: f32 = 1.6;
/// Beyond it, the extra distance `x` is sent as `AIM_SPARE * x / (x + AIM_EASE)`
/// (approaching `AIM_SPARE`, so the sent offset stays inside what fits).
const AIM_SPARE: f32 = 0.9;
const AIM_EASE: f32 = 4.0;

/// The aim as it's sent: the same direction, the distance past `AIM_EXACT`
/// squeezed. (Plain arithmetic, so every machine gets the same bits.)
fn squeeze(aim: Vec2) -> Vec2 {
    let r = aim.length();
    if r <= AIM_EXACT {
        return aim;
    }
    let x = r - AIM_EXACT;
    aim * ((AIM_EXACT + AIM_SPARE * x / (x + AIM_EASE)) / r)
}

/// Undoes `squeeze`.
fn unsqueeze(sent: Vec2) -> Vec2 {
    let r = sent.length();
    if r <= AIM_EXACT {
        return sent;
    }
    let y = (r - AIM_EXACT).min(AIM_SPARE * 0.999);
    sent * ((AIM_EXACT + AIM_EASE * y / (AIM_SPARE - y)) / r)
}

/// Layout: buttons 0-7 in bits 0-7, aim x/y in bits 8-23, buttons 8-15 in bits 24-31.
pub fn encode(buttons: u16, aim: Vec2) -> u32 {
    let aim = squeeze(aim);
    let q = |v: f32| ((v / AIM_UNIT).round().clamp(-127.0, 127.0) as i8 as u8) as u32;
    (buttons & 0xff) as u32 | (q(aim.x) << 8) | (q(aim.y) << 16) | ((buttons >> 8) as u32) << 24
}

pub fn decode(v: u32) -> (u16, Vec2) {
    let x = ((v >> 8) & 0xff) as u8 as i8;
    let y = ((v >> 16) & 0xff) as u8 as i8;
    let buttons = (v & 0xff) as u16 | ((v >> 24) as u16) << 8;
    (buttons, unsqueeze(Vec2::new(x as f32 * AIM_UNIT, y as f32 * AIM_UNIT)))
}

/// Reads the local player's controls. `aim` is mouse-world minus neck.
pub fn read_local(input: &Input, aim: Vec2) -> u32 {
    let any = |keys: &[KeyCode]| keys.iter().any(|k| input.down(*k));
    let mut b = 0;
    if any(&[KeyCode::KeyA, KeyCode::ArrowLeft]) {
        b |= LEFT;
    }
    if any(&[KeyCode::KeyD, KeyCode::ArrowRight]) {
        b |= RIGHT;
    }
    if any(&[KeyCode::KeyW, KeyCode::Space, KeyCode::ArrowUp]) {
        b |= JUMP;
    }
    if any(&[KeyCode::KeyS, KeyCode::ControlLeft, KeyCode::ArrowDown]) {
        b |= CROUCH;
    }
    if input.mouse_down(Mouse::Left) {
        b |= ATTACK;
    }
    if input.mouse_down(Mouse::Right) {
        b |= ALT;
    }
    if input.down(KeyCode::ShiftLeft) {
        b |= GRAB;
    }
    if input.down(KeyCode::KeyE) {
        b |= PICKUP;
    }
    if input.down(KeyCode::KeyQ) {
        b |= THROW;
    }
    if input.down(KeyCode::KeyF) {
        b |= FINISH;
    }
    encode(b, aim)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let v = encode(ATTACK | LEFT | ALT | FINISH, Vec2::new(-1.234, 0.6));
        let (b, a) = decode(v);
        assert_eq!(b, ATTACK | LEFT | ALT | FINISH);
        assert!((a.x + 1.24).abs() < 1e-4 && (a.y - 0.6).abs() < 1e-4);
        // way off: still the same direction, and still a long way out
        let (_, far) = decode(encode(0, Vec2::new(99.0, -99.0)));
        assert!((far.x + far.y).abs() < 0.05 && far.x > 10.0, "{far:?}");
    }
}

#[cfg(test)]
mod aim_tests {
    use super::*;

    #[test]
    fn aim_keeps_its_direction_however_far() {
        let mut worst_deg: f32 = 0.0;
        for k in 0..400 {
            let a = k as f32 * 0.0157;
            let r = 0.3 + (k % 40) as f32 * 0.75;
            let aim = Vec2::new(a.cos(), a.sin()) * r;
            let (_, back) = decode(encode(0, aim));
            let off = (back.y.atan2(back.x) - aim.y.atan2(aim.x) + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            if r <= AIM_EXACT {
                assert!((back - aim).length() < AIM_UNIT, "close in it's exact: {aim:?} -> {back:?}");
            } else {
                assert!((back.length() / r - 1.0).abs() < 0.25, "far out it still says roughly how far: {r} -> {}", back.length());
                // (close in, the 2 cm steps decide the angle; far out it mustn't bend)
                worst_deg = worst_deg.max(off.abs().to_degrees());
            }
        }
        eprintln!("aim direction off by at most {worst_deg:.2} deg");
        assert!(worst_deg < 0.6);
    }
}
