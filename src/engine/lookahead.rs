//! The fixed look-ahead: what a path costs past the fixed copper alone.
//!
//! A table of it is worked out only as far as the search that asked for it can need: out to the cost between
//! the two ends of the connection and a quarter more (a search past the fixed copper alone never looks at a
//! state further than that cost from either end). Everything further is given that distance, which
//! is never more than the real cost, so the search stays right and only loses its aim if it strays that far.
//! On a board with many short connections this is what keeps the work per connection local.

use super::shared::CopperFloor;
use super::*;
use crate::mask::VIA_BIT;
use crate::queue::BucketQueue;

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
    hw: usize,
    w: usize,
}

impl Straight {
    #[inline]
    fn get(&self, at: usize) -> f32 {
        let (layer, cell) = (at / self.hw, at % self.hw);
        let (j, i) = ((cell / self.w) as i32, (cell % self.w) as i32);
        let mut best = f32::INFINITY;
        for &(tj, ti, layers) in &self.targets {
            let (a, b) = ((j - tj).abs() as f32, (i - ti).abs() as f32);
            let run = (a.max(b) + (std::f32::consts::SQRT_2 - 1.0) * a.min(b)) * self.unit;
            best = best.min(if layers & (1 << layer) != 0 { run } else { run + self.change });
        }
        best
    }
}

/// What a search looks ahead with past the fixed copper: the exact cost from a table, or the straight way.
pub(crate) enum Look {
    Exact(Arc<Table>),
    Straight(Straight),
}

impl Look {
    #[inline]
    pub(super) fn get(&self, at: usize) -> f32 {
        match self {
            Look::Exact(table) => table.get(at),
            Look::Straight(straight) => straight.get(at),
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
    pub(super) fn states_of(&self, terminals: &[(u32, u8)]) -> Vec<u32> {
        let mut states = Vec::new();
        for &(cell, bits) in terminals {
            for l in 0..self.nl {
                if bits & (1 << l) != 0 {
                    states.push((l * self.hw) as u32 + cell);
                }
            }
        }
        states
    }

    /// What to look ahead with towards `goals`. The first times a connection is laid, the straight way: on a
    /// board where most connections are laid once, an exact table for each would cost far more than the
    /// searches themselves. A connection that is laid again and again (the nets that negotiate) gets its
    /// table from the `exact_after`-th time on, and that table is shared by all attempts.
    pub(super) fn look(&self, n: usize, goals: &[(u32, u8)], holes: &[u32], others: &[u32]) -> Look {
        let after = self.p.options.exact_after;
        let asked = {
            let mut asked = self.asked.borrow_mut();
            let count = asked.entry((n, goals.to_vec(), holes.to_vec())).or_insert(0);
            *count += 1;
            *count
        };
        if asked as usize > after {
            return Look::Exact(self.floor(n, goals, holes, others));
        }
        let w = self.grid.w;
        let unit = self.layer_cost[n]
            .iter()
            .zip(&self.p.layers)
            .filter(|(_, l)| self.p.nets[n].layers.contains(l))
            .map(|(c, _)| *c)
            .fold(f64::INFINITY, f64::min);
        let free = !self.fixed_free[n].is_empty() || !holes.is_empty();
        let change = if free && self.p.options.own_hole_free {
            0.0
        } else {
            self.p.nets[n].via_penalty_mm / self.p.step * 0.9999
        };
        Look::Straight(Straight {
            targets: goals.iter().map(|g| ((g.0 as usize / w) as i32, (g.0 as usize % w) as i32, g.1)).collect(),
            unit: (unit * 0.9999) as f32,
            change: change as f32,
            hw: self.hw,
            w,
        })
    }

    /// The cost from every (layer, cell) to the nearest of `goals` when only the fixed copper is in the way: no
    /// other new net, no corner cost. The real cost is never less, so the search can use it to look ahead; in
    /// open ground it is exact and the search walks straight to the goal.
    /// `holes`: cells where the net has a hole to change layer on for nothing, beyond those it has from the start.
    /// `others`: the states at the other end of the search; the table is exact far enough round them.
    fn floor(&self, n: usize, goals: &[(u32, u8)], holes: &[u32], others: &[u32]) -> Arc<Table> {
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
        let (w, hw, nl) = (self.grid.w as i32, self.hw, self.nl);
        let mask = &self.shared.masks[n];
        let open = &self.shared.open[n][..];
        let lc = &self.layer_cost[n];
        let via_cost = (net.via_penalty_mm / self.p.step) as f32;
        let own_hole = if self.p.options.own_hole_free { 0.0 } else { via_cost };
        let off: [isize; 8] = DIRS.map(|(dj, di)| (dj * w + di) as isize);
        let len: [f32; 8] = DIRS.map(|(dj, di)| if dj != 0 && di != 0 { std::f32::consts::SQRT_2 } else { 1.0 });
        let mut free: Vec<u32> = self.fixed_free[n].iter().chain(holes).copied().collect();
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
            let mut bits = open[ui];
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
            let m = mask[c];
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
