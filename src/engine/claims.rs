//! Claims: the ground each laid net takes from the others, and where a net stands in ground another claims.

use super::*;
use crate::geom::dist2_segment;
use crate::grid::{Grid, EPS};

/// The cells whose centres lie within `r` of the segment from `a` to `b`.
fn cells_near(g: &Grid, a: Pt, b: Pt, r: f64) -> Vec<usize> {
    let lo = |v: f64, o: f64| (((v - r - o) / g.step).floor() as i64).max(0);
    let hi = |v: f64, o: f64, n: usize| (((v + r - o) / g.step).ceil() as i64).min(n as i64 - 1);
    let (i0, i1) = (lo(a[0].min(b[0]), g.x0), hi(a[0].max(b[0]), g.x0, g.w));
    let (j0, j1) = (lo(a[1].min(b[1]), g.y0), hi(a[1].max(b[1]), g.y0, g.h));
    let mut out = Vec::new();
    for j in j0..=j1 {
        for i in i0..=i1 {
            let (x, y) = (g.x0 + i as f64 * g.step, g.y0 + j as f64 * g.step);
            if dist2_segment([x, y], a, b) <= (r + EPS) * (r + EPS) {
                out.push(j as usize * g.w + i as usize);
            }
        }
    }
    out
}

impl Engine<'_> {
    /// Claim the cells the copper of net `n` takes, for each size of track and via another net may have.
    pub(super) fn claim(&mut self, n: usize) {
        let _t = self.shared.timed(4);
        let (plan, rules) = (self.plan, &self.p.rules);
        let quiet = if rules.quiet_clearance_mm > 0.0 { rules.quiet_clearance_mm } else { rules.new_clearance_mm };
        let (plain, mine) = (rules.new_clearance_mm, self.p.nets[n].quiet);
        let mut claims_t = Vec::new();
        for (q, &(size, theirs)) in plan.tr.iter().enumerate() {
            let out = self.claimed(n, if theirs != mine { quiet } else { plain }, size, false);
            for &idx in &out {
                self.claims.occ_t[q][idx as usize] += 1;
            }
            claims_t.push(out);
        }
        let mut claims_v = Vec::new();
        for (q, &(size, theirs)) in plan.vr.iter().enumerate() {
            let out = self.claimed(n, if theirs != mine { quiet } else { plain }, size, true);
            for &idx in &out {
                self.claims.occ_v[q][idx as usize] += 1;
            }
            claims_v.push(out);
        }
        self.laid.nets[n].claims_t = claims_t;
        self.laid.nets[n].claims_v = claims_v;
    }

    /// The cells where the centre of another net's copper of half width `size` would be nearer than `c` to
    /// the copper of net `n`, sorted. `against_via`: that copper is a via, which is on every layer: the cells
    /// are then plain cells, not (layer, cell) states.
    fn claimed(&mut self, n: usize, c: f64, size: f64, against_via: bool) -> Vec<u32> {
        let (plan, net) = (self.plan, &self.p.nets[n]);
        let (grid, space) = (&plan.grid, plan.space);
        let (half, via_r) = (net.width / 2.0, net.via[0] / 2.0);
        let track = self.claims.disc(grid, half + c + size);
        let via = if against_via {
            // two holes also keep the drill spacing, whatever the copper round them would allow
            self.claims.disc(grid, (via_r + c + size).max(self.p.rules.hole_centre_mm))
        } else {
            self.claims.disc(grid, via_r + c + size)
        };
        let st = &self.laid.nets[n];
        let near: Vec<(usize, Vec<usize>)> =
            st.shapes.iter().map(|&(l, a, b, half)| (l, cells_near(grid, a, b, half + c + size))).collect();
        self.claims.mark_gen += 1;
        let (mark, gen) = (&mut self.claims.mark, self.claims.mark_gen);
        let mut out: Vec<u32> = Vec::new();
        let mut put = |layer: usize, cell: usize, disc: &[(i32, i32)]| {
            for c in grid.around(cell, disc) {
                let idx = space.state(layer, c);
                if mark[idx] != gen {
                    mark[idx] = gen;
                    out.push(idx as u32);
                }
            }
        };
        // against a via every layer counts as the first
        let on = |layer: usize| if against_via { 0 } else { layer };
        for conn in &st.conns {
            for &s in conn {
                let (layer, cell) = space.split(s as usize);
                put(on(layer), cell, &track);
            }
        }
        for &v in &st.vias {
            for layer in 0..if against_via { 1 } else { space.nl } {
                put(layer, v as usize, &via);
            }
        }
        for (layer, cells) in &near {
            for &cell in cells {
                put(on(*layer), cell, &[(0, 0)]);
            }
        }
        out.sort_unstable();
        out
    }

    pub(super) fn unclaim(&mut self, n: usize) {
        for (q, list) in self.laid.nets[n].claims_t.iter().enumerate() {
            for &idx in list {
                self.claims.occ_t[q][idx as usize] -= 1;
            }
        }
        for (q, list) in self.laid.nets[n].claims_v.iter().enumerate() {
            for &idx in list {
                self.claims.occ_v[q][idx as usize] -= 1;
            }
        }
        self.laid.nets[n].claims_t.clear();
        self.laid.nets[n].claims_v.clear();
    }

    /// Where a laid net stands in ground claimed by another (its own claim counts once everywhere near it).
    pub(super) fn trouble(&mut self, n: usize) -> Trouble {
        let siblings: Vec<usize> =
            self.plan.siblings[n].iter().copied().filter(|&m| !self.laid.nets[m].claims_t.is_empty()).collect();
        for &m in &siblings {
            self.unclaim(m);
        }
        let t = self.trouble_alone(n);
        for &m in &siblings {
            self.claim(m);
        }
        t
    }

    pub(super) fn trouble_alone(&self, n: usize) -> Trouble {
        let _t = self.shared.timed(5);
        let occ_t = &self.claims.occ_t[self.plan.tclass[n]];
        let occ_v = &self.claims.occ_v[self.plan.vclass[n]];
        let st = &self.laid.nets[n];
        let mut t = Trouble { track: Vec::new(), via: Vec::new(), own: Vec::new() };
        for conn in &st.conns {
            // The middle of a diagonal step is nearer to a neighbour than its two ends by under 0.002 mm at these
            // sizes, well inside the 0.02 mm the clearance carries for the grid; the ends are what is tested.
            for &s in conn {
                if occ_t[s as usize] > 1 {
                    t.track.push(s);
                }
            }
        }
        for &v in &st.vias {
            if occ_v[v as usize] > 1 {
                t.via.push(v);
            }
        }
        let need = self.p.rules.hole_centre_mm / self.plan.grid.step;
        for (a, &va) in st.vias.iter().enumerate() {
            for &vb in &st.vias[a + 1..] {
                let ((ja, ia), (jb, ib)) = (self.plan.space.rc(va as usize), self.plan.space.rc(vb as usize));
                let (dj, di) = (ja as f64 - jb as f64, ia as f64 - ib as f64);
                if (dj * dj + di * di).sqrt() < need - 1e-9 {
                    t.own.push(va);
                    t.own.push(vb);
                }
            }
        }
        t
    }
}
