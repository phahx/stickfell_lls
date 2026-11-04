//! Sound playback. Turns the simulation's `SfxEvent`s into positioned sounds
//! (left/right pan and distance falloff around the camera). Sounds are baked
//! into the exe and decoded once at startup. If there's no audio device the
//! game just runs silent.

use std::io::Cursor;
use std::num::NonZero;

use rodio::source::ChannelVolume;
use rodio::static_buffer::StaticSamplesBuffer;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Source};
use stickfell_physics::Vec2;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::sfx::{Sfx, SfxEvent};
use crate::voice;

macro_rules! sounds {
    ($($name:literal),* $(,)?) => {
        &[$(include_bytes!(concat!("../assets/sfx/", $name)) as &[u8]),*]
    };
}

/// The files that can play for each kind of event (one is picked at random).
/// Footsteps and landings have none: they're cut from other sounds (`cuts`).
fn files(kind: Sfx) -> &'static [&'static [u8]] {
    match kind {
        Sfx::Swing => sounds!("universal_swing_miss_light_ufx_1.ogg", "universal_swing_miss_light_ufx_2.ogg"),
        Sfx::SwingHeavy => sounds!(
            "universal_swing_miss_heavy_ufx_1.ogg",
            "universal_swing_miss_heavy_ufx_2.ogg",
            "universal_swing_miss_heavy_ufx_3.ogg"
        ),
        Sfx::Stab => sounds!("knife_swing_miss_ufx_1.ogg"),
        Sfx::HitBlade => sounds!(
            "slash_bloody_heavy_ufx_1.ogg",
            "slash_bloody_heavy_ufx_2.ogg",
            "sharp_slash_body_ufx_1.ogg",
            "slash_bloody_light_ufx_1.ogg"
        ),
        Sfx::HitBlunt => sounds!("blunt_hit_body_ufx_1.ogg", "blunt_hit_body_ufx_2.ogg", "blunt_hit_body_ufx_3.ogg", "blunt_hit_body_ufx_4.ogg"),
        Sfx::HitHeavy => sounds!("blunt_hit_bloody_heavy_ufx_1.ogg", "blunt_hit_heavy_ufx_1.ogg", "blunt_hit_bloody_moderate_ufx_1.ogg"),
        Sfx::Punch => sounds!("blunt_hit_soft_ufx_1.ogg", "blunt_hit_bloody_light_ufx_1.ogg", "blunt_hit_bloody_light_ufx_2.ogg"),
        Sfx::Impale => sounds!(
            "sharp_critical_hit_body_ufx_1.ogg",
            "sharp_critical_hit_body_ufx_2.ogg",
            "sticky_hit_body_ufx_1.ogg"
        ),
        Sfx::Sever => sounds!("sharp_hit_heavy_ufx_1.ogg", "slash_bloody_heavy_ufx_3.ogg"),
        Sfx::Clash => sounds!("sword_clash_ufx_1.ogg", "sword_hit_ufx_1.ogg", "blunt_hit_metal_ufx_1.ogg"),
        Sfx::Embed => sounds!("knife_hit_hard_surface_ufx_1.ogg", "knife_hit_hard_surface_ufx_2.ogg", "knife_hit_hard_surface_ufx_3.ogg"),
        Sfx::Unembed => sounds!("sword_draw_ufx_1.ogg", "sword_draw_ufx_2.ogg"),
        Sfx::Pickup => sounds!("item_equip_ufx_1.ogg", "item_pickup_ufx_1.ogg"),
        Sfx::Throw => sounds!("throw_heavy_ufx_1.ogg", "throw_light_ufx_1.ogg"),
        // (one take per gun, so every shot from it sounds the same)
        Sfx::Shot => sounds!("revolver_fire_ufx_1.ogg"),
        Sfx::PistolShot => sounds!("pistol_fire_ufx_1.ogg"),
        Sfx::Ricochet => sounds!("bullet_ricochet_ufx_3.wav", "bullet_ricochet_ufx_4.wav"),
        Sfx::BulletHit => sounds!("impact_body_ufx_1.ogg", "impact_body_ufx_2.ogg"),
        Sfx::Empty => sounds!("pistol_universal_empty_barrel_ufx_1.ogg"),
        Sfx::Footstep | Sfx::Land => &[],
        // a body slamming into a wall or the floor
        Sfx::Thud => sounds!("impact_body_ufx_1.ogg", "impact_body_ufx_2.ogg", "blunt_hit_heavy_ufx_1.ogg"),
        Sfx::Saw => sounds!("impact_metal_ufx_1.ogg", "impact_metal_ufx_2.ogg", "impact_metal_ufx_3.ogg"),
        Sfx::Spikes => sounds!("sticky_hit_body_ufx_1.ogg", "sticky_hit_body_ufx_2.ogg"),
        Sfx::Splash => sounds!("impact_body_ufx_1.ogg"),
        // voice lines live in `voice` (see `Voices`)
        Sfx::Announce | Sfx::Plea | Sfx::Quip | Sfx::Choke | Sfx::ChokeEnd | Sfx::Pain => &[],
    }
}

/// Finisher voice lines, decoded once.
struct Voices {
    /// Per `Finish` (empty if that one wasn't recorded).
    announce: Vec<Vec<Clip>>,
    /// Per recorded voice (players beyond them share): per `Finish`.
    quips: Vec<Vec<Vec<Clip>>>,
    quips_any: Vec<Vec<Clip>>,
    pleas: Vec<Vec<Clip>>,
    /// Per recorded voice: choking sounds looped while hanging, and the last one.
    chokes: Vec<Vec<Clip>>,
    choke_end: Vec<Vec<Clip>>,
    /// Per recorded voice: cries of pain (hurt mid-finisher).
    pains: Vec<Vec<Clip>>,
    /// Which line plays, when, and at what pitch (the same for every player).
    lines: Lines,
    /// The line each fighter is saying right now (and what kind), steered
    /// every frame.
    live: [Option<(Arc<VoiceCtl>, Sfx)>; 4],
}

/// Decides voice lines from the frame they're said on alone, so every
/// player, hearing the same final frames in the same order, makes the same
/// choices: which take, what pitch, and whether the speaker is still busy
/// with their last line.
#[derive(Default)]
struct Lines {
    /// Frames heard up to (lines from later frames are still to come).
    heard: u32,
    /// The frame each speaker is free to talk again (no talking over yourself).
    busy_until: [u32; 4],
    /// The last take per (kind, voice), so it isn't picked again next.
    last: std::collections::HashMap<(usize, usize), usize>,
}

