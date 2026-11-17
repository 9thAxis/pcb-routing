//! Where the parts of a board belong: its power domains, the zones within them, an area of the board for each,
//! and how far the parts stand from that today. Nothing here moves a part.
//!
//! A board is first of all its grounds: the parts on one ground are a domain, a part on two stands between
//! two domains. A supply is a net that two or more capacitors join straight to a ground, whatever it is
//! called. Within a domain, parts are tied by the signal nets they share (not by supplies and grounds, which
//! reach everything): the parts of a sheet of the schematic that are tied to each other are a group, and
//! groups are gathered into zones, and zones into larger ones, level by level, by the method of Blondel et
//! al. (Louvain), whatever sheet they are drawn on.
//!
//! Then each zone is given ground. Where a zone is pulled is worked out first: by its fixed parts (connectors)
//! where they stand, by the zones it is tied to, and by what already has its ground. The board is cut across
//! its longer side, the zones in the order they are pulled along it, into two lots as near the same size as
//! that order allows, each with the share of the board its zones need; each half is cut again, down to the
//! smallest zones.

use crate::error::{invalid, Error};
use crate::geom::dist;
use crate::model::Pt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A part of the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// Its reference.
    pub name: String,
    /// The sheet of the schematic it is drawn on.
    #[serde(default)]
    pub sheet: String,
    /// Where it stands.
    pub at: Pt,
    /// Whether it stands on the board (a part not placed yet stands off it).
    pub placed: bool,
    /// Whether it is to stay where it stands: it pulls its zone to itself.
    #[serde(default)]
    pub fixed: bool,
    /// Whether it belongs on an edge of the board (a connector, a switch: what is plugged into or pressed).
    /// It moves with its zone, and pulls the zone to the nearest edge.
    #[serde(default)]
    pub edge: bool,
    /// Whether it is a capacitor: two of them from a net straight to a ground make the net a supply.
    #[serde(default)]
    pub capacitor: bool,
    /// The ground it takes on the board, in mm2.
    pub area: f64,
    /// The net of each of its pads, as an index into [`Board::nets`]; None for a pad on no net.
    pub pads: Vec<Option<usize>>,
}

/// A net of the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// Its name.
    pub name: String,
    /// Whether it is a supply by its name (one found by its capacitors need not be said).
    #[serde(default)]
    pub supply: bool,
    /// Whether it is one half of a differential pair.
    #[serde(default)]
    pub pair: bool,
}

/// A board, as far as its floorplan needs it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Board {
    /// The board: [x0, y0, x1, y1].
    #[serde(rename = "box")]
    pub outline: [f64; 4],
    /// Its parts.
    pub parts: Vec<Part>,
    /// Its nets.
    pub nets: Vec<Net>,
}

/// A supply: a net that feeds parts.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Rail {
    /// The net.
    pub net: String,
    /// Its parts.
    pub parts: Vec<String>,
    /// The capacitors from it straight to its ground.
    pub capacitors: usize,
}

/// The parts on one ground.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Domain {
    /// The ground.
    pub ground: String,
    /// Its parts: those on this ground alone, or mostly (see [`Power::between`]).
    pub parts: Vec<String>,
    /// Its supplies, the one with the most parts first.
    pub rails: Vec<Rail>,
}

/// The power of a board.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Power {
    /// The domains, the largest first.
    pub domains: Vec<Domain>,
    /// The parts that stand on two grounds or more, each with them: what joins two domains.
    pub between: Vec<(String, Vec<String>)>,
    /// Two supplies with the parts that stand on both and are no capacitors: one feeds the other through them.
    pub links: Vec<(String, String, Vec<String>)>,
    /// The parts that no ground and no supply places in a domain.
    pub adrift: Vec<String>,
}

/// Parts that belong together on the board: a group of a sheet's parts, zones that are tied together, or a
/// domain.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Zone {
    /// What it is called: its ground, or the sheet most of its parts are drawn on.
    pub name: String,
    /// The sheets its parts are drawn on.
    pub sheets: Vec<String>,
    /// Its parts.
    pub parts: Vec<String>,
    /// How many of them stand on the board.
    pub placed: usize,
    /// The box of those: [x0, y0, x1, y1]; None with none.
    #[serde(rename = "box")]
    pub bounds: Option<[f64; 4]>,
    /// What its parts take, in mm2: the least it needs.
    pub area: f64,
    /// The signal nets that stay within it.
    pub within: usize,
    /// The zones it is made of, the largest first.
    pub inside: Vec<Zone>,
    /// The ground it is given: [x0, y0, x1, y1].
    pub rect: [f64; 4],
    /// The share of its placed parts that stand within that ground today.
    pub at_home: f64,
    /// How far the middle of its placed parts is from the middle of that ground, in mm; None with none placed.
    pub off: Option<f64>,
}

/// A sheet whose groups are not gathered together: its name, and each group (its parts) with the zone it is
/// gathered into.
pub type Apart = (String, Vec<(Vec<String>, String)>);

