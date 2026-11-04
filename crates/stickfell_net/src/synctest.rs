//! Local determinism checker.

use crate::RollbackGame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesyncError {
    pub frame: i32,
    pub expected: u64,
    pub got: u64,
}

impl core::fmt::Display for DesyncError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "desync at frame {}: expected checksum {:016x}, resimulation produced {:016x}",
            self.frame, self.expected, self.got
        )
    }
}

impl std::error::Error for DesyncError {}

/// Every frame, rolls back `check_distance` frames and resimulates them,
/// comparing checksums with the first run. Any hidden non-determinism (state
/// not captured by `Clone`, platform math, iteration order) shows up as an
/// error here long before it would desync a real match.
pub struct SyncTestSession<G: RollbackGame> {
    pub game: G,
    pub frame: i32,
    check_distance: i32,
    ring: usize,
    snapshots: Vec<Option<(i32, G)>>,
    checksums: Vec<(i32, u64)>,
    inputs: Vec<(i32, Vec<G::Input>)>,
}

impl<G: RollbackGame> SyncTestSession<G> {
    pub fn new(game: G, check_distance: i32) -> Self {
        let ring = check_distance.max(1) as usize + 2;
        Self {
            game,
            frame: 0,
            check_distance: check_distance.max(1),
            ring,
            snapshots: (0..ring).map(|_| None).collect(),
            checksums: vec![(-1, 0); ring],
            inputs: vec![(-1, Vec::new()); ring],
        }
    }

    pub fn advance(&mut self, inputs: &[G::Input]) -> Result<(), DesyncError> {
        let f = self.frame;
        let slot = f as usize % self.ring;
        self.snapshots[slot] = Some((f, self.game.clone()));
        self.checksums[slot] = (f, self.game.checksum());
        self.inputs[slot] = (f, inputs.to_vec());
        self.game.step(inputs);
        self.frame += 1;

        let start = self.frame - self.check_distance;
        if start < 0 {
            return Ok(());
        }
        let end_sum = self.game.checksum();
        let (tag, snap) = self.snapshots[start as usize % self.ring].as_ref().unwrap();
        debug_assert_eq!(*tag, start);
        let mut g = snap.clone();
        for rf in start..self.frame {
            let s = rf as usize % self.ring;
            let (ctag, expected) = self.checksums[s];
            debug_assert_eq!(ctag, rf);
            let got = g.checksum();
            if got != expected {
                return Err(DesyncError { frame: rf, expected, got });
            }
            g.step(&self.inputs[s].1);
        }
        let got = g.checksum();
        if got != end_sum {
            return Err(DesyncError { frame: self.frame, expected: end_sum, got });
        }
        // Continue from the resimulated state, like a real rollback would.
        self.game = g;
        Ok(())
    }
}
