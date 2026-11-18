//! Sheets that repeat, laid out alike.

use super::blocks::{in_blocks, Block};
use super::Site;
use crate::floorplan::constants::{AS_NEAR, COLOURINGS, REPEAT_LEAST, UNIT_REACH};
use crate::floorplan::known::Known;
use crate::floorplan::settle::{median, quartered, settled_shape};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// Sheets that repeat: the sheets, the parts of each that are placed whole, a part of each, and the parts of
/// each in one order.
pub(super) type Copies = (Vec<String>, usize, Vec<usize>, Vec<Vec<usize>>);

/// The sheets that repeat, and what is tried for those laid out alike.
pub(super) struct Units {
    pub(super) groups: Vec<Copies>,
    /// For a body of several copies, by its first part: what is tried if it finds no room, in turn; each a
    /// set of bodies, by a part of each.
    pub(super) alternatives: BTreeMap<usize, Vec<Vec<(usize, Block)>>>,
}

/// Where each part of a sheet lies, and how many quarters it is turned from its drawing.
type Lies = BTreeMap<usize, (Pt, u8)>;

/// The colours of the parts of some sheets, the same numbers for all these sheets: each part by its kind,
/// then again and again by what its pads are joined to, until two parts have one colour only if they sit
/// alike in their sheets.
fn coloured(cx: &Known, sheets: &[&String], of_sheet: &BTreeMap<String, Vec<usize>>) -> BTreeMap<usize, usize> {
    let (parts, found, reach) = (cx.parts, &cx.found, &cx.reach);
    // (the colours, the same numbers for all these sheets)
    let mut colour: BTreeMap<usize, usize> = BTreeMap::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for &name in sheets {
        for &p in &of_sheet[name] {
            let next = seen.len();
            colour.insert(p, *seen.entry(format!("{} {}", cx.kind(p), parts[p].pads.len())).or_insert(next));
        }
    }
    for _ in 0..COLOURINGS {
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut then: BTreeMap<usize, usize> = BTreeMap::new();
        for &name in sheets {
            let own: BTreeSet<usize> = of_sheet[name].iter().copied().collect();
            for &p in &of_sheet[name] {
                let mut told = format!("{}", colour[&p]);
                for net in &parts[p].pads {
                    let Some(net) = *net else {
                        told.push_str(" -");
                        continue;
                    };
                    let class = if found.grounds.contains(&net) {
                        "G".to_string()
                    } else if found.rails.contains_key(&net) {
                        format!("R{net}")
                    } else if reach[net].iter().any(|q| !own.contains(q)) {
                        "Sx".to_string()
                    } else {
                        "S".to_string()
                    };
                    let mut on: Vec<(usize, usize)> = Vec::new();
                    if cx.is_signal(net) {
                        for &q in reach[net].iter().filter(|q| own.contains(q) && **q != p) {
                            let pads = (0..parts[q].pads.len()).filter(|&i| parts[q].pads[i] == Some(net));
                            on.extend(pads.map(|i| (colour[&q], i)));
                        }
                        on.sort_unstable();
                    }
                    told.push_str(&format!(" {class}{on:?}"));
                }
                let next = seen.len();
                then.insert(p, *seen.entry(told).or_insert(next));
            }
        }
        colour = then;
    }
    colour
}

