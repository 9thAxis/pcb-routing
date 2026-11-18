//! The parts that stand on the gaps between two domains.

use super::{domain_named, Site, Standing};
use crate::floorplan::boards::Boards;
use crate::floorplan::ground::Work;
use crate::floorplan::known::Known;
use crate::floorplan::settle::{between, median, middle, off_rect, quartered, settled, spread, strip_between};
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
    // (a part that is held where it stands has its place before any other: there, as it lies; its body is
    // ground the board shuts)
    for p in (0..parts.len()).filter(|&p| cx.held(p)) {
        on_gap.insert(p, parts[p].at);
        gap_quarter.insert(p, 0);
    }
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
                entry.2.push((b, p, here[along], cx.lying(p, quarters)[along] + cx.board.tune.beside_mm));
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
                    between(v, low + size / 2.0, high - size / 2.0)
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
            // (along its gap as it lies; and where there is no room for it so, anywhere on its board, turned if
            // it must be: no body stands over another)
            let mut quarters = quarters;
            if let Some((free, false)) = settled(band, &[([w, h], at)], &shut)[0] {
                at = free;
            } else if let Some((free, turn)) = settled(region, &[([w, h], at)], &shut)[0] {
                (at, quarters) = (free, (quarters + u8::from(turn)) % 4);
                gap_quarter.insert(p, quarters);
            }
            bridges[b].at = at.map(|v| (v * 100.0).round() / 100.0);
            bridges[b].off = parts[p].placed.then(|| (dist(parts[p].at, at) * 10.0).round() / 10.0);
            on_gap.insert(p, at);
            gap_rects.insert(p, cx.body_at(p, at, quarters));
        }
    }
    Gaps { on: on_gap, quarter: gap_quarter, rects: gap_rects }
}

/// The parts on the gaps are stood along them once more, now by where the parts they are tied to have come
/// to stand (and not the middles of their zones): each gap's parts spread as before.
pub(super) fn slid(
    site: &Site,
    gap_rects: &mut BTreeMap<usize, [f64; 4]>,
    bridges: &mut [Bridge],
    standing: &mut Standing,
) {
    let (cx, top, gap_quarter, shift_of) = (site.cx, site.top, &site.gap_quarter, &site.shift_of);
    let (board, parts, nets) = (cx.board, cx.parts, &cx.nets);
    let Standing { stood, centre, quarter } = standing;
    type Strip = (bool, [f64; 4], Vec<(usize, usize, f64, f64)>); // (bridge, part, want along, size)
    let mut strips: BTreeMap<[i64; 4], Strip> = BTreeMap::new();
    for (b, bridge) in bridges.iter().enumerate().filter(|(_, bridge)| bridge.on == Across::Gap) {
        let Some(p) = parts.iter().position(|part| part.name == bridge.part) else { continue };
        let (Some(home), Some(other)) = (domain_named(top, &bridge.grounds[0]), domain_named(top, &bridge.grounds[1]))
        else {
            continue;
        };
        let Some((strip, up)) = strip_between(top[home].rect, top[other].rect, board.gap) else { continue };
        let shift = shift_of[p];
        let mates: Vec<Pt> = (0..nets.len())
            .filter(|&n| nets[n].contains(&p))
            .flat_map(|n| nets[n].iter().filter(|&&q| q != p))
            .filter_map(|&q| centre[q].map(|at| [at[0] + shift_of[q][0] - shift[0], at[1] + shift_of[q][1] - shift[1]]))
            .collect();
        let Some(want) = median(&mates) else { continue };
        let key = strip.map(|v| (v * 1000.0).round() as i64);
        let entry = strips.entry(key).or_insert((up, strip, Vec::new()));
        let quarters = gap_quarter.get(&p).copied().unwrap_or(0);
        entry.2.push((b, p, want[usize::from(up)], cx.lying(p, quarters)[usize::from(up)] + cx.board.tune.beside_mm));
    }
    for (up, strip, on_it) in strips.into_values() {
        let along = usize::from(up);
        let (want, size): (Vec<f64>, Vec<f64>) = on_it.iter().map(|&(_, _, want, size)| (want, size)).unzip();
        for (&(b, p, _, _), place) in on_it.iter().zip(spread(&want, &size, strip[along], strip[along + 2])) {
            let (Some(was), quarters) = (centre[p], gap_quarter.get(&p).copied().unwrap_or(0)) else { continue };
            let mut at = was;
            at[along] = place;
            // (it goes there if nothing that has its place is in its way there: no body, no other gap's part)
            let here = cx.body_at(p, at, quarters);
            let over = |r: [f64; 4]| {
                r[2].min(here[2]) - r[0].max(here[0]) > 1e-6 && r[3].min(here[3]) - r[1].max(here[1]) > 1e-6
            };
            let bodies = stood
                .iter()
                .filter(|(&q, _)| q != p && cx.side_of(q) & cx.side_of(p) != 0 && shift_of[q] == shift_of[p])
                .filter_map(|(&q, at)| {
                    at.map(|_| centre[q]).flatten().map(|at| cx.taken(q, (at, quarter[q] % 2 == 1)))
                });
            let others = gap_rects.iter().filter(|(&q, _)| q != p).map(|(_, &r)| r);
            if bodies.chain(others).chain(board.keepouts.iter().copied()).any(over) {
                continue;
            }
            centre[p] = Some(at);
            gap_rects.insert(p, here);
            bridges[b].at = at.map(|v| (v * 100.0).round() / 100.0);
            bridges[b].off = parts[p].placed.then(|| (dist(parts[p].at, at) * 10.0).round() / 10.0);
        }
    }
}