/// What [`floorplan`] finds.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Floorplan {
    /// The power.
    pub power: Power,
    /// The domains as zones, each with the zones within it; and the parts on no ground, if any, as one more.
    pub zones: Vec<Zone>,
    /// What flows between the zones just under the domains: (zone, zone, nets, of which pairs), the heaviest
    /// first. A zone is named `GROUND: NAME`.
    pub flows: Vec<(String, String, usize, usize)>,
    /// The sheets whose groups are not gathered together at the first level, each with its groups and the
    /// zone each is gathered into: drawn as one, tied apart.
    pub apart: Vec<Apart>,
    /// The length of the signal nets between the smallest zones, with each zone at the middle of its placed
    /// parts as they stand, in mm (zones with no part placed are left out).
    pub now_mm: f64,
    /// And the same with each of those zones at the middle of the ground it is given.
    pub then_mm: f64,
}

/// Capacitors from a net straight to a ground that make it a supply.
const BYPASSED: usize = 2;
/// What a fixed part pulls its zone with, as so many nets.
const ANCHOR: f64 = 4.0;

/// Whether a net is a ground by its name: GND, VSS, 0V or GROUND as a word of it (VIB_GND, FIELD_0V, AGND1).
pub fn is_ground(name: &str) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        let word = word.to_ascii_uppercase();
        let named = |w: &str| matches!(w, "GND" | "VSS" | "0V" | "GROUND");
        let bare = word.trim_end_matches(|c: char| c.is_ascii_digit());
        let kind = |w: &str| named(w) || (w.len() > 1 && matches!(&w[..1], "A" | "D" | "P") && named(&w[1..]));
        kind(&word) || kind(bare)
    })
}

/// Groups of nodes after one round of local moves: each group, as one, goes to the neighbouring group that
/// raises the modularity most, until none moves. `ties`: the weight between two nodes, the lower first. The
/// groups that end together are one, in the order of the first.
pub fn gathered(groups: &[Vec<usize>], ties: &BTreeMap<(usize, usize), f64>) -> Vec<Vec<usize>> {
    let mut index: BTreeMap<usize, usize> = BTreeMap::new();
    for (g, members) in groups.iter().enumerate() {
        for &node in members {
            index.insert(node, g);
        }
    }
    let mut between: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for (&(a, b), &weight) in ties {
        if let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) {
            *between.entry((i.min(j), i.max(j))).or_insert(0.0) += weight;
        }
    }
    let home = moved(groups.len(), &between);
    let mut merged: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (g, &to) in home.iter().enumerate() {
        merged.entry(to).or_default().push(g);
    }
    // (in the order of the first group of each)
    let mut lots: Vec<Vec<usize>> = merged.into_values().collect();
    lots.sort_by_key(|lot| lot[0]);
    lots.into_iter().map(|lot| lot.into_iter().flat_map(|g| groups[g].iter().copied()).collect()).collect()
}

/// Nodes in groups that are tied more within than without, the largest first: rounds of [`gathered`], each on
/// the groups of the round before, until a round changes nothing. The order of the nodes decides between
/// equals, so the same question has the same answer.
pub fn communities(nodes: &[usize], ties: &BTreeMap<(usize, usize), f64>) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = nodes.iter().map(|&node| vec![node]).collect();
    loop {
        let found = gathered(&groups, ties);
        if found.len() == groups.len() {
            let place = |node: usize| nodes.iter().position(|&n| n == node).unwrap_or(usize::MAX);
            groups.sort_by_key(|members| (usize::MAX - members.len(), place(members[0])));
            return groups;
        }
        groups = found;
    }
}

