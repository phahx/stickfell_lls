//! A room's match settings (free for all or teams, AI fighters), as the host
//! picks them, and getting them to everyone in the room before it starts.
//!
//! The relay only knows who's in a room, so the host sends its settings to
//! each player through it (as game data, while still in the room) and each
//! answers with the version they got. The host can start once every player
//! has the latest: everyone then builds the same match.

use crate::sim::MAX_PLAYERS;

/// Settings packets (the first byte), apart from the match's own.
const RULES: u8 = 0xE1;
const GOT: u8 = 0xE2;

/// What the host picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    /// Team size (2 for 2v2), or 0 for every one for themselves.
    pub teams: u8,
    /// Every one for themselves: how many AI fighters join in. Teams: 1 to
    /// fill the teams up with AI fighters, 0 not to.
    pub ai: u8,
}

impl Rules {
    /// How many AI fighters join a match with `humans` players in it.
    pub fn bots(&self, humans: usize) -> usize {
        let room = MAX_PLAYERS.saturating_sub(humans);
        if self.teams > 0 {
            if self.ai > 0 { (self.teams as usize * 2).saturating_sub(humans).min(room) } else { 0 }
        } else {
            (self.ai as usize).min(room)
        }
    }

    /// Why a match with `humans` players can't start like this, if it can't.
    pub fn problem(&self, humans: usize) -> Option<String> {
        if self.teams == 0 {
            return None;
        }
        let full = self.teams as usize * 2;
        if humans > full {
            Some(format!("Too many players for {}", self.mode()))
        } else if humans < full && self.ai == 0 {
            Some(format!("{} needs {full} players - or turn on AI to fill the teams", self.mode()))
        } else {
            None
        }
    }

    pub fn mode(&self) -> String {
        match self.teams {
            0 => "Free for all".to_string(),
            n => format!("{n}v{n}"),
        }
    }

    pub fn ai_label(&self, humans: usize) -> String {
        match (self.teams, self.bots(humans)) {
            (_, 0) if self.ai == 0 => "AI: off".to_string(),
            (0, 1) => "AI: 1 fighter".to_string(),
            (0, n) => format!("AI: {n} fighters"),
            (_, 0) => "AI: fill teams (full)".to_string(),
            (_, n) => format!("AI: fill teams ({n})"),
        }
    }

    /// The next mode: free for all, 2v2, 3v3, 4v4, and round again.
    pub fn next_mode(self) -> Self {
        let teams = match self.teams {
            0 => 2,
            4 => 0,
            n => n + 1,
        };
        Self { teams, ai: if teams > 0 { self.ai.min(1) } else { self.ai } }
    }

    /// More AI fighters (back to none past as many as there's room for), or
    /// in teams, AI filling them or not.
    pub fn next_ai(self, humans: usize) -> Self {
        let ai = if self.teams > 0 {
            1 - self.ai.min(1)
        } else if self.ai as usize >= MAX_PLAYERS.saturating_sub(humans) {
            0
        } else {
            self.ai + 1
        };
        Self { ai, ..self }
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let (&teams, &ai) = (b.first()?, b.get(1)?);
        (matches!(teams, 0 | 2..=4) && (ai as usize) < MAX_PLAYERS).then_some(Self { teams, ai })
    }
}

/// The settings on their way round the room.
#[derive(Default)]
pub struct RoomSync {
    /// The host's settings, or (anyone else) the last ones heard from it.
    pub rules: Rules,
    /// Heard from the host yet (always, for the host).
    pub heard: bool,
    /// Bumped with every change.
    version: u16,
    /// Host: the version each slot has said it has.
    got: Vec<u16>,
    /// Players in the room the last time (when it changes, the slots move
    /// up, so who has what is asked again).
    players: usize,
}

impl RoomSync {
    pub fn host() -> Self {
        Self { heard: true, version: 1, ..Default::default() }
    }

    /// Host: new settings (sent round from the next `outgoing`).
    pub fn set(&mut self, rules: Rules) {
        if rules != self.rules {
            self.rules = rules;
            self.version = self.version.wrapping_add(1).max(1);
        }
    }

