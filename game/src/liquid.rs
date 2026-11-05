//! Liquids: how each one looks (the renderer's liquid shader does the
//! drawing, see [`stickfell_render::LiquidLook`]) and how it treats what
//! falls in. Water floats a body and can drown it; acid and lava take it
//! down and eat it. Every number is per liquid, so a new one (oil, tar,
//! poison) is just another preset.

use stickfell_physics::FluidKind;
use stickfell_render::{Color, LiquidLook};

/// Which drops are which (the order of [`fluid_kinds`]).
pub const WATER_DROP: u8 = 0;
pub const ACID_DROP: u8 = 1;
pub const LAVA_DROP: u8 = 2;

/// The liquids, in drop-kind order.
pub const LIQUIDS: [&Liquid; 3] = [&WATER, &ACID, &LAVA];

/// How each kind of drop flows, for the fluid solver.
pub fn fluid_kinds() -> Vec<FluidKind> {
    LIQUIDS.iter().map(|l| l.flow_kind).collect()
}

/// The kind of drop a hazard is made of, if it's a liquid.
pub fn drop_kind(k: crate::level::HazardKind) -> Option<u8> {
    use crate::level::HazardKind;
    match k {
        HazardKind::Water => Some(WATER_DROP),
        HazardKind::Acid => Some(ACID_DROP),
        HazardKind::Lava => Some(LAVA_DROP),
        HazardKind::Spikes | HazardKind::Saw => None,
    }
}

/// How much one drop of blood (a blood spark) dyes the liquid it falls in.
pub const BLOOD_DYE: f32 = 0.35;

/// Water meeting lava: it boils off, and the lava drop it touched cools by
/// this much (cooled through, it's rock: it doesn't burn or boil any more).
pub const LAVA_COOLING: f32 = 1.0 / LAVA_BOILS;
/// How many drops of water the heat in a drop of lava boils off: basaltic
/// lava at 1150 C (2700 kg/m3, 1.2 kJ/kg K, 400 kJ/kg to set solid) cooled
/// to 100 C, against water from 20 C (4.186 kJ/kg K, 2257 kJ/kg to boil):
/// 1.729.
pub const LAVA_BOILS: f32 = (2700.0 * (1.2 * 1050.0 + 400.0)) / (1000.0 * (4.186 * 80.0 + 2257.0));
/// Lava this cooled (its dye) is rock: no glow, no heat.
pub const COOLED_LAVA: f32 = 0.75;

/// How far apart drops sit at rest (m).
pub const DROP_SPACING: f32 = 0.12;
/// How big a blob each drop is drawn as.
pub const DROP_BLOB: f32 = 0.19;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Liquid {
    // ---- look ----
    /// Colour and opacity just under the surface, and at `depth` (m) down.
    pub shallow: (u32, f32),
    pub deep: (u32, f32),
    pub depth: f32,
    /// The foam along the surface (colour, opacity) and how deep it reaches.
    pub foam: (u32, f32),
    pub foam_width: f32,
    /// How much it glows, and how much more its bright bits do.
    pub glow: f32,
    pub sparkle: f32,
    /// Caustics, rising bubbles and a cracked crust (0..1 each).
    pub caustics: f32,
    pub bubbles: f32,
    pub crust: f32,
    /// Fine ripples (m) on top of the waves, and how fast it all drifts.
    pub ripple: f32,
    pub flow: f32,
    pub seed: f32,
    /// Light it casts up out of itself (colour, strength), if any.
    pub light: Option<(u32, f32)>,
    /// What flies up when something splashes in, and whether what's in it
    /// steams.
    pub drop: u32,
    pub steams: bool,
    /// What it turns where it's dyed: (shallow, deep, glow then, how soon a
    /// little shows). Water and acid go red with blood; lava cools to rock.
    pub dyed: ((u32, f32), (u32, f32), f32, f32),

    // ---- the liquid itself (see `stickfell_physics::fluid`) ----
    /// How it flows: holding its volume, thickness, how heavy each drop is
    /// against the bodies it shoves (and that shove it), grip on the level.
    pub flow_kind: FluidKind,

    // ---- feel ----
    /// Push up on something all the way under, as a share of gravity (over
    /// 1 floats, under 1 sinks), and drag on it (1/s).
    pub buoyancy: f32,
    pub drag: f32,
    /// Damage a second to whoever's in it, and how fast it scorches them.
    pub damage: f32,
    pub scorch: f32,
    /// How well you can swim in it (0: not at all), and whether staying
    /// under drowns you.
    pub swim: f32,
    pub drowns: bool,
}