/// Which group each of `count` nodes ends in after local moves; `between`: the weight between two of them, the
/// lower first (a node with itself: what it, a group of an earlier round, has within).
fn moved(count: usize, between: &BTreeMap<(usize, usize), f64>) -> Vec<usize> {
    let mut beside: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); count];
    let mut degree = vec![0.0f64; count];
    for (&(i, j), &weight) in between {
        if i == j {
            degree[i] += 2.0 * weight;
        } else {
            *beside[i].entry(j).or_insert(0.0) += weight;
            *beside[j].entry(i).or_insert(0.0) += weight;
            degree[i] += weight;
            degree[j] += weight;
        }
    }
    let total: f64 = degree.iter().sum();
    let mut home: Vec<usize> = (0..count).collect();
    if total <= 0.0 {
        return home;
    }
    let mut weight_of = degree.clone();
    for _ in 0..100 {
        let mut changed = false;
        for node in 0..count {
            let here = home[node];
            let mut to: BTreeMap<usize, f64> = BTreeMap::new();
            for (&other, &weight) in &beside[node] {
                *to.entry(home[other]).or_insert(0.0) += weight;
            }
            weight_of[here] -= degree[node];
            let gain = |group: usize| to.get(&group).copied().unwrap_or(0.0) - weight_of[group] * degree[node] / total;
            let mut best = (here, f64::NEG_INFINITY);
            for &group in to.keys() {
                if gain(group) > best.1 {
                    best = (group, gain(group));
                }
            }
            let to_group = if best.1 > gain(here) + 1e-12 { best.0 } else { here };
            weight_of[to_group] += degree[node];
            if to_group != here {
                home[node] = to_group;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    home
}

/// Ties between things from nets (each the parts it reaches; `of`: the thing a part is of, if any): a net of k
/// things ties each two of them by 1 / (k - 1), so that it counts once for each thing it reaches.
fn tied(nets: &[Vec<usize>], of: &dyn Fn(usize) -> Option<usize>) -> BTreeMap<(usize, usize), f64> {
    let mut ties = BTreeMap::new();
    for parts in nets {
        let things: BTreeSet<usize> = parts.iter().filter_map(|&part| of(part)).collect();
        let things: Vec<usize> = things.into_iter().collect();
        for (i, &a) in things.iter().enumerate() {
            for &b in &things[i + 1..] {
                *ties.entry((a, b)).or_insert(0.0) += 1.0 / (things.len() - 1) as f64;
            }
        }
    }
    ties
}

/// What the power analysis finds, by index: the grounds (net indices), the rails with the ground of each, the
/// ground a part is counted with, and the grounds each part stands on.
struct Powered {
    home: Vec<Option<usize>>,
    stands: Vec<BTreeSet<usize>>,
    rails: BTreeMap<usize, (Option<usize>, usize)>,
    grounds: Vec<usize>,
}

fn powered(board: &Board) -> Powered {
    let (parts, nets) = (&board.parts, &board.nets);
    let ground: Vec<bool> = nets.iter().map(|net| is_ground(&net.name)).collect();
    let mut parts_on: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nets.len()];
    for (p, part) in parts.iter().enumerate() {
        for &net in part.pads.iter().flatten() {
            parts_on[net].insert(p);
        }
    }
    // A capacitor from a net straight to a ground bypasses the net.
    let mut bypass: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); nets.len()];
    for part in parts.iter().filter(|part| part.capacitor && part.pads.len() == 2) {
        if let [Some(a), Some(b)] = part.pads[..] {
            for (net, other) in [(a, b), (b, a)] {
                if a != b && ground[other] && !ground[net] {
                    *bypass[net].entry(other).or_insert(0) += 1;
                }
            }
        }
    }
    let mut rails: BTreeMap<usize, (Option<usize>, usize)> = BTreeMap::new();
    for net in (0..nets.len()).filter(|&net| !ground[net]) {
        let mut most: Option<(usize, usize)> = None;
        for (&to, &count) in &bypass[net] {
            if most.map_or(true, |m| count > m.1) {
                most = Some((to, count));
            }
        }
        if most.is_some_and(|m| m.1 >= BYPASSED) || nets[net].supply {
            rails.insert(net, (most.map(|m| m.0), most.map_or(0, |m| m.1)));
        }
    }
    // A part is on the grounds it has a pad on; without one, on those of the supplies it has a pad on;
    let mut stands: Vec<BTreeSet<usize>> = Vec::new();
    for part in parts {
        let own: BTreeSet<usize> = part.pads.iter().flatten().copied().filter(|&net| ground[net]).collect();
        let by_rail = || part.pads.iter().flatten().filter_map(|net| rails.get(net).and_then(|rail| rail.0)).collect();
        stands.push(if own.is_empty() { by_rail() } else { own });
    }
    // and what neither says, on the ground of most of the parts it shares a net with, as far as that goes.
    for _ in 0..10 {
        let mut found: Vec<(usize, usize)> = Vec::new();
        for p in (0..parts.len()).filter(|&p| stands[p].is_empty()) {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            let own: BTreeSet<usize> = parts[p].pads.iter().flatten().copied().collect();
            for &net in &own {
                for &other in &parts_on[net] {
                    for &g in &stands[other] {
                        *count.entry(g).or_insert(0) += 1;
                    }
                }
            }
            let mut best: Option<(usize, usize)> = None;
            for (&g, &n) in &count {
                if best.map_or(true, |b| n > b.1) {
                    best = Some((g, n));
                }
            }
            if let Some((g, _)) = best {
                found.push((p, g));
            }
        }
        if found.is_empty() {
            break;
        }
        for (p, g) in found {
            stands[p].insert(g);
        }
    }
    // A part on several grounds is counted with the one most of its pads are on, or lead to.
    let home: Vec<Option<usize>> = (0..parts.len())
        .map(|p| {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            for net in parts[p].pads.iter().flatten() {
                let g = if ground[*net] { Some(*net) } else { rails.get(net).and_then(|rail| rail.0) };
                if let Some(g) = g.filter(|g| stands[p].contains(g)) {
                    *count.entry(g).or_insert(0) += 1;
                }
            }
            let mut best: Option<(usize, usize)> = None;
            for &g in &stands[p] {
                let n = count.get(&g).copied().unwrap_or(0);
                if best.map_or(true, |b| n > b.1) {
                    best = Some((g, n));
                }
            }
            best.map(|b| b.0)
        })
        .collect();
    let mut grounds: Vec<usize> = (0..nets.len()).filter(|&net| ground[net]).collect();
    grounds.sort_by(|&a, &b| nets[a].name.cmp(&nets[b].name));
    Powered { home, stands, rails, grounds }
}

