//! What the placing of the parts comes to, as the plan tells it.

use super::beside::Switcher;
use super::facing::Facing;
use super::pads::ByPads;
use super::units::Copies;
use super::{Site, Standing};
use crate::floorplan::constants::{BACK, CHAIN_MOST, LOOP_MOST};
use crate::floorplan::ground::Work;
use crate::floorplan::known::Known;
use crate::floorplan::settle::{round1, round2};
use crate::floorplan::types::{Loop, Place, Repeat};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// A chain that doubles back: its two heads, the parts between them, and how long it is straight and along.
pub(super) type Back = (String, String, Vec<String>, f64, f64);
/// A part by a pad: the part, what it stands by, how far its pad is from that one's, and how many share it.
pub(super) type ByPad = (String, String, f64, usize);
/// Nets by how far apart in the plan their parts are: the kind, how many, and how long today and as placed.
pub(super) type Against = (String, usize, f64, f64);

/// The sheets that repeat, as they came out: a copy is laid out like the others if it was placed whole.
pub(super) fn repeats(
    cx: &Known,
    facing: &Facing,
    groups: &[Copies],
    in_rows: &BTreeMap<usize, usize>,
    centre: &[Option<Pt>],
    quarter: &[u8],
) -> Vec<Repeat> {
    let (parts, block_quarter) = (cx.parts, facing.block_quarter);
    let mut repeats: Vec<Repeat> = groups
        .iter()
        .map(|(names, count, keys, copies)| {
            // The nets of the copies, matched by the pad of the matched part they are on. A net that is
            // one and the same for every copy is shared by them (a bias, a clock): its length is told. The
            // others are each copy's own: how much the longest of a net's copies is longer than the
            // shortest says how alike the copies are joined to what is round them.
            let mut seen: BTreeSet<usize> = BTreeSet::new();
            let (mut shared, mut shared_mm, mut matched, mut spread, mut longest) = (0, 0.0, 0, 0.0, 0.0f64);
            let mut worst: (f64, String) = (0.0, String::new());
            for (j, &p) in copies[0].iter().enumerate() {
                for (i, net) in parts[p].pads.iter().enumerate() {
                    let Some(net) = net.filter(|&net| cx.is_signal(net) && seen.insert(net)) else { continue };
                    let theirs: Vec<Option<usize>> = copies
                        .iter()
                        .map(|own| {
                            own.get(j)
                                .and_then(|&q| parts[q].pads.get(i).copied().flatten())
                                .filter(|&of| cx.is_signal(of))
                        })
                        .collect();
                    if theirs.iter().any(|of| of.is_none()) {
                        continue;
                    }
                    let theirs: Vec<usize> = theirs.into_iter().flatten().collect();
                    if theirs.iter().all(|&of| of == net) {
                        shared += 1;
                        shared_mm += facing.span_of(net, centre, quarter);
                        continue;
                    }
                    let spans: Vec<f64> = theirs.iter().map(|&of| facing.span_of(of, centre, quarter)).collect();
                    let (least, most) = (
                        spans.iter().copied().fold(f64::INFINITY, f64::min),
                        spans.iter().copied().fold(0.0, f64::max),
                    );
                    matched += 1;
                    spread += most - least;
                    longest = longest.max(most);
                    if most - least > worst.0 {
                        worst = (most - least, cx.net_name(net));
                    }
                }
            }
            Repeat {
                sheets: names.clone(),
                parts: *count,
                alike: keys.iter().filter(|key| block_quarter.contains_key(key)).count(),
                together: keys.first().and_then(|key| in_rows.get(key)).copied().unwrap_or(0),
                like: names[0].clone(),
                matched,
                spread_mm: if matched > 0 { round1(spread / matched as f64) } else { 0.0 },
                worst_mm: round1(worst.0),
                worst: worst.1,
                longest_mm: round1(longest),
                shared,
                shared_mm: round1(shared_mm),
            }
        })
        .collect();
    repeats.sort_by(|a, b| (b.sheets.len() * b.parts).cmp(&(a.sheets.len() * a.parts)).then(a.sheets.cmp(&b.sheets)));
    repeats
}

