//! Laying one net against the claims of the others as they stand.

use super::search::From;
use super::worker::{Worker, SHUT_COST, TAKEN};
use super::*;
use crate::mask::VIA_BIT;

impl Engine<'_> {
    /// Lay one net against the claims of the others as they stand: grow a tree from its first terminal to the
    /// nearest remaining terminal until all are joined. Nothing of the engine changes; the caller puts the result
    /// in place. `hard`: cells claimed by other nets are closed instead of dearer.
    pub(super) fn route_net(&self, n: usize, present: f64, hard: bool) -> Result<NetState, String> {
        let mut w = Worker::take(&self.shared, self.space.states(), self.space.hw);
        let mine: Vec<usize> = std::iter::once(n).chain(self.siblings[n].iter().copied()).collect();
        for &m in &mine {
            for &idx in self.nets[m].claims_t.get(self.tclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_t[idx as usize] += 1;
            }
            for &idx in self.nets[m].claims_v.get(self.vclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_v[idx as usize] += 1;
            }
        }
        let result = self.route_alone(n, present, hard, &mut w);
        for &m in &mine {
            for &idx in self.nets[m].claims_t.get(self.tclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_t[idx as usize] = 0;
            }
            for &idx in self.nets[m].claims_v.get(self.vclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_v[idx as usize] = 0;
            }
        }
        w.give_back(&self.shared);
        match result {
            // with the others fixed, a net is laid whole or not at all
            Ok(state) if hard && !state.failed.is_empty() => Err(state.failed[0].clone()),
            other => other,
        }
    }

    pub(super) fn route_alone(&self, n: usize, present: f64, hard: bool, w: &mut Worker) -> Result<NetState, String> {
        let p = self.p;
        let net = &p.nets[n];
        let (space, hw, nl) = (self.space, self.space.hw, self.space.nl);
        let shut = &self.shut;
        for &c in &self.fixed_free[n] {
            w.flag_hole(&self.grid, c, shut);
        }
        // Holes the net gets to change layer on for nothing that the fixed look-aheads do not know from the start:
        // vias of other entries of the same net, and its own new vias unless they become starting points below.
        let mut holes: Vec<u32> = Vec::new();
        for &m in &self.siblings[n] {
            for &v in &self.nets[m].vias {
                w.flag_hole(&self.grid, v, shut);
                holes.push(v);
            }
        }
        holes.sort_unstable();
        holes.dedup();
        let via_cost = net.via_penalty_mm / p.step;
        let own_hole = if p.options.own_hole_free { 0.0 } else { via_cost as f32 };
        {
            let _t = self.shared.timed(0);
            let give = if hard { 0.0 } else { net.yield_ }; // with the others fixed there is nothing left to give way to
            let mask = &self.shared.masks[n];
            let (occ_t, occ_v) = (&self.occ_t[self.tclass[n]], &self.occ_v[self.vclass[n]]);
            for l in 0..nl {
                let bit = 1u8 << l;
                #[allow(clippy::needless_range_loop)] // the cell indexes four tables
                for c in 0..hw {
                    let k = l * hw + c;
                    let occ = occ_t[k] - w.own_t[k];
                    w.swept[k].mult = if mask[c] & bit != 0 {
                        SHUT_COST
                    } else if hard {
                        if occ > 0 {
                            TAKEN
                        } else {
                            1.0
                        }
                    } else {
                        ((1.0 + give * self.hist[k] as f64) * (1.0 + give * present * occ as f64)) as f32
                    };
                }
            }
            for c in 0..hw {
                let occ = occ_v[c] - w.own_v[c];
                w.vcost[c] = if net.no_vias || mask[c] & VIA_BIT != 0 || (hard && occ > 0) {
                    SHUT_COST
                } else {
                    (via_cost * (1.0 + give * self.hist_v[c] as f64) * (1.0 + give * present * occ as f64)) as f32
                };
            }
            w.price_holes(own_hole, net.no_vias);
        }
        let mut state = NetState::default();
        let mut terms: Vec<Term> = Vec::new();
        for (t, at) in self.terms[n].iter().enumerate() {
            match at {
                Ok(term) => terms.push(*term),
                Err(why) => state.failed.push(format!("{}: terminal {:?} {why}", net.key, net.terminals[t].xy)),
            }
        }
        terms.extend_from_slice(&self.extra_terms[n]);
        if terms.len() < 2 {
            if state.failed.is_empty() {
                state.failed.push(format!("{}: fewer than two terminals", net.key));
            }
            return Ok(state);
        }
        let mut tree: Vec<u32> = Vec::new();
        let add_term = |tree: &mut Vec<u32>, t: Term| tree.extend(t.states(space));
        add_term(&mut tree, terms[0]);
        let mut joined: Vec<Term> = vec![terms[0]];
        let mut left: Vec<Term> = terms[1..].to_vec();
        // connections laid before the tree was last begun anew: a later one branches from the present tree only
        let mut islands = 0usize;
        let mut first_of_tree = 0usize;
        let mut begun = 0usize;
        while !left.is_empty() {
            // the sources are just terminals for the first connection, and for later ones unless the net may branch anywhere
            if islands > begun {
                begun = islands;
                first_of_tree = state.conns.len();
            }
            let from = if net.branch_anywhere && state.conns.len() > first_of_tree {
                From::Copper
            } else {
                From::Terminals(&joined[..])
            };
            let path = match self.find(n, &tree, &left, from, &holes, w) {
                Some(path) => path,
                None => {
                    // What is left cannot be joined to what has been laid. It is said so, and the rest is
                    // joined among itself as far as it goes, starting from the next terminal.
                    let at: Vec<Pt> = left.iter().map(|t| self.grid.xy(t.cell as usize)).collect();
                    state.failed.push(format!(
                        "{}: no path to {:?}{}",
                        net.key,
                        at,
                        if hard { " with the other nets fixed" } else { " past the fixed copper" }
                    ));
                    let next = left.remove(0);
                    tree.clear();
                    add_term(&mut tree, next);
                    joined = vec![next];
                    islands += 1;
                    continue;
                }
            };
            let end = *path.last().unwrap();
            let (el, ec) = space.split(end as usize);
            let k = left.iter().position(|t| t.cell as usize == ec && t.on(el)).unwrap();
            let reached = left.remove(k);
            let lc = &self.layer_cost[n];
            let mut new_vias = Vec::new();
            let mut dir = NO_DIR;
            let bend = p.options.bend_mm / p.step;
            for pair in path.windows(2) {
                let ((la, ca), (lb, cb)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
                if la != lb {
                    if w.via_flag[ca] != FREE && !new_vias.contains(&(ca as u32)) {
                        new_vias.push(ca as u32);
                        state.cost += net.via_penalty_mm / p.step;
                    } else {
                        state.cost += own_hole as f64;
                    }
                    dir = NO_DIR;
                } else {
                    let d = dir_of(self.grid.w, ca, cb);
                    state.cost += turn(dir, d).unwrap_or(0.0) * bend;
                    dir = d;
                    let ((ja, ia), (jb, ib)) = (space.rc(ca), space.rc(cb));
                    let diag = ja != jb && ia != ib;
                    let len = if diag { std::f64::consts::SQRT_2 } else { 1.0 };
                    state.cost += lc[lb] * len;
                    state.steps += len;
                }
            }
            for v in new_vias {
                w.flag_hole(&self.grid, v, shut);
                state.vias.push(v);
                if net.branch_anywhere {
                    // a via is copper of the net on every layer: a later connection may start from it on any
                    for l in 0..nl {
                        let at = space.state(l, v as usize) as u32;
                        if self.shared.masks[n][v as usize] & (1 << l) == 0 && !path.contains(&at) {
                            tree.push(at);
                        }
                    }
                } else {
                    holes.push(v);
                    holes.sort_unstable();
                }
            }
            w.price_holes(own_hole, net.no_vias);
            if net.branch_anywhere {
                tree.extend_from_slice(&path);
            }
            add_term(&mut tree, reached);
            joined.push(reached);
            state.conns.push(path);
        }
        Ok(state)
    }
}
