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

use crate::model::*;
use crate::raster::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Cheapest cost from every (layer, cell) to a set of terminals of a net past the fixed copper alone, shared by all attempts.
pub type Floors = Arc<Mutex<HashMap<(usize, Vec<(u32, u8)>, Vec<u32>), Arc<Vec<f32>>>>>;

/// What does not change during a run and is the same for every attempt.
pub struct Shared {
    /// Per net: per cell, a bit per routing layer closed to its tracks, and VIA_BIT where no via of it may be.
    pub masks: Vec<Vec<u8>>,
    /// Per net: per (layer, cell), a bit per direction in which a step is possible past the fixed copper (inside
    /// the window, the next cell open, and for a slanted step both cells beside it open).
    open: Vec<Vec<u8>>,
    floors: Floors,
    /// The look-ahead towards a net's copper as it stood in a recent search, a few per net: the first connection
    /// of a net with several terminals is often the same from round to round and from attempt to attempt.
    copper_floors: Mutex<HashMap<usize, Vec<(Vec<u32>, Vec<u32>, Arc<Vec<f32>>)>>>,
}

impl Shared {
    pub fn new(p: &Problem) -> Arc<Shared> {
        use rayon::prelude::*;
        let grid = Grid::new(p.window, p.step);
        let (nl, hw, w, h) = (p.layers.len(), grid.cells(), grid.w as i32, grid.h as i32);
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
                                if dj != 0 && di != 0 && (mask[(j * w + ni) as usize] | mask[(nj * w + i) as usize]) & lbit != 0 {
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
        Arc::new(Shared { masks, open, floors: Floors::default(), copper_floors: Mutex::new(HashMap::new()) })
    }
}

/// Time spent per kind of work, in nanoseconds, over the whole run (printed with PCB_ROUTING_TRACE).
pub static CLOCK: [std::sync::atomic::AtomicU64; 6] = [const { std::sync::atomic::AtomicU64::new(0) }; 6];
pub const CLOCK_NAMES: [&str; 6] = ["cost tables", "look-ahead to copper", "first pass", "second pass", "claims", "conflict test"];

struct Timed(usize, std::time::Instant);

fn timed(kind: usize) -> Timed {
    Timed(kind, std::time::Instant::now())
}

impl Drop for Timed {
    fn drop(&mut self) {
        CLOCK[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

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

/// Queue key of a state: its estimated total cost in 64ths of a grid step, and below that the cost still ahead
/// (16ths of a step), so that of two states with the same estimate the one nearer the goal comes out first.
/// The 64ths make the order of states that differ by less than 0.0008 mm (at a 0.05 mm step) arbitrary but
/// fixed; a path can be dearer than the cheapest by amounts of that size.
#[inline]
fn key(total: f64) -> u64 {
    (total * 64.0) as u64
}

/// The corner costs as a table: [direction of arrival][new direction], in grid steps; and per direction of
/// arrival the new directions that are allowed, a bit each.
fn turns(bend: f64) -> ([[f64; 8]; 9], [u8; 9]) {
    let mut cost = [[0.0; 8]; 9];
    let mut allowed = [0u8; 9];
    for from in 0..9 {
        for to in 0..8 {
            if let Some(t) = turn(from, to) {
                cost[from][to] = t * bend;
                allowed[from] |= 1 << to;
            }
        }
    }
    (cost, allowed)
}

fn dir_of(w: usize, from: usize, to: usize) -> usize {
    let d = ((to / w) as i32 - (from / w) as i32, (to % w) as i32 - (from % w) as i32);
    DIRS.iter().position(|&x| x == d).unwrap()
}

/// What a search starts from, for the look-ahead towards it: a set of terminals, or more copper than that.
#[derive(Clone, Copy)]
enum From<'t> {
    Terminals(&'t [(u32, u8)]),
    Copper,
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
    /// Copper the entry holds besides its paths, as real shapes: (layer, from, to, half width). The leg of a
    /// pair, held by the stub at its first end (see `split`).
    shapes: Vec<(usize, Pt, Pt, f64)>,
}

/// What the search keeps per state, together so that one state costs one memory fetch.
#[derive(Clone, Copy)]
struct Node {
    g: f32,
    seen: u16,
    /// How the state was reached: START; or, for a state with a direction, the direction its predecessor (the
    /// cell one step back) was reached in; or, after a layer change, VIA + 9 * the layer come from + the
    /// direction that state was reached in.
    prev: u8,
}

const START: u8 = 0xFF;
const VIA: u8 = 16;

struct Search {
    node: Vec<Node>,
    gen: u16,
    heap: crate::queue::BucketQueue,
}

/// What one search needs for itself. Nets of a round are laid side by side, each with a worker of its own;
/// workers are handed out from a common stock and returned clean.
struct Worker {
    search: Search,
    /// 1 where the net being laid (or another entry of the same electrical net) holds a claim itself: taken off
    /// the common count, so that a net is never in its own way.
    own_t: Vec<u8>,
    own_v: Vec<u8>,
    via_flag: Vec<u8>,
    flagged: Vec<u32>,
    /// What one step into each (layer, cell) costs the net being laid, as a factor on its length: 1 in free
    /// ground, more where others claim it or conflicts have been, TAKEN where another net stands and may not be
    /// crossed (closing passes), SHUT_COST where fixed copper closes it. Drawn up once per net, so that the
    /// search reads one table instead of four.
    /// It is kept in `swept`, beside what the first pass of a search finds for the same (layer, cell).
    /// The cost of a layer change at each cell, SHUT_COST where no via may be.
    vcost: Vec<f32>,
    /// The first pass of a search (see `sweep`): per (layer, cell) the cost from there to the goal.
    swept: Vec<Swept>,
    sweep_gen: u32,
    sweep_queue: crate::queue::BucketQueue,
}

#[derive(Clone, Copy)]
struct Swept {
    d: f32,
    seen: u32,
    mult: f32,
}

const SHUT_COST: f32 = f32::INFINITY;
const TAKEN: f32 = f32::MAX;

static STOCK: Mutex<Vec<Worker>> = Mutex::new(Vec::new());

impl Worker {
    fn take(states: usize, cells: usize) -> Worker {
        let found = {
            let mut stock = STOCK.lock().unwrap();
            let at = stock.iter().position(|w| w.own_t.len() == states && w.own_v.len() == cells);
            at.map(|i| stock.swap_remove(i))
        };
        found.unwrap_or_else(|| Worker {
            search: Search { node: vec![Node { g: 0.0, prev: START, seen: 0 }; 9 * states], gen: 0, heap: crate::queue::BucketQueue::new() },
            own_t: vec![0; states],
            own_v: vec![0; cells],
            via_flag: vec![0; cells],
            flagged: Vec::new(),
            vcost: vec![0.0; cells],
            swept: vec![Swept { d: 0.0, seen: 0, mult: 1.0 }; states],
            sweep_gen: 0,
            sweep_queue: crate::queue::BucketQueue::new(),
        })
    }

    fn give_back(mut self) {
        for &c in &self.flagged {
            self.via_flag[c as usize] = 0;
        }
        self.flagged.clear();
        STOCK.lock().unwrap().push(self);
    }

    fn flag_hole(&mut self, w: i32, h: i32, cell: u32, shut: &[(i32, i32)]) {
        let (j, i) = (cell as i32 / w, cell as i32 % w);
        for &(dj, di) in shut {
            let (jj, ii) = (j + dj, i + di);
            if jj >= 0 && ii >= 0 && jj < h && ii < w {
                let c = (jj * w + ii) as usize;
                if self.via_flag[c] == 0 {
                    self.via_flag[c] = SHUT;
                    self.flagged.push(c as u32);
                }
            }
        }
        if self.via_flag[cell as usize] == 0 {
            self.flagged.push(cell);
        }
        self.via_flag[cell as usize] = FREE;
    }

    /// Bring the layer-change costs in line with the holes flagged so far.
    fn price_holes(&mut self, own_hole: f32, no_vias: bool) {
        for &c in &self.flagged {
            self.vcost[c as usize] = if self.via_flag[c as usize] == FREE && !no_vias { own_hole } else { SHUT_COST };
        }
    }
}

pub struct Engine<'a> {
    p: &'a Problem,
    pub grid: Grid,
    nl: usize,
    hw: usize,
    pub shared: Arc<Shared>,
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
    /// How readily each net gives way just now: its own `yield`, lowered round by round while it is longer than its `max_mm`.
    give: Vec<f64>,
    /// Nets held fixed during a repair, and the count of their claims per cell (as occ_t and occ_v).
    frozen: Vec<bool>,
    fro_t: Vec<Vec<u8>>,
    fro_v: Vec<Vec<u8>>,
    pub expanded: std::sync::atomic::AtomicU64,
    pub swept: std::sync::atomic::AtomicU64,
    /// Terminals an entry has besides those of the problem, as (cell, layers): the end of a pair's leg, for a stub.
    extra_terms: Vec<Vec<(u32, u8)>>,
    /// Per pair, once split: the layer and the two legs as lines.
    legs: Vec<Option<(usize, [Vec<Pt>; 2])>>,
}

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
    pub fn new(p: &'a Problem) -> Engine<'a> {
        Engine::with_shared(p, Shared::new(p))
    }

    pub fn with_shared(p: &'a Problem, shared: Arc<Shared>) -> Engine<'a> {
        let grid = Grid::new(p.window, p.step);
        let (nl, hw) = (p.layers.len(), grid.cells());
        assert!(p.nets.len() < 250, "too many nets for one run");
        let (mut tr, mut vr) = (Vec::new(), Vec::new());
        let tclass: Vec<usize> = p.nets.iter().map(|n| class_of(&mut tr, (n.width / 2.0, n.quiet))).collect();
        let vclass: Vec<usize> = p.nets.iter().map(|n| class_of(&mut vr, (n.via[0] / 2.0, n.quiet))).collect();
        let layer_cost = p.nets.iter().map(|n| p.layers.iter().map(|l| n.layer_cost.get(l).copied().unwrap_or(1.0)).collect()).collect();
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
        let classes = (tr.len(), vr.len());
        Engine {
            p,
            nl,
            hw,
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
            siblings: (0..p.nets.len()).map(|n| (0..p.nets.len()).filter(|&m| m != n && p.nets[m].net == p.nets[n].net).collect()).collect(),
            grid,
            give: p.nets.iter().map(|n| n.yield_).collect(),
            frozen: vec![false; p.nets.len()],
            fro_t: vec![vec![0u8; nl * hw]; classes.0],
            fro_v: vec![vec![0u8; hw]; classes.1],
            expanded: std::sync::atomic::AtomicU64::new(0),
            swept: std::sync::atomic::AtomicU64::new(0),
            extra_terms: vec![Vec::new(); p.nets.len()],
            legs: vec![None; p.pairs.len()],
        }
    }

    fn disc(&mut self, r: f64) -> Vec<(i32, i32)> {
        let grid = &self.grid;
        self.discs.entry(r.to_bits()).or_insert_with(|| grid.disc(r)).clone()
    }

    /// Lay one net against the claims of the others as they stand: grow a tree from its first terminal to the
    /// nearest remaining terminal until all are joined. Nothing of the engine changes; the caller puts the result
    /// in place. `hard`: cells claimed by other nets are closed instead of dearer.
    fn route_net(&self, n: usize, present: f64, hard: bool) -> Result<NetState, String> {
        let mut w = Worker::take(self.nl * self.hw, self.hw);
        let mine: Vec<usize> = std::iter::once(n).chain(self.siblings[n].iter().copied()).collect();
        for &m in &mine {
            for &idx in self.nets[m].claims_t.get(self.tclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_t[idx as usize] += 1;
            }
            for &idx in self.nets[m].claims_v.get(self.vclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_v[idx as usize] += 1;
            }
        }
        let result = self.route_alone(n, present, hard, &mut w);
        for &m in &mine {
            for &idx in self.nets[m].claims_t.get(self.tclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_t[idx as usize] = 0;
            }
            for &idx in self.nets[m].claims_v.get(self.vclass[n]).map_or(&[][..], |v| &v[..]) {
                w.own_v[idx as usize] = 0;
            }
        }
        w.give_back();
        result
    }

    fn route_alone(&self, n: usize, present: f64, hard: bool, w: &mut Worker) -> Result<NetState, String> {
        let p = self.p;
        let net = &p.nets[n];
        let (hw, nl) = (self.hw, self.nl);
        let (gw, gh) = (self.grid.w as i32, self.grid.h as i32);
        let shut = &self.shut;
        for &c in &self.fixed_free[n] {
            w.flag_hole(gw, gh, c, shut);
        }
        // Holes the net gets to change layer on for nothing that the fixed look-aheads do not know from the start:
        // vias of other entries of the same net, and its own new vias unless they become starting points below.
        let mut holes: Vec<u32> = Vec::new();
        for &m in &self.siblings[n] {
            for &v in &self.nets[m].vias {
                w.flag_hole(gw, gh, v, shut);
                holes.push(v);
            }
        }
        if !p.options.own_hole_free {
            holes.clear(); // a layer change on an own hole then costs as any other
        }
        holes.sort_unstable();
        holes.dedup();
        let via_cost = net.via_penalty_mm / p.step;
        let own_hole = if p.options.own_hole_free { 0.0 } else { via_cost as f32 };
        {
            let _t = timed(0);
            let give = if hard { 0.0 } else { self.give[n] }; // with the others fixed there is nothing left to give way to
            let mask = &self.shared.masks[n];
            let (occ_t, occ_v) = (&self.occ_t[self.tclass[n]], &self.occ_v[self.vclass[n]]);
            let (fro_t, fro_v) = (&self.fro_t[self.tclass[n]], &self.fro_v[self.vclass[n]]);
            for l in 0..nl {
                let bit = 1u8 << l;
                for c in 0..hw {
                    let k = l * hw + c;
                    let occ = occ_t[k] - w.own_t[k];
                    w.swept[k].mult = if mask[c] & bit != 0 {
                        SHUT_COST
                    } else if hard || fro_t[k] > 0 {
                        if occ > 0 { TAKEN } else { 1.0 }
                    } else {
                        ((1.0 + give * self.hist[k] as f64) * (1.0 + give * present * occ as f64)) as f32
                    };
                }
            }
            for c in 0..hw {
                let occ = occ_v[c] - w.own_v[c];
                w.vcost[c] = if net.no_vias || mask[c] & VIA_BIT != 0 || ((hard || fro_v[c] > 0) && occ > 0) {
                    SHUT_COST
                } else {
                    (via_cost * (1.0 + give * self.hist_v[c] as f64) * (1.0 + give * present * occ as f64)) as f32
                };
            }
            w.price_holes(own_hole, net.no_vias);
        }
        let mut terms: Vec<(u32, u8)> = Vec::new();
        for t in &net.terminals {
            let cell = self.grid.cell(t.xy).ok_or_else(|| format!("{}: terminal {:?} is outside the window", net.key, t.xy))? as u32;
            let bits = layer_bits(&p.layers, &t.layers) & !self.shared.masks[n][cell as usize];
            if bits == 0 {
                return Err(format!("{}: terminal {:?} is closed by fixed copper or a closed region on {:?}", net.key, t.xy, t.layers));
            }
            terms.push((cell, bits));
        }
        terms.extend_from_slice(&self.extra_terms[n]);
        if terms.len() < 2 {
            return Err(format!("{}: fewer than two terminals", net.key));
        }
        let mut state = NetState::default();
        let mut tree: Vec<u32> = Vec::new();
        let add_term = |tree: &mut Vec<u32>, t: (u32, u8)| {
            for l in 0..nl {
                if t.1 & (1 << l) != 0 {
                    tree.push((l * hw) as u32 + t.0);
                }
            }
        };
        add_term(&mut tree, terms[0]);
        let mut joined: Vec<(u32, u8)> = vec![terms[0]];
        let mut left: Vec<(u32, u8)> = terms[1..].to_vec();
        while !left.is_empty() {
            // the sources are just terminals for the first connection, and for later ones unless the net may branch anywhere
            let from = if net.branch_anywhere && !state.conns.is_empty() { From::Copper } else { From::Terminals(&joined[..]) };
            let path = match self.find(n, &tree, &left, from, &holes, hard, w) {
                Some(path) => path,
                None => {
                    let at: Vec<Pt> = left.iter().map(|t| self.grid.xy(t.0 as usize)).collect();
                    return Err(format!("{}: no path to {:?}{}", net.key, at, if hard { " with the other nets fixed" } else { " past the fixed copper" }));
                }
            };
            let end = *path.last().unwrap();
            let (el, ec) = (end as usize / hw, end % hw as u32);
            let k = left.iter().position(|t| t.0 == ec && t.1 & (1 << el) != 0).unwrap();
            let reached = left.remove(k);
            let lc = &self.layer_cost[n];
            let mut new_vias = Vec::new();
            let mut dir = NO_DIR;
            let bend = p.options.bend_mm / p.step;
            for pair in path.windows(2) {
                let (a, b) = (pair[0] as usize, pair[1] as usize);
                let (la, ca, lb, cb) = (a / hw, a % hw, b / hw, b % hw);
                if la != lb {
                    if w.via_flag[ca] != FREE && !new_vias.contains(&(ca as u32)) {
                        new_vias.push(ca as u32);
                        state.cost += net.via_penalty_mm / p.step;
                    }
                    dir = NO_DIR;
                } else {
                    let d = dir_of(self.grid.w, ca, cb);
                    state.cost += turn(dir, d).unwrap_or(0.0) * bend;
                    dir = d;
                    let diag = (ca / self.grid.w != cb / self.grid.w) && (ca % self.grid.w != cb % self.grid.w);
                    let len = if diag { std::f64::consts::SQRT_2 } else { 1.0 };
                    state.cost += lc[lb] * len;
                    state.steps += len;
                }
            }
            for v in new_vias {
                w.flag_hole(gw, gh, v, shut);
                state.vias.push(v);
                if net.branch_anywhere {
                    // a via is copper of the net on every layer: a later connection may start from it on any
                    for l in 0..nl {
                        let at = (l * hw) as u32 + v;
                        if self.shared.masks[n][v as usize] & (1 << l) == 0 && !path.contains(&at) {
                            tree.push(at);
                        }
                    }
                } else if p.options.own_hole_free {
                    holes.push(v);
                    holes.sort_unstable();
                }
            }
            w.price_holes(own_hole, net.no_vias);
            if net.branch_anywhere {
                tree.extend_from_slice(&path);
            }
            add_term(&mut tree, reached);
            joined.push(reached);
            state.conns.push(path);
        }
        Ok(state)
    }

    fn place(&mut self, n: usize, mut state: NetState) {
        self.unclaim(n);
        if state.shapes.is_empty() {
            state.shapes = std::mem::take(&mut self.nets[n].shapes);
        }
        self.nets[n] = state;
        self.claim(n);
    }

    /// The cost from every (layer, cell) to the nearest of `goals` when only the fixed copper is in the way: no
    /// other new net, no corner cost. The real cost is never less, so the search can use it to look ahead; in
    /// open ground it is exact and the search walks straight to the goal.
    /// `holes`: cells where the net has a hole to change layer on for nothing, beyond those it has from the start.
    fn floor(&self, n: usize, goals: &[(u32, u8)], holes: &[u32]) -> Arc<Vec<f32>> {
        let key = (n, goals.to_vec(), holes.to_vec());
        if let Some(found) = self.shared.floors.lock().unwrap().get(&key) {
            return found.clone();
        }
        let (hw, nl) = (self.hw, self.nl);
        let mut states = Vec::new();
        for &(cell, bits) in goals {
            for l in 0..nl {
                if bits & (1 << l) != 0 {
                    states.push((l * hw) as u32 + cell);
                }
            }
        }
        let made = Arc::new(self.floor_to(n, &states, holes, f32::INFINITY));
        self.shared.floors.lock().unwrap().insert(key, made.clone());
        made
    }

    /// The look-ahead towards copper of the net that is more than terminals. Where the goal is near (a short
    /// hop), only out to a radius round the copper, and that radius everywhere beyond (never more than the real
    /// cost); otherwise everywhere, and kept for the next search from the same copper.
    fn floor_to_copper(&self, n: usize, sources: &[u32], holes: &[u32], floor: &[f32]) -> Arc<Vec<f32>> {
        let near = sources.iter().map(|&s| floor[s as usize]).fold(f32::INFINITY, f32::min);
        if near <= 400.0 {
            return Arc::new(self.floor_to(n, sources, holes, 2.0 * near + 100.0));
        }
        if let Some(found) = self.shared.copper_floors.lock().unwrap().get(&n).and_then(|kept| kept.iter().find(|k| k.0 == sources && k.1 == holes)) {
            return found.2.clone();
        }
        let made = Arc::new(self.floor_to(n, sources, holes, f32::INFINITY));
        let mut all = self.shared.copper_floors.lock().unwrap();
        let kept = all.entry(n).or_default();
        if kept.len() >= 4 {
            kept.remove(0);
        }
        kept.push((sources.to_vec(), holes.to_vec(), made.clone()));
        made
    }

    /// The same from every (layer, cell) to the nearest of the given states, out to `radius` (and `radius` beyond).
    fn floor_to(&self, n: usize, states: &[u32], holes: &[u32], radius: f32) -> Vec<f32> {
        let net = &self.p.nets[n];
        let (w, hw, nl) = (self.grid.w as i32, self.hw, self.nl);
        let mask = &self.shared.masks[n];
        let open = &self.shared.open[n][..];
        let lc = &self.layer_cost[n];
        let via_cost = (net.via_penalty_mm / self.p.step) as f32;
        let own_hole = if self.p.options.own_hole_free { 0.0 } else { via_cost };
        let off: [isize; 8] = DIRS.map(|(dj, di)| (dj * w + di) as isize);
        let len: [f32; 8] = DIRS.map(|(dj, di)| if dj != 0 && di != 0 { std::f32::consts::SQRT_2 } else { 1.0 });
        let mut free = vec![false; hw];
        for &c in self.fixed_free[n].iter().chain(holes) {
            free[c as usize] = true;
        }
        let mut dist = vec![f32::INFINITY; nl * hw];
        let mut queue = crate::queue::BucketQueue::new();
        for &at in states {
            dist[at as usize] = 0.0;
            queue.push(0, at);
        }
        while let Some((k, u)) = queue.pop() {
            let ui = u as usize;
            let d = dist[ui];
            if (d * 64.0) as u64 != k {
                continue; // a cheaper way here was found after this entry was put in
            }
            if d > radius {
                break;
            }
            let (l, c) = (ui / hw, ui % hw);
            let step = lc[l] as f32;
            let mut bits = open[ui];
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let v = (ui as isize + off[d2]) as usize;
                let nd = d + len[d2] * step;
                if nd < dist[v] {
                    dist[v] = nd;
                    queue.push((nd * 64.0) as u64, v as u32);
                }
            }
            let m = mask[c];
            let cost = if free[c] {
                own_hole
            } else if m & VIA_BIT == 0 {
                via_cost
            } else {
                continue;
            };
            for ll in 0..nl {
                if ll != l && m & (1 << ll) == 0 && d + cost < dist[ll * hw + c] {
                    dist[ll * hw + c] = d + cost;
                    queue.push(((d + cost) * 64.0) as u64, (ll * hw + c) as u32);
                }
            }
        }
        // a little under, so that rounding in single precision can never put it above the real cost
        for v in dist.iter_mut() {
            *v = v.min(radius) * 0.9999;
        }
        dist
    }

    /// Cheapest path from any state of `sources` to any terminal of `goals`. `from`: the terminals the sources
    /// are made of, when they are nothing else (not when the net may branch off anywhere along its copper).
    fn find(&self, n: usize, sources: &[u32], goals: &[(u32, u8)], from: From, holes: &[u32], hard: bool, worker: &mut Worker) -> Option<Vec<u32>> {
        let two = Some(from).filter(|_| self.p.options.bend_mm > 0.0 && self.p.options.two_pass);
        let found = self.search(n, sources, goals, two, holes, hard, worker);
        if std::env::var("PCB_ROUTING_CHECK").is_ok() && two.is_some() {
            let plain = self.search(n, sources, goals, None, holes, hard, worker);
            let (a, b) = (found.as_ref().map(|f| f.1), plain.as_ref().map(|f| f.1));
            let agree = match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() <= 0.25 + 1e-6 * b,
                (None, None) => true,
                _ => false,
            };
            if !agree {
                eprintln!("CHECK FAILED {}: two passes {:?}, one pass {:?} (cost in grid steps)", self.p.nets[n].key, a, b);
            }
        }
        found.map(|f| f.0)
    }

    /// The first pass of a search: the cost from every (layer, cell) to the goal at the costs of this moment,
    /// without corner costs and so without directions (a ninth of the states, and a small record each). It runs
    /// backwards from the goals, steered towards the sources by the fixed look-ahead `back`, until every state
    /// that could lie on a path up to `margin` dearer than the cheapest has its cost. The second pass then has a
    /// look-ahead that knows the other nets, and only widens by the cost of the corners.
    /// How far beyond: by the corner cost of one cheapest corner-free path (followed keeping straight where it
    /// can), since the cheapest path with corners cannot cost more than that one; at most `sweep_margin` corners.
    /// Returns None if no path exists; else the bound below which every state has its cost, and if known a
    /// cost the cheapest path with corners cannot exceed.
    fn sweep(&self, n: usize, goals: &[(u32, u8)], back: &[f32], margin: f64, worker: &mut Worker) -> Option<(f64, f64)> {
        let _t = timed(2);
        let (w, hw, nl) = (self.grid.w as i32, self.hw, self.nl);
        let lc = &self.layer_cost[n];
        let open = &self.shared.open[n][..];
        let vcost = &worker.vcost;
        let off: [isize; 8] = DIRS.map(|(dj, di)| (dj * w + di) as isize);
        let len: [f64; 8] = DIRS.map(|(dj, di)| if dj != 0 && di != 0 { std::f64::consts::SQRT_2 } else { 1.0 });
        let bend = self.p.options.bend_mm / self.p.step;
        let mut upper = f64::INFINITY;
        worker.sweep_gen = worker.sweep_gen.wrapping_add(2);
        if worker.sweep_gen < 2 {
            worker.swept.iter_mut().for_each(|v| v.seen = 0);
            worker.sweep_gen = 2;
        }
        let (open_mark, closed) = (worker.sweep_gen, worker.sweep_gen + 1);
        let (a, q) = (&mut worker.swept, &mut worker.sweep_queue);
        q.clear();
        for &(cell, bits) in goals {
            for l in 0..nl {
                let at = l * hw + cell as usize;
                if bits & (1 << l) != 0 && back[at].is_finite() {
                    a[at].d = 0.0;
                    a[at].seen = open_mark;
                    q.push((back[at] as f64 * 64.0) as u64, at as u32);
                }
            }
        }
        let (mut limit, mut reached, mut last, mut count) = (f64::INFINITY, false, f64::INFINITY, 0u64);
        while let Some((_, u)) = q.pop() {
            let ui = u as usize;
            if a[ui].seen == closed {
                continue;
            }
            let d = a[ui].d as f64;
            if d + back[ui] as f64 > limit {
                last = d + back[ui] as f64;
                break;
            }
            a[ui].seen = closed;
            count += 1;
            let (l, c) = (ui / hw, ui % hw);
            // the look-ahead is 0 at the sources (and where a hole of the net joins them to another layer)
            if !reached && back[ui] == 0.0 {
                reached = true;
                // Follow one cheapest path from here to the goal, keeping straight where that is as cheap, and
                // count its corners.
                let (mut at, mut dir, mut units, mut steps) = (ui, NO_DIR, 0.0f64, 0usize);
                while a[at].d > 0.0 && units.is_finite() {
                    let here = a[at].d as f64;
                    let near = 1e-4 * here + 1e-3;
                    let (la, ca) = (at / hw, at % hw);
                    let mut best: Option<(f64, usize, usize)> = None;
                    let mut bits = open[at];
                    while bits != 0 {
                        let d2 = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        let v = (at as isize + off[d2]) as usize;
                        if a[v].seen == closed && a[v].mult < TAKEN && (a[v].d as f64 + len[d2] * lc[la] * a[v].mult as f64 - here).abs() <= near {
                            if let Some(t) = turn(dir, d2) {
                                if best.map_or(true, |b| t < b.0) {
                                    best = Some((t, v, d2));
                                }
                            }
                        }
                    }
                    if best.is_none() && vcost[ca] != SHUT_COST && vcost[ca] > 0.0 {
                        for ll in 0..nl {
                            let v = ll * hw + ca;
                            if ll != la && a[v].seen == closed && a[v].mult < TAKEN && (a[v].d as f64 + vcost[ca] as f64 - here).abs() <= near {
                                best = Some((0.0, v, NO_DIR));
                            }
                        }
                    }
                    steps += 1;
                    match best {
                        Some((t, v, d2)) if steps < 1_000_000 => {
                            units += t;
                            at = v;
                            dir = d2;
                        }
                        _ => units = f64::INFINITY, // not followed to the end: nothing is known
                    }
                }
                if units.is_finite() {
                    upper = d + units * bend;
                }
                limit = d + (units + 0.5).min(margin) * bend;
            }
            let m = a[ui].mult;
            if m >= TAKEN {
                continue; // no step can end here, so none is taken back from here
            }
            let mut relax = |v: usize, nd: f64| {
                let node = &mut a[v];
                if node.seen == closed || (node.seen == open_mark && nd >= node.d as f64) {
                    return;
                }
                // nothing beyond the bound is wanted (this also drops states the sources cannot reach at all)
                let total = nd + back[v] as f64;
                if total > limit || total == f64::INFINITY {
                    return;
                }
                node.d = nd as f32;
                node.seen = open_mark;
                q.push((total * 64.0) as u64, v as u32);
            };
            let step = lc[l] * m as f64;
            let mut bits = open[ui];
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                relax((ui as isize + off[d2]) as usize, d + len[d2] * step);
            }
            let cost = vcost[c];
            if cost != SHUT_COST {
                for ll in 0..nl {
                    if ll != l {
                        relax(ll * hw + c, d + cost as f64);
                    }
                }
            }
        }
        self.swept.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        if reached { Some((last.min(limit), upper)) } else { None }
    }

