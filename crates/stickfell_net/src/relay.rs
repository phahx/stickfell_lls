//! Relay server, lobby client and relay transport.
//!
//! Players never connect to each other directly: every client talks only to
//! the relay (outbound UDP, so no port forwarding or NAT punching), which
//! groups them into rooms identified by a short code and forwards game
//! packets between room members.
//!
//! Protocol (first byte = message kind):
//!
//! | dir | kind | body |
//! |---|---|---|
//! | C→S | `HOST` | max players |
//! | C→S | `JOIN` | 4-byte room code |
//! | C→S | `START` | – (host only) |
//! | C→S | `KEEPALIVE` | – (server answers with current ROOM/START) |
//! | C→S | `LEAVE` | – |
//! | C→S | `RELAY` | to-slot, payload |
//! | S→C | `ROOM` | code[4], your slot, players, max |
//! | S→C | `ERROR` | utf-8 message |
//! | S→C | `START` | players, your slot, seed u32 |
//! | S→C | `DATA` | from-slot, payload |
//!
//! `RELAY` works in a room before the match starts too: the game uses it to
//! send the host's match settings round (see `LobbyClient::send_data`).
//!
//! Lobby messages are idempotent and clients resend them until the state
//! they expect arrives, so packet loss is harmless.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use crate::Transport;

const C_HOST: u8 = 0x10;
const C_JOIN: u8 = 0x11;
const C_START: u8 = 0x12;
const C_RELAY: u8 = 0x13;
const C_KEEPALIVE: u8 = 0x14;
const C_LEAVE: u8 = 0x15;
const S_ROOM: u8 = 0x20;
const S_ERROR: u8 = 0x21;
const S_START: u8 = 0x22;
const S_DATA: u8 = 0x23;

pub const DEFAULT_RELAY_PORT: u16 = 7777;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";

// ---- server ---------------------------------------------------------------

struct Member {
    addr: SocketAddr,
    last_seen: Instant,
}

struct Room {
    code: [u8; 4],
    max: u8,
    members: Vec<Member>,
    seed: Option<u32>,
}

/// The relay server. Run [`RelayServer::run`] in a process of its own (see
/// the `stickfell_relay` binary) or call [`RelayServer::poll`] from a thread.
pub struct RelayServer {
    sock: UdpSocket,
    rooms: Vec<Room>,
    index: HashMap<SocketAddr, [u8; 4]>,
    rng: u64,
    buf: Vec<u8>,
    out: Vec<u8>,
    last_sweep: Instant,
    pub verbose: bool,
}

