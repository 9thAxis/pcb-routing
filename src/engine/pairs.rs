//! Pairs: a settled trunk becomes two legs, each joined to its pads by stubs.

use super::report::push;
use super::*;
use crate::geom::{dist, dist_segment};
use crate::grid::round6;

impl Engine<'_> {
    /// The centre line of a laid connection that stays on one layer, as points.
    pub(super) fn line(&self, conn: &[u32]) -> Vec<Pt> {
        let mut run = Vec::new();
        for &state in conn {
            push(&mut run, self.grid.xy(state as usize % self.hw));
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
        let conn = self.nets[trunk].conns.first().ok_or_else(|| format!("{}: trunk not laid", pair.key))?.clone();
        let layer = conn[0] as usize / self.hw;
        if conn.iter().any(|&c| c as usize / self.hw != layer) {
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
        let mut lines: [Vec<Pt>; 2] = [Vec::new(), Vec::new()];
        let mut outward: [[Pt; 2]; 2] = [[[0.0; 2]; 2]; 2]; // [leg][end]: unit vector from the centre line to the leg
        for leg in 0..2 {
            let sign = if (leg == 0) == left { 1.0 } else { -1.0 };
            for i in 0..centre.len() {
                let normal = |a: Pt, b: Pt| {
                    let u = unit(a, b);
                    [-u[1] * sign, u[0] * sign]
                };
                let m = if i == 0 {
                    normal(centre[0], centre[1])
                } else if i == centre.len() - 1 {
                    normal(centre[i - 1], centre[i])
                } else {
                    // at a corner the two offset lines meet on the bisector
                    let (a, b) = (normal(centre[i - 1], centre[i]), normal(centre[i], centre[i + 1]));
                    let f = 1.0 + a[0] * b[0] + a[1] * b[1];
                    [(a[0] + b[0]) / f, (a[1] + b[1]) / f]
                };
                if i == 0 {
                    outward[leg][0] = m;
                }
                if i == centre.len() - 1 {
                    outward[leg][1] = m;
                }
                lines[leg].push([round6(centre[i][0] + d * m[0]), round6(centre[i][1] + d * m[1])]);
            }
        }
        self.unclaim(trunk);
        for leg in 0..2 {
            let holder = stubs[0][leg];
            self.unclaim(holder);
            self.nets[holder].shapes =
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
                        let Some(cell) = self.grid.cell(q) else { continue };
                        if self.shared.masks[n][cell] & (1 << layer) != 0 {
                            continue;
                        }
                        let c = self.grid.xy(cell);
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
                self.extra_terms[n] = vec![(cell, 1 << layer)];
                self.exact[n].insert(cell, at);
            }
        }
        self.legs[k] = Some((layer, lines));
        Ok(vec![stubs[0][0], stubs[0][1], stubs[1][0], stubs[1][1]])
    }
}
