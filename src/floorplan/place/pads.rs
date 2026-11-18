//! The parts that stand by a pad are moved beside it.

use super::beside::Beside;
use super::blocks::Block;
use super::facing::{Facing, Pins};
use super::{Site, Standing};
use crate::floorplan::constants::{BLOCK_REACH, SETTLE};
use crate::floorplan::settle::{quartered, settled};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// The parts that stand by a pad, as they came to stand.
pub(super) struct ByPads {
    /// Each with its own pad, what it stands by, and that one's pad.
    pub(super) pins: Pins,
    /// How many stand by each pad of a part.
    pub(super) at_pad: BTreeMap<(usize, usize), usize>,
    /// Those that have their place by their pad: they are not moved from there.
    pub(super) by_its_pad: BTreeSet<usize>,
    /// The parts that others stand by.
    pub(super) with_capacitors: BTreeSet<usize>,
}

/// A capacitor that stands for a chip is moved beside the pad of the chip that is on its supply (the pads
/// of the chip taking them in turn, the smallest capacitors first: they want to be the nearest), just off
/// the chip's body, as near that as what stands there leaves room; if that is nearer the pad than it stood.
/// And so each part that stands by a pad.
pub(super) fn by_pads(
    site: &Site,
    facing: &Facing,
    beside: &Beside,
    gap_rects: &BTreeMap<usize, [f64; 4]>,
    laid_blocks: &BTreeMap<usize, Block>,
    standing: &mut Standing,
) -> ByPads {
    let (cx, top, roads, on_gap) = (site.cx, site.top, site.roads, &site.on_gap);
    let (parts, domain_of_part, block_quarter) = (cx.parts, &site.domain_of_part, facing.block_quarter);
    let (bypasses, order) = (&beside.bypasses, &beside.order);
    let Standing { stood, centre, quarter } = standing;
    let mut pins: Pins = BTreeMap::new();
    let mut at_pad: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let with_capacitors: BTreeSet<usize> = bypasses.iter().map(|bypass| bypass.1).collect();
    let mut by_its_pad: BTreeSet<usize> = BTreeSet::new();
    // (what stands in a block has its place and its pad from the block; the rest is taken one by one)
    for (&head, block) in laid_blocks.iter().filter(|(head, _)| block_quarter.contains_key(head)) {
        pins.extend(block.pins.iter().copied());
        by_its_pad.extend(block.at.iter().map(|placed| placed.0).filter(|&m| m != head));
    }
    for &(c, chip, rail) in order {
        if block_quarter.contains_key(&c) {
            continue;
        }
        let (Some(was), Some(chip_at)) = (centre[c], centre[chip]) else { continue };
        if parts[chip].offsets.is_empty() || on_gap.contains_key(&c) {
            continue;
        }
        let pad_of = |i: usize| {
            let from = quartered(quarter[chip], parts[chip].offsets[i]);
            [chip_at[0] + from[0], chip_at[1] + from[1]]
        };
        let on_rail = (0..parts[chip].pads.len()).filter(|&i| parts[chip].pads[i] == Some(rail));
        // (a part whose other pad is on a net of the same chip stands between the two pads: of the chip's
        // pads on this net, the one nearest a pad of the chip on that other net; else the pads in turn)
        let partners: Vec<Pt> = parts[c]
            .pads
            .iter()
            .flatten()
            .filter(|&&other| other != rail && cx.is_signal(other))
            .flat_map(|&other| (0..parts[chip].pads.len()).filter(move |&i| parts[chip].pads[i] == Some(other)))
            .map(pad_of)
            .collect();
        let across = |i: usize| {
            let nearest = partners.iter().map(|&there| dist(pad_of(i), there)).fold(f64::INFINITY, f64::min);
            if nearest.is_finite() {
                (nearest / SETTLE).round() as i64
            } else {
                0
            }
        };
        let Some(pin) = on_rail.min_by(|&a, &b| {
            let taken = |i: usize| at_pad.get(&(chip, i)).copied().unwrap_or(0);
            across(a)
                .cmp(&across(b))
                .then(taken(a).cmp(&taken(b)))
                .then(dist(pad_of(a), was).total_cmp(&dist(pad_of(b), was)))
                .then(a.cmp(&b))
        }) else {
            continue;
        };
        *at_pad.entry((chip, pin)).or_insert(0) += 1;
        if let Some(mine) = parts[c].pads.iter().position(|&net| net == Some(rail)) {
            pins.insert(c, (mine, chip, pin));
        }
        // (just off the chip's body, on the side its pad is nearest)
        let (pad, [w, h]) = (pad_of(pin), cx.body(chip));
        let (w, h) = if quarter[chip] % 2 == 1 { (h, w) } else { (w, h) };
        let chip_body = [chip_at[0] - w / 2.0, chip_at[1] - h / 2.0, chip_at[0] + w / 2.0, chip_at[1] + h / 2.0];
        // (lying away from the chip, its own pad on the supply toward the chip's: so the track between is short)
        let [cw, ch] = cx.body(c);
        let (long, short) = (cw.max(ch), cw.min(ch));
        let half = long / 2.0;
        let sides = [
            (pad[0] - chip_body[0], [chip_body[0] - half, pad[1]], true),
            (chip_body[2] - pad[0], [chip_body[2] + half, pad[1]], true),
            (pad[1] - chip_body[1], [pad[0], chip_body[1] - half], false),
            (chip_body[3] - pad[1], [pad[0], chip_body[3] + half], false),
        ];
        let (_, want, along_x) = sides.iter().fold(sides[0], |best, side| if side.0 < best.0 { *side } else { best });
        let lying = if along_x { [long, short] } else { [short, long] };
        // (whether lying so is a quarter from how its body is given)
        let quarter_more = (lying[0] - cw).abs() > 1e-9;
        // It takes the place it wants there though a small part stands on it: that one is settled again, as
        // near where it stood as may be. What is not moved for it: what was settled first, a chip that has
        // capacitors of its own, and a capacitor that has its place by its pad.
        let domain = domain_of_part[c];
        let (ground, mine) = (top[domain].rect, &top[domain].parts);
        let stands = |q: usize| !on_gap.contains_key(&q) && stood.get(&q).is_some_and(|at| at.is_some());
        let hard = |q: usize| cx.goes_first(q) || with_capacitors.contains(&q) || by_its_pad.contains(&q);
        let rect_of = |q: usize, centre: &[Option<Pt>], quarter: &[u8]| {
            centre[q].filter(|_| stands(q)).map(|at| cx.taken(q, (at, quarter[q] % 2 == 1)))
        };
        let mut shut: Vec<[f64; 4]> = site.closed(gap_rects);
        shut.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
        let open = shut.clone();
        let meets = |a: usize, b: usize| cx.side_of(a) & cx.side_of(b) != 0;
        shut.extend(
            mine.iter().filter(|&&q| q != c && hard(q) && meets(q, c)).filter_map(|&q| rect_of(q, centre, quarter)),
        );
        let Some((at, turn)) = settled(ground, &[(lying, want)], &shut)[0] else { continue };
        if dist(at, pad) >= dist(was, pad) - 1e-6 {
            // (it stands as near its pad already as there is room to: if that is by it, it is not moved from
            // there later; if it is far, it is left to be moved with the other small parts)
            if dist(was, pad) <= BLOCK_REACH {
                by_its_pad.insert(c);
            }
            continue;
        }
        let turn = turn != quarter_more;
        let (mut new_centre, mut new_quarter) = (centre.clone(), quarter.clone());
        (new_centre[c], new_quarter[c]) = (Some(at), u8::from(turn));
        let here = cx.taken(c, (at, turn));
        let over =
            |r: [f64; 4]| r[2].min(here[2]) - r[0].max(here[0]) > 1e-6 && r[3].min(here[3]) - r[1].max(here[1]) > 1e-6;
        let mut moved: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|&q| q != c && !hard(q) && meets(q, c) && rect_of(q, centre, quarter).is_some_and(over))
            .collect();
        moved.sort_by(|&a, &b| parts[b].area.total_cmp(&parts[a].area).then(a.cmp(&b)));
        let mut done = true;
        for &q in &moved {
            let mut shut = open.clone();
            let others = mine.iter().filter(|&&other| other != q && meets(other, q) && (other == c || stands(other)));
            shut.extend(
                others.filter_map(|&other| {
                    new_centre[other].map(|at| cx.taken(other, (at, new_quarter[other] % 2 == 1)))
                }),
            );
            match (centre[q], settled(ground, &[(cx.body(q), centre[q].unwrap_or(at))], &shut)[0]) {
                (Some(_), Some((to, turn))) => (new_centre[q], new_quarter[q]) = (Some(to), u8::from(turn)),
                _ => done = false,
            }
        }
        // (and only if what it moves and itself are the nearer what they are joined to for it, all told)
        let reach = |centre: &[Option<Pt>], quarter: &[u8]| -> f64 {
            moved.iter().chain([&c]).map(|&q| facing.reach_of(q, quarter[q], centre, quarter, &pins)).sum()
        };
        if done && reach(&new_centre, &new_quarter) < reach(centre, quarter) - 1e-6 {
            for &q in moved.iter().chain([&c]) {
                if let Some(at) = new_centre[q] {
                    stood.insert(q, Some((at, new_quarter[q] % 2 == 1)));
                }
            }
            (*centre, *quarter) = (new_centre, new_quarter);
            by_its_pad.insert(c);
        }
    }
    ByPads { pins, at_pad, by_its_pad, with_capacitors }
}
