//! The fixed things of the board as each net sees them, worked out only where a search goes.
//!
//! A net's mask says, per cell, which layers are closed to its tracks and whether a via of it may stand there;
//! from it follows, per (layer, cell), in which of the eight directions a step can be taken. Both used to be
//! drawn up for the whole board and every net before anything was laid, which on a board of a hundred short
//! nets was most of the work and most of the memory. Here they are kept in pages of [`PAGE`] cells in a row,
//! and a page is drawn up the first time it is looked at: from the copper, regions and holes near it, which
//! are found through three sets of bins.
//!
//! Nets with the same sizes, clearances, layers and regions differ in their masks only near their own copper
//! and holes. Such nets are a class, and a page with none of a net's own things near it is the page of its
//! class, drawn up once and held by all of them.
//!
//! A page is the same whoever asks first and whenever: it is a function of the problem alone.

use super::shared::Counters;
use super::DIRS;
use crate::grid::Grid;
use crate::mask::{free_cells, reaches, stamp_fixed, Rect, Things, VIA_BIT};
use crate::model::*;
use std::sync::{Arc, OnceLock};

pub(super) const PAGE_BITS: usize = 8;
/// Cells in a page: a stretch of the cells as they are numbered, so of one row or the end of one and the
/// start of the next.
pub(super) const PAGE: usize = 1 << PAGE_BITS;

/// Cells along the side of a bin.
const BIN: usize = 32;

/// Things of the board by where they are: per square of [`BIN`] cells, those that can matter to a cell in it.
struct Bins {
    across: usize,
    down: usize,
    lists: Vec<Vec<u32>>,
}

impl Bins {
    fn new(grid: &Grid) -> Bins {
        let (across, down) = (grid.w.div_ceil(BIN), grid.h.div_ceil(BIN));
        Bins { across, down, lists: vec![Vec::new(); across * down] }
    }

    /// Put thing `id` in every bin within `reach` of the box `[min x, min y, max x, max y]`.
    fn add(&mut self, grid: &Grid, id: u32, bounds: [f64; 4], reach: f64) {
        // a cell further out, so that no rounding of the stamps can reach past the bins
        let reach = reach + 2.0 * grid.step;
        let bin = |v: f64, origin: f64, count: usize| -> Option<usize> {
            let cell = ((v - origin) / grid.step).floor();
            (cell >= 0.0).then(|| ((cell as usize) / BIN).min(count - 1))
        };
        let first = |v: f64, origin: f64| ((v - origin) / grid.step).floor().max(0.0) as usize / BIN;
        let (i0, j0) = (first(bounds[0] - reach, grid.x0), first(bounds[1] - reach, grid.y0));
        let (Some(i1), Some(j1)) =
            (bin(bounds[2] + reach, grid.x0, self.across), bin(bounds[3] + reach, grid.y0, self.down))
        else {
            return; // wholly below or left of the grid
        };
        for j in j0..=j1 {
            for i in i0..=i1 {
                self.lists[j * self.across + i].push(id);
            }
        }
    }

