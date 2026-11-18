//! How the parts are turned: so that their pads face what they are joined to.

use super::Site;
use crate::floorplan::constants::BYPASS;
use crate::floorplan::known::Known;
use crate::floorplan::settle::quartered;
use crate::model::Pt;
use std::collections::BTreeMap;

/// The parts that stand by a pad: each with its own pad, what it stands by, and that one's pad.
pub(super) type Pins = BTreeMap<usize, (usize, usize, usize)>;

/// What the pads of the parts are asked: where each is with its part at a place, and how far it is from
/// what it is joined to. Where the parts stand (`centre`, as their boards are drawn) and how they are turned
/// (`quarter`) are handed in, for they change as the parts are placed.
pub(super) struct Facing<'a> {
    cx: &'a Known<'a>,
    on_gap: &'a BTreeMap<usize, Pt>,
    shift_of: &'a [Pt],
    /// How many quarters each part of a block that was placed whole is turned: it is not turned again.
    pub(super) block_quarter: &'a BTreeMap<usize, u8>,
    /// The pads on each signal net: the part and which of its pads.
    pub(super) pads_on: Vec<Vec<(usize, usize)>>,
}

impl<'a> Facing<'a> {
    pub(super) fn of(site: &'a Site<'a>, block_quarter: &'a BTreeMap<usize, u8>) -> Facing<'a> {
        let (cx, parts, all_nets) = (site.cx, site.cx.parts, site.cx.all_nets);
        let mut pads_on: Vec<Vec<(usize, usize)>> = vec![Vec::new(); all_nets.len()];
        for (p, part) in parts.iter().enumerate() {
            for (i, net) in part.pads.iter().enumerate() {
                if let Some(net) = net.filter(|&net| cx.is_signal(net)) {
                    pads_on[net].push((p, i));
                }
            }
        }
        Facing { cx, on_gap: &site.on_gap, shift_of: &site.shift_of, block_quarter, pads_on }
    }

    /// Where a pad of a part is, as the boards stack, with the part turned so many quarters.
    pub(super) fn pad_at(&self, p: usize, i: usize, quarters: u8, centre: &[Option<Pt>]) -> Option<Pt> {
        let (parts, shift_of) = (self.cx.parts, self.shift_of);
        let at = centre[p]?;
        let from = parts[p].offsets.get(i).map_or([0.0, 0.0], |&offset| quartered(quarters, offset));
        Some([at[0] + shift_of[p][0] + from[0], at[1] + shift_of[p][1] + from[1]])
    }

    /// Where the middle of a part is, as the boards stack.
    pub(super) fn in_stack(&self, q: usize, centre: &[Option<Pt>]) -> Option<Pt> {
        let shift_of = self.shift_of;
        centre[q].map(|at| [at[0] + shift_of[q][0], at[1] + shift_of[q][1]])
    }

    /// How far the pads of a part turned so many quarters are from the nearest other pad of their nets, all
    /// told. A capacitor that stands for a chip counts the way from its pad on the supply to the pad of the
    /// chip it is for.
    pub(super) fn reach_of(&self, p: usize, quarters: u8, centre: &[Option<Pt>], quarter: &[u8], pins: &Pins) -> f64 {
        let (parts, pads_on) = (self.cx.parts, &self.pads_on);
        let mut all = 0.0;
        for (i, net) in parts[p].pads.iter().enumerate() {
            let (Some(net), Some(here)) = (net, self.pad_at(p, i, quarters, centre)) else { continue };
            let others = pads_on[*net].iter().filter(|other| other.0 != p);
            let nearest = others
                .filter_map(|&(q, j)| self.pad_at(q, j, quarter[q], centre))
                .map(|there| (there[0] - here[0]).abs() + (there[1] - here[1]).abs())
                .fold(f64::INFINITY, f64::min);
            if nearest.is_finite() {
                all += nearest;
            }
        }
        if let Some(&(i, chip, j)) = pins.get(&p) {
            if let (Some(here), Some(there)) =
                (self.pad_at(p, i, quarters, centre), self.pad_at(chip, j, quarter[chip], centre))
            {
                all += BYPASS as f64 * ((there[0] - here[0]).abs() + (there[1] - here[1]).abs());
            }
        }
        // (and a chip counts the ways to the capacitors that stand for it)
        for (&c, &(i, _, j)) in pins.iter().filter(|(_, pin)| pin.1 == p) {
            if let (Some(here), Some(there)) =
                (self.pad_at(p, j, quarters, centre), self.pad_at(c, i, quarter[c], centre))
            {
                all += BYPASS as f64 * ((there[0] - here[0]).abs() + (there[1] - here[1]).abs());
            }
        }
        all
    }

    /// Each part is turned so that its pads face what they are joined to: of the ways its body may lie (as
    /// it was settled, or that turned half round; any of the four for a square one), the one
    /// that leaves its pads nearest the nearest other pad of their nets.
    pub(super) fn face(&self, centre: &[Option<Pt>], quarter: &mut [u8], pins: &Pins) {
        let (cx, parts, on_gap, block_quarter) = (self.cx, self.cx.parts, self.on_gap, self.block_quarter);
        for _ in 0..2 {
            for p in 0..parts.len() {
                if centre[p].is_none()
                    || on_gap.contains_key(&p)
                    || parts[p].offsets.is_empty()
                    || cx.stays[p]
                    || block_quarter.contains_key(&p)
                {
                    continue;
                }
                // (a body in pieces is not turned again: the ground between its pieces is taken as it stands)
                if !parts[p].pieces.is_empty() {
                    continue;
                }
                let [w, h] = cx.body(p);
                // (only a body that is square takes the same ground turned a quarter; one that is nearly so
                // would reach over what was settled against it)
                let any = (w - h).abs() < 1e-6;
                let ways: Vec<u8> = if any { vec![0, 1, 2, 3] } else { vec![quarter[p] % 2, quarter[p] % 2 + 2] };
                let mut best = (self.reach_of(p, quarter[p], centre, quarter, pins), quarter[p]);
                for way in ways {
                    let far = self.reach_of(p, way, centre, quarter, pins);
                    if far < best.0 - 1e-6 {
                        best = (far, way);
                    }
                }
                quarter[p] = best.1;
            }
        }
    }

    /// How far the pads of all the parts are from what they are joined to.
    pub(super) fn all_reach(&self, centre: &[Option<Pt>], quarter: &[u8]) -> f64 {
        let (parts, no_pins) = (self.cx.parts, Pins::new());
        (0..parts.len()).map(|p| self.reach_of(p, quarter[p], centre, quarter, &no_pins)).sum()
    }

    /// How long a net is: the box of its pads, as the boards stack.
    pub(super) fn span_of(&self, net: usize, centre: &[Option<Pt>], quarter: &[u8]) -> f64 {
        let (parts, reach) = (self.cx.parts, &self.cx.reach);
        let mut spots: Vec<Pt> = Vec::new();
        for &q in &reach[net] {
            let on = (0..parts[q].pads.len()).filter(|&i| parts[q].pads[i] == Some(net));
            spots.extend(on.filter_map(|i| self.pad_at(q, i, quarter[q], centre)));
        }
        if spots.len() < 2 {
            return 0.0;
        }
        let low = |k: usize| spots.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let high = |k: usize| spots.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        high(0) - low(0) + high(1) - low(1)
    }
}
