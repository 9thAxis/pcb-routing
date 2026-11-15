//! Routing all nets together by negotiated congestion.
//!
//! Every net is first laid on its cheapest path, with the other nets only making ground dearer, not closing it.
//! Where two nets come closer than the rules allow, the ground is made dearer still (a factor that grows with
//! each round, plus a standing cost that remembers where trouble was) and the nets involved are laid again.
//! The rounds end when no two nets are too close. Then each net is laid once more with all the others fixed,
//! and the new path kept if it is better: shorter, or nearer the other nets of its bus.
//!
//! How "too close" is measured: each laid net claims, for every size of track and via another net may have,
//! the cells where the centre of such a track or via would be too close to it. A net is in conflict where the
//! centre of its own track or via stands in a cell claimed by another net.

mod claims;
mod ends;
mod fixed;
mod lay;
mod lookahead;
mod negotiate;
mod pairs;
pub(crate) mod plan;
mod price;
mod report;
mod search;
pub(crate) mod shared;
mod space;
mod worker;

use crate::grid::Grid;
use crate::model::*;
use plan::Plan;
use shared::Shared;
use space::{Steps, Term};
use std::collections::HashMap;

const NONE: u32 = u32::MAX;

const FREE: u8 = 1; // a layer change here costs nothing: a hole of this net is already there

const SHUT: u8 = 2; // too close to a hole of this net for another via

const DIRS: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// A search state is (direction of arrival, layer, cell); direction 8 is "none" (a start, or just after a layer change).
const NO_DIR: usize = 8;

/// Cost of turning from one direction to another, in units of the 45 degree corner cost; None for a turn sharper than a right angle.
fn turn(from: usize, to: usize) -> Option<f64> {
    if from == NO_DIR || from == to {
        return Some(0.0);
    }
    let (a, b) = (DIRS[from], DIRS[to]);
    let dot = (a.0 * b.0 + a.1 * b.1) as f64 / (((a.0 * a.0 + a.1 * a.1) * (b.0 * b.0 + b.1 * b.1)) as f64).sqrt();
    if dot > 0.5 {
        Some(1.0)
    } else if dot > -0.5 {
        Some(3.0)
    } else {
        None
    }
}

fn dir_of(w: usize, from: usize, to: usize) -> usize {
    let d = ((to / w) as i32 - (from / w) as i32, (to % w) as i32 - (from % w) as i32);
    DIRS.iter().position(|&x| x == d).unwrap()
}

#[derive(Clone, Default)]
struct NetState {
    /// One list of states (layer * cells + cell) per connection, from the tree to the terminal reached.
    conns: Vec<Vec<u32>>,
    vias: Vec<u32>,
    claims_t: Vec<Vec<u32>>,
    claims_v: Vec<Vec<u32>>,
    cost: f64,
    /// Track length in grid steps.
    steps: f64,
    /// For a net of a bus: what its path costs more than `cost` for where it is by the laid nets of its bus,
    /// as they stood when this was laid (see `Options::bus_pull` and `bus_cross`).
    apart: f64,
    /// Terminals that could not be joined, each with the reason.
    failed: Vec<String>,
    /// Copper the entry holds besides its paths, as real shapes: (layer, from, to, half width). The leg of a
    /// pair, held by the stub at its first end (see `split`).
    shapes: Vec<(usize, Pt, Pt, f64)>,
}

/// One attempt at a problem.
pub(crate) struct Engine<'a> {
    p: &'a Problem,
    shared: &'a Shared<'a>,
    /// What follows from the problem alone; the same for every attempt.
    plan: &'a Plan,
    /// Per net: the exact place of what stands in a cell. The plan's, and the leg ends of the pairs split in this attempt.
    exact: Vec<HashMap<u32, Pt>>,
    claims: Claims,
    history: History,
    laid: Laid,
    pairs: Pairs,
    policy: LookPolicy,
}

/// The ground the laid nets claim, as it stands.
struct Claims {
    /// Per size of track (see `Plan::tr`) and (layer, cell): how many nets claim the cell against a track of that size.
    occ_t: Vec<Vec<u8>>,
    /// Per size of via and cell: how many nets claim the cell against a via of that size.
    occ_v: Vec<Vec<u8>>,
    /// Per (layer, cell): `mark_gen` if the cell is already in the list being drawn up.
    mark: Vec<u32>,
    mark_gen: u32,
    /// The offsets of the cells within a radius of a cell, by the radius (its bits).
    discs: HashMap<u64, Vec<(i32, i32)>>,
}

/// The standing costs left where conflicts were.
struct History {
    /// Per (layer, cell), for tracks.
    hist: Vec<f32>,
    /// Per cell, for vias.
    hist_v: Vec<f32>,
}

/// What the attempt has laid, and what it could not.
struct Laid {
    nets: Vec<NetState>,
    /// Nets laid with the others fixed that could not be laid, each with the reason.
    unlaid: Vec<String>,
}

/// What splitting the pairs has added.
struct Pairs {
    /// Terminals an entry has besides those of the problem: the end of a pair's leg, for a stub.
    extra_terms: Vec<Vec<Term>>,
    /// Per pair, once split: the layer and the two legs as lines.
    legs: Vec<Option<(usize, [Vec<Pt>; 2])>>,
}

/// What decides which look-ahead a search gets.
#[derive(Default)]
struct LookPolicy {
    /// How often each look-ahead has been asked for in this attempt: (net, goals, holes).
    asked: std::cell::RefCell<HashMap<LookKey, u32>>,
}

type LookKey = (usize, Vec<Term>, Vec<u32>);

struct Trouble {
    track: Vec<u32>,
    via: Vec<u32>,
    own: Vec<u32>,
}

impl Trouble {
    fn any(&self) -> bool {
        !(self.track.is_empty() && self.via.is_empty() && self.own.is_empty())
    }
}

impl Claims {
    fn disc(&mut self, grid: &Grid, r: f64) -> Vec<(i32, i32)> {
        self.discs.entry(r.to_bits()).or_insert_with(|| grid.disc(r)).clone()
    }
}

impl<'a> Engine<'a> {
    pub(crate) fn new(p: &'a Problem, shared: &'a Shared<'a>) -> Engine<'a> {
        let plan = &shared.plan;
        let (states, cells) = (plan.space.states(), plan.space.hw);
        Engine {
            p,
            shared,
            plan,
            exact: plan.exact.clone(),
            claims: Claims {
                occ_t: vec![vec![0u8; states]; plan.tr.len()],
                occ_v: vec![vec![0u8; cells]; plan.vr.len()],
                mark: vec![0; states],
                mark_gen: 0,
                discs: HashMap::new(),
            },
            history: History { hist: vec![0.0; states], hist_v: vec![0.0; cells] },
            laid: Laid { nets: vec![NetState::default(); p.nets.len()], unlaid: Vec::new() },
            pairs: Pairs { extra_terms: vec![Vec::new(); p.nets.len()], legs: vec![None; p.pairs.len()] },
            policy: LookPolicy::default(),
        }
    }

    fn place(&mut self, n: usize, mut state: NetState) {
        self.unclaim(n);
        if state.shapes.is_empty() {
            state.shapes = std::mem::take(&mut self.laid.nets[n].shapes);
        }
        self.laid.nets[n] = state;
        self.claim(n);
    }

    fn index(&self, key: &str) -> Result<usize, String> {
        self.p.nets.iter().position(|n| n.key == key).ok_or_else(|| format!("no net entry with key {key}"))
    }
}