impl Lines {
    /// For a line said on `frame`: which of `pool` (by index), at what
    /// pitch, or `None` if the speaker is still talking. `seconds` is each
    /// take's length.
    fn decide(&mut self, frame: u32, e: SfxEvent, voices: usize, seconds: &[f32], ids: &[usize]) -> Option<(usize, f32)> {
        let slot = e.speaker as usize % 4;
        let fighter = e.kind != Sfx::Announce;
        // (the death rattle cuts in over whatever choke was playing, and
        // pain over anything: they're free to talk again once it's out)
        let cuts_in = matches!(e.kind, Sfx::ChokeEnd | Sfx::Pain);
        if fighter && frame < self.busy_until[slot] && !cuts_in {
            return None;
        }
        if e.kind == Sfx::Pain {
            self.busy_until[slot] = frame;
        }
        if seconds.is_empty() {
            return None;
        }
        let h = line_hash(frame, e);
        // never the same take twice in a row (from this voice) if there's another
        let key = (e.kind as usize, if fighter { e.speaker as usize % voices.max(1) } else { 0 });
        let last = self.last.get(&key).copied();
        let ok: Vec<usize> = (0..ids.len()).filter(|&i| Some(ids[i]) != last).collect();
        let pick = if ok.is_empty() { h as usize % ids.len() } else { ok[h as usize % ok.len()] };
        self.last.insert(key, ids[pick]);
        // players sharing a recorded voice are told apart by pitch, and a
        // touch of variety either way
        let shared = if voices < 4 { [1.0, 0.94, 1.06, 0.97][slot] } else { 1.0 };
        let jitter = ((h >> 16) % 1000) as f32 / 1000.0;
        let pitch = if fighter { shared + (jitter - 0.5) * 0.04 } else { 1.0 };
        if fighter {
            self.busy_until[slot] = frame + (seconds[pick] / pitch * 60.0).ceil() as u32;
        }
        Some((pick, pitch))
    }
}

/// A well-mixed number from a line and the frame it was said on.
fn line_hash(frame: u32, e: SfxEvent) -> u32 {
    let mut h = frame.wrapping_mul(0x9E37_79B1) ^ (e.kind as u32).wrapping_mul(0x85EB_CA77) ^ ((e.speaker as u32) << 8 | e.variant as u32).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// One fighter, as the voice player sees them: where their mouth is, and
/// whether they're alive and strung up choking.
#[derive(Clone, Copy, Debug)]
pub struct Speaker {
    pub mouth: Vec2,
    pub alive: bool,
    pub choking: bool,
}

/// Live controls for one voice line: where it sits (left/right gain and
/// distance, set every frame from the speaker's position) and whether to cut
/// it off.
#[derive(Default)]
struct VoiceCtl {
    left: AtomicU32,
    right: AtomicU32,
    dist: AtomicU32,
    /// How loud the line is right now, before panning (for the mouth).
    level: AtomicU32,
    stop: AtomicBool,
    finished: AtomicBool,
}

impl VoiceCtl {
    fn set(&self, left: f32, right: f32, dist: f32) {
        self.left.store(left.to_bits(), Ordering::Relaxed);
        self.right.store(right.to_bits(), Ordering::Relaxed);
        self.dist.store(dist.to_bits(), Ordering::Relaxed);
    }

    fn get(&self) -> (f32, f32, f32) {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        (f(&self.left), f(&self.right), f(&self.dist))
    }
}

/// A mixer for every game sound, and its output (limited, then held to
/// full scale) to hand the speakers. It always has an endless silence
/// playing: an empty mixer reports itself finished, and the speakers would
/// drop it before the first sound ever came.
fn limited_bus(channels: rodio::ChannelCount, rate: rodio::SampleRate) -> (rodio::mixer::Mixer, Safety<impl Source>) {
    let (bus, mixed) = rodio::mixer::mixer(channels, rate);
    bus.add(rodio::source::Zero::new(channels, rate));
    (bus, Safety(mixed.limit(rodio::source::LimitSettings::dynamic_content())))
}

/// The very last stage before the speakers: whatever the limiter let
/// through in its first instant is held to full scale, and a sample that
/// isn't a number becomes silence (one bad sample would mute the whole mix).
struct Safety<S: Source>(S);

impl<S: Source> Iterator for Safety<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        // (and never finishes: the speakers would drop it)
        Some(self.0.next().map_or(0.0, |v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }))
    }
}

impl<S: Source> Source for Safety<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.0.current_span_len()
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.0.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.0.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// A feedback comb with a damped (low-passed) loop: the body of the reverb.
struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], i: 0, store: 0.0 }
    }
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.i];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.i] = x + self.store * feedback;
        self.i = (self.i + 1) % self.buf.len();
        out
    }
}

/// An allpass diffuser: smears the combs' echoes into a smooth tail.
struct Allpass {
    buf: Vec<f32>,
    i: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], i: 0 }
    }
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        b - x
    }
}

/// A small stereo room (Freeverb-style: parallel damped combs into series
/// allpasses, the right side slightly longer so the tail is wide).
struct Room {
    combs: [Vec<Comb>; 2],
    passes: [Vec<Allpass>; 2],
    feedback: f32,
    damp: f32,
}

impl Room {
    /// `size` scales the delay lengths (1 = a medium hall at 44.1 kHz).
    fn new(rate: u32, size: f32, feedback: f32, damp: f32) -> Self {
        let s = |n: usize, spread: usize| ((n + spread) as f32 * size * rate as f32 / 44_100.0) as usize;
        let side = |spread| {
            ([1116, 1188, 1277, 1356].map(|n| Comb::new(s(n, spread))).into(), [556, 441].map(|n| Allpass::new(s(n, spread))).into())
        };
        let (cl, pl) = side(0);
        let (cr, pr) = side(23);
        Self { combs: [cl, cr], passes: [pl, pr], feedback, damp }
    }

    fn tick(&mut self, x: f32) -> (f32, f32) {
        let mut out = [0.0; 2];
        for side in 0..2 {
            let mut y = 0.0;
            for c in &mut self.combs[side] {
                y += c.tick(x, self.feedback, self.damp);
            }
            y *= 0.25;
            for a in &mut self.passes[side] {
                y = a.tick(y);
            }
            out[side] = y;
        }
        (out[0], out[1])
    }
}