/// A sheet laid out by itself on empty ground: where each of its parts lies and how it is turned from its
/// drawing, and the ground they take; None if there is no room for one of them.
///
/// Its blocks are laid as they are, its other parts each by itself; the largest first at the middle, then
/// each time what is tied the most to what lies already, set down as near the parts it is tied to as there
/// is room, turned the way that leaves its pads nearest theirs. `model`: the parts of the sheet, of which
/// those at `whole` are laid.
fn laid_alone(
    cx: &Known,
    model: &[usize],
    whole: &[usize],
    laid_blocks: &BTreeMap<usize, Block>,
) -> Option<(Lies, Vec<[f64; 4]>)> {
    let (parts, reach, nets) = (cx.parts, &cx.reach, &cx.nets);
    let chosen: BTreeSet<usize> = whole.iter().map(|&j| model[j]).collect();
    let mut items: Vec<Vec<(usize, Pt, u8)>> = Vec::new(); // each: its parts, where from its middle, quarters from their drawing
    let mut taken_up: BTreeSet<usize> = BTreeSet::new();
    for &p in &chosen {
        if taken_up.contains(&p) {
            continue;
        }
        match in_blocks(p, laid_blocks) {
            Some(head) => {
                let block = &laid_blocks[&head];
                taken_up.extend(block.drawn.iter().map(|placed| placed.0));
                items.push(block.drawn.clone());
            }
            None => {
                taken_up.insert(p);
                items.push(vec![(p, [0.0, 0.0], 0)]);
            }
        }
    }
    let takes =
        |item: &Vec<(usize, Pt, u8)>| -> f64 { item.iter().map(|&(p, _, _)| cx.body(p)[0] * cx.body(p)[1]).sum() };
    let tied = |item: &Vec<(usize, Pt, u8)>, lies: &BTreeMap<usize, (Pt, u8)>| -> usize {
        let mine: BTreeSet<usize> = item.iter().map(|placed| placed.0).collect();
        nets.iter()
            .filter(|net| net.iter().any(|p| mine.contains(p)) && net.iter().any(|p| lies.contains_key(p)))
            .count()
    };
    let mut lies: BTreeMap<usize, (Pt, u8)> = BTreeMap::new();
    let mut shut: Vec<[f64; 4]> = Vec::new();
    let ground = [-UNIT_REACH, -UNIT_REACH, UNIT_REACH, UNIT_REACH];
    let mut left: Vec<Vec<(usize, Pt, u8)>> = items;
    while !left.is_empty() {
        let next = (0..left.len())
            .max_by(|&a, &b| {
                tied(&left[a], &lies)
                    .cmp(&tied(&left[b], &lies))
                    .then(takes(&left[a]).total_cmp(&takes(&left[b])))
                    .then(b.cmp(&a))
            })
            .unwrap_or(0);
        let item = left.remove(next);
        let mine: BTreeSet<usize> = item.iter().map(|placed| placed.0).collect();
        // (the pads of what lies already that this is joined to)
        let pad_of = |p: usize, i: usize, at: Pt, q: u8| {
            let from = quartered(q, cx.drawn_pad(p, i));
            [at[0] + from[0], at[1] + from[1]]
        };
        let mut theirs: BTreeMap<usize, Vec<Pt>> = BTreeMap::new();
        for (&q, &(at, turn)) in &lies {
            for (i, net) in parts[q].pads.iter().enumerate() {
                if let Some(net) = net.filter(|&net| cx.is_signal(net) && reach[net].iter().any(|p| mine.contains(p))) {
                    theirs.entry(net).or_default().push(pad_of(q, i, at, turn));
                }
            }
        }
        let all: Vec<Pt> = theirs.values().flatten().copied().collect();
        let want = median(&all).unwrap_or([0.0, 0.0]);
        let ways: Vec<Vec<[f64; 4]>> = (0..4u8)
            .map(|way| item.iter().map(|&(p, at, q)| cx.drawn_rect(p, quartered(way, at), (q + way) % 4)).collect())
            .collect();
        let tried = settled_shape(ground, &ways, want, &shut, UNIT_REACH);
        let apart = |way: u8, here: Pt| -> f64 {
            let mut far = 0.0;
            for &(p, at, q) in &item {
                let from = quartered(way, at);
                let at = [here[0] + from[0], here[1] + from[1]];
                for (i, net) in parts[p].pads.iter().enumerate() {
                    let Some(pads) = net.and_then(|net| theirs.get(&net)) else { continue };
                    let pad = pad_of(p, i, at, (q + way) % 4);
                    far += pads.iter().map(|&there| dist(pad, there)).fold(f64::INFINITY, f64::min);
                }
            }
            far
        };
        let nearest = tried.iter().flatten().map(|&at| dist(at, want)).fold(f64::INFINITY, f64::min);
        let best = tried
            .iter()
            .enumerate()
            .filter_map(|(way, at)| at.map(|at| (way as u8, at)))
            .filter(|&(_, at)| dist(at, want) <= nearest + AS_NEAR)
            .map(|(way, at)| (apart(way, at), way, at))
            .fold(None::<(f64, u8, Pt)>, |best, now| if best.is_some_and(|b| b.0 <= now.0) { best } else { Some(now) });
        let (_, way, here) = best?;
        for &(p, at, q) in &item {
            let from = quartered(way, at);
            let (to, turn) = ([here[0] + from[0], here[1] + from[1]], (q + way) % 4);
            lies.insert(p, (to, turn));
            shut.push(cx.drawn_rect(p, to, turn));
        }
    }
    Some((lies, shut))
}

