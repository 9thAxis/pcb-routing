//! Laying one net against the claims of the others as they stand.

use super::price::Mode;
use super::search::{From, Query};
use super::worker::Worker;
use super::*;

impl Engine<'_> {
    /// Lay one net among the others: ground they claim is dearer by `present` per claim, not closed. What
    /// could not be joined is listed in the result.
    pub(super) fn lay_among(&self, n: usize, present: f64) -> NetState {
        self.lay(n, present, false)
    }

    /// Lay one net with the others fixed: ground they claim is closed. What could not be joined is listed in
    /// the result, and what could be is in it all the same.
    pub(super) fn lay_fixed(&self, n: usize) -> NetState {
        self.lay(n, 0.0, true)
    }

    /// Lay one net with the others fixed, whole or not at all.
    pub(super) fn lay_whole(&self, n: usize) -> Result<NetState, String> {
        let state = self.lay_fixed(n);
        match state.failed.first() {
            Some(why) => Err(why.clone()),
            None => Ok(state),
        }
    }

    /// Lay one net against the claims of the others as they stand: grow a tree from its first terminal to the
    /// nearest remaining terminal until all are joined. Nothing of the engine changes; the caller puts the result
    /// in place. `hard`: cells claimed by other nets are closed instead of dearer.
    fn lay(&self, n: usize, present: f64, hard: bool) -> NetState {
        let mut w = Worker::take(self.shared, self.plan.space.states(), self.plan.space.hw);
        let mine: Vec<usize> = std::iter::once(n).chain(self.plan.siblings[n].iter().copied()).collect();
        for &m in &mine {
            for &idx in self.laid.nets[m].claims_t.get(self.plan.tclass[n]).map_or(&[][..], |v| &v[..]) {
                w.ground.own_t[idx as usize] += 1;
            }
            for &idx in self.laid.nets[m].claims_v.get(self.plan.vclass[n]).map_or(&[][..], |v| &v[..]) {
                w.ground.own_v[idx as usize] += 1;
            }
        }
        self.mark_bus(n, &mut w);
        let mut result = self.route_alone(n, present, hard, &mut w);
        result.was_apart = self.apart_of(n, &w.ground);
        w.ground.clear_bus();
        for &m in &mine {
            for &idx in self.laid.nets[m].claims_t.get(self.plan.tclass[n]).map_or(&[][..], |v| &v[..]) {
                w.ground.own_t[idx as usize] = 0;
            }
            for &idx in self.laid.nets[m].claims_v.get(self.plan.vclass[n]).map_or(&[][..], |v| &v[..]) {
                w.ground.own_v[idx as usize] = 0;
            }
        }
        w.give_back(self.shared);
        result
    }

    /// Lay net `n` (see `lay`), with the search memory `w` in which the net's own claims and its bus are marked.
    fn route_alone(&self, n: usize, present: f64, hard: bool, w: &mut Worker) -> NetState {
        let p = self.p;
        let net = &p.nets[n];
        let (space, nl) = (self.plan.space, self.plan.space.nl);
        let shut = &self.plan.shut;
        for &c in &self.plan.fixed_free[n] {
            w.ground.flag_hole(&self.plan.grid, c, shut);
        }
        // Holes the net gets to change layer on for nothing that the fixed look-aheads do not know from the start:
        // vias of other entries of the same net, and its own new vias unless they become starting points below.
        let mut holes: Vec<u32> = Vec::new();
        for &m in &self.plan.siblings[n] {
            for &v in &self.laid.nets[m].vias {
                w.ground.flag_hole(&self.plan.grid, v, shut);
                holes.push(v);
            }
        }
        holes.sort_unstable();
        holes.dedup();
        // (those vias, as they are now: `holes` goes on to take the net's own)
        let lent = holes.clone();
        // the costs of the ground are drawn up as the searches come to them (see `Prices`)
        w.ground.begin();
        let mode = Mode { present, hard };
        let own_hole = self.own_hole(n);
        let mut state = NetState::default();
        let mut terms: Vec<Term> = Vec::new();
        for (t, at) in self.plan.terms[n].iter().enumerate() {
            match at {
                Ok(term) => terms.push(*term),
                Err(why) => state.failed.push(format!("{}: terminal {:?} {why}", net.key, net.terminals[t].xy)),
            }
        }
        terms.extend_from_slice(&self.pairs.extra_terms[n]);
        if terms.len() < 2 {
            if state.failed.is_empty() {
                state.failed.push(format!("{}: fewer than two terminals", net.key));
            }
            return state;
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
            let query = Query { net: n, mode, sources: &tree, goals: &left, from: Some(from), holes: &holes };
            let path = match self.find(query, w) {
                Some(path) => path,
                None => {
                    // What is left cannot be joined to what has been laid. It is said so, with both sides (either
                    // may be the one shut in), and the rest is joined among itself as far as it goes, starting
                    // from the next terminal.
                    let places = |terms: &[Term]| -> Vec<Pt> {
                        terms.iter().map(|t| self.plan.grid.xy(t.cell as usize)).collect()
                    };
                    state.failed.push(format!(
                        "{}: no path from {:?} to any of {:?} past the fixed copper{}",
                        net.key,
                        places(&joined),
                        places(&left),
                        if hard { " and the other nets" } else { "" }
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
            let new_vias = self.account(n, &path, own_hole, w, &mut state);
            // A layer change on the via of another entry of the net is on a via of this entry too from now
            // on, at no cost: the other entry may be laid again without it.
            for pair in path.windows(2) {
                let ((la, ca), (lb, _)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
                let cell = ca as u32;
                if la != lb && lent.binary_search(&cell).is_ok() && !state.vias.contains(&cell) {
                    state.vias.push(cell);
                }
            }
            for v in new_vias {
                w.ground.flag_hole(&self.plan.grid, v, shut);
                state.vias.push(v);
                if net.branch_anywhere {
                    // a via is copper of the net on every layer: a later connection may start from it on any
                    for l in 0..nl {
                        let at = space.state(l, v as usize) as u32;
                        if self.shared.fixed.mask(n, v as usize) & (1 << l) == 0 && !path.contains(&at) {
                            tree.push(at);
                        }
                    }
                } else {
                    holes.push(v);
                    holes.sort_unstable();
                }
            }
            w.ground.price_holes(own_hole, net.no_vias);
            if net.branch_anywhere {
                tree.extend_from_slice(&path);
            }
            add_term(&mut tree, reached);
            joined.push(reached);
            state.conns.push(path);
        }
        state
    }

    /// Add what the connection `path` of net `n` costs, free of the other nets (its steps by layer, its
    /// corners, its layer changes), its length, and what of the cost is away from its bus, to `state`. Returns the cells where it needs a new via.
    fn account(&self, n: usize, path: &[u32], own_hole: f32, w: &Worker, state: &mut NetState) -> Vec<u32> {
        let (p, net, space) = (self.p, &self.p.nets[n], self.plan.space);
        let lc = &self.plan.layer_cost[n];
        let mut new_vias = Vec::new();
        let mut dir = NO_DIR;
        let bend = p.options.bend_mm / p.step;
        for pair in path.windows(2) {
            let ((la, ca), (lb, cb)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
            if la != lb {
                if w.ground.via_flag[ca] != FREE && !new_vias.contains(&(ca as u32)) {
                    new_vias.push(ca as u32);
                    state.cost += net.via_penalty_mm / p.step;
                } else {
                    state.cost += own_hole as f64;
                }
                dir = NO_DIR;
            } else {
                let d = dir_of(space.w, ca, cb);
                state.cost += turn(dir, d).unwrap_or(0.0) * bend;
                dir = d;
                let ((ja, ia), (jb, ib)) = (space.rc(ca), space.rc(cb));
                let diag = ja != jb && ia != ib;
                let len = if diag { std::f64::consts::SQRT_2 } else { 1.0 };
                state.cost += lc[lb] * len;
                state.apart += lc[lb] * len * (w.ground.apart_at(pair[1] as usize) as f64 - 1.0);
                state.steps += len;
            }
        }
        new_vias
    }
}
