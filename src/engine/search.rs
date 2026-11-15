//! The path search, in two passes.

use super::lookahead::Look;
use super::worker::{Search, Worker, SHUT_COST, START, TAKEN, VIA};
use super::*;
use crate::Event;
use std::sync::atomic::Ordering;

/// What a search starts from, for the look-ahead towards it: a set of terminals, or more copper than that.
#[derive(Clone, Copy)]
pub(super) enum From<'t> {
    Terminals(&'t [Term]),
    Copper,
}

/// Queue key of a state: its estimated total cost in 64ths of a grid step. The order of states that differ by
/// less than that (0.0008 mm at a 0.05 mm step) is arbitrary but fixed; a path can be dearer than the cheapest
/// by amounts of that size.
#[inline]
fn key(total: f64) -> u64 {
    (total * 64.0) as u64
}

/// The corner costs as a table: [direction of arrival][new direction], in grid steps; and per direction of
/// arrival the new directions that are allowed, a bit each.
fn turns(bend: f64) -> ([[f64; 8]; 9], [u8; 9]) {
    let mut cost = [[0.0; 8]; 9];
    let mut allowed = [0u8; 9];
    #[allow(clippy::needless_range_loop)] // both tables are indexed by direction
    for from in 0..9 {
        for to in 0..8 {
            if let Some(t) = turn(from, to) {
                cost[from][to] = t * bend;
                allowed[from] |= 1 << to;
            }
        }
    }
    (cost, allowed)
}

