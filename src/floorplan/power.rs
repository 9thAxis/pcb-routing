//! The grounds and the supplies of a board, and the ground each part is on.

use super::constants::BYPASSED;
use super::types::Board;
use std::collections::{BTreeMap, BTreeSet};

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

/// What the power analysis finds, by index: the grounds (net indices), the rails with the ground of each, the
/// ground a part is counted with, and the grounds each part stands on.
pub(super) struct Powered {
    pub(super) home: Vec<Option<usize>>,
    pub(super) stands: Vec<BTreeSet<usize>>,
    pub(super) rails: BTreeMap<usize, (Option<usize>, usize)>,
    pub(super) grounds: Vec<usize>,
}

pub(super) fn powered(board: &Board) -> Powered {
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
