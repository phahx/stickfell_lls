//! Weapons are physics bodies made of particles: rigid ones are braced with
//! extra distance constraints, floppy ones (fish, flail chain) are not. A
//! held weapon is pinned to the hand and its heft is real — heavy heads swing
//! slowly and hit hard.

use core::f32::consts::PI;

use stickfell_physics::{Hinge, Particle, Vec2, World, dmath};

/// A severed limb lying around as a weapon: what it was on the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimbKind {
    Arm,
    Leg,
    /// Just the forearm (cut at the elbow).
    Forearm,
    /// Just the shin (cut at the knee).
    Shin,
}

/// The pin finisher's thrust lands on this finisher frame.
pub const PIN_HIT: u16 = 100;
/// The curb stomp's two stomps land on these finisher frames: the first
/// flattens the head, the second bursts it.
pub const STOMP_HITS: [u16; 2] = [52, 92];

/// Finishers: one per weapon (and one for bare hands), plus ones that need
/// the right surroundings (a sword with a wall behind the victim pins them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finish {
    Behead,
    /// Spear: stand them up, back off, and throw it through them.
    Javelin,
    /// Spear: stand them up, run them through, lift them high, slam them down.
    Lift,
    Smash,
    Spin,
    Slap,
    Bonk,
    Execute,
    Shank,
    /// Bare hands: limbs ripped off one by one.
    Dismember,
    /// Bare hands: head and spine torn out and held up.
    Spine,
    /// Grapple gun: the rope goes round their neck; fire the hook up at
    /// something and they hang (the kill comes later, if nobody cuts them down).
    Hang,
    /// Sword, with their back to a wall: shoved against it, run through the
    /// chest and pinned there by the blade, left hanging on it.
    Pin,
    /// Any weapon (or none), right after throwing them over your shoulder:
    /// they're left lying on their back; two stomps on the head, the second
    /// bursts it.
    Stomp,
}

