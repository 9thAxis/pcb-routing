//! Pairs: a settled trunk becomes two legs, each joined to its pads by stubs.

use super::*;
use crate::geom::{dist, dist_segment, push};
use crate::grid::round6;

impl Engine<'_> {
    /// The centre line of a laid connection that stays on one layer, as points.
    pub(super) fn line(&self, conn: &[u32]) -> Vec<Pt> {
        let mut run = Vec::new();
        for &state in conn {
            push(&mut run, self.plan.grid.xy(self.plan.space.split(state as usize).1));
        }
        run
    }

    /// Split the settled trunk of pair `k` into its two legs. The trunk's claim (one wide track) is given up
    /// and each leg is claimed as the shape it is, held by the stub at its first end; every stub gets the end
    /// of its leg as second terminal. Returns the four stub entries, to be laid next.
    pub(super) fn split(&mut self, k: usize) -> Result<Vec<usize>, String> {
        let pair = &self.p.pairs[k];
        let trunk = self.index(&pair.trunk)?;
        let stubs = [
            [self.index(&pair.stubs[0][0])?, self.index(&pair.stubs[0][1])?],
            [self.index(&pair.stubs[1][0])?, self.index(&pair.stubs[1][1])?],
        ];
        let conn = self.laid.nets[trunk].conns.first().ok_or_else(|| format!("{}: trunk not laid", pair.key))?.clone();
        let layer = self.plan.space.split(conn[0] as usize).0;
        if conn.iter().any(|&c| self.plan.space.split(c as usize).0 != layer) {
            return Err(format!("{}: the trunk changes layer", pair.key));
        }
        let centre = self.line(&conn);
        if centre.len() < 2 {
            return Err(format!("{}: the trunk has no length", pair.key));
        }
        let unit = |a: Pt, b: Pt| {
            let (x, y) = (b[0] - a[0], b[1] - a[1]);
            let l = (x * x + y * y).sqrt();
            [x / l, y / l]
        };
        // the first net of the pair takes the side on which its pad at the first end lies further out than the
        // second net's pad (both pads can lie on one side of the trunk)
        let first = unit(centre[0], centre[1]);
        let pad = |n: usize| self.p.nets[n].terminals[0].xy;
        let side_of = |q: Pt| first[0] * (q[1] - centre[0][1]) - first[1] * (q[0] - centre[0][0]);
        let left = side_of(pad(stubs[0][0])) > side_of(pad(stubs[0][1]));
        let d = pair.pitch / 2.0;
        // (what the trunk is wider than its two legs side by side, on either side)
        let spare = ((self.p.nets[trunk].width - pair.pitch - pair.leg_width) / 2.0).max(0.0);
        let mut lines: [Vec<Pt>; 2] = [Vec::new(), Vec::new()];
        for (leg, line) in lines.iter_mut().enumerate() {
            *line = beside(&centre, if (leg == 0) == left { d } else { -d }, spare);
        }
        self.unclaim(trunk);
        for leg in 0..2 {
            let holder = stubs[0][leg];
            self.unclaim(holder);
            self.laid.nets[holder].shapes =
                lines[leg].windows(2).map(|w| (layer, w[0], w[1], pair.leg_width / 2.0)).collect();
            self.claim(holder);
            for end in 0..2 {
                let at = if end == 0 { lines[leg][0] } else { *lines[leg].last().unwrap() };
                let n = stubs[end][leg];
                // The stub arrives on the cell nearest the leg's end from which a track of the stub still keeps
                // its distance from the other leg, and joins the end itself from there.
                let other = &lines[1 - leg];
                let (me, them) = (&self.p.nets[n], &self.p.nets[stubs[0][1 - leg]]);
                let rules = &self.p.rules;
                let apart = if me.quiet != them.quiet && rules.quiet_clearance_mm > 0.0 {
                    rules.quiet_clearance_mm
                } else {
                    rules.new_clearance_mm
                };
                let need = me.width / 2.0 + apart + pair.leg_width / 2.0;
                let step = self.p.step;
                let mut best: Option<(f64, u32)> = None;
                for dj in -3i32..=3 {
                    for di in -3i32..=3 {
                        let q = [at[0] + di as f64 * step, at[1] + dj as f64 * step];
                        let Some(cell) = self.plan.grid.cell(q) else { continue };
                        if self.shared.fixed.mask(n, cell) & (1 << layer) != 0 {
                            continue;
                        }
                        let c = self.plan.grid.xy(cell);
                        let clear = other.windows(2).map(|w| dist_segment(c, w[0], w[1])).fold(f64::INFINITY, f64::min);
                        if clear < need + 1e-6 {
                            continue;
                        }
                        let far = dist(c, at);
                        if best.map_or(true, |b| far < b.0 - 1e-9) {
                            best = Some((far, cell as u32));
                        }
                    }
                }
                let (_, cell) = best.ok_or_else(|| format!("{}: no free cell at the end of a leg at {:?} for its stub {} (the legs are closer than the rules let a stub come, or fixed copper is in the way)", pair.key, at, me.key))?;
                self.pairs.extra_terms[n] = vec![Term { cell, layers: 1 << layer }];
                self.exact[n].insert(cell, at);
            }
        }
        self.pairs.legs[k] = Some((layer, lines));
        Ok(vec![stubs[0][0], stubs[0][1], stubs[1][0], stubs[1][1]])
    }

    /// The stub entries of pair `k`, those of the first end first.
    pub(super) fn stubs_of(&self, k: usize) -> Vec<usize> {
        self.p.pairs[k].stubs.iter().flatten().filter_map(|key| self.index(key).ok()).collect()
    }

    /// Take pair `k` out after its trunk could not be split: the trunk and whatever the split had begun give
    /// up their ground, so that the nets laid after stand as if the pair were not there.
    pub(super) fn take_out(&mut self, k: usize) {
        if let Ok(trunk) = self.index(&self.p.pairs[k].trunk) {
            self.place(trunk, NetState::default());
        }
        for n in self.stubs_of(k) {
            self.unclaim(n);
            self.laid.nets[n] = NetState::default();
            self.pairs.extra_terms[n].clear();
        }
        self.pairs.legs[k] = None;
    }

    /// Lay the four stubs of a pair, one after the other, each fixed for the next. Which goes first can decide
    /// whether all four fit, so the orders are tried in turn until one does. If none does, they are laid in
    /// the order given as far as each goes, and what is missing is listed with each.
    pub(super) fn lay_stubs(&mut self, four: &[usize]) {
        for code in 0..24 {
            let mut ok = true;
            for n in nth_order(four, code) {
                match self.lay_whole(n) {
                    Ok(state) => self.place(n, state),
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return;
            }
            for &n in four {
                self.place(n, NetState::default()); // the leg an entry holds stays with it
            }
        }
        for &n in four {
            let state = self.lay_fixed(n);
            self.place(n, state);
        }
    }
}