/// What disturbs and stands near what is disturbed, part by part: by how far their bodies are apart, as the
/// boards stack (on one board: another board is another matter).
pub(super) fn close(site: &Site, centre: &[Option<Pt>], quarter: &[u8]) -> Vec<(String, String, f64)> {
    let (cx, to, domain_of_part) = (site.cx, &site.on.to, &site.domain_of_part);
    let (parts, aggressor, victim) = (cx.parts, &cx.aggressor, &cx.victim);
    let stands_at = |p: usize| -> Option<[f64; 4]> {
        let at = centre[p].filter(|_| !cx.stays[p])?;
        let [w, h] = cx.body(p);
        let (w, h) = if quarter[p] % 2 == 1 { (h, w) } else { (w, h) };
        Some([at[0] - w / 2.0, at[1] - h / 2.0, at[0] + w / 2.0, at[1] + h / 2.0])
    };
    let mut close: Vec<(String, String, f64)> = Vec::new();
    for a in (0..parts.len()).filter(|&a| aggressor[a]) {
        let Some(loud) = stands_at(a) else { continue };
        for v in (0..parts.len()).filter(|&v| victim[v] && to[domain_of_part[v]] == to[domain_of_part[a]]) {
            let Some(hushed) = stands_at(v) else { continue };
            let apart = [0, 1].map(|k| (loud[k].max(hushed[k]) - loud[k + 2].min(hushed[k + 2])).max(0.0));
            let far = dist([0.0, 0.0], apart);
            if far < cx.board.tune.quiet_mm {
                close.push((cx.name(a), cx.name(v), round1(far)));
            }
        }
    }
    close.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    close
}

/// The sensitive nets: how many, how long in all, and the longest.
pub(super) fn sensitive(
    cx: &Known,
    facing: &Facing,
    centre: &[Option<Pt>],
    quarter: &[u8],
) -> (usize, f64, f64, String) {
    let all_nets = cx.all_nets;
    let mut sensitive: (usize, f64, f64, String) = (0, 0.0, 0.0, String::new());
    for net in (0..all_nets.len()).filter(|&net| all_nets[net].sensitive && cx.is_signal(net)) {
        let far = facing.span_of(net, centre, quarter);
        sensitive.0 += 1;
        sensitive.1 += far;
        if far > sensitive.2 {
            (sensitive.2, sensitive.3) = (far, cx.net_name(net));
        }
    }
    (sensitive.0, sensitive.1.round(), round1(sensitive.2), sensitive.3)
}

/// The loops, as they came out: the box of the pads on the node and on the supplies of the loop.
pub(super) fn loops(
    cx: &Known,
    facing: &Facing,
    switchers: &[Switcher],
    centre: &[Option<Pt>],
    quarter: &[u8],
) -> Vec<Loop> {
    let (parts, found) = (cx.parts, &cx.found);
    let mut loops: Vec<Loop> = switchers
        .iter()
        .map(|(chip, node, by_it, at_pads)| {
            let mut spots: Vec<Pt> = Vec::new();
            let mut of_loop: BTreeSet<usize> = [*node].into_iter().collect();
            // (of the capacitors on one supply, the one nearest the chip closes the loop: the others are behind it)
            let here = facing.in_stack(*chip, centre);
            let mut nearest: BTreeMap<usize, (f64, usize)> = BTreeMap::new();
            for &m in by_it.iter().filter(|&&m| parts[m].capacitor) {
                let far = match (here, facing.in_stack(m, centre)) {
                    (Some(a), Some(b)) => dist(a, b),
                    _ => f64::INFINITY,
                };
                for &rail in parts[m].pads.iter().flatten().filter(|net| !found.grounds.contains(net)) {
                    if nearest.get(&rail).map_or(true, |best| far < best.0) {
                        nearest.insert(rail, (far, m));
                    }
                }
            }
            let closing: BTreeSet<usize> = nearest.values().map(|best| best.1).collect();
            let mut bodies = cx.body(*chip)[0] * cx.body(*chip)[1];
            for &m in by_it.iter().filter(|&&m| !parts[m].capacitor || closing.contains(&m)) {
                bodies += cx.body(m)[0] * cx.body(m)[1];
                of_loop.extend(parts[m].pads.iter().flatten().filter(|net| !found.grounds.contains(net)));
                spots.extend((0..parts[m].pads.len()).filter_map(|i| facing.pad_at(m, i, quarter[m], centre)));
            }
            for (i, net) in parts[*chip].pads.iter().enumerate() {
                if net.is_some_and(|net| of_loop.contains(&net)) {
                    spots.extend(facing.pad_at(*chip, i, quarter[*chip], centre));
                }
            }
            let low = |k: usize| spots.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| spots.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            let span = if spots.is_empty() { [0.0, 0.0] } else { [high(0) - low(0), high(1) - low(1)] };
            Loop {
                chip: cx.name(*chip),
                node: cx.net_name(*node),
                parts: cx.names(by_it),
                beside: cx.names(at_pads),
                span: span.map(|v| (v * 10.0).round() / 10.0),
                mm2: (span[0] * span[1]).round(),
                bodies: bodies.round(),
                tight: span[0] * span[1] <= LOOP_MOST * bodies,
            }
        })
        .collect();
    loops.sort_by(|a, b| b.mm2.total_cmp(&a.mm2).then(a.chip.cmp(&b.chip)));
    loops
}

