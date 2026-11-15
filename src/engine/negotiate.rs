//! One attempt: rounds of negotiation, closing passes, pairs, and the nets kept for last.

use super::*;
use crate::Event;

impl Engine<'_> {
    /// One attempt at the whole problem. `order` is the order in which the nets are laid within a round.
    ///
    /// Returns the solution and what attempts that end alike are ranked by.
    pub(crate) fn attempt(&mut self, order: &[usize], variant: usize) -> (Solution, f64) {
        let p = self.p;
        if !p.options.negotiate {
            return self.lay_in_turn(order, variant);
        }
        // the nets kept for last take no part until the others have settled
        let first: Vec<usize> = order.iter().copied().filter(|&n| !p.nets[n].last).collect();
        let mut rounds = 0;
        if !self.settle(&first, &mut rounds, variant) {
            if !p.options.drop_conflicts {
                return self.report(Status::Conflicts, rounds, variant);
            }
            self.drop_conflicts(&first);
        }
        self.refine(&first);
        let stubs = self.lay_pairs();
        self.lay_last(order, &stubs);
        self.report(Status::Routed, rounds, variant)
    }

    /// Without negotiation: each net once, in the given order, with those before it fixed.
    fn lay_in_turn(&mut self, order: &[usize], variant: usize) -> (Solution, f64) {
        for &n in order {
            let state = self.lay_fixed(n);
            self.place(n, state);
        }
        self.report(Status::Routed, 1, variant)
    }

    /// Rounds of laying the nets in conflict again until none is; true if they settled. `order` is the order of
    /// the first round and of every later one; `rounds` is counted up.
    fn settle(&mut self, order: &[usize], rounds: &mut usize, variant: usize) -> bool {
        let o = &self.p.options;
        let mut present = o.present_start;
        let mut todo: Vec<usize> = order.to_vec();
        // the fewest nets in conflict after any round so far, and the rounds since that count last fell
        let (mut fewest, mut since) = (usize::MAX, 0usize);
        for _ in 0..o.max_iterations {
            *rounds += 1;
            for &n in &todo {
                let state = self.lay_among(n, present);
                self.place(n, state);
            }
            let mut next = Vec::new();
            for &n in order {
                let t = self.trouble(n);
                if t.any() {
                    next.push(n);
                }
                for &s in &t.track {
                    self.history.hist[s as usize] += o.history_step as f32;
                }
                if o.stall_after > 0 && since >= o.stall_after && !t.track.is_empty() {
                    self.spread_standing_cost(
                        n,
                        &t.track,
                        (o.history_step * (since + 1 - o.stall_after) as f64) as f32,
                    );
                }
                for &v in &t.via {
                    self.history.hist_v[v as usize] += o.history_step as f32;
                }
                for &v in &t.own {
                    self.history.hist_v[v as usize] += 4.0 * o.history_step as f32;
                }
            }
            self.shared.tell(|| Event::Round {
                attempt: variant,
                round: *rounds,
                relaid: todo.len(),
                in_conflict: next.len(),
                present,
            });
            if next.is_empty() {
                return true;
            }
            if next.len() < fewest {
                fewest = next.len();
                since = 0;
            } else {
                since += 1;
            }
            todo = next;
            present = (present * o.present_growth).min(o.present_max);
            if o.patience > 0 && since >= o.patience {
                return false;
            }
        }
        false
    }

    /// Stalls: two nets squeezed into a gap with room for one stay in each other's way round after round,
    /// shifting by a cell each time, while every other net, no longer in conflict, is no longer moved. The
    /// standing cost, added only where the centre lines stood, then grows too slowly to drive one of them
    /// out. So once the count of nets in conflict has not fallen for `stall_after` rounds, the standing cost
    /// is added over the whole width a track claims round each place of conflict (here: `add`, round the
    /// states `track` of net `n`), and more each further round; one of the nets then gives up the gap and
    /// pushes a third net instead, which is laid again. After `patience` such rounds the attempt is given up.
    fn spread_standing_cost(&mut self, n: usize, track: &[u32], add: f32) {
        let (grid, space) = (&self.plan.grid, self.plan.space);
        let disc = self.claims.disc(grid, self.p.nets[n].width + self.p.rules.new_clearance_mm);
        self.claims.mark_gen += 1;
        for &s in track {
            let (l, c) = space.split(s as usize);
            for cell in grid.around(c, &disc) {
                let k = space.state(l, cell);
                if self.claims.mark[k] != self.claims.mark_gen {
                    self.claims.mark[k] = self.claims.mark_gen;
                    self.history.hist[k] += add;
                }
            }
        }
    }

    /// The rounds ended with nets still too close. So that what is returned can be used, the net that stands
    /// in the most claimed ground is taken out, and so on until none is in conflict; each is then tried once
    /// more with the others fixed; what still does not fit is listed, and what of it does fit is kept.
    fn drop_conflicts(&mut self, order: &[usize]) {
        let mut out: Vec<usize> = Vec::new();
        loop {
            let worst = order
                .iter()
                .map(|&n| {
                    let t = self.trouble(n);
                    (t.track.len() + t.via.len() + t.own.len(), n)
                })
                .filter(|t| t.0 > 0)
                .max_by_key(|t| (t.0, std::cmp::Reverse(t.1)));
            let Some((_, n)) = worst else { break };
            self.place(n, NetState::default());
            out.push(n);
        }
        for n in out {
            let state = self.lay_fixed(n);
            self.place(n, state);
        }
    }

    /// Closing passes: each net is laid once more with all the others fixed, and the new path is kept only if
    /// it joins more of the net, or as much for less, and stands in nobody's ground. Repeated until nothing
    /// improves; a net that found no room before is so tried again once the others have moved.
    fn refine(&mut self, order: &[usize]) {
        for _ in 0..self.p.options.refine_passes {
            let mut better = false;
            for &n in order {
                let state = self.lay_fixed(n);
                let now = &self.laid.nets[n];
                let (missing, was_missing) = (state.failed.len(), now.failed.len());
                if missing < was_missing || (missing == was_missing && state.cost < now.cost - 1e-6) {
                    let old = now.clone();
                    self.place(n, state);
                    if self.trouble(n).any() {
                        self.place(n, old);
                    } else {
                        better = true;
                    }
                }
            }
            if !better {
                break;
            }
        }
    }

    /// Pairs: each settled trunk becomes two legs, and the stubs that join the legs to their pads are laid
    /// first of the nets kept for last. A pair whose trunk cannot be split is taken out and said so; the
    /// others go on. Returns the stub entries of all pairs.
    fn lay_pairs(&mut self) -> Vec<usize> {
        let mut stubs: Vec<usize> = Vec::new();
        let mut fours: Vec<Vec<usize>> = Vec::new();
        for k in 0..self.p.pairs.len() {
            stubs.extend(self.stubs_of(k));
            match self.split(k) {
                Ok(four) => fours.push(four),
                Err(why) => {
                    self.take_out(k);
                    self.laid.unlaid.push(why);
                }
            }
        }
        for four in &fours {
            self.lay_stubs(four);
        }
        stubs
    }

    /// The nets marked `last` (of `order`, and but for the stubs, which are laid already): each once, with
    /// everything else fixed.
    fn lay_last(&mut self, order: &[usize], stubs: &[usize]) {
        let rest: Vec<usize> = order.iter().copied().filter(|n| self.p.nets[*n].last && !stubs.contains(n)).collect();
        for n in rest {
            let state = self.lay_fixed(n);
            self.place(n, state);
        }
    }
}