    /// Host: what to send to each other player now (call every quarter
    /// second or so): the settings, to slots 1.. of `players`.
    pub fn outgoing(&mut self, players: usize) -> Vec<(u8, Vec<u8>)> {
        if players != self.players {
            self.players = players;
            self.got = vec![0; players];
        }
        let v = self.version.to_le_bytes();
        (1..players).map(|slot| (slot as u8, vec![RULES, v[0], v[1], self.rules.teams, self.rules.ai])).collect()
    }

    /// Host: whether every player in the room has the latest settings.
    pub fn everyone_has_them(&self, players: usize) -> bool {
        players == self.players && (1..players).all(|s| self.got.get(s) == Some(&self.version))
    }

    /// A packet from `slot` while in the room. Returns the answer to send
    /// back to the host, if any.
    pub fn receive(&mut self, from: usize, pkt: &[u8], hosting: bool) -> Option<Vec<u8>> {
        match pkt.first()? {
            &RULES if !hosting && from == 0 => {
                let rules = Rules::decode(pkt.get(3..)?)?;
                let v = [*pkt.get(1)?, *pkt.get(2)?];
                self.rules = rules;
                self.heard = true;
                Some(vec![GOT, v[0], v[1]])
            }
            &GOT if hosting => {
                let v = u16::from_le_bytes([*pkt.get(1)?, *pkt.get(2)?]);
                if let Some(g) = self.got.get_mut(from) {
                    *g = v;
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_get_round_the_room_and_changes_have_to_be_heard_again() {
        let mut host = RoomSync::host();
        let mut guests = [RoomSync::default(), RoomSync::default()];
        let exchange = |host: &mut RoomSync, guests: &mut [RoomSync; 2]| {
            for (slot, pkt) in host.outgoing(3) {
                if let Some(back) = guests[slot as usize - 1].receive(0, &pkt, false) {
                    host.receive(slot as usize, &back, true);
                }
            }
        };
        assert!(!host.everyone_has_them(3), "nobody's heard anything yet");
        host.set(Rules { teams: 3, ai: 1 });
        exchange(&mut host, &mut guests);
        assert!(host.everyone_has_them(3));
        assert!(guests.iter().all(|g| g.heard && g.rules == Rules { teams: 3, ai: 1 }));
        host.set(Rules { teams: 0, ai: 2 });
        assert!(!host.everyone_has_them(3), "a change has to go round again");
        exchange(&mut host, &mut guests);
        assert!(host.everyone_has_them(3) && guests[1].rules.ai == 2);
        // someone joins: they have to hear it too
        let _ = host.outgoing(4);
        assert!(!host.everyone_has_them(4));
        // (and nobody but the host is listened to)
        let mut g = RoomSync::default();
        assert!(g.receive(1, &[RULES, 1, 0, 2, 1], false).is_none() && !g.heard);
    }

    #[test]
    fn ai_fills_teams_or_joins_in_and_bad_team_sizes_are_caught() {
        let ffa = Rules { teams: 0, ai: 3 };
        assert_eq!(ffa.bots(2), 3);
        assert_eq!(ffa.bots(7), 1, "never more than eight in all");
        let fill = Rules { teams: 3, ai: 1 };
        assert_eq!(fill.bots(2), 4);
        assert_eq!(fill.problem(2), None);
        assert!(Rules { teams: 2, ai: 0 }.problem(3).is_some(), "2v2 needs 4 without AI");
        assert!(Rules { teams: 2, ai: 1 }.problem(5).is_some(), "too many for 2v2");
        assert_eq!(Rules { teams: 4, ai: 0 }.problem(8), None);
        // round the modes and back
        let mut r = Rules::default();
        let modes: Vec<u8> = (0..5).map(|_| { r = r.next_mode(); r.teams }).collect();
        assert_eq!(modes, [2, 3, 4, 0, 2]);
        // AI counts go up to the room left, then off
        let mut r = Rules::default();
        let counts: Vec<u8> = (0..4).map(|_| { r = r.next_ai(6); r.ai }).collect();
        assert_eq!(counts, [1, 2, 0, 1]);
    }
}