    fn search(&self, n: usize, sources: &[u32], goals: &[(u32, u8)], from: Option<From>, holes: &[u32], hard: bool, worker: &mut Worker) -> Option<(Vec<u32>, f64)> {
        let floor = self.floor(n, goals, holes);
        let floor = &floor[..];
        let haste = if hard { 1.0 } else { self.p.options.haste.max(1.0) };

        // With a first pass: the look-ahead is its cost where it got to, and elsewhere the most that is certain
        // (the fixed look-ahead, or that the state lies beyond the bound the first pass stopped at).
        let back: Option<Arc<Vec<f32>>> = from.map(|from| match from {
            From::Terminals(from) => self.floor(n, from, holes),
            From::Copper => {
                let _t = timed(1);
                self.floor_to_copper(n, sources, holes, floor)
            }
        });
        let (bound, upper) = match &back {
            Some(back) => {
                let margin = self.p.options.sweep_margin;
                let (bound, upper) = self.sweep(n, goals, &back[..], margin, worker)?;
                (Some(bound), upper)
            }
            None => (None, f64::INFINITY),
        };
        let _t = timed(3);
        let back = back.as_ref().map(|b| &b[..]);
        let swept = &worker.swept[..];
        let swept_closed = worker.sweep_gen + 1;
        let look = |at: usize| -> f64 {
            match (bound, back) {
                (Some(bound), Some(back)) => {
                    if swept[at].seen == swept_closed {
                        // kept in single precision: a little under, so that rounding can never put it above the real cost
                        swept[at].d as f64 * 0.9999
                    } else {
                        (floor[at] as f64).max(bound - back[at] as f64)
                    }
                }
                _ => floor[at] as f64,
            }
        };
        let (w, hw, nl) = (self.grid.w as i32, self.hw, self.nl);
        let lc = &self.layer_cost[n];
        let bend = self.p.options.bend_mm / self.p.step;
        let vcost = &worker.vcost;
        let mut count = 0u64;
        let s = &mut worker.search;
        s.gen = s.gen.wrapping_add(2);
        if s.gen < 2 {
            s.node.iter_mut().for_each(|v| v.seen = 0);
            s.gen = 2;
        }
        let (open_mark, closed) = (s.gen, s.gen + 1);
        s.heap.clear();
        // no state is wanted that cannot be on a path as cheap as one already known (with room for rounding)
        let cut = if haste == 1.0 { upper * 1.0005 + 1.0 } else { f64::INFINITY };
        let open = &self.shared.open[n][..];
        let (corner, allowed) = turns(bend);
        let off: [isize; 8] = DIRS.map(|(dj, di)| (dj * w + di) as isize);
        let len: [f64; 8] = DIRS.map(|(dj, di)| if dj != 0 && di != 0 { std::f64::consts::SQRT_2 } else { 1.0 });
        // A state is ((layer, cell), direction of arrival), the nine directions of a cell side by side in memory.
        for &src in sources {
            let ahead = look(src as usize);
            let src = src * 9 + NO_DIR as u32;
            if s.node[src as usize].seen != open_mark {
                s.node[src as usize].seen = open_mark;
                s.node[src as usize].g = 0.0;
                s.node[src as usize].prev = START;
                if ahead.is_finite() {
                    s.heap.push(key(haste * ahead), src);
                }
            }
        }
        let mut found = NONE;
        while let Some((_, u)) = s.heap.pop() {
            let ui = u as usize;
            if s.node[ui].seen == closed {
                continue;
            }
            s.node[ui].seen = closed;
            count += 1;
            let (dir, at) = (ui % 9, ui / 9);
            let ahead_here = look(at);
            let (l, c) = (at / hw, at % hw);
            // the look-ahead is 0 at a goal (and at little else), so this test is rarely made in full
            if ahead_here == 0.0 && goals.iter().any(|g| g.0 as usize == c && g.1 & (1 << l) != 0) {
                found = u;
                break;
            }
            let gu = s.node[ui].g as f64;
            let step_cost = lc[l];
            let relax = |v: usize, cost: f64, how: u8, s: &mut Search| {
                let nd = gu + cost;
                let node = &mut s.node[v];
                // a closed state is opened again if a cheaper way to it turns up: that can happen only where the
                // look-ahead changes from the first pass's cost to the bound beyond it
                if if node.seen == closed { nd * 1.000001 + 1e-6 < node.g as f64 } else { node.seen != open_mark || nd < node.g as f64 } {
                    let ahead = look(v / 9);
                    if ahead.is_finite() && nd + ahead <= cut {
                        node.seen = open_mark;
                        node.g = nd as f32;
                        node.prev = how;
                        s.heap.push(key(nd + haste * ahead), v as u32);
                    }
                }
            };
            let mut bits = open[at] & allowed[dir];
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let v = (at as isize + off[d2]) as usize;
                let m2 = swept[v].mult;
                if m2 >= TAKEN {
                    continue;
                }
                relax(v * 9 + d2, corner[dir][d2] + len[d2] * step_cost * m2 as f64, dir as u8, s);
            }
            let cost = vcost[c];
            if cost == SHUT_COST {
                continue;
            }
            for ll in 0..nl {
                if ll != l && swept[ll * hw + c].mult < TAKEN {
                    relax((ll * hw + c) * 9 + NO_DIR, cost as f64, VIA + (9 * l + dir) as u8, s);
                }
            }
        }
        self.expanded.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        if std::env::var("PCB_ROUTING_COUNT").is_ok() {
            let mut cells = 0u64;
            for at in 0..nl * hw {
                if s.node[at * 9..at * 9 + 9].iter().any(|x| x.seen == closed) {
                    cells += 1;
                }
            }
            eprintln!("find {}: expanded {count}, distinct (layer, cell) {cells}, path {}", self.p.nets[n].key, if found == NONE { 0 } else { 1 });
        }
        if found == NONE {
            return None;
        }
        let mut path = vec![found];
        loop {
            let ui = *path.last().unwrap() as usize;
            let (how, dir, at) = (s.node[ui].prev, ui % 9, ui / 9);
            if how == START {
                break;
            }
            path.push(if dir != NO_DIR {
                ((at as isize - off[dir]) as usize * 9) as u32 + how as u32
            } else {
                let (from, how) = (((how - VIA) / 9) as usize, (how - VIA) % 9);
                ((from * hw + at % hw) * 9) as u32 + how as u32
            });
        }
        path.reverse();
        // the direction is only needed during the search
        let cost = s.node[found as usize].g as f64;
        Some((path.into_iter().map(|u| u / 9).collect(), cost))
    }

