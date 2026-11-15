//! What does not change during a run and is the same for every attempt.

use super::lookahead::Table;
use super::worker::Worker;
use super::DIRS;
use crate::grid::Grid;
use crate::mask::net_mask;
use crate::model::*;
use crate::Event;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Cheapest cost from every (layer, cell) to a set of terminals of a net past the fixed copper alone, shared by all attempts.
pub(super) type Floors = Mutex<HashMap<(usize, Vec<(u32, u8)>, Vec<u32>), Arc<Table>>>;

/// The look-ahead towards a net's copper as it stood in a recent search.
pub(super) struct CopperFloor {
    pub(super) sources: Vec<u32>,
    pub(super) holes: Vec<u32>,
    pub(super) table: Arc<Table>,
}

pub(super) type CopperFloors = Mutex<HashMap<usize, Vec<CopperFloor>>>;

/// A function told of what happens during a run, from the threads the attempts run on.
pub(crate) type Listener = Box<dyn Fn(&Event) + Send + Sync>;

/// What the caller asked to be told or checked during the run.
#[derive(Default)]
pub(crate) struct Diagnostics {
    /// Run every two-pass search a second time in one pass and count those whose cost differs.
    pub(crate) verify: bool,
    /// Told of each round and each mismatch.
    pub(crate) on_event: Option<Listener>,
}

/// The kinds of work whose time is kept.
pub(super) const CLOCK_NAMES: [&str; 8] = [
    "cost tables",
    "look-ahead to copper",
    "first pass",
    "second pass",
    "claims",
    "conflict test",
    "masks",
    "fixed look-ahead",
];

/// Counters of one run, all attempts together.
#[derive(Default)]
pub(super) struct Counters {
    nanos: [AtomicU64; 8],
    pub(super) expanded: AtomicU64,
    pub(super) swept: AtomicU64,
    pub(super) mismatches: AtomicU64,
}

pub(crate) struct Shared {
    /// Per net: per cell, a bit per routing layer closed to its tracks, and VIA_BIT where no via of it may be.
    pub(super) masks: Vec<Vec<u8>>,
    /// Per net: per (layer, cell), a bit per direction in which a step is possible past the fixed copper (inside
    /// the window, the next cell open, and for a slanted step both cells beside it open).
    pub(super) open: Vec<Vec<u8>>,
    pub(super) floors: Floors,
    /// A few per net: the first connection of a net with several terminals is often the same from round to
    /// round and from attempt to attempt.
    pub(super) copper_floors: CopperFloors,
    /// Search memory not in use: nets are laid one after the other within an attempt, each taking a worker from
    /// here and giving it back clean, so that an attempt allocates its tables once.
    pub(super) stock: Mutex<Vec<Worker>>,
    pub(super) counters: Arc<Counters>,
    pub(super) diagnostics: Diagnostics,
}

impl Shared {
    pub(crate) fn new(p: &Problem, diagnostics: Diagnostics) -> Arc<Shared> {
        use rayon::prelude::*;
        let grid = Grid::new(p.window, p.step);
        let (nl, hw, w, h) = (p.layers.len(), grid.cells(), grid.w as i32, grid.h as i32);
        let counters: Arc<Counters> = Arc::default();
        let setup = Timed(counters.clone(), 6, std::time::Instant::now());
        let masks: Vec<Vec<u8>> = p.nets.par_iter().map(|n| net_mask(p, &grid, n)).collect();
        let open = masks
            .par_iter()
            .map(|mask| {
                let mut open = vec![0u8; nl * hw];
                for l in 0..nl {
                    let lbit = 1u8 << l;
                    for j in 0..h {
                        for i in 0..w {
                            let mut bits = 0u8;
                            for (d, &(dj, di)) in DIRS.iter().enumerate() {
                                let (nj, ni) = (j + dj, i + di);
                                if nj < 0 || ni < 0 || nj >= h || ni >= w || mask[(nj * w + ni) as usize] & lbit != 0 {
                                    continue;
                                }
                                if dj != 0
                                    && di != 0
                                    && (mask[(j * w + ni) as usize] | mask[(nj * w + i) as usize]) & lbit != 0
                                {
                                    continue;
                                }
                                bits |= 1 << d;
                            }
                            open[l * hw + (j * w + i) as usize] = bits;
                        }
                    }
                }
                open
            })
            .collect();
        drop(setup);
        Arc::new(Shared {
            masks,
            open,
            floors: Floors::default(),
            copper_floors: CopperFloors::default(),
            stock: Mutex::new(Vec::new()),
            counters,
            diagnostics,
        })
    }

    /// Time the work until the returned value is dropped.
    pub(super) fn timed(&self, kind: usize) -> Timed {
        Timed(self.counters.clone(), kind, std::time::Instant::now())
    }

    pub(super) fn tell(&self, event: impl FnOnce() -> Event) {
        if let Some(f) = &self.diagnostics.on_event {
            f(&event());
        }
    }

    pub(crate) fn stats(&self) -> Stats {
        let c = &self.counters;
        Stats {
            seconds: CLOCK_NAMES
                .iter()
                .zip(&c.nanos)
                .map(|(name, n)| (name.to_string(), n.load(Ordering::Relaxed) as f64 / 1e9))
                .collect(),
            swept: c.swept.load(Ordering::Relaxed),
            expanded: c.expanded.load(Ordering::Relaxed),
            mismatches: c.mismatches.load(Ordering::Relaxed),
        }
    }
}

pub(super) struct Timed(Arc<Counters>, usize, std::time::Instant);

impl Drop for Timed {
    fn drop(&mut self) {
        self.0.nanos[self.1].fetch_add(self.2.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}