/// How a voice line sounds in the world.
#[derive(Clone, Copy, PartialEq)]
enum Space {
    /// Someone in the arena: panned to them, duller and roomier with distance.
    Arena,
    /// The announcer, over the arena's speakers: mid-heavy, a little driven,
    /// a big hall echo, dead centre.
    PublicAddress,
}

/// A voice line placed in the arena. Mixes the clip to mono, takes the
/// close-mic boom off, dulls it with distance, adds the room (more of it
/// the further away), pans it with gains read from `VoiceCtl` every few
/// milliseconds (eased, so moving never clicks), and lets the room ring out
/// after the last word. Told to stop, it fades out over ~25 ms.
struct Voice<S: Source> {
    inner: S,
    ctl: Arc<VoiceCtl>,
    space: Space,
    rate: f32,
    /// eased left/right gain and distance
    gain: (f32, f32),
    dist: f32,
    /// filter state: high-pass (in, out), low-pass
    hp: (f32, f32),
    lp: f32,
    lp2: f32,
    room: Room,
    /// frames of room tail left once the clip has run out
    tail: Option<usize>,
    /// peak follower on the words, for lip flap
    env: f32,
    fade: f32,
    frame: [f32; 2],
    at: usize,
}

impl<S: Source> Voice<S> {
    fn new(inner: S, ctl: Arc<VoiceCtl>, space: Space) -> Self {
        let (l, r, d) = ctl.get();
        let rate = inner.sample_rate().get();
        let room = match space {
            Space::Arena => Room::new(rate, 0.55, 0.72, 0.35),
            Space::PublicAddress => Room::new(rate, 1.1, 0.8, 0.25),
        };
        Self { inner, ctl, space, rate: rate as f32, gain: (l, r), dist: d, hp: (0.0, 0.0), lp: 0.0, lp2: 0.0, room, tail: None, env: 0.0, fade: 1.0, frame: [0.0; 2], at: 2 }
    }

    /// One-pole coefficient for a cutoff in Hz.
    fn coef(&self, hz: f32) -> f32 {
        1.0 - (-core::f32::consts::TAU * hz / self.rate).exp()
    }

    /// The next mono input sample, or `None` once the clip is used up.
    fn input(&mut self) -> Option<f32> {
        let ch = self.inner.channels().get() as usize;
        let mut sum = 0.0;
        for i in 0..ch {
            match self.inner.next() {
                Some(v) => sum += v,
                None if i == 0 => return None,
                None => {}
            }
        }
        Some(sum / ch as f32)
    }

    fn process(&mut self, x: f32) -> [f32; 2] {
        let (tl, tr, td) = self.ctl.get();
        let ease = 1.0 - (-1.0 / (0.02 * self.rate)).exp(); // ~20 ms
        self.gain.0 += (tl - self.gain.0) * ease;
        self.gain.1 += (tr - self.gain.1) * ease;
        self.dist += (td - self.dist) * ease;
        let far = (self.dist / VOICE_SILENT).clamp(0.0, 1.0);
        let (low_cut, high_cut, wet) = match self.space {
            // close: bright and fairly dry; far: muffled and mostly room
            Space::Arena => (140.0, 9_000.0 - 6_500.0 * far, 0.16 + 0.34 * far),
            Space::PublicAddress => (260.0, 4_800.0, 0.32),
        };
        // high-pass (no proximity boom), then two low-pass poles (air)
        let a = 1.0 - self.coef(low_cut);
        self.hp.1 = a * (self.hp.1 + x - self.hp.0);
        self.hp.0 = x;
        let k = self.coef(high_cut);
        self.lp += (self.hp.1 - self.lp) * k;
        self.lp2 += (self.lp - self.lp2) * k;
        let mut dry = self.lp2;
        if self.space == Space::PublicAddress {
            dry = (dry * 2.2).tanh() / 2.2_f32.tanh(); // a loudspeaker pushed a bit
        }
        let (rl, rr) = self.room.tick(dry);
        // the room comes from all around, not from where they stand
        let around = (self.gain.0 + self.gain.1) * 0.5;
        let f = self.fade.max(0.0);
        [(dry * self.gain.0 * (1.0 - wet * 0.5) + rl * wet * around) * f, (dry * self.gain.1 * (1.0 - wet * 0.5) + rr * wet * around) * f]
    }
}

impl<S: Source> Iterator for Voice<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.at == 2 {
            if self.fade <= 0.0 || self.tail == Some(0) {
                self.ctl.finished.store(true, Ordering::Relaxed);
                return None;
            }
            let x = match self.tail {
                Some(ref mut left) => {
                    *left -= 1;
                    0.0
                }
                None => match self.input() {
                    Some(x) => x,
                    None => {
                        // the words are done: let the room ring out
                        self.tail = Some((self.rate * 0.7) as usize);
                        0.0
                    }
                },
            };
            if self.ctl.stop.load(Ordering::Relaxed) {
                self.fade -= 1.0 / (0.025 * self.rate);
            }
            // fast attack, ~60 ms release: syllables open and close the mouth
            let release = (-1.0 / (0.06 * self.rate)).exp();
            self.env = x.abs().max(self.env * release) * self.fade.max(0.0);
            self.ctl.level.store(self.env.to_bits(), Ordering::Relaxed);
            self.frame = self.process(x);
            self.at = 0;
        }
        let v = self.frame[self.at];
        self.at += 1;
        Some(v)
    }
}

impl<S: Source> Source for Voice<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(2).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Drops the line that played last from `pool`, unless it's the only one.
/// Voices carry this far at full volume, then fade out to nothing by
/// `VOICE_SILENT` (metres from the camera): talking is for whoever's nearby,
/// not the whole map.
const VOICE_FULL: f32 = 5.0;
const VOICE_SILENT: f32 = 16.0;

/// Left/right gain for a voice at `pos` heard from `listener`.
fn voice_gains(pos: Vec2, listener: Vec2, volume: f32) -> (f32, f32) {
    let d = pos - listener;
    let t = ((d.length() - VOICE_FULL) / (VOICE_SILENT - VOICE_FULL)).clamp(0.0, 1.0);
    let falloff = 1.0 - t * t * (3.0 - 2.0 * t);
    let gain = (volume * falloff).min(1.5);
    let pan = (d.x / 14.0).clamp(-1.0, 1.0);
    (gain * (1.0 - pan.max(0.0) * 0.7), gain * (1.0 + pan.min(0.0) * 0.7))
}

