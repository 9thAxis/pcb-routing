//! Ways along the roads for the nets that go from zone to zone.

use super::constants::{LOCAL_MM, THROUGH, VIA_MM};
use super::settle::middle;
use super::types::{Road, Travel};
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// Give each net that reaches several of the smallest zones (`leaves`: the domain, the ground and the parts of
/// each) a way along the roads, and say what that comes to; each road is told how many ways run along it
/// where it is fullest.
///
/// The roads are lines down their middles, joined where one ends on another; a zone comes onto every road
/// that runs along one of its sides, at the point nearest its middle. A way is the shortest from zone to
/// zone along those lines, a turn from one road into another counted as VIA_MM more (it is a change of
/// layer), a road that is full as four times its length, and a way under a zone that is not its end as
/// twice its length. A net of several zones is joined zone by
/// zone, each to the nearest of what is joined already. The gaps between domains are no roads.
pub(super) fn travelled(roads: &mut [Road], leaves: &[(usize, [f64; 4], &Vec<usize>)], nets: &[Vec<usize>]) -> Travel {
    const NEAR: f64 = 0.03;
    let key = |p: Pt| ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64);
    let mut ids: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    let mut points: Vec<Pt> = Vec::new();
    let node = |p: Pt, ids: &mut BTreeMap<(i64, i64), usize>, points: &mut Vec<Pt>| -> usize {
        *ids.entry(key(p)).or_insert_with(|| {
            points.push(p);
            points.len() - 1
        })
    };
    let line = |road: &Road| -> (f64, f64, f64) {
        // (where its middle line is, and from where to where it runs)
        let r = road.rect;
        if road.up {
            ((r[0] + r[2]) / 2.0, r[1], r[3])
        } else {
            ((r[1] + r[3]) / 2.0, r[0], r[2])
        }
    };
    let real: Vec<usize> = (0..roads.len()).filter(|&i| !roads[i].gap).collect();
    // the points on each road: where another meets it, and where a zone comes onto it
    let mut on_road: Vec<Vec<(f64, usize)>> = vec![Vec::new(); roads.len()];
    for (a, &i) in real.iter().enumerate() {
        for &j in &real[a + 1..] {
            if roads[i].up == roads[j].up {
                continue;
            }
            let (up, across) = if roads[i].up { (i, j) } else { (j, i) };
            let (x, y0, y1) = line(&roads[up]);
            let (y, x0, x1) = line(&roads[across]);
            let (ru, ra) = (roads[up].rect, roads[across].rect);
            // (they meet where the one's strip touches or crosses the other's)
            let meets =
                ra[0] <= ru[2] + NEAR && ra[2] >= ru[0] - NEAR && ru[1] <= ra[3] + NEAR && ru[3] >= ra[1] - NEAR;
            if meets
                && y >= y0 - roads[across].width
                && y <= y1 + roads[across].width
                && x >= x0 - roads[up].width
                && x <= x1 + roads[up].width
            {
                let at = node([x, y], &mut ids, &mut points);
                on_road[up].push((y, at));
                on_road[across].push((x, at));
            }
        }
    }
    let mut edges: Vec<Vec<(usize, f64, Option<usize>)>> = Vec::new(); // (to, length, the road it runs along)
    let mut zone_node: Vec<usize> = Vec::new();
    let mut access: Vec<Vec<(usize, f64)>> = vec![Vec::new(); leaves.len()];
    for (z, &(_, r, _)) in leaves.iter().enumerate() {
        let mid = middle(r);
        zone_node.push(node([mid[0] + 1e-4 * z as f64, mid[1] + 1e-4], &mut ids, &mut points));
        for &i in &real {
            let (at, from, to) = line(&roads[i]);
            let road = roads[i].rect;
            let (beside, low, high, along) = if roads[i].up {
                ((r[2] - road[0]).abs() < NEAR || (r[0] - road[2]).abs() < NEAR, r[1].max(from), r[3].min(to), mid[1])
            } else {
                ((r[3] - road[1]).abs() < NEAR || (r[1] - road[3]).abs() < NEAR, r[0].max(from), r[2].min(to), mid[0])
            };
            if !beside || high - low <= NEAR {
                continue;
            }
            let along = along.clamp(low, high);
            let p = if roads[i].up { [at, along] } else { [along, at] };
            let there = node(p, &mut ids, &mut points);
            on_road[i].push((along, there));
            access[z].push((there, (p[0] - mid[0]).abs() + (p[1] - mid[1]).abs()));
        }
    }
    edges.resize(points.len(), Vec::new());
    for (z, onto) in access.iter().enumerate() {
        for &(there, far) in onto {
            edges[zone_node[z]].push((there, far, None));
            edges[there].push((zone_node[z], far, None));
        }
    }
    for &i in &real {
        on_road[i].sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        on_road[i].dedup_by_key(|point| point.1);
        for pair in on_road[i].windows(2) {
            let far = (pair[1].0 - pair[0].0).abs();
            edges[pair[0].1].push((pair[1].1, far, Some(i)));
            edges[pair[1].1].push((pair[0].1, far, Some(i)));
        }
    }
    let mut leaf_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (z, (_, _, parts)) in leaves.iter().enumerate() {
        leaf_of.extend(parts.iter().map(|&p| (p, z)));
    }
    let about: Vec<(bool, usize)> = roads.iter().map(|road| (road.up, road.lanes)).collect();
    let count = roads.len();
    let mut fullest = vec![0usize; count];
    let mut used: BTreeMap<(usize, usize), usize> = BTreeMap::new(); // per stretch of road: the ways along it
    let mut travel = Travel::default();
    let widths: Vec<f64> = roads.iter().map(|road| road.width).collect();
    let mut turned_at: BTreeMap<usize, (usize, f64)> = BTreeMap::new();
    // (the way from any of `from` to the nearest of `to`: its nodes; a state is (node, the road come along))
    let way = |from: &BTreeSet<usize>,
               to: &BTreeSet<usize>,
               used: &BTreeMap<(usize, usize), usize>|
     -> Option<Vec<(usize, Option<usize>)>> {
        let slot = |node: usize, road: Option<usize>| node * (count + 1) + road.unwrap_or(count);
        let mut best: BTreeMap<usize, (f64, Option<usize>)> = BTreeMap::new(); // state -> (cost, the state before)
        let mut open: BTreeSet<(u64, usize)> = BTreeSet::new();
        for &start in from {
            best.insert(slot(start, None), (0.0, None));
            open.insert((0, slot(start, None)));
        }
        while let Some(&(at_cost, state)) = open.iter().next() {
            open.remove(&(at_cost, state));
            let (here, came) = (state / (count + 1), state % (count + 1));
            let cost = best[&state].0;
            if to.contains(&here) {
                let mut path = vec![(here, (came < count).then_some(came))];
                let mut back = best[&state].1;
                while let Some(before) = back {
                    let came = before % (count + 1);
                    path.push((before / (count + 1), (came < count).then_some(came)));
                    back = best[&before].1;
                }
                path.reverse();
                return Some(path);
            }
            for &(next, far, road) in &edges[here] {
                // (a zone is the end of a way; a way through one runs under its parts on the inner layers,
                // which is dearer than a road: THROUGH times as far)
                let under =
                    (zone_node.contains(&next) && !to.contains(&next)) || (zone_node.contains(&here) && cost > 0.0);
                let turn = match (came < count, road) {
                    (true, Some(r)) if about[came].0 != about[r].0 => VIA_MM,
                    _ => 0.0,
                };
                let full = road
                    .is_some_and(|r| used.get(&(here.min(next), here.max(next))).copied().unwrap_or(0) >= about[r].1);
                let then = cost
                    + turn
                    + if full {
                        4.0 * far
                    } else if under {
                        THROUGH * far
                    } else {
                        far
                    };
                let to_state = slot(next, road);
                if best.get(&to_state).map_or(true, |b| then < b.0 - 1e-9) {
                    best.insert(to_state, (then, Some(state)));
                    open.insert(((then * 1000.0) as u64, to_state));
                }
            }
        }
        None
    };
    for (number, parts) in nets.iter().enumerate() {
        let reached: BTreeSet<usize> = parts.iter().filter_map(|p| leaf_of.get(p).copied()).collect();
        if reached.len() < 2 {
            continue;
        }
        let domains: BTreeSet<usize> = reached.iter().map(|&z| leaves[z].0).collect();
        if domains.len() > 1 {
            travel.through += 1;
            continue;
        }
        if reached.iter().any(|&z| access[z].is_empty()) {
            travel.stranded += 1;
            travel.left.push(number);
            continue;
        }
        travel.nets += 1;
        let mut joined: BTreeSet<usize> = [zone_node[*reached.iter().next().unwrap()]].into_iter().collect();
        let mut left: BTreeSet<usize> = reached.iter().skip(1).map(|&z| zone_node[z]).collect();
        while !left.is_empty() {
            let Some(path) = way(&joined, &left, &used) else {
                travel.stranded += 1;
                travel.left.push(number);
                break;
            };
            let (mut along, mut turns, mut last) = (0.0, 0usize, None::<usize>);
            let mut turned: Vec<(usize, f64)> = Vec::new();
            for pair in path.windows(2) {
                let ((a, _), (b, road)) = (pair[0], pair[1]);
                let far = (points[a][0] - points[b][0]).abs() + (points[a][1] - points[b][1]).abs();
                travel.mm += far;
                if let Some(r) = road {
                    along += far;
                    *used.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                    let now = used[&(a.min(b), a.max(b))];
                    fullest[r] = fullest[r].max(now);
                    if let Some(before) = last.filter(|&before| about[before].0 != about[r].0) {
                        turns += 1;
                        turned.push((a, widths[before] * widths[r]));
                    }
                    last = Some(r);
                }
            }
            if along > LOCAL_MM {
                travel.vias += 2 + turns;
                for (node, ground) in turned {
                    turned_at.entry(node).or_insert((0, ground)).0 += 1;
                }
            }
            let end = path[path.len() - 1].0;
            left.remove(&end);
            joined.extend(path.iter().map(|step| step.0));
        }
    }
    for (road, &most) in roads.iter_mut().zip(&fullest) {
        road.used = most;
    }
    travel.turns = turned_at.into_iter().map(|(node, (changes, ground))| (points[node], changes, ground)).collect();
    travel.mm = travel.mm.round();
    travel.over = roads.iter().filter(|road| !road.gap && road.used > road.lanes).count();
    travel
}
