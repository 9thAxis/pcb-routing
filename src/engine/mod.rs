//! Routing all nets together by negotiated congestion.
//!
//! Every net is first laid on its cheapest path, with the other nets only making ground dearer, not closing it.
//! Where two nets come closer than the rules allow, the ground is made dearer still (a factor that grows with
//! each round, plus a standing cost that remembers where trouble was) and the nets involved are laid again.
//! The rounds end when no two nets are too close. Then each net is laid once more with all the others fixed,
//! which can only shorten it.
//!
//! How "too close" is measured: each laid net claims, for every size of track and via another net may have,
//! the cells where the centre of such a track or via would be too close to it. A net is in conflict where the
//! centre of its own track or via stands in a cell claimed by another net.

mod claims;
mod lay;
mod lookahead;
mod negotiate;
mod pairs;
mod report;
mod search;
pub(crate) mod shared;
mod space;
mod worker;

use crate::grid::Grid;
use crate::model::*;
use shared::Shared;
use space::{Space, Steps, Term};
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Terminals that could not be joined, each with the reason.
    failed: Vec<String>,
    /// Copper the entry holds besides its paths, as real shapes: (layer, from, to, half width). The leg of a
    /// pair, held by the stub at its first end (see `split`).
    shapes: Vec<(usize, Pt, Pt, f64)>,
}

pub(crate) struct Engine<'a> {
    p: &'a Problem,
    grid: Grid,
    space: Space,
    steps: Steps,
    shared: Arc<Shared>,
    tclass: Vec<usize>,
    vclass: Vec<usize>,
    /// The sizes of track and via a net may have, as (half width, quiet or not): one set of claims is kept per size.
    tr: Vec<(f64, bool)>,
    vr: Vec<(f64, bool)>,
    occ_t: Vec<Vec<u8>>,
    occ_v: Vec<Vec<u8>>,
    hist: Vec<f32>,
    hist_v: Vec<f32>,
    nets: Vec<NetState>,
    mark: Vec<u32>,
    mark_gen: u32,
    shut: Vec<(i32, i32)>,
    discs: HashMap<u64, Vec<(i32, i32)>>,
    layer_cost: Vec<Vec<f64>>,
    exact: Vec<HashMap<u32, Pt>>,
    fixed_free: Vec<Vec<u32>>,
    /// Other entries of the same electrical net: their copper is no obstacle and their vias count as holes of the net.
    siblings: Vec<Vec<usize>>,
    /// Terminals an entry has besides those of the problem, the end of a pair's leg, for a stub.
    extra_terms: Vec<Vec<Term>>,
    /// Per net and terminal: its cell and layers, or why it cannot be reached at all.
    terms: Vec<Vec<Result<Term, String>>>,
    /// Nets laid with the others fixed that could not be laid, each with the reason.
    unlaid: Vec<String>,
    /// How often each look-ahead has been asked for in this attempt: (net, goals, holes).
    asked: std::cell::RefCell<HashMap<LookKey, u32>>,
    /// Per pair, once split: the layer and the two legs as lines.
    legs: Vec<Option<(usize, [Vec<Pt>; 2])>>,
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

fn class_of(list: &mut Vec<(f64, bool)>, v: (f64, bool)) -> usize {
    match list.iter().position(|x| (x.0 - v.0).abs() < 1e-9 && x.1 == v.1) {
        Some(i) => i,
        None => {
            list.push(v);
            list.len() - 1
        }
    }
}