/// The chains, and those that double back.
///
/// The chains: from the pad of a head on a net where it is the only head, through parts of two pads in
/// series (each on to a net with no head and one such part more), to the next head. One doubles back if
/// the way from pad to pad through its parts is more than BACK times as long as straight, and more than that
/// by what its parts may stand off the pads they are by: twice `by_pad` for each, less BESIDE_PIN for each
/// joint between two of them.
pub(super) fn chains(cx: &Known, facing: &Facing, centre: &[Option<Pt>], quarter: &[u8]) -> (usize, Vec<Back>) {
    let (parts, all_nets, reach) = (cx.parts, cx.all_nets, &cx.reach);
    let series_on = |net: usize, from: usize| -> Vec<(usize, usize)> {
        reach[net]
            .iter()
            .copied()
            .filter(|&m| m != from && parts[m].pads.len() == 2 && !cx.is_head(m))
            .filter_map(|m| {
                let other = parts[m].pads.iter().flatten().copied().find(|&other| other != net)?;
                cx.is_signal(other).then_some((m, other))
            })
            .collect()
    };
    let pad_on = |q: usize, net: usize| -> Option<Pt> {
        let i = parts[q].pads.iter().position(|&of| of == Some(net))?;
        facing.pad_at(q, i, quarter[q], centre)
    };
    let mut chains = 0;
    let mut back: Vec<(String, String, Vec<String>, f64, f64)> = Vec::new();
    for start in (0..all_nets.len()).filter(|&net| cx.is_signal(net)) {
        let heads: Vec<usize> = reach[start].iter().copied().filter(|&q| cx.is_head(q)).collect();
        let [from] = heads[..] else { continue };
        for (first, mut net) in series_on(start, from) {
            let (mut through, mut last) = (vec![first], first);
            let mut joints: Vec<usize> = vec![start, net]; // the nets along the chain, from its start
            let mut to: Option<(usize, usize)> = None;
            for _ in 0..CHAIN_MOST {
                let heads: Vec<usize> = reach[net].iter().copied().filter(|&q| cx.is_head(q)).collect();
                match (&heads[..], &series_on(net, last)[..]) {
                    ([head], _) => {
                        to = Some((*head, net));
                        break;
                    }
                    ([], [(next, onward)]) => {
                        through.push(*next);
                        (last, net) = (*next, *onward);
                        joints.push(net);
                    }
                    _ => break,
                }
            }
            // (each chain once: from its end of the lesser net)
            let Some((to, end)) = to.filter(|&(_, end)| start < end) else { continue };
            let (Some(a), Some(b)) = (pad_on(from, start), pad_on(to, end)) else { continue };
            chains += 1;
            // (the track there is to lay: from pad to pad, hop by hop; what is inside a part is no track.
            // Each hop may be as long as a part stands from the pad it is by)
            let mut along = 0.0;
            let mut here = a;
            for (k, &m) in through.iter().enumerate() {
                let (Some(near), Some(far)) = (pad_on(m, joints[k]), pad_on(m, joints[k + 1])) else { continue };
                along += dist(here, near);
                here = far;
            }
            along += dist(here, b);
            let direct = dist(a, b);
            // (each part may stand as far from the pad on either side of it as a part of its size stands by a pad)
            let room: f64 = through.iter().map(|&m| 2.0 * cx.by_pad(m)).sum::<f64>()
                - (through.len() - 1) as f64 * cx.board.tune.beside_pin_mm;
            if along > BACK * direct + room {
                let round = |v: f64| (v * 10.0).round() / 10.0;
                back.push((
                    cx.name(from),
                    cx.name(to),
                    through.iter().map(|&m| cx.name(m)).collect(),
                    round(direct),
                    round(along),
                ));
            }
        }
    }
    back.sort_by(|a, b| (b.4 - b.3).total_cmp(&(a.4 - a.3)).then(a.0.cmp(&b.0)).then(a.2.cmp(&b.2)));
    (chains, back)
}

