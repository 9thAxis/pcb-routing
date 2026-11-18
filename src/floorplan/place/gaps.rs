//! The parts that stand on the gaps between two domains.

use super::domain_named;
use crate::floorplan::boards::Boards;
use crate::floorplan::constants::BESIDE;
use crate::floorplan::ground::Work;
use crate::floorplan::known::Known;
use crate::floorplan::settle::{middle, off_rect, quartered, settled, spread, strip_between};
use crate::floorplan::types::{Across, Bridge};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::BTreeMap;

/// The parts on the gaps: where the middle of each is, how many quarters it is turned, and the ground its
/// body takes.
pub(super) struct Gaps {
    pub(super) on: BTreeMap<usize, Pt>,
    pub(super) quarter: BTreeMap<usize, u8>,
    pub(super) rects: BTreeMap<usize, [f64; 4]>,
}

/// A part on a gap has a body like any other, and it is the first to have its ground: turned so that its
/// pads lie on the side of the gap their nets belong to, and along the gap clear of the others there. No
/// other part may stand on it. Each bridge is told where its part came to stand.
pub(super) fn on_gaps(cx: &Known, top: &[Work], on: &Boards, bridges: &mut [Bridge]) -> Gaps {
    let (board, parts) = (cx.board, cx.parts);
    let (boards, to) = (&on.boards, &on.to);
    let mut on_gap: BTreeMap<usize, Pt> = bridges
        .iter()
        .filter_map(|bridge| parts.iter().position(|part| part.name == bridge.part).map(|p| (p, bridge.at)))
        .collect();
    let mut gap_quarter: BTreeMap<usize, u8> = BTreeMap::new();
    let mut gap_rects: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
    {
        type Strip = (bool, [f64; 4], Vec<(usize, usize, f64, f64)>); // (bridge, part, want along, size)
        let mut strips: BTreeMap<[i64; 4], Strip> = BTreeMap::new();
        let mut gap_along: BTreeMap<usize, usize> = BTreeMap::new(); // a part on a gap -> the way its gap runs
        for (b, bridge) in bridges.iter().enumerate() {
            let Some(p) = parts.iter().position(|part| part.name == bridge.part) else { continue };
            let Some(home) = domain_named(top, &bridge.grounds[0]) else { continue };
            let here = bridge.at;
            let across = |quarters: u8| -> f64 {
                let mut far = 0.0;
                for (i, net) in parts[p].pads.iter().enumerate() {
                    let (Some(net), Some(&offset)) = (net, parts[p].offsets.get(i)) else { continue };
                    let Some(domain) = cx.net_domain(*net).filter(|&domain| to[domain] == to[home]) else { continue };
                    let from = quartered(quarters, offset);
                    far += off_rect([here[0] + from[0], here[1] + from[1]], top[domain].rect);
                }
                far
            };
            let quarters = (0..4u8)
                .fold((f64::INFINITY, 0u8), |best, k| if across(k) < best.0 - 1e-6 { (across(k), k) } else { best })
                .1;
            gap_quarter.insert(p, quarters);
            let other = domain_named(top, &bridge.grounds[1]);
            let strip = other
                .filter(|_| bridge.on == Across::Gap)
                .and_then(|other| strip_between(top[home].rect, top[other].rect, board.gap));
            if let Some((strip, up)) = strip {
                let along = usize::from(up);
                let key = strip.map(|v| (v * 1000.0).round() as i64);
                let entry = strips.entry(key).or_insert((up, strip, Vec::new()));
                entry.2.push((b, p, here[along], cx.lying(p, quarters)[along] + BESIDE));
            }
        }
        for (up, strip, on_it) in strips.into_values() {
            let along = usize::from(up);
            let (want, size): (Vec<f64>, Vec<f64>) = on_it.iter().map(|&(_, _, want, size)| (want, size)).unzip();
            for (&(b, p, _, _), place) in on_it.iter().zip(spread(&want, &size, strip[along], strip[along + 2])) {
                let mut at = middle(strip);
                at[along] = place;
                bridges[b].at = at;
                on_gap.insert(p, at);
                gap_along.insert(p, along);
            }
        }
        // (and each within its board: a long connector on a short gap reaches past the gap, not past the edge;
        // and off what the board shuts and the bodies on the other gaps: the largest first, each moved along
        // its gap to the nearest room there is)
        let mut order: Vec<usize> = (0..bridges.len()).collect();
        let their_parts: Vec<Option<usize>> =
            bridges.iter().map(|bridge| parts.iter().position(|part| part.name == bridge.part)).collect();
        let part_of = |b: usize| their_parts[b];
        order.sort_by(|&a, &b| {
            let takes = |b: usize| part_of(b).map_or(0.0, |p| cx.body(p)[0] * cx.body(p)[1]);
            takes(b).total_cmp(&takes(a)).then(a.cmp(&b))
        });
        for b in order {
            let Some(p) = part_of(b) else { continue };
            let Some(home) = domain_named(top, &bridges[b].grounds[0]) else { continue };
            let (quarters, region) = (gap_quarter.get(&p).copied().unwrap_or(0), boards[to[home]]);
            let [w, h] = cx.lying(p, quarters);
            let hold = |v: f64, low: f64, high: f64, size: f64| {
                if high - low >= size {
                    v.clamp(low + size / 2.0, high - size / 2.0)
                } else {
                    (low + high) / 2.0
                }
            };
            let mut at =
                [hold(bridges[b].at[0], region[0], region[2], w), hold(bridges[b].at[1], region[1], region[3], h)];
            let shut: Vec<[f64; 4]> = board.keepouts.iter().chain(gap_rects.values()).copied().collect();
            let band = match gap_along.get(&p) {
                Some(&along) => {
                    let mut band = cx.body_at(p, at, quarters);
                    (band[along], band[along + 2]) = (region[along], region[along + 2]);
                    band
                }
                None => region,
            };
            if let Some((free, false)) = settled(band, &[([w, h], at)], &shut)[0] {
                at = free;
            }
            bridges[b].at = at.map(|v| (v * 100.0).round() / 100.0);
            bridges[b].off = parts[p].placed.then(|| (dist(parts[p].at, at) * 10.0).round() / 10.0);
            on_gap.insert(p, at);
            gap_rects.insert(p, cx.body_at(p, at, quarters));
        }
    }
    Gaps { on: on_gap, quarter: gap_quarter, rects: gap_rects }
}
