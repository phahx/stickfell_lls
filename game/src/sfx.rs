//! Sound events. The simulation records what happened each frame as a list of
//! `SfxEvent`s (where, what, how loud); the audio player outside the
//! simulation turns them into sounds. Events never feed back into the game
//! state, so they can't affect determinism.

use stickfell_physics::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Swing,
    SwingHeavy,
    Stab,
    HitBlade,
    HitBlunt,
    HitHeavy,
    Punch,
    Impale,
    Sever,
    Clash,
    Embed,
    Unembed,
    Pickup,
    Throw,
    /// The revolver going off (always the same shot).
    Shot,
    /// The pistol going off.
    PistolShot,
    Ricochet,
    BulletHit,
    Empty,
    Footstep,
    Land,
    Thud,
    Saw,
    Spikes,
    Splash,
    /// Voice lines around a finisher: the announcer naming it (`variant` =
    /// the `Finish`), the victim begging, the finisher's one-liner after
    /// (`variant` = the `Finish`). `speaker` is who says it.
    Announce,
    Plea,
    Quip,
    /// Strung up and choking (looped by the player while `choking`), and
    /// the last gasp as they die.
    Choke,
    ChokeEnd,
    /// Hurt mid-finisher: cuts off whatever they were saying.
    Pain,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfxEvent {
    pub kind: Sfx,
    pub pos: Vec2,
    pub volume: f32,
    pub variant: u8,
    pub speaker: u8,
}

impl Sfx {
    /// Someone talking (or the announcer): heard by every player alike, once
    /// the frame it was said on is final.
    pub fn is_voice(self) -> bool {
        matches!(self, Sfx::Announce | Sfx::Plea | Sfx::Quip | Sfx::Choke | Sfx::ChokeEnd | Sfx::Pain)
    }

    pub fn at(self, pos: Vec2) -> SfxEvent {
        SfxEvent { kind: self, pos, volume: 1.0, variant: 0, speaker: 0 }
    }
}

impl SfxEvent {
    pub fn vol(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Which line (voice lines) and who says it.
    pub fn line(mut self, variant: u8, speaker: u8) -> Self {
        self.variant = variant;
        self.speaker = speaker;
        self
    }
}
