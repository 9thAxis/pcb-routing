//! The grid, and the fixed obstacles of one net drawn onto it.
//!
//! A cell is closed to a track when its centre lies within (half track width + clearance) of foreign copper
//! on that layer, or within (half track width + margin) of a closed region. It is closed to a via when its
//! centre lies within (via radius + clearance) of foreign copper on any layer, within (drill radius + margin)
//! of any solder land, within the hole spacing of another hole, or within (via radius + margin) of a closed
//! region. Distances are exact (to the segment, circle or polygon), not to a polygon drawn round them.

use crate::geom::dist2_segment;
use crate::grid::{Grid, EPS};
use crate::model::*;

/// Bit set in a cell of a net's mask when a via may not be placed there. Bits 0..n are the routing layers.
pub const VIA_BIT: u8 = 0x80;

/// A rectangle of cells of the grid. A mask kept for one holds its cells row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    /// First column and first row.
    pub(crate) i0: usize,
    pub(crate) j0: usize,
    /// Columns and rows.
    pub(crate) w: usize,
    pub(crate) h: usize,
}

impl Rect {
    /// All of the grid.
    pub(crate) fn whole(g: &Grid) -> Rect {
        Rect { i0: 0, j0: 0, w: g.w, h: g.h }
    }

    /// The first column and row, then the last, as the stamps count them.
    fn ends(&self) -> (i64, i64, i64, i64) {
        (self.i0 as i64, self.j0 as i64, (self.i0 + self.w) as i64 - 1, (self.j0 + self.h) as i64 - 1)
    }
}

/// Set `bits` in every cell of `rect` whose centre is within `r` of the segment a-b. `mask` is that of `rect`.
/// Which cells are set does not depend on the rectangle: a cell gets the same test whatever else is asked for.
pub(crate) fn stamp_capsule(g: &Grid, rect: Rect, mask: &mut [u8], bits: u8, a: Pt, b: Pt, r: f64) {
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
    let (ri0, rj0, ri1, rj1) = rect.ends();
    if i1 < i0 || j1 < j0 || i1 < ri0 || i0 > ri1 || j1 < rj0 || j0 > rj1 {
        return;
    }
    // a long slanted segment has a large, mostly empty box: halve it
    let box_cells = ((i1 - i0 + 1) * (j1 - j0 + 1)) as f64;
    let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let own_cells = (len + 2.0 * r) * (2.0 * r) / (g.step * g.step);
    if box_cells > 4.0 * own_cells + 64.0 && len > 8.0 * g.step {
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        stamp_capsule(g, rect, mask, bits, a, m, r);
        stamp_capsule(g, rect, mask, bits, m, b, r);
        return;
    }
    let r2 = (r + EPS) * (r + EPS);
    for j in j0.max(rj0)..=j1.min(rj1) {
        let y = g.y0 + j as f64 * g.step;
        let row = (j - rj0) as usize * rect.w;
        for i in i0.max(ri0)..=i1.min(ri1) {
            let x = g.x0 + i as f64 * g.step;
            if dist2_segment([x, y], a, b) <= r2 {
                mask[row + (i - ri0) as usize] |= bits;
            }
        }
    }
}

/// Set `bits` in every cell of `rect` whose centre is inside the polygon (even-odd over its rings) or within
/// `r` of its edge. `mask` is that of `rect`.
pub(crate) fn stamp_polygon(g: &Grid, rect: Rect, mask: &mut [u8], bits: u8, rings: &[Vec<Pt>], r: f64) {
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
    let (ri0, rj0, ri1, rj1) = rect.ends();
    let j0 = (((miny - g.y0) / g.step).ceil().max(0.0)) as i64;
    let j1 = (((maxy - g.y0) / g.step).floor()).min(g.h as f64 - 1.0) as i64;
    let mut xs: Vec<f64> = Vec::new();
    for j in j0.max(rj0)..=j1.min(rj1) {
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
        let row = (j - rj0) as usize * rect.w;
        for pair in xs.chunks_exact(2) {
            let i0 = (((pair[0] - g.x0) / g.step - 1e-9).ceil().max(0.0)) as i64;
            let i1 = (((pair[1] - g.x0) / g.step + 1e-9).floor()).min(g.w as f64 - 1.0) as i64;
            for i in i0.max(ri0)..=i1.min(ri1) {
                mask[row + (i - ri0) as usize] |= bits;
            }
        }
    }
    for ring in rings {
        let n = ring.len();
        for k in 0..n {
            stamp_capsule(g, rect, mask, bits, ring[k], ring[(k + 1) % n], r);
        }
    }
}

fn stamp_shape(g: &Grid, rect: Rect, mask: &mut [u8], bits: u8, shape: &Shape, grow: f64) {
    match shape {
        Shape::Seg { a, b, w } => stamp_capsule(g, rect, mask, bits, *a, *b, w / 2.0 + grow),
        Shape::Circle { c, r } => stamp_capsule(g, rect, mask, bits, *c, *c, r + grow),
        Shape::Poly { rings } => stamp_polygon(g, rect, mask, bits, rings, grow),
    }
}

/// A bit per routing layer (in the order of `layers`) that is among `names`.
pub(crate) fn layer_bits(layers: &[String], names: &[String]) -> u8 {
    let mut bits = 0u8;
    for (i, l) in layers.iter().enumerate() {
        if names.iter().any(|n| n == l) {
            bits |= 1 << i;
        }
    }
    bits
}

