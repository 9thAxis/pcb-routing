//! Claims: the ground each laid net takes from the others, and where a net stands in ground another claims.

use super::*;
use crate::geom::dist2_segment;
use crate::grid::EPS;

impl Engine<'_> {
    /// The cells a net's copper claims, for each size of track and via another net may have.
    /// The cells whose centres lie within `r` of the segment from `a` to `b`.
    pub(super) fn near(&self, a: Pt, b: Pt, r: f64) -> Vec<usize> {
        let g = &self.grid;
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

    pub(super) fn claim(&mut self, n: usize) {
        let _t = self.shared.timed(4);
        let net = &self.p.nets[n];
        let rules = &self.p.rules;
        let quiet = if rules.quiet_clearance_mm > 0.0 { rules.quiet_clearance_mm } else { rules.new_clearance_mm };
        let (plain, mine) = (rules.new_clearance_mm, net.quiet);
        let shapes = self.nets[n].shapes.clone();
        let (grid, space, nl) = (self.grid.clone(), self.space, self.space.nl);
        let (half, via_r) = (net.width / 2.0, net.via[0] / 2.0);
        let conns = self.nets[n].conns.clone();
        let vias = self.nets[n].vias.clone();
        let mut claims_t = Vec::new();
        for q in 0..self.tr.len() {
            let c = if self.tr[q].1 != mine { quiet } else { plain };
            let track = self.disc(half + c + self.tr[q].0);
            let via = self.disc(via_r + c + self.tr[q].0);
            let near: Vec<(usize, Vec<usize>)> =
                shapes.iter().map(|&(l, a, b, hw_)| (l, self.near(a, b, hw_ + c + self.tr[q].0))).collect();
            self.mark_gen += 1;
            let gen = self.mark_gen;
            let mut out: Vec<u32> = Vec::new();
            let mut put = |mark: &mut Vec<u32>, layer: usize, cell: usize, disc: &[(i32, i32)]| {
                for c in grid.around(cell, disc) {
                    let idx = space.state(layer, c);
                    if mark[idx] != gen {
                        mark[idx] = gen;
                        out.push(idx as u32);
                    }
                }
            };
            for conn in &conns {
                for &s in conn {
                    let (layer, cell) = space.split(s as usize);
                    put(&mut self.mark, layer, cell, &track);
                }
            }
            for &v in &vias {
                for l in 0..nl {
                    put(&mut self.mark, l, v as usize, &via);
                }
            }
            for (l, cells) in &near {
                for &cell in cells {
                    put(&mut self.mark, *l, cell, &[(0, 0)]);
                }
            }
            out.sort_unstable();
            for &idx in &out {
                self.occ_t[q][idx as usize] += 1;
            }
            claims_t.push(out);
        }
        let mut claims_v = Vec::new();
        for q in 0..self.vr.len() {
            let c = if self.vr[q].1 != mine { quiet } else { plain };
            let track = self.disc(half + c + self.vr[q].0);
            // two holes also keep the drill spacing, whatever the copper round them would allow
            let via = self.disc((via_r + c + self.vr[q].0).max(self.p.rules.hole_centre_mm));
            let near: Vec<Vec<usize>> =
                shapes.iter().map(|&(_, a, b, hw_)| self.near(a, b, hw_ + c + self.vr[q].0)).collect();
            self.mark_gen += 1;
            let gen = self.mark_gen;
            let mut out: Vec<u32> = Vec::new();
            let mut put = |mark: &mut Vec<u32>, cell: usize, disc: &[(i32, i32)]| {
                for idx in grid.around(cell, disc) {
                    if mark[idx] != gen {
                        mark[idx] = gen;
                        out.push(idx as u32);
                    }
                }
            };
            for conn in &conns {
                for &s in conn {
                    put(&mut self.mark, space.split(s as usize).1, &track);
                }
            }
            for &v in &vias {
                put(&mut self.mark, v as usize, &via);
            }
            for cells in &near {
                for &cell in cells {
                    put(&mut self.mark, cell, &[(0, 0)]);
                }
            }
            out.sort_unstable();
            for &idx in &out {
                self.occ_v[q][idx as usize] += 1;
            }
            claims_v.push(out);
        }
        self.nets[n].claims_t = claims_t;
        self.nets[n].claims_v = claims_v;
    }

    pub(super) fn unclaim(&mut self, n: usize) {
        for (q, list) in self.nets[n].claims_t.iter().enumerate() {
            for &idx in list {
                self.occ_t[q][idx as usize] -= 1;
            }
        }
        for (q, list) in self.nets[n].claims_v.iter().enumerate() {
            for &idx in list {
                self.occ_v[q][idx as usize] -= 1;
            }
        }
        self.nets[n].claims_t.clear();
        self.nets[n].claims_v.clear();
    }

    /// Where a laid net stands in ground claimed by another (its own claim counts once everywhere near it).
    pub(super) fn trouble(&mut self, n: usize) -> Trouble {
        let siblings: Vec<usize> =
            self.siblings[n].iter().copied().filter(|&m| !self.nets[m].claims_t.is_empty()).collect();
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
        let occ_t = &self.occ_t[self.tclass[n]];
        let occ_v = &self.occ_v[self.vclass[n]];
        let st = &self.nets[n];
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
        let need = self.p.rules.hole_centre_mm / self.grid.step;
        for (a, &va) in st.vias.iter().enumerate() {
            for &vb in &st.vias[a + 1..] {
                let ((ja, ia), (jb, ib)) = (self.space.rc(va as usize), self.space.rc(vb as usize));
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