    /// The things that can matter to a cell of `rect`, in the order they were given.
    fn near(&self, rect: Rect) -> Vec<u32> {
        let mut out = Vec::new();
        for j in rect.j0 / BIN..=(rect.j0 + rect.h - 1) / BIN {
            for i in rect.i0 / BIN..=(rect.i0 + rect.w - 1) / BIN {
                out.extend_from_slice(&self.lists[j * self.across + i]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn bounds_of(points: impl Iterator<Item = Pt>) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in points {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

/// The box round a shape and how far the shape itself reaches beyond the points it is given by.
fn shape_bounds(shape: &Shape) -> ([f64; 4], f64) {
    match shape {
        Shape::Seg { a, b, w } => (bounds_of([*a, *b].into_iter()), w / 2.0),
        Shape::Circle { c, r } => (bounds_of([*c].into_iter()), *r),
        Shape::Poly { rings } => (bounds_of(rings.iter().flatten().copied()), 0.0),
    }
}

/// One page of one net (or of one class).
struct Page {
    /// Per cell: the mask.
    mask: Arc<[u8]>,
    /// Whether the mask is the class's.
    of_class: bool,
    /// Per layer and cell: the directions in which a step can be taken; drawn up when first asked for.
    open: OnceLock<Arc<[u8]>>,
}

type Pages = Vec<OnceLock<Box<Page>>>;

pub(crate) struct Fixed<'p> {
    p: &'p Problem,
    grid: Grid,
    copper: Bins,
    regions: Bins,
    holes: Bins,
    /// Per net: its class, and the cells its mask is opened at for a layer change (see `free_cells`).
    class: Vec<usize>,
    free: Vec<Vec<u32>>,
    /// Per class: a net of it.
    one_of: Vec<usize>,
    nets: Vec<Pages>,
    classes: Vec<Pages>,
    counters: Arc<Counters>,
}

impl<'p> Fixed<'p> {
    pub(super) fn new(p: &'p Problem, grid: &Grid, counters: Arc<Counters>) -> Fixed<'p> {
        let (copper_reach, region_reach, hole_reach) = reaches(p);
        let (mut copper, mut regions, mut holes) = (Bins::new(grid), Bins::new(grid), Bins::new(grid));
        for (k, c) in p.copper.iter().enumerate() {
            let (bounds, own) = shape_bounds(&c.shape);
            copper.add(grid, k as u32, bounds, own + copper_reach);
        }
        for (k, r) in p.regions.iter().enumerate() {
            regions.add(grid, k as u32, bounds_of(r.rings.iter().flatten().copied()), region_reach);
        }
        for (k, h) in p.holes.iter().enumerate() {
            holes.add(grid, k as u32, bounds_of([h.xy].into_iter()), hole_reach);
        }
        // nets are of one class if all that their masks are made from, but for their own copper, is the same
        let mut kinds: Vec<String> = Vec::new();
        let mut one_of = Vec::new();
        let mut class = Vec::new();
        for (n, net) in p.nets.iter().enumerate() {
            let mut clearances: Vec<(&String, &f64)> = net.clearance.nets.iter().collect();
            clearances.sort_by(|a, b| a.0.cmp(b.0));
            let kind = serde_json::to_string(&(
                net.width,
                net.via,
                &net.layers,
                net.clearance.default,
                clearances,
                &net.regions,
            ))
            .expect("numbers and names can be written");
            class.push(kinds.iter().position(|k| *k == kind).unwrap_or_else(|| {
                kinds.push(kind);
                one_of.push(n);
                kinds.len() - 1
            }));
        }
        let pages = grid.cells().div_ceil(PAGE);
        let empty = || (0..pages).map(|_| OnceLock::new()).collect::<Pages>();
        Fixed {
            p,
            grid: grid.clone(),
            copper,
            regions,
            holes,
            free: p.nets.iter().map(|net| free_cells(p, grid, net)).collect(),
            nets: p.nets.iter().map(|_| empty()).collect(),
            classes: one_of.iter().map(|_| empty()).collect(),
            class,
            one_of,
            counters,
        }
    }

    /// The cells where net `n` has a hole from the start. Sorted.
    pub(super) fn free(&self, n: usize) -> &[u32] {
        &self.free[n]
    }

    /// The mask of net `n` at a cell: a bit per routing layer closed to its tracks, and `VIA_BIT` where no via
    /// of it may be.
    #[inline]
    pub(crate) fn mask(&self, n: usize, cell: usize) -> u8 {
        self.page(n, cell >> PAGE_BITS).mask[cell & (PAGE - 1)]
    }

    /// The directions in which net `n` can take a step from a cell on a layer past the fixed copper, a bit
    /// each (see `DIRS`): inside the window, the next cell open, and for a slanted step both cells beside it open.
    #[inline]
    pub(super) fn open(&self, n: usize, layer: usize, cell: usize) -> u8 {
        self.open_page(n, cell >> PAGE_BITS)[(layer << PAGE_BITS) | (cell & (PAGE - 1))]
    }

    /// The masks of a page of net `n`, a cell each.
    #[inline]
    pub(super) fn mask_page(&self, n: usize, page: usize) -> &[u8] {
        &self.page(n, page).mask
    }

    /// The open directions of a page of net `n`: layer by layer, a cell each.
    pub(super) fn open_page(&self, n: usize, page: usize) -> &[u8] {
        let own = self.page(n, page);
        own.open.get_or_init(|| {
            // the masks looked at are those of the page and of the cells one row and one column round it
            let (first, last) = self.round(page);
            if (first..=last).all(|q| self.page(n, q).of_class) {
                let class = self.class[n];
                let page_of_class = self.class_page(class, page);
                page_of_class.open.get_or_init(|| self.open_from(page, |q| &self.class_page(class, q).mask)).clone()
            } else {
                self.open_from(page, |q| &self.page(n, q).mask)
            }
        })
    }

    /// The first and last page that hold a cell beside one of `page`.
    fn round(&self, page: usize) -> (usize, usize) {
        let (from, to) = self.cells_of(page);
        let reach = self.grid.w + 1;
        (from.saturating_sub(reach) >> PAGE_BITS, (to + reach).min(self.grid.cells()).saturating_sub(1) >> PAGE_BITS)
    }

    /// The cells of a page: the first, and one past the last.
    fn cells_of(&self, page: usize) -> (usize, usize) {
        (page << PAGE_BITS, ((page + 1) << PAGE_BITS).min(self.grid.cells()))
    }

    #[inline]
    fn page(&self, n: usize, page: usize) -> &Page {
        self.nets[n][page].get_or_init(|| Box::new(self.page_of_net(n, page)))
    }

    fn class_page(&self, class: usize, page: usize) -> &Page {
        self.classes[class][page].get_or_init(|| {
            let mask = self.stamp(&self.p.nets[self.one_of[class]], false, &self.near(page));
            Box::new(Page { mask: mask.into(), of_class: true, open: OnceLock::new() })
        })
    }

    fn page_of_net(&self, n: usize, page: usize) -> Page {
        let net = &self.p.nets[n];
        let near = self.near(page);
        let (from, to) = self.cells_of(page);
        let free = &self.free[n];
        let free = &free[free.partition_point(|&c| (c as usize) < from)..free.partition_point(|&c| (c as usize) < to)];
        let own_copper = near
            .iter()
            .flat_map(|stretch| &stretch.copper)
            .any(|&k| self.p.copper[k as usize].net.as_deref() == Some(net.net.as_str()));
        if free.is_empty() && !own_copper {
            let mask = self.class_page(self.class[n], page).mask.clone();
            return Page { mask, of_class: true, open: OnceLock::new() };
        }
        let mut mask = self.stamp(net, true, &near);
        for &cell in free {
            mask[cell as usize - from] &= !VIA_BIT;
        }
        Page { mask: mask.into(), of_class: false, open: OnceLock::new() }
    }

    /// A page as the stretches of rows it is made of, each with what is near it.
    fn near(&self, page: usize) -> Vec<Stretch> {
        let (from, to) = self.cells_of(page);
        let w = self.grid.w;
        let mut out = Vec::new();
        let mut at = from;
        while at < to {
            let (j, i) = (at / w, at % w);
            let len = (w - i).min(to - at);
            let rect = Rect { i0: i, j0: j, w: len, h: 1 };
            out.push(Stretch {
                rect,
                at: at - from,
                copper: self.copper.near(rect),
                regions: self.regions.near(rect),
                holes: self.holes.near(rect),
            });
            at += len;
        }
        out
    }

    /// The mask of a page for `net` (see `stamp_fixed`), always [`PAGE`] long.
    fn stamp(&self, net: &Net, own: bool, near: &[Stretch]) -> Vec<u8> {
        let _t = self.counters.timed(6);
        let p = self.p;
        let mut mask = vec![0u8; PAGE];
        for s in near {
            stamp_fixed(
                p,
                &self.grid,
                net,
                own,
                s.rect,
                &mut mask[s.at..s.at + s.rect.w],
                Things {
                    copper: s.copper.iter().map(|&k| &p.copper[k as usize]),
                    regions: s.regions.iter().map(|&k| &p.regions[k as usize]),
                    holes: s.holes.iter().map(|&k| &p.holes[k as usize]),
                },
            );
        }
        mask
    }

    /// The open directions of a page from the masks, given per page.
    fn open_from<'m>(&self, page: usize, mask_page: impl Fn(usize) -> &'m Arc<[u8]>) -> Arc<[u8]> {
        let _t = self.counters.timed(6);
        let (from, to) = self.cells_of(page);
        let (w, h, nl) = (self.grid.w as i32, self.grid.h as i32, self.p.layers.len());
        let mask = |j: i32, i: i32| -> u8 {
            let cell = (j * w + i) as usize;
            mask_page(cell >> PAGE_BITS)[cell & (PAGE - 1)]
        };
        let mut open = vec![0u8; nl << PAGE_BITS];
        for cell in from..to {
            let (j, i) = (cell as i32 / w, cell as i32 % w);
            for (d, &(dj, di)) in DIRS.iter().enumerate() {
                let (nj, ni) = (j + dj, i + di);
                if nj < 0 || ni < 0 || nj >= h || ni >= w {
                    continue;
                }
                // closed on a layer: the cell stepped to, and for a slanted step the two beside it
                let mut closed = mask(nj, ni);
                if dj != 0 && di != 0 {
                    closed |= mask(j, ni) | mask(nj, i);
                }
                for l in 0..nl {
                    if closed & (1 << l) == 0 {
                        open[(l << PAGE_BITS) | (cell - from)] |= 1 << d;
                    }
                }
            }
        }
        open.into()
    }
}

/// A stretch of one row within a page: where it lies, where it begins in the page, and the copper, regions
/// and holes of the problem that can matter to it.
struct Stretch {
    rect: Rect,
    at: usize,
    copper: Vec<u32>,
    regions: Vec<u32>,
    holes: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::net_mask;
    use serde_json::json;

    /// Three nets of one class and one wider, on a grid whose rows are not a whole number of pages, with
    /// copper of every shape, a region, holes, and copper that reaches past the window.
    fn problem() -> Problem {
        let net = |key: &str, width: f64, a: [f64; 2], b: [f64; 2]| {
            json!({"key": key, "net": key, "width": width, "via": [0.45, 0.2], "layers": ["F", "B"],
                   "clearance": {"default": 0.17, "nets": {"WALL": 0.3}},
                   "terminals": [{"xy": a, "layers": ["F", "B"]}, {"xy": b, "layers": ["F"]}]})
        };
        serde_json::from_value(json!({
            "window": [0.0, 0.0, 31.3, 9.7], "step": 0.1, "layers": ["F", "B"],
            "copper": [
                {"t": "seg", "a": [5.0, -1.0], "b": [9.0, 12.0], "w": 0.2, "net": "WALL", "layers": ["F"]},
                {"t": "seg", "a": [2.0, 2.0], "b": [2.6, 2.0], "w": 0.4, "net": "a", "layers": ["F"], "pad": true},
                {"t": "circle", "c": [20.0, 5.0], "r": 0.6, "net": "b", "layers": ["F", "B"], "pad": true},
                {"t": "poly", "rings": [[[12.0, 1.0], [16.0, 1.5], [15.0, 6.0], [12.5, 4.0]]], "net": null, "layers": ["B"]},
                {"t": "seg", "a": [25.6, 0.0], "b": [25.6, 9.7], "w": 0.15, "net": "c", "layers": ["B"]}
            ],
            "regions": [{"rings": [[[27.0, 3.0], [30.0, 3.0], [30.0, 6.0], [27.0, 6.0]]], "layers": ["F"]}],
            "holes": [{"xy": [20.0, 5.0], "net": "b"}, {"xy": [3.0, 8.0], "net": null}, {"xy": [31.3, 9.7], "net": "a"}],
            "nets": [net("a", 0.15, [2.0, 2.0], [29.0, 8.0]), net("b", 0.15, [20.0, 5.0], [1.0, 9.0]),
                     net("c", 0.15, [25.6, 1.0], [10.0, 9.0]), net("d", 0.4, [1.0, 1.0], [30.0, 1.0])]
        }))
        .unwrap()
    }

    #[test]
    fn a_mask_drawn_up_page_by_page_is_the_mask_of_the_whole_board() {
        let p = problem();
        let grid = Grid::new(p.window, p.step);
        assert_ne!(grid.w % PAGE, 0);
        let fixed = Fixed::new(&p, &grid, Default::default());
        for (n, net) in p.nets.iter().enumerate() {
            let whole = net_mask(&p, &grid, net);
            // from the last cell to the first, so that no page is first asked for in order
            for cell in (0..grid.cells()).rev() {
                assert_eq!(fixed.mask(n, cell), whole[cell], "net {} cell {cell}", net.key);
            }
        }
    }

    #[test]
    fn the_open_directions_follow_from_the_masks() {
        let p = problem();
        let grid = Grid::new(p.window, p.step);
        let fixed = Fixed::new(&p, &grid, Default::default());
        let (w, h) = (grid.w as i32, grid.h as i32);
        for (n, net) in p.nets.iter().enumerate() {
            let mask = net_mask(&p, &grid, net);
            let closed = |j: i32, i: i32, l: usize| mask[(j * w + i) as usize] & (1 << l) != 0;
            for cell in 0..grid.cells() {
                let (j, i) = (cell as i32 / w, cell as i32 % w);
                for l in 0..2 {
                    let mut bits = 0u8;
                    for (d, &(dj, di)) in DIRS.iter().enumerate() {
                        let (nj, ni) = (j + dj, i + di);
                        let inside = nj >= 0 && ni >= 0 && nj < h && ni < w;
                        if inside
                            && !closed(nj, ni, l)
                            && (dj == 0 || di == 0 || !(closed(j, ni, l) || closed(nj, i, l)))
                        {
                            bits |= 1 << d;
                        }
                    }
                    assert_eq!(fixed.open(n, l, cell), bits, "net {} layer {l} cell {cell}", net.key);
                }
            }
        }
    }

    #[test]
    fn nets_of_one_class_hold_one_page_where_none_of_their_own_copper_is_near() {
        let p = problem();
        let grid = Grid::new(p.window, p.step);
        let fixed = Fixed::new(&p, &grid, Default::default());
        assert_eq!(fixed.class, vec![0, 0, 0, 1]);
        // the page of the cell at (10.0, 9.0): far from the copper and holes of a and b; it begins in the row
        // below, where the track of c crosses it
        let page = grid.cell([10.0, 9.0]).unwrap() >> PAGE_BITS;
        let of = |n: usize| fixed.page(n, page);
        assert!(of(0).of_class && of(1).of_class && !of(2).of_class && of(3).of_class);
        assert!(Arc::ptr_eq(&of(0).mask, &of(1).mask));
        assert!(!Arc::ptr_eq(&of(0).mask, &of(3).mask));
    }
}