/// A zone while it is worked out: its parts by index.
struct Work {
    name: String,
    parts: Vec<usize>,
    inside: Vec<Work>,
    rect: [f64; 4],
}

fn middle(rect: [f64; 4]) -> Pt {
    [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0]
}

/// The middle value of each coordinate of some points.
fn median(points: &[Pt]) -> Option<Pt> {
    if points.is_empty() {
        return None;
    }
    let of = |k: usize| {
        let mut values: Vec<f64> = points.iter().map(|p| p[k]).collect();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    Some([of(0), of(1)])
}

/// Give the zones of `zones` ground within `rect`, and theirs within that (see the module). `at`: where each
/// part is taken to stand so far (a fixed part where it stands; another at the middle of the ground its zone
/// has been given, once it has).
fn arrange(zones: &mut [Work], rect: [f64; 4], board: &Board, nets: &[Vec<usize>], at: &mut Vec<Option<Pt>>) {
    let settle = |zone: &Work, rect: [f64; 4], at: &mut Vec<Option<Pt>>| {
        for &part in &zone.parts {
            if !(board.parts[part].fixed && board.parts[part].placed) {
                at[part] = Some(middle(rect));
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
            arrange(&mut taken, rect, board, nets, at);
            zones[0].inside = taken;
            return;
        }
        _ => {}
    }
    let count = zones.len();
    let area: Vec<f64> =
        zones.iter().map(|zone| zone.parts.iter().map(|&p| board.parts[p].area).sum::<f64>().max(1.0)).collect();
    let total: f64 = area.iter().sum();
    let mut zone_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (z, zone) in zones.iter().enumerate() {
        for &part in &zone.parts {
            zone_of.insert(part, z);
        }
    }
    let ties = tied(nets, &|part| zone_of.get(&part).copied());
    // Where each zone is pulled: by its fixed parts where they stand, by what it is tied to outside these zones
    // where that has been given ground, and by the others of these zones, each where it is pulled itself. The
    // place that all of it together asks for is found by going round until nothing moves much.
    let mut outside: Vec<Vec<(Pt, f64)>> = vec![Vec::new(); count];
    for parts in nets {
        let weight = 1.0 / (parts.len().max(2) - 1) as f64;
        let touched: BTreeSet<usize> = parts.iter().filter_map(|p| zone_of.get(p).copied()).collect();
        for &other in parts.iter().filter(|&&p| !zone_of.contains_key(&p)) {
            if let Some(there) = at[other] {
                touched.iter().for_each(|&z| outside[z].push((there, weight)));
            }
        }
    }
    for (&part, &z) in zone_of.iter().filter(|(&p, _)| board.parts[p].fixed && board.parts[p].placed) {
        outside[z].push((board.parts[part].at, ANCHOR));
    }
    // (a zone with parts that belong on an edge is pulled to the nearest point of the board's edge, as it goes)
    let on_edge: Vec<f64> = (0..count)
        .map(|z| zones[z].parts.iter().filter(|&&p| board.parts[p].edge).count().min(3) as f64 * ANCHOR)
        .collect();
    let to_edge = |p: Pt| -> Pt {
        let [x0, y0, x1, y1] = board.outline;
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
    let wide = rect[2] - rect[0] >= rect[3] - rect[1];
    let along = usize::from(!wide);
    // Where nothing pulls the zones apart along that side (nothing fixed, nothing outside), they are put in the
    // order their ties alone give: each goes half way to the middle of those it is tied to, over and over, with
    // the middle of all taken out, which leaves those tied to each other side by side.
    let low = spot.iter().map(|p| p[along]).fold(f64::INFINITY, f64::min);
    let high = spot.iter().map(|p| p[along]).fold(f64::NEG_INFINITY, f64::max);
    let mut line: Vec<f64> = spot.iter().map(|p| p[along]).collect();
    if high - low < 1e-6 * (rect[2] - rect[0] + rect[3] - rect[1]) {
        line = (0..count).map(|z| z as f64).collect();
        for _ in 0..500 {
            let before = line.clone();
            for z in 0..count {
                let (mut sum, mut weights) = (0.0, 0.0);
                for (&(a, b), &weight) in ties.iter().filter(|(&(a, b), _)| a == z || b == z) {
                    sum += weight * before[if a == z { b } else { a }];
                    weights += weight;
                }
                if weights > 0.0 {
                    line[z] = (before[z] + sum / weights) / 2.0;
                }
            }
            let mean = line.iter().sum::<f64>() / count as f64;
            let size = line.iter().map(|v| (v - mean).abs()).fold(0.0, f64::max).max(1e-12);
            line.iter_mut().for_each(|v| *v = (*v - mean) / size);
        }
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
    let share = (0..count).filter(|&z| first[z]).map(|z| area[z]).sum::<f64>() / total;
    let (of_first, of_second) = if wide {
        let x = rect[0] + share * (rect[2] - rect[0]);
        ([rect[0], rect[1], x, rect[3]], [x, rect[1], rect[2], rect[3]])
    } else {
        let y = rect[1] + share * (rect[3] - rect[1]);
        ([rect[0], rect[1], rect[2], y], [rect[0], y, rect[2], rect[3]])
    };
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
        });
    }
    let [mut one, mut other] = lots;
    arrange(&mut one, of_first, board, nets, at);
    arrange(&mut other, of_second, board, nets, at);
    let mut back: [Vec<Option<Work>>; 2] = [one.into_iter().map(Some).collect(), other.into_iter().map(Some).collect()];
    for (zone, (side, k)) in zones.iter_mut().zip(order) {
        if let Some(done) = back[side][k].take() {
            *zone = done;
        }
    }
}

/// The floorplan of a board (see the module). An error if a pad names a net that is not there, or the board
/// has no size.
pub fn floorplan(board: &Board) -> Result<Floorplan, Error> {
    let (parts, all_nets) = (&board.parts, &board.nets);
    let [x0, y0, x1, y1] = board.outline;
    if !(x1 > x0 && y1 > y0) {
        return Err(invalid("box", "has no size"));
    }
    for (p, part) in parts.iter().enumerate() {
        if part.pads.iter().flatten().any(|&net| net >= all_nets.len()) {
            return Err(invalid(format!("parts[{p}].pads"), format!("names a net of {}", all_nets.len())));
        }
    }
    let found = powered(board);
    let name = |p: usize| parts[p].name.clone();
    let names = |list: &[usize]| -> Vec<String> {
        let mut out: Vec<String> = list.iter().map(|&p| name(p)).collect();
        out.sort();
        out
    };
    let net_name = |net: usize| all_nets[net].name.clone();

    // The signal nets: what is no ground and no supply, each as the parts it reaches.
    let mut reach: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); all_nets.len()];
    for (p, part) in parts.iter().enumerate() {
        for &net in part.pads.iter().flatten() {
            reach[net].insert(p);
        }
    }
    let is_signal = |net: usize| !found.grounds.contains(&net) && !found.rails.contains_key(&net);
    let signal: Vec<usize> = (0..all_nets.len()).filter(|&net| is_signal(net) && reach[net].len() > 1).collect();
    let nets: Vec<Vec<usize>> = signal.iter().map(|&net| reach[net].iter().copied().collect()).collect();
    let between_parts = tied(&nets, &Some);

    // The power, as it is told.
    let mut domains: Vec<(usize, Vec<usize>)> = found
        .grounds
        .iter()
        .map(|&g| (g, (0..parts.len()).filter(|&p| found.home[p] == Some(g)).collect::<Vec<usize>>()))
        .filter(|(_, own)| !own.is_empty())
        .collect();
    domains.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(net_name(a.0).cmp(&net_name(b.0))));
    let adrift: Vec<usize> = (0..parts.len()).filter(|&p| found.home[p].is_none()).collect();
    let rails_of = |g: usize| -> Vec<Rail> {
        let mut rails: Vec<Rail> = found
            .rails
            .iter()
            .filter(|(_, rail)| rail.0 == Some(g))
            .map(|(&net, rail)| Rail {
                net: net_name(net),
                parts: names(&reach[net].iter().copied().collect::<Vec<_>>()),
                capacitors: rail.1,
            })
            .collect();
        rails.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.net.cmp(&b.net)));
        rails
    };
    let mut links: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (p, part) in parts.iter().enumerate().filter(|(_, part)| !part.capacitor) {
        let on: BTreeSet<String> =
            part.pads.iter().flatten().filter(|net| found.rails.contains_key(net)).map(|&net| net_name(net)).collect();
        let on: Vec<String> = on.into_iter().collect();
        for (i, a) in on.iter().enumerate() {
            for b in &on[i + 1..] {
                links.entry((a.clone(), b.clone())).or_default().push(name(p));
            }
        }
    }
    let mut between: Vec<(String, Vec<String>)> = (0..parts.len())
        .filter(|&p| found.stands[p].len() > 1)
        .map(|p| {
            let mut on: Vec<String> = found.stands[p].iter().map(|&g| net_name(g)).collect();
            on.sort();
            (name(p), on)
        })
        .collect();
    between.sort();
    let power = Power {
        domains: domains
            .iter()
            .map(|(g, own)| Domain { ground: net_name(*g), parts: names(own), rails: rails_of(*g) })
            .collect(),
        between,
        links: links
            .into_iter()
            .map(|((a, b), mut by)| {
                (a, b, {
                    by.sort();
                    by
                })
            })
            .collect(),
        adrift: names(&adrift),
    };

    // The zones within a lot of parts: the groups within each sheet, gathered level by level.
    let sheet = |p: usize| if parts[p].sheet.is_empty() { "(on no sheet)".to_string() } else { parts[p].sheet.clone() };
    let sheet_size = |name: &str| (0..parts.len()).filter(|&p| sheet(p) == name).count();
    let mut apart: Vec<Apart> = Vec::new();
    let mut within = |lot: &[usize]| -> Vec<Work> {
        let mine: BTreeSet<usize> = lot.iter().copied().collect();
        let mut sheets: Vec<String> = mine.iter().map(|&p| sheet(p)).collect::<BTreeSet<_>>().into_iter().collect();
        sheets.sort();
        let mut atoms: Vec<(String, Vec<usize>)> = Vec::new();
        let mut atom_of: BTreeMap<usize, usize> = BTreeMap::new();
        for of_sheet in &sheets {
            let mut own: Vec<usize> = mine.iter().copied().filter(|&p| &sheet(p) == of_sheet).collect();
            own.sort_by_key(|&p| name(p));
            let here: BTreeSet<usize> = own.iter().copied().collect();
            let ties: BTreeMap<(usize, usize), f64> = between_parts
                .iter()
                .filter(|(&(a, b), _)| here.contains(&a) && here.contains(&b))
                .map(|(&k, &w)| (k, w))
                .collect();
            let is_tied: BTreeSet<usize> = ties.keys().flat_map(|&(a, b)| [a, b]).collect();
            let nodes: Vec<usize> = own.iter().copied().filter(|p| is_tied.contains(p)).collect();
            let mut groups = communities(&nodes, &ties);
            if groups.is_empty() {
                groups.push(Vec::new());
            }
            groups[0].extend(own.iter().copied().filter(|p| !is_tied.contains(p)));
            for mut group in groups {
                group.sort_by_key(|&p| name(p));
                for &p in &group {
                    atom_of.insert(p, atoms.len());
                }
                atoms.push((of_sheet.clone(), group));
            }
        }
        // (a zone by the sheet most of its parts are drawn on; and by its first parts, if it is not all of it)
        let named = |members: &[usize]| -> String {
            let mut count: BTreeMap<&str, usize> = BTreeMap::new();
            for &atom in members {
                *count.entry(atoms[atom].0.as_str()).or_insert(0) += atoms[atom].1.len();
            }
            let mut most: (&str, usize) = ("", 0);
            for (&of_sheet, &n) in &count {
                if n > most.1 {
                    most = (of_sheet, n);
                }
            }
            let mut of_it: Vec<String> = members
                .iter()
                .filter(|&&atom| atoms[atom].0 == most.0)
                .flat_map(|&atom| names(&atoms[atom].1))
                .collect();
            of_it.sort();
            let some = if most.1 == sheet_size(most.0) {
                String::new()
            } else {
                format!(" ({}{})", of_it[..of_it.len().min(3)].join(", "), if of_it.len() > 3 { " ..." } else { "" })
            };
            let more = count.len() - 1;
            format!("{}{}{}", most.0, some, if more > 0 { format!(" and {more} more") } else { String::new() })
        };
        let local: Vec<Vec<usize>> =
            nets.iter().map(|net| net.iter().copied().filter(|p| mine.contains(p)).collect()).collect();
        let ties = tied(&local, &|p| atom_of.get(&p).copied());
        let mut levels: Vec<Vec<Vec<usize>>> =
            vec![(0..atoms.len()).filter(|&a| !atoms[a].1.is_empty()).map(|a| vec![a]).collect()];
        loop {
            let last = &levels[levels.len() - 1];
            let next = gathered(last, &ties);
            if next.len() == last.len() || next.len() < 2 {
                break;
            }
            levels.push(next);
        }
        let parts_of = |members: &[usize]| -> Vec<usize> {
            members.iter().flat_map(|&atom| atoms[atom].1.iter().copied()).collect()
        };
        let blank = [0.0; 4];
        let mut kept: Vec<(Vec<usize>, Work)> = levels[0]
            .iter()
            .map(|members| {
                (
                    members.clone(),
                    Work { name: named(members), parts: parts_of(members), inside: Vec::new(), rect: blank },
                )
            })
            .collect();
        for level in &levels[1..] {
            let mut made: Vec<(Vec<usize>, Work)> = Vec::new();
            for members in level {
                let of: BTreeSet<usize> = members.iter().copied().collect();
                let (mut inside, rest): (Vec<_>, Vec<_>) =
                    kept.into_iter().partition(|(under, _)| under.iter().all(|a| of.contains(a)));
                kept = rest;
                if inside.len() == 1 {
                    made.push((members.clone(), inside.remove(0).1));
                } else {
                    let mut inner: Vec<Work> = inside.into_iter().map(|(_, work)| work).collect();
                    inner.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.name.cmp(&b.name)));
                    made.push((
                        members.clone(),
                        Work { name: named(members), parts: parts_of(members), inside: inner, rect: blank },
                    ));
                }
            }
            kept = made;
        }
        // A sheet whose groups are not gathered together at the first level: drawn as one, tied apart.
        let first = if levels.len() > 1 { &levels[1] } else { &levels[0] };
        let mut first_of: BTreeMap<usize, usize> = BTreeMap::new();
        for (g, members) in first.iter().enumerate() {
            for &atom in members {
                first_of.insert(atom, g);
            }
        }
        for of_sheet in &sheets {
            let own: Vec<usize> =
                (0..atoms.len()).filter(|&a| &atoms[a].0 == of_sheet && !atoms[a].1.is_empty()).collect();
            let into: BTreeSet<usize> = own.iter().map(|a| first_of[a]).collect();
            if into.len() > 1 {
                apart.push((
                    of_sheet.clone(),
                    own.iter().map(|&a| (names(&atoms[a].1), named(&first[first_of[&a]]))).collect(),
                ));
            }
        }
        let mut zones: Vec<Work> = kept.into_iter().map(|(_, work)| work).collect();
        zones.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.name.cmp(&b.name)));
        zones
    };
    let mut top: Vec<Work> = Vec::new();
    for (g, own) in &domains {
        let inside = within(own);
        top.push(Work { name: net_name(*g), parts: own.clone(), inside, rect: [0.0; 4] });
    }
    if !adrift.is_empty() {
        let inside = within(&adrift);
        top.push(Work { name: "(on no ground)".to_string(), parts: adrift.clone(), inside, rect: [0.0; 4] });
    }

    // Ground for each zone, from the whole board down.
    let mut at: Vec<Option<Pt>> = parts.iter().map(|part| (part.fixed && part.placed).then_some(part.at)).collect();
    arrange(&mut top, board.outline, board, &nets, &mut at);

    // What flows between the zones just under the domains.
    let mut home: BTreeMap<usize, String> = BTreeMap::new();
    for zone in &top {
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

    // How far the parts stand from the ground their zone is given.
    let mut leaf_of: BTreeMap<usize, usize> = BTreeMap::new();
    let mut leaves: Vec<(Option<Pt>, Pt)> = Vec::new(); // (where its placed parts are, where it is given ground)
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
            at_home: if stands.is_empty() {
                0.0
            } else {
                (at_home as f64 / stands.len() as f64 * 1000.0).round() / 1000.0
            },
            off: median(&stands).map(|m| (dist(m, middle(r)) * 10.0).round() / 10.0),
        }
    }
    let zones: Vec<Zone> = top.iter().map(|work| told(work, board, &nets, &mut leaf_of, &mut leaves)).collect();
    let (mut now_mm, mut then_mm) = (0.0, 0.0);
    for net in &nets {
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
    Ok(Floorplan { power, zones, flows, apart, now_mm: now_mm.round(), then_mm: then_mm.round() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ties(of: &[(usize, usize, f64)]) -> BTreeMap<(usize, usize), f64> {
        of.iter().map(|&(a, b, w)| ((a, b), w)).collect()
    }

    #[test]
    fn a_ground_is_a_net_named_as_one() {
        for name in ["GND", "VIB_GND", "FIELD_0V", "AGND", "DGND1", "/power/GND", "VSS", "HOST_GND"] {
            assert!(is_ground(name), "{name}");
        }
        for name in ["VIB_3V3", "GNDX", "SIGNAL", "10V", "AGNDA", "GROUNDED", "V0V1"] {
            assert!(!is_ground(name), "{name}");
        }
    }

    #[test]
    fn nodes_tied_more_within_than_without_are_a_group() {
        // two triangles joined by one weak tie
        let two = ties(&[(0, 1, 1.0), (0, 2, 1.0), (1, 2, 1.0), (3, 4, 1.0), (3, 5, 1.0), (4, 5, 1.0), (2, 3, 0.2)]);
        assert_eq!(communities(&[0, 1, 2, 3, 4, 5], &two), [vec![0, 1, 2], vec![3, 4, 5]]);
        assert_eq!(communities(&[5, 4, 3, 2, 1, 0], &two), [vec![5, 4, 3], vec![2, 1, 0]]);
        // nothing tied: every node by itself; and one round gathers what is tied
        assert_eq!(communities(&[7, 8], &BTreeMap::new()), [vec![7], vec![8]]);
        assert_eq!(gathered(&[vec![0], vec![1], vec![2]], &ties(&[(0, 1, 1.0)])), [vec![0, 1], vec![2]]);
    }

    /// A chip on each of two grounds with its supply and bypass capacitors, an isolator on both, a regulator
    /// from one supply to another, and a filtered signal with one capacitor to ground.
    fn two_grounds() -> Board {
        let nets = ["A_GND", "FIELD_0V", "A_3V3", "RAW", "B_VCC", "SIG", "FILT", "OUT", "LED"];
        let net = |name: &str| nets.iter().position(|n| *n == name);
        let part = |name: &str, x: f64, on: &[&str]| Part {
            name: name.to_string(),
            sheet: "all".to_string(),
            at: [x, 5.0],
            placed: true,
            fixed: false,
            edge: name.starts_with('J'),
            capacitor: name.starts_with('C'),
            area: 10.0,
            pads: on.iter().map(|n| net(n)).collect(),
        };
        Board {
            outline: [0.0, 0.0, 40.0, 10.0],
            nets: nets.iter().map(|n| Net { name: n.to_string(), supply: false, pair: false }).collect(),
            parts: vec![
                part("U1", 2.0, &["A_3V3", "A_GND", "SIG", "FILT"]),
                part("C1", 3.0, &["A_3V3", "A_GND"]),
                part("C2", 4.0, &["A_3V3", "A_GND"]),
                part("C3", 5.0, &["FILT", "A_GND"]),
                part("R1", 6.0, &["FILT", "SIG"]),
                part("U2", 7.0, &["RAW", "A_3V3", "A_GND"]),
                part("C4", 8.0, &["RAW", "A_GND"]),
                part("C5", 9.0, &["RAW", "A_GND"]),
                part("U3", 20.0, &["SIG", "A_3V3", "A_GND", "OUT", "B_VCC", "FIELD_0V"]),
                part("U4", 30.0, &["OUT", "B_VCC", "FIELD_0V"]),
                part("C6", 31.0, &["B_VCC", "FIELD_0V"]),
                part("C7", 32.0, &["B_VCC", "FIELD_0V"]),
                part("R2", 33.0, &["OUT", "LED"]),
                part("J1", 38.0, &["LED", "FIELD_0V"]),
            ],
        }
    }

    #[test]
    fn grounds_are_domains_and_a_supply_is_what_two_capacitors_bypass() {
        let found = floorplan(&two_grounds()).unwrap();
        let power = &found.power;
        type Rails<'a> = Vec<(&'a str, Vec<(&'a str, usize, usize)>)>;
        let rails: Rails = power
            .domains
            .iter()
            .map(|d| {
                (d.ground.as_str(), d.rails.iter().map(|r| (r.net.as_str(), r.parts.len(), r.capacitors)).collect())
            })
            .collect();
        // (the filtered signal has one capacitor to ground: no supply)
        assert_eq!(rails, [("A_GND", vec![("A_3V3", 5, 2), ("RAW", 3, 2)]), ("FIELD_0V", vec![("B_VCC", 4, 2)])]);
        // the resistor with no pad on a ground or a supply goes with the parts it shares nets with
        assert_eq!(power.domains[1].parts, ["C6", "C7", "J1", "R2", "U4"]);
        assert_eq!(power.domains[0].parts, ["C1", "C2", "C3", "C4", "C5", "R1", "U1", "U2", "U3"]);
        assert_eq!(power.between, [("U3".to_string(), vec!["A_GND".to_string(), "FIELD_0V".to_string()])]);
        assert!(power.adrift.is_empty());
        let links: Vec<(&str, &str, &[String])> =
            power.links.iter().map(|(a, b, by)| (a.as_str(), b.as_str(), &by[..])).collect();
        assert_eq!(links, [("A_3V3", "B_VCC", &["U3".to_string()][..]), ("A_3V3", "RAW", &["U2".to_string()][..])]);
    }

    #[test]
    fn each_domain_is_given_ground_by_what_it_needs_wherever_its_parts_stand_today() {
        let found = floorplan(&two_grounds()).unwrap();
        let [a, field] = &found.zones[..] else { panic!("{:?}", found.zones.len()) };
        assert_eq!((a.name.as_str(), field.name.as_str()), ("A_GND", "FIELD_0V"));
        // nine parts to five of the same size: nine fourteenths of the board
        assert_eq!(a.rect, [0.0, 0.0, 25.71, 10.0]);
        assert_eq!(field.rect, [25.71, 0.0, 40.0, 10.0]);
        // (as it happens the parts stand where their zones are given ground)
        assert_eq!((a.at_home, field.at_home, a.placed, field.placed), (1.0, 1.0, 9, 5));
        assert!(a.off.unwrap() < 8.0 && field.off.unwrap() < 2.0, "{:?} {:?}", a.off, field.off);
        // every part is in one zone at each level
        for zone in &found.zones {
            if !zone.inside.is_empty() {
                let mut inner: Vec<&String> = zone.inside.iter().flat_map(|z| &z.parts).collect();
                inner.sort();
                assert_eq!(inner, zone.parts.iter().collect::<Vec<_>>());
            }
        }
        // nothing is fixed: turned round, the board is given the same ground (the connector moves with its zone)
        let mut turned = two_grounds();
        turned.parts.iter_mut().for_each(|part| part.at[0] = 40.0 - part.at[0]);
        let again = floorplan(&turned).unwrap();
        assert_eq!((again.zones[0].rect, again.zones[1].rect), (a.rect, field.rect));
    }

    #[test]
    fn a_board_without_size_or_a_pad_on_a_net_that_is_not_there_is_refused() {
        let mut board = two_grounds();
        board.outline = [0.0, 0.0, 0.0, 10.0];
        assert!(floorplan(&board).is_err());
        let mut board = two_grounds();
        board.parts[0].pads.push(Some(99));
        assert!(floorplan(&board).is_err());
    }
}