/// How far from a piece of fixed copper, a closed region and a hole a cell can be and still be closed by it
/// to some net of the problem: (copper, region, hole). What lies further from a cell than this need not be
/// looked at to know the cell's mask.
pub(crate) fn stamp_reach(p: &Problem) -> (f64, f64, f64) {
    let (mut copper, mut region) = (0.0f64, 0.0f64);
    for net in &p.nets {
        let clear = net.clearance.nets.values().copied().fold(net.clearance.default, f64::max);
        let (half, via_r, drill_r) = (net.width / 2.0, net.via[0] / 2.0, net.via[1] / 2.0);
        copper = copper.max(half + clear).max(via_r + clear).max(drill_r + p.rules.pad_drill_margin_mm);
        region = region.max(half.max(via_r) + p.rules.region_margin_mm);
    }
    let aside = p.pairs.iter().map(|pair| pair.via_pitch / 2.0).fold(0.0, f64::max);
    (copper, region, p.rules.hole_centre_mm + aside)
}

/// The cells where a net may change layer whatever else is near: where it has a hole from the start (a hole
/// of its electrical net, or a terminal on more than one layer). Sorted.
pub(crate) fn free_cells(p: &Problem, g: &Grid, net: &Net) -> Vec<u32> {
    let holes = p.holes.iter().filter(|h| h.net.as_deref() == Some(net.net.as_str())).map(|h| h.xy);
    // (the terminals of a pair's trunk are places beside its pads, not pads: there is no hole in them)
    let trunk = p.pairs.iter().any(|pair| pair.trunk == net.key);
    let terminals = net.terminals.iter().filter(|t| t.layers.len() > 1 && !trunk).map(|t| t.xy);
    let mut cells: Vec<u32> = holes.chain(terminals).filter_map(|xy| g.cell(xy)).map(|c| c as u32).collect();
    cells.sort_unstable();
    cells.dedup();
    cells
}

/// The fixed things of a problem, or those of them near some cells.
pub(crate) struct Things<C, R, H> {
    pub(crate) copper: C,
    pub(crate) regions: R,
    pub(crate) holes: H,
}

/// What the fixed things given close to a net within `rect`, into `mask` (that of `rect`): the layers the net
/// may not use at all, then the copper, the regions and the holes. `own`: whether copper of the net's own
/// electrical net is left out, as it is for the net itself; without it the mask is that of any net with the
/// same sizes, clearances, layers and regions wherever none of its own copper is near. The cells of
/// [`free_cells`] are not opened here.
pub(crate) fn stamp_fixed<'a>(
    p: &Problem,
    g: &Grid,
    net: &'a Net,
    own: bool,
    rect: Rect,
    mask: &mut [u8],
    things: Things<impl Iterator<Item = &'a Copper>, impl Iterator<Item = &'a Region>, impl Iterator<Item = &'a Hole>>,
) {
    let Things { copper, regions, holes } = things;
    let all: u8 = ((1u16 << p.layers.len()) - 1) as u8;
    let allowed = layer_bits(&p.layers, &net.layers);
    mask.fill(all & !allowed);
    let (half, via_r, drill_r) = (net.width / 2.0, net.via[0] / 2.0, net.via[1] / 2.0);
    for c in copper {
        let own = own && c.net.as_deref() == Some(net.net.as_str());
        if c.pad && !(own && p.rules.via_in_pad) {
            stamp_shape(g, rect, mask, VIA_BIT, &c.shape, drill_r + p.rules.pad_drill_margin_mm);
        }
        if own {
            continue;
        }
        let clear = c.net.as_ref().and_then(|n| net.clearance.nets.get(n)).copied().unwrap_or(net.clearance.default);
        stamp_shape(g, rect, mask, layer_bits(&p.layers, &c.layers) & allowed, &c.shape, half + clear);
        stamp_shape(g, rect, mask, VIA_BIT, &c.shape, via_r + clear);
    }
    for region in regions.chain(net.regions.iter()) {
        if region.tracks {
            let bits = match &region.layers {
                Some(ls) => layer_bits(&p.layers, ls),
                None => all,
            };
            stamp_polygon(g, rect, mask, bits & allowed, &region.rings, half + p.rules.region_margin_mm);
        }
        if region.vias {
            stamp_polygon(g, rect, mask, VIA_BIT, &region.rings, via_r + p.rules.region_margin_mm);
        }
    }
    // (the via of a pair's trunk stands for two, each half their distance from its middle)
    let aside = p.pairs.iter().find(|pair| pair.trunk == net.key).map_or(0.0, |pair| pair.via_pitch / 2.0);
    for hole in holes {
        stamp_capsule(g, rect, mask, VIA_BIT, hole.xy, hole.xy, p.rules.hole_centre_mm + aside);
    }
}

/// The fixed mask of one net, one byte per cell of the grid: bit i set where routing layer i is closed to its
/// tracks, [`VIA_BIT`] where no via of it may be. This is the router's whole picture of the board for that net;
/// it is public so that it can be compared with another tool's. (The router itself works the mask out piece
/// by piece, only where a search goes.)
///
/// The problem must be valid ([`Problem::validate`]).
pub fn net_mask(p: &Problem, g: &Grid, net: &Net) -> Vec<u8> {
    let mut mask = vec![0u8; g.cells()];
    let all = Things { copper: p.copper.iter(), regions: p.regions.iter(), holes: p.holes.iter() };
    stamp_fixed(p, g, net, true, Rect::whole(g), &mut mask, all);
    // a hole of this net can be used again as it is
    for cell in free_cells(p, g, net) {
        mask[cell as usize] &= !VIA_BIT;
    }
    mask
}
