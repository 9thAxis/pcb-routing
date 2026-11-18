//! Ground for each zone: the board cut again and again, the zones in the order they are pulled.

use super::constants::{ANCHOR, POURED, RAIL_LANES, SQUARISH};
use super::settle::middle;
use super::ties::{in_line, tied};
use super::types::{Board, Road};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// A zone while it is worked out: its parts by index.
#[derive(Clone)]
pub(super) struct Work {
    pub(super) name: String,
    pub(super) parts: Vec<usize>,
    pub(super) inside: Vec<Work>,
    pub(super) rect: [f64; 4],
    /// The ground it needs besides that of its parts, in mm2: for the parts of another domain that stand half
    /// on this one.
    pub(super) more: f64,
}

/// What the giving of ground works on, and what it leaves.
pub(super) struct Ground<'a> {
    pub(super) board: &'a Board,
    /// The signal nets, each as its parts.
    pub(super) nets: &'a [Vec<usize>],
    /// The supplies: of each, the parts that stand on its pour on a plane layer, and those that do not (all of
    /// them, where there is no such layer).
    pub(super) rails: &'a [(Vec<usize>, Vec<usize>)],
    /// Where each part is taken to stand so far.
    pub(super) at: Vec<Option<Pt>>,
    pub(super) roads: Vec<Road>,
    /// The board being laid out, and what is added to a place on it to say where that is in the stack.
    pub(super) region: [f64; 4],
    pub(super) shift: Pt,
    /// The cuts whose two lots change sides (a cut is known by the zones it parts: see `cut_of`).
    pub(super) turned: &'a BTreeSet<String>,
    /// The cuts made across the shorter side of their ground, not the longer.
    pub(super) crossed: &'a BTreeSet<String>,
    /// Zones put in the first lot of a cut (true) or the second, whatever they are pulled to: by (cut, zone).
    pub(super) sided: &'a BTreeMap<(String, String), bool>,
    /// And the tracks a cut's road is asked to hold at the least.
    pub(super) asked: &'a BTreeMap<String, usize>,
    /// The part each part mates with, if any.
    pub(super) mates: &'a [Option<usize>],
    /// Whether it is parts that are given ground, within a zone: no road is kept between them.
    pub(super) fine: bool,
    /// For each supply poured on a plane layer: a part of each of the smallest zones it is poured under. Those
    /// zones are pulled together, that the pour be of one piece.
    pub(super) kin: &'a [Vec<usize>],
}

/// What a cut is known by from one laying out to the next: the zones it parts, by name.
pub(super) fn cut_of(zones: &[Work]) -> String {
    let mut names: Vec<&str> = zones.iter().map(|zone| zone.name.as_str()).collect();
    names.sort_unstable();
    names.join(" | ")
}

/// The ground a lot of parts takes, in mm2. A board has two sides: what stands on the front and what stands on
/// the back share the same ground, so the lot takes the more of the two, and a part with holes takes its ground
/// on both.
pub(super) fn taken_by(board: &Board, parts: &[usize]) -> f64 {
    let (mut front, mut back, mut through) = (0.0, 0.0, 0.0);
    for &p in parts {
        let part = &board.parts[p];
        if part.through {
            through += part.area;
        } else if part.back {
            back += part.area;
        } else {
            front += part.area;
        }
    }
    through + f64::max(front, back)
}