    /// The cells a net's copper claims, for each size of track and via another net may have.
    /// The cells whose centres lie within `r` of the segment from `a` to `b`.
    fn near(&self, a: Pt, b: Pt, r: f64) -> Vec<usize> {
        let g = &self.grid;
        let lo = |v: f64, o: f64| (((v - r - o) / g.step).floor() as i64).max(0);
        let hi = |v: f64, o: f64, n: usize| (((v + r - o) / g.step).ceil() as i64).min(n as i64 - 1);
        let (i0, i1) = (lo(a[0].min(b[0]), g.x0), hi(a[0].max(b[0]), g.x0, g.w));
        let (j0, j1) = (lo(a[1].min(b[1]), g.y0), hi(a[1].max(b[1]), g.y0, g.h));
        let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = vx * vx + vy * vy;
        let mut out = Vec::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (x, y) = (g.x0 + i as f64 * g.step, g.y0 + j as f64 * g.step);
                let t = if l2 > 0.0 { (((x - a[0]) * vx + (y - a[1]) * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let (dx, dy) = (x - a[0] - t * vx, y - a[1] - t * vy);
                if dx * dx + dy * dy <= (r + EPS) * (r + EPS) {
                    out.push(j as usize * g.w + i as usize);
                }
            }
        }
        out
    }

    fn claim(&mut self, n: usize) {
        let _t = timed(4);
        let net = &self.p.nets[n];
        let rules = &self.p.rules;
        let quiet = if rules.quiet_clearance_mm > 0.0 { rules.quiet_clearance_mm } else { rules.new_clearance_mm };
        let (plain, mine) = (rules.new_clearance_mm, net.quiet);
        let shapes = self.nets[n].shapes.clone();
        let (w, h, hw, nl) = (self.grid.w as i32, self.grid.h as i32, self.hw, self.nl);
        let (half, via_r) = (net.width / 2.0, net.via[0] / 2.0);
        let conns = self.nets[n].conns.clone();
        let vias = self.nets[n].vias.clone();
        let mut claims_t = Vec::new();
        for q in 0..self.tr.len() {
            let c = if self.tr[q].1 != mine { quiet } else { plain };
            let track = self.disc(half + c + self.tr[q].0);
            let via = self.disc(via_r + c + self.tr[q].0);
            let near: Vec<(usize, Vec<usize>)> = shapes.iter().map(|&(l, a, b, hw_)| (l, self.near(a, b, hw_ + c + self.tr[q].0))).collect();
            self.mark_gen += 1;
            let gen = self.mark_gen;
            let mut out: Vec<u32> = Vec::new();
            let mut put = |mark: &mut Vec<u32>, layer: usize, cell: usize, disc: &[(i32, i32)]| {
                let (j, i) = (cell as i32 / w, cell as i32 % w);
                for &(dj, di) in disc {
                    let (jj, ii) = (j + dj, i + di);
                    if jj >= 0 && ii >= 0 && jj < h && ii < w {
                        let idx = layer * hw + (jj * w + ii) as usize;
                        if mark[idx] != gen {
                            mark[idx] = gen;
                            out.push(idx as u32);
                        }
                    }
                }
            };
            for conn in &conns {
                for &s in conn {
                    put(&mut self.mark, s as usize / hw, s as usize % hw, &track);
                }
            }
            for &v in &vias {
                for l in 0..nl {
                    put(&mut self.mark, l, v as usize, &via);
                }
            }
            for (l, cells) in &near {
                for &cell in cells {
                    put(&mut self.mark, *l, cell, &[(0, 0)]);
                }
            }
            out.sort_unstable();
            for &idx in &out {
                self.occ_t[q][idx as usize] += 1;
                if self.frozen[n] {
                    self.fro_t[q][idx as usize] += 1;
                }
            }
            claims_t.push(out);
        }
        let mut claims_v = Vec::new();
        for q in 0..self.vr.len() {
            let c = if self.vr[q].1 != mine { quiet } else { plain };
            let track = self.disc(half + c + self.vr[q].0);
            let via = self.disc(via_r + c + self.vr[q].0);
            let near: Vec<Vec<usize>> = shapes.iter().map(|&(_, a, b, hw_)| self.near(a, b, hw_ + c + self.vr[q].0)).collect();
            self.mark_gen += 1;
            let gen = self.mark_gen;
            let mut out: Vec<u32> = Vec::new();
            let mut put = |mark: &mut Vec<u32>, cell: usize, disc: &[(i32, i32)]| {
                let (j, i) = (cell as i32 / w, cell as i32 % w);
                for &(dj, di) in disc {
                    let (jj, ii) = (j + dj, i + di);
                    if jj >= 0 && ii >= 0 && jj < h && ii < w {
                        let idx = (jj * w + ii) as usize;
                        if mark[idx] != gen {
                            mark[idx] = gen;
                            out.push(idx as u32);
                        }
                    }
                }
            };
            for conn in &conns {
                for &s in conn {
                    put(&mut self.mark, s as usize % hw, &track);
                }
            }
            for &v in &vias {
                put(&mut self.mark, v as usize, &via);
            }
            for cells in &near {
                for &cell in cells {
                    put(&mut self.mark, cell, &[(0, 0)]);
                }
            }
            out.sort_unstable();
            for &idx in &out {
                self.occ_v[q][idx as usize] += 1;
                if self.frozen[n] {
                    self.fro_v[q][idx as usize] += 1;
                }
            }
            claims_v.push(out);
        }
        self.nets[n].claims_t = claims_t;
        self.nets[n].claims_v = claims_v;
    }

    fn unclaim(&mut self, n: usize) {
        for (q, list) in self.nets[n].claims_t.iter().enumerate() {
            for &idx in list {
                self.occ_t[q][idx as usize] -= 1;
                if self.frozen[n] {
                    self.fro_t[q][idx as usize] -= 1;
                }
            }
        }
        for (q, list) in self.nets[n].claims_v.iter().enumerate() {
            for &idx in list {
                self.occ_v[q][idx as usize] -= 1;
                if self.frozen[n] {
                    self.fro_v[q][idx as usize] -= 1;
                }
            }
        }
        self.nets[n].claims_t.clear();
        self.nets[n].claims_v.clear();
    }

    /// Where a laid net stands in ground claimed by another (its own claim counts once everywhere near it).
    fn trouble(&mut self, n: usize) -> Trouble {
        let siblings: Vec<usize> = self.siblings[n].iter().copied().filter(|&m| !self.nets[m].claims_t.is_empty()).collect();
        for &m in &siblings {
            self.unclaim(m);
        }
        let t = self.trouble_alone(n);
        for &m in &siblings {
            self.claim(m);
        }
        t
    }

    fn trouble_alone(&self, n: usize) -> Trouble {
        let _t = timed(5);
        let w = self.grid.w;
        let occ_t = &self.occ_t[self.tclass[n]];
        let occ_v = &self.occ_v[self.vclass[n]];
        let st = &self.nets[n];
        let mut t = Trouble { track: Vec::new(), via: Vec::new(), own: Vec::new() };
        for conn in &st.conns {
            // The middle of a diagonal step is nearer to a neighbour than its two ends by under 0.002 mm at these
            // sizes, well inside the 0.02 mm the clearance carries for the grid; the ends are what is tested.
            for &s in conn {
                if occ_t[s as usize] > 1 {
                    t.track.push(s);
                }
            }
        }
        for &v in &st.vias {
            if occ_v[v as usize] > 1 {
                t.via.push(v);
            }
        }
        let need = self.p.rules.hole_centre_mm / self.grid.step;
        for (a, &va) in st.vias.iter().enumerate() {
            for &vb in &st.vias[a + 1..] {
                let (dj, di) = ((va as usize / w) as f64 - (vb as usize / w) as f64, (va as usize % w) as f64 - (vb as usize % w) as f64);
                if (dj * dj + di * di).sqrt() < need - 1e-9 {
                    t.own.push(va);
                    t.own.push(vb);
                }
            }
        }
        t
    }

    /// Route everything. `order` is the order of the first round and of every later one.
    /// Rounds of laying the nets in conflict again until none is. Nets held fixed are not moved; to the others
    /// their ground is closed. Ok(true): settled.
    fn negotiate(&mut self, order: &[usize], first: Vec<usize>, limit: usize, rounds: &mut usize, variant: usize) -> Result<bool, String> {
        let o = self.p.options.clone();
        let mut present = o.present_start;
        let mut todo: Vec<usize> = first.into_iter().filter(|&n| !self.frozen[n]).collect();
        // Stalls: two nets squeezed into a gap with room for one stay in each other's way round after round,
        // shifting by a cell each time, while every other net, no longer in conflict, is no longer moved. The
        // standing cost, added only where the centre lines stood, then grows too slowly to drive one of them
        // out. So once the count of nets in conflict has not fallen for `stall_after` rounds, the standing cost
        // is added over the whole width a track claims round each place of conflict, and more each further
        // round; one of the nets then gives up the gap and pushes a third net instead, which is laid again.
        // After `patience` such rounds the attempt is given up.
        let (mut fewest, mut since) = (usize::MAX, 0usize);
        for _ in 0..limit {
            *rounds += 1;
            for &n in &todo {
                let state = self.route_net(n, present, false)?;
                self.place(n, state);
            }
            let mut next = Vec::new();
            for &n in order {
                let t = self.trouble(n);
                if t.any() && !self.frozen[n] {
                    next.push(n);
                }
                for &s in &t.track {
                    self.hist[s as usize] += o.history_step as f32;
                }
                if o.stall_after > 0 && since >= o.stall_after && !t.track.is_empty() {
                    let disc = self.disc(self.p.nets[n].width + self.p.rules.new_clearance_mm);
                    let (w, h, hw) = (self.grid.w as i32, self.grid.h as i32, self.hw);
                    let add = (o.history_step * (since + 1 - o.stall_after) as f64) as f32;
                    self.mark_gen += 1;
                    for &s in &t.track {
                        let (l, c) = (s as usize / hw, s as usize % hw);
                        let (j, i) = (c as i32 / w, c as i32 % w);
                        for &(dj, di) in &disc {
                            let (jj, ii) = (j + dj, i + di);
                            if jj >= 0 && ii >= 0 && jj < h && ii < w {
                                let k = l * hw + (jj * w + ii) as usize;
                                if self.mark[k] != self.mark_gen {
                                    self.mark[k] = self.mark_gen;
                                    self.hist[k] += add;
                                }
                            }
                        }
                    }
                }
                for &v in &t.via {
                    self.hist_v[v as usize] += o.history_step as f32;
                }
                for &v in &t.own {
                    self.hist_v[v as usize] += 4.0 * o.history_step as f32;
                }
            }
            // A net that the others have pushed past its wanted length gives way less from now on, so that nets
            // with length to spare make the detour instead.
            for &n in order {
                if self.excess(n) > 0.0 {
                    self.give[n] = (self.give[n] * o.stiffen).max(0.02);
                }
            }
            if std::env::var("PCB_ROUTING_TRACE").is_ok() {
                eprintln!("attempt {variant} round {}: laid {} nets, {} still in conflict, present {present:.2}, states expanded so far {} (first pass {})", *rounds, todo.len(), next.len(), self.expanded.load(std::sync::atomic::Ordering::Relaxed), self.swept.load(std::sync::atomic::Ordering::Relaxed));
            }
            if next.is_empty() {
                return Ok(true);
            }
            if next.len() < fewest {
                fewest = next.len();
                since = 0;
            } else {
                since += 1;
            }
            todo = next;
            present = (present * o.present_growth).min(o.present_max);
            if o.patience > 0 && since >= o.patience {
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// By how much a laid net is longer than wanted, in mm.
    fn excess(&self, n: usize) -> f64 {
        self.p.nets[n].max_mm.map_or(0.0, |max| (self.nets[n].steps * self.p.step - max).max(0.0))
    }

    fn hold(&mut self, n: usize, fixed: bool) {
        let state = std::mem::take(&mut self.nets[n]);
        self.nets[n] = state.clone();
        self.unclaim(n);
        self.frozen[n] = fixed;
        self.nets[n] = NetState { claims_t: Vec::new(), claims_v: Vec::new(), ..state };
        self.claim(n);
    }

    /// Route everything. `order` is the order in which the nets are laid within a round.
    pub fn solve(&mut self, order: &[usize], variant: usize) -> Solution {
        let o = self.p.options.clone();
        let fail = |status: &str, message: String, iterations: usize| Solution { status: status.into(), iterations, variant, score: f64::INFINITY, nets: Vec::new(), conflicts: Vec::new(), message, attempts: Vec::new() };
        if !o.negotiate {
            for &n in order {
                match self.route_net(n, 0.0, true) {
                    Ok(state) => self.place(n, state),
                    Err(e) => return fail("unroutable", e, 1),
                }
            }
            return self.report("routed", 1, variant, String::new());
        }
        let mut rounds = 0;
        let every = order;
        let order: Vec<usize> = every.iter().copied().filter(|&n| !self.p.nets[n].last).collect();
        let order = &order[..];
        let settled = match self.negotiate(order, order.to_vec(), o.max_iterations, &mut rounds, variant) {
            Ok(settled) => settled,
            Err(e) => return fail("unroutable", e, rounds),
        };
        if !settled {
            return self.report("conflicts", rounds, variant, String::new());
        }
        // Repair of lengths. The rounds end as soon as no net is in conflict, whoever made the detours. So: take
        // the net furthest over its wanted length, lay it on its own cheapest path as if the others were not
        // there, hold it fixed, and let the others settle round it. Keep that only if the sum of the excesses
        // over the wanted lengths went down; otherwise put everything back and try the next net.
        let mut tried = vec![false; self.p.nets.len()];
        for _ in 0..o.repairs {
            let total: f64 = order.iter().map(|&n| self.excess(n)).sum();
            let worst = order.iter().copied().filter(|&n| !tried[n] && !self.frozen[n] && self.excess(n) > 0.0).max_by(|&a, &b| self.excess(a).partial_cmp(&self.excess(b)).unwrap().then(b.cmp(&a)));
            let Some(n) = worst else { break };
            tried[n] = true;
            let saved = (self.nets.clone(), self.occ_t.clone(), self.occ_v.clone(), self.fro_t.clone(), self.fro_v.clone(), self.hist.clone(), self.hist_v.clone(), self.give.clone(), self.frozen.clone());
            let before = self.give[n];
            self.give[n] = 0.0;
            let alone = self.route_net(n, 0.0, false);
            self.give[n] = before;
            let mut kept = false;
            if let Ok(state) = alone {
                self.place(n, state);
                if self.excess(n) < total {
                    self.hold(n, true);
                    self.hist.iter_mut().for_each(|v| *v = 0.0);
                    self.hist_v.iter_mut().for_each(|v| *v = 0.0);
                    let first: Vec<usize> = order.iter().copied().filter(|&m| self.trouble(m).any()).collect();
                    if let Ok(true) = self.negotiate(order, first, o.max_iterations, &mut rounds, variant) {
                        let now: f64 = order.iter().map(|&m| self.excess(m)).sum();
                        kept = now < total - 1e-6;
                        if std::env::var("PCB_ROUTING_TRACE").is_ok() {
                            eprintln!("attempt {variant} repair of {}: excess {total:.1} -> {now:.1} mm, {}", self.p.nets[n].key, if kept { "kept" } else { "put back" });
                        }
                    }
                }
            }
            if !kept {
                (self.nets, self.occ_t, self.occ_v, self.fro_t, self.fro_v, self.hist, self.hist_v, self.give, self.frozen) = saved;
            }
        }
        for n in 0..self.p.nets.len() {
            if self.frozen[n] {
                self.hold(n, false);
            }
        }
        for _ in 0..o.refine_passes {
            let mut better = false;
            for &n in order {
                if let Ok(state) = self.route_net(n, 0.0, true) {
                    if state.cost < self.nets[n].cost - 1e-6 {
                        let old = self.nets[n].clone();
                        self.place(n, state);
                        if self.trouble(n).any() {
                            self.place(n, old);
                        } else {
                            better = true;
                        }
                    }
                }
            }
            if !better {
                break;
            }
        }
        // Pairs: each settled trunk becomes two legs, and the stubs that join the legs to their pads are laid first
        // of the nets kept for last.
        let mut stubs: Vec<usize> = Vec::new();
        let mut fours: Vec<Vec<usize>> = Vec::new();
        for k in 0..self.p.pairs.len() {
            match self.split(k) {
                Ok(four) => {
                    stubs.extend_from_slice(&four);
                    fours.push(four);
                }
                Err(e) => return fail("unroutable", e, rounds),
            }
        }
        // The four stubs of a pair are laid one after the other, each fixed for the next; which goes first can
        // decide whether all four fit, so the orders are tried in turn until one does.
        for (k, four) in fours.iter().enumerate() {
            let mut order = four.clone();
            let mut tried = 0;
            let mut last_error = String::new();
            let done = loop {
                let mut ok = true;
                for &n in &order {
                    match self.route_net(n, 0.0, true) {
                        Ok(state) => self.place(n, state),
                        Err(e) => {
                            last_error = e;
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    break true;
                }
                for &n in four {
                    self.place(n, NetState::default()); // the leg an entry holds stays with it
                }
                tried += 1;
                if tried == 24 {
                    break false;
                }
                // the next of the 24 orders
                let mut left: Vec<usize> = four.clone();
                let mut code = tried;
                order.clear();
                for size in (1..=4).rev() {
                    order.push(left.remove(code % size));
                    code /= size;
                }
            };
            if !done {
                // what was laid is given back with the failure, to show where the stubs were shut in
                let mut partial = self.report("unroutable", rounds, variant, format!("pair {}: its stubs cannot all be laid in any order; last: {last_error}", self.p.pairs[k].key));
                partial.status = "unroutable".into();
                partial.score = f64::INFINITY;
                return partial;
            }
        }
        let rest: Vec<usize> = every.iter().copied().filter(|n| self.p.nets[*n].last && !stubs.contains(n)).collect();
        for n in rest {
            match self.route_net(n, 0.0, true) {
                Ok(state) => self.place(n, state),
                Err(e) => return fail("unroutable", format!("with the other nets settled: {e}"), rounds),
            }
        }
        self.report("routed", rounds, variant, String::new())
    }

    fn index(&self, key: &str) -> Result<usize, String> {
        self.p.nets.iter().position(|n| n.key == key).ok_or_else(|| format!("no net entry with key {key}"))
    }

    /// The centre line of a laid connection that stays on one layer, as points.
    fn line(&self, conn: &[u32]) -> Vec<Pt> {
        let mut run = Vec::new();
        for &state in conn {
            push(&mut run, self.grid.xy(state as usize % self.hw));
        }
        run
    }

    /// Split the settled trunk of pair `k` into its two legs. The trunk's claim (one wide track) is given up
    /// and each leg is claimed as the shape it is, held by the stub at its first end; every stub gets the end
    /// of its leg as second terminal. Returns the four stub entries, to be laid next.
    fn split(&mut self, k: usize) -> Result<Vec<usize>, String> {
        let pair = &self.p.pairs[k];
        let trunk = self.index(&pair.trunk)?;
        let stubs = [[self.index(&pair.stubs[0][0])?, self.index(&pair.stubs[0][1])?], [self.index(&pair.stubs[1][0])?, self.index(&pair.stubs[1][1])?]];
        let conn = self.nets[trunk].conns.first().ok_or_else(|| format!("{}: trunk not laid", pair.key))?.clone();
        let layer = conn[0] as usize / self.hw;
        if conn.iter().any(|&c| c as usize / self.hw != layer) {
            return Err(format!("{}: the trunk changes layer", pair.key));
        }
        let centre = self.line(&conn);
        if centre.len() < 2 {
            return Err(format!("{}: the trunk has no length", pair.key));
        }
        let unit = |a: Pt, b: Pt| {
            let (x, y) = (b[0] - a[0], b[1] - a[1]);
            let l = (x * x + y * y).sqrt();
            [x / l, y / l]
        };
        // the first net of the pair takes the side its pad at the first end lies on
        let first = unit(centre[0], centre[1]);
        let pad = |n: usize| self.p.nets[n].terminals[0].xy;
        let side_of = |q: Pt| first[0] * (q[1] - centre[0][1]) - first[1] * (q[0] - centre[0][0]);
        let left = if side_of(pad(stubs[0][0])).abs() > 1e-9 { side_of(pad(stubs[0][0])) > 0.0 } else { side_of(pad(stubs[0][1])) < 0.0 };
        let d = pair.pitch / 2.0;
        let mut lines: [Vec<Pt>; 2] = [Vec::new(), Vec::new()];
        let mut outward: [[Pt; 2]; 2] = [[[0.0; 2]; 2]; 2]; // [leg][end]: unit vector from the centre line to the leg
        for leg in 0..2 {
            let sign = if (leg == 0) == left { 1.0 } else { -1.0 };
            for i in 0..centre.len() {
                let normal = |a: Pt, b: Pt| {
                    let u = unit(a, b);
                    [-u[1] * sign, u[0] * sign]
                };
                let m = if i == 0 {
                    normal(centre[0], centre[1])
                } else if i == centre.len() - 1 {
                    normal(centre[i - 1], centre[i])
                } else {
                    // at a corner the two offset lines meet on the bisector
                    let (a, b) = (normal(centre[i - 1], centre[i]), normal(centre[i], centre[i + 1]));
                    let f = 1.0 + a[0] * b[0] + a[1] * b[1];
                    [(a[0] + b[0]) / f, (a[1] + b[1]) / f]
                };
                if i == 0 {
                    outward[leg][0] = m;
                }
                if i == centre.len() - 1 {
                    outward[leg][1] = m;
                }
                lines[leg].push([round6(centre[i][0] + d * m[0]), round6(centre[i][1] + d * m[1])]);
            }
        }
        self.unclaim(trunk);
        for leg in 0..2 {
            let holder = stubs[0][leg];
            self.unclaim(holder);
            self.nets[holder].shapes = lines[leg].windows(2).map(|w| (layer, w[0], w[1], pair.leg_width / 2.0)).collect();
            self.claim(holder);
            for end in 0..2 {
                let at = if end == 0 { lines[leg][0] } else { *lines[leg].last().unwrap() };
                // the stub arrives on the cell just outside the leg's end, so that nothing of it comes nearer the other leg
                let out = outward[leg][end];
                let cell = self.grid.cell([at[0] + 0.03 * out[0], at[1] + 0.03 * out[1]]).ok_or_else(|| format!("{}: a leg ends outside the window", pair.key))? as u32;
                let n = stubs[end][leg];
                self.extra_terms[n] = vec![(cell, 1 << layer)];
                self.exact[n].insert(cell, at);
            }
        }
        self.legs[k] = Some((layer, lines));
        Ok(vec![stubs[0][0], stubs[0][1], stubs[1][0], stubs[1][1]])
    }

    fn report(&mut self, status: &str, iterations: usize, variant: usize, message: String) -> Solution {
        let (hw, p) = (self.hw, self.p);
        let mut nets = Vec::new();
        let mut conflicts = Vec::new();
        let mut score = 0.0;
        // entries that are part of a pair are not reported by themselves: (pair, leg) for a stub, the pair for a trunk
        let mut part: Vec<Option<(usize, Option<usize>)>> = vec![None; p.nets.len()];
        for (k, pair) in p.pairs.iter().enumerate() {
            if let Ok(t) = self.index(&pair.trunk) {
                part[t] = Some((k, None));
            }
            for end in 0..2 {
                for leg in 0..2 {
                    if let Ok(n) = self.index(&pair.stubs[end][leg]) {
                        part[n] = Some((k, Some(leg)));
                    }
                }
            }
        }
        let mut pieces: Vec<[(Vec<Track>, Vec<Pt>); 2]> = p.pairs.iter().map(|_| [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())]).collect();
        for (n, net) in p.nets.iter().enumerate() {
            let st = &self.nets[n];
            let fixed = &self.fixed_free[n];
            let exact = &self.exact[n];
            let point = |cell: u32| exact.get(&cell).copied();
            let mut tracks: Vec<Track> = Vec::new();
            for conn in &st.conns {
                let first = conn[0];
                let mut layer = first as usize / hw;
                let mut run: Vec<Pt> = Vec::new();
                if let Some(e) = point(first % hw as u32) {
                    run.push(e);
                }
                push(&mut run, self.grid.xy(first as usize % hw));
                for pair in conn.windows(2) {
                    let (a, b) = (pair[0] as usize, pair[1] as usize);
                    if a / hw != b / hw {
                        let cell = (a % hw) as u32;
                        let at = if fixed.contains(&cell) { point(cell).unwrap_or(self.grid.xy(cell as usize)) } else { self.grid.xy(cell as usize) };
                        push(&mut run, at);
                        if run.len() > 1 {
                            tracks.push(Track { layer: p.layers[layer].clone(), points: std::mem::take(&mut run) });
                        }
                        run = vec![at];
                        layer = b / hw;
                    } else {
                        push(&mut run, self.grid.xy(b % hw));
                    }
                }
                let last = *conn.last().unwrap();
                if let Some(e) = point(last % hw as u32) {
                    push(&mut run, e);
                }
                if run.len() > 1 {
                    tracks.push(Track { layer: p.layers[layer].clone(), points: run });
                }
            }
            let length: f64 = tracks.iter().map(|t| t.points.windows(2).map(|s| ((s[0][0] - s[1][0]).powi(2) + (s[0][1] - s[1][1]).powi(2)).sqrt()).sum::<f64>()).sum();
            // lower is better: weighted length, a via as 1 mm, and each mm over the wanted length as 100 mm
            score += net.weight * (length + 100.0 * net.max_mm.map_or(0.0, |m| (length - m).max(0.0))) + st.vias.len() as f64;
            // a trunk that has been split has given up its claim: its legs stand for it
            let t = if matches!(part[n], Some((k, None)) if self.legs[k].is_some()) { Trouble { track: Vec::new(), via: Vec::new(), own: Vec::new() } } else { self.trouble(n) };
            let st = &self.nets[n];
            let others = |q_list: &dyn Fn(usize) -> bool| -> Vec<String> { (0..p.nets.len()).filter(|&m| m != n && q_list(m)).map(|m| p.nets[m].key.clone()).collect() };
            let mut seen_with: Vec<Vec<String>> = Vec::new();
            for &s in &t.track {
                let with = others(&|m| self.nets[m].claims_t.get(self.tclass[n]).map_or(false, |c| c.binary_search(&s).is_ok()));
                if !seen_with.contains(&with) {
                    seen_with.push(with.clone());
                    conflicts.push(Conflict { kind: "track".into(), key: net.key.clone(), with, layer: Some(p.layers[s as usize / hw].clone()), xy: self.grid.xy(s as usize % hw) });
                }
            }
            for &v in &t.via {
                let with = others(&|m| self.nets[m].claims_v.get(self.vclass[n]).map_or(false, |c| c.binary_search(&v).is_ok()));
                conflicts.push(Conflict { kind: "via".into(), key: net.key.clone(), with, layer: None, xy: self.grid.xy(v as usize) });
            }
            for &v in &t.own {
                conflicts.push(Conflict { kind: "two vias of the net too close".into(), key: net.key.clone(), with: Vec::new(), layer: None, xy: self.grid.xy(v as usize) });
            }
            let vias: Vec<Pt> = st.vias.iter().map(|&v| self.grid.xy(v as usize)).collect();
            match part[n] {
                None => nets.push(NetResult { key: net.key.clone(), net: net.net.clone(), length_mm: round6(length), tracks, vias }),
                Some((k, Some(leg))) => {
                    pieces[k][leg].0.extend(tracks);
                    pieces[k][leg].1.extend(vias);
                }
                Some((k, None)) if self.legs[k].is_none() => nets.push(NetResult { key: net.key.clone(), net: net.net.clone(), length_mm: round6(length), tracks, vias }),
                Some(_) => {}
            }
        }
        // each net of a pair: its leg and its two stubs
        for (k, pair) in p.pairs.iter().enumerate() {
            if let Some((layer, lines)) = &self.legs[k] {
                for leg in 0..2 {
                    let mut tracks = vec![Track { layer: p.layers[*layer].clone(), points: lines[leg].clone() }];
                    tracks.append(&mut pieces[k][leg].0);
                    let length: f64 = tracks.iter().map(|t| t.points.windows(2).map(|s| ((s[0][0] - s[1][0]).powi(2) + (s[0][1] - s[1][1]).powi(2)).sqrt()).sum::<f64>()).sum();
                    nets.push(NetResult { key: format!("{}/{}", pair.key, pair.legs[leg]), net: pair.legs[leg].clone(), length_mm: round6(length), tracks, vias: std::mem::take(&mut pieces[k][leg].1) });
                }
            }
        }
        let status = if conflicts.is_empty() { status } else { "conflicts" };
        Solution { status: status.into(), iterations, variant, score: round6(score), nets, conflicts, message, attempts: Vec::new() }
    }
}

/// Append a point, dropping repeats and the middle point of three in a straight line (same direction).
fn push(run: &mut Vec<Pt>, p: Pt) {
    if let Some(last) = run.last() {
        if (last[0] - p[0]).abs() < 1e-9 && (last[1] - p[1]).abs() < 1e-9 {
            return;
        }
    }
    run.push(p);
    while run.len() >= 3 {
        let (a, b, c) = (run[run.len() - 3], run[run.len() - 2], run[run.len() - 1]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        if (u[0] * v[1] - u[1] * v[0]).abs() < 1e-7 && u[0] * v[0] + u[1] * v[1] > 0.0 {
            let n = run.len();
            run.remove(n - 2);
        } else {
            break;
        }
    }
}

/// Solve a problem: one attempt per variant (the first in the given order, the others shuffled), side by side;
/// the best complete one is returned. The same problem always gives the same answer.
pub fn solve(p: &Problem) -> Solution {
    use rayon::prelude::*;
    let variants = p.options.variants.max(1);
    let orders: Vec<Vec<usize>> = (0..variants)
        .map(|v| {
            let mut order: Vec<usize> = (0..p.nets.len()).collect();
            let mut seed = 0x9E3779B97F4A7C15u64.wrapping_mul(v as u64 + 1);
            if v > 0 {
                for k in (1..order.len()).rev() {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    order.swap(k, ((seed >> 33) as usize) % (k + 1));
                }
            }
            order
        })
        .collect();
    let shared = Shared::new(p);
    let run = |(v, order): (usize, &Vec<usize>)| Engine::with_shared(p, shared.clone()).solve(order, v);
    // PCB_ROUTING_ONLY=v runs attempt v alone (to look at one attempt of a larger run)
    let only: Option<usize> = std::env::var("PCB_ROUTING_ONLY").ok().and_then(|v| v.parse().ok());
    let results: Vec<Solution> = match only {
        Some(v) => vec![run((v, &orders[v]))],
        None if variants == 1 => vec![run((0, &orders[0]))],
        None => orders.par_iter().enumerate().map(run).collect(),
    };
    let rank = |s: &Solution| (match s.status.as_str() { "routed" => 0, "conflicts" => 1, _ => 2 }, s.conflicts.len());
    let mut best = 0;
    for k in 1..results.len() {
        let (a, b) = (rank(&results[k]), rank(&results[best]));
        if a < b || (a == b && results[k].score < results[best].score) {
            best = k;
        }
    }
    let attempts: Vec<Attempt> = results
        .iter()
        .map(|s| Attempt { variant: s.variant, status: s.status.clone(), score: s.score, rounds: s.iterations, lengths_mm: s.nets.iter().map(|n| n.length_mm).collect(), vias: s.nets.iter().map(|n| n.vias.len()).sum() })
        .collect();
    let mut chosen = results.into_iter().nth(best).unwrap();
    chosen.attempts = attempts;
    chosen
}