fn decode(bytes: &'static [u8]) -> Option<Clip> {
    let dec = Decoder::new(Cursor::new(bytes)).ok()?;
    let (channels, rate) = (dec.channels(), dec.sample_rate());
    let data: Vec<f32> = dec.collect();
    Some(Clip { channels, rate, data: Box::leak(data.into_boxed_slice()) })
}

impl Voices {
    fn load() -> Self {
        let all = |list: &[&'static [u8]]| list.iter().filter_map(|b| decode(b)).collect::<Vec<_>>();
        Self {
            announce: voice::ANNOUNCE.iter().map(|l| all(l)).collect(),
            quips: voice::QUIPS.iter().map(|per| per.iter().map(|l| all(l)).collect()).collect(),
            quips_any: voice::QUIPS_ANY.iter().map(|l| all(l)).collect(),
            pleas: voice::PLEAS.iter().map(|l| all(l)).collect(),
            chokes: voice::CHOKES.iter().map(|l| all(l)).collect(),
            choke_end: voice::CHOKE_END.iter().map(|l| all(l)).collect(),
            pains: voice::PAINS.iter().map(|l| all(l)).collect(),
            lines: Lines::default(),
            live: Default::default(),
        }
    }
}

impl Clip {
    fn seconds(&self) -> f32 {
        self.data.len() as f32 / self.channels.get() as f32 / self.rate.get() as f32
    }
}

/// A short piece cut out of a recording: (file, start ms, length ms, speed).
/// Speed below 1 plays it slower and lower.
type Cut = (&'static [u8], f32, f32, f32);

const PLASTIC: &[u8] = include_bytes!("../assets/sfx/plastic_container_open_small_ufx_1.ogg");
const BUTTON: &[u8] = include_bytes!("../assets/sfx/button_mx_2.ogg");
const CLICK: &[u8] = include_bytes!("../assets/sfx/ui_click_mx_5.ogg");

/// Sounds cut from inside longer recordings. Stick figures on flat shapes
/// don't walk on concrete: their steps are small plastic clicks (a plastic
/// tub's lid, a button), landings the same clicks played slow and low.
fn cuts(kind: Sfx) -> &'static [Cut] {
    match kind {
        Sfx::Footstep => &[(PLASTIC, 220.0, 40.0, 0.9), (PLASTIC, 442.0, 60.0, 0.85), (BUTTON, 8.0, 60.0, 0.9), (CLICK, 46.0, 34.0, 0.85)],
        Sfx::Land => &[(PLASTIC, 442.0, 70.0, 0.6), (BUTTON, 8.0, 70.0, 0.55), (PLASTIC, 220.0, 45.0, 0.55)],
        _ => &[],
    }
}

/// Decodes `cut`, keeps just that stretch (with a 1 ms fade in and a fade
/// out over its last third so it doesn't click), and slows it by `speed`.
fn decode_cut(&(bytes, start_ms, len_ms, speed): &Cut) -> Option<Clip> {
    let dec = Decoder::new(Cursor::new(bytes)).ok()?;
    let (channels, rate) = (dec.channels(), dec.sample_rate());
    let ch = channels.get() as usize;
    let frame_at = |ms: f32| (ms / 1000.0 * rate.get() as f32) as usize * ch;
    let all: Vec<f32> = dec.collect();
    let (a, b) = (frame_at(start_ms).min(all.len()), frame_at(start_ms + len_ms).min(all.len()));
    let mut data = all[a..b].to_vec();
    let frames = data.len() / ch;
    let fade_in = (rate.get() as usize / 1000).max(1);
    for (i, frame) in data.chunks_mut(ch).enumerate() {
        let t = i as f32 / frames.max(1) as f32;
        let out = if t > 0.66 { (1.0 - t) / 0.34 } else { 1.0 };
        let inn = (i as f32 / fade_in as f32).min(1.0);
        for v in frame {
            *v *= out * inn;
        }
    }
    let rate = NonZero::new((rate.get() as f32 * speed) as u32)?;
    Some(Clip { channels, rate, data: Box::leak(data.into_boxed_slice()) })
}

/// Everything's volume (M mutes).
pub const MASTER_VOLUME: f32 = 0.6;

/// Base loudness per kind, so footsteps sit under hits and gunshots.
fn loudness(kind: Sfx) -> f32 {
    match kind {
        Sfx::Footstep => 0.2,
        Sfx::Land => 0.45,
        Sfx::Swing | Sfx::Stab => 0.45,
        Sfx::SwingHeavy => 0.55,
        Sfx::Pickup | Sfx::Throw | Sfx::Empty => 0.5,
        Sfx::Shot | Sfx::PistolShot => 0.8,
        Sfx::Announce => 1.0,
        Sfx::Plea | Sfx::Quip | Sfx::Choke | Sfx::ChokeEnd | Sfx::Pain => 0.85,
        _ => 0.7,
    }
}

const ALL: [Sfx; 31] = [
    Sfx::Swing,
    Sfx::SwingHeavy,
    Sfx::Stab,
    Sfx::HitBlade,
    Sfx::HitBlunt,
    Sfx::HitHeavy,
    Sfx::Punch,
    Sfx::Impale,
    Sfx::Sever,
    Sfx::Clash,
    Sfx::Embed,
    Sfx::Unembed,
    Sfx::Pickup,
    Sfx::Throw,
    Sfx::Shot,
    Sfx::PistolShot,
    Sfx::Ricochet,
    Sfx::BulletHit,
    Sfx::Empty,
    Sfx::Footstep,
    Sfx::Land,
    Sfx::Thud,
    Sfx::Saw,
    Sfx::Spikes,
    Sfx::Splash,
    Sfx::Announce,
    Sfx::Plea,
    Sfx::Quip,
    Sfx::Choke,
    Sfx::ChokeEnd,
    Sfx::Pain,
];

#[derive(Clone, Copy)]
struct Clip {
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    data: &'static [f32],
}

pub struct Audio {
    /// The open output (kept alive for as long as the game plays sound).
    _sink: MixerDeviceSink,
    /// Every sound plays into this, and it goes out through a limiter: a
    /// pile-up of loud hits never goes over full scale (which some drivers
    /// answer by muting everything for a few seconds).
    bus: rodio::mixer::Mixer,
    clips: Vec<Vec<Clip>>,
    voices: Voices,
    /// Cheap random numbers for picking variants and pitch (not game state).
    seed: u32,
    /// How many of each kind started this frame (stops pile-ups).
    this_frame: [u8; ALL.len()],
    pub volume: f32,
}

impl Audio {
    /// Opens the default output device and decodes every sound. `None` if
    /// there's no usable device.
    pub fn new() -> Option<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink().map_err(|e| eprintln!("[audio] no output: {e}")).ok()?;
        sink.log_on_drop(false);
        let mut clips = Vec::new();
        for kind in ALL {
            let mut list = Vec::new();
            for bytes in files(kind) {
                match Decoder::new(Cursor::new(*bytes)) {
                    Ok(dec) => {
                        let (channels, rate) = (dec.channels(), dec.sample_rate());
                        let data: Vec<f32> = dec.collect();
                        list.push(Clip { channels, rate, data: Box::leak(data.into_boxed_slice()) });
                    }
                    Err(e) => eprintln!("[audio] couldn't decode a {kind:?} sound: {e}"),
                }
            }
            for cut in cuts(kind) {
                match decode_cut(cut) {
                    Some(clip) => list.push(clip),
                    None => eprintln!("[audio] couldn't cut a {kind:?} sound"),
                }
            }
            clips.push(list);
        }
        let (bus, mixed) = limited_bus(sink.config().channel_count(), sink.config().sample_rate());
        sink.mixer().add(mixed);
        Some(Self { _sink: sink, bus, clips, voices: Voices::load(), seed: 0x9E37_79B9, this_frame: [0; ALL.len()], volume: MASTER_VOLUME })
    }

    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    /// Plays one frame's events as heard from `listener` (the camera).
    /// Who is talking right now, per fighter: what kind of line, and how far
    /// open their mouth is (0..1, following the words).
    pub fn talking(&self) -> [Option<(Sfx, f32)>; 4] {
        std::array::from_fn(|i| {
            let (ctl, kind) = self.voices.live[i].as_ref()?;
            if ctl.finished.load(Ordering::Relaxed) {
                return None;
            }
            let level = f32::from_bits(ctl.level.load(Ordering::Relaxed));
            Some((*kind, ((level - 0.04) / 0.35).clamp(0.0, 1.0)))
        })
    }

