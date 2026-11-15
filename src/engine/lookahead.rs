//! The fixed look-ahead: what a path costs past the fixed copper alone.
//!
//! A table of it is worked out only as far as the search that asked for it can need: out to the cost between
//! the two ends of the connection and a quarter more (a search past the fixed copper alone never looks at a
//! state further than that cost from either end). Everything further is given that distance, which
//! is never more than the real cost, so the search stays right and only loses its aim if it strays that far.
//! On a board with many short connections this is what keeps the work per connection local.

use super::shared::CopperFloor;
use super::*;
use crate::geom::octile;
use crate::mask::VIA_BIT;
use crate::queue::BucketQueue;
use space::Space;
use std::sync::Arc;

const PAGE_BITS: usize = 8;
const PAGE: usize = 1 << PAGE_BITS;
const NO_PAGE: u32 = u32::MAX;
/// How far a table goes beyond the cost between the two ends, as a factor and in grid steps.
const FURTHER: f32 = 1.25;
const BEYOND: f32 = 50.0;

/// Cost per (layer, cell) state, kept only for the stretches of states that were reached; every other state
/// has the value `beyond`.
pub(crate) struct Table {
    index: Vec<u32>,
    data: Vec<f32>,
    beyond: f32,
}

impl Table {
    fn new(states: usize) -> Table {
        Table { index: vec![NO_PAGE; states.div_ceil(PAGE)], data: Vec::new(), beyond: f32::INFINITY }
    }

    #[inline]
    pub(super) fn get(&self, at: usize) -> f32 {
        let page = self.index[at >> PAGE_BITS];
        if page == NO_PAGE {
            self.beyond
        } else {
            self.data[((page as usize) << PAGE_BITS) | (at & (PAGE - 1))]
        }
    }

    #[inline]
    fn set(&mut self, at: usize, value: f32) {
        let mut page = self.index[at >> PAGE_BITS];
        if page == NO_PAGE {
            page = (self.data.len() >> PAGE_BITS) as u32;
            self.index[at >> PAGE_BITS] = page;
            self.data.resize(self.data.len() + PAGE, f32::INFINITY);
        }
        self.data[((page as usize) << PAGE_BITS) | (at & (PAGE - 1))] = value;
    }

    /// True if the table is exact as far as a search between its own ends and `others` can need.
    fn reaches(&self, others: &[u32]) -> bool {
        if self.beyond == f32::INFINITY {
            return true;
        }
        let furthest = others.iter().map(|&s| self.get(s as usize)).fold(0.0, f32::max);
        furthest < self.beyond && wanted(furthest) <= self.beyond + 1.0 // the table's values are kept a little under
    }
}

/// A lower bound on the cost from a state to the nearest of a few cells that needs no table: the length of the
/// straight way there in the eight directions, on the net's cheapest layer, as if nothing were in the way.
pub(crate) struct Straight {
    /// Row, column and layers of each target.
    targets: Vec<(i32, i32, u8)>,
    unit: f32,
    /// What a layer change costs at least (0 if the net has a hole it may change layer on for nothing).
    change: f32,
    space: Space,
}

impl Straight {
    #[inline]
    fn get(&self, at: usize) -> f32 {
        let (layer, cell) = self.space.split(at);
        let (j, i) = self.space.rc(cell);
        let (j, i) = (j as i32, i as i32);
        let mut best = f32::INFINITY;
        for &(tj, ti, layers) in &self.targets {
            let (a, b) = ((j - tj).abs() as f32, (i - ti).abs() as f32);
            let run = octile(a, b) * self.unit;
            best = best.min(if layers & (1 << layer) != 0 { run } else { run + self.change });
        }
        best
    }
}

/// A lower bound on the cost from a state to copper of any shape: the straight way to the box round it (0 inside).
pub(crate) struct Around {
    rows: (i32, i32),
    columns: (i32, i32),
    unit: f32,
    space: Space,
}

impl Around {
    #[inline]
    fn get(&self, at: usize) -> f32 {
        let (j, i) = self.space.rc(self.space.split(at).1);
        let (j, i) = (j as i32, i as i32);
        let a = (self.rows.0 - j).max(j - self.rows.1).max(0) as f32;
        let b = (self.columns.0 - i).max(i - self.columns.1).max(0) as f32;
        octile(a, b) * self.unit
    }
}