/// Give the zones of `zones` ground within `rect`, and theirs within that (see the module). `at`: where each
/// part is taken to stand so far (a fixed part where it stands; another at the middle of the ground its zone
/// has been given, once it has).
pub(super) fn arrange(zones: &mut [Work], rect: [f64; 4], level: usize, domains: bool, on: &mut Ground) {
    let (board, nets) = (on.board, on.nets);
    let (shift, region) = (on.shift, on.region);
    let at = &mut on.at;
    let settle = |zone: &Work, rect: [f64; 4], at: &mut Vec<Option<Pt>>| {
        for &part in &zone.parts {
            if !(board.parts[part].fixed && board.parts[part].placed) {
                // (kept as places of the stack: what is on another board is pulled to the same place)
                let here = middle(rect);
                at[part] = Some([here[0] + shift[0], here[1] + shift[1]]);
            }
        }
    };
    match zones.len() {
        0 => return,
        1 => {
            zones[0].rect = rect;
            settle(&zones[0], rect, at);
            let (inner, _) = (&mut zones[0].inside, ());
            let mut taken = std::mem::take(inner);
            arrange(&mut taken, rect, level, false, on);
            zones[0].inside = taken;
            return;
        }
        _ => {}
    }
    let count = zones.len();
    let area: Vec<f64> = zones.iter().map(|zone| (taken_by(board, &zone.parts) + zone.more).max(1.0)).collect();
    let total: f64 = area.iter().sum();
    let mut zone_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (z, zone) in zones.iter().enumerate() {
        for &part in &zone.parts {
            zone_of.insert(part, z);
        }
    }
    let mut ties = tied(nets, &|part| zone_of.get(&part).copied());
    // (zones under the pour of one supply are tied by it: each of the smallest to every other, POURED nets in all)
    let mut kin_here: Vec<(BTreeMap<usize, f64>, f64)> = Vec::new();
    for kin in on.kin.iter().filter(|kin| kin.len() > 1) {
        let each = POURED / (kin.len() - 1) as f64;
        let mut here: BTreeMap<usize, f64> = BTreeMap::new();
        kin.iter().filter_map(|p| zone_of.get(p)).for_each(|&z| *here.entry(z).or_insert(0.0) += 1.0);
        let list: Vec<(usize, f64)> = here.iter().map(|(&z, &n)| (z, n)).collect();
        for (i, &(a, of_a)) in list.iter().enumerate() {
            for &(b, of_b) in &list[i + 1..] {
                *ties.entry((a, b)).or_insert(0.0) += each * of_a * of_b;
            }
        }
        kin_here.push((here, each));
    }
    let ties = ties;
    // Where each zone is pulled: by its fixed parts where they stand, by what it is tied to outside these zones
    // where that has been given ground, and by the others of these zones, each where it is pulled itself. The
    // place that all of it together asks for is found by going round until nothing moves much.
    let mut outside: Vec<Vec<(Pt, f64)>> = vec![Vec::new(); count];
    for parts in nets {
        let weight = 1.0 / (parts.len().max(2) - 1) as f64;
        let touched: BTreeSet<usize> = parts.iter().filter_map(|p| zone_of.get(p).copied()).collect();
        for &other in parts.iter().filter(|&&p| !zone_of.contains_key(&p)) {
            if let Some(there) = at[other] {
                let there = [there[0] - shift[0], there[1] - shift[1]];
                touched.iter().for_each(|&z| outside[z].push((there, weight)));
            }
        }
    }
    for (kin, (here, each)) in on.kin.iter().filter(|kin| kin.len() > 1).zip(&kin_here) {
        for there in kin.iter().filter(|p| !zone_of.contains_key(p)).filter_map(|&p| at[p]) {
            let there = [there[0] - shift[0], there[1] - shift[1]];
            here.iter().for_each(|(&z, &count)| outside[z].push((there, each * count)));
        }
    }
    for (&part, &z) in zone_of.iter().filter(|(&p, _)| board.parts[p].fixed && board.parts[p].placed) {
        outside[z].push((board.parts[part].at, ANCHOR));
    }
    // (and a part that is to stand near a point pulls its zone there)
    for (&part, &z) in &zone_of {
        if let Some(near) = board.parts[part].near {
            outside[z].push(([near[0] - shift[0], near[1] - shift[1]], ANCHOR));
        }
    }
    // (and a part that mates with another, to where that one is taken to stand)
    for (&part, &z) in &zone_of {
        let other = on.mates[part].filter(|other| !zone_of.contains_key(other));
        if let Some(there) = other.and_then(|other| at[other]) {
            outside[z].push(([there[0] - shift[0], there[1] - shift[1]], ANCHOR));
        }
    }
    // (a zone with parts that belong on an edge is pulled to the nearest point of the board's edge, as it goes)
    let on_edge: Vec<f64> = (0..count)
        .map(|z| zones[z].parts.iter().filter(|&&p| board.parts[p].edge).count().min(3) as f64 * ANCHOR)
        .collect();
    let to_edge = |p: Pt| -> Pt {
        let [x0, y0, x1, y1] = region;
        let ways = [(p[0] - x0, [x0, p[1]]), (x1 - p[0], [x1, p[1]]), (p[1] - y0, [p[0], y0]), (y1 - p[1], [p[0], y1])];
        ways.iter().fold(ways[0], |best, way| if way.0 < best.0 { *way } else { best }).1
    };
    let mut spot: Vec<Pt> = vec![middle(rect); count];
    for _ in 0..200 {
        let mut far = 0.0f64;
        for z in 0..count {
            let (mut sum, mut weights) = ([0.0, 0.0], 0.0);
            if on_edge[z] > 0.0 {
                let there = to_edge(spot[z]);
                sum = [on_edge[z] * there[0], on_edge[z] * there[1]];
                weights = on_edge[z];
            }
            for &(there, weight) in &outside[z] {
                sum = [sum[0] + weight * there[0], sum[1] + weight * there[1]];
                weights += weight;
            }
            for (&(a, b), &weight) in ties.iter().filter(|(&(a, b), _)| a == z || b == z) {
                let there = spot[if a == z { b } else { a }];
                sum = [sum[0] + weight * there[0], sum[1] + weight * there[1]];
                weights += weight;
            }
            if weights > 0.0 {
                let to = [sum[0] / weights, sum[1] / weights];
                far = far.max(dist(to, spot[z]));
                spot[z] = to;
            }
        }
        if far < 1e-3 {
            break;
        }
    }
    // The board is cut across its longer side, the zones in the order they are pulled along it, where the two
    // lots are nearest the same size; each lot gets the share of the board its zones need.
    // (across its longer side; or across the shorter where that was found the better, unless the ground is so
    // long that its pieces would be slivers)
    let (across, down) = (rect[2] - rect[0], rect[3] - rect[1]);
    let squarish = across.min(down) >= SQUARISH * across.max(down);
    let wide = (across >= down) != (squarish && on.crossed.contains(&cut_of(zones)));
    let along = usize::from(!wide);
    // Where nothing pulls the zones apart along that side (nothing fixed, nothing outside), they are put in the
    // order their ties alone give.
    let low = spot.iter().map(|p| p[along]).fold(f64::INFINITY, f64::min);
    let high = spot.iter().map(|p| p[along]).fold(f64::NEG_INFINITY, f64::max);
    let mut line: Vec<f64> = spot.iter().map(|p| p[along]).collect();
    if high - low < 1e-6 * (rect[2] - rect[0] + rect[3] - rect[1]) {
        line = in_line(count, &ties);
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| line[a].total_cmp(&line[b]).then(a.cmp(&b)));
    let mut best = (1usize, f64::INFINITY);
    let mut held = 0.0;
    for (k, &z) in order.iter().enumerate().take(count - 1) {
        held += area[z];
        let lean = (2.0 * held - total).abs();
        if lean < best.1 {
            best = (k + 1, lean);
        }
    }
    let mut first = vec![false; count];
    order[..best.0].iter().for_each(|&z| first[z] = true);
    // (a zone is in the other lot where that was found the better, as long as neither lot is left empty)
    let this_cut = cut_of(zones);
    let as_pulled = first.clone();
    for z in 0..count {
        if let Some(&side) = on.sided.get(&(this_cut.clone(), zones[z].name.clone())) {
            first[z] = side;
        }
    }
    if first.iter().all(|&f| f) || !first.iter().any(|&f| f) {
        first = as_pulled;
    }
    let share = (0..count).filter(|&z| first[z]).map(|z| area[z]).sum::<f64>() / total;
    // Between the two lots a road is kept free of parts: what crosses the cut runs along it to where it is
    // going. It is as wide as the tracks that cross take side by side on the layers there are (a supply as
    // several), and no more than a share of the piece that is cut.
    let crosses = |parts: &Vec<usize>| {
        let sides: BTreeSet<bool> = parts.iter().filter_map(|p| zone_of.get(p)).map(|&z| first[z]).collect();
        sides.len() == 2
    };
    // (a supply crosses by a track where a lot has parts of it and none of them on its pour, and the other lot
    // has any of it)
    let rail_crosses = |(fed, left): &(Vec<usize>, Vec<usize>)| {
        let sides = |parts: &Vec<usize>| {
            let mut found = [false; 2];
            parts.iter().filter_map(|p| zone_of.get(p)).for_each(|&z| found[usize::from(first[z])] = true);
            found
        };
        let (fed, left) = (sides(fed), sides(left));
        (0..2).any(|k| left[k] && !fed[k] && (fed[1 - k] || left[1 - k]))
    };
    let (signals, supplies) =
        (nets.iter().filter(|n| crosses(n)).count(), on.rails.iter().filter(|n| rail_crosses(n)).count());
    let length = if wide { rect[2] - rect[0] } else { rect[3] - rect[1] };
    let lanes =
        (signals as f64 + RAIL_LANES * supplies as f64).max(on.asked.get(&this_cut).copied().unwrap_or(0) as f64);
    // (a road along a cut across the board's width runs up and down, and so on: it has the layers that run so;
    // between two domains it is no road but the gap they keep)
    let run_layers = board.layers_that_run(wide);
    // (whole tracks: a road for three nets on four layers is one track wide, not three quarters of one; and
    // one track at the least along every cut, or a zone with nothing to its neighbour has no way out past it)
    let abreast = (lanes / run_layers as f64).ceil().max(1.0);
    let road = if on.fine {
        0.0
    } else {
        if domains { board.gap } else { board.pitch * abreast }.min(board.tune.road_most * length)
    };
    let low = if wide { rect[0] } else { rect[1] };
    let piece = |from: f64, to: f64| if wide { [from, rect[1], to, rect[3]] } else { [rect[0], from, rect[2], to] };
    // (the lots change sides where that was found the better)
    let flip = on.turned.contains(&this_cut);
    let aim = if flip { 1.0 - share } else { share };
    let mut cut = low + aim * (length - road);
    // (where no part may stand is no room: with such ground here, each lot has its share of what is left)
    let shut = |r: [f64; 4], k: &[f64; 4]| {
        (r[2].min(k[2]) - r[0].max(k[0])).max(0.0) * (r[3].min(k[3]) - r[1].max(k[1])).max(0.0)
    };
    let free =
        |r: [f64; 4]| ((r[2] - r[0]) * (r[3] - r[1]) - board.keepouts.iter().map(|k| shut(r, k)).sum::<f64>()).max(0.0);
    if board.keepouts.iter().any(|k| shut(rect, k) > 0.0) {
        let (mut from, mut till) = (low, low + length - road);
        for _ in 0..40 {
            let mid = (from + till) / 2.0;
            let (one, other) = (free(piece(low, mid)), free(piece(mid + road, low + length)));
            if one < aim * (one + other) {
                from = mid;
            } else {
                till = mid;
            }
        }
        cut = (from + till) / 2.0;
    }
    let (before, after) = (piece(low, cut), piece(cut + road, low + length));
    let (of_first, of_second) = if flip { (after, before) } else { (before, after) };
    if road > 0.0 {
        let names = |side: bool| (0..count).filter(|&z| first[z] == side).map(|z| zones[z].name.clone()).collect();
        on.roads.push(Road {
            rect: piece(cut, cut + road).map(|v| (v * 100.0).round() / 100.0),
            width: (road * 100.0).round() / 100.0,
            nets: signals,
            rails: supplies,
            level,
            between: [names(true), names(false)],
            up: wide,
            gap: domains,
            lanes: if domains { 0 } else { (road / board.pitch + 1e-6).floor() as usize * run_layers },
            used: 0,
            parts: Vec::new(),
            cut: this_cut,
        });
    }
    // (each lot stands at the middle of its half until its zones have their own)
    for (z, zone) in zones.iter().enumerate() {
        settle(zone, if first[z] { of_first } else { of_second }, at);
    }
    let mut lots: [Vec<Work>; 2] = [Vec::new(), Vec::new()];
    let mut order: Vec<(usize, usize)> = Vec::new();
    for (z, zone) in zones.iter_mut().enumerate() {
        let side = usize::from(!first[z]);
        order.push((side, lots[side].len()));
        lots[side].push(Work {
            name: std::mem::take(&mut zone.name),
            parts: std::mem::take(&mut zone.parts),
            inside: std::mem::take(&mut zone.inside),
            rect: zone.rect,
            more: zone.more,
        });
    }
    let [mut one, mut other] = lots;
    arrange(&mut one, of_first, level + 1, domains, on);
    arrange(&mut other, of_second, level + 1, domains, on);
    let mut back: [Vec<Option<Work>>; 2] = [one.into_iter().map(Some).collect(), other.into_iter().map(Some).collect()];
    for (zone, (side, k)) in zones.iter_mut().zip(order) {
        if let Some(done) = back[side][k].take() {
            *zone = done;
        }
    }
}