impl Finish {
    pub fn title(self) -> &'static str {
        match self {
            Finish::Behead => "BEHEADED!",
            Finish::Javelin => "HARPOONED!",
            Finish::Lift => "SKEWERED!",
            Finish::Smash => "FLATTENED!",
            Finish::Spin => "LAUNCHED!",
            Finish::Slap => "SLAPPED SILLY!",
            Finish::Bonk => "BONK!",
            Finish::Execute => "EXECUTED!",
            Finish::Shank => "SHANKED!",
            Finish::Dismember => "TORN APART!",
            Finish::Spine => "SPINE RIPPED!",
            Finish::Hang => "STRUNG UP!",
            Finish::Pin => "PINNED!",
            Finish::Stomp => "CURB STOMPED!",
        }
    }

    /// Grabs the victim by the neck with the free hand at the start.
    pub fn holds(self) -> bool {
        !matches!(self, Finish::Smash | Finish::Stomp)
    }

    /// When the attacker lets go and the victim is left standing, swaying,
    /// to take the blow on their feet.
    pub fn stand_at(self) -> Option<u16> {
        match self {
            Finish::Javelin | Finish::Lift | Finish::Spin => Some(70),
            _ => None,
        }
    }

    /// Frames spent hauling the victim up by the neck before a held
    /// finisher's script starts (time to see what's happening).
    pub fn lead_in(self) -> u16 {
        match self {
            Finish::Behead | Finish::Slap | Finish::Bonk | Finish::Execute | Finish::Shank | Finish::Dismember | Finish::Spine | Finish::Hang | Finish::Pin => 24,
            _ => 0,
        }
    }

    /// The climactic moment (slow motion is timed to it).
    pub fn blow(self) -> u16 {
        match self {
            Finish::Javelin => 140,
            Finish::Lift => 110,
            _ => *self.hits().last().unwrap(),
        }
    }

    /// Finisher frames the strikes land on; the last one kills. (These run
    /// `FINISH_TEMPO` slower than the hand scripts are written in, so the
    /// wind-ups read.)
    pub fn hits(self) -> &'static [u16] {
        match self {
            Finish::Behead => &[74],
            // stand up, throw (the kill comes when it lands)
            Finish::Javelin => &[70, 140],
            // stand up, jam it in (the kill), slam them down
            Finish::Lift => &[70, 110, 160],
            Finish::Smash => &[53],
            // stand up, launch
            Finish::Spin => &[70, 150],
            Finish::Slap => &[46, 72, 100],
            Finish::Bonk => &[72],
            Finish::Execute => &[80],
            Finish::Shank => &[44, 60, 77, 94],
            Finish::Dismember => &[54, 72, 90, 108],
            Finish::Spine => &[80],
            // pushed to their knees, then the rope knotted round their neck
            Finish::Hang => &[90],
            // shoved back against the wall, the sword drawn back, then driven home
            Finish::Pin => &[PIN_HIT],
            Finish::Stomp => &STOMP_HITS,
        }
    }

    /// Total length including the follow-through.
    pub fn length(self) -> u16 {
        match self {
            Finish::Javelin => 230,
            Finish::Lift => 192,
            Finish::Spin => 190,
            _ => self.hits().last().copied().unwrap_or(30) + 32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Melee,
    Gun,
    /// Thrown one at a time with Q (a bundle keeps the rest in hand).
    Knives,
    /// Fires the hook of a rope tied round someone's neck.
    Grapple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Shafts and chains: barely hurt.
    Handle,
    /// Cuts; strong hits can sever limbs.
    Blade,
    /// Bonks; big knockback and knockdowns.
    Blunt,
    /// Stabs; high damage at the tip.
    Spike,
}

pub struct Pt {
    pub x: f32,
    pub y: f32,
    pub mass: f32,
    pub r: f32,
}

pub struct WSeg {
    pub a: u8,
    pub b: u8,
    pub r: f32,
    pub kind: Kind,
    pub color: u32,
}

pub struct WeaponDef {
    pub name: &'static str,
    pub points: &'static [Pt],
    pub segs: &'static [WSeg],
    /// Distance constraints (rigid pairs).
    pub links: &'static [(u8, u8)],
    /// Floppy joints (a, pivot, c) that prefer straight.
    pub flex: &'static [(u8, u8, u8)],
    pub flex_compliance: f32,
    /// Where the weapon hand holds it (the front hand on two-handers).
    pub grip: u8,
    /// Rear-hand grip for two-handed weapons; must sit behind `grip` within
    /// arm's reach, or the two hands fight each other.
    pub grip2: Option<u8>,
    /// Bone that points where the player aims.
    pub aim: (u8, u8),
    /// One rigid body (shape matched) instead of loose links.
    pub rigid: bool,
    /// Attacks are thrusts (spear) instead of swings.
    pub thrust: bool,
    /// Can run people through: (tip capsule index, blade base particle, tip
    /// particle). A fast stab landing with that capsule's tip impales.
    pub impale: Option<(u8, u8, u8)>,
    /// Guns: rounds in a fresh one; knives: how many in the bundle.
    pub ammo: u8,
    pub muzzle: u8,
    pub class: Class,
    /// Runs people through on a clean stab (needs `impale` tip geometry).
    pub skewer: bool,
    /// Launch speed when thrown.
    pub throw_speed: f32,
    /// The finisher this weapon does.
    pub finish: Finish,
    pub damage: f32,
    pub knock: f32,
}

const fn p(x: f32, y: f32, mass: f32, r: f32) -> Pt {
    Pt { x, y, mass, r }
}
const fn s(a: u8, b: u8, r: f32, kind: Kind, color: u32) -> WSeg {
    WSeg { a, b, r, kind, color }
}

const WOOD: u32 = 0x6b4a2e;
const STEEL: u32 = 0xd8dde6;
const IRON: u32 = 0x3c4048;

impl WeaponDef {
    /// Not symmetric about its length (a gun's grip hangs below the
    /// barrel), so it has a right way up.
    pub fn lopsided(&self) -> bool {
        self.points.iter().any(|p| p.y.abs() > 1e-4 && !self.points.iter().any(|q| (q.x - p.x).abs() < 1e-4 && (q.y + p.y).abs() < 1e-4))
    }

    /// The name as players read it ("Frying pan").
    pub fn label(&self) -> String {
        let lower = self.name.to_lowercase();
        let mut c = lower.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    }
}

pub static WEAPONS: [WeaponDef; 14] = [
    WeaponDef {
        name: "SWORD",
        // (fitted to its art: pommel, grip, guard and tip at 420 px per metre)
        points: &[p(-0.226, 0.0, 0.25, 0.05), p(0.0, 0.0, 0.3, 0.05), p(0.136, 0.0, 0.3, 0.05), p(1.138, 0.0, 0.3, 0.04)],
        segs: &[s(0, 2, 0.035, Kind::Handle, 0x4a3020), s(2, 3, 0.045, Kind::Blade, STEEL)],
        links: &[(0, 1), (1, 2), (2, 3), (0, 3), (0, 2), (1, 3)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 1,
        grip2: None,
        aim: (1, 3),
        rigid: true,
        thrust: false,
        impale: Some((1, 2, 3)),
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: true,
        throw_speed: 13.0,
        finish: Finish::Behead,
        damage: 2.4,
        knock: 0.8,
    },
    WeaponDef {
        name: "SLEDGEHAMMER",
        points: &[
            p(-0.1, 0.0, 0.3, 0.05),
            p(0.0, 0.0, 0.3, 0.05),
            p(0.45, 0.0, 0.3, 0.05),
            p(1.15, 0.0, 2.6, 0.08),
            p(1.15, 0.2, 0.6, 0.08),
            p(1.15, -0.2, 0.6, 0.08),
        ],
        segs: &[s(0, 3, 0.04, Kind::Handle, WOOD), s(4, 5, 0.13, Kind::Blunt, IRON)],
        links: &[(0, 1), (1, 2), (2, 3), (0, 3), (1, 3), (0, 2), (3, 4), (3, 5), (4, 5), (2, 4), (2, 5)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 2,
        grip2: Some(0),
        aim: (1, 3),
        rigid: true,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Smash,
        damage: 1.7,
        knock: 2.6,
    },
    WeaponDef {
        name: "SPEAR",
        points: &[
            p(-0.4, 0.0, 0.3, 0.05),
            p(0.0, 0.0, 0.3, 0.05),
            p(0.55, 0.0, 0.3, 0.05),
            p(1.6, 0.0, 0.2, 0.05),
            p(1.95, 0.0, 0.3, 0.04),
        ],
        segs: &[s(0, 3, 0.035, Kind::Handle, 0x7a5a36), s(3, 4, 0.045, Kind::Spike, 0xc8ccd4)],
        links: &[(0, 1), (1, 2), (2, 3), (3, 4), (0, 4), (0, 2), (1, 3), (2, 4), (1, 4), (0, 3)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 2,
        grip2: Some(1),
        aim: (1, 4),
        rigid: true,
        thrust: true,
        impale: Some((1, 2, 4)),
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: true,
        throw_speed: 13.0,
        finish: Finish::Javelin,
        damage: 2.8,
        knock: 1.0,
    },
    WeaponDef {
        name: "FLAIL",
        points: &[
            p(0.0, 0.0, 0.3, 0.05),
            p(0.45, 0.0, 0.3, 0.05),
            p(0.62, 0.0, 0.08, 0.03),
            p(0.79, 0.0, 0.08, 0.03),
            p(0.96, 0.0, 0.08, 0.03),
            p(1.16, 0.0, 2.2, 0.15),
        ],
        segs: &[
            s(0, 1, 0.04, Kind::Handle, WOOD),
            s(1, 2, 0.02, Kind::Handle, 0x55585e),
            s(2, 3, 0.02, Kind::Handle, 0x55585e),
            s(3, 4, 0.02, Kind::Handle, 0x55585e),
            s(4, 5, 0.02, Kind::Handle, 0x55585e),
            s(5, 5, 0.17, Kind::Blunt, 0x2f3238),
        ],
        links: &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (0, 1),
        rigid: false,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Spin,
        damage: 2.0,
        knock: 2.2,
    },
    WeaponDef {
        name: "FISH",
        points: &[
            p(0.0, 0.0, 0.25, 0.06),
            p(0.18, 0.0, 0.3, 0.08),
            p(0.36, 0.0, 0.35, 0.1),
            p(0.54, 0.0, 0.3, 0.1),
            p(0.72, 0.0, 0.3, 0.09),
        ],
        segs: &[
            s(0, 1, 0.05, Kind::Blunt, 0x7fa8c9),
            s(1, 2, 0.08, Kind::Blunt, 0x7fa8c9),
            s(2, 3, 0.1, Kind::Blunt, 0x8db6d4),
            s(3, 4, 0.09, Kind::Blunt, 0x7fa8c9),
        ],
        links: &[(0, 1), (1, 2), (2, 3), (3, 4)],
        flex: &[(0, 1, 2), (1, 2, 3), (2, 3, 4)],
        flex_compliance: 4e-3,
        grip: 0,
        grip2: None,
        aim: (0, 1),
        rigid: false,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Slap,
        damage: 0.7,
        knock: 2.6,
    },
    WeaponDef {
        name: "FRYING PAN",
        points: &[p(0.0, 0.0, 0.3, 0.04), p(0.38, 0.0, 0.3, 0.04), p(0.64, 0.0, 1.4, 0.05)],
        segs: &[s(0, 1, 0.03, Kind::Handle, 0x222222), s(2, 2, 0.24, Kind::Blunt, 0x2b2b2e)],
        links: &[(0, 1), (1, 2), (0, 2)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (0, 2),
        rigid: true,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Bonk,
        damage: 1.3,
        knock: 1.9,
    },
    WeaponDef {
        name: "REVOLVER",
        // (fitted to its art: grip, cylinder and muzzle, at the character's
        // 420 px per metre)
        points: &[p(0.0, 0.0, 0.3, 0.04), p(0.126, 0.086, 0.35, 0.05), p(0.493, 0.112, 0.15, 0.03)],
        segs: &[s(0, 1, 0.04, Kind::Handle, 0x4a3020), s(1, 2, 0.03, Kind::Blunt, IRON)],
        links: &[(0, 1), (1, 2), (0, 2)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (1, 2),
        rigid: true,
        thrust: false,
        impale: None,
        ammo: 6,
        muzzle: 2,
        class: Class::Gun,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Execute,
        damage: 0.9,
        knock: 1.2,
    },
    WeaponDef {
        name: "KNIVES",
        // (fitted to its art: grip, guard and point at 420 px per metre)
        points: &[p(0.0, 0.0, 0.12, 0.03), p(0.119, 0.0, 0.1, 0.03), p(0.486, 0.0, 0.08, 0.025)],
        segs: &[s(0, 1, 0.03, Kind::Handle, 0x2a2a2e), s(1, 2, 0.028, Kind::Blade, STEEL)],
        links: &[(0, 1), (1, 2), (0, 2)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (0, 2),
        rigid: true,
        thrust: true,
        impale: Some((1, 1, 2)),
        ammo: 3,
        muzzle: 2,
        class: Class::Knives,
        skewer: false,
        throw_speed: 24.0,
        finish: Finish::Shank,
        damage: 1.5,
        knock: 2.4,
    },
    WeaponDef {
        name: "KNIFE",
        // (fitted to its art: grip, guard and point at 420 px per metre)
        points: &[p(0.0, 0.0, 0.12, 0.03), p(0.119, 0.0, 0.1, 0.03), p(0.486, 0.0, 0.08, 0.025)],
        segs: &[s(0, 1, 0.03, Kind::Handle, 0x2a2a2e), s(1, 2, 0.028, Kind::Blade, STEEL)],
        links: &[(0, 1), (1, 2), (0, 2)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (0, 2),
        rigid: true,
        thrust: true,
        impale: Some((1, 1, 2)),
        ammo: 1,
        muzzle: 2,
        class: Class::Knives,
        skewer: false,
        throw_speed: 24.0,
        finish: Finish::Shank,
        damage: 1.5,
        knock: 2.4,
    },

    // ---- severed limbs, picked up and swung (held by the hand or foot) ----
    WeaponDef {
        name: "ARM",
        points: &[p(0.0, 0.0, 0.3, 0.075), p(0.29, 0.0, 0.35, 0.075), p(0.6, 0.0, 0.45, 0.075)],
        segs: &[s(0, 1, 0.075, Kind::Blunt, 0), s(1, 2, 0.075, Kind::Blunt, 0)],
        links: &[(0, 1), (1, 2)],
        flex: &[(0, 1, 2)],
        flex_compliance: 2e-3,
        grip: 0,
        grip2: None,
        aim: (0, 1),
        rigid: false,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Slap,
        damage: 1.0,
        knock: 2.8,
    },
    WeaponDef {
        name: "LEG",
        points: &[p(0.0, 0.0, 0.35, 0.08), p(0.44, 0.0, 0.5, 0.08), p(0.89, 0.0, 0.6, 0.08)],
        segs: &[s(0, 1, 0.075, Kind::Blunt, 0), s(1, 2, 0.075, Kind::Blunt, 0)],
        links: &[(0, 1), (1, 2)],
        flex: &[(0, 1, 2)],
        flex_compliance: 2e-3,
        grip: 0,
        grip2: None,
        aim: (0, 1),
        rigid: false,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 12.0,
        finish: Finish::Slap,
        damage: 1.3,
        knock: 3.2,
    },
    WeaponDef {
        name: "STUMP",
        points: &[p(0.0, 0.0, 0.3, 0.075), p(0.36, 0.0, 0.4, 0.075)],
        segs: &[s(0, 1, 0.075, Kind::Blunt, 0)],
        links: &[(0, 1)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (0, 1),
        rigid: false,
        thrust: false,
        impale: None,
        ammo: 0,
        muzzle: 0,
        class: Class::Melee,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Slap,
        damage: 0.6,
        knock: 2.4,
    },

    // ---- grapple gun (fires the hook of a rope tied round someone's neck) ----
    WeaponDef {
        name: "GRAPPLE GUN",
        // (fitted to its art: grip, breech and muzzle at 420 px per metre)
        points: &[p(0.0, 0.0, 0.3, 0.045), p(0.19, 0.167, 0.4, 0.055), p(0.571, 0.167, 0.25, 0.05)],
        segs: &[s(0, 1, 0.045, Kind::Handle, 0x3a3f2e), s(1, 2, 0.06, Kind::Blunt, 0x55623f)],
        links: &[(0, 1), (1, 2), (0, 2)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (1, 2),
        rigid: true,
        thrust: false,
        impale: None,
        // (one hook: once it's fired the gun is spent)
        ammo: 1,
        muzzle: 2,
        class: Class::Grapple,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Hang,
        damage: 1.0,
        knock: 1.6,
    },
    WeaponDef {
        name: "PISTOL",
        // (fitted to its art: grip, slide and muzzle, at the character's
        // 420 px per metre)
        points: &[p(0.0, 0.0, 0.3, 0.04), p(0.131, 0.167, 0.35, 0.05), p(0.457, 0.19, 0.15, 0.04)],
        segs: &[s(0, 1, 0.045, Kind::Handle, 0x6b4a32), s(1, 2, 0.045, Kind::Blunt, IRON)],
        links: &[(0, 1), (1, 2), (0, 2)],
        flex: &[],
        flex_compliance: 0.0,
        grip: 0,
        grip2: None,
        aim: (1, 2),
        rigid: true,
        thrust: false,
        impale: None,
        ammo: 7,
        muzzle: 2,
        class: Class::Gun,
        skewer: false,
        throw_speed: 13.0,
        finish: Finish::Execute,
        damage: 0.9,
        knock: 1.2,
    },
];

/// A single thrown knife (what a bundle throws, and what you pick back up).
pub const KNIFE: u8 = 8;
/// Severed limbs (spawned from bodies, never dropped): a whole arm, a whole
/// leg, a forearm or shin.
pub const ARM: u8 = 9;
pub const LEG: u8 = 10;
pub const STUMP: u8 = 11;
pub const GRAPPLE: u8 = 12;
/// A seven-shot semi-automatic.
pub const PISTOL: u8 = 13;
/// Weapons that can appear at spawn points (not loose single knives).
pub const DROPS: [u8; 9] = [0, 1, 2, 3, 4, 5, 6, 7, GRAPPLE];
/// The practice rack: every droppable weapon, plus ones still being tried
/// out before they join the drops.
pub const RACK: [u8; 10] = [0, 1, 2, 3, 4, 5, 6, 7, GRAPPLE, PISTOL];

/// Segment tags at or above this value belong to weapons.
pub const WEAPON_TAG: u32 = 0x10000;

pub fn decode_tag(tag: u32) -> Option<(usize, usize)> {
    (tag >= WEAPON_TAG).then(|| (((tag - WEAPON_TAG) / 16) as usize, ((tag - WEAPON_TAG) % 16) as usize))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Weapon {
    pub def: u8,
    pub base: u32,
    pub n: u8,
    pub seg_base: u32,
    pub nseg: u8,
    pub hinge_base: u32,
    pub aim: u32,
    /// Index into `World::rigids` (rigid weapons only).
    pub rigid: Option<u32>,
    /// Skewer constraint slot, and who is stuck on the blade (if anyone).
    pub skewer: u32,
    pub victim: Option<u8>,
    /// Frames the current victim has been on the blade.
    pub skewer_t: u16,
    /// Frames the blade keeps passing through its last victim after they
    /// come off (so it doesn't hit them again on the way out).
    pub pass_grace: u8,
    /// Stuck point-first in the level: fixed in place by two pins to kinematic
    /// anchors until someone wrenches it out.
    pub embedded: bool,
    /// Stuck in a wall through someone (the pin finisher): they don't slide on it.
    pub pinned: bool,
    /// Stuck in the level by a stab of the one holding it: the hand can lever
    /// on it (moving the mouse heaves you up it). Not for a stuck weapon
    /// someone grabs afterwards, which mustn't wrench them around.
    pub lever: bool,
    /// Mirrored from how it's defined: a lopsided weapon (a gun) is flipped
    /// over in the hand rather than ever held upside down. A symmetric one's
    /// drawing just follows which way its holder faces (its art can still
    /// have a top and bottom, like a curved blade).
    pub flip: bool,
    /// Stuck in someone (a thrown knife that went in point first): whose body
    /// and which part, and where each of its points sits in that part's frame.
    pub stuck: Option<(u8, u8)>,
    pub stuck_at: [Vec2; 6],
    /// A severed limb: which (for drawing it with the right textures), and
    /// how eaten away by acid, melted to the bone by lava (out of
    /// `fighter::ROT_FULL`) and on fire (frames left) it is.
    pub limb: Option<LimbKind>,
    pub eaten: u16,
    pub melted: u16,
    pub fire: u16,
    /// Kinematic anchor particles and the pin slots holding an embedded weapon.
    pub anchors: [u32; 2],
    pub pins: [u32; 2],
    /// Build-up of pulling on an embedded weapon; it comes out at `YANK_OUT`.
    pub yank: u8,
    /// Rounds left (guns), frames until it can fire again, and muzzle flash frames.
    pub ammo: u8,
    pub fire_cd: u8,
    pub flash: u8,
    /// Tip speed along the blade going into this frame (impact slows the tip,
    /// so impaling judges the stab by its speed before the hit).
    pub tip_speed: f32,
    pub holder: Option<u8>,
    pub last_holder: Option<u8>,
    /// Frames a thrown weapon stays dangerous.
    pub thrown: u8,
    /// Frames before a released weapon collides with its old owner again.
    pub grace: u8,
    pub hit_cd: [u8; crate::sim::MAX_PLAYERS],
    pub active: bool,
    /// Severed limbs: whose body it came off (player id + 1; 0 = not a limb).
    pub owner: u8,
}

impl Weapon {
    pub fn def(&self) -> &'static WeaponDef {
        &WEAPONS[self.def as usize]
    }

    pub fn is_gun(&self) -> bool {
        self.def().class == Class::Gun
    }

    pub fn is_grapple(&self) -> bool {
        self.def().class == Class::Grapple
    }

    pub fn is_knives(&self) -> bool {
        self.def().class == Class::Knives
    }

    pub fn particle(&self, i: u8) -> u32 {
        self.base + i as u32
    }

    pub fn mass(&self, w: &World) -> f32 {
        (0..self.n).map(|i| w.particles[self.particle(i) as usize].mass()).sum()
    }

    pub fn spawn(w: &mut World, def: u8, index: usize, pos: Vec2, angle: f32) -> Self {
        let d = &WEAPONS[def as usize];
        let (sn, cs) = dmath::sin_cos(angle);
        let base = w.particles.len() as u32;
        let rot = |x: f32, y: f32| Vec2::new(x * cs - y * sn, x * sn + y * cs);
        for (i, pt) in d.points.iter().enumerate() {
            // a point is at least as fat as the capsules it ends, so it
            // rests on the floor where it is drawn
            let r = d.segs.iter().filter(|sg| sg.a as usize == i || sg.b as usize == i).fold(pt.r, |r, sg| r.max(sg.r));
            let mut part = Particle::new(pos + rot(pt.x, pt.y), pt.mass, r);
            part.friction = 0.7;
            w.add_particle(part);
        }
        if d.rigid {
            // light filler points along long capsules so their whole length
            // collides with the level, not only the ends
            for sg in d.segs {
                let (a, b) = (&d.points[sg.a as usize], &d.points[sg.b as usize]);
                let len = ((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y)).sqrt();
                let k = (len / 0.16) as usize;
                for j in 1..k {
                    let t = j as f32 / k as f32;
                    let mut part = Particle::new(pos + rot(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t), 0.04, sg.r);
                    part.friction = 0.7;
                    w.add_particle(part);
                }
            }
        } else {
            for &(a, b) in d.links {
                w.add_distance(base + a as u32, base + b as u32, 0.0);
            }
        }
        let n = w.particles.len() as u32 - base;
        let rigid = d.rigid.then(|| w.add_rigid(base, n));
        let hinge_base = w.hinges.len() as u32;
        for &(a, b, c) in d.flex {
            w.add_hinge(Hinge {
                a: base + a as u32,
                b: base + b as u32,
                c: base + c as u32,
                limit: false,
                min: 0.0,
                max: 0.0,
                motor: true,
                target: PI,
                compliance: d.flex_compliance,
            });
        }
        let seg_base = w.segments.len() as u32;
        for (i, sg) in d.segs.iter().enumerate() {
            let s = w.add_segment(base + sg.a as u32, base + sg.b as u32, sg.r, 0, WEAPON_TAG + index as u32 * 16 + i as u32);
            w.segments[s as usize].collide_static = true;
        }
        let aim = w.add_aim(base + d.aim.0 as u32, base + d.aim.1 as u32, 1e-4);
        w.aims[aim as usize].enabled = false;
        let skewer = w.add_skewer_slot();
        let mut anchors = [0; 2];
        let mut pins = [0; 2];
        for k in 0..2 {
            let mut a = Particle::new(Vec2::new(0.0, -1000.0), 0.0, 0.01);
            a.collide_static = false;
            anchors[k] = w.add_particle(a);
            pins[k] = w.add_pin_slot(0.0);
        }
        Weapon {
            def,
            base,
            n: n as u8,
            seg_base,
            nseg: d.segs.len() as u8,
            hinge_base,
            aim,
            rigid,
            skewer,
            victim: None,
            stuck: None,
            stuck_at: [Vec2::ZERO; 6],
            skewer_t: 0,
            pass_grace: 0,
            tip_speed: 0.0,
            ammo: d.ammo,
            fire_cd: 0,
            flash: 0,
            embedded: false,
            pinned: false,
            lever: false,
            flip: false,
            limb: None,
            eaten: 0,
            melted: 0,
            fire: 0,
            anchors,
            pins,
            yank: 0,
            holder: None,
            last_holder: None,
            thrown: 0,
            grace: 0,
            hit_cd: [0; crate::sim::MAX_PLAYERS],
            active: true,
            owner: 0,
        }
    }

    /// Lets the blade pass through fighter group `group` (0 = collide with everyone).
    pub fn set_pass_through(&self, w: &mut World, group: u16) {
        for i in 0..self.nseg as u32 {
            w.segments[(self.seg_base + i) as usize].ignore_group = group;
        }
    }

    pub fn set_group(&self, w: &mut World, group: u16) {
        for i in 0..self.nseg as u32 {
            w.segments[(self.seg_base + i) as usize].group = group;
        }
    }

    /// Removes the weapon from play (fell out of the level).
    pub fn park(&mut self, w: &mut World) {
        self.active = false;
        self.holder = None;
        for i in 0..self.n {
            w.park_particle(self.particle(i));
        }
        for i in 0..self.nseg as u32 {
            w.segments[(self.seg_base + i) as usize].enabled = false;
        }
        w.aims[self.aim as usize].enabled = false;
        w.skewers[self.skewer as usize].enabled = false;
        self.victim = None;
        for k in 0..2 {
            w.distances[self.pins[k] as usize].enabled = false;
        }
        self.embedded = false;
        self.pinned = false;
        self.lever = false;
        if let Some(r) = self.rigid {
            w.rigids[r as usize].enabled = false;
        }
    }

    /// Moves the whole weapon so `grip` sits at `at` with its aim bone
    /// pointing along `angle`, moving with velocity `vel`. Used on pickup so
    /// the weapon doesn't get yanked across the room by the grip constraint.
    pub fn place(&self, w: &mut World, at: Vec2, angle: f32, vel: Vec2) {
        let d = self.def();
        let g = w.particles[self.particle(d.grip) as usize].pos;
        let bone = w.particles[self.particle(d.aim.1) as usize].pos - w.particles[self.particle(d.aim.0) as usize].pos;
        let (sn, cs) = dmath::sin_cos(dmath::angle_diff(angle, dmath::angle(bone)));
        for i in 0..self.n {
            let p = &mut w.particles[self.particle(i) as usize];
            let o = p.pos - g;
            p.pos = at + Vec2::new(o.x * cs - o.y * sn, o.x * sn + o.y * cs);
            p.prev = p.pos;
            p.vel = vel;
        }
    }

    /// Grip to tip, as the weapon is right now (0 without a tip).
    pub fn tip_reach(&self, w: &World) -> f32 {
        let d = self.def();
        let Some((_, _, tip)) = d.impale else { return 0.0 };
        (w.particles[self.particle(tip) as usize].pos - w.particles[self.particle(d.grip) as usize].pos).length()
    }

    /// Sticks the weapon in the level: drives it `depth` further along its
    /// blade and pins it there (it no longer collides with the level).
    pub fn embed(&mut self, w: &mut World, axis: Vec2, depth: f32) {
        let d = self.def();
        let Some((_, _, tip)) = d.impale else { return };
        for i in 0..self.n {
            let p = &mut w.particles[self.particle(i) as usize];
            p.pos += axis * depth;
            p.prev = p.pos;
            p.vel = Vec2::ZERO;
            p.collide_static = false;
        }
        for i in 0..self.nseg as u32 {
            w.segments[(self.seg_base + i) as usize].collide_static = false;
        }
        // pin the tip and a point ~0.35 m back up the blade: that fixes it
        let tip_pos = w.particles[self.particle(tip) as usize].pos;
        let back = (0..self.n)
            .filter(|&i| i != tip)
            .min_by(|&i, &j| {
                let di = ((tip_pos - w.particles[self.particle(i) as usize].pos).length() - 0.35).abs();
                let dj = ((tip_pos - w.particles[self.particle(j) as usize].pos).length() - 0.35).abs();
                di.total_cmp(&dj)
            })
            .unwrap_or(0);
        for (k, i) in [tip, back].into_iter().enumerate() {
            let pid = self.particle(i);
            let at = w.particles[pid as usize].pos;
            let a = &mut w.particles[self.anchors[k] as usize];
            a.pos = at;
            a.prev = at;
            w.distances[self.pins[k] as usize] = stickfell_physics::Distance { a: pid, b: self.anchors[k], rest: 0.0, compliance: 0.0, enabled: true };
        }
        self.embedded = true;
        self.yank = 0;
    }

    /// Pulls an embedded weapon free, popping it back along its blade.
    pub fn unembed(&mut self, w: &mut World, pop: Vec2) {
        for k in 0..2 {
            w.distances[self.pins[k] as usize].enabled = false;
        }
        if self.pinned {
            // driven in deep (through someone): slide it back out to the
            // wall's surface first, or the buried length would be shoved out
            // of the wall all at once (and whoever's holding it with it)
            if let Some((_, base, tip)) = self.def().impale {
                let (b, t) = (w.particles[self.particle(base) as usize].pos, w.particles[self.particle(tip) as usize].pos);
                let len = (t - b).length();
                let axis = (t - b) / len.max(1e-6);
                let inside = w.raycast(b, axis, len).map_or(0.0, |h| len - h.t);
                for i in 0..self.n {
                    let p = &mut w.particles[self.particle(i) as usize];
                    p.pos -= axis * (inside + 0.02);
                    p.prev = p.pos;
                }
            }
        }
        for i in 0..self.n {
            let p = &mut w.particles[self.particle(i) as usize];
            p.collide_static = true;
            p.vel += pop;
        }
        let d = self.def();
        for (i, sg) in d.segs.iter().enumerate() {
            let _ = sg;
            w.segments[(self.seg_base + i as u32) as usize].collide_static = true;
        }
        self.embedded = false;
        self.pinned = false;
        self.lever = false;
        self.yank = 0;
    }

    /// Is any part of the weapon inside the level geometry?
    pub fn touches_level(&self, w: &World) -> bool {
        (0..self.n).any(|i| {
            let p = &w.particles[self.particle(i) as usize];
            w.statics.iter().any(|o| o.collide_circle(p.pos, p.radius + 0.02).is_some())
        })
    }

    /// Moves the whole weapon by `d` (no velocity change).
    pub fn shift(&self, w: &mut World, d: Vec2) {
        for i in 0..self.n {
            let p = &mut w.particles[self.particle(i) as usize];
            p.pos += d;
            p.prev += d;
        }
    }

    /// Lowest distance from `p` to any of the weapon's particles.
    pub fn distance_to(&self, w: &World, p: Vec2) -> f32 {
        (0..self.n).map(|i| (w.particles[self.particle(i) as usize].pos - p).length()).fold(f32::MAX, f32::min)
    }
}
