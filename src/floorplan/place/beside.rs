//! The parts that stand by a pad: the capacitors of a chip, the loop of a switching regulator, and what is
//! there for one pad of a chip or a connector.

use super::Site;
use crate::floorplan::constants::{BYPASS, SERVED, SWITCHED};
use crate::floorplan::known::Known;
use crate::floorplan::settle::farads;
use crate::floorplan::types::Pin;
use std::collections::{BTreeMap, BTreeSet};

/// A chip that switches an inductor: the chip, its node, its loop's parts, and those at its other pads.
pub(super) type Switcher = (usize, usize, Vec<usize>, Vec<usize>);
/// A part that stands by a pad: the part, what it stands by, and the net of that one's pad.
pub(super) type By = (usize, usize, usize);

/// The parts that stand by a pad, and the nets the parts are given ground by.
pub(super) struct Beside {
    /// The nets of the parts: the signals, and a tie from each part that stands by a pad to what it stands by.
    pub(super) fine_nets: Vec<Vec<usize>>,
    /// The parts that stand by a pad: those of the loops first, then what is across two pads of a chip, the
    /// capacitors of the chips, and what else is there for one pad.
    pub(super) bypasses: Vec<By>,
    /// The same in the order they are given their places: the capacitors and the rest the smallest first.
    pub(super) order: Vec<By>,
    pub(super) switchers: Vec<Switcher>,
    /// What is there for one pad of a chip or a connector.
    pub(super) served_parts: BTreeSet<usize>,
}

/// The nets of the parts (the signals, and for each capacitor from a supply to a ground a tie to a chip of
/// its zone on that supply, the chips taking them in turn), and those capacitors, each with its chip and
/// the supply.
fn capacitors_of_chips(site: &Site) -> (Vec<Vec<usize>>, Vec<By>) {
    let (cx, top, fine_leaves) = (site.cx, site.top, &site.leaves);
    let (parts, found, nets) = (cx.parts, &cx.found, &cx.nets);
    // (the nets of the parts: the signals, and for each capacitor from a supply to a ground a tie to a chip of
    // its zone on that supply, the chips taking them in turn)
    let mut fine_nets: Vec<Vec<usize>> = nets.clone();
    let mut bypasses: Vec<(usize, usize, usize)> = Vec::new(); // (the capacitor, its chip, the supply)
    let mut taken: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (domain, _, own) in fine_leaves {
        for &c in own.iter().filter(|&&c| parts[c].capacitor && parts[c].pads.len() == 2) {
            for &rail in parts[c].pads.iter().flatten().filter(|net| found.rails.contains_key(net)) {
                // (a chip of its zone; failing that, one of its domain)
                let of = |among: &Vec<usize>| {
                    among
                        .iter()
                        .copied()
                        .filter(|&q| cx.is_chip(q) && parts[q].pads.contains(&Some(rail)))
                        .min_by_key(|&q| (taken.get(&(rail, q)).copied().unwrap_or(0), q))
                };
                let chip = of(own).or_else(|| of(&top[*domain].parts));
                if let Some(chip) = chip {
                    *taken.entry((rail, chip)).or_insert(0) += 1;
                    fine_nets.extend(vec![vec![c, chip]; BYPASS]);
                    if bypasses.last().map_or(true, |last| last.0 != c) {
                        bypasses.push((c, chip, rail));
                    }
                }
            }
        }
    }
    (fine_nets, bypasses)
}