    /// Plays one frame's events and steers the voice lines already playing:
    /// `speakers` is each fighter's mouth position and whether they're alive
    /// (a line stops the moment its speaker dies).
    pub fn play_frame(&mut self, events: &[SfxEvent], listener: Vec2, speakers: &[Speaker]) {
        self.this_frame = [0; ALL.len()];
        for (i, slot) in self.voices.live.iter_mut().enumerate() {
            let Some((ctl, kind)) = slot else { continue };
            if ctl.finished.load(Ordering::Relaxed) {
                *slot = None;
                continue;
            }
            match speakers.get(i) {
                // (the last gasp goes on after they're gone; a choke stops
                // the moment they're cut down)
                Some(s) if (s.alive && (s.choking || *kind != Sfx::Choke)) || *kind == Sfx::ChokeEnd => {
                    let (l, r) = voice_gains(s.mouth, listener, self.volume * loudness(Sfx::Quip));
                    ctl.set(l, r, (s.mouth - listener).length());
                }
                // dead (or gone): cut off mid-word
                _ => ctl.stop.store(true, Ordering::Relaxed),
            }
        }
        // (voice lines come through `speak`, once their frame is final)
        for e in events.iter().filter(|e| !e.kind.is_voice()) {
            self.play(*e, listener);
        }
    }

    /// Says the logged voice lines from frames up to `through` (final for
    /// everyone) that haven't been heard yet, in the order they were said.
    pub fn speak(&mut self, log: &[(u32, SfxEvent)], through: u32, listener: Vec2) {
        if through < self.voices.lines.heard {
            // a new game: its frames start again from the beginning
            self.voices.lines = Lines::default();
        }
        let heard = self.voices.lines.heard;
        let mut due: Vec<(u32, SfxEvent)> = log.iter().copied().filter(|(f, _)| *f > heard && *f <= through).collect();
        due.sort_by_key(|(f, _)| *f);
        for (f, e) in due {
            self.say(f, e, listener);
        }
        self.voices.lines.heard = self.voices.lines.heard.max(through);
    }

    fn play(&mut self, e: SfxEvent, listener: Vec2) {
        let idx = ALL.iter().position(|k| *k == e.kind).unwrap_or(0);
        if self.this_frame[idx] >= 3 || self.clips[idx].is_empty() {
            return;
        }
        let d = e.pos - listener;
        let dist = d.length();
        if dist > 40.0 {
            return;
        }
        let falloff = 1.0 / (1.0 + (dist / 10.0) * (dist / 10.0));
        let gain = (e.volume * loudness(e.kind) * falloff * self.volume).min(1.0);
        if gain < 0.01 {
            return;
        }
        self.this_frame[idx] += 1;
        let pick = self.rand() as usize % self.clips[idx].len();
        // (guns always sound the same; everything else varies a touch)
        let steady = matches!(e.kind, Sfx::Shot | Sfx::PistolShot);
        let pitch = if steady { 1.0 } else { 0.92 + (self.rand() % 1000) as f32 / 1000.0 * 0.16 };
        let pan = (d.x / 14.0).clamp(-1.0, 1.0);
        let left = gain * (1.0 - pan.max(0.0) * 0.7);
        let right = gain * (1.0 + pan.min(0.0) * 0.7);
        let clip = &self.clips[idx][pick];
        let src = StaticSamplesBuffer::new(clip.channels, clip.rate, clip.data).speed(pitch);
        self.bus.add(ChannelVolume::new(src, vec![left, right]));
    }