impl RelayServer {
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let sock = UdpSocket::bind(addr)?;
        sock.set_read_timeout(Some(Duration::from_millis(50)))?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678);
        Ok(Self {
            sock,
            rooms: Vec::new(),
            index: HashMap::new(),
            rng: seed | 1,
            buf: vec![0; 2048],
            out: Vec::with_capacity(2048),
            last_sweep: Instant::now(),
            verbose: false,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn new_code(&mut self) -> [u8; 4] {
        loop {
            let mut c = [0u8; 4];
            for ch in &mut c {
                *ch = CODE_CHARS[(self.rand() % CODE_CHARS.len() as u64) as usize];
            }
            if !self.rooms.iter().any(|r| r.code == c) {
                return c;
            }
        }
    }

    /// Serves forever.
    pub fn run(&mut self) -> ! {
        loop {
            self.poll();
        }
    }

    /// Handles everything received within one read timeout (~50 ms).
    pub fn poll(&mut self) {
        loop {
            let (n, from) = match self.sock.recv_from(&mut self.buf) {
                Ok(v) => v,
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                Err(_) => continue, // e.g. Windows ConnectionReset
            };
            let msg = self.buf[..n].to_vec();
            self.handle(from, &msg);
            // keep draining without blocking
            self.sock.set_nonblocking(true).ok();
        }
        self.sock.set_nonblocking(false).ok();
        if self.last_sweep.elapsed() > Duration::from_secs(1) {
            self.sweep();
            self.last_sweep = Instant::now();
        }
    }

    fn sweep(&mut self) {
        let now = Instant::now();
        let mut gone = Vec::new();
        for r in &mut self.rooms {
            r.members.retain(|m| {
                let keep = now.duration_since(m.last_seen) < CLIENT_TIMEOUT;
                if !keep {
                    gone.push(m.addr);
                }
                keep
            });
        }
        for a in gone {
            self.index.remove(&a);
        }
        let verbose = self.verbose;
        self.rooms.retain(|r| {
            let keep = !r.members.is_empty();
            if !keep && verbose {
                eprintln!("[relay] room {} closed", String::from_utf8_lossy(&r.code));
            }
            keep
        });
    }

    fn room_of(&self, addr: &SocketAddr) -> Option<usize> {
        let code = self.index.get(addr)?;
        self.rooms.iter().position(|r| &r.code == code)
    }

    fn send(&self, to: SocketAddr, data: &[u8]) {
        let _ = self.sock.send_to(data, to);
    }

    fn send_state(&mut self, room: usize, to_slot: usize) {
        let r = &self.rooms[room];
        let addr = r.members[to_slot].addr;
        self.out.clear();
        match r.seed {
            None => {
                self.out.push(S_ROOM);
                self.out.extend_from_slice(&r.code);
                self.out.extend_from_slice(&[to_slot as u8, r.members.len() as u8, r.max]);
            }
            Some(seed) => {
                self.out.push(S_START);
                self.out.extend_from_slice(&[r.members.len() as u8, to_slot as u8]);
                self.out.extend_from_slice(&seed.to_le_bytes());
            }
        }
        let out = std::mem::take(&mut self.out);
        self.send(addr, &out);
        self.out = out;
    }

    fn broadcast_state(&mut self, room: usize) {
        for slot in 0..self.rooms[room].members.len() {
            self.send_state(room, slot);
        }
    }

    fn error(&self, to: SocketAddr, msg: &str) {
        let mut out = vec![S_ERROR];
        out.extend_from_slice(msg.as_bytes());
        self.send(to, &out);
    }

    fn leave(&mut self, addr: SocketAddr) {
        if let Some(ri) = self.room_of(&addr) {
            self.rooms[ri].members.retain(|m| m.addr != addr);
            self.index.remove(&addr);
            if self.rooms[ri].seed.is_none() && !self.rooms[ri].members.is_empty() {
                self.broadcast_state(ri);
            }
        }
    }

    fn handle(&mut self, from: SocketAddr, msg: &[u8]) {
        let Some(&kind) = msg.first() else { return };
        // refresh liveness
        if let Some(ri) = self.room_of(&from) {
            if let Some(m) = self.rooms[ri].members.iter_mut().find(|m| m.addr == from) {
                m.last_seen = Instant::now();
            }
        }
        match kind {
            C_HOST => {
                if let Some(ri) = self.room_of(&from) {
                    let slot = self.rooms[ri].members.iter().position(|m| m.addr == from).unwrap();
                    self.send_state(ri, slot);
                    return;
                }
                let max = msg.get(1).copied().unwrap_or(4).clamp(2, 8);
                let code = self.new_code();
                self.rooms.push(Room { code, max, members: vec![Member { addr: from, last_seen: Instant::now() }], seed: None });
                self.index.insert(from, code);
                if self.verbose {
                    eprintln!("[relay] {from} hosts room {}", String::from_utf8_lossy(&code));
                }
                let ri = self.rooms.len() - 1;
                self.send_state(ri, 0);
            }
            C_JOIN => {
                if let Some(ri) = self.room_of(&from) {
                    let slot = self.rooms[ri].members.iter().position(|m| m.addr == from).unwrap();
                    self.send_state(ri, slot);
                    return;
                }
                let Some(code) = msg.get(1..5) else { return };
                let code: [u8; 4] = code.to_ascii_uppercase().try_into().unwrap();
                let Some(ri) = self.rooms.iter().position(|r| r.code == code) else {
                    self.error(from, "No room with that code");
                    return;
                };
                let r = &mut self.rooms[ri];
                if r.seed.is_some() {
                    self.error(from, "That match has already started");
                    return;
                }
                if r.members.len() >= r.max as usize {
                    self.error(from, "That room is full");
                    return;
                }
                r.members.push(Member { addr: from, last_seen: Instant::now() });
                self.index.insert(from, code);
                if self.verbose {
                    eprintln!("[relay] {from} joins room {}", String::from_utf8_lossy(&code));
                }
                self.broadcast_state(ri);
            }
            C_START => {
                let Some(ri) = self.room_of(&from) else { return };
                let is_host = self.rooms[ri].members.first().is_some_and(|m| m.addr == from);
                if !is_host {
                    return;
                }
                if self.rooms[ri].seed.is_none() {
                    if self.rooms[ri].members.len() < 2 {
                        self.error(from, "Need at least 2 players");
                        return;
                    }
                    let seed = self.rand() as u32;
                    self.rooms[ri].seed = Some(seed);
                    if self.verbose {
                        eprintln!(
                            "[relay] room {} starts with {} players",
                            String::from_utf8_lossy(&self.rooms[ri].code),
                            self.rooms[ri].members.len()
                        );
                    }
                }
                self.broadcast_state(ri);
            }
            C_KEEPALIVE => {
                if let Some(ri) = self.room_of(&from) {
                    let slot = self.rooms[ri].members.iter().position(|m| m.addr == from).unwrap();
                    self.send_state(ri, slot);
                }
            }
            C_LEAVE => self.leave(from),
            C_RELAY => {
                let (Some(&to), Some(ri)) = (msg.get(1), self.room_of(&from)) else { return };
                let r = &self.rooms[ri];
                let Some(from_slot) = r.members.iter().position(|m| m.addr == from) else { return };
                let Some(target) = r.members.get(to as usize) else { return };
                self.out.clear();
                self.out.push(S_DATA);
                self.out.push(from_slot as u8);
                self.out.extend_from_slice(&msg[2..]);
                let _ = self.sock.send_to(&self.out, target.addr);
            }
            _ => {}
        }
    }
}

// ---- client ---------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyState {
    Connecting,
    InRoom { code: String, slot: usize, players: usize, max: usize },
    Started { num_players: usize, slot: usize, seed: u32 },
    Failed(String),
}

