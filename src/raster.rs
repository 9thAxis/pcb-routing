//! The grid, and the fixed obstacles of one net drawn onto it.
//!
//! A cell is closed to a track when its centre lies within (half track width + clearance) of foreign copper
//! on that layer, or within (half track width + margin) of a closed region. It is closed to a via when its
//! centre lies within (via radius + clearance) of foreign copper on any layer, within (drill radius + margin)
//! of any solder land, within the hole spacing of another hole, or within (via radius + margin) of a closed
//! region. Distances are exact (to the segment, circle or polygon), not to a polygon drawn round them.

use crate::model::*;

/// A cell exactly at the limit distance is closed (as the project's older router has it); this much further still counts as at the limit.
pub const EPS: f64 = 1e-6;
/// Bit set in a cell of a net's mask when a via may not be placed there. Bits 0..n are the routing layers.
pub const VIA_BIT: u8 = 0x80;

#[derive(Clone, Debug)]
pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub w: usize,
    pub h: usize,
}

impl Grid {
    pub fn new(window: [f64; 4], step: f64) -> Grid {
        // the same count as numpy.arange(start, stop + step / 2, step)
        let n = |a: f64, b: f64| ((b + step / 2.0 - a) / step).ceil() as usize;
        Grid { x0: window[0], y0: window[1], step, w: n(window[0], window[2]), h: n(window[1], window[3]) }
    }
    pub fn cells(&self) -> usize {
        self.w * self.h
    }
    pub fn xy(&self, cell: usize) -> Pt {
        let (j, i) = (cell / self.w, cell % self.w);
        [round6(self.x0 + i as f64 * self.step), round6(self.y0 + j as f64 * self.step)]
    }
    /// Nearest cell, or None outside the window. A point midway between two cells goes to the even one, as Python's round() does.
    pub fn cell(&self, p: Pt) -> Option<usize> {
        let i = ((p[0] - self.x0) / self.step).round_ties_even();
        let j = ((p[1] - self.y0) / self.step).round_ties_even();
        if i < 0.0 || j < 0.0 || i >= self.w as f64 || j >= self.h as f64 {
            return None;
        }
        Some(j as usize * self.w + i as usize)
    }
    /// Offsets (dj, di) of the cells whose centres lie within `r` of a cell centre.
    pub fn disc(&self, r: f64) -> Vec<(i32, i32)> {
        let n = (r / self.step + 1e-6).floor() as i32;
        let mut out = Vec::new();
        for dj in -n..=n {
            for di in -n..=n {
                let d2 = ((di * di + dj * dj) as f64) * self.step * self.step;
                if d2 <= (r + EPS) * (r + EPS) {
                    out.push((dj, di));
                }
            }
        }
        out
    }
}