/// The switching regulators and the parts of their loops, each with what it stands by.
///
/// A chip with a pad on a net that goes to an inductor and to little else switches it: that net is its
/// switch node, and what carries the switched current stands by the chip, the loop as small as may be:
/// the inductor and what else of two pads is on the node (a diode) by the chip's pad on the node; and the
/// capacitors of the chip's sheet from the far side of each of those to a ground (two at the most, the
/// smallest first) by that part's pad there. These go before the other capacitors.
fn loops_of(cx: &Known, bypasses: &[By]) -> (Vec<Switcher>, Vec<By>) {
    let (parts, all_nets, found, reach) = (cx.parts, cx.all_nets, &cx.found, &cx.reach);
    let mut switchers: Vec<Switcher> = Vec::new();
    let mut of_loops: Vec<(usize, usize, usize)> = Vec::new();
    // (of two nets from one chip to one inductor, the one with the fewer parts is the node: the other is
    // where the chip senses what the inductor gives)
    let mut nodes: Vec<usize> =
        (0..all_nets.len()).filter(|&net| cx.is_signal(net) && reach[net].len() <= SWITCHED).collect();
    nodes.sort_by_key(|&net| (reach[net].len(), net));
    let mut switched: BTreeSet<(usize, usize)> = BTreeSet::new();
    for node in nodes {
        let on: Vec<usize> = reach[node].iter().copied().collect();
        let (chips, coils): (Vec<usize>, Vec<usize>) = (
            on.iter().copied().filter(|&q| cx.is_chip(q)).collect(),
            on.iter().copied().filter(|&q| parts[q].inductor).collect(),
        );
        let ([chip], [coil]) = (&chips[..], &coils[..]) else { continue };
        if !switched.insert((*chip, *coil)) {
            continue;
        }
        let mut by_it: Vec<usize> = Vec::new();
        for &m in on.iter().filter(|&&m| parts[m].pads.len() == 2 && !parts[m].capacitor) {
            of_loops.push((m, *chip, node));
            by_it.push(m);
            let far =
                parts[m].pads.iter().flatten().copied().find(|&net| net != node && found.rails.contains_key(&net));
            let Some(far) = far else { continue };
            let mut closing: Vec<usize> = reach[far]
                .iter()
                .copied()
                .filter(|&c| parts[c].capacitor && parts[c].pads.len() == 2 && cx.sheet(c) == cx.sheet(*chip))
                .filter(|&c| parts[c].pads.iter().flatten().any(|net| found.grounds.contains(net)))
                .collect();
            closing.sort_by(|&a, &b| {
                let held = |c: usize| farads(&parts[c].value).unwrap_or(f64::INFINITY);
                held(a).total_cmp(&held(b)).then(parts[a].name.cmp(&parts[b].name))
            });
            for &c in closing.iter().take(2) {
                of_loops.push((c, m, far));
                by_it.push(c);
            }
        }
        // (and the capacitors that already stand for the chip are of its loop)
        by_it.extend(bypasses.iter().filter(|bypass| bypass.1 == *chip).map(|bypass| bypass.0));
        by_it.sort_unstable();
        by_it.dedup();
        switchers.push((*chip, node, by_it, Vec::new()));
    }
    // And what of two pads is on another small net of such a chip stands by the chip's pad on that net: the
    // divider it senses its output by, what sets its frequency or its limit, the capacitor it drives its
    // switch from. A long track there picks up what the switch node sends.
    for switcher in switchers.iter_mut() {
        let (chip, node) = (switcher.0, switcher.1);
        let own: BTreeSet<usize> = parts[chip].pads.iter().flatten().copied().collect();
        for net in own.into_iter().filter(|&net| net != node && cx.is_signal(net) && reach[net].len() <= SWITCHED) {
            let small = |m: &usize| parts[*m].pads.len() == 2 && !parts[*m].inductor;
            for m in reach[net].iter().copied().filter(small).collect::<Vec<usize>>() {
                if of_loops.iter().all(|of| of.0 != m) {
                    of_loops.push((m, chip, net));
                    switcher.3.push(m);
                }
            }
        }
    }
    (switchers, of_loops)
}

