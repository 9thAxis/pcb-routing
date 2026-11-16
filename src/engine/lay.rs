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

    /// Lay one net against the claims of the others as they stand: grow a tree from its first piece (a terminal, or
    /// those that stand on one piece of copper) to the nearest remaining one until all are joined. Nothing of the
    /// engine changes; the caller puts the result in place. `hard`: cells claimed by other nets are closed instead
    /// of dearer.
    fn lay(&self, n: usize, present: f64, hard: bool) -> NetState {
        // An entry with terminals on different hands (see `Terminal::hand`) is laid once for each, and the
        // one that joins the most for the least is kept.
        let mut hands: Vec<u8> = self.p.nets[n].terminals.iter().map(|t| t.hand).filter(|&hand| hand != 0).collect();
        hands.sort_unstable();
        hands.dedup();
        if hands.len() < 2 {
            return self.lay_on(n, present, hard, None);
        }
        let worth = |state: &NetState| (state.failed.len(), state.cost + state.apart);
        let laid = hands.into_iter().map(|hand| self.lay_on(n, present, hard, Some(hand)));
        laid.min_by(|a, b| worth(a).partial_cmp(&worth(b)).unwrap_or(std::cmp::Ordering::Equal))
            .expect("there are two hands at least")
    }

    /// Lay a net at its terminals of one hand (all of them, without one).
    fn lay_on(&self, n: usize, present: f64, hard: bool, hand: Option<u8>) -> NetState {
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
        let mut result = self.route_alone(n, present, hard, hand, &mut w);
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
    fn route_alone(&self, n: usize, present: f64, hard: bool, hand: Option<u8>, w: &mut Worker) -> NetState {
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
        // The terminals that can be reached, each with its piece of copper (see `Terminal::piece`): the pieces
        // are what is joined, each at whichever of its terminals comes cheapest. A terminal that cannot be
        // reached is a failure only where its piece has no other.
        let mut terms: Vec<(Term, u64)> = Vec::new();
        let mut closed: Vec<(usize, u64, &String)> = Vec::new();
        // (of the open terminals of a piece, those whose turn it is: see `Terminal::turn`)
        // (and of the hand that is being laid: a terminal of another is as if it were not there)
        let ours = |t: usize| hand.map_or(true, |hand| net.terminals[t].hand == 0 || net.terminals[t].hand == hand);
        let open = || (0..net.terminals.len()).filter(|&t| ours(t) && self.plan.terms[n][t].is_ok());
        let first =
            |piece: u64| open().filter(|&t| self.plan.pieces[n][t] == piece).map(|t| net.terminals[t].turn).min();
        for (t, at) in self.plan.terms[n].iter().enumerate().filter(|&(t, _)| ours(t)) {
            let piece = self.plan.pieces[n][t];
            match at {
                Ok(term) if first(piece) == Some(net.terminals[t].turn) => terms.push((*term, piece)),
                Ok(_) => {}
                Err(why) => closed.push((t, piece, why)),
            }
        }
        for (t, piece, why) in closed {
            if !terms.iter().any(|&(_, other)| other == piece) {
                state.failed.push(format!("{}: terminal {:?} {why}", net.key, net.terminals[t].xy));
            }
        }
        let extra = self.pairs.extra_terms[n].iter().enumerate();
        terms.extend(extra.map(|(k, &term)| (term, super::plan::piece_of(net, net.terminals.len() + k))));
        let mut left: Vec<Term> = terms.iter().map(|t| t.0).collect();
        let mut left_piece: Vec<u64> = terms.iter().map(|t| t.1).collect();
        // (take every terminal of a piece out of what is left)
        let take = |left: &mut Vec<Term>, left_piece: &mut Vec<u64>, piece: u64| -> Vec<Term> {
            let mut taken = Vec::new();
            for k in (0..left.len()).rev() {
                if left_piece[k] == piece {
                    left_piece.remove(k);
                    taken.push(left.remove(k));
                }
            }
            taken.reverse();
            taken
        };
        let first = left_piece.first().copied();
        let mut joined: Vec<Term> = first.map_or(Vec::new(), |piece| take(&mut left, &mut left_piece, piece));
        if left.is_empty() {
            if state.failed.is_empty() {
                state.failed.push(format!("{}: fewer than two terminals", net.key));
            }
            return state;
        }
        let mut tree: Vec<u32> = Vec::new();
        let add_term = |tree: &mut Vec<u32>, t: Term| tree.extend(t.states(space));
        joined.iter().for_each(|&t| add_term(&mut tree, t));
        // connections laid before the tree was last begun anew: a later one branches from the present tree only
        let mut islands = 0usize;
        let mut first_of_tree = 0usize;
        let mut begun = 0usize;
        while !left.is_empty() {
            // the sources are just terminals for the first connection, and for later ones unless the net may branch
            // anywhere
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
                    let piece = left_piece[0];
                    joined = take(&mut left, &mut left_piece, piece);
                    tree.clear();
                    joined.iter().for_each(|&t| add_term(&mut tree, t));
                    islands += 1;
                    continue;
                }
            };
            let end = *path.last().unwrap();
            let (el, ec) = space.split(end as usize);
            let k = left.iter().position(|t| t.cell as usize == ec && t.on(el)).unwrap();
            let piece = left_piece[k];
            let reached = take(&mut left, &mut left_piece, piece);
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
            reached.iter().for_each(|&t| add_term(&mut tree, t));
            joined.extend(reached);
            state.conns.push(path);
        }
        state
    }

    /// What a step of length `len` (in grid steps) into a state costs net `n`, free of the other nets: by its
    /// layer and by the ground it is in. The search pays the same (see `Prices`).
    pub(super) fn step_cost(&self, n: usize, state: usize, len: f64) -> f64 {
        let layer = self.plan.space.split(state).0;
        self.plan.layer_cost[n][layer] * len * self.plan.dear.track(n, state) as f64
    }

    /// Add what the connection `path` of net `n` costs, free of the other nets (its steps by layer, its corners,
    /// its layer changes), its length, and what of the cost is away from its bus, to `state`. Returns the cells
    /// where it needs a new via.
    fn account(&self, n: usize, path: &[u32], own_hole: f32, w: &Worker, state: &mut NetState) -> Vec<u32> {
        let (p, net, space) = (self.p, &self.p.nets[n], self.plan.space);
        let mut new_vias = Vec::new();
        let mut dir = NO_DIR;
        let bend = p.options.bend_mm / p.step;
        for pair in path.windows(2) {
            let ((la, ca), (lb, cb)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
            if la != lb {
                if w.ground.via_flag[ca] != FREE && !new_vias.contains(&(ca as u32)) {
                    new_vias.push(ca as u32);
                    state.cost += net.via_penalty_mm / p.step + self.plan.dear.via(n, ca);
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
                let step = self.step_cost(n, pair[1] as usize, len);
                state.cost += step;
                state.apart += step * (w.ground.apart_at(pair[1] as usize) as f64 - 1.0);
                state.steps += len;
            }
        }
        new_vias
    }
}