enum Intent {
    Host(u8),
    Join([u8; 4]),
}

/// Client side of the lobby: host or join a room, wait, then turn into a
/// [`RelayTransport`] once the match starts.
pub struct LobbyClient {
    sock: UdpSocket,
    server: SocketAddr,
    intent: Intent,
    state: LobbyState,
    last_send: Option<Instant>,
    created: Instant,
    last_heard: Instant,
    start_requested: bool,
    /// Data from other players in the room (from-slot, payload), before the
    /// match starts (the host's match settings): see `take_data`.
    inbox: Vec<(usize, Vec<u8>)>,
}

impl LobbyClient {
    fn new(server: &str, intent: Intent) -> io::Result<Self> {
        let server = server
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "relay address did not resolve"))?;
        let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
        let sock = UdpSocket::bind(bind)?;
        sock.set_nonblocking(true)?;
        let now = Instant::now();
        Ok(Self {
            sock,
            server,
            intent,
            state: LobbyState::Connecting,
            last_send: None,
            created: now,
            last_heard: now,
            start_requested: false,
            inbox: Vec::new(),
        })
    }

    /// Creates a room for up to `max_players`.
    pub fn host(server: &str, max_players: u8) -> io::Result<Self> {
        Self::new(server, Intent::Host(max_players))
    }

    /// Joins the room with this 4-letter code.
    pub fn join(server: &str, code: &str) -> io::Result<Self> {
        let mut c = [b'A'; 4];
        for (i, b) in code.bytes().take(4).enumerate() {
            c[i] = b.to_ascii_uppercase();
        }
        Self::new(server, Intent::Join(c))
    }

    pub fn state(&self) -> &LobbyState {
        &self.state
    }

    pub fn is_host(&self) -> bool {
        matches!(self.intent, Intent::Host(_))
    }

    /// Host only: start the match with everyone currently in the room.
    pub fn start(&mut self) {
        self.start_requested = true;
        self.last_send = None;
    }

    /// Sends `payload` to the player in `slot` of the room, through the relay.
    pub fn send_data(&self, slot: u8, payload: &[u8]) {
        let mut msg = Vec::with_capacity(payload.len() + 2);
        msg.push(C_RELAY);
        msg.push(slot);
        msg.extend_from_slice(payload);
        let _ = self.sock.send_to(&msg, self.server);
    }

    /// Data other players in the room have sent since last asked.
    pub fn take_data(&mut self) -> Vec<(usize, Vec<u8>)> {
        std::mem::take(&mut self.inbox)
    }

    pub fn leave(&mut self) {
        let _ = self.sock.send_to(&[C_LEAVE], self.server);
    }

    /// Network I/O; call every frame/tick.
    pub fn poll(&mut self) -> &LobbyState {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            if from != self.server || n == 0 {
                continue;
            }
            self.last_heard = Instant::now();
            let m = &buf[..n];
            match m[0] {
                S_ROOM if n >= 8 && !matches!(self.state, LobbyState::Started { .. }) => {
                    self.state = LobbyState::InRoom {
                        code: String::from_utf8_lossy(&m[1..5]).into_owned(),
                        slot: m[5] as usize,
                        players: m[6] as usize,
                        max: m[7] as usize,
                    };
                }
                S_START if n >= 7 => {
                    self.state = LobbyState::Started {
                        num_players: m[1] as usize,
                        slot: m[2] as usize,
                        seed: u32::from_le_bytes([m[3], m[4], m[5], m[6]]),
                    };
                }
                // (only while in the room: once it's started, it's the match's)
                S_DATA if n >= 2 && matches!(self.state, LobbyState::InRoom { .. }) && self.inbox.len() < 64 => {
                    self.inbox.push((m[1] as usize, m[2..].to_vec()));
                }
                S_ERROR => {
                    let msg = String::from_utf8_lossy(&m[1..]).into_owned();
                    if self.start_requested && matches!(self.state, LobbyState::InRoom { .. }) {
                        // e.g. "need at least 2 players": stay in the room
                        self.start_requested = false;
                    } else {
                        self.state = LobbyState::Failed(msg);
                    }
                }
                _ => {}
            }
        }
        if matches!(self.state, LobbyState::Failed(_) | LobbyState::Started { .. }) {
            return &self.state;
        }
        if self.last_heard.elapsed() > Duration::from_secs(6) {
            self.state = LobbyState::Failed("The relay isn't responding".into());
            return &self.state;
        }
        let due = self.last_send.is_none_or(|t| t.elapsed() > Duration::from_millis(250));
        if due {
            let msg: Vec<u8> = match (&self.state, &self.intent) {
                (LobbyState::Connecting, Intent::Host(max)) => vec![C_HOST, *max],
                (LobbyState::Connecting, Intent::Join(code)) => [&[C_JOIN][..], code].concat(),
                _ if self.start_requested => vec![C_START],
                _ => vec![C_KEEPALIVE],
            };
            let _ = self.sock.send_to(&msg, self.server);
            self.last_send = Some(Instant::now());
        }
        let _ = self.created;
        &self.state
    }

    /// Converts into the match transport (valid once `Started`).
    pub fn into_transport(self) -> RelayTransport {
        RelayTransport { sock: self.sock, server: self.server, buf: Box::new([0; 2048]), out: Vec::with_capacity(512) }
    }
}