/// A closer lower bound on the cost from a state to copper of any shape: the straight way to the nearest of
/// the tiles (squares of 16 cells) that hold some of it. Each tile of the board keeps the few copper tiles
/// that can be the nearest one for a cell in it, so a look-up tries a handful of boxes.
pub(crate) struct Tiles {
    /// Per tile of the board: the copper tiles that may be nearest, as (first row, last row, first column, last column).
    near: Vec<Vec<[i32; 4]>>,
    across: usize,
    unit: f32,
    space: Space,
}

const TILE_BITS: usize = 4;

#[inline]
fn to_box(j: i32, i: i32, b: &[i32; 4]) -> f32 {
    let a = (b[0] - j).max(j - b[1]).max(0) as f32;
    let c = (b[2] - i).max(i - b[3]).max(0) as f32;
    octile(a, c)
}

impl Tiles {
    #[inline]
    fn get(&self, at: usize) -> f32 {
        let (j, i) = self.space.rc(self.space.split(at).1);
        let tile = (j >> TILE_BITS) * self.across + (i >> TILE_BITS);
        let mut best = f32::INFINITY;
        for b in &self.near[tile] {
            best = best.min(to_box(j as i32, i as i32, b));
        }
        best * self.unit
    }
}

/// What a search looks ahead with past the fixed copper: the exact cost from a table, or the straight way to a
/// few cells, or to the box round a net's copper.
pub(crate) enum Look {
    Exact(Arc<Table>),
    Straight(Straight),
    Around(Around),
    Tiles(Tiles),
}

impl Look {
    #[inline]
    pub(super) fn get(&self, at: usize) -> f32 {
        match self {
            Look::Exact(table) => table.get(at),
            Look::Straight(straight) => straight.get(at),
            Look::Around(around) => around.get(at),
            Look::Tiles(tiles) => tiles.get(at),
        }
    }

    /// True if a value of 0 means the state is joined to the target at no cost (a table); false if it can
    /// also mean the state is at a target's cell on another layer, with a free layer change assumed.
    pub(super) fn exact(&self) -> bool {
        matches!(self, Look::Exact(_))
    }
}

/// How far a table must go for a search whose other end is `furthest` away.
fn wanted(furthest: f32) -> f32 {
    FURTHER * furthest + BEYOND
}