/// The line that runs `by` to the left of a line (to its right if negative), its corners where the stretches
/// beside two stretches meet. On the outside of a corner sharp enough for that meeting point to lie more
/// than `spare` further from the line than `by`, the corner is cut straight across instead: the leg stays
/// within the trunk it was split from. A stretch too short to have room on the inside of its corners is
/// left out: beside it the line would run backwards, towards the other leg of the pair.
fn beside(centre: &[Pt], by: f64, spare: f64) -> Vec<Pt> {
    let along = |a: Pt, b: Pt| {
        let (x, y) = (b[0] - a[0], b[1] - a[1]);
        let l = (x * x + y * y).sqrt();
        [x / l, y / l]
    };
    // each stretch moved aside: (where it begins, where it ends, its direction)
    let mut stretches: Vec<(Pt, Pt, Pt)> = centre
        .windows(2)
        .map(|w| {
            let u = along(w[0], w[1]);
            let aside = [-u[1] * by, u[0] * by];
            ([w[0][0] + aside[0], w[0][1] + aside[1]], [w[1][0] + aside[0], w[1][1] + aside[1]], u)
        })
        .collect();
    loop {
        // the stretches as they meet: each from its corner with the one before to its corner with the next
        let mut met: Vec<(Pt, Pt)> = stretches.iter().map(|&(a, b, _)| (a, b)).collect();
        for k in 1..stretches.len() {
            let ((a, _, u), (c, _, v)) = (stretches[k - 1], stretches[k]);
            let (cross, dot) = (u[0] * v[1] - u[1] * v[0], u[0] * v[0] + u[1] * v[1]);
            // (how much further from the line than `by` the two meet: by / cos of half the turn)
            let out = by.abs() * ((2.0 / (1.0 + dot)).sqrt() - 1.0);
            let outside = cross * by < 0.0;
            if cross.abs() < 1e-9 || (outside && out > spare) {
                continue; // in line, or cut across: each keeps its own end
            }
            let t = ((c[0] - a[0]) * v[1] - (c[1] - a[1]) * v[0]) / cross;
            let corner = [a[0] + t * u[0], a[1] + t * u[1]];
            (met[k - 1].1, met[k].0) = (corner, corner);
        }
        let backwards = |k: &usize| {
            let ((p, q), u) = (met[*k], stretches[*k].2);
            (q[0] - p[0]) * u[0] + (q[1] - p[1]) * u[1] <= 1e-9
        };
        match (0..stretches.len()).find(backwards).filter(|_| stretches.len() > 1) {
            Some(k) => {
                // (at an end, the line ends where its neighbour met it)
                if k == 0 {
                    stretches[1].0 = met[1].0;
                } else if k == stretches.len() - 1 {
                    stretches[k - 1].1 = met[k - 1].1;
                }
                stretches.remove(k);
            }
            None => {
                let mut line = Vec::new();
                for (from, to) in met {
                    push(&mut line, [round6(from[0]), round6(from[1])]);
                    push(&mut line, [round6(to[0]), round6(to[1])]);
                }
                return line;
            }
        }
    }
}

