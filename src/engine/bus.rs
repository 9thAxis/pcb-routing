//! Buses: the nets of one bus are drawn to run beside each other, each in its lane, and kept from crossing.
//!
//! A net of a bus pays `Options::bus_pull` more per unit length wherever it is not beside a laid net of its bus
//! on the same layer, and `Options::bus_cross` more where it runs over or under one on another layer. Away
//! from the bus it pays more, it never pays less beside it: so every look-ahead of the search stays a lower
//! bound. What a path pays in this way is kept apart from its cost (`NetState::apart`), since it changes as
//! the other nets of the bus move.

use super::worker::{Ground, Worker, BESIDE, NEAR, OVER};
use super::*;

/// Nets of a bus more lanes apart than this are not drawn to each other: so far off, where one runs says
/// little of where the other should, and the ground to mark grows with the square of the distance. (They
/// still pay for passing over each other.)
const FAR_LANES: i64 = 3;

impl Engine<'_> {
    /// For a net of a bus: mark in `w` the ground beside the laid nets of its bus within a few lanes of its
    /// own, each on its layer, and the ground over and under all of them on the other layers; and set what
    /// the net pays more there and away from them. Nothing for a net of no bus, or with none of its bus laid.
    ///
    /// Beside a net `k` lanes away is from `k - 1/2` to `k + 1` times the least distance between the two
    /// tracks' middles from its path: nearer, the nets between them would find no room (for a neighbour:
    /// from its path on). Over or under it is within that distance of its path, on another layer. Beside one
    /// net of the bus counts before too near or over another, whatever order they are taken in.
    pub(super) fn mark_bus(&self, n: usize, w: &mut Worker) {
        let (p, plan) = (self.p, self.plan);
        let pull = p.options.bus_pull;
        if pull <= 0.0 {
            return;
        }
        let (grid, space, rules) = (&plan.grid, plan.space, &p.rules);
        let quiet = if rules.quiet_clearance_mm > 0.0 { rules.quiet_clearance_mm } else { rules.new_clearance_mm };
        // per mate that is laid: how many lanes off, and the least distance between the middles of the two tracks
        let mates: Vec<(usize, i64, f64)> = plan.mates[n]
            .iter()
            .filter(|&&m| !self.laid.nets[m].conns.is_empty())
            .map(|&m| {
                let lanes = (p.nets[n].bus_lane as i64 - p.nets[m].bus_lane as i64).abs().max(1);
                let gap = if p.nets[m].quiet != p.nets[n].quiet { quiet } else { rules.new_clearance_mm };
                (m, lanes, p.nets[n].width / 2.0 + gap + p.nets[m].width / 2.0)
            })
            .collect();
        if mates.is_empty() {
            return;
        }
        let _t = self.shared.timed(9);
        let ground = &mut w.ground;
        ground.apart = pull as f32;
        ground.over = p.options.bus_cross.max(pull) as f32;
        ground.beside.resize(space.states(), 0);
        // The cells of a mate's path that discs are drawn round: every few, which leaves a band nearly as wide
        // between them, and the ends of every stretch on a layer whatever its length.
        let drawn = |m: usize, pitch: f64| -> Vec<(usize, usize)> {
            let stride = ((pitch / grid.step * 2.0 / 3.0) as usize).max(1);
            let mut out = Vec::new();
            for conn in &self.laid.nets[m].conns {
                let layer = |k: usize| space.split(conn[k] as usize).0;
                for (k, &s) in conn.iter().enumerate() {
                    let end = k == 0 || k + 1 == conn.len() || layer(k - 1) != layer(k) || layer(k + 1) != layer(k);
                    if k % stride == 0 || end {
                        out.push(space.split(s as usize));
                    }
                }
            }
            out
        };
        for &(m, lanes, pitch) in mates.iter().filter(|mate| mate.1 <= FAR_LANES) {
            let lanes = lanes as f64;
            let (near, far) = (if lanes > 1.0 { (lanes - 0.5) * pitch } else { 0.0 }, (lanes + 1.0) * pitch);
            let (inner, outer) = (grid.disc(near), grid.disc(far));
            let cells = drawn(m, pitch);
            // too near this mate first (unless beside another already), then beside it, then the first undone
            let mut too_near: Vec<usize> = Vec::new();
            if near > 0.0 {
                for &(layer, cell) in &cells {
                    for c in grid.around(cell, &inner) {
                        let idx = space.state(layer, c);
                        if ground.beside[idx] == 0 {
                            ground.beside[idx] = NEAR;
                            too_near.push(idx);
                        }
                    }
                }
            }
            for &(layer, cell) in &cells {
                for c in grid.around(cell, &outer) {
                    let idx = space.state(layer, c);
                    if ground.beside[idx] == 0 {
                        ground.beside[idx] = BESIDE;
                        ground.beside_set.push(idx as u32);
                    }
                }
            }
            for idx in too_near {
                ground.beside[idx] = 0;
            }
        }
        // over or under each mate on the other layers, where the net is beside none
        if ground.over > ground.apart {
            for &(m, _, pitch) in &mates {
                let under = grid.disc(pitch);
                for (layer, cell) in drawn(m, pitch) {
                    for other in (0..space.nl).filter(|&l| l != layer) {
                        for c in grid.around(cell, &under) {
                            let idx = space.state(other, c);
                            if ground.beside[idx] == 0 {
                                ground.beside[idx] = OVER;
                                ground.beside_set.push(idx as u32);
                            }
                        }
                    }
                }
            }
        }
    }

    /// What the path of net `n` as it lies pays more for where it is by its bus, by the marks in `ground`
    /// (see `mark_bus`): the same as `account` adds up for a path being laid.
    pub(super) fn apart_of(&self, n: usize, ground: &Ground) -> f64 {
        if ground.apart <= 0.0 {
            return 0.0;
        }
        let space = self.plan.space;
        let lc = &self.plan.layer_cost[n];
        let mut apart = 0.0;
        for pair in self.laid.nets[n].conns.iter().flat_map(|conn| conn.windows(2)) {
            let ((la, ca), (lb, cb)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
            if la == lb {
                let ((ja, ia), (jb, ib)) = (space.rc(ca), space.rc(cb));
                let len = if ja != jb && ia != ib { std::f64::consts::SQRT_2 } else { 1.0 };
                apart += lc[lb] * len * (ground.apart_at(pair[1] as usize) as f64 - 1.0);
            }
        }
        apart
    }
}
