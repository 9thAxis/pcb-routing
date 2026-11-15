//! What one search keeps for itself.

use super::fixed::{PAGE, PAGE_BITS};
use super::shared::Shared;
use super::{FREE, SHUT};
use crate::grid::Grid;
use crate::queue::BucketQueue;
use std::cell::Cell;

/// What the search keeps per state, together so that one state costs one memory fetch.
#[derive(Clone, Copy)]
pub(super) struct Node {
    pub(super) g: f32,
    pub(super) seen: u16,
    /// How the state was reached: START; or, for a state with a direction, the direction its predecessor (the
    /// cell one step back) was reached in; or, after a layer change, VIA + 9 * the layer come from + the
    /// direction that state was reached in.
    pub(super) prev: u8,
}

pub(super) const START: u8 = 0xFF;

pub(super) const VIA: u8 = 16;

pub(super) struct Search {
    pub(super) node: Vec<Node>,
    pub(super) gen: u16,
    pub(super) heap: BucketQueue,
}

/// The first pass of a search (see `sweep`): its mark and its queue. What it finds is kept in `Ground::swept`.
pub(super) struct Sweep {
    pub(super) gen: u32,
    pub(super) queue: BucketQueue,
}

/// What one search needs for itself. The nets of an attempt are laid one after the other, each taking a
/// worker from the common stock and returning it clean; attempts running side by side each hold one at a time.
pub(super) struct Worker {
    pub(super) search: Search,
    pub(super) sweep: Sweep,
    pub(super) ground: Ground,
}

/// The board as the net being laid finds it: where it stands itself, where its holes are, and what a step and
/// a layer change cost it. The costs are drawn up page by page as the searches of the net come to them (see
/// `Prices`), so a net that stays in one corner of the board costs nothing for the rest of it.
pub(super) struct Ground {
    /// 1 where the net being laid (or another entry of the same electrical net) holds a claim itself: taken off
    /// the common count, so that a net is never in its own way.
    pub(super) own_t: Vec<u8>,
    pub(super) own_v: Vec<u8>,
    pub(super) via_flag: Vec<u8>,
    pub(super) flagged: Vec<u32>,
    /// What a layer change at each cell costs the net being laid, SHUT_COST where no via may be.
    pub(super) vcost: Vec<Cell<f32>>,
    /// Per (layer, cell): what the first pass of a search finds (see `sweep`), and beside it what a step into
    /// the state costs and where a step from it can go.
    pub(super) swept: Vec<Swept>,
    /// Per page of states, and per page of cells: the lay (see `lay`) whose costs it holds.
    pub(super) priced: Vec<Cell<u32>>,
    pub(super) priced_vias: Vec<Cell<u32>>,
    /// Counted up for each net laid.
    pub(super) lay: u32,
    /// For a net of a bus: BESIDE at the states beside a laid net of its bus, where it does not pay for being
    /// apart (see `Options::bus_pull`), OVER at those over or under one on another layer, where it pays more
    /// (`Options::bus_cross`). Empty until a net of a bus is laid; `beside_set` lists what is set.
    pub(super) beside: Vec<u8>,
    pub(super) beside_set: Vec<u32>,
    /// What the net being laid pays more per unit length away from its bus; 0 for a net of none.
    pub(super) apart: f32,
    /// And what it pays more over or under a net of its bus on another layer.
    pub(super) over: f32,
}

/// The tables are read and filled through shared references (a page of costs is drawn up in the middle of a
/// search that is reading its neighbours), so every field is a cell.
#[derive(Clone)]
pub(super) struct Swept {
    /// The cost from here to the goal, as far as the first pass of the present search has got.
    pub(super) d: Cell<f32>,
    /// The mark of the search that last touched the state, and whether its cost is final.
    pub(super) seen: Cell<u32>,
    /// What one step into this state costs the net being laid, as a factor on its length: 1 in free ground
    /// (more by `Ground::apart` away from the net's bus), more where others claim it or conflicts have been, TAKEN where another net stands and may not be
    /// crossed (with the others fixed), SHUT_COST where fixed copper closes it. Kept here so that it comes
    /// with the same memory fetch as the first pass's cost.
    pub(super) mult: Cell<f32>,
    /// The directions in which a step can be taken from here past the fixed copper (see `Fixed::open`).
    pub(super) open: Cell<u8>,
    /// The first pass's look-ahead from here towards the sources, for a state the present search has touched:
    /// worked out once, where every neighbour that reaches the state would work it out again.
    pub(super) ahead: Cell<f32>,
}

