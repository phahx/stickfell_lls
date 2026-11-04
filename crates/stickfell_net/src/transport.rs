//! Packet transports. Sessions address peers by player index.

use std::cell::RefCell;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;

pub trait Transport {
    /// Unreliable, unordered send to the peer that owns `player`.
    fn send_to(&mut self, player: usize, data: &[u8]);
    /// Next received datagram, if any (non-blocking).
    fn recv(&mut self) -> Option<Vec<u8>>;
}

impl Transport for Box<dyn Transport> {
    fn send_to(&mut self, player: usize, data: &[u8]) {
        (**self).send_to(player, data)
    }
    fn recv(&mut self) -> Option<Vec<u8>> {
        (**self).recv()
    }
}

/// Non-blocking UDP transport. `peers[player]` is that player's address
/// (`None` for the local player).
pub struct UdpTransport {
    socket: UdpSocket,
    peers: Vec<Option<SocketAddr>>,
    buf: Box<[u8; 2048]>,
}

impl UdpTransport {
    pub fn bind(local: SocketAddr, peers: Vec<Option<SocketAddr>>) -> io::Result<Self> {
        let socket = UdpSocket::bind(local)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, peers, buf: Box::new([0; 2048]) })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Transport for UdpTransport {
    fn send_to(&mut self, player: usize, data: &[u8]) {
        if let Some(Some(addr)) = self.peers.get(player) {
            let _ = self.socket.send_to(data, addr);
        }
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        loop {
            match self.socket.recv_from(&mut self.buf[..]) {
                Ok((n, from)) => {
                    // Only accept datagrams from known peers.
                    if self.peers.iter().any(|p| *p == Some(from)) {
                        return Some(self.buf[..n].to_vec());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
                // Windows reports ICMP "port unreachable" as ConnectionReset on
                // the next recv; it just means a peer isn't up yet.
                Err(_) => continue,
            }
        }
    }
}

// ---- simulated network ---------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct SimNetConfig {
    /// One-way latency in ticks.
    pub latency: u32,
    /// Extra random latency in [0, jitter] ticks (causes reordering).
    pub jitter: u32,
    /// Probability in [0, 1) that a packet is dropped.
    pub loss: f32,
}

struct SimPacket {
    to: usize,
    deliver_at: u64,
    seq: u64,
    data: Vec<u8>,
}

/// In-process lossy network shared by several [`SimTransport`]s. Fully
/// deterministic (seeded), so netcode tests are reproducible.
pub struct SimNetwork {
    cfg: SimNetConfig,
    now: u64,
    seq: u64,
    rng: u64,
    queue: Vec<SimPacket>,
}

impl SimNetwork {
    pub fn new(cfg: SimNetConfig, seed: u64) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self { cfg, now: 0, seq: 0, rng: seed | 1, queue: Vec::new() }))
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Advance network time by one tick.
    pub fn tick(&mut self) {
        self.now += 1;
    }

    pub fn set_config(&mut self, cfg: SimNetConfig) {
        self.cfg = cfg;
    }

    pub fn transport(net: &Rc<RefCell<Self>>, me: usize) -> SimTransport {
        SimTransport { net: net.clone(), me }
    }
}

pub struct SimTransport {
    net: Rc<RefCell<SimNetwork>>,
    me: usize,
}

impl Transport for SimTransport {
    fn send_to(&mut self, player: usize, data: &[u8]) {
        let _ = self.me;
        let mut net = self.net.borrow_mut();
        let roll = (net.rand() % 10_000) as f32 / 10_000.0;
        if roll < net.cfg.loss {
            return;
        }
        let jitter = if net.cfg.jitter > 0 { net.rand() % (net.cfg.jitter as u64 + 1) } else { 0 };
        let deliver_at = net.now + net.cfg.latency as u64 + jitter;
        net.seq += 1;
        let seq = net.seq;
        net.queue.push(SimPacket { to: player, deliver_at, seq, data: data.to_vec() });
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        let mut net = self.net.borrow_mut();
        let now = net.now;
        let mut best: Option<usize> = None;
        for (i, p) in net.queue.iter().enumerate() {
            if p.to == self.me && p.deliver_at <= now {
                let better = match best {
                    None => true,
                    Some(b) => {
                        let q = &net.queue[b];
                        (p.deliver_at, p.seq) < (q.deliver_at, q.seq)
                    }
                };
                if better {
                    best = Some(i);
                }
            }
        }
        best.map(|i| net.queue.swap_remove(i).data)
    }
}