    /// A voice line. The announcer is heard by everyone at full volume, dead
    /// centre; fighters are placed like other sounds, each with their own
    /// pitch, and never talk over themselves.
    fn say(&mut self, frame: u32, e: SfxEvent, listener: Vec2) {
        let v = &self.voices;
        let voices = v.pleas.len().max(1);
        let who = e.speaker as usize % voices;
        let mut pool: Vec<&Clip> = Vec::new();
        match e.kind {
            Sfx::Announce => pool.extend(v.announce.get(e.variant as usize).into_iter().flatten()),
            Sfx::Plea => pool.extend(v.pleas.get(who).into_iter().flatten()),
            Sfx::Choke => pool.extend(v.chokes.get(who).into_iter().flatten()),
            Sfx::ChokeEnd => pool.extend(v.choke_end.get(who).into_iter().flatten()),
            Sfx::Pain => pool.extend(v.pains.get(who).into_iter().flatten()),
            _ => {
                // the finisher's own lines twice as likely as the generic ones
                for c in v.quips.get(who).and_then(|q| q.get(e.variant as usize)).into_iter().flatten() {
                    pool.extend([c, c]);
                }
                pool.extend(v.quips_any.get(who).into_iter().flatten());
            }
        }
        // (a take is known by its samples, the same on every machine)
        let seconds: Vec<f32> = pool.iter().map(|c| c.seconds()).collect();
        let ids: Vec<usize> = pool.iter().map(|c| c.data.as_ptr() as usize).collect();
        let pool: Vec<Clip> = pool.into_iter().copied().collect();
        if e.kind == Sfx::Pain {
            // hurt: whatever they were saying stops (with or without a cry)
            if let Some((old, _)) = self.voices.live[e.speaker as usize % 4].take() {
                old.stop.store(true, Ordering::Relaxed);
            }
        }
        let Some((pick, pitch)) = self.voices.lines.decide(frame, e, voices, &seconds, &ids) else { return };
        let clip = pool[pick];
        if e.kind == Sfx::Announce {
            // the announcer isn't in the arena: everyone hears them dead
            // centre, over the arena's speakers
            let g = loudness(e.kind) * e.volume * self.volume;
            let ctl = Arc::new(VoiceCtl::default());
            ctl.set(g, g, 0.0);
            let src = StaticSamplesBuffer::new(clip.channels, clip.rate, clip.data);
            self.bus.add(Voice::new(src, ctl, Space::PublicAddress));
            return;
        }
        let slot = e.speaker as usize % 4;
        let ctl = Arc::new(VoiceCtl::default());
        let (l, r) = voice_gains(e.pos, listener, e.volume * loudness(e.kind) * self.volume);
        if l + r < 0.005 && (e.pos - listener).length() > VOICE_SILENT {
            // out of earshot: still said (the speaker's busy all the same,
            // for everyone), just not heard here
            return;
        }
        ctl.set(l, r, (e.pos - listener).length());
        if let Some((old, _)) = self.voices.live[slot].replace((ctl.clone(), e.kind)) {
            old.stop.store(true, Ordering::Relaxed);
        }
        let src = StaticSamplesBuffer::new(clip.channels, clip.rate, clip.data).speed(pitch);
        self.bus.add(Voice::new(src, ctl, Space::Arena));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_decodes() {
        for kind in ALL {
            if kind.is_voice() {
                continue; // checked in `every_voice_line_decodes`
            }
            assert!(!files(kind).is_empty() || !cuts(kind).is_empty(), "{kind:?} has no sound");
            for cut in cuts(kind) {
                let clip = decode_cut(cut).unwrap_or_else(|| panic!("{kind:?}: bad cut"));
                let peak = clip.data.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(clip.data.len() > 1000 && peak > 0.05, "{kind:?} cut is silent or empty");
            }
            for bytes in files(kind) {
                let dec = Decoder::new(Cursor::new(*bytes)).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
                let secs = dec.total_duration().map_or(0.0, |d| d.as_secs_f32());
                let n = dec.count();
                assert!(n > 1000, "{kind:?} sound is empty");
                assert!(secs < 6.0, "{kind:?} sound is too long ({secs} s)");
            }
        }
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;

    #[test]
    fn every_player_picks_the_same_lines() {
        // two players' games, hearing the same final frames: same takes,
        // same pitches, the same lines skipped while someone's still talking
        let seconds = [1.2, 0.8, 2.0, 1.5];
        let ids = [11, 22, 33, 44];
        let said: Vec<(u32, SfxEvent)> = (0..40u32)
            .map(|k| (100 + k * 23, Sfx::Quip.at(Vec2::ZERO).line((k % 3) as u8, (k % 4) as u8)))
            .chain([(130, Sfx::Announce.at(Vec2::ZERO)), (131, Sfx::Plea.at(Vec2::ZERO).line(0, 1))])
            .collect();
        let run = || {
            let mut lines = Lines::default();
            let mut sorted = said.clone();
            sorted.sort_by_key(|(f, _)| *f);
            sorted.iter().map(|&(f, e)| lines.decide(f, e, 2, &seconds, &ids)).collect::<Vec<_>>()
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        let played = a.iter().flatten().count();
        assert!(played > 10 && played < a.len(), "some said, some skipped for talking over themselves ({played} of {})", a.len());
        // and nobody talks over themselves: a speaker's next line waits for the last
        let mut lines = Lines::default();
        let e = Sfx::Quip.at(Vec2::ZERO).line(0, 2);
        let (pick, pitch) = lines.decide(500, e, 4, &seconds, &ids).unwrap();
        let ends = 500 + (seconds[pick] / pitch * 60.0).ceil() as u32;
        assert!(lines.decide(ends - 1, e, 4, &seconds, &ids).is_none());
        let (next, _) = lines.decide(ends, e, 4, &seconds, &ids).unwrap();
        assert_ne!(next, pick, "not the same take twice in a row");
    }

    /// Energy of the next `n` stereo frames, (left, right).
    fn energy<S: Source>(v: &mut Voice<S>, n: usize) -> (f32, f32) {
        let mut e = (0.0, 0.0);
        for _ in 0..n {
            let (l, r) = (v.next().unwrap_or(0.0), v.next().unwrap_or(0.0));
            e.0 += l * l;
            e.1 += r * r;
        }
        e
    }

    fn tone(n: usize, hz: f32) -> &'static [f32] {
        Box::leak((0..n).map(|i| (i as f32 * hz * core::f32::consts::TAU / 48_000.0).sin() * 0.5).collect::<Vec<_>>().into_boxed_slice())
    }

    #[test]
    fn voice_lines_follow_their_speaker_and_stop_when_told() {
        let rate = NonZero::new(48_000).unwrap();
        let ctl = Arc::new(VoiceCtl::default());
        ctl.set(1.0, 0.0, 2.0);
        let mut v = Voice::new(StaticSamplesBuffer::new(NonZero::new(1).unwrap(), rate, tone(96_000, 440.0)), ctl.clone(), Space::Arena);
        let (l, r) = energy(&mut v, 4800);
        assert!(l > 3.0 * r, "starts on the left: {l:.1} vs {r:.1}");
        // the speaker walks to the other side: the sound follows
        ctl.set(0.0, 1.0, 2.0);
        energy(&mut v, 4800);
        let (l, r) = energy(&mut v, 4800);
        assert!(r > 3.0 * l, "followed them right: {l:.1} vs {r:.1}");
        // further off: still there (quieter and roomier)
        ctl.set(0.5, 0.5, 14.0);
        energy(&mut v, 4800);
        let (l, r) = energy(&mut v, 4800);
        assert!(l > 0.1 && r > 0.1, "still audible: {l:.2} {r:.2}");
        // they die: gone within ~25 ms (no room tail), and it says so
        ctl.stop.store(true, Ordering::Relaxed);
        let mut n = 0;
        while v.next().is_some() {
            n += 1;
            assert!(n < 2 * 4800, "didn't stop");
        }
        assert!(n / 2 <= 1300, "took {} frames to stop", n / 2);
        assert!(ctl.finished.load(Ordering::Relaxed));
    }

    #[test]
    fn the_room_rings_out_after_the_last_word() {
        let rate = NonZero::new(48_000).unwrap();
        let ctl = Arc::new(VoiceCtl::default());
        ctl.set(1.0, 1.0, 3.0);
        let mut v = Voice::new(StaticSamplesBuffer::new(NonZero::new(1).unwrap(), rate, tone(4800, 300.0)), ctl.clone(), Space::Arena);
        energy(&mut v, 4800); // the line itself
        let (l, r) = energy(&mut v, 4800); // 0.1 s after it ended
        assert!(l + r > 1e-3, "no room after the last word ({:.5})", l + r);
        let mut n = 0;
        while v.next().is_some() {
            n += 1;
        }
        assert!(n / 2 < 48_000, "tail went on for {} s", n as f32 / 2.0 / 48_000.0);
        assert!(ctl.finished.load(Ordering::Relaxed));
    }

    #[test]
    fn the_speakers_keep_the_bus_from_the_start() {
        // as the device sees it: the bus is handed over before anything has
        // played, the device pulls silence for a while, then a sound starts
        // and has to come out (an empty rodio mixer would have ended there)
        let rate = NonZero::new(48_000).unwrap();
        let (device, mut speakers) = rodio::mixer::mixer(NonZero::new(2).unwrap(), rate);
        let (bus, out) = limited_bus(NonZero::new(2).unwrap(), rate);
        device.add(out);
        for _ in 0..9_600 {
            assert_eq!(speakers.next(), Some(0.0), "silence until something plays");
        }
        let shot = decode(files(Sfx::Shot)[0]).unwrap();
        bus.add(ChannelVolume::new(StaticSamplesBuffer::new(shot.channels, shot.rate, shot.data), vec![0.8, 0.8]));
        let energy: f32 = (0..48_000).map(|_| speakers.next().unwrap_or(0.0).powi(2)).sum();
        assert!(energy > 10.0, "the shot should be heard ({energy:.1})");
    }

    #[test]
    fn a_pile_up_of_loud_hits_stays_within_full_scale() {
        // eight of the loudest hits at once, through the same limiter and
        // safety stage the speakers get: nothing over full scale, nothing NaN
        let rate = NonZero::new(48_000).unwrap();
        let (bus, mut out) = limited_bus(NonZero::new(2).unwrap(), rate);
        let loud = decode(files(Sfx::BulletHit)[0]).unwrap();
        for k in 0..8 {
            bus.add(ChannelVolume::new(StaticSamplesBuffer::new(loud.channels, loud.rate, loud.data).speed(0.95 + k as f32 * 0.01), vec![1.0, 1.0]));
        }
        let (mut peak, mut energy) = (0.0f32, 0.0f32);
        for _ in 0..48_000 * 2 {
            let v = out.next().unwrap_or(0.0);
            assert!(v.is_finite());
            peak = peak.max(v.abs());
            energy += v * v;
        }
        assert!(peak <= 1.0, "over full scale: {peak}");
        assert!(energy > 100.0, "and still loud, not silenced ({energy:.0})");
    }

    #[test]
    fn pain_cuts_in_over_whatever_they_were_saying() {
        let seconds = [3.0, 3.0];
        let ids = [1, 2];
        let mut lines = Lines::default();
        let plea = Sfx::Plea.at(Vec2::ZERO).line(0, 1);
        assert!(lines.decide(100, plea, 4, &seconds, &ids).is_some());
        // still begging at frame 110: another plea waits, a cry of pain doesn't
        assert!(lines.decide(110, plea, 4, &seconds, &ids).is_none());
        let pain = Sfx::Pain.at(Vec2::ZERO).line(0, 1);
        assert!(lines.decide(110, pain, 4, &[0.5], &[9]).is_some(), "pain cuts in");
        // with no pain recorded, they're still cut off and free to talk again
        assert!(lines.decide(200, plea, 4, &seconds, &ids).is_some());
        assert!(lines.decide(205, pain, 4, &[], &[]).is_none());
        assert!(lines.decide(206, plea, 4, &seconds, &ids).is_some(), "free to talk again after the cry");
    }

    #[test]
    fn no_line_twice_in_a_row_unless_its_the_only_one() {
        // (the announcer never waits on themselves, so every call says a line;
        // takes listed twice are the more likely ones)
        let e = Sfx::Announce.at(Vec2::ZERO);
        let ids = [1, 1, 2, 2, 3];
        let seconds = [1.0; 5];
        let mut lines = Lines::default();
        let mut prev = None;
        for f in 0..200 {
            let (pick, _) = lines.decide(f, e, 4, &seconds, &ids).unwrap();
            assert_ne!(Some(ids[pick]), prev, "repeated take {} on frame {f}", ids[pick]);
            prev = Some(ids[pick]);
        }
        let mut only = Lines::default();
        for f in 0..3 {
            assert_eq!(only.decide(f, e, 4, &[1.0], &[7]).map(|(p, _)| p), Some(0), "the only take still plays");
        }
    }

    /// Dev tool: `VOICE_PREVIEW=<dir> cargo test render_voice_previews -- --ignored`
    /// writes WAVs of real lines as the game plays them (near, mid, far, and
    /// the announcer over the PA).
    #[test]
    #[ignore]
    fn render_voice_previews() {
        let dir = std::env::var("VOICE_PREVIEW").expect("set VOICE_PREVIEW");
        let v = Voices::load();
        let quip = v.quips[0].iter().flatten().next().copied().expect("a quip");
        let announce = v.announce.iter().flatten().next().copied().expect("an announcement");
        let write = |name: &str, clip: Clip, space: Space, dist: f32| {
            let (l, r) = if space == Space::Arena { voice_gains(Vec2::new(dist * 0.6, dist * 0.8), Vec2::ZERO, 1.0) } else { (1.0, 1.0) };
            let ctl = Arc::new(VoiceCtl::default());
            ctl.set(l, r, dist);
            let samples: Vec<f32> = Voice::new(StaticSamplesBuffer::new(clip.channels, clip.rate, clip.data), ctl, space).collect();
            let rate = clip.rate.get();
            let mut out = Vec::new();
            let data_len = (samples.len() * 2) as u32;
            out.extend(b"RIFF");
            out.extend((36 + data_len).to_le_bytes());
            out.extend(b"WAVEfmt ");
            out.extend(16u32.to_le_bytes());
            out.extend(1u16.to_le_bytes());
            out.extend(2u16.to_le_bytes());
            out.extend(rate.to_le_bytes());
            out.extend((rate * 4).to_le_bytes());
            out.extend(4u16.to_le_bytes());
            out.extend(16u16.to_le_bytes());
            out.extend(b"data");
            out.extend(data_len.to_le_bytes());
            for s in samples {
                out.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
            std::fs::write(format!("{dir}/{name}.wav"), out).unwrap();
        };
        write("quip_near_2m", quip, Space::Arena, 2.0);
        write("quip_mid_8m", quip, Space::Arena, 8.0);
        write("quip_far_13m", quip, Space::Arena, 13.0);
        write("announcer_pa", announce, Space::PublicAddress, 0.0);
    }

    #[test]
    fn voices_only_carry_so_far() {
        let here = Vec2::ZERO;
        let (l, r) = voice_gains(Vec2::new(3.0, 0.0), here, 1.0);
        assert!(l > 0.2 && r > 0.9, "close by: full volume, panned right ({l:.2}, {r:.2})");
        let (l, r) = voice_gains(Vec2::new(-10.0, 0.0), here, 1.0);
        assert!(l > r && l < 0.9 && l > 0.05, "mid range: quieter, panned left ({l:.2}, {r:.2})");
        let (l, r) = voice_gains(Vec2::new(17.0, 0.0), here, 1.0);
        assert_eq!((l, r), (0.0, 0.0), "across the map: silent");
    }

    #[test]
    fn every_voice_line_decodes() {
        // (any subset of lines may be recorded: everything listed must decode)
        let v = Voices::load();
        let listed = |l: &[&[u8]]| l.len();
        assert_eq!(v.announce.len(), crate::voice::ANNOUNCE.len(), "a slot per finisher");
        assert_eq!(crate::voice::ANNOUNCE.len(), crate::weapon::Finish::Pin as usize + 1, "voice.rs is out of date with the finishers");
        assert_eq!(v.announce.iter().map(|l| l.len()).sum::<usize>(), crate::voice::ANNOUNCE.iter().map(|l| listed(l)).sum::<usize>());
        assert_eq!(v.quips.len(), crate::voice::QUIPS.len());
        assert_eq!(v.chokes.iter().map(|l| l.len()).sum::<usize>(), crate::voice::CHOKES.iter().map(|l| listed(l)).sum::<usize>());
        assert_eq!(v.choke_end.iter().map(|l| l.len()).sum::<usize>(), crate::voice::CHOKE_END.iter().map(|l| listed(l)).sum::<usize>());
        assert_eq!(v.pleas.iter().map(|l| l.len()).sum::<usize>(), crate::voice::PLEAS.iter().map(|l| listed(l)).sum::<usize>());
        assert!(v.quips.iter().all(|p| p.len() == crate::voice::ANNOUNCE.len()));
        for c in v.announce.iter().flatten().chain(v.quips.iter().flatten().flatten()).chain(v.quips_any.iter().flatten()).chain(v.pleas.iter().flatten()).chain(v.chokes.iter().flatten()).chain(v.choke_end.iter().flatten()) {
            let s = c.seconds();
            assert!((0.2..4.0).contains(&s), "voice line {s:.2} s long");
        }
    }
}

#[cfg(test)]
mod analyse {
    use super::*;

    /// Dev tool: `AUDIO_FILES="a.ogg;b.ogg" cargo test analyse_files -- --ignored --nocapture`
    /// prints each file's length and its separate transients (start, length,
    /// peak, brightness as zero crossings per ms).
    #[test]
    #[ignore]
    fn analyse_files() {
        let list = std::env::var("AUDIO_FILES").unwrap_or_default();
        for path in list.split(';').filter(|p| !p.is_empty()) {
            let bytes = std::fs::read(path).unwrap();
            let dec = Decoder::new(Cursor::new(bytes)).unwrap();
            let (ch, rate) = (dec.channels().get() as usize, dec.sample_rate().get() as f32);
            let raw: Vec<f32> = dec.collect();
            let mono: Vec<f32> = raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
            let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let name = path.rsplit(['/', '\\']).next().unwrap();
            println!("{name}: {:.3} s, {}ch {} Hz, peak {peak:.2}", mono.len() as f32 / rate, ch, rate);
            // 2 ms windows; a transient = rising above 25% of the file's peak
            let win = (rate * 0.002) as usize;
            let rms: Vec<f32> = mono.chunks(win).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect();
            let rpeak = rms.iter().fold(0.0f32, |m, v| m.max(*v));
            let (on, off) = (rpeak * 0.25, rpeak * 0.06);
            let mut i = 0;
            while i < rms.len() {
                if rms[i] > on {
                    let start = (0..i).rev().take_while(|&j| rms[j] > off).last().unwrap_or(i);
                    let mut end = i;
                    while end < rms.len() && rms[end] > off {
                        end += 1;
                    }
                    let seg = &mono[start * win..(end * win).min(mono.len())];
                    let zc = seg.windows(2).filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0)).count() as f32;
                    let ms = seg.len() as f32 / rate * 1000.0;
                    let p = seg.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                    println!("  hit at {:6.1} ms  len {:5.1} ms  peak {p:.2}  brightness {:.1} zc/ms", start as f32 * 2.0, ms, zc / ms);
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
        }
    }
}