/// The parts that stand by a pad, as they came out: how many capacitors of supplies there are and how many
/// of them are by their pad; the same for what stands for one pad; and each with how far it is from its pad.
pub(super) fn by_pads_told(
    cx: &Known,
    facing: &Facing,
    by: &ByPads,
    served_parts: &BTreeSet<usize>,
    centre: &[Option<Pt>],
    quarter: &[u8],
) -> ([usize; 2], [usize; 2], Vec<ByPad>) {
    let (pins, at_pad) = (&by.pins, &by.at_pad);
    let beside = pins
        .iter()
        .filter(|(c, _)| cx.of_supplies(c))
        .filter(|(&c, &(mine, chip, pin))| {
            match (facing.pad_at(c, mine, quarter[c], centre), facing.pad_at(chip, pin, quarter[chip], centre)) {
                (Some(here), Some(there)) => dist(here, there) <= cx.by_pad(c),
                _ => false,
            }
        })
        .count();
    // (what stands for one pad, and how much of it is by that pad)
    let served_by = pins
        .iter()
        .filter(|(c, _)| served_parts.contains(c))
        .filter(|(&c, &(mine, head, pin))| {
            match (facing.pad_at(c, mine, quarter[c], centre), facing.pad_at(head, pin, quarter[head], centre)) {
                (Some(here), Some(there)) => dist(here, there) <= cx.by_pad(c),
                _ => false,
            }
        })
        .count();
    let served = [pins.keys().filter(|c| served_parts.contains(c)).count(), served_by];
    let bypass = [pins.keys().filter(|c| cx.of_supplies(c)).count(), beside];
    let mut capacitors: Vec<(String, String, f64, usize)> = pins
        .iter()
        .filter_map(|(&c, &(mine, chip, pin))| {
            let (here, there) =
                (facing.pad_at(c, mine, quarter[c], centre)?, facing.pad_at(chip, pin, quarter[chip], centre)?);
            let shared = at_pad.get(&(chip, pin)).copied().unwrap_or(1);
            Some((cx.name(c), cx.name(chip), (dist(here, there) * 10.0).round() / 10.0, shared))
        })
        .collect();
    capacitors.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    (bypass, served, capacitors)
}

/// The place of every part, as it is told; and where each is as the boards stack.
pub(super) fn places(
    site: &Site,
    grounds: &BTreeMap<usize, [f64; 4]>,
    gap_rects: &BTreeMap<usize, [f64; 4]>,
    standing: &Standing,
) -> (Vec<Place>, Vec<Option<Pt>>) {
    let (cx, parts, fine_leaves) = (site.cx, site.cx.parts, &site.leaves);
    let (on_gap, shift_of) = (&site.on_gap, &site.shift_of);
    let Standing { stood, centre, quarter } = standing;
    let mut places: Vec<Place> = Vec::new();
    let mut spot: Vec<Option<Pt>> = vec![None; parts.len()];
    for (_, _, own) in fine_leaves {
        for &p in own {
            let Some(at) = centre[p] else { continue };
            let settled_here = stood.get(&p).is_some_and(|at| at.is_some());
            let (rect, fits) = match (on_gap.contains_key(&p), settled_here) {
                (true, _) => (gap_rects.get(&p).copied(), true),
                (false, true) => (Some(cx.taken(p, (at, quarter[p] % 2 == 1))), true),
                (false, false) => (grounds.get(&p).copied(), false),
            };
            spot[p] = Some([at[0] + shift_of[p][0], at[1] + shift_of[p][1]]);
            places.push(Place {
                part: cx.name(p),
                at: at.map(round2),
                rect: rect.map(|r| r.map(round2)),
                turn: 90 * u16::from(quarter[p] % 4),
                back: parts[p].back,
                pieces: if parts[p].pieces.is_empty() || rect.is_none() {
                    Vec::new()
                } else {
                    cx.stands_on(p, (at, quarter[p] % 2 == 1)).into_iter().map(|r| r.map(round2)).collect()
                },
                fits,
                off: parts[p].placed.then(|| (dist(parts[p].at, at) * 10.0).round() / 10.0),
            });
        }
    }
    // (what is held where it stands: there, as it lies)
    for p in (0..parts.len()).filter(|&p| cx.held(p)) {
        spot[p] = Some([parts[p].at[0] + shift_of[p][0], parts[p].at[1] + shift_of[p][1]]);
        places.push(Place {
            part: cx.name(p),
            at: parts[p].at.map(round2),
            rect: Some(cx.body_at(p, parts[p].at, 0).map(round2)),
            turn: 0,
            back: parts[p].back,
            pieces: if parts[p].pieces.is_empty() {
                Vec::new()
            } else {
                cx.stands_on(p, (parts[p].at, false)).into_iter().map(|r| r.map(round2)).collect()
            },
            fits: true,
            off: Some(0.0),
        });
    }
    // (what is the board's own stays where it stands)
    for part in parts.iter().zip(&cx.stays).filter(|(_, &stays)| stays).map(|(part, _)| part) {
        places.push(Place {
            part: part.name.clone(),
            at: part.at.map(round2),
            rect: None,
            turn: 0,
            back: part.back,
            pieces: Vec::new(),
            fits: true,
            off: Some(0.0),
        });
    }
    places.sort_by(|a, b| a.part.cmp(&b.part));
    (places, spot)
}