impl Engine<'_> {
    /// Cheapest path from any state of `sources` to any terminal of `goals`. `from`: the terminals the sources
    /// are made of, when they are nothing else (not when the net may branch off anywhere along its copper).
    pub(super) fn find(
        &self,
        n: usize,
        sources: &[u32],
        goals: &[Term],
        from: From,
        holes: &[u32],
        worker: &mut Worker,
    ) -> Option<Vec<u32>> {
        let two = (self.p.options.bend_mm > 0.0 && self.p.options.two_pass).then_some(from);
        let found = self.search(n, sources, goals, two, holes, worker);
        if self.shared.diagnostics.verify && two.is_some() {
            let plain = self.search(n, sources, goals, None, holes, worker);
            let (a, b) = (found.as_ref().map(|f| f.1), plain.as_ref().map(|f| f.1));
            let agree = match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() <= 0.25 + 1e-6 * b,
                (None, None) => true,
                _ => false,
            };
            if !agree {
                self.shared.counters.mismatches.fetch_add(1, Ordering::Relaxed);
                self.shared.tell(|| Event::Mismatch { net: self.p.nets[n].key.clone(), two_pass: a, one_pass: b });
            }
        }
        found.map(|f| f.0)
    }

    /// The first pass of a search: the cost from every (layer, cell) to the goal at the costs of this moment,
    /// without corner costs and so without directions (a ninth of the states, and a small record each). It runs
    /// backwards from the goals, steered towards the sources by the fixed look-ahead `back`, until every state
    /// that could lie on a path up to `margin` dearer than the cheapest has its cost. The second pass then has a
    /// look-ahead that knows the other nets, and only widens by the cost of the corners.
    /// How far beyond: by the corner cost of one cheapest corner-free path (followed keeping straight where it
    /// can), since the cheapest path with corners cannot cost more than that one; at most `sweep_margin` corners.
    /// Returns None if no path exists; else the bound below which every state has its cost, and if known a
    /// cost the cheapest path with corners cannot exceed.
    pub(super) fn sweep(
        &self,
        n: usize,
        sources: &[u32],
        goals: &[Term],
        back: &Look,
        margin: f64,
        worker: &mut Worker,
    ) -> Option<(f64, f64)> {
        let _t = self.shared.timed(2);
        let (hw, nl) = (self.plan.space.hw, self.plan.space.nl);
        let lc = &self.plan.layer_cost[n];
        let open = &self.shared.open[n][..];
        let vcost = &worker.vcost;
        let Steps { off, len } = self.plan.steps;
        let bend = self.p.options.bend_mm / self.p.step;
        let mut upper = f64::INFINITY;
        worker.sweep_gen = worker.sweep_gen.wrapping_add(2);
        if worker.sweep_gen < 2 {
            worker.swept.iter_mut().for_each(|v| v.seen = 0);
            worker.sweep_gen = 2;
        }
        let (open_mark, closed) = (worker.sweep_gen, worker.sweep_gen + 1);
        let (a, q) = (&mut worker.swept, &mut worker.sweep_queue);
        q.clear();
        for at in goals.iter().flat_map(|g| g.states(self.plan.space)) {
            let at = at as usize;
            if back.get(at).is_finite() {
                a[at].d = 0.0;
                a[at].seen = open_mark;
                q.push((back.get(at) as f64 * 64.0) as u64, at as u32);
            }
        }
        let (mut limit, mut reached, mut last, mut count) = (f64::INFINITY, false, f64::INFINITY, 0u64);
        while let Some((_, u)) = q.pop() {
            let ui = u as usize;
            if a[ui].seen == closed {
                continue;
            }
            let d = a[ui].d as f64;
            if d + back.get(ui) as f64 > limit {
                last = d + back.get(ui) as f64;
                break;
            }
            a[ui].seen = closed;
            count += 1;
            let (l, c) = (ui / hw, ui % hw);
            // the look-ahead is 0 at the sources (and where a hole of the net joins them to another layer)
            if !reached && back.get(ui) == 0.0 && (back.exact() || sources.binary_search(&(ui as u32)).is_ok()) {
                reached = true;
                // Follow one cheapest path from here to the goal, keeping straight where that is as cheap, and
                // count its corners.
                let (mut at, mut dir, mut units, mut steps, mut real) = (ui, NO_DIR, 0.0f64, 0usize, 0.0f64);
                while a[at].d > 0.0 && units.is_finite() {
                    let here = a[at].d as f64;
                    let near = 1e-4 * here + 1e-3;
                    let (la, ca) = (at / hw, at % hw);
                    // (corners, next state, direction, what the step really costs)
                    let mut best: Option<(f64, usize, usize, f64)> = None;
                    let mut bits = open[at];
                    while bits != 0 {
                        let d2 = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        let v = (at as isize + off[d2]) as usize;
                        let step = len[d2] * lc[la] * a[v].mult as f64;
                        if a[v].seen == closed && a[v].mult < TAKEN && (a[v].d as f64 + step - here).abs() <= near {
                            if let Some(t) = turn(dir, d2) {
                                if best.map_or(true, |b| t < b.0) {
                                    best = Some((t, v, d2, step));
                                }
                            }
                        }
                    }
                    if best.is_none() && vcost[ca] != SHUT_COST && vcost[ca] > 0.0 {
                        for ll in 0..nl {
                            let v = ll * hw + ca;
                            if ll != la
                                && a[v].seen == closed
                                && a[v].mult < TAKEN
                                && (a[v].d as f64 + vcost[ca] as f64 - here).abs() <= near
                            {
                                best = Some((0.0, v, NO_DIR, vcost[ca] as f64));
                            }
                        }
                    }
                    steps += 1;
                    match best {
                        Some((t, v, d2, step)) if steps < 1_000_000 => {
                            units += t;
                            real += step;
                            at = v;
                            dir = d2;
                        }
                        _ => units = f64::INFINITY, // not followed to the end: nothing is known
                    }
                }
                // The path followed is a real one, so what it costs with its corners is a cost the cheapest path
                // cannot exceed. Its steps are summed as taken, not read off `d`: a step is accepted within a
                // tolerance (the costs are kept in single precision), and over a long line those tolerances add
                // up to more than the difference between this path and a cheaper-looking one that does not exist.
                let base = if units.is_finite() { real.max(d) } else { d };
                if units.is_finite() {
                    upper = base + units * bend;
                }
                limit = base + (units + 0.5).min(margin) * bend;
            }
            let m = a[ui].mult;
            if m >= TAKEN {
                continue; // no step can end here, so none is taken back from here
            }
            let mut relax = |v: usize, nd: f64| {
                let node = &mut a[v];
                if node.seen == closed || (node.seen == open_mark && nd >= node.d as f64) {
                    return;
                }
                // nothing beyond the bound is wanted (this also drops states the sources cannot reach at all)
                let total = nd + back.get(v) as f64;
                if total > limit || total == f64::INFINITY {
                    return;
                }
                node.d = nd as f32;
                node.seen = open_mark;
                q.push((total * 64.0) as u64, v as u32);
            };
            let step = lc[l] * m as f64;
            let mut bits = open[ui];
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                relax((ui as isize + off[d2]) as usize, d + len[d2] * step);
            }
            let cost = vcost[c];
            if cost != SHUT_COST {
                for ll in 0..nl {
                    if ll != l {
                        relax(ll * hw + c, d + cost as f64);
                    }
                }
            }
        }
        self.shared.counters.swept.fetch_add(count, Ordering::Relaxed);
        if reached {
            Some((last.min(limit), upper))
        } else {
            None
        }
    }

    pub(super) fn search(
        &self,
        n: usize,
        sources: &[u32],
        goals: &[Term],
        from: Option<From>,
        holes: &[u32],
        worker: &mut Worker,
    ) -> Option<(Vec<u32>, f64)> {
        let floor = self.look(n, goals, holes, sources);
        let floor = &floor;

        // With a first pass: the look-ahead is its cost where it got to, and elsewhere the most that is certain
        // (the fixed look-ahead, or that the state lies beyond the bound the first pass stopped at).
        let back: Option<Look> = from.map(|from| match from {
            From::Terminals(from) => self.look(n, from, holes, &self.states_of(goals)),
            From::Copper if self.p.options.exact_after == 0 => {
                let _t = self.shared.timed(1);
                Look::Exact(self.floor_to_copper(n, sources, holes, floor, &self.states_of(goals)))
            }
            From::Copper => self.look_around(n, sources),
        });
        let (bound, upper) = match &back {
            Some(back) => {
                let margin = self.p.options.sweep_margin;
                let mut sorted = sources.to_vec();
                sorted.sort_unstable();
                let (bound, upper) = self.sweep(n, &sorted, goals, back, margin, worker)?;
                (Some(bound), upper)
            }
            None => (None, f64::INFINITY),
        };
        let _t = self.shared.timed(3);
        let back = back.as_ref();
        let swept = &worker.swept[..];
        let swept_closed = worker.sweep_gen + 1;
        let look = |at: usize| -> f64 {
            match (bound, back) {
                (Some(bound), Some(back)) => {
                    if swept[at].seen == swept_closed {
                        // kept in single precision: a little under, so that rounding can never put it above the real cost
                        swept[at].d as f64 * 0.9999
                    } else {
                        (floor.get(at) as f64).max(bound - back.get(at) as f64)
                    }
                }
                _ => floor.get(at) as f64,
            }
        };
        let (hw, nl) = (self.plan.space.hw, self.plan.space.nl);
        let lc = &self.plan.layer_cost[n];
        let bend = self.p.options.bend_mm / self.p.step;
        let vcost = &worker.vcost;
        let mut count = 0u64;
        let s = &mut worker.search;
        s.gen = s.gen.wrapping_add(2);
        if s.gen < 2 {
            s.node.iter_mut().for_each(|v| v.seen = 0);
            s.gen = 2;
        }
        let (open_mark, closed) = (s.gen, s.gen + 1);
        s.heap.clear();
        // no state is wanted that cannot be on a path as cheap as one already known (with room for rounding)
        let cut = upper * 1.0005 + 1.0;
        let open = &self.shared.open[n][..];
        let (corner, allowed) = turns(bend);
        let Steps { off, len } = self.plan.steps;
        // A state is ((layer, cell), direction of arrival), the nine directions of a cell side by side in memory.
        for &src in sources {
            let ahead = look(src as usize);
            let src = src * 9 + NO_DIR as u32;
            if s.node[src as usize].seen != open_mark {
                s.node[src as usize].seen = open_mark;
                s.node[src as usize].g = 0.0;
                s.node[src as usize].prev = START;
                if ahead.is_finite() {
                    s.heap.push(key(ahead), src);
                }
            }
        }
        let mut found = NONE;
        while let Some((_, u)) = s.heap.pop() {
            let ui = u as usize;
            if s.node[ui].seen == closed {
                continue;
            }
            s.node[ui].seen = closed;
            count += 1;
            let (dir, at) = (ui % 9, ui / 9);
            let ahead_here = look(at);
            let (l, c) = (at / hw, at % hw);
            // the look-ahead is 0 at a goal (and at little else), so this test is rarely made in full
            if ahead_here == 0.0 && goals.iter().any(|g| g.cell as usize == c && g.on(l)) {
                found = u;
                break;
            }
            let gu = s.node[ui].g as f64;
            let step_cost = lc[l];
            let relax = |v: usize, cost: f64, how: u8, s: &mut Search| {
                let nd = gu + cost;
                let node = &mut s.node[v];
                // a closed state is opened again if a cheaper way to it turns up: that can happen only where the
                // look-ahead changes from the first pass's cost to the bound beyond it
                if if node.seen == closed {
                    nd * 1.000001 + 1e-6 < node.g as f64
                } else {
                    node.seen != open_mark || nd < node.g as f64
                } {
                    let ahead = look(v / 9);
                    if ahead.is_finite() && nd + ahead <= cut {
                        node.seen = open_mark;
                        node.g = nd as f32;
                        node.prev = how;
                        s.heap.push(key(nd + ahead), v as u32);
                    }
                }
            };
            let mut bits = open[at] & allowed[dir];
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let v = (at as isize + off[d2]) as usize;
                let m2 = swept[v].mult;
                if m2 >= TAKEN {
                    continue;
                }
                relax(v * 9 + d2, corner[dir][d2] + len[d2] * step_cost * m2 as f64, dir as u8, s);
            }
            let cost = vcost[c];
            if cost == SHUT_COST {
                continue;
            }
            for ll in 0..nl {
                if ll != l && swept[ll * hw + c].mult < TAKEN {
                    relax((ll * hw + c) * 9 + NO_DIR, cost as f64, VIA + (9 * l + dir) as u8, s);
                }
            }
        }
        self.shared.counters.expanded.fetch_add(count, Ordering::Relaxed);
        if found == NONE {
            return None;
        }
        let mut path = vec![found];
        loop {
            let ui = *path.last().unwrap() as usize;
            let (how, dir, at) = (s.node[ui].prev, ui % 9, ui / 9);
            if how == START {
                break;
            }
            path.push(if dir != NO_DIR {
                ((at as isize - off[dir]) as usize * 9) as u32 + how as u32
            } else {
                let (from, how) = (((how - VIA) / 9) as usize, (how - VIA) % 9);
                ((from * hw + at % hw) * 9) as u32 + how as u32
            });
        }
        path.reverse();
        // the direction is only needed during the search
        let cost = s.node[found as usize].g as f64;
        Some((path.into_iter().map(|u| u / 9).collect(), cost))
    }
}
