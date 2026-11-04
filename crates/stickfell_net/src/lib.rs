//! # stickfell_net
//!
//! Rollback netcode for deterministic games.
//!
//! * [`RollbackGame`] — what a game implements: `Clone` state, `step(inputs)`,
//!   and a checksum.
//! * [`P2PSession`] — peer-to-peer rollback: input delay, prediction,
//!   rollback + resimulation, time sync, disconnect and desync detection.
//! * [`SyncTestSession`] — local determinism checker: rolls back every frame
//!   and verifies that resimulating produces identical checksums.
//! * [`LocalSession`] — no networking; same driving API.
//! * Online play: [`RelayServer`] (room codes + packet forwarding, no port
//!   forwarding needed), [`LobbyClient`] and [`RelayTransport`].
//! * Transports: [`UdpTransport`] (direct) and the in-process [`SimNetwork`]
//!   (latency, jitter and packet loss) for tests.

mod p2p;
mod relay;
mod synctest;
mod transport;

pub use p2p::*;
pub use relay::*;
pub use synctest::*;
pub use transport::*;

/// Fixed-size input that can be sent over the wire.
pub trait NetInput: Copy + Default + PartialEq + core::fmt::Debug {
    const SIZE: usize;
    fn write(&self, out: &mut Vec<u8>);
    fn read(bytes: &[u8]) -> Self;
}

impl NetInput for u8 {
    const SIZE: usize = 1;
    fn write(&self, out: &mut Vec<u8>) {
        out.push(*self);
    }
    fn read(b: &[u8]) -> Self {
        b[0]
    }
}

impl NetInput for u16 {
    const SIZE: usize = 2;
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
    fn read(b: &[u8]) -> Self {
        u16::from_le_bytes([b[0], b[1]])
    }
}

impl NetInput for u32 {
    const SIZE: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
    fn read(b: &[u8]) -> Self {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
}

/// A deterministic game simulation that can be snapshotted by cloning.
pub trait RollbackGame: Clone {
    type Input: NetInput;
    /// Advances exactly one frame. Must be a pure function of (state, inputs).
    fn step(&mut self, inputs: &[Self::Input]);
    /// Hash of all simulation state, used to detect desyncs.
    fn checksum(&self) -> u64;
}

/// FNV-1a, handy for implementing [`RollbackGame::checksum`].
#[derive(Clone, Copy, Debug)]
pub struct Fnv64(pub u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv64 {
    #[inline]
    pub fn write_u8(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }
    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.write_u8(b);
        }
    }
    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }
    #[inline]
    pub fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }
    #[inline]
    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// Runs a game locally with no networking (same shape as the other sessions).
pub struct LocalSession<G: RollbackGame> {
    pub game: G,
    pub frame: i32,
}

impl<G: RollbackGame> LocalSession<G> {
    pub fn new(game: G) -> Self {
        Self { game, frame: 0 }
    }
    pub fn advance(&mut self, inputs: &[G::Input]) {
        self.game.step(inputs);
        self.frame += 1;
    }
}
