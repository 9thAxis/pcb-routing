//! The fixed look-ahead: what a path costs past the fixed copper alone.

use super::shared::CopperFloor;
use super::*;
use crate::mask::VIA_BIT;
use crate::queue::BucketQueue;

impl Engine<'_> {
    /// The cost from every (layer, cell) to the nearest of `goals` when only the fixed copper is in the way: no
    /// other new net, no corner cost. The real cost is never less, so the search can use it to look ahead; in
    /// open ground it is exact and the search walks straight to the goal.
    /// `holes`: cells where the net has a hole to change layer on for nothing, beyond those it has from the start.
    pub(super) fn floor(&self, n: usize, goals: &[(u32, u8)], holes: &[u32]) -> Arc<Vec<f32>> {
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
    pub(super) fn floor_to_copper(&self, n: usize, sources: &[u32], holes: &[u32], floor: &[f32]) -> Arc<Vec<f32>> {
        let near = sources.iter().map(|&s| floor[s as usize]).fold(f32::INFINITY, f32::min);
        if near <= 400.0 {
            return Arc::new(self.floor_to(n, sources, holes, 2.0 * near + 100.0));
        }
        if let Some(found) = self
            .shared
            .copper_floors
            .lock()
            .unwrap()
            .get(&n)
            .and_then(|kept| kept.iter().find(|k| k.sources == sources && k.holes == holes))
        {
            return found.table.clone();
        }
        let made = Arc::new(self.floor_to(n, sources, holes, f32::INFINITY));
        let mut all = self.shared.copper_floors.lock().unwrap();
        let kept = all.entry(n).or_default();
        if kept.len() >= 4 {
            kept.remove(0);
        }
        kept.push(CopperFloor { sources: sources.to_vec(), holes: holes.to_vec(), table: made.clone() });
        made
    }

    /// The same from every (layer, cell) to the nearest of the given states, out to `radius` (and `radius` beyond).
    pub(super) fn floor_to(&self, n: usize, states: &[u32], holes: &[u32], radius: f32) -> Vec<f32> {
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
        let mut queue = BucketQueue::new();
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
}