impl<'a> Engine<'a> {
    pub(crate) fn new(p: &'a Problem, shared: Arc<Shared>) -> Engine<'a> {
        let grid = Grid::new(p.window, p.step);
        let space = Space::new(&grid, p.layers.len());
        let (nl, hw) = (space.nl, space.hw);
        let (mut tr, mut vr) = (Vec::new(), Vec::new());
        let tclass: Vec<usize> = p.nets.iter().map(|n| class_of(&mut tr, (n.width / 2.0, n.quiet))).collect();
        let vclass: Vec<usize> = p.nets.iter().map(|n| class_of(&mut vr, (n.via[0] / 2.0, n.quiet))).collect();
        let layer_cost = p
            .nets
            .iter()
            .map(|n| p.layers.iter().map(|l| n.layer_cost.get(l).copied().unwrap_or(1.0)).collect())
            .collect();
        let mut exact = Vec::new();
        let mut fixed_free = Vec::new();
        for n in &p.nets {
            let mut ex: HashMap<u32, Pt> = HashMap::new();
            let mut free = Vec::new();
            for hole in &p.holes {
                if hole.net.as_deref() == Some(n.net.as_str()) {
                    if let Some(c) = grid.cell(hole.xy) {
                        ex.insert(c as u32, hole.xy);
                        free.push(c as u32);
                    }
                }
            }
            for t in &n.terminals {
                if let Some(c) = grid.cell(t.xy) {
                    ex.insert(c as u32, t.xy);
                    if t.layers.len() > 1 {
                        free.push(c as u32);
                    }
                }
            }
            free.sort();
            free.dedup();
            exact.push(ex);
            fixed_free.push(free);
        }
        // Where each terminal is reached. Its own cell if a track of the net may end there; else the nearest
        // cell on the same piece of the net's copper (a pad too near the board edge or another pad is entered
        // where there is room), and the track then ends on that cell, not at the terminal's own point.
        let mut terms = Vec::new();
        for (n, net) in p.nets.iter().enumerate() {
            let mask = &shared.masks[n];
            let mut list = Vec::new();
            for t in &net.terminals {
                let wanted = crate::mask::layer_bits(&p.layers, &t.layers);
                let Some(cell) = grid.cell(t.xy) else {
                    list.push(Err("is outside the window".to_string()));
                    continue;
                };
                let bits = wanted & !mask[cell];
                if bits != 0 {
                    list.push(Ok(Term { cell: cell as u32, layers: bits }));
                    continue;
                }
                let mut best: Option<(f64, u32, u8)> = None;
                let reach = (2.0 / p.step).ceil() as i32;
                for c in grid.around(cell, &Grid::square(reach)) {
                    let open = wanted & !mask[c];
                    if open == 0 {
                        continue;
                    }
                    let q = grid.xy(c);
                    let far = crate::geom::dist(q, t.xy);
                    if best.is_some_and(|b| far >= b.0) {
                        continue;
                    }
                    // on a layer where one piece of the net's copper holds both the terminal and this cell
                    let mut on = 0u8;
                    for piece in &p.copper {
                        if piece.net.as_deref() == Some(net.net.as_str())
                            && crate::geom::inside(&piece.shape, t.xy)
                            && crate::geom::inside(&piece.shape, q)
                        {
                            on |= crate::mask::layer_bits(&p.layers, &piece.layers);
                        }
                    }
                    if open & on != 0 {
                        best = Some((far, c as u32, open & on));
                    }
                }
                match best {
                    Some((_, c, bits)) => {
                        exact[n].remove(&(cell as u32));
                        list.push(Ok(Term { cell: c, layers: bits }));
                    }
                    None => list.push(Err(format!("is closed by fixed copper or a closed region on {:?}", t.layers))),
                }
            }
            terms.push(list);
        }
        Engine {
            p,
            space,
            steps: Steps::new(grid.w),
            shared,
            tclass,
            vclass,
            occ_t: vec![vec![0u8; nl * hw]; tr.len()],
            occ_v: vec![vec![0u8; hw]; vr.len()],
            tr,
            vr,
            hist: vec![0.0; nl * hw],
            hist_v: vec![0.0; hw],
            nets: vec![NetState::default(); p.nets.len()],
            mark: vec![0; nl * hw],
            mark_gen: 0,
            shut: grid.disc(p.rules.hole_centre_mm),
            discs: HashMap::new(),
            layer_cost,
            exact,
            fixed_free,
            siblings: (0..p.nets.len())
                .map(|n| (0..p.nets.len()).filter(|&m| m != n && p.nets[m].net == p.nets[n].net).collect())
                .collect(),
            grid,
            extra_terms: vec![Vec::new(); p.nets.len()],
            terms,
            unlaid: Vec::new(),
            asked: Default::default(),
            legs: vec![None; p.pairs.len()],
        }
    }

    fn disc(&mut self, r: f64) -> Vec<(i32, i32)> {
        let grid = &self.grid;
        self.discs.entry(r.to_bits()).or_insert_with(|| grid.disc(r)).clone()
    }

    fn place(&mut self, n: usize, mut state: NetState) {
        self.unclaim(n);
        if state.shapes.is_empty() {
            state.shapes = std::mem::take(&mut self.nets[n].shapes);
        }
        self.nets[n] = state;
        self.claim(n);
    }

    fn index(&self, key: &str) -> Result<usize, String> {
        self.p.nets.iter().position(|n| n.key == key).ok_or_else(|| format!("no net entry with key {key}"))
    }
}
