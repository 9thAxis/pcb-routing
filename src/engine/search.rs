//! The path search, in two passes.

use super::lookahead::Look;
use super::plan::Head;
use super::price::{Mode, Prices};
use super::worker::{Search, Sweep, Worker, SHUT_COST, START, TAKEN, VIA};
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

/// How much dearer than the way it would go if nothing held it a line may be laid: so many times, and the
/// cost of so many mm besides.
const LINE_ROUND: f64 = 2.0;
const LINE_SPARE_MM: f64 = 10.0;

/// One search: the cheapest path of a net from any of some states to any of some terminals.
#[derive(Clone, Copy)]
pub(super) struct Query<'q> {
    pub(super) net: usize,
    /// How the net is being laid: what its ground costs it.
    pub(super) mode: Mode,
    /// The states to start from.
    pub(super) sources: &'q [u32],
    pub(super) goals: &'q [Term],
    /// What the sources are made of, for the first pass to look ahead towards them; None for a search in one
    /// pass.
    pub(super) from: Option<From<'q>>,
    /// Cells where the net has a hole to change layer on for nothing, beyond those it has from the start.
    pub(super) holes: &'q [u32],
    /// The hand the net is being laid on (see `Terminal::hand`), if it has more than one.
    pub(super) hand: Option<u8>,
    /// What the path may cost at the most, if there is a bound: none is found that costs more.
    pub(super) cut: Option<f64>,
    /// States the path may not enter, in order: for a line, the ground of what is laid of it (it cannot
    /// cross itself).
    pub(super) shut: &'q [u32],
}

