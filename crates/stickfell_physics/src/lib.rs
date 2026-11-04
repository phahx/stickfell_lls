//! # stickfell_physics
//!
//! Deterministic 2D position-based physics (small-step XPBD) built for
//! active ragdolls: particles, distance constraints, hinge limits/motors,
//! world-angle "muscles", capsule-vs-capsule contacts, and static oriented
//! boxes. The whole [`World`] is `Clone`, so rollback netcode can snapshot it.
//!
//! Determinism rules for anything that runs inside the simulation:
//! use [`dmath`] instead of `f32::sin/cos/atan2/powf`, iterate in index order,
//! and never use hash maps or wall-clock time.

pub mod collide;
pub mod dmath;
mod world;

pub use collide::{Obb, RayHit};
pub use glam::Vec2;
pub use world::*;
