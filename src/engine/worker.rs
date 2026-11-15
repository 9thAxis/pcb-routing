//! What one search keeps for itself.

use super::shared::Shared;
use super::{FREE, SHUT};
use crate::grid::Grid;
use crate::queue::BucketQueue;

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

/// What one search needs for itself. Nets of a round are laid side by side, each with a worker of its own;
/// workers are handed out from a common stock and returned clean.
pub(super) struct Worker {
    pub(super) search: Search,
    /// 1 where the net being laid (or another entry of the same electrical net) holds a claim itself: taken off
    /// the common count, so that a net is never in its own way.
    pub(super) own_t: Vec<u8>,
    pub(super) own_v: Vec<u8>,
    pub(super) via_flag: Vec<u8>,
    pub(super) flagged: Vec<u32>,
    /// What one step into each (layer, cell) costs the net being laid, as a factor on its length: 1 in free
    /// ground, more where others claim it or conflicts have been, TAKEN where another net stands and may not be
    /// crossed (closing passes), SHUT_COST where fixed copper closes it. Drawn up once per net, so that the
    /// search reads one table instead of four.
    /// It is kept in `swept`, beside what the first pass of a search finds for the same (layer, cell).
    /// The cost of a layer change at each cell, SHUT_COST where no via may be.
    pub(super) vcost: Vec<f32>,
    /// The first pass of a search (see `sweep`): per (layer, cell) the cost from there to the goal.
    pub(super) swept: Vec<Swept>,
    pub(super) sweep_gen: u32,
    pub(super) sweep_queue: BucketQueue,
}

#[derive(Clone, Copy)]
pub(super) struct Swept {
    pub(super) d: f32,
    pub(super) seen: u32,
    pub(super) mult: f32,
}

pub(super) const SHUT_COST: f32 = f32::INFINITY;

pub(super) const TAKEN: f32 = f32::MAX;

impl Worker {
    pub(super) fn take(shared: &Shared, states: usize, cells: usize) -> Worker {
        let found = {
            let mut stock = shared.stock.lock().unwrap();
            let at = stock.iter().position(|w| w.own_t.len() == states && w.own_v.len() == cells);
            at.map(|i| stock.swap_remove(i))
        };
        found.unwrap_or_else(|| Worker {
            search: Search {
                node: vec![Node { g: 0.0, prev: START, seen: 0 }; 9 * states],
                gen: 0,
                heap: BucketQueue::new(),
            },
            own_t: vec![0; states],
            own_v: vec![0; cells],
            via_flag: vec![0; cells],
            flagged: Vec::new(),
            vcost: vec![0.0; cells],
            swept: vec![Swept { d: 0.0, seen: 0, mult: 1.0 }; states],
            sweep_gen: 0,
            sweep_queue: BucketQueue::new(),
        })
    }

    pub(super) fn give_back(mut self, shared: &Shared) {
        for &c in &self.flagged {
            self.via_flag[c as usize] = 0;
        }
        self.flagged.clear();
        shared.stock.lock().unwrap().push(self);
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

    /// Bring the layer-change costs in line with the holes flagged so far.
    pub(super) fn price_holes(&mut self, own_hole: f32, no_vias: bool) {
        for &c in &self.flagged {
            self.vcost[c as usize] = if self.via_flag[c as usize] == FREE && !no_vias { own_hole } else { SHUT_COST };
        }
    }
}