/// The `code`-th order of four things (0 to 23); the 0th is the order given.
fn nth_order(four: &[usize], mut code: usize) -> Vec<usize> {
    let mut left: Vec<usize> = four.to_vec();
    let mut order = Vec::new();
    for size in (1..=4).rev() {
        order.push(left.remove(code % size));
        code /= size;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::{beside, nth_order};
    use crate::geom::dist_segments;

    #[test]
    fn the_legs_beside_a_line_keep_their_distance_at_its_corners() {
        let centre = [[0.0, 0.0], [2.0, 0.0], [3.0, 1.0], [3.0, 3.0]];
        let (left, right) = (beside(&centre, 0.2, 0.02), beside(&centre, -0.2, 0.02));
        assert_eq!(left[0], [0.0, 0.2]);
        assert_eq!(right[3], [3.2, 3.0]);
        assert_eq!((left.len(), right.len()), (4, 4));
        let least = |a: &[[f64; 2]], b: &[[f64; 2]]| {
            let pairs = a.windows(2).flat_map(|s| b.windows(2).map(move |t| dist_segments(s[0], s[1], t[0], t[1])));
            pairs.fold(f64::INFINITY, f64::min)
        };
        assert!((least(&left, &right) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn the_outside_of_a_sharp_corner_is_cut_across_so_that_the_leg_stays_within_the_trunk() {
        // a turn of 90 degrees: the legs would meet 0.083 mm further out than they run; the trunk has 0.02 to spare
        let centre = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]];
        let (inside, outside) = (beside(&centre, 0.2, 0.02), beside(&centre, -0.2, 0.02));
        assert_eq!(inside, [[0.0, 0.2], [1.8, 0.2], [1.8, 2.0]]);
        assert_eq!(outside, [[0.0, -0.2], [2.0, -0.2], [2.2, 0.0], [2.2, 2.0]]);
        // every point of the outer leg is within its distance of the line, and the two keep theirs from each other
        let from_line =
            |p: [f64; 2]| dist_segments(p, p, centre[0], centre[1]).min(dist_segments(p, p, centre[1], centre[2]));
        assert!(outside.iter().all(|&p| from_line(p) < 0.2 + 1e-9));
        let least =
            inside.windows(2).flat_map(|s| outside.windows(2).map(move |t| dist_segments(s[0], s[1], t[0], t[1])));
        assert!(least.fold(f64::INFINITY, f64::min) > 0.4 - 1e-9);
        // with room to spare the corner is a corner
        assert_eq!(beside(&centre, -0.2, 0.1), [[0.0, -0.2], [2.2, -0.2], [2.2, 2.0]]);
    }

    #[test]
    fn a_last_stretch_too_short_for_the_inside_of_its_corner_is_left_out() {
        // a line that ends with one slanted step of the grid: beside it on the inside there is no room for it
        let centre = [[0.0, 0.0], [2.0, 0.0], [2.05, 0.05]];
        let (inside, outside) = (beside(&centre, 0.2, 0.02), beside(&centre, -0.2, 0.02));
        assert_eq!(inside, [[0.0, 0.2], [1.917157, 0.2]]); // it ends where the slanted stretch would have met it
        assert_eq!(outside.len(), 3);
        let least = dist_segments(inside[0], inside[1], outside[1], outside[2])
            .min(dist_segments(inside[0], inside[1], outside[0], outside[1]));
        assert!(least > 0.4 - 1e-6, "{least}");
        // the same at the beginning, and for a short stretch between two corners
        let begun = beside(&[[2.05, 0.05], [2.0, 0.0], [0.0, 0.0]], -0.2, 0.02);
        assert_eq!(begun, [[1.917157, 0.2], [0.0, 0.2]]);
        let jog = beside(&[[0.0, 0.0], [2.0, 0.0], [2.05, 0.05], [2.05, 2.0]], 0.2, 0.02);
        assert_eq!(jog, [[0.0, 0.2], [1.85, 0.2], [1.85, 2.0]]);
    }

    #[test]
    fn the_orders_of_four_are_all_there_once_and_begin_with_the_one_given() {
        let four = [7, 3, 9, 5];
        assert_eq!(nth_order(&four, 0), four);
        assert_eq!(nth_order(&four, 1), [3, 7, 9, 5]);
        let mut all: Vec<Vec<usize>> = (0..24).map(|code| nth_order(&four, code)).collect();
        for order in &all {
            let mut sorted = order.clone();
            sorted.sort();
            assert_eq!(sorted, [3, 5, 7, 9]);
        }
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 24);
    }
}