/// What is there for one pad, and what is across two pads of one chip, each with its head and the net.
///
/// A signal is laid out in the order it flows, and what is there for one pad stands by it. On a small net
/// with one chip or connector on it (its head): a capacitor from the net to a ground or a supply, or
/// across to another net of that chip, filters what the chip takes in there and stands by its pad; a
/// diode from it to a ground or a supply clamps it and stands there too; and at a connector what is in
/// series with its pin comes first (a fuse, a resistor, a bead).
fn served(cx: &Known, bypasses: &[By]) -> (Vec<By>, Vec<By>) {
    let (parts, all_nets, found, reach) = (cx.parts, cx.all_nets, &cx.found, &cx.reach);
    let mut serving: Vec<(usize, usize, usize)> = Vec::new();
    let mut across_chip: Vec<(usize, usize, usize)> = Vec::new();
    // (the nets a chip takes a signal in on come first, those it drives last: a part that could stand for
    // either of two chips, the resistor between a driver and a filtered input, is the receiver's, with its
    // capacitor. An analog input is often drawn as a passive pin: so it is the drivers that are told apart)
    let rank = |pin: Pin| match pin {
        Pin::Input => 0u8,
        Pin::Output | Pin::PowerOut => 2,
        _ => 1,
    };
    let mut headed: Vec<(u8, usize, usize)> = (0..all_nets.len())
        .filter(|&net| cx.is_signal(net))
        .filter_map(|net| cx.only_head(net).map(|head| (rank(cx.pin_on(head, net)), net, head)))
        .collect();
    headed.sort_unstable();
    for (_, net, head) in headed {
        let small = reach[net].len() <= SERVED;
        let mut filtered = false;
        let mut in_series: Vec<usize> = Vec::new();
        for &m in reach[net].iter().filter(|&&m| m != head && parts[m].pads.len() == 2) {
            if parts[m].inductor || parts[m].edge || bypasses.iter().any(|bypass| bypass.0 == m) {
                continue;
            }
            let Some(other) = parts[m].pads.iter().flatten().copied().find(|&other| other != net) else { continue };
            let to_power = found.grounds.contains(&other) || found.rails.contains_key(&other);
            let across = cx.is_signal(other) && reach[other].contains(&head);
            // (what is across two pads of one chip and of nothing else is its own: its feedback, its
            // compensation, whatever the size of the nets)
            let its_own = cx.is_signal(other) && cx.only_head(other) == Some(head);
            let shunt = small && ((parts[m].capacitor && (to_power || across)) || (parts[m].diode && to_power));
            let first = small && parts[head].edge && !to_power && !parts[m].capacitor;
            filtered |= shunt && parts[m].capacitor;
            let new = serving.iter().chain(&across_chip).all(|of| of.0 != m);
            if its_own && new {
                across_chip.push((m, head, net));
            } else if (shunt || first) && new {
                serving.push((m, head, net));
            } else if small && cx.is_signal(other) && !parts[m].capacitor && !parts[m].diode {
                in_series.push(m);
            }
        }
        // (and the resistor in series with a filter's capacitor is of the filter: it stands there with it)
        for m in in_series.into_iter().filter(|_| filtered) {
            if serving.iter().chain(&across_chip).all(|of| of.0 != m) {
                serving.push((m, head, net));
            }
        }
    }
    (serving, across_chip)
}

/// The parts that stand by a pad, in the order they are given their places.
pub(super) fn beside(site: &Site) -> Beside {
    let (cx, parts) = (site.cx, site.cx.parts);
    let (mut fine_nets, mut bypasses) = capacitors_of_chips(site);
    let (switchers, of_loops) = loops_of(cx, &bypasses);
    let in_loops: BTreeSet<usize> = of_loops.iter().map(|of| of.0).collect();
    bypasses.retain(|bypass| !in_loops.contains(&bypass.0));
    for &(c, by, _) in &of_loops {
        fine_nets.extend(vec![vec![c, by]; BYPASS]);
    }
    let loop_parts = of_loops.len();
    bypasses.splice(0..0, of_loops);
    let (serving, across_chip) = served(cx, &bypasses);
    for &(m, head, _) in serving.iter().chain(&across_chip) {
        fine_nets.extend(vec![vec![m, head]; BYPASS]);
    }
    let served_parts: BTreeSet<usize> = serving.iter().chain(&across_chip).map(|of| of.0).collect();
    // (what is across two pads of a chip has one place it can stand, between them: it has it before the
    // capacitors, which may stand anywhere along a side)
    bypasses.retain(|bypass| !served_parts.contains(&bypass.0));
    let placed_first = loop_parts + across_chip.len();
    bypasses.splice(loop_parts..loop_parts, across_chip);
    bypasses.extend(serving);
    let mut order = bypasses.clone();
    // (the parts of the loops first, as they were found: each inductor before the capacitors by it)
    order[placed_first..].sort_by(|a, b| {
        let held = |c: usize| farads(&parts[c].value).unwrap_or(f64::INFINITY);
        held(a.0).total_cmp(&held(b.0)).then(parts[a.0].name.cmp(&parts[b.0].name))
    });
    Beside { fine_nets, bypasses, order, switchers, served_parts }
}
