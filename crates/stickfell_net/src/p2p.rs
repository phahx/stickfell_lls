//! Peer-to-peer rollback session.
//!
//! Every peer runs the full simulation. Each frame the local input is
//! scheduled `input_delay` frames ahead and sent (redundantly, until acked)
//! to every peer. Missing remote inputs are predicted by repeating the last
//! confirmed input. When a real input arrives that differs from what was
//! predicted, the session restores the snapshot from that frame and
//! resimulates up to the present, all within one tick.

use crate::{NetInput, RollbackGame, Transport};

/// How many frames we may run ahead of the latest confirmed remote input.
pub const MAX_PREDICTION: i32 = 8;
const INPUT_RING: usize = 128;
const SNAP_RING: usize = MAX_PREDICTION as usize + 2;
const CHECKSUM_INTERVAL: i32 = 30;
const MAX_INPUTS_PER_PACKET: i32 = 64;
const MAGIC: u32 = 0x314E_5753; // "SWN1"

const PKT_HELLO: u8 = 0;
const PKT_INPUT: u8 = 1;
const PKT_CHECKSUM: u8 = 2;

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub num_players: usize,
    pub local_player: usize,
    /// Frames of input delay. 1–3 hides most rollbacks on decent connections.
    pub input_delay: i32,
    /// Ticks of silence before a peer counts as disconnected.
    pub disconnect_ticks: u32,
}