/// The sheets that repeat; those laid out alike become blocks in `laid_blocks`, the copies side by side as
/// one body.
///
/// Sheets that repeat are laid out alike: one is laid out, by itself on empty ground, and every copy is
/// placed as that one lies, as one body. Two sheets repeat if their parts are of the same kinds, joined the
/// same way: told by colouring the parts, each by its kind, then again and again by what its pads are
/// joined to (the kinds and pads of the parts on each net within the sheet; whether the net is a ground,
/// a supply, or leaves the sheet), until two parts have one colour only if they sit alike in their
/// sheets. The parts of two such sheets are matched colour by colour.
pub(super) fn units(
    site: &Site,
    grounds: &BTreeMap<usize, [f64; 4]>,
    laid_blocks: &mut BTreeMap<usize, Block>,
) -> Units {
    let (cx, parts, part_domain) = (site.cx, site.cx.parts, &site.part_domain);
    let mut unit_groups: Vec<Copies> = Vec::new();
    // (for a body of several copies, by its first part: what is tried if it finds no room, in turn; each a
    // set of bodies, by a part of each)
    let mut alternatives: BTreeMap<usize, Vec<Vec<(usize, Block)>>> = BTreeMap::new();
    let mut of_sheet: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for p in (0..parts.len()).filter(|&p| !cx.stays[p]) {
        of_sheet.entry(cx.sheet(p)).or_default().push(p);
    }
    let mut by_make: BTreeMap<Vec<String>, Vec<&String>> = BTreeMap::new();
    for (name, own) in of_sheet.iter().filter(|(_, own)| own.len() >= REPEAT_LEAST) {
        let mut kinds: Vec<String> = own.iter().map(|&p| cx.kind(p)).collect();
        kinds.sort();
        by_make.entry(kinds).or_default().push(name);
    }
    for sheets in by_make.values().filter(|sheets| sheets.len() > 1) {
        let colour = coloured(cx, sheets, &of_sheet);
        // (each sheet's parts in the order of their colours; sheets with the same colours are one make-up)
        let mut same: BTreeMap<Vec<usize>, Vec<(&String, Vec<usize>)>> = BTreeMap::new();
        for &name in sheets {
            let mut own = of_sheet[name].clone();
            own.sort_by(|&a, &b| colour[&a].cmp(&colour[&b]).then(parts[a].name.cmp(&parts[b].name)));
            same.entry(own.iter().map(|p| colour[p]).collect()).or_default().push((name, own));
        }
        for copies in same.into_values().filter(|copies| copies.len() > 1) {
            let names: Vec<String> = copies.iter().map(|copy| copy.0.clone()).collect();
            // A part is of its sheet's body if, in every copy, it has ground in the copy's domain, with its
            // pads known, and stands in no block that reaches out of the sheet.
            let within_sheet = |own: &Vec<usize>, p: usize| -> bool {
                let block = in_blocks(p, laid_blocks);
                let inner =
                    block.map_or(true, |head| laid_blocks[&head].at.iter().all(|placed| own.contains(&placed.0)));
                grounds.contains_key(&p)
                    && part_domain.get(&p) == part_domain.get(&own[0])
                    && !parts[p].offsets.is_empty()
                    && inner
            };
            let whole: Vec<usize> =
                (0..copies[0].1.len()).filter(|&j| copies.iter().all(|(_, own)| within_sheet(own, own[j]))).collect();
            if whole.len() < REPEAT_LEAST {
                unit_groups.push((
                    names,
                    copies[0].1.len(),
                    Vec::new(),
                    copies.iter().map(|copy| copy.1.clone()).collect(),
                ));
                continue;
            }
            // The first is laid out: its blocks as they are, its other parts each by itself; the largest
            // first at the middle, then each time what is tied the most to what lies already, set down as
            // near the parts it is tied to as there is room, turned the way that leaves its pads nearest theirs.
            let model = &copies[0].1;
            let Some((lies, shut)) = laid_alone(cx, model, &whole, laid_blocks).filter(|(_, shut)| !shut.is_empty())
            else {
                unit_groups.push((
                    names,
                    copies[0].1.len(),
                    Vec::new(),
                    copies.iter().map(|copy| copy.1.clone()).collect(),
                ));
                continue;
            };
            let low = |k: usize| shut.iter().map(|r| r[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| shut.iter().map(|r| r[k + 2]).fold(f64::NEG_INFINITY, f64::max);
            let mid = [(low(0) + high(0)) / 2.0, (low(1) + high(1)) / 2.0];
            // (every copy is a block of its own parts lying as the first's: with the pads its own blocks gave)
            let mut keys: Vec<usize> = Vec::new();
            let mut singles: Vec<(usize, Block)> = Vec::new();
            for (_, own) in &copies {
                let mut pins: Vec<(usize, (usize, usize, usize))> = Vec::new();
                let mut at: Vec<(usize, Pt, u8)> = Vec::new();
                let mut drawn: Vec<(usize, Pt, u8)> = Vec::new();
                for &j in &whole {
                    let (p, (to, turn)) = (own[j], lies[&model[j]]);
                    let from = [to[0] - mid[0], to[1] - mid[1]];
                    at.push((p, from, (turn + 4 - cx.stands_turned(p)) % 4));
                    drawn.push((p, from, turn));
                }
                let mine: BTreeSet<usize> = at.iter().map(|placed| placed.0).collect();
                let heads: Vec<usize> = laid_blocks.keys().copied().filter(|head| mine.contains(head)).collect();
                for head in heads {
                    if let Some(block) = laid_blocks.remove(&head) {
                        pins.extend(block.pins);
                    }
                }
                let through = mine.iter().any(|&p| parts[p].through);
                keys.push(at[0].0);
                singles.push((at[0].0, Block { at, drawn, through, pins }));
            }
            // The copies stand together, side by side and turned alike, as one body: in a row; failing
            // that (from four of them) in two rows; failing that each by itself. So they are placed as a
            // designer lays repeated channels, and what feeds them all runs along them.
            let size = [high(0) - low(0), high(1) - low(1)];
            let side_by_side = |across: usize| -> Block {
                let down = singles.len().div_ceil(across);
                let mut all = Block { at: Vec::new(), drawn: Vec::new(), through: false, pins: Vec::new() };
                for (n, (_, one)) in singles.iter().enumerate() {
                    let step = [
                        ((n % across) as f64 - (across - 1) as f64 / 2.0) * (size[0] + cx.board.tune.between_copies_mm),
                        ((n / across) as f64 - (down - 1) as f64 / 2.0) * (size[1] + cx.board.tune.between_copies_mm),
                    ];
                    let moved =
                        |placed: &(usize, Pt, u8)| (placed.0, [placed.1[0] + step[0], placed.1[1] + step[1]], placed.2);
                    all.at.extend(one.at.iter().map(moved));
                    all.drawn.extend(one.drawn.iter().map(moved));
                    all.pins.extend(one.pins.iter().copied());
                    all.through |= one.through;
                }
                all
            };
            let key = singles[0].0;
            let mut then: Vec<Vec<(usize, Block)>> = Vec::new();
            if singles.len() >= 4 {
                then.push(vec![(key, side_by_side(singles.len().div_ceil(2)))]);
            }
            laid_blocks.insert(key, side_by_side(singles.len()));
            then.push(singles);
            alternatives.insert(key, then);
            unit_groups.push((names, whole.len(), keys, copies.iter().map(|copy| copy.1.clone()).collect()));
        }
    }
    Units { groups: unit_groups, alternatives }
}