/// Marks in `Ground::beside`; NEAR only while the marks of one net of the bus are drawn.
pub(super) const BESIDE: u8 = 1;
pub(super) const NEAR: u8 = 2;
pub(super) const OVER: u8 = 3;

pub(super) const SHUT_COST: f32 = f32::INFINITY;

pub(super) const TAKEN: f32 = f32::MAX;

impl Worker {
    pub(super) fn take(shared: &Shared, states: usize, cells: usize) -> Worker {
        let found = {
            let mut stock = shared.stock.lock().unwrap();
            let at = stock.iter().position(|w| w.ground.own_t.len() == states && w.ground.own_v.len() == cells);
            at.map(|i| stock.swap_remove(i))
        };
        found.unwrap_or_else(|| Worker {
            search: Search {
                node: vec![Node { g: 0.0, prev: START, seen: 0 }; 9 * states],
                gen: 0,
                heap: BucketQueue::new(),
            },
            sweep: Sweep { gen: 0, queue: BucketQueue::new() },
            ground: Ground {
                own_t: vec![0; states],
                own_v: vec![0; cells],
                via_flag: vec![0; cells],
                flagged: Vec::new(),
                vcost: vec![Cell::new(0.0); cells],
                swept: vec![
                    Swept {
                        d: Cell::new(0.0),
                        seen: Cell::new(0),
                        mult: Cell::new(1.0),
                        open: Cell::new(0),
                        ahead: Cell::new(0.0)
                    };
                    states
                ],
                priced: vec![Cell::new(0); states.div_ceil(PAGE)],
                priced_vias: vec![Cell::new(0); cells.div_ceil(PAGE)],
                lay: 0,
                beside: Vec::new(),
                beside_set: Vec::new(),
                apart: 0.0,
                over: 0.0,
            },
        })
    }

    pub(super) fn give_back(mut self, shared: &Shared) {
        let ground = &mut self.ground;
        for &c in &ground.flagged {
            ground.via_flag[c as usize] = 0;
        }
        ground.flagged.clear();
        shared.stock.lock().unwrap().push(self);
    }
}

impl Ground {
    /// Begin a new net: no page holds its costs yet.
    pub(super) fn begin(&mut self) {
        self.lay = self.lay.wrapping_add(1);
        if self.lay == 0 {
            self.priced.iter().chain(&self.priced_vias).for_each(|page| page.set(0));
            self.lay = 1;
        }
    }

    /// What a step into a state costs more for where it is by the bus of the net being laid, as a factor: 1
    /// beside a net of the bus, more away from it, more still over or under one on another layer.
    #[inline]
    pub(super) fn apart_at(&self, state: usize) -> f32 {
        if self.apart <= 0.0 {
            return 1.0;
        }
        match self.beside[state] {
            BESIDE => 1.0,
            OVER => 1.0 + self.over,
            _ => 1.0 + self.apart,
        }
    }

    /// Take the marks of a bus away again.
    pub(super) fn clear_bus(&mut self) {
        for idx in self.beside_set.drain(..) {
            self.beside[idx as usize] = 0;
        }
        self.apart = 0.0;
    }

    /// Note a hole of the net being laid at `cell`: a layer change there is free, and no other via may be
    /// within the offsets `shut` of it.
    pub(super) fn flag_hole(&mut self, grid: &Grid, cell: u32, shut: &[(i32, i32)]) {
        for c in grid.around(cell as usize, shut) {
            if self.via_flag[c] == 0 {
                self.via_flag[c] = SHUT;
                self.flagged.push(c as u32);
            }
        }
        if self.via_flag[cell as usize] == 0 {
            self.flagged.push(cell);
        }
        self.via_flag[cell as usize] = FREE;
    }

    /// What a layer change costs at a cell that a hole of the net has been noted at or near (see `flag_hole`),
    /// None at any other cell.
    #[inline]
    pub(super) fn hole_price(&self, cell: usize, own_hole: f32, no_vias: bool) -> Option<f32> {
        match self.via_flag[cell] {
            0 => None,
            FREE if !no_vias => Some(own_hole),
            _ => Some(SHUT_COST),
        }
    }

    /// Bring the layer-change costs already drawn up in line with the holes noted so far. (A page drawn up
    /// later looks at the holes itself.)
    pub(super) fn price_holes(&mut self, own_hole: f32, no_vias: bool) {
        for &c in &self.flagged {
            if self.priced_vias[c as usize >> PAGE_BITS].get() == self.lay {
                let price = self.hole_price(c as usize, own_hole, no_vias).unwrap_or(SHUT_COST);
                self.vcost[c as usize].set(price);
            }
        }
    }
}
