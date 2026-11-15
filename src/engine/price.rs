//! What a step and a layer change cost the net being laid, drawn up page by page as its searches come to them.

use super::fixed::{PAGE, PAGE_BITS};
use super::worker::{Ground, Swept, SHUT_COST, TAKEN};
use super::*;
use crate::mask::VIA_BIT;
use std::cell::Cell;

/// How a net is being laid: what the costs of its ground follow from, besides the claims and the standing
/// costs as they are.
#[derive(Clone, Copy)]
pub(super) struct Mode {
    /// The cost factor per other net claiming a cell.
    pub(super) present: f64,
    /// Cells claimed by other nets are closed instead of dearer.
    pub(super) hard: bool,
}

/// The costs of the ground for the net being laid, read through a check that the page read has been drawn
/// up. One table per (layer, cell) holds the factor on a step into the state, the directions open from it and
/// what the first pass of a search finds there; another, per cell, what a layer change costs.
pub(super) struct Prices<'a> {
    engine: &'a Engine<'a>,
    ground: &'a Ground,
    net: usize,
    mode: Mode,
    /// How readily the net gives way; 0 with the others fixed, when there is nothing left to give way to.
    give: f64,
    via_cost: f64,
    own_hole: f32,
    swept: &'a [Swept],
    priced: &'a [Cell<u32>],
    priced_vias: &'a [Cell<u32>],
    lay: u32,
}

impl Engine<'_> {
    /// What a layer change on a hole of net `n` costs it.
    pub(super) fn own_hole(&self, n: usize) -> f32 {
        if self.p.options.own_hole_free {
            0.0
        } else {
            (self.p.nets[n].via_penalty_mm / self.p.step) as f32
        }
    }
}

impl<'a> Prices<'a> {
    pub(super) fn new(engine: &'a Engine<'a>, ground: &'a Ground, net: usize, mode: Mode) -> Prices<'a> {
        let n = &engine.p.nets[net];
        Prices {
            engine,
            ground,
            net,
            mode,
            give: if mode.hard { 0.0 } else { n.yield_ },
            via_cost: n.via_penalty_mm / engine.p.step,
            own_hole: engine.own_hole(net),
            swept: &ground.swept,
            priced: &ground.priced,
            priced_vias: &ground.priced_vias,
            lay: ground.lay,
        }
    }

    /// The whole table per state. Of a state not got through [`Prices::at`] only what the first pass keeps may
    /// be read (`d`, `seen`, `ahead`): its cost and open directions may not be drawn up yet.
    #[inline]
    pub(super) fn swept(&self) -> &'a [Swept] {
        self.swept
    }

    /// The record of a state, its cost and open directions drawn up.
    #[inline(always)]
    pub(super) fn at(&self, state: usize) -> &'a Swept {
        if self.priced[state >> PAGE_BITS].get() != self.lay {
            self.fill(state >> PAGE_BITS);
        }
        &self.swept[state]
    }

    /// What a layer change at a cell costs, SHUT_COST where no via may be.
    #[inline(always)]
    pub(super) fn via(&self, cell: usize) -> f32 {
        if self.priced_vias[cell >> PAGE_BITS].get() != self.lay {
            self.fill_vias(cell >> PAGE_BITS);
        }
        self.ground.vcost[cell].get()
    }

    /// Draw up a page of states: the factor on a step into each, and the directions open from it.
    #[cold]
    #[inline(never)]
    fn fill(&self, page: usize) {
        let e = self.engine;
        let _t = e.shared.timed(0);
        let (hw, fixed, n) = (e.plan.space.hw, &e.shared.fixed, self.net);
        let occ_t = &e.claims.occ_t[e.plan.tclass[n]];
        let (give, present, hard) = (self.give, self.mode.present, self.mode.hard);
        let (from, to) = (page << PAGE_BITS, ((page + 1) << PAGE_BITS).min(e.plan.space.states()));
        let mut k = from;
        while k < to {
            // a stretch of states on one layer whose cells are of one page of the fixed masks
            let (l, c) = (k / hw, k % hw);
            let within = c & (PAGE - 1);
            let len = (to - k).min(hw - c).min(PAGE - within);
            let mask = &fixed.mask_page(n, c >> PAGE_BITS)[within..within + len];
            let open = &fixed.open_page(n, c >> PAGE_BITS)[(l << PAGE_BITS) | within..][..len];
            let bit = 1u8 << l;
            for i in 0..len {
                let s = k + i;
                let occ = occ_t[s] - self.ground.own_t[s];
                let mult = if mask[i] & bit != 0 {
                    SHUT_COST
                } else if hard {
                    if occ > 0 {
                        TAKEN
                    } else {
                        1.0
                    }
                } else {
                    ((1.0 + give * e.history.hist[s] as f64) * (1.0 + give * present * occ as f64)) as f32
                };
                self.swept[s].mult.set(mult * self.ground.apart_at(s));
                self.swept[s].open.set(open[i]);
            }
            k += len;
        }
        self.priced[page].set(self.lay);
    }

    /// Draw up a page of cells: what a layer change costs at each, the holes of the net noted so far included.
    #[cold]
    #[inline(never)]
    fn fill_vias(&self, page: usize) {
        let e = self.engine;
        let _t = e.shared.timed(0);
        let (n, net) = (self.net, &e.p.nets[self.net]);
        let occ_v = &e.claims.occ_v[e.plan.vclass[n]];
        let (give, present, hard) = (self.give, self.mode.present, self.mode.hard);
        let (from, to) = (page << PAGE_BITS, ((page + 1) << PAGE_BITS).min(e.plan.space.hw));
        let mask = e.shared.fixed.mask_page(n, page);
        for c in from..to {
            let occ = occ_v[c] - self.ground.own_v[c];
            let cost = if net.no_vias || mask[c - from] & VIA_BIT != 0 || (hard && occ > 0) {
                SHUT_COST
            } else {
                (self.via_cost * (1.0 + give * e.history.hist_v[c] as f64) * (1.0 + give * present * occ as f64)) as f32
            };
            self.ground.vcost[c].set(self.ground.hole_price(c, self.own_hole, net.no_vias).unwrap_or(cost));
        }
        self.priced_vias[page].set(self.lay);
    }
}
