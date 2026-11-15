//! Ground that is open but costs more (see [`crate::model::Dear`]), drawn onto the grid once.

use super::space::Space;
use crate::grid::Grid;
use crate::mask::{layer_bits, stamp_polygon, Rect};
use crate::model::Problem;

/// Where the dear ground of a problem is, and what each net pays there.
pub(crate) struct DearGround {
    space: Space,
    /// Per (layer, cell): 0 in ordinary ground, else the kind of the dearest ground there. Empty for a
    /// problem without any.
    kind: Vec<u8>,
    /// Per net and kind (0: ordinary ground): the factor on a unit of track length.
    factor: Vec<Vec<f32>>,
    /// Per net: the diameter of its via in grid steps, the length a via pays for.
    via_span: Vec<f64>,
}

/// The kinds of dear ground of a problem: (factor, net whose ground it is), the cheapest first. Ground of
/// one factor and one net is of one kind however many pieces it is in.
pub(crate) fn kinds(p: &Problem) -> Vec<(f64, Option<&str>)> {
    let mut kinds: Vec<(f64, Option<&str>)> = p.dear.iter().map(|d| (d.factor, d.net.as_deref())).collect();
    kinds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    kinds.dedup();
    kinds
}

/// The most kinds of dear ground a problem may have.
pub(crate) const MOST_KINDS: usize = 255;

impl DearGround {
    pub(super) fn new(p: &Problem, grid: &Grid, space: Space) -> DearGround {
        let kinds = kinds(p);
        let factor = |n: &crate::model::Net| -> Vec<f32> {
            let of =
                |&(factor, net): &(f64, Option<&str>)| if net == Some(n.net.as_str()) { 1.0 } else { factor as f32 };
            std::iter::once(1.0).chain(kinds.iter().map(of)).collect()
        };
        let mut ground = DearGround {
            space,
            kind: Vec::new(),
            factor: p.nets.iter().map(factor).collect(),
            via_span: p.nets.iter().map(|n| n.via[0] / p.step).collect(),
        };
        if p.dear.is_empty() {
            return ground;
        }
        ground.kind = vec![0u8; space.states()];
        // the cheapest first, so that the dearest is drawn last and stays
        let mut order: Vec<usize> = (0..p.dear.len()).collect();
        let kind_of = |k: usize| kinds.iter().position(|&x| x == (p.dear[k].factor, p.dear[k].net.as_deref())).unwrap();
        order.sort_by_key(|&k| kind_of(k));
        let whole = Rect::whole(grid);
        let mut drawn = vec![0u8; grid.cells()];
        for k in order {
            let d = &p.dear[k];
            let layers = d.layers.as_ref().map_or((1u8 << space.nl) - 1, |names| layer_bits(&p.layers, names));
            if layers == 0 || d.factor <= 1.0 {
                continue;
            }
            stamp_polygon(grid, whole, &mut drawn, 1, &d.rings, 0.0);
            let kind = (kind_of(k) + 1) as u8;
            for (cell, mark) in drawn.iter_mut().enumerate().filter(|(_, mark)| **mark != 0) {
                *mark = 0;
                for layer in (0..space.nl).filter(|l| layers & (1 << l) != 0) {
                    ground.kind[space.state(layer, cell)] = kind;
                }
            }
        }
        ground
    }

    /// What a step of net `n` into a state costs more for the ground it is in, as a factor: 1 in ordinary ground.
    #[inline]
    pub(super) fn track(&self, n: usize, state: usize) -> f32 {
        if self.kind.is_empty() {
            1.0
        } else {
            self.factor[n][self.kind[state] as usize]
        }
    }

    /// What a new via of net `n` at a cell costs more for the ground it passes, in grid steps: what a track
    /// of the via's diameter would on the dearest of its layers.
    #[inline]
    pub(super) fn via(&self, n: usize, cell: usize) -> f64 {
        if self.kind.is_empty() {
            return 0.0;
        }
        let dearest = (0..self.space.nl).map(|l| self.track(n, self.space.state(l, cell))).fold(1.0f32, f32::max);
        (dearest as f64 - 1.0) * self.via_span[n]
    }
}