pub fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn dist2_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let (wx, wy) = (p[0] - a[0], p[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 0.0 { ((wx * vx + wy * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    dx * dx + dy * dy
}

/// Set `bits` in every cell whose centre is within `r` of the segment a-b.
pub fn stamp_capsule(g: &Grid, mask: &mut [u8], bits: u8, a: Pt, b: Pt, r: f64) {
    if bits == 0 {
        return;
    }
    let reach = r + 2.0 * EPS; // the box must not cut off a cell exactly at the limit
    let (minx, maxx) = (a[0].min(b[0]) - reach, a[0].max(b[0]) + reach);
    let (miny, maxy) = (a[1].min(b[1]) - reach, a[1].max(b[1]) + reach);
    let i0 = (((minx - g.x0) / g.step).ceil().max(0.0)) as i64;
    let i1 = (((maxx - g.x0) / g.step).floor()).min(g.w as f64 - 1.0) as i64;
    let j0 = (((miny - g.y0) / g.step).ceil().max(0.0)) as i64;
    let j1 = (((maxy - g.y0) / g.step).floor()).min(g.h as f64 - 1.0) as i64;
    if i1 < i0 || j1 < j0 {
        return;
    }
    // a long slanted segment has a large, mostly empty box: halve it
    let box_cells = ((i1 - i0 + 1) * (j1 - j0 + 1)) as f64;
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let own_cells = (len + 2.0 * r) * (2.0 * r) / (g.step * g.step);
    if box_cells > 4.0 * own_cells + 64.0 && len > 8.0 * g.step {
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        stamp_capsule(g, mask, bits, a, m, r);
        stamp_capsule(g, mask, bits, m, b, r);
        return;
    }
    let r2 = (r + EPS) * (r + EPS);
    for j in j0..=j1 {
        let y = g.y0 + j as f64 * g.step;
        let row = j as usize * g.w;
        for i in i0..=i1 {
            let x = g.x0 + i as f64 * g.step;
            if dist2_segment([x, y], a, b) <= r2 {
                mask[row + i as usize] |= bits;
            }
        }
    }
}

/// Set `bits` in every cell whose centre is inside the polygon (even-odd over its rings) or within `r` of its edge.
pub fn stamp_polygon(g: &Grid, mask: &mut [u8], bits: u8, rings: &[Vec<Pt>], r: f64) {
    if bits == 0 {
        return;
    }
    let mut miny = f64::INFINITY;
    let mut maxy = f64::NEG_INFINITY;
    for ring in rings {
        for p in ring {
            miny = miny.min(p[1]);
            maxy = maxy.max(p[1]);
        }
    }
    if miny > maxy {
        return;
    }
    let j0 = (((miny - g.y0) / g.step).ceil().max(0.0)) as i64;
    let j1 = (((maxy - g.y0) / g.step).floor()).min(g.h as f64 - 1.0) as i64;
    let mut xs: Vec<f64> = Vec::new();
    for j in j0..=j1 {
        let y = g.y0 + j as f64 * g.step;
        xs.clear();
        for ring in rings {
            let n = ring.len();
            for k in 0..n {
                let (p, q) = (ring[k], ring[(k + 1) % n]);
                if (p[1] > y) != (q[1] > y) {
                    xs.push(p[0] + (y - p[1]) * (q[0] - p[0]) / (q[1] - p[1]));
                }
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let row = j as usize * g.w;
        for pair in xs.chunks_exact(2) {
            let i0 = (((pair[0] - g.x0) / g.step - 1e-9).ceil().max(0.0)) as i64;
            let i1 = (((pair[1] - g.x0) / g.step + 1e-9).floor()).min(g.w as f64 - 1.0) as i64;
            for i in i0..=i1 {
                mask[row + i as usize] |= bits;
            }
        }
    }
    for ring in rings {
        let n = ring.len();
        for k in 0..n {
            stamp_capsule(g, mask, bits, ring[k], ring[(k + 1) % n], r);
        }
    }
}

fn stamp_shape(g: &Grid, mask: &mut [u8], bits: u8, shape: &Shape, grow: f64) {
    match shape {
        Shape::Seg { a, b, w } => stamp_capsule(g, mask, bits, *a, *b, w / 2.0 + grow),
        Shape::Circle { c, r } => stamp_capsule(g, mask, bits, *c, *c, r + grow),
        Shape::Poly { rings } => stamp_polygon(g, mask, bits, rings, grow),
    }
}

pub fn layer_bits(layers: &[String], names: &[String]) -> u8 {
    let mut bits = 0u8;
    for (i, l) in layers.iter().enumerate() {
        if names.iter().any(|n| n == l) {
            bits |= 1 << i;
        }
    }
    bits
}

/// The fixed mask of one net: bit i closed to its tracks on routing layer i, VIA_BIT closed to its vias.
pub fn net_mask(p: &Problem, g: &Grid, net: &Net) -> Vec<u8> {
    assert!(p.layers.len() <= 7, "at most seven routing layers");
    let all: u8 = ((1u16 << p.layers.len()) - 1) as u8;
    let allowed = layer_bits(&p.layers, &net.layers);
    let mut mask = vec![all & !allowed; g.cells()];
    let (half, via_r, drill_r) = (net.width / 2.0, net.via[0] / 2.0, net.via[1] / 2.0);
    for c in &p.copper {
        if c.pad {
            stamp_shape(g, &mut mask, VIA_BIT, &c.shape, drill_r + p.rules.pad_drill_margin_mm);
        }
        if c.net.as_deref() == Some(net.net.as_str()) {
            continue;
        }
        let clear = c.net.as_ref().and_then(|n| net.clearance.nets.get(n)).copied().unwrap_or(net.clearance.default);
        stamp_shape(g, &mut mask, layer_bits(&p.layers, &c.layers) & allowed, &c.shape, half + clear);
        stamp_shape(g, &mut mask, VIA_BIT, &c.shape, via_r + clear);
    }
    for region in p.regions.iter().chain(net.regions.iter()) {
        if region.tracks {
            let bits = match &region.layers {
                Some(ls) => layer_bits(&p.layers, ls),
                None => all,
            };
            stamp_polygon(g, &mut mask, bits & allowed, &region.rings, half + p.rules.region_margin_mm);
        }
        if region.vias {
            stamp_polygon(g, &mut mask, VIA_BIT, &region.rings, via_r + p.rules.region_margin_mm);
        }
    }
    for hole in &p.holes {
        stamp_capsule(g, &mut mask, VIA_BIT, hole.xy, hole.xy, p.rules.hole_centre_mm);
    }
    // a hole of this net can be used again as it is
    for hole in &p.holes {
        if hole.net.as_deref() == Some(net.net.as_str()) {
            if let Some(cell) = g.cell(hole.xy) {
                mask[cell] &= !VIA_BIT;
            }
        }
    }
    for t in &net.terminals {
        if t.layers.len() > 1 {
            if let Some(cell) = g.cell(t.xy) {
                mask[cell] &= !VIA_BIT;
            }
        }
    }
    mask
}
