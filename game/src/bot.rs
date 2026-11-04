//! CPU brain used for headless tests and the menu's background demo match.
//! Reads only `GameState`, so it is deterministic.

use stickfell_physics::{Vec2, dmath};

use crate::fighter::{HEAD, NECK, PELVIS};
use crate::input::*;
use crate::sim::{CLASH_KEYS, ClashStage, GameState};

/// A practice dummy: gets back up, turns to face the player, never attacks.
pub fn dummy_input(s: &GameState, me: usize) -> u32 {
    let (Some(f), Some(p)) = (s.fighters.get(me), s.fighters.first()) else { return 0 };
    let to = p.pos(&s.world, PELVIS) - f.pos(&s.world, PELVIS);
    let aim = Vec2::new(if to.x >= 0.0 { 0.5 } else { -0.5 }, -0.2);
    encode(0, aim)
}

pub fn bot_input(s: &GameState, me: usize) -> u32 {
    let Some(f) = s.fighters.get(me) else { return 0 };
    if !f.alive {
        return 0;
    }
    // someone to finish off right here: do it
    if s.finish_target(me).is_some() && (s.frame + me as u32 * 5) % 12 == 0 {
        return encode(FINISH, f.aim);
    }
    // blade lock: answer each prompt after a human-ish delay, sometimes wrong
    if let Some(c) = s.clash {
        if let Some(side) = c.side_of(me) {
            if c.stage == ClashStage::Prompt && c.lockout[side] == 0 {
                let start = (c.total - c.t) as usize;
                let delay = 14 + (start * 7 + me * 13) % 24;
                if c.t as usize == delay {
                    let key = if (start + me) % 6 == 0 { (c.prompt as usize + 1) % 4 } else { c.prompt as usize };
                    return encode(CLASH_KEYS[key], f.aim);
                }
            }
            return encode(0, f.aim);
        }
    }
    let w = &s.world;
    let my = f.pos(w, PELVIS);
    let neck = f.pos(w, NECK);
    let tick = s.frame.wrapping_add(me as u32 * 37);
    // grabbed: thrash about to break free (a beat late, and not every frame)
    if f.held_by.is_some() {
        let k = (tick / 3) % 4;
        let bits = if k == 0 { LEFT } else if k == 2 { RIGHT } else { 0 };
        return encode(if f.struggle > 0 || tick % 20 > 6 { bits } else { 0 }, f.aim);
    }
    // someone tied to the hook in the grapple gun: fire it up at something
    // to hang them from
    if let Some(wi) = f.weapon {
        let tied = s.nooses.iter().any(|n| n.weapon == wi && !n.cut && n.hook == crate::sim::Hook::Loaded);
        if tied && !s.in_finisher(me) {
            let up = Vec2::new(f.facing * 0.25, 1.0).normalize() * 0.62;
            return encode(if tick % 45 == 0 { ATTACK } else { 0 }, up);
        }
    }
    // holding someone: haul them about for a moment, then heave them away, or
    // (every other hold) turn your back on them and throw them over your shoulder
    if let Some(v) = f.hold {
        let them = s.fighters[v as usize].pos(w, PELVIS);
        let away = if them.x >= my.x { 1.0 } else { -1.0 };
        let wait = 10 + (me as u16 * 7) % 15;
        let over = (me + v as usize) % 2 == 1 && f.hold_t + 4 >= wait;
        let throw = f.hold_t >= wait && f.heave == 0 && f.shoulder == 0 && tick % 4 == 0;
        let aim = Vec2::new(if over { -away } else { away } * 0.55, 0.35);
        return encode(GRAB | if throw { THROW } else { 0 }, aim);
    }
    let mut bits = 0u16;

    let enemy = s
        .fighters
        .iter()
        .filter(|o| o.alive && o.id != f.id)
        .min_by(|a, b| (a.pos(w, PELVIS) - my).length_squared().total_cmp(&(b.pos(w, PELVIS) - my).length_squared()));
    // unarmed: go get a weapon if one is reasonably close
    let weapon_goal = if f.weapon.is_none() {
        s.weapons
            .iter()
            .filter(|wp| wp.active && wp.holder.is_none())
            .map(|wp| w.particles[wp.base as usize].pos)
            .filter(|p| (*p - my).length() < 9.0)
            .min_by(|a, b| (*a - my).length_squared().total_cmp(&(*b - my).length_squared()))
    } else {
        None
    };
    let enemy_dist = enemy.map_or(f32::MAX, |e| (e.pos(w, PELVIS) - my).length());
    let goal = match (weapon_goal, enemy) {
        (Some(g), _) if enemy_dist > 2.5 => g,
        (_, Some(e)) => e.pos(w, PELVIS),
        (Some(g), None) => g,
        (None, None) => return encode(0, Vec2::new(f.facing * 0.5, 0.0)),
    };
    let d = goal - my;
    let dir = if d.x > 0.0 { 1.0 } else { -1.0 };
    let close = enemy_dist < 2.4;

    if d.x.abs() > 0.8 {
        bits |= if dir > 0.0 { RIGHT } else { LEFT };
    }
    if weapon_goal.is_some() && d.length() < 1.2 && tick % 10 == 0 {
        bits |= PICKUP;
    }

    // ledges: only jump a gap when there is somewhere to land
    if bits & (LEFT | RIGHT) != 0 && f.grounded {
        // hazards ahead (spikes, acid, saws) count as a drop to stay out of
        let ahead = my + Vec2::new(dir * 1.1, -0.9);
        let danger = s.hazard_near(ahead, 0.3) || s.hazard_near(my + Vec2::new(dir * 1.4, 0.0), 0.8);
        let edge = danger || w.raycast(my + Vec2::new(dir * 0.7, 0.2), Vec2::NEG_Y, 2.5).is_none();
        if edge {
            let landing = (2..=10).any(|k| {
                let x = my.x + dir * (0.5 * k as f32 + 0.7);
                w.raycast(Vec2::new(x, my.y + 2.2), Vec2::NEG_Y, 4.5).is_some_and(|h| !s.hazard_near(h.point, 0.4))
            });
            let my_turn = (s.frame / 150) as usize % s.num_players == me;
            let drop_ok = d.y < -0.8 && w.raycast(Vec2::new(my.x + dir * 0.9, my.y), Vec2::NEG_Y, 9.0).is_some();
            if drop_ok {
            } else if landing && d.x.abs() > 2.0 && d.y > -1.0 && my_turn {
                bits |= JUMP;
            } else {
                bits &= !(LEFT | RIGHT);
            }
        }
    }
    // climb toward goals above: get out from under ceilings first
    if d.y > 1.4 {
        if let Some(hit) = w.raycast(my + Vec2::new(0.0, 1.1), Vec2::Y, 2.6) {
            let c = w.statics[hit.index].corners();
            let min_x = c.iter().map(|p| p.x).fold(f32::MAX, f32::min);
            let max_x = c.iter().map(|p| p.x).fold(f32::MIN, f32::max);
            let right_cost = (max_x - my.x) + if d.x < 0.0 { 1.5 } else { 0.0 };
            let left_cost = (my.x - min_x) + if d.x > 0.0 { 1.5 } else { 0.0 };
            bits &= !(LEFT | RIGHT | JUMP);
            bits |= if right_cost < left_cost { RIGHT } else { LEFT };
        } else if f.grounded && tick % 30 < 10 {
            bits |= JUMP;
        }
    }
    if bits & (LEFT | RIGHT) != 0 && tick % 20 < 10 && w.raycast(my + Vec2::new(0.0, 0.3), Vec2::new(dir, 0.0), 0.8).is_some() {
        bits |= JUMP;
    }
    if !f.grounded && tick % 16 < 8 {
        bits |= JUMP;
    }
    if !close && bits & (LEFT | RIGHT) == 0 && tick % 240 < 45 {
        bits |= if dir > 0.0 { RIGHT } else { LEFT };
        if tick % 240 < 10 {
            bits |= JUMP;
        }
    }

    // aim: point at the enemy's head and swing back and forth through it
    let mut aim = Vec2::new(dir * 0.6, 0.1);
    if let Some(e) = enemy {
        let to = e.pos(w, HEAD) - neck;
        let base = dmath::angle(to);
        let swing = dmath::sin(tick as f32 * 0.22) * 1.4;
        aim = dmath::dir(base + swing) * 0.62;
        if close && f.weapon.is_some() {
            // click attacks: a fresh press every ~half second
            if tick % 64 < 2 {
                bits |= ATTACK;
            } else if (32..34).contains(&(tick % 64)) {
                bits |= ALT; // stab
            }
        } else if close {
            // bare hands: one-two-three combos, the odd low kick, and grabs
            aim = to.normalize_or(Vec2::X) * 0.62;
            match tick % 120 {
                0 | 9 | 18 | 60 | 69 => bits |= ATTACK,
                34..=46 => {
                    bits |= CROUCH;
                    if tick % 120 == 40 {
                        bits |= ATTACK;
                    }
                }
                85..=110 if e.held_by.is_none() => {
                    let to_neck = e.pos(w, NECK) - neck;
                    aim = to_neck.normalize_or(Vec2::X) * to_neck.length().min(0.62);
                    bits |= GRAB;
                }
                _ => {}
            }
        } else if f.weapon.is_none() && f.grounded && (2.6..4.2).contains(&enemy_dist) && tick % 240 == 0 {
            bits |= JUMP; // a dropkick, coming up
        }
        // mid-air and closing on them bare-handed: feet first
        if f.weapon.is_none() && f.air_t > 6 && f.kick == 0 && enemy_dist < 2.6 && tick % 240 < 40 {
            aim = (e.pos(w, NECK) - my).normalize_or(Vec2::X) * 0.6;
            bits |= ATTACK;
        }
        if f.weapon.is_some() && (5.0..9.0).contains(&enemy_dist) && tick % 400 == 0 {
            aim = to.normalize_or(Vec2::X) * 0.6;
            bits |= THROW;
        }
        // knives: throw them from a distance
        let knives = f.weapon.map(|wi| &s.weapons[wi as usize]).filter(|w| w.is_knives());
        if knives.is_some() {
            let chest = e.pos(w, PELVIS) * 0.5 + e.pos(w, HEAD) * 0.5;
            let to = chest - neck + Vec2::new(0.0, (chest - neck).length() * 0.04);
            if (2.5..11.0).contains(&enemy_dist) && w.raycast(neck, to, 1.0).is_none() {
                aim = to.normalize_or(Vec2::X) * 0.62;
                if tick % 35 == 0 {
                    bits |= THROW;
                }
            }
        }
        // guns: aim at the body and shoot when there's a clear line
        let gun = f.weapon.map(|wi| &s.weapons[wi as usize]).filter(|w| w.is_gun());
        if let Some(g) = gun {
            let chest = e.pos(w, PELVIS) * 0.5 + e.pos(w, HEAD) * 0.5;
            let to = chest - neck;
            aim = to.normalize_or(Vec2::X) * 0.62;
            bits &= !ATTACK;
            let clear = w.raycast(neck, to, 1.0).is_none();
            if g.ammo == 0 {
                if enemy_dist < 7.0 && tick % 60 == 0 {
                    bits |= THROW;
                }
            } else if clear && enemy_dist < 16.0 && tick % 22 < 2 {
                bits |= ATTACK;
            }
            if enemy_dist > 6.0 && clear {
                bits &= !(LEFT | RIGHT); // hold position and shoot
            }
        }
    }
    encode(bits, aim)
}