/// How long the signal nets are with the parts as they stand, and with each at its place (nets all of whose
/// parts stand on the board today; as the boards stack); and the same by how far apart in the plan the
/// parts of a net are.
pub(super) fn against(site: &Site, spot: &[Option<Pt>]) -> (f64, f64, Vec<Against>) {
    let (cx, top) = (site.cx, site.top);
    let (parts, nets) = (cx.parts, &cx.nets);
    let (boards, shifts) = (&site.on.boards, &site.on.shifts);
    // (how long the signal nets are with the parts as they stand, and with each at its place: nets all of
    // whose parts stand on the board today; as the boards stack)
    let (mut parts_now_mm, mut parts_then_mm) = (0.0, 0.0);
    // (and the same by how far apart in the plan the parts of a net are: within one of the smallest zones;
    // between such zones of one zone under a domain; between those; between domains)
    let mut where_of: BTreeMap<usize, [usize; 3]> = BTreeMap::new(); // part -> (domain, the zone under it, the smallest zone)
    fn told_where(work: &Work, at: [usize; 2], count: &mut usize, out: &mut BTreeMap<usize, [usize; 3]>) {
        if work.inside.is_empty() {
            out.extend(work.parts.iter().map(|&p| (p, [at[0], at[1], *count])));
            *count += 1;
        }
        work.inside.iter().for_each(|inner| told_where(inner, at, count, out));
    }
    let mut smallest = 0;
    for (domain, work) in top.iter().enumerate() {
        if work.inside.is_empty() {
            told_where(work, [domain, 0], &mut smallest, &mut where_of);
        }
        for (k, inner) in work.inside.iter().enumerate() {
            told_where(inner, [domain, k], &mut smallest, &mut where_of);
        }
    }
    let kinds = [
        "within one of the smallest zones",
        "between zones of one larger zone",
        "between larger zones of a domain",
        "between domains",
    ];
    let mut against: Vec<(String, usize, f64, f64)> =
        kinds.iter().map(|kind| (kind.to_string(), 0, 0.0, 0.0)).collect();
    for net in nets {
        if !net.iter().all(|&p| parts[p].placed && spot[p].is_some()) {
            continue;
        }
        let span = |points: Vec<Pt>| {
            let low = |k: usize| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| points.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            high(0) - low(0) + high(1) - low(1)
        };
        // (today the parts stand as the file draws them: on the board they are drawn on)
        let today = |p: usize| {
            // (the board it stands within; one that stands off every board, the board it stands above or below)
            let at = parts[p].at;
            let across = |b: &[f64; 4]| at[0] >= b[0] && at[0] <= b[2];
            let home = boards
                .iter()
                .position(|b| across(b) && at[1] >= b[1] && at[1] <= b[3])
                .or_else(|| boards.iter().position(across));
            let shift = home.map_or(site.shift_of[p], |b| shifts[b]);
            [parts[p].at[0] + shift[0], parts[p].at[1] + shift[1]]
        };
        let (now, then) =
            (span(net.iter().map(|&p| today(p)).collect()), span(net.iter().filter_map(|&p| spot[p]).collect()));
        parts_now_mm += now;
        parts_then_mm += then;
        let at: Vec<[usize; 3]> = net.iter().filter_map(|p| where_of.get(p).copied()).collect();
        let same = |k: usize| at.windows(2).all(|pair| pair[0][..=k] == pair[1][..=k]);
        let kind = if same(2) {
            0
        } else if same(1) {
            1
        } else if same(0) {
            2
        } else {
            3
        };
        (against[kind].1, against[kind].2, against[kind].3) =
            (against[kind].1 + 1, against[kind].2 + now, against[kind].3 + then);
    }
    (parts_now_mm, parts_then_mm, against)
}