impl Liquid {
    pub fn look(&self) -> LiquidLook {
        let c = |(rgb, a): (u32, f32)| Color::hex(rgb).alpha(a);
        LiquidLook {
            shallow: c(self.shallow),
            deep: c(self.deep),
            depth: self.depth,
            foam: c(self.foam),
            foam_width: self.foam_width,
            glow: self.glow,
            sparkle: self.sparkle,
            caustics: self.caustics,
            bubbles: self.bubbles,
            crust: self.crust,
            ripple: self.ripple,
            flow: self.flow,
            seed: self.seed,
            dyed_shallow: c(self.dyed.0),
            dyed_deep: c(self.dyed.1),
            dyed_glow: self.dyed.2,
            dye_curve: self.dyed.3,
        }
    }
}

pub const WATER: Liquid = Liquid {
    shallow: (0x3f8fc4, 0.42),
    deep: (0x0e2c4c, 0.86),
    depth: 2.2,
    foam: (0xe4f4ff, 0.85),
    foam_width: 0.07,
    glow: 0.0,
    sparkle: 0.0,
    caustics: 0.7,
    bubbles: 0.04,
    crust: 0.0,
    ripple: 0.018,
    flow: 1.0,
    seed: 0.0,
    light: None,
    drop: 0xbfe2ff,
    steams: false,
    dyed: ((0x8f1c1c, 0.7), (0x3d0606, 0.95), 0.0, 8.0),
    flow_kind: FluidKind { pressure: 2000.0, near_pressure: 4000.0, viscosity: 0.3, viscosity_sq: 0.3, mass: 0.3, friction: 0.01, density: 1.0, dye_spread: 0.05 },
    buoyancy: 1.8,
    drag: 2.5,
    damage: 0.0,
    scorch: 0.0,
    swim: 1.0,
    drowns: true,
};

pub const ACID: Liquid = Liquid {
    shallow: (0x7ad83c, 0.6),
    deep: (0x23561a, 0.94),
    depth: 1.1,
    foam: (0xd6ff8a, 0.9),
    foam_width: 0.06,
    glow: 0.3,
    sparkle: 1.2,
    caustics: 0.0,
    bubbles: 0.14,
    crust: 0.0,
    ripple: 0.012,
    flow: 0.7,
    seed: 3.1,
    light: Some((0x60ff50, 1.1)),
    drop: 0xa8ff60,
    steams: true,
    dyed: ((0x7a5a18, 0.75), (0x3a2a0a, 0.95), 0.2, 8.0),
    flow_kind: FluidKind { pressure: 2000.0, near_pressure: 4000.0, viscosity: 2.5, viscosity_sq: 0.6, mass: 0.08, friction: 0.01, density: 1.2, dye_spread: 0.05 },
    buoyancy: 0.55,
    drag: 5.0,
    damage: 30.0,
    scorch: 0.35,
    swim: 0.45,
    drowns: false,
};

pub const LAVA: Liquid = Liquid {
    shallow: (0xff6a14, 0.97),
    deep: (0x8a1606, 1.0),
    depth: 0.9,
    foam: (0xffc050, 1.0),
    foam_width: 0.04,
    glow: 0.6,
    sparkle: 1.2,
    caustics: 0.0,
    bubbles: 0.06,
    crust: 0.9,
    ripple: 0.01,
    flow: 0.35,
    seed: 7.7,
    light: Some((0xff6a20, 1.2)),
    drop: 0xffa040,
    steams: true,
    dyed: ((0x4a4240, 1.0), (0x231f1e, 1.0), 0.0, 2.5),
    flow_kind: FluidKind { pressure: 2000.0, near_pressure: 4000.0, viscosity: 4.0, viscosity_sq: 0.5, mass: 0.1, friction: 0.04, density: 2.6, dye_spread: 0.0 },
    buoyancy: 0.6,
    drag: 7.0,
    damage: 60.0,
    scorch: 1.2,
    swim: 0.0,
    drowns: false,
};