impl SessionConfig {
    pub fn new(num_players: usize, local_player: usize) -> Self {
        Self { num_players, local_player, input_delay: 2, disconnect_ticks: 300 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Waiting to hear from every peer.
    Syncing,
    Running,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Connected { player: usize },
    Disconnected { player: usize },
    Desync { frame: i32, local: u64, remote: u64, player: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvanceResult {
    Advanced,
    /// Too far ahead of the remote peers' confirmed input; try again next tick.
    Stalled,
    /// Still syncing.
    NotRunning,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NetStats {
    pub rollbacks: u64,
    pub resimulated_frames: u64,
    pub stalls: u64,
    pub skipped_ticks: u64,
    pub last_rollback_len: i32,
}

#[derive(Clone)]
struct Ring<T: Copy> {
    slots: Vec<(i32, T)>,
}

impl<T: Copy + Default> Ring<T> {
    fn new() -> Self {
        Self { slots: vec![(-1, T::default()); INPUT_RING] }
    }
    fn get(&self, f: i32) -> Option<T> {
        if f < 0 {
            return None;
        }
        let (tag, v) = self.slots[f as usize % INPUT_RING];
        (tag == f).then_some(v)
    }
    fn set(&mut self, f: i32, v: T) {
        self.slots[f as usize % INPUT_RING] = (f, v);
    }
}

#[derive(Clone, Debug)]
struct Peer {
    synced: bool,
    disconnected: bool,
    last_recv_tick: u32,
    /// Highest frame of *our* local input this peer has confirmed.
    ack: i32,
    remote_frame: i32,
    local_adv: f32,
    remote_adv: f32,
}

pub struct P2PSession<G: RollbackGame, T: Transport> {
    game: G,
    transport: T,
    cfg: SessionConfig,
    state: SessionState,
    frame: i32,
    tick: u32,
    confirmed: Vec<Ring<G::Input>>,
    last_confirmed: Vec<i32>,
    last_input: Vec<G::Input>,
    used: Vec<Ring<G::Input>>,
    snapshots: Vec<Option<(i32, G)>>,
    checksums: Ring<u64>,
    local_final: Vec<(i32, u64)>,
    remote_checksums: Vec<(usize, i32, u64)>,
    last_checksum_frame: i32,
    first_incorrect: i32,
    peers: Vec<Peer>,
    events: Vec<SessionEvent>,
    wait_cooldown: u32,
    pub stats: NetStats,
    scratch: Vec<u8>,
}

impl<G: RollbackGame, T: Transport> P2PSession<G, T> {
    pub fn new(game: G, transport: T, cfg: SessionConfig) -> Self {
        assert!(cfg.local_player < cfg.num_players);
        let n = cfg.num_players;
        let mut s = Self {
            game,
            transport,
            state: SessionState::Syncing,
            frame: 0,
            tick: 0,
            confirmed: (0..n).map(|_| Ring::new()).collect(),
            last_confirmed: vec![-1; n],
            last_input: vec![G::Input::default(); n],
            used: (0..n).map(|_| Ring::new()).collect(),
            snapshots: (0..SNAP_RING).map(|_| None).collect(),
            checksums: Ring::new(),
            local_final: Vec::new(),
            remote_checksums: Vec::new(),
            last_checksum_frame: 0,
            first_incorrect: i32::MAX,
            peers: (0..n)
                .map(|_| Peer {
                    synced: false,
                    disconnected: false,
                    last_recv_tick: 0,
                    ack: -1,
                    remote_frame: 0,
                    local_adv: 0.0,
                    remote_adv: 0.0,
                })
                .collect(),
            events: Vec::new(),
            wait_cooldown: 0,
            stats: NetStats::default(),
            scratch: Vec::with_capacity(512),
            cfg,
        };
        // The first `input_delay` frames of local input are implicitly empty.
        let local = s.cfg.local_player;
        for f in 0..s.cfg.input_delay {
            s.confirmed[local].set(f, G::Input::default());
            s.last_confirmed[local] = f;
        }
        s.peers[local].synced = true;
        if n == 1 {
            s.state = SessionState::Running;
        }
        s
    }

    pub fn game(&self) -> &G {
        &self.game
    }
    pub fn frame(&self) -> i32 {
        self.frame
    }
    pub fn state(&self) -> SessionState {
        self.state
    }
    pub fn local_player(&self) -> usize {
        self.cfg.local_player
    }
    pub fn transport(&self) -> &T {
        &self.transport
    }
    pub fn drain_events(&mut self) -> Vec<SessionEvent> {
        std::mem::take(&mut self.events)
    }
    /// Whether `player`'s connection is gone (anyone who isn't a player in
    /// this session: no).
    pub fn is_disconnected(&self, player: usize) -> bool {
        self.peers.get(player).is_some_and(|p| p.disconnected)
    }

    fn remotes(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.cfg.num_players).filter(move |&p| p != self.cfg.local_player)
    }

    /// Frames the local peer is estimated to be ahead of the slowest peer.
    pub fn frames_ahead(&self) -> f32 {
        self.remotes()
            .filter(|&p| !self.peers[p].disconnected)
            .map(|p| (self.peers[p].local_adv - self.peers[p].remote_adv) * 0.5)
            .fold(0.0, f32::max)
    }

    /// Time sync: returns true when the caller should skip advancing this
    /// tick so the slower peer can catch up.
    pub fn should_wait(&mut self) -> bool {
        if self.state != SessionState::Running {
            return false;
        }
        if self.wait_cooldown > 0 {
            self.wait_cooldown -= 1;
            return false;
        }
        if self.frames_ahead() >= 1.0 {
            self.wait_cooldown = 15;
            self.stats.skipped_ticks += 1;
            return true;
        }
        false
    }

    /// Network I/O. Call exactly once per tick, before `advance`.
    pub fn poll(&mut self) {
        self.tick += 1;
        while let Some(pkt) = self.transport.recv() {
            self.handle_packet(&pkt);
        }
        match self.state {
            SessionState::Syncing => {
                if self.tick % 10 == 1 {
                    for p in self.remotes().collect::<Vec<_>>() {
                        self.send_hello(p, false);
                    }
                }
                if self.remotes().all(|p| self.peers[p].synced) {
                    self.state = SessionState::Running;
                }
            }
            SessionState::Running => {
                for p in self.remotes().collect::<Vec<_>>() {
                    let peer = &mut self.peers[p];
                    if !peer.disconnected
                        && self.tick.saturating_sub(peer.last_recv_tick) > self.cfg.disconnect_ticks
                    {
                        peer.disconnected = true;
                        self.events.push(SessionEvent::Disconnected { player: p });
                    }
                    let adv = (self.frame - peer.remote_frame) as f32;
                    peer.local_adv = peer.local_adv * 0.9 + adv * 0.1;
                }
                // Correct mispredictions right away, even if this tick ends
                // up waiting for time sync, so state and checksums are current.
                self.apply_pending_rollback();
                self.send_inputs();
            }
        }
    }

    fn apply_pending_rollback(&mut self) {
        if self.first_incorrect < self.frame {
            self.rollback(self.first_incorrect);
        }
        self.first_incorrect = i32::MAX;
    }

    /// Simulates one frame with the given local input (rolling back first if
    /// a misprediction was discovered).
    pub fn advance(&mut self, local_input: G::Input) -> AdvanceResult {
        if self.state != SessionState::Running {
            return AdvanceResult::NotRunning;
        }
        self.apply_pending_rollback();

        let min_remote = self
            .remotes()
            .filter(|&p| !self.peers[p].disconnected)
            .map(|p| self.last_confirmed[p])
            .min()
            .unwrap_or(i32::MAX);
        if min_remote != i32::MAX && self.frame - min_remote > MAX_PREDICTION {
            self.stats.stalls += 1;
            return AdvanceResult::Stalled;
        }

        let local = self.cfg.local_player;
        let lf = self.frame + self.cfg.input_delay;
        if self.last_confirmed[local] < lf {
            self.confirmed[local].set(lf, local_input);
            self.last_confirmed[local] = lf;
            self.last_input[local] = local_input;
        }

        self.sim_frame();
        self.exchange_checksums();
        self.send_inputs();
        AdvanceResult::Advanced
    }

    fn input_for(&self, p: usize, f: i32) -> G::Input {
        self.confirmed[p].get(f).unwrap_or(self.last_input[p])
    }

    fn sim_frame(&mut self) {
        let f = self.frame;
        self.checksums.set(f, self.game.checksum());
        let slot = &mut self.snapshots[f as usize % SNAP_RING];
        match slot {
            Some((tag, g)) => {
                *tag = f;
                g.clone_from(&self.game);
            }
            None => *slot = Some((f, self.game.clone())),
        }
        let inputs: Vec<G::Input> = (0..self.cfg.num_players).map(|p| self.input_for(p, f)).collect();
        for (p, i) in inputs.iter().enumerate() {
            self.used[p].set(f, *i);
        }
        self.game.step(&inputs);
        self.frame += 1;
    }

    fn rollback(&mut self, to: i32) {
        let target = self.frame;
        let (tag, snap) = self.snapshots[to as usize % SNAP_RING]
            .as_ref()
            .expect("rollback snapshot missing");
        assert_eq!(*tag, to, "rollback target {to} outside snapshot window");
        self.game.clone_from(snap);
        self.frame = to;
        self.stats.rollbacks += 1;
        self.stats.last_rollback_len = target - to;
        self.stats.resimulated_frames += (target - to) as u64;
        while self.frame < target {
            self.sim_frame();
        }
    }

    // ---- checksums ------------------------------------------------------

    fn exchange_checksums(&mut self) {
        // State at the start of frame f is final once every input < f is confirmed.
        let conf = (0..self.cfg.num_players)
            .filter(|&p| !self.peers[p].disconnected)
            .map(|p| self.last_confirmed[p])
            .min()
            .unwrap_or(-1);
        let final_upto = (conf + 1).min(self.frame - 1);
        let mut f = self.last_checksum_frame + CHECKSUM_INTERVAL;
        while f <= final_upto {
            if let Some(sum) = self.checksums.get(f) {
                self.local_final.push((f, sum));
                self.scratch.clear();
                self.scratch.push(PKT_CHECKSUM);
                self.scratch.push(self.cfg.local_player as u8);
                self.scratch.extend_from_slice(&f.to_le_bytes());
                self.scratch.extend_from_slice(&sum.to_le_bytes());
                for p in self.remotes().collect::<Vec<_>>() {
                    // Sent twice for loss tolerance; duplicates are harmless.
                    self.transport.send_to(p, &self.scratch);
                    self.transport.send_to(p, &self.scratch);
                }
            }
            self.last_checksum_frame = f;
            f += CHECKSUM_INTERVAL;
        }
        // Compare.
        let local_final = &self.local_final;
        let events = &mut self.events;
        self.remote_checksums.retain(|&(player, rf, rsum)| {
            if let Some(&(_, lsum)) = local_final.iter().find(|(lf, _)| *lf == rf) {
                if lsum != rsum {
                    events.push(SessionEvent::Desync { frame: rf, local: lsum, remote: rsum, player });
                }
                false
            } else {
                true
            }
        });
        if self.local_final.len() > 32 {
            let cut = self.local_final.len() - 32;
            self.local_final.drain(..cut);
        }
        if self.remote_checksums.len() > 64 {
            let cut = self.remote_checksums.len() - 64;
            self.remote_checksums.drain(..cut);
        }
    }

    /// Checksum of the state at the start of `frame`, if still recorded.
    pub fn checksum_at(&self, frame: i32) -> Option<u64> {
        self.checksums.get(frame)
    }

    /// Highest frame whose inputs every player still in the game has
    /// confirmed: everything up to it is final, and the same for everyone.
    pub fn final_frame(&self) -> i32 {
        (0..self.cfg.num_players)
            .filter(|&p| !self.peers[p].disconnected)
            .map(|p| self.last_confirmed[p])
            .min()
            .unwrap_or(-1)
    }

    /// Highest frame for which all players' inputs are confirmed.
    pub fn confirmed_frame(&self) -> i32 {
        (0..self.cfg.num_players).map(|p| self.last_confirmed[p]).min().unwrap_or(-1)
    }

    // ---- packets --------------------------------------------------------

    fn send_hello(&mut self, to: usize, reply: bool) {
        self.scratch.clear();
        self.scratch.push(PKT_HELLO);
        self.scratch.push(self.cfg.local_player as u8);
        self.scratch.extend_from_slice(&MAGIC.to_le_bytes());
        self.scratch.push(reply as u8);
        self.transport.send_to(to, &self.scratch);
    }

    fn send_inputs(&mut self) {
        let local = self.cfg.local_player;
        let last = self.last_confirmed[local];
        for p in self.remotes().collect::<Vec<_>>() {
            if self.peers[p].disconnected {
                continue;
            }
            let start = (self.peers[p].ack + 1).max(last - MAX_INPUTS_PER_PACKET + 1).max(0);
            let count = (last - start + 1).max(0);
            self.scratch.clear();
            self.scratch.push(PKT_INPUT);
            self.scratch.push(local as u8);
            self.scratch.extend_from_slice(&self.last_confirmed[p].to_le_bytes());
            self.scratch.extend_from_slice(&self.frame.to_le_bytes());
            let adv = (self.peers[p].local_adv * 16.0).clamp(-32000.0, 32000.0) as i16;
            self.scratch.extend_from_slice(&adv.to_le_bytes());
            self.scratch.extend_from_slice(&start.to_le_bytes());
            self.scratch.push(count as u8);
            for f in start..start + count {
                let i = self.confirmed[local].get(f).unwrap_or_default();
                i.write(&mut self.scratch);
            }
            self.transport.send_to(p, &self.scratch);
        }
    }

    fn handle_packet(&mut self, pkt: &[u8]) {
        if pkt.len() < 2 {
            return;
        }
        let kind = pkt[0];
        let p = pkt[1] as usize;
        if p >= self.cfg.num_players || p == self.cfg.local_player {
            return;
        }
        let rd_i32 = |o: usize| -> Option<i32> {
            pkt.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        match kind {
            PKT_HELLO => {
                let Some(magic) = rd_i32(2) else { return };
                if magic as u32 != MAGIC {
                    return;
                }
                self.mark_heard(p);
                if pkt.get(6) == Some(&0) {
                    self.send_hello(p, true);
                }
            }
            PKT_INPUT => {
                let (Some(ack), Some(rframe), Some(start)) = (rd_i32(2), rd_i32(6), rd_i32(12)) else {
                    return;
                };
                let Some(adv) = pkt.get(10..12).map(|b| i16::from_le_bytes([b[0], b[1]])) else {
                    return;
                };
                let Some(&count) = pkt.get(16) else { return };
                let sz = G::Input::SIZE;
                if pkt.len() < 17 + count as usize * sz {
                    return;
                }
                self.mark_heard(p);
                let peer = &mut self.peers[p];
                peer.ack = peer.ack.max(ack);
                peer.remote_frame = peer.remote_frame.max(rframe);
                peer.remote_adv = adv as f32 / 16.0;
                for k in 0..count as i32 {
                    let f = start + k;
                    if f != self.last_confirmed[p] + 1 {
                        continue; // already have it (or a gap: wait for resend)
                    }
                    let o = 17 + k as usize * sz;
                    let input = G::Input::read(&pkt[o..o + sz]);
                    self.confirmed[p].set(f, input);
                    self.last_confirmed[p] = f;
                    self.last_input[p] = input;
                    if f < self.frame && self.used[p].get(f) != Some(input) {
                        self.first_incorrect = self.first_incorrect.min(f);
                    }
                }
            }
            PKT_CHECKSUM => {
                let (Some(f), Some(sum)) = (
                    rd_i32(2),
                    pkt.get(6..14).map(|b| u64::from_le_bytes(b.try_into().unwrap())),
                ) else {
                    return;
                };
                self.mark_heard(p);
                if !self.remote_checksums.iter().any(|&(pp, ff, _)| pp == p && ff == f) {
                    self.remote_checksums.push((p, f, sum));
                }
            }
            _ => {}
        }
    }

    fn mark_heard(&mut self, p: usize) {
        let peer = &mut self.peers[p];
        peer.last_recv_tick = self.tick;
        if !peer.synced {
            peer.synced = true;
            self.events.push(SessionEvent::Connected { player: p });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fnv64, SimNetConfig, SimNetwork, SimTransport, SyncTestSession};

    /// Toy game whose state is a hash chain over every input it has seen,
    /// so any input ordering/prediction bug changes the checksum.
    #[derive(Clone, Default)]
    struct ChainGame {
        h: Fnv64,
        frame: i32,
        pos: [i32; 2],
    }

    impl RollbackGame for ChainGame {
        type Input = u8;
        fn step(&mut self, inputs: &[u8]) {
            for (i, inp) in inputs.iter().enumerate() {
                self.h.write_u8(*inp);
                self.pos[i] += (*inp as i32 & 3) - 1;
            }
            self.frame += 1;
            self.h.write_i32(self.frame);
        }
        fn checksum(&self) -> u64 {
            let mut h = self.h;
            h.write_i32(self.pos[0]);
            h.write_i32(self.pos[1]);
            h.finish()
        }
    }

    fn input_pattern(player: usize, tick: u32) -> u8 {
        // Changes every few ticks so predictions are often wrong.
        let x = (tick / (3 + player as u32 * 2)).wrapping_mul(2_654_435_761u32 >> (player * 3));
        (x >> 7) as u8 & 7
    }

    fn run_pair(cfg: SimNetConfig, ticks: u32) -> (P2PSession<ChainGame, SimTransport>, P2PSession<ChainGame, SimTransport>) {
        let net = SimNetwork::new(cfg, 0xDEADBEEF);
        let mut a = P2PSession::new(ChainGame::default(), SimNetwork::transport(&net, 0), SessionConfig::new(2, 0));
        let mut b = P2PSession::new(ChainGame::default(), SimNetwork::transport(&net, 1), SessionConfig::new(2, 1));
        for t in 0..ticks {
            net.borrow_mut().tick();
            a.poll();
            if !a.should_wait() {
                a.advance(input_pattern(0, t));
            }
            // b runs a tick "late" some of the time to exercise time sync
            if t % 97 != 0 {
                b.poll();
                if !b.should_wait() {
                    b.advance(input_pattern(1, t));
                }
            }
        }
        (a, b)
    }

    fn assert_agree(a: &P2PSession<ChainGame, SimTransport>, b: &P2PSession<ChainGame, SimTransport>) {
        let mut evs = Vec::new();
        for e in a.events.iter().chain(b.events.iter()) {
            if let SessionEvent::Desync { .. } = e {
                evs.push(e.clone());
            }
        }
        assert!(evs.is_empty(), "desync events: {evs:?}");
        let conf = a.confirmed_frame().min(b.confirmed_frame());
        assert!(conf > 300, "too little progress: confirmed frame {conf}");
        let mut compared = 0;
        for f in (conf - 60).max(0)..=conf {
            if let (Some(x), Some(y)) = (a.checksum_at(f), b.checksum_at(f)) {
                assert_eq!(x, y, "checksum mismatch at frame {f}");
                compared += 1;
            }
        }
        assert!(compared > 10);
    }

    #[test]
    fn clean_network_agrees() {
        let (a, b) = run_pair(SimNetConfig { latency: 2, jitter: 0, loss: 0.0 }, 900);
        assert_agree(&a, &b);
    }

    #[test]
    fn laggy_lossy_network_agrees_with_rollbacks() {
        let (a, b) = run_pair(SimNetConfig { latency: 6, jitter: 4, loss: 0.15 }, 1500);
        assert_agree(&a, &b);
        assert!(a.stats.rollbacks + b.stats.rollbacks > 50, "expected rollbacks: {:?} {:?}", a.stats, b.stats);
    }

    #[test]
    fn peers_stay_in_time_sync() {
        let (a, b) = run_pair(SimNetConfig { latency: 4, jitter: 1, loss: 0.02 }, 1200);
        assert!((a.frame() - b.frame()).abs() <= MAX_PREDICTION, "frames {} vs {}", a.frame(), b.frame());
    }

    #[test]
    fn disconnect_is_detected() {
        let net = SimNetwork::new(SimNetConfig { latency: 1, jitter: 0, loss: 0.0 }, 7);
        let mut cfg = SessionConfig::new(2, 0);
        cfg.disconnect_ticks = 30;
        let mut a = P2PSession::new(ChainGame::default(), SimNetwork::transport(&net, 0), cfg);
        let mut b = P2PSession::new(ChainGame::default(), SimNetwork::transport(&net, 1), SessionConfig::new(2, 1));
        for t in 0..200u32 {
            net.borrow_mut().tick();
            a.poll();
            a.advance(1);
            if t < 60 {
                b.poll();
                b.advance(2);
            }
        }
        assert!(a.drain_events().contains(&SessionEvent::Disconnected { player: 1 }));
        assert!(a.frame() > 150, "should keep running after disconnect");
    }

    #[test]
    fn synctest_accepts_deterministic_game() {
        let mut s = SyncTestSession::new(ChainGame::default(), 7);
        for t in 0..300 {
            s.advance(&[input_pattern(0, t), input_pattern(1, t)]).unwrap();
        }
    }

    #[test]
    fn real_udp_loopback_agrees() {
        use crate::UdpTransport;
        use std::net::SocketAddr;
        let a_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        // bind both first to learn the OS-assigned ports
        let ta = UdpTransport::bind(a_addr, vec![None, None]).unwrap();
        let tb = UdpTransport::bind(a_addr, vec![None, None]).unwrap();
        let (pa, pb) = (ta.local_addr().unwrap(), tb.local_addr().unwrap());
        drop((ta, tb));
        let ta = UdpTransport::bind(pa, vec![None, Some(pb)]).unwrap();
        let tb = UdpTransport::bind(pb, vec![Some(pa), None]).unwrap();
        let mut a = P2PSession::new(ChainGame::default(), ta, SessionConfig::new(2, 0));
        let mut b = P2PSession::new(ChainGame::default(), tb, SessionConfig::new(2, 1));
        for t in 0..600u32 {
            a.poll();
            if !a.should_wait() {
                a.advance(input_pattern(0, t));
            }
            b.poll();
            if !b.should_wait() {
                b.advance(input_pattern(1, t));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let conf = a.confirmed_frame().min(b.confirmed_frame()).min(a.frame() - 1).min(b.frame() - 1);
        assert!(conf > 400, "confirmed only {conf}");
        for f in (conf - 30)..=conf {
            let (x, y) = (a.checksum_at(f), b.checksum_at(f));
            assert!(x.is_some() && x == y, "frame {f}: {x:?} vs {y:?}");
        }
    }
}