impl Engine<'_> {
    /// The states of a set of terminals.
    pub(super) fn states_of(&self, terminals: &[Term]) -> Vec<u32> {
        terminals.iter().flat_map(|t| t.states(self.plan.space)).collect()
    }

    /// What to look ahead with towards `goals`. The first times a connection is laid, the straight way: on a
    /// board where most connections are laid once, an exact table for each would cost far more than the
    /// searches themselves. A connection that is laid again and again (the nets that negotiate) gets its
    /// table from the `exact_after`-th time on, and that table is shared by all attempts.
    pub(super) fn look(&self, n: usize, goals: &[Term], holes: &[u32], others: &[u32]) -> Look {
        let after = self.p.options.exact_after;
        let asked = {
            let mut asked = self.policy.asked.borrow_mut();
            let count = asked.entry((n, goals.to_vec(), holes.to_vec())).or_insert(0);
            *count += 1;
            *count
        };
        // (a table towards many terminals at once covers most of the board and changes as they are joined)
        if asked as usize > after && goals.len() <= 4 {
            return Look::Exact(self.floor(n, goals, holes, others));
        }
        let unit = self.plan.layer_cost[n]
            .iter()
            .zip(&self.p.layers)
            .filter(|(_, l)| self.p.nets[n].layers.contains(l))
            .map(|(c, _)| *c)
            .fold(f64::INFINITY, f64::min);
        // A layer change is counted as free only if the net has a hole to change on for nothing. This rests on
        // every such hole being known here: it is in `fixed_free` or in `holes`, or (for a net that may branch
        // anywhere) it is a source of the search on every open layer, so that no path needs to change on it.
        // A hole that was none of these would make the look-ahead too high.
        let free = !self.plan.fixed_free[n].is_empty() || !holes.is_empty();
        let change = if free && self.p.options.own_hole_free {
            0.0
        } else {
            self.p.nets[n].via_penalty_mm / self.p.step * 0.9999
        };
        Look::Straight(Straight {
            targets: goals
                .iter()
                .map(|g| {
                    let (j, i) = self.plan.space.rc(g.cell as usize);
                    (j as i32, i as i32, g.layers)
                })
                .collect(),
            unit: (unit * 0.9999) as f32,
            change: change as f32,
            space: self.plan.space,
        })
    }

    /// The cheapest layer cost of a net, a little under.
    fn unit(&self, n: usize) -> f32 {
        let costs =
            self.plan.layer_cost[n].iter().zip(&self.p.layers).filter(|(_, l)| self.p.nets[n].layers.contains(l));
        (costs.map(|(c, _)| *c).fold(f64::INFINITY, f64::min) * 0.9999) as f32
    }

    /// The look-ahead towards copper of the net that is more than terminals (the tree laid so far): the
    /// straight way to the box round it. The tree changes with every connection and every round, so a table
    /// of the exact cost would be worked out anew each time, at many times the cost of the search it serves.
    pub(super) fn look_around(&self, n: usize, sources: &[u32]) -> Look {
        let (space, w) = (self.plan.space, self.plan.space.w);
        let (mut rows, mut columns) = ((i32::MAX, i32::MIN), (i32::MAX, i32::MIN));
        for &s in sources {
            let (j, i) = space.rc(space.split(s as usize).1);
            let (j, i) = (j as i32, i as i32);
            rows = (rows.0.min(j), rows.1.max(j));
            columns = (columns.0.min(i), columns.1.max(i));
        }
        let size = 1usize << TILE_BITS;
        let (across, down) = (w.div_ceil(size), self.plan.grid.h.div_ceil(size));
        let mut held = vec![false; across * down];
        for &s in sources {
            let (j, i) = space.rc(space.split(s as usize).1);
            held[(j >> TILE_BITS) * across + (i >> TILE_BITS)] = true;
        }
        let boxes: Vec<[i32; 4]> = (0..across * down)
            .filter(|&t| held[t])
            .map(|t| {
                let (tj, ti) = ((t / across) as i32, (t % across) as i32);
                [tj << TILE_BITS, ((tj + 1) << TILE_BITS) - 1, ti << TILE_BITS, ((ti + 1) << TILE_BITS) - 1]
            })
            .collect();
        if boxes.len() * across * down > 4_000_000 {
            // too much copper on too large a board to sort the tiles for: the one box round it all
            return Look::Around(Around { rows, columns, unit: self.unit(n), space });
        }
        // A copper tile can be the nearest for some cell of a tile only if its nearest corner is no further
        // than the furthest corner of the copper tile that is nearest overall.
        let across_tile = (size as f32 - 1.0) * std::f32::consts::SQRT_2;
        let mut near = Vec::with_capacity(across * down);
        for t in 0..across * down {
            let (j0, i0) = (((t / across) << TILE_BITS) as i32, ((t % across) << TILE_BITS) as i32);
            let (j1, i1) = (j0 + size as i32 - 1, i0 + size as i32 - 1);
            // the least distance between this tile and a copper tile, both as boxes
            let apart = |b: &[i32; 4]| {
                let a = (b[0] - j1).max(j0 - b[1]).max(0) as f32;
                let c = (b[2] - i1).max(i0 - b[3]).max(0) as f32;
                octile(a, c)
            };
            let least = boxes.iter().map(apart).fold(f32::INFINITY, f32::min);
            near.push(
                boxes.iter().filter(|b| apart(b) <= least + 2.0 * across_tile + 1.0).copied().collect::<Vec<_>>(),
            );
        }
        Look::Tiles(Tiles { near, across, unit: self.unit(n), space })
    }

    /// The cost from every (layer, cell) to the nearest of `goals` when only the fixed copper is in the way: no
    /// other new net, no corner cost. The real cost is never less, so the search can use it to look ahead; in
    /// open ground it is exact and the search walks straight to the goal.
    /// `holes`: cells where the net has a hole to change layer on for nothing, beyond those it has from the start.
    /// `others`: the states at the other end of the search; the table is exact far enough round them.
    fn floor(&self, n: usize, goals: &[Term], holes: &[u32], others: &[u32]) -> Arc<Table> {
        let key = (n, goals.to_vec(), holes.to_vec());
        if let Some(found) = self.shared.floors.lock().unwrap().get(&key) {
            if found.reaches(others) {
                return found.clone();
            }
        }
        let made = {
            let _t = self.shared.timed(7);
            Arc::new(self.floor_to(n, &self.states_of(goals), holes, None, others))
        };
        self.shared.floors.lock().unwrap().insert(key, made.clone());
        made
    }

    /// The look-ahead towards copper of the net that is more than terminals. Where the goal is near (a short
    /// hop), only out to a radius round the copper; otherwise as far as the search can need, and kept for the
    /// next search from the same copper.
    pub(super) fn floor_to_copper(
        &self,
        n: usize,
        sources: &[u32],
        holes: &[u32],
        floor: &Look,
        goals: &[u32],
    ) -> Arc<Table> {
        let near = sources.iter().map(|&s| floor.get(s as usize)).fold(f32::INFINITY, f32::min);
        if near <= 400.0 {
            return Arc::new(self.floor_to(n, sources, holes, Some(2.0 * near + 100.0), &[]));
        }
        if let Some(found) =
            self.shared.copper_floors.lock().unwrap().get(&n).and_then(|kept| {
                kept.iter().find(|k| k.sources == sources && k.holes == holes && k.table.reaches(goals))
            })
        {
            return found.table.clone();
        }
        let made = Arc::new(self.floor_to(n, sources, holes, None, goals));
        let mut all = self.shared.copper_floors.lock().unwrap();
        let kept = all.entry(n).or_default();
        if kept.len() >= 4 {
            kept.remove(0);
        }
        kept.push(CopperFloor { sources: sources.to_vec(), holes: holes.to_vec(), table: made.clone() });
        made
    }

    /// The cost from every (layer, cell) to the nearest of the given states, out to a radius (and that radius
    /// beyond): `radius` if given, else far enough for a search whose other end is `others` (everywhere, if
    /// one of them cannot be reached at all).
    fn floor_to(&self, n: usize, states: &[u32], holes: &[u32], radius: Option<f32>, others: &[u32]) -> Table {
        let net = &self.p.nets[n];
        let (hw, nl) = (self.plan.space.hw, self.plan.space.nl);
        let fixed = &self.shared.fixed;
        let lc = &self.plan.layer_cost[n];
        let via_cost = (net.via_penalty_mm / self.p.step) as f32;
        let own_hole = if self.p.options.own_hole_free { 0.0 } else { via_cost };
        let off = self.plan.steps.off;
        let len: [f32; 8] = self.plan.steps.len.map(|v| v as f32);
        let mut free: Vec<u32> = self.plan.fixed_free[n].iter().chain(holes).copied().collect();
        free.sort_unstable();
        let mut waiting: Vec<u32> = others.to_vec();
        waiting.sort_unstable();
        waiting.dedup();
        let mut left = waiting.len();
        let mut found = vec![false; waiting.len()];
        let mut radius = radius.unwrap_or(f32::INFINITY);
        let mut dist = Table::new(nl * hw);
        let mut queue = BucketQueue::new();
        for &at in states {
            dist.set(at as usize, 0.0);
            queue.push(0, at);
        }
        while let Some((k, u)) = queue.pop() {
            let ui = u as usize;
            let d = dist.get(ui);
            if (d * 64.0) as u64 != k {
                continue; // a cheaper way here was found after this entry was put in
            }
            if d > radius {
                break;
            }
            if left > 0 {
                if let Ok(i) = waiting.binary_search(&u) {
                    if !found[i] {
                        found[i] = true;
                        left -= 1;
                        if left == 0 {
                            radius = radius.min(wanted(d)); // the last of the other end: this far and no further
                        }
                    }
                }
            }
            let (l, c) = (ui / hw, ui % hw);
            let step = lc[l] as f32;
            let mut bits = fixed.open(n, l, c);
            while bits != 0 {
                let d2 = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let v = (ui as isize + off[d2]) as usize;
                let nd = d + len[d2] * step;
                if nd < dist.get(v) {
                    dist.set(v, nd);
                    queue.push((nd * 64.0) as u64, v as u32);
                }
            }
            let m = fixed.mask(n, c);
            let cost = if free.binary_search(&(c as u32)).is_ok() {
                own_hole
            } else if m & VIA_BIT == 0 {
                via_cost
            } else {
                continue;
            };
            for ll in 0..nl {
                if ll != l && m & (1 << ll) == 0 && d + cost < dist.get(ll * hw + c) {
                    dist.set(ll * hw + c, d + cost);
                    queue.push(((d + cost) * 64.0) as u64, (ll * hw + c) as u32);
                }
            }
        }
        // A state not taken out has a cost of at least the radius, whatever it was last given. And everything
        // a little under, so that rounding in single precision can never put it above the real cost.
        for v in dist.data.iter_mut() {
            *v = v.min(radius) * 0.9999;
        }
        dist.beyond = radius * 0.9999;
        dist
    }
}
