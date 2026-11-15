//! What does not change during a run and is the same for every attempt.

use super::fixed::Fixed;
use super::lookahead::Table;
use super::plan::Plan;
use super::space::Term;
use super::worker::Worker;
use crate::grid::Grid;
use crate::model::*;
use crate::Event;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Cheapest cost from every (layer, cell) to a set of terminals of a net past the fixed copper alone, shared by all attempts.
pub(super) type Floors = Mutex<HashMap<(usize, Vec<Term>, Vec<u32>), Arc<Table>>>;

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
pub(super) const CLOCK_NAMES: [&str; 10] = [
    "cost tables",
    "look-ahead to copper",
    "first pass",
    "second pass",
    "claims",
    "conflict test",
    "masks",
    "fixed look-ahead",
    "ends",
    "bus marks",
];

/// Counters of one run, all attempts together.
#[derive(Default)]
pub(super) struct Counters {
    nanos: [AtomicU64; 10],
    pub(super) expanded: AtomicU64,
    pub(super) swept: AtomicU64,
    pub(super) mismatches: AtomicU64,
}

impl Counters {
    /// Time the work of one kind (see `CLOCK_NAMES`) until the returned value is dropped. Work of one kind done
    /// within work of another is counted under both.
    pub(super) fn timed(self: &Arc<Self>, kind: usize) -> Timed {
        Timed(self.clone(), kind, std::time::Instant::now())
    }
}

pub(crate) struct Shared<'p> {
    /// What follows from the problem alone.
    pub(super) plan: Plan,
    /// The fixed things of the board as each net sees them.
    pub(super) fixed: Fixed<'p>,
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

impl<'p> Shared<'p> {
    pub(crate) fn new(p: &'p Problem, diagnostics: Diagnostics) -> Shared<'p> {
        let grid = Grid::new(p.window, p.step);
        let counters: Arc<Counters> = Arc::default();
        let fixed = Fixed::new(p, &grid, counters.clone());
        Shared {
            plan: Plan::new(p, grid, &fixed),
            fixed,
            floors: Floors::default(),
            copper_floors: CopperFloors::default(),
            stock: Mutex::new(Vec::new()),
            counters,
            diagnostics,
        }
    }

    /// Time the work until the returned value is dropped.
    pub(super) fn timed(&self, kind: usize) -> Timed {
        self.counters.timed(kind)
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
