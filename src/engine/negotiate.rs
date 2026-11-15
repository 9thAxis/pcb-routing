//! One attempt: rounds of negotiation, closing passes, pairs, and the nets kept for last.

use super::*;
use crate::Event;

impl Engine<'_> {
    /// Route everything. `order` is the order of the first round and of every later one.
    /// Rounds of laying the nets in conflict again until none is. Ok(true): settled.
    pub(super) fn negotiate(
        &mut self,
        order: &[usize],
        first: Vec<usize>,
        limit: usize,
        rounds: &mut usize,
        variant: usize,
    ) -> Result<bool, String> {
        let o = self.p.options.clone();
        let mut present = o.present_start;
        let mut todo: Vec<usize> = first;
        // Stalls: two nets squeezed into a gap with room for one stay in each other's way round after round,
        // shifting by a cell each time, while every other net, no longer in conflict, is no longer moved. The
        // standing cost, added only where the centre lines stood, then grows too slowly to drive one of them
        // out. So once the count of nets in conflict has not fallen for `stall_after` rounds, the standing cost
        // is added over the whole width a track claims round each place of conflict, and more each further
        // round; one of the nets then gives up the gap and pushes a third net instead, which is laid again.
        // After `patience` such rounds the attempt is given up.
        let (mut fewest, mut since) = (usize::MAX, 0usize);
        for _ in 0..limit {
            *rounds += 1;
            for &n in &todo {
                let state = self.route_net(n, present, false)?;
                self.place(n, state);
            }
            let mut next = Vec::new();
            for &n in order {
                let t = self.trouble(n);
                if t.any() {
                    next.push(n);
                }
                for &s in &t.track {
                    self.hist[s as usize] += o.history_step as f32;
                }
                if o.stall_after > 0 && since >= o.stall_after && !t.track.is_empty() {
                    let disc = self.disc(self.p.nets[n].width + self.p.rules.new_clearance_mm);
                    let (w, h, hw) = (self.grid.w as i32, self.grid.h as i32, self.hw);
                    let add = (o.history_step * (since + 1 - o.stall_after) as f64) as f32;
                    self.mark_gen += 1;
                    for &s in &t.track {
                        let (l, c) = (s as usize / hw, s as usize % hw);
                        let (j, i) = (c as i32 / w, c as i32 % w);
                        for &(dj, di) in &disc {
                            let (jj, ii) = (j + dj, i + di);
                            if jj >= 0 && ii >= 0 && jj < h && ii < w {
                                let k = l * hw + (jj * w + ii) as usize;
                                if self.mark[k] != self.mark_gen {
                                    self.mark[k] = self.mark_gen;
                                    self.hist[k] += add;
                                }
                            }
                        }
                    }
                }
                for &v in &t.via {
                    self.hist_v[v as usize] += o.history_step as f32;
                }
                for &v in &t.own {
                    self.hist_v[v as usize] += 4.0 * o.history_step as f32;
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
                return Ok(true);
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
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// One attempt at the whole problem. `order` is the order in which the nets are laid within a round.
    pub(crate) fn attempt(&mut self, order: &[usize], variant: usize) -> Solution {
        let o = self.p.options.clone();
        let fail = |message: String, iterations: usize| Solution {
            status: Status::Unroutable,
            iterations,
            variant,
            score: None,
            nets: Vec::new(),
            conflicts: Vec::new(),
            unrouted: vec![message.clone()],
            message,
            attempts: Vec::new(),
            stats: Stats::default(),
        };
        if !o.negotiate {
            for &n in order {
                match self.route_net(n, 0.0, true) {
                    Ok(state) => self.place(n, state),
                    Err(e) => self.unlaid.push(e),
                }
            }
            return self.report(Status::Routed, 1, variant, String::new());
        }
        let mut rounds = 0;
        let every = order;
        let order: Vec<usize> = every.iter().copied().filter(|&n| !self.p.nets[n].last).collect();
        let order = &order[..];
        let settled = match self.negotiate(order, order.to_vec(), o.max_iterations, &mut rounds, variant) {
            Ok(settled) => settled,
            Err(e) => return fail(e, rounds),
        };
        if !settled {
            return self.report(Status::Conflicts, rounds, variant, String::new());
        }
        for _ in 0..o.refine_passes {
            let mut better = false;
            for &n in order {
                if let Ok(state) = self.route_net(n, 0.0, true) {
                    if state.cost < self.nets[n].cost - 1e-6 {
                        let old = self.nets[n].clone();
                        self.place(n, state);
                        if self.trouble(n).any() {
                            self.place(n, old);
                        } else {
                            better = true;
                        }
                    }
                }
            }
            if !better {
                break;
            }
        }
        // Pairs: each settled trunk becomes two legs, and the stubs that join the legs to their pads are laid first
        // of the nets kept for last.
        let mut stubs: Vec<usize> = Vec::new();
        let mut fours: Vec<Vec<usize>> = Vec::new();
        for k in 0..self.p.pairs.len() {
            match self.split(k) {
                Ok(four) => {
                    stubs.extend_from_slice(&four);
                    fours.push(four);
                }
                Err(e) => return fail(e, rounds),
            }
        }
        // The four stubs of a pair are laid one after the other, each fixed for the next; which goes first can
        // decide whether all four fit, so the orders are tried in turn until one does.
        for (k, four) in fours.iter().enumerate() {
            let mut order = four.clone();
            let mut tried = 0;
            let mut last_error = String::new();
            let done = loop {
                let mut ok = true;
                for &n in &order {
                    match self.route_net(n, 0.0, true) {
                        Ok(state) => self.place(n, state),
                        Err(e) => {
                            last_error = e;
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    break true;
                }
                for &n in four {
                    self.place(n, NetState::default()); // the leg an entry holds stays with it
                }
                tried += 1;
                if tried == 24 {
                    break false;
                }
                // the next of the 24 orders
                let mut left: Vec<usize> = four.clone();
                let mut code = tried;
                order.clear();
                for size in (1..=4).rev() {
                    order.push(left.remove(code % size));
                    code /= size;
                }
            };
            if !done {
                // what was laid is given back with the failure, to show where the stubs were shut in
                let partial = self.report(
                    Status::Unroutable,
                    rounds,
                    variant,
                    format!(
                        "pair {}: its stubs cannot all be laid in any order; last: {last_error}",
                        self.p.pairs[k].key
                    ),
                );
                return partial;
            }
        }
        let rest: Vec<usize> = every.iter().copied().filter(|n| self.p.nets[*n].last && !stubs.contains(n)).collect();
        for n in rest {
            match self.route_net(n, 0.0, true) {
                Ok(state) => self.place(n, state),
                Err(e) => self.unlaid.push(format!("with the other nets settled: {e}")),
            }
        }
        self.report(Status::Routed, rounds, variant, String::new())
    }
}