/// Sends game packets through the relay.
pub struct RelayTransport {
    sock: UdpSocket,
    server: SocketAddr,
    buf: Box<[u8; 2048]>,
    out: Vec<u8>,
}

impl RelayTransport {
    pub fn leave(&self) {
        let _ = self.sock.send_to(&[C_LEAVE], self.server);
    }
}

impl Transport for RelayTransport {
    fn send_to(&mut self, player: usize, data: &[u8]) {
        self.out.clear();
        self.out.push(C_RELAY);
        self.out.push(player as u8);
        self.out.extend_from_slice(data);
        let _ = self.sock.send_to(&self.out, self.server);
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        loop {
            match self.sock.recv_from(&mut self.buf[..]) {
                Ok((n, from)) => {
                    if from == self.server && n >= 2 && self.buf[0] == S_DATA {
                        return Some(self.buf[2..n].to_vec());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
                Err(_) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{P2PSession, RollbackGame, SessionConfig};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct Sum(u64, i32);
    impl RollbackGame for Sum {
        type Input = u8;
        fn step(&mut self, inputs: &[u8]) {
            for (i, x) in inputs.iter().enumerate() {
                self.0 = self.0.wrapping_mul(31).wrapping_add(*x as u64 * (i as u64 + 1));
            }
            self.1 += 1;
        }
        fn checksum(&self) -> u64 {
            self.0 ^ self.1 as u64
        }
    }

    #[test]
    fn host_join_start_and_play_through_relay() {
        let mut server = RelayServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                server.poll();
            }
        });

        let mut host = LobbyClient::host(&addr, 4).unwrap();
        let code = loop {
            if let LobbyState::InRoom { code, .. } = host.poll() {
                break code.clone();
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let mut guest = LobbyClient::join(&addr, &code.to_lowercase()).unwrap();
        loop {
            host.poll();
            if let LobbyState::InRoom { players: 2, slot: 1, .. } = guest.poll() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        // room data both ways before the start (the host's match settings,
        // and the answer)
        let (mut got, mut back) = (None, None);
        for _ in 0..500 {
            host.send_data(1, &[0xE1, 7]);
            host.poll();
            guest.poll();
            if let Some((from, d)) = guest.take_data().into_iter().next() {
                got = Some((from, d));
                guest.send_data(0, &[0xE2, 7]);
            }
            if let Some(x) = host.take_data().into_iter().next() {
                back = Some(x);
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got, Some((0, vec![0xE1, 7])), "the guest hears the host in the room");
        assert_eq!(back, Some((1, vec![0xE2, 7])), "and the host hears back");
        host.start();
        let (mut hs, mut gs) = (None, None);
        while hs.is_none() || gs.is_none() {
            if let LobbyState::Started { num_players, slot, seed } = host.poll().clone() {
                hs = Some((num_players, slot, seed));
            }
            if let LobbyState::Started { num_players, slot, seed } = guest.poll().clone() {
                gs = Some((num_players, slot, seed));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let (hs, gs) = (hs.unwrap(), gs.unwrap());
        assert_eq!((hs.0, hs.1), (2, 0));
        assert_eq!((gs.0, gs.1), (2, 1));
        assert_eq!(hs.2, gs.2, "both peers get the same seed");

        let mut a = P2PSession::new(Sum::default(), host.into_transport(), SessionConfig::new(2, 0));
        let mut b = P2PSession::new(Sum::default(), guest.into_transport(), SessionConfig::new(2, 1));
        for t in 0..400u32 {
            a.poll();
            if !a.should_wait() {
                a.advance((t / 7) as u8);
            }
            b.poll();
            if !b.should_wait() {
                b.advance((t / 5) as u8 ^ 3);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
        let conf = a.confirmed_frame().min(b.confirmed_frame()).min(a.frame() - 1).min(b.frame() - 1);
        assert!(conf > 250, "confirmed only {conf}");
        for f in conf - 20..=conf {
            let (x, y) = (a.checksum_at(f), b.checksum_at(f));
            assert!(x.is_some() && x == y, "frame {f}");
        }
    }

    #[test]
    fn joining_unknown_room_fails() {
        let mut server = RelayServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let mut c = LobbyClient::join(&addr, "ZZZZ").unwrap();
        for _ in 0..40 {
            c.poll();
            server.poll();
            if matches!(c.state(), LobbyState::Failed(_)) {
                break;
            }
        }
        // one more round trip to deliver the error
        c.poll();
        assert_eq!(c.state(), &LobbyState::Failed("No room with that code".into()));
    }
}