impl Engine<'_> {
    /// Cheapest path from any state of `q.sources` to any terminal of `q.goals`, in two passes if the options
    /// say so. `q.from`: the terminals the sources are made of, when they are nothing else (not when the net
    /// may branch off anywhere along its copper).
    pub(super) fn find(&self, q: Query, worker: &mut Worker) -> Option<Vec<u32>> {
        let n = q.net;
        let two = if self.p.options.bend_mm > 0.0 && self.p.options.two_pass { q.from } else { None };
        let found = self.search(Query { from: two, ..q }, worker);
        if self.shared.diagnostics.verify && two.is_some() {
            let plain = self.search(Query { from: None, ..q }, worker);
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
    /// `q.sources` must be sorted; `q.from` is not looked at.
    fn sweep(&self, q: Query, back: &Look, margin: f64, sweep: &mut Sweep, prices: &Prices) -> Option<(f64, f64)> {
        let _t = self.shared.timed(2);
        let Query { net: n, sources, goals, .. } = q;
        let (hw, nl) = (self.plan.space.hw, self.plan.space.nl);
        let lc = &self.plan.layer_cost[n];
        let Steps { off, len } = self.plan.steps;
        let bend = self.p.options.bend_mm / self.p.step;
        let mut upper = f64::INFINITY;
        let a = prices.swept();
        sweep.gen = sweep.gen.wrapping_add(2);
        if sweep.gen < 2 {
            a.iter().for_each(|v| v.seen.set(0));
            sweep.gen = 2;
        }
        let (open_mark, closed) = (sweep.gen, sweep.gen + 1);
        let q = &mut sweep.queue;
        q.clear();
        for at in goals.iter().flat_map(|g| g.states(self.plan.space)) {
            let at = at as usize;
            let ahead = back.get(at);
            if ahead.is_finite() {
                a[at].d.set(0.0);
                a[at].seen.set(open_mark);
                a[at].ahead.set(ahead);
                q.push((ahead as f64 * 64.0) as u64, at as u32);
            }
        }
        let (mut limit, mut reached, mut last, mut count) = (f64::INFINITY, false, f64::INFINITY, 0u64);
        while let Some((_, u)) = q.pop() {
            let ui = u as usize;
            if a[ui].seen.get() == closed {
                continue;
            }
            let (d, ahead) = (a[ui].d.get() as f64, a[ui].ahead.get());
            if d + ahead as f64 > limit {
                last = d + ahead as f64;
                break;
            }
            a[ui].seen.set(closed);
            count += 1;
            let (l, c) = (ui / hw, ui % hw);
            // the look-ahead is 0 at the sources (and where a hole of the net joins them to another layer)
            if !reached && ahead == 0.0 && (back.exact() || sources.binary_search(&(ui as u32)).is_ok()) {
                reached = true;
                let (units, real) = self.follow(n, prices, closed, ui);
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
            let here = prices.at(ui);
            let m = here.mult.get();
            if m >= TAKEN {
                continue; // no step can end here, so none is taken back from here
            }
            let mut relax = |v: usize, nd: f64| {
                let node = &a[v];
                let seen = node.seen.get();
                if seen == closed || (seen == open_mark && nd >= node.d.get() as f64) {
                    return;
                }
                let ahead = if seen == open_mark { node.ahead.get() } else { back.get(v) };
                // nothing beyond the bound is wanted (this also drops states the sources cannot reach at all)
                let total = nd + ahead as f64;
                if total > limit || total == f64::INFINITY {
                    return;
                }
                node.d.set(nd as f32);
                node.seen.set(open_mark);
                node.ahead.set(ahead);
                q.push((total * 64.0) as u64, v as u32);
            };
            let step = lc[l] * m as f64;
            let mut bits = here.open.get();
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                relax((ui as isize + off[d2]) as usize, d + len[d2] * step);
            }
            let cost = prices.via(c);
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

    /// Follow one cheapest path of the first pass from the state `from` to the goal, keeping straight where
    /// that is as cheap. `a`: what the first pass has found, with `closed` the mark of the states whose cost
    /// is final. Returns its corners (in units of the 45 degree corner cost; infinite if the path could not be
    /// followed to the end, and then nothing is known) and what its steps really cost.
    fn follow(&self, n: usize, prices: &Prices, closed: u32, from: usize) -> (f64, f64) {
        let (hw, nl) = (self.plan.space.hw, self.plan.space.nl);
        let lc = &self.plan.layer_cost[n];
        let Steps { off, len } = self.plan.steps;
        let a = prices.swept();
        // a state the first pass has finished with that a step can end in: its factor, else None
        let done = |v: usize| -> Option<f64> {
            if a[v].seen.get() != closed {
                return None;
            }
            let mult = prices.at(v).mult.get();
            (mult < TAKEN).then_some(mult as f64)
        };
        let (mut at, mut dir, mut units, mut steps, mut real) = (from, NO_DIR, 0.0f64, 0usize, 0.0f64);
        while a[at].d.get() > 0.0 && units.is_finite() {
            let here = a[at].d.get() as f64;
            let near = 1e-4 * here + 1e-3;
            let (la, ca) = (at / hw, at % hw);
            // (corners, next state, direction, what the step really costs)
            let mut best: Option<(f64, usize, usize, f64)> = None;
            let mut bits = prices.at(at).open.get();
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let v = (at as isize + off[d2]) as usize;
                let Some(mult) = done(v) else { continue };
                let step = len[d2] * lc[la] * mult;
                if (a[v].d.get() as f64 + step - here).abs() <= near {
                    if let Some(t) = turn(dir, d2) {
                        if best.map_or(true, |b| t < b.0) {
                            best = Some((t, v, d2, step));
                        }
                    }
                }
            }
            let change = prices.via(ca);
            if best.is_none() && change != SHUT_COST && change > 0.0 {
                for ll in 0..nl {
                    let v = ll * hw + ca;
                    if ll != la && done(v).is_some() && (a[v].d.get() as f64 + change as f64 - here).abs() <= near {
                        best = Some((0.0, v, NO_DIR, change as f64));
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
        (units, real)
    }

    /// The search proper: the cheapest path with its corners and what it costs, after a first pass if
    /// `q.from` is given.
    fn search(&self, q: Query, worker: &mut Worker) -> Option<(Vec<u32>, f64)> {
        let Query { net: n, mode, sources, goals, from, holes, hand, cut: most, shut } = q;
        let Worker { search: s, sweep, ground } = worker;
        let prices = Prices::new(self, ground, n, mode);
        let floor = self.look(n, goals, holes, sources);
        let floor = &floor;

        // With a first pass: the look-ahead is its cost where it got to, and elsewhere the most that is certain
        // (the fixed look-ahead, or that the state lies beyond the bound the first pass stopped at).
        let back: Option<Look> = from.map(|from| match from {
            From::Terminals(from) => self.look(n, from, holes, &self.states_of(goals)),
            From::Copper if self.p.options.exact_after == Some(0) => {
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
                let (bound, upper) = self.sweep(Query { sources: &sorted, ..q }, back, margin, sweep, &prices)?;
                (Some(bound), upper)
            }
            None => (None, f64::INFINITY),
        };
        let _t = self.shared.timed(3);
        let back = back.as_ref();
        let swept = prices.swept();
        let (swept_open, swept_closed) = (sweep.gen, sweep.gen + 1);
        let look = |at: usize| -> f64 {
            match (bound, back) {
                (Some(bound), Some(back)) => {
                    if swept[at].seen.get() == swept_closed {
                        // kept in single precision: a little under, so that rounding can never put it above the real
                        // cost
                        swept[at].d.get() as f64 * 0.9999
                    } else {
                        // (a state the first pass reached but did not finish has its look-ahead kept)
                        let behind =
                            if swept[at].seen.get() == swept_open { swept[at].ahead.get() } else { back.get(at) };
                        (floor.get(at) as f64).max(bound - behind as f64)
                    }
                }
                _ => floor.get(at) as f64,
            }
        };
        let (hw, nl) = (self.plan.space.hw, self.plan.space.nl);
        let lc = &self.plan.layer_cost[n];
        let bend = self.p.options.bend_mm / self.p.step;
        let mut count = 0u64;
        s.gen = s.gen.wrapping_add(2);
        if s.gen < 2 {
            s.node.iter_mut().for_each(|v| v.seen = 0);
            s.gen = 2;
        }
        let (open_mark, closed) = (s.gen, s.gen + 1);
        s.heap.clear();
        // No state is wanted that cannot be on a path as cheap as one already known (with room for rounding).
        // Not so for a net with terminals that are left one way only, or the trunk of a pair, whose layer
        // changes are whole runs: the first pass knows neither, so the path it followed may be one the net
        // cannot take, and cheaper than any it can. What the first pass found is a look-ahead all the same:
        // what holds a path to a heading or a run only ever makes it dearer.
        let held = !self.plan.heads[n].is_empty() || self.plan.runs[n].is_some();
        // A line (see `Terminal::pass`) is not followed further round than twice the way it would go if nothing
        // held it, and 10 mm: so far round it is no pair worth laying, and the search for it would be most of a run.
        let line = self.plan.heads[n].iter().any(|head| head.pass);
        let cut = if line {
            upper * LINE_ROUND + LINE_SPARE_MM / self.p.step
        } else if held {
            f64::INFINITY
        } else {
            upper * 1.0005 + 1.0
        };
        let cut = cut.min(most.unwrap_or(f64::INFINITY));
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
        // (the terminal in this cell, if it is left one way only: of the hand being laid, where two hands
        // have one in the same cell)
        let heads = &self.plan.heads[n];
        let ours = |head: &&Head| head.hand == 0 || hand.map_or(true, |hand| head.hand == hand);
        let head_at = |cell: usize| heads.iter().filter(ours).find(|head| head.cell as usize == cell);
        // (the cells a trunk may not enter, and whether it may enter a cell that way: one it is to pass
        // only the way it is passed)
        let bars = &self.plan.bars[n];
        let barred = |state: usize, way: usize| {
            let cell = state % hw;
            let at_bar = || bars.binary_search(&(cell as u32)).is_ok();
            let wrong_way = || head_at(cell).is_some_and(|head| head.pass && head.way as usize != way);
            (!bars.is_empty() && (at_bar() || wrong_way()))
                || (!shut.is_empty() && shut.binary_search(&(state as u32)).is_ok())
        };
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
            let head = if heads.is_empty() { None } else { head_at(c) };
            // (a terminal that is left one way is come to against that way; one the net runs on through, that way)
            let comes_right = head
                .map_or(true, |head| dir == if head.pass { head.way as usize } else { OPPOSITE[head.way as usize] });
            if ahead_here == 0.0 && comes_right && goals.iter().any(|g| g.cell as usize == c && g.on(l)) {
                found = u;
                break;
            }
            // (and a path that begins on it sets out that way, on the layer it is on)
            let sets_out = head.filter(|_| s.node[ui].prev == START).map(|head| head.way as usize);
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
            let mut bits = prices.at(at).open.get() & allowed[dir];
            if let Some(way) = sets_out {
                bits &= 1 << way;
            }
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let v = (at as isize + off[d2]) as usize;
                let m2 = prices.at(v).mult.get();
                if m2 >= TAKEN || barred(v, d2) {
                    continue;
                }
                relax(v * 9 + d2, corner[dir][d2] + len[d2] * step_cost * m2 as f64, dir as u8, s);
            }
            if sets_out.is_some() {
                continue;
            }
            // The trunk of a pair changes layer in the middle of a straight run, all of it one move: so many
            // steps on, the layer change, and as many steps on the other layer.
            if let Some(run) = &self.plan.runs[n] {
                if dir == NO_DIR {
                    continue;
                }
                let on = |from: usize, layer_cost: f64| {
                    let (mut at, mut cost) = (from, 0.0);
                    for _ in 0..run[dir] {
                        if prices.at(at).open.get() & (1 << dir) == 0 {
                            return None;
                        }
                        at = (at as isize + off[dir]) as usize;
                        let mult = prices.at(at).mult.get();
                        if mult >= TAKEN || barred(at, dir) {
                            return None;
                        }
                        cost += len[dir] * layer_cost * mult as f64;
                    }
                    Some((at, cost))
                };
                let Some((via, before)) = on(at, step_cost) else { continue };
                let change = prices.via(via % hw);
                if change == SHUT_COST {
                    continue;
                }
                for ll in (0..nl).filter(|&ll| ll != l) {
                    let under = ll * hw + via % hw;
                    if prices.at(under).mult.get() >= TAKEN {
                        continue;
                    }
                    if let Some((to, after)) = on(under, lc[ll]) {
                        relax(to * 9 + dir, before + change as f64 + after, VIA + (9 * l + dir) as u8, s);
                    }
                }
                continue;
            }
            let cost = prices.via(c);
            if cost == SHUT_COST {
                continue;
            }
            for ll in 0..nl {
                if ll != l && prices.at(ll * hw + c).mult.get() < TAKEN {
                    relax((ll * hw + c) * 9 + NO_DIR, cost as f64, VIA + (9 * l + dir) as u8, s);
                }
            }
        }
        self.shared.counters.expanded.fetch_add(count, Ordering::Relaxed);
        if found == NONE {
            return None;
        }
        let cost = s.node[found as usize].g as f64;
        Some((self.walk_back(n, s, found), cost))
    }

    /// The path of net `n` that ends in the state `found` of a finished search, from its start, as (layer,
    /// cell) states.
    fn walk_back(&self, n: usize, s: &Search, found: u32) -> Vec<u32> {
        let (hw, off) = (self.plan.space.hw, self.plan.steps.off);
        let mut path = vec![found];
        loop {
            let ui = *path.last().unwrap() as usize;
            let (how, dir, at) = (s.node[ui].prev, ui % 9, ui / 9);
            if how == START {
                break;
            }
            if how < VIA {
                path.push(((at as isize - off[dir]) as usize * 9) as u32 + how as u32);
                continue;
            }
            let (from, how) = (((how - VIA) / 9) as usize, (how - VIA) % 9);
            // (a trunk's layer change is the middle of a straight run that was one move: see `search`)
            let run = self.plan.runs[n].map_or(0, |run| run[dir]);
            let mut back = at;
            for _ in 0..run {
                back = (back as isize - off[dir]) as usize;
                path.push((back * 9 + dir) as u32);
            }
            back = from * hw + back % hw;
            for _ in 0..run {
                path.push((back * 9 + dir) as u32);
                back = (back as isize - off[dir]) as usize;
            }
            path.push((back * 9) as u32 + how as u32);
        }
        path.reverse();
        // the direction is only needed during the search
        path.into_iter().map(|u| u / 9).collect()
    }
}
