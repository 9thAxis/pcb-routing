//! What the plan tells of the layout chosen: its junctions, what flows between its zones, and the zones.

use super::ground::Work;
use super::known::Known;
use super::settle::{median, middle};
use super::types::{Board, Junction, Road, Travel, Zone};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// Ground for the vias where ways turn: each takes its own and as much again, in a square where the roads meet.
pub(super) fn junctions(board: &Board, travel: &Travel) -> Vec<Junction> {
    let mut junctions: Vec<Junction> = travel
        .turns
        .iter()
        .map(|&(at, changes, has)| {
            let needs = 2.0 * changes as f64 * board.via * board.via;
            Junction {
                at: at.map(|v| (v * 100.0).round() / 100.0),
                changes,
                needs: (needs * 10.0).round() / 10.0,
                has: (has * 100.0).round() / 100.0,
                side: if needs > has { (needs.sqrt() * 10.0).round() / 10.0 } else { 0.0 },
            }
        })
        .collect();
    junctions
        .sort_by(|a, b| b.changes.cmp(&a.changes).then(a.at[0].total_cmp(&b.at[0])).then(a.at[1].total_cmp(&b.at[1])));
    junctions
}

/// What stands on each gap is told with it: the parts on a ground of either side.
pub(super) fn gap_parts(cx: &Known, roads: &mut [Road]) {
    let (parts, all_nets, found) = (cx.parts, cx.all_nets, &cx.found);
    for road in roads.iter_mut().filter(|road| road.gap) {
        let side = |p: usize, names: &Vec<String>| found.stands[p].iter().any(|&g| names.contains(&all_nets[g].name));
        let mut on_it: Vec<String> = (0..parts.len())
            .filter(|&p| side(p, &road.between[0]) && side(p, &road.between[1]))
            .map(|p| parts[p].name.clone())
            .collect();
        on_it.sort();
        road.parts = on_it;
    }
}

/// What flows between the zones just under the domains: for each two of them, the nets that reach both, and
/// how many of those are pairs.
pub(super) fn flows(cx: &Known, top: &[Work]) -> Vec<(String, String, usize, usize)> {
    let (all_nets, nets, signal) = (cx.all_nets, &cx.nets, &cx.signal);
    let mut home: BTreeMap<usize, String> = BTreeMap::new();
    for zone in top {
        if zone.inside.is_empty() {
            home.extend(zone.parts.iter().map(|&p| (p, zone.name.clone())));
        }
        for inner in &zone.inside {
            home.extend(inner.parts.iter().map(|&p| (p, format!("{}: {}", zone.name, inner.name))));
        }
    }
    let mut flows: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    for (k, net) in nets.iter().enumerate() {
        let reached: BTreeSet<&String> = net.iter().filter_map(|p| home.get(p)).collect();
        let reached: Vec<&String> = reached.into_iter().collect();
        for (i, a) in reached.iter().enumerate() {
            for b in &reached[i + 1..] {
                let entry = flows.entry(((*a).clone(), (*b).clone())).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += usize::from(all_nets[signal[k]].pair);
            }
        }
    }
    let mut flows: Vec<(String, String, usize, usize)> =
        flows.into_iter().map(|((a, b), (n, pairs))| (a, b, n, pairs / 2)).collect();
    flows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    flows
}

/// A zone as it is told, and those within it; the smallest are added to `leaves`, each with where its placed
/// parts are and where it is given ground, and `leaf_of` says which of them a part is of.
fn told(
    work: &Work,
    board: &Board,
    nets: &[Vec<usize>],
    leaf_of: &mut BTreeMap<usize, usize>,
    leaves: &mut Vec<(Option<Pt>, Pt)>,
) -> Zone {
    let parts = &board.parts;
    let stands: Vec<Pt> = work.parts.iter().filter(|&&p| parts[p].placed).map(|&p| parts[p].at).collect();
    let bounds = (!stands.is_empty()).then(|| {
        let low = |k: usize| stands.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let high = |k: usize| stands.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        [low(0), low(1), high(0), high(1)]
    });
    let r = work.rect;
    let at_home = stands.iter().filter(|p| p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]).count();
    let among: BTreeSet<usize> = work.parts.iter().copied().collect();
    if work.inside.is_empty() {
        leaf_of.extend(work.parts.iter().map(|&p| (p, leaves.len())));
        leaves.push((median(&stands), middle(r)));
    }
    let mut names: Vec<String> = work.parts.iter().map(|&p| parts[p].name.clone()).collect();
    names.sort();
    let sheets: BTreeSet<String> = work
        .parts
        .iter()
        .map(|&p| if parts[p].sheet.is_empty() { "(on no sheet)".to_string() } else { parts[p].sheet.clone() })
        .collect();
    Zone {
        name: work.name.clone(),
        sheets: sheets.into_iter().collect(),
        parts: names,
        placed: stands.len(),
        bounds,
        area: work.parts.iter().map(|&p| parts[p].area).sum::<f64>().round(),
        within: nets.iter().filter(|net| net.iter().all(|p| among.contains(p))).count(),
        inside: work.inside.iter().map(|inner| told(inner, board, nets, leaf_of, leaves)).collect(),
        rect: r.map(|v| (v * 100.0).round() / 100.0),
        at_home: if stands.is_empty() { 0.0 } else { (at_home as f64 / stands.len() as f64 * 1000.0).round() / 1000.0 },
        off: median(&stands).map(|m| (dist(m, middle(r)) * 10.0).round() / 10.0),
    }
}

/// The zones as they are told, and how far the parts stand from the ground their zone is given: how long the
/// nets between the smallest zones are with each zone where its parts stand today, and where it is given
/// ground.
pub(super) fn zones_told(cx: &Known, top: &[Work]) -> (Vec<Zone>, f64, f64) {
    let nets = &cx.nets;
    let mut leaf_of: BTreeMap<usize, usize> = BTreeMap::new();
    let mut leaves: Vec<(Option<Pt>, Pt)> = Vec::new(); // (where its placed parts are, where it is given ground)
    let zones: Vec<Zone> = top.iter().map(|work| told(work, cx.board, nets, &mut leaf_of, &mut leaves)).collect();
    let (mut now_mm, mut then_mm) = (0.0, 0.0);
    for net in nets {
        let reached: BTreeSet<usize> =
            net.iter().filter_map(|p| leaf_of.get(p).copied()).filter(|&l| leaves[l].0.is_some()).collect();
        if reached.len() < 2 {
            continue;
        }
        let span = |points: Vec<Pt>| {
            let low = |k: usize| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| points.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            high(0) - low(0) + high(1) - low(1)
        };
        now_mm += span(reached.iter().filter_map(|&l| leaves[l].0).collect());
        then_mm += span(reached.iter().map(|&l| leaves[l].1).collect());
    }
    (zones, now_mm, then_mm)
}
