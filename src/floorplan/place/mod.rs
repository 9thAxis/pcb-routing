//! A place for every part within its zone.

mod beside;
mod blocks;
mod gaps;
mod settling;
mod units;

use self::beside::{beside, Beside};
use self::blocks::{alike, blocks, Block};
use self::gaps::on_gaps;
use self::settling::grounds;
use self::units::{units, Units};
use super::boards::Boards;
use super::constants::*;
use super::ground::Work;
use super::known::Known;
use super::settle::{median, middle, quartered, round1, round2, settled, settled_shape, spread, strip_between};
use super::types::*;
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// Which domain has a name.
pub(super) fn domain_named(top: &[Work], name: &str) -> Option<usize> {
    top.iter().position(|work| work.name == name)
}

/// What the placing of the parts reads at every step: what is known of the board, the layout chosen, and
/// what follows from them for each part.
pub(super) struct Site<'a> {
    pub(super) cx: &'a Known<'a>,
    /// The zones with their ground, a domain at the top of each tree.
    pub(super) top: &'a [Work],
    pub(super) roads: &'a [Road],
    pub(super) on: &'a Boards,
    /// The parts on the gaps: where the middle of each is, and how many quarters it is turned.
    pub(super) on_gap: BTreeMap<usize, Pt>,
    pub(super) gap_quarter: BTreeMap<usize, u8>,
    /// The smallest zones: the domain, the ground and the parts of each.
    pub(super) leaves: Vec<(usize, [f64; 4], Vec<usize>)>,
    /// The domain of each part that is of a zone.
    pub(super) part_domain: BTreeMap<usize, usize>,
    pub(super) domain_of_part: Vec<usize>,
    /// What is added to the place of each part to say where that is in the stack.
    pub(super) shift_of: Vec<Pt>,
}

impl<'a> Site<'a> {
    fn of(
        cx: &'a Known<'a>,
        top: &'a [Work],
        roads: &'a [Road],
        on: &'a Boards,
        on_gap: BTreeMap<usize, Pt>,
        gap_quarter: BTreeMap<usize, u8>,
    ) -> Site<'a> {
        let (parts, shifts, to) = (cx.parts, &on.shifts, &on.to);
        let mut fine_leaves: Vec<(usize, [f64; 4], Vec<usize>)> = Vec::new();
        fn parts_of_leaves(work: &Work, domain: usize, out: &mut Vec<(usize, [f64; 4], Vec<usize>)>) {
            if work.inside.is_empty() {
                out.push((domain, work.rect, work.parts.clone()));
            }
            work.inside.iter().for_each(|inner| parts_of_leaves(inner, domain, out));
        }
        top.iter().enumerate().for_each(|(domain, work)| parts_of_leaves(work, domain, &mut fine_leaves));
        let part_domain: BTreeMap<usize, usize> =
            fine_leaves.iter().flat_map(|(domain, _, own)| own.iter().map(move |&p| (p, *domain))).collect();
        let mut shift_of: Vec<Pt> = vec![[0.0, 0.0]; parts.len()];
        let mut domain_of_part: Vec<usize> = vec![0; parts.len()];
        for (domain, _, own) in &fine_leaves {
            for &p in own {
                (shift_of[p], domain_of_part[p]) = (shifts[to[*domain]], *domain);
            }
        }
        Site { cx, top, roads, on, on_gap, gap_quarter, leaves: fine_leaves, part_domain, domain_of_part, shift_of }
    }

    /// The ground that is shut to every part: what the board shuts, and the bodies on the gaps.
    pub(super) fn closed(&self, gaps: &BTreeMap<usize, [f64; 4]>) -> Vec<[f64; 4]> {
        self.cx.board.keepouts.iter().chain(gaps.values()).copied().collect()
    }
}

/// What the placing of the parts comes to, as the plan tells it.
pub(super) struct Placed {
    pub(super) blocks: [usize; 2],
    pub(super) alike: Vec<(usize, usize, usize, Vec<String>)>,
    pub(super) disturbing: [usize; 2],
    pub(super) close: Vec<(String, String, f64)>,
    pub(super) sensitive: (usize, f64, f64, String),
    pub(super) repeats: Vec<Repeat>,
    pub(super) served: [usize; 2],
    pub(super) chains: usize,
    pub(super) back: Vec<(String, String, Vec<String>, f64, f64)>,
    pub(super) loops: Vec<Loop>,
    pub(super) places: Vec<Place>,
    pub(super) parts_now_mm: f64,
    pub(super) parts_then_mm: f64,
    pub(super) against: Vec<(String, usize, f64, f64)>,
    pub(super) pads_first_mm: f64,
    pub(super) pads_mm: f64,
    pub(super) bypass: [usize; 2],
    pub(super) capacitors: Vec<(String, String, f64, usize)>,
}

/// A place for every part within its zone, and what that comes to. The bridges are told where the parts on
/// the gaps came to stand.
pub(super) fn placed(cx: &Known, top: &[Work], roads: &[Road], on: &Boards, bridges: &mut [Bridge]) -> Placed {
    let gaps = on_gaps(cx, top, on, bridges);
    let mut gap_rects = gaps.rects;
    let site = &Site::of(cx, top, roads, on, gaps.on, gaps.quarter);
    let beside = beside(site);
    let grounds = grounds(site, &beside.fine_nets);
    let mut laid_blocks = blocks(site, &beside.order, &grounds);
    let alike = alike(cx, &laid_blocks);
    let blocks = [laid_blocks.len(), laid_blocks.values().map(|block| block.at.len()).sum()];
    let Units { groups: unit_groups, alternatives } = units(site, &grounds, &mut laid_blocks);
    let (parts, all_nets, found, reach, nets) = (cx.parts, cx.all_nets, &cx.found, &cx.reach, &cx.nets);
    let (board, aggressor, victim) = (cx.board, &cx.aggressor, &cx.victim);
    let (roads, boards, shifts, to) = (site.roads, &on.boards, &on.shifts, &on.to);
    let (on_gap, gap_quarter, fine_leaves) = (&site.on_gap, &site.gap_quarter, &site.leaves);
    let (shift_of, domain_of_part) = (&site.shift_of, &site.domain_of_part);
    let Beside { fine_nets, bypasses, order, switchers, served_parts } = &beside;
    let mut in_rows: BTreeMap<usize, usize> = BTreeMap::new(); // a body of copies, by its first part -> the copies side by side
    let mut places: Vec<Place> = Vec::new();
    let mut spot: Vec<Option<Pt>> = vec![None; parts.len()];
    // (where each came to stand: its middle and whether it is turned; None where there was no room. A domain's
    // bodies are settled together, off its roads and what is shut: a large part may reach over the ground of
    // its zone, which was cut by what its parts take and not by their shapes)
    let mut stood: BTreeMap<usize, Option<(Pt, bool)>> = BTreeMap::new();
    // First the parts with holes and the large ones, over the whole ground of their domain: they are the
    // hardest to find room for. No track runs under a part with holes: it stands off the gaps and the roads
    // that part much of the board (a lesser road, a track or two, goes round it); the others may stand over
    // a road, whose tracks run on the layers beneath.
    // (what of a block's parts found no room in it is no part of it, and is placed with the rest)
    let in_block: BTreeMap<usize, usize> =
        laid_blocks.iter().flat_map(|(&head, block)| block.at.iter().map(move |&(p, _, _)| (p, head))).collect();
    let holed = |p: usize| laid_blocks.get(&p).map_or(parts[p].through, |block| block.through);
    let mut block_quarter: BTreeMap<usize, u8> = BTreeMap::new();
    // What disturbs is placed before all else, and what is disturbed then keeps QUIET_MM from it wherever
    // there is room for it so: the ground that near each part that disturbs, as it comes to stand.
    let mut halos: Vec<[f64; 4]> = Vec::new();
    let loud =
        |p: usize| laid_blocks.get(&p).map_or(aggressor[p], |block| block.at.iter().any(|placed| aggressor[placed.0]));
    let round_it = |r: [f64; 4]| [r[0] - QUIET_MM, r[1] - QUIET_MM, r[2] + QUIET_MM, r[3] + QUIET_MM];
    // (where a part that is not yet settled is taken to come: the middle of the ground that came to it)
    let wanted = |q: usize| on_gap.get(&q).copied().or_else(|| grounds.get(&q).map(|&r| middle(r)));
    for work in top.iter() {
        let mine: Vec<usize> = work
            .parts
            .iter()
            .copied()
            .filter(|&p| {
                grounds.contains_key(&p)
                    && (laid_blocks.contains_key(&p) || (cx.goes_first(p) && !in_block.contains_key(&p)))
            })
            .collect();
        let mut sides: [Vec<[f64; 4]>; 2] = [Vec::new(), Vec::new()]; // what stands on the front, and on the back
        for with_holes in [true, false] {
            let lot: Vec<usize> = mine.iter().copied().filter(|&p| holed(p) == with_holes).collect();
            let mut base: Vec<[f64; 4]> = site.closed(&gap_rects);
            base.extend(
                roads.iter().filter(|road| road.gap || (with_holes && road.level <= MAIN)).map(|road| road.rect),
            );
            // (the largest first, a block by what its parts take; each one by one, on what the others left)
            let takes = |p: usize| -> f64 {
                // (copies side by side take their turn as one of them would: by what one copy takes)
                let copies =
                    alternatives.get(&p).and_then(|options| options.last()).map_or(1, |singles| singles.len().max(1));
                laid_blocks.get(&p).map_or(cx.body(p)[0] * cx.body(p)[1], |block| {
                    block.at.iter().map(|&(m, _, _)| cx.body(m)[0] * cx.body(m)[1]).sum::<f64>() / copies as f64
                })
            };
            let mut lot = lot;
            lot.sort_by(|&a, &b| loud(b).cmp(&loud(a)).then(takes(b).total_cmp(&takes(a))).then(a.cmp(&b)));
            for p in lot {
                let want = middle(grounds[&p]);
                // (what is in its way: what is shut, and what stands on the sides it takes ground on)
                let on = laid_blocks
                    .get(&p)
                    .map_or(cx.side_of(p), |block| block.at.iter().fold(0, |on, placed| on | cx.side_of(placed.0)));
                let mut shut: Vec<[f64; 4]> = base.clone();
                (0..2).filter(|k| on & (1 << k) != 0).for_each(|k| shut.extend(sides[k].iter().copied()));
                // (and the same with the roads open: a part with holes stands off the main roads if it can, but
                // on a small board a part may be as large as what the roads leave)
                let mut over_roads: Vec<[f64; 4]> = site.closed(&gap_rects);
                over_roads.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
                (0..2).filter(|k| on & (1 << k) != 0).for_each(|k| over_roads.extend(sides[k].iter().copied()));
                let Some(first) = laid_blocks.get(&p) else {
                    let kept: Vec<[f64; 4]> = shut.iter().chain(&halos).copied().collect();
                    let away = if victim[p] { settled(work.rect, &[(cx.body(p), want)], &kept)[0] } else { None };
                    let at = away
                        .or_else(|| settled(work.rect, &[(cx.body(p), want)], &shut)[0])
                        .or_else(|| settled(work.rect, &[(cx.body(p), want)], &over_roads)[0]);
                    if aggressor[p] {
                        halos.extend(at.map(|at| round_it(cx.taken(p, at))));
                    }
                    if let Some(at) = at {
                        (0..2)
                            .filter(|k| cx.side_of(p) & (1 << k) != 0)
                            .for_each(|k| sides[k].extend(cx.stands_on(p, at)));
                    }
                    stood.insert(p, at);
                    continue;
                };
                // (what is tried in turn: the block; for copies side by side, then what else they may be.
                // All but the last only near where they are wanted; the last anywhere, and what of it finds
                // no room is taken apart)
                let mut options: Vec<Vec<(usize, &Block)>> = vec![vec![(p, first)]];
                if let Some(more) = alternatives.get(&p) {
                    options.extend(more.iter().map(|option| option.iter().map(|(key, block)| (*key, block)).collect()));
                }
                if board.rows == Rows::Never && options.len() > 1 {
                    options = options.split_off(options.len() - 1);
                }
                let last = options.len() - 1;
                // (where a block would come to stand, of the ways it may be turned: placed by the bodies of
                // its parts, not by a box round them (blocks then lie into each other's corners); of the ways
                // that come about as near where it is wanted as the nearest, the one that leaves its pads
                // nearest what they join outside it, with how far that is)
                let best_for = |block: &Block, anywhere: bool, shut: &Vec<[f64; 4]>| -> Option<(f64, u8, Pt)> {
                    let pieces = |way: u8| -> Vec<[f64; 4]> {
                        block
                            .at
                            .iter()
                            .map(|&(m, from, q)| cx.taken(m, (quartered(way, from), (q + way) % 2 == 1)))
                            .collect()
                    };
                    let ways: Vec<Vec<[f64; 4]>> = (0..4).map(pieces).collect();
                    let outside = |way: u8, here: Pt| -> f64 {
                        let mut all = 0.0;
                        for &(m, from, q) in &block.at {
                            for (i, net) in parts[m].pads.iter().enumerate() {
                                let (Some(net), Some(&offset)) =
                                    (net.filter(|&net| cx.is_signal(net)), parts[m].offsets.get(i))
                                else {
                                    continue;
                                };
                                let own = quartered(q, offset);
                                let pad = quartered(way, [from[0] + own[0], from[1] + own[1]]);
                                let pad = [here[0] + pad[0], here[1] + pad[1]];
                                let nearest = reach[net]
                                    .iter()
                                    .filter(|other| in_block.get(other) != Some(&p))
                                    .filter_map(|&other| wanted(other))
                                    .map(|there| (there[0] - pad[0]).abs() + (there[1] - pad[1]).abs())
                                    .fold(f64::INFINITY, f64::min);
                                if nearest.is_finite() {
                                    all += nearest;
                                }
                            }
                        }
                        all
                    };
                    // (a block is wanted where the grounds that came to its parts are, taken together)
                    let want = {
                        let (mut sum, mut all) = ([0.0, 0.0], 0.0);
                        for &(m, _, _) in &block.at {
                            if let Some(&ground) = grounds.get(&m) {
                                let (mid, weight) = (middle(ground), cx.body(m)[0] * cx.body(m)[1]);
                                sum = [sum[0] + weight * mid[0], sum[1] + weight * mid[1]];
                                all += weight;
                            }
                        }
                        if all > 0.0 {
                            [sum[0] / all, sum[1] / all]
                        } else {
                            want
                        }
                    };
                    let mut tried = settled_shape(work.rect, &ways, want, shut, BLOCK_NEAR);
                    if anywhere && tried.iter().all(|at| at.is_none()) {
                        tried = settled_shape(work.rect, &ways, want, shut, f64::INFINITY);
                    }
                    let nearest = tried.iter().flatten().map(|&at| dist(at, want)).fold(f64::INFINITY, f64::min);
                    tried
                        .iter()
                        .enumerate()
                        .filter_map(|(way, at)| at.map(|at| (way as u8, at)))
                        .filter(|&(_, at)| dist(at, want) <= nearest + AS_NEAR)
                        .map(|(way, at)| (outside(way, at), way, at))
                        .fold(
                            None::<(f64, u8, Pt)>,
                            |best, now| if best.is_some_and(|b| b.0 <= now.0) { best } else { Some(now) },
                        )
                };
                for (n, option) in options.iter().enumerate() {
                    let mut placed_all = true;
                    for &(_, block) in option {
                        let anywhere = n == last || board.rows == Rows::Always;
                        let kept: Vec<[f64; 4]> = shut.iter().chain(&halos).copied().collect();
                        let hushed = block.at.iter().any(|placed| victim[placed.0]);
                        let away = if hushed { best_for(block, false, &kept) } else { None };
                        let mut chosen = away.or_else(|| best_for(block, anywhere, &shut)).or_else(|| {
                            if n == last {
                                best_for(block, true, &over_roads)
                            } else {
                                None
                            }
                        });
                        // (copies side by side are worth it if their pads are not much further from what
                        // they join than with each copy where it would stand by itself)
                        if let (true, Some((together, _, _))) = (n < last && board.rows == Rows::Cheap, chosen) {
                            let alone: f64 = options[last]
                                .iter()
                                .map(|&(_, one)| best_for(one, false, &shut).map_or(f64::INFINITY, |c| c.0))
                                .sum();
                            if together > ROW_WORTH * alone {
                                chosen = None;
                            }
                        }
                        let Some((_, way, here)) = chosen else {
                            placed_all = false;
                            if n == last {
                                // (no room for it as a block: its parts are placed with the rest, each by
                                // itself; but those that go first have their place now, before the small ones)
                                for &(m, _, _) in &block.at {
                                    let alone = if cx.goes_first(m) {
                                        settled(work.rect, &[(cx.body(m), middle(grounds[&m]))], &shut)[0]
                                    } else {
                                        None
                                    };
                                    if let Some(at) = alone {
                                        shut.extend(cx.stands_on(m, at));
                                        (0..2)
                                            .filter(|k| cx.side_of(m) & (1 << k) != 0)
                                            .for_each(|k| sides[k].extend(cx.stands_on(m, at)));
                                    }
                                    stood.insert(m, alone);
                                }
                                continue;
                            }
                            break;
                        };
                        for &(m, from, q) in &block.at {
                            let from = quartered(way, from);
                            let (to, quarters) = ([here[0] + from[0], here[1] + from[1]], (q + way) % 4);
                            stood.insert(m, Some((to, quarters % 2 == 1)));
                            block_quarter.insert(m, quarters);
                            shut.push(cx.taken(m, (to, quarters % 2 == 1)));
                            (0..2)
                                .filter(|k| cx.side_of(m) & (1 << k) != 0)
                                .for_each(|k| sides[k].extend(cx.stands_on(m, (to, quarters % 2 == 1))));
                            if aggressor[m] {
                                halos.push(round_it(cx.taken(m, (to, quarters % 2 == 1))));
                            }
                        }
                    }
                    if placed_all || n == last {
                        // (how many copies came to stand side by side as one body)
                        if placed_all && n < last && alternatives.contains_key(&p) {
                            in_rows.insert(p, alternatives[&p][last - 1].len());
                        }
                        break;
                    }
                }
            }
        }
    }
    // Then the others, zone by zone, each on the ground of its own.
    let settled_first: Vec<([f64; 4], u8)> = stood
        .iter()
        .filter_map(|(&p, at)| at.map(|at| (p, at)))
        .flat_map(|(p, at)| cx.stands_on(p, at).into_iter().map(move |r| (r, cx.side_of(p))))
        .collect();
    for (side, (_, rect, own)) in [1u8, 2].into_iter().flat_map(|side| fine_leaves.iter().map(move |leaf| (side, leaf)))
    {
        let mine: Vec<usize> = own
            .iter()
            .copied()
            .filter(|&p| {
                grounds.contains_key(&p) && !cx.goes_first(p) && !in_block.contains_key(&p) && cx.side_of(p) == side
            })
            .collect();
        let mut shut: Vec<[f64; 4]> = site.closed(&gap_rects);
        shut.extend(settled_first.iter().filter(|first| first.1 & side != 0).map(|first| first.0));
        // (what is disturbed first, off the ground near what disturbs; what of it finds no room so, with the rest)
        let hushed: Vec<usize> = mine.iter().copied().filter(|&p| victim[p]).collect();
        let kept: Vec<[f64; 4]> = shut.iter().chain(&halos).copied().collect();
        let bodies: Vec<([f64; 2], Pt)> = hushed.iter().map(|&p| (cx.body(p), middle(grounds[&p]))).collect();
        for (&p, at) in hushed.iter().zip(settled(*rect, &bodies, &kept)) {
            if let Some(at) = at {
                stood.insert(p, Some(at));
                shut.extend(cx.stands_on(p, at));
            }
        }
        let mine: Vec<usize> = mine.into_iter().filter(|p| !stood.get(p).is_some_and(|at| at.is_some())).collect();
        let bodies: Vec<([f64; 2], Pt)> = mine.iter().map(|&p| (cx.body(p), middle(grounds[&p]))).collect();
        stood.extend(mine.iter().copied().zip(settled(*rect, &bodies, &shut)));
    }
    // What found no room in its zone (a zone cut by what its parts take, not by their shapes) is settled on
    // what is left of the ground of its domain, off every body that has its place: first the parts with
    // holes, off the roads and gaps too; then the others, which may stand over a road (its tracks run on
    // the layers beneath them) but over no gap.
    for work in top.iter() {
        for (with_holes, side) in [(true, 3u8), (false, 1), (false, 2)] {
            let left: Vec<usize> = work
                .parts
                .iter()
                .copied()
                .filter(|&p| stood.get(&p) == Some(&None) && parts[p].through == with_holes && cx.side_of(p) == side)
                .collect();
            if left.is_empty() {
                continue;
            }
            let mut shut: Vec<[f64; 4]> = site.closed(&gap_rects);
            shut.extend(
                roads.iter().filter(|road| road.gap || (with_holes && road.level <= MAIN)).map(|road| road.rect),
            );
            let in_the_way = work.parts.iter().filter(|&&p| cx.side_of(p) & side != 0);
            shut.extend(
                in_the_way
                    .filter_map(|&p| stood.get(&p).copied().flatten().map(|at| (p, at)))
                    .flat_map(|(p, at)| cx.stands_on(p, at)),
            );
            let bodies: Vec<([f64; 2], Pt)> = left.iter().map(|&p| (cx.body(p), middle(grounds[&p]))).collect();
            stood.extend(left.iter().copied().zip(settled(work.rect, &bodies, &shut)));
            // (a part with holes that found no room off the main roads may stand on one: one by one)
            if with_holes {
                for &p in &left {
                    if stood.get(&p) != Some(&None) {
                        continue;
                    }
                    let mut open: Vec<[f64; 4]> = site.closed(&gap_rects);
                    open.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
                    let others = work.parts.iter().filter(|&&q| q != p);
                    open.extend(
                        others
                            .filter_map(|&q| stood.get(&q).copied().flatten().map(|at| (q, at)))
                            .flat_map(|(q, at)| cx.stands_on(q, at)),
                    );
                    stood.insert(p, settled(work.rect, &[(cx.body(p), middle(grounds[&p]))], &open)[0]);
                }
            }
        }
    }
    // And what found no room anywhere stays where it stands today, if it stands on the board and nothing that
    // has its place is in its way there: its designer found that room for it.
    let homeless: Vec<usize> = (0..parts.len()).filter(|p| stood.get(p) == Some(&None) && parts[*p].placed).collect();
    for p in homeless {
        let here = cx.taken(p, (parts[p].at, false));
        let over =
            |r: [f64; 4]| r[2].min(here[2]) - r[0].max(here[0]) > 1e-6 && r[3].min(here[3]) - r[1].max(here[1]) > 1e-6;
        let in_its_way = stood
            .iter()
            .filter(|(&q, _)| q != p && cx.side_of(q) & cx.side_of(p) != 0)
            .filter_map(|(&q, at)| at.map(|at| (q, at)))
            .flat_map(|(q, at)| cx.stands_on(q, at))
            .any(over);
        if !in_its_way && !board.keepouts.iter().any(|&shut| over(shut)) {
            stood.insert(p, Some((parts[p].at, false)));
        }
    }
    // Where the middle of each part is (as its board is drawn), how many quarters it is turned, and what is
    // added to its place to say where that is in the stack.
    let mut centre: Vec<Option<Pt>> = vec![None; parts.len()];
    let mut quarter: Vec<u8> = vec![0; parts.len()];
    for (_, _, own) in fine_leaves {
        for &p in own {
            match (on_gap.get(&p), stood.get(&p)) {
                (Some(&at), _) => (centre[p], quarter[p]) = (Some(at), gap_quarter.get(&p).copied().unwrap_or(0)),
                (None, Some(&Some((at, turn)))) => (centre[p], quarter[p]) = (Some(at), u8::from(turn)),
                // (no room found: told where it stands today if it stands on the board, else where it was pulled)
                (None, Some(None)) => centre[p] = Some(if parts[p].placed { parts[p].at } else { middle(grounds[&p]) }),
                _ => {}
            }
            if let Some(&quarters) = block_quarter.get(&p).filter(|_| stood.get(&p).is_some_and(|at| at.is_some())) {
                quarter[p] = quarters;
            }
        }
    }
    // Then each part is turned so that its pads face what they are joined to: of the ways its body may lie
    // (as it was settled, or that turned half round; any of the four for a square one or one on a gap), the
    // one that leaves its pads nearest the nearest other pad of their nets. A capacitor that stands for a
    // chip counts the way from its pad on the supply to the pad of the chip it is for.
    let mut pads_on: Vec<Vec<(usize, usize)>> = vec![Vec::new(); all_nets.len()];
    for (p, part) in parts.iter().enumerate() {
        for (i, net) in part.pads.iter().enumerate() {
            if let Some(net) = net.filter(|&net| cx.is_signal(net)) {
                pads_on[net].push((p, i));
            }
        }
    }
    let pad_at = |p: usize, i: usize, quarters: u8, centre: &[Option<Pt>]| -> Option<Pt> {
        let at = centre[p]?;
        let from = parts[p].offsets.get(i).map_or([0.0, 0.0], |&offset| quartered(quarters, offset));
        Some([at[0] + shift_of[p][0] + from[0], at[1] + shift_of[p][1] + from[1]])
    };
    type Pins = BTreeMap<usize, (usize, usize, usize)>; // a capacitor -> (its pad, its chip, the chip's pad)
    let reach_of = |p: usize, quarters: u8, centre: &[Option<Pt>], quarter: &[u8], pins: &Pins| -> f64 {
        let mut all = 0.0;
        for (i, net) in parts[p].pads.iter().enumerate() {
            let (Some(net), Some(here)) = (net, pad_at(p, i, quarters, centre)) else { continue };
            let others = pads_on[*net].iter().filter(|other| other.0 != p);
            let nearest = others
                .filter_map(|&(q, j)| pad_at(q, j, quarter[q], centre))
                .map(|there| (there[0] - here[0]).abs() + (there[1] - here[1]).abs())
                .fold(f64::INFINITY, f64::min);
            if nearest.is_finite() {
                all += nearest;
            }
        }
        if let Some(&(i, chip, j)) = pins.get(&p) {
            if let (Some(here), Some(there)) = (pad_at(p, i, quarters, centre), pad_at(chip, j, quarter[chip], centre))
            {
                all += BYPASS as f64 * ((there[0] - here[0]).abs() + (there[1] - here[1]).abs());
            }
        }
        // (and a chip counts the ways to the capacitors that stand for it)
        for (&c, &(i, _, j)) in pins.iter().filter(|(_, pin)| pin.1 == p) {
            if let (Some(here), Some(there)) = (pad_at(p, j, quarters, centre), pad_at(c, i, quarter[c], centre)) {
                all += BYPASS as f64 * ((there[0] - here[0]).abs() + (there[1] - here[1]).abs());
            }
        }
        all
    };
    let face = |centre: &[Option<Pt>], quarter: &mut Vec<u8>, pins: &Pins| {
        for _ in 0..2 {
            for p in 0..parts.len() {
                if centre[p].is_none()
                    || on_gap.contains_key(&p)
                    || parts[p].offsets.is_empty()
                    || cx.stays[p]
                    || block_quarter.contains_key(&p)
                {
                    continue;
                }
                // (a body in pieces is not turned again: the ground between its pieces is taken as it stands)
                if !parts[p].pieces.is_empty() {
                    continue;
                }
                let [w, h] = cx.body(p);
                let any = on_gap.contains_key(&p) || (w - h).abs() < SETTLE;
                let ways: Vec<u8> = if any { vec![0, 1, 2, 3] } else { vec![quarter[p] % 2, quarter[p] % 2 + 2] };
                let mut best = (reach_of(p, quarter[p], centre, quarter, pins), quarter[p]);
                for way in ways {
                    let far = reach_of(p, way, centre, quarter, pins);
                    if far < best.0 - 1e-6 {
                        best = (far, way);
                    }
                }
                quarter[p] = best.1;
            }
        }
    };
    let no_pins: Pins = BTreeMap::new();
    let all_reach = |centre: &[Option<Pt>], quarter: &[u8]| -> f64 {
        (0..parts.len()).map(|p| reach_of(p, quarter[p], centre, quarter, &no_pins)).sum()
    };
    let pads_first_mm = all_reach(&centre, &quarter);
    face(&centre, &mut quarter, &no_pins);
    // A capacitor that stands for a chip is moved beside the pad of the chip that is on its supply (the pads
    // of the chip taking them in turn, the smallest capacitors first: they want to be the nearest), just off
    // the chip's body, as near that as what stands there leaves room; if that is nearer the pad than it stood.
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
        let mut shut: Vec<[f64; 4]> = site.closed(&gap_rects);
        shut.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
        let open = shut.clone();
        let meets = |a: usize, b: usize| cx.side_of(a) & cx.side_of(b) != 0;
        shut.extend(
            mine.iter().filter(|&&q| q != c && hard(q) && meets(q, c)).filter_map(|&q| rect_of(q, &centre, &quarter)),
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
            .filter(|&q| q != c && !hard(q) && meets(q, c) && rect_of(q, &centre, &quarter).is_some_and(over))
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
            moved.iter().chain([&c]).map(|&q| reach_of(q, quarter[q], centre, quarter, &pins)).sum()
        };
        if done && reach(&new_centre, &new_quarter) < reach(&centre, &quarter) - 1e-6 {
            for &q in moved.iter().chain([&c]) {
                if let Some(at) = new_centre[q] {
                    stood.insert(q, Some((at, new_quarter[q] % 2 == 1)));
                }
            }
            (centre, quarter) = (new_centre, new_quarter);
            by_its_pad.insert(c);
        }
    }
    face(&centre, &mut quarter, &pins);
    // The parts on the gaps are stood along them once more, now by where the parts they are tied to have come
    // to stand (and not the middles of their zones): each gap's parts spread as before.
    {
        type Strip = (bool, [f64; 4], Vec<(usize, usize, f64, f64)>); // (bridge, part, want along, size)
        let mut strips: BTreeMap<[i64; 4], Strip> = BTreeMap::new();
        for (b, bridge) in bridges.iter().enumerate().filter(|(_, bridge)| bridge.on == Across::Gap) {
            let Some(p) = parts.iter().position(|part| part.name == bridge.part) else { continue };
            let (Some(home), Some(other)) =
                (domain_named(top, &bridge.grounds[0]), domain_named(top, &bridge.grounds[1]))
            else {
                continue;
            };
            let Some((strip, up)) = strip_between(top[home].rect, top[other].rect, board.gap) else { continue };
            let shift = shift_of[p];
            let mates: Vec<Pt> = (0..nets.len())
                .filter(|&n| nets[n].contains(&p))
                .flat_map(|n| nets[n].iter().filter(|&&q| q != p))
                .filter_map(|&q| {
                    centre[q].map(|at| [at[0] + shift_of[q][0] - shift[0], at[1] + shift_of[q][1] - shift[1]])
                })
                .collect();
            let Some(want) = median(&mates) else { continue };
            let key = strip.map(|v| (v * 1000.0).round() as i64);
            let entry = strips.entry(key).or_insert((up, strip, Vec::new()));
            let quarters = gap_quarter.get(&p).copied().unwrap_or(0);
            entry.2.push((b, p, want[usize::from(up)], cx.lying(p, quarters)[usize::from(up)] + BESIDE));
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
    // Then the small parts are gone over, a few times: each is tried at the middle of what its nets join it
    // to (as near that as there is room, within REFINE_MM of it), and stays there if its nets are the shorter
    // for it by the boxes of their parts. Not what was settled first, nor a capacitor by its pad.
    let mut nets_of: Vec<Vec<usize>> = vec![Vec::new(); parts.len()];
    for (n, net) in fine_nets.iter().enumerate() {
        net.iter().for_each(|&p| nets_of[p].push(n));
    }
    let in_stack =
        |q: usize, centre: &[Option<Pt>]| centre[q].map(|at| [at[0] + shift_of[q][0], at[1] + shift_of[q][1]]);
    let boxes = |p: usize, at: Pt, centre: &[Option<Pt>]| -> f64 {
        let mut all = 0.0;
        for &n in &nets_of[p] {
            let (mut low, mut high) = (at, at);
            for there in fine_nets[n].iter().filter(|&&q| q != p).filter_map(|&q| in_stack(q, centre)) {
                low = [low[0].min(there[0]), low[1].min(there[1])];
                high = [high[0].max(there[0]), high[1].max(there[1])];
            }
            all += high[0] - low[0] + high[1] - low[1];
        }
        all
    };
    for _ in 0..REFINES {
        let mut better = false;
        for p in 0..parts.len() {
            let settled_here = !on_gap.contains_key(&p) && stood.get(&p).is_some_and(|at| at.is_some());
            let free = settled_here && !cx.goes_first(p) && !by_its_pad.contains(&p) && !with_capacitors.contains(&p);
            let Some(was) = centre[p].filter(|_| free) else { continue };
            let shift = shift_of[p];
            let mates: Vec<Pt> = nets_of[p]
                .iter()
                .flat_map(|&n| fine_nets[n].iter().filter(|&&q| q != p))
                .filter_map(|&q| in_stack(q, &centre))
                .map(|at| [at[0] - shift[0], at[1] - shift[1]])
                .collect();
            // (a part of two pads with one of them on a net of its own with one pad of another part is there
            // for that pad, a resistor at its driver, a protector at its connector: it wants to be by it, and
            // goes there if its nets are no longer for it)
            let own_pad: Option<Pt> = if parts[p].pads.len() == 2 && !parts[p].capacitor {
                let alone: Vec<(usize, usize)> = parts[p]
                    .pads
                    .iter()
                    .flatten()
                    .filter(|&&net| pads_on[net].len() == 2)
                    .filter_map(|&net| pads_on[net].iter().copied().find(|other| other.0 != p))
                    .collect();
                // (with a net of its own to a pad at each end, it is for the one that drives, if the schematic
                // says which that is)
                let drives =
                    |&(q, j): &(usize, usize)| matches!(parts[q].pins.get(j), Some(Pin::Output) | Some(Pin::PowerOut));
                let chosen = match alone[..] {
                    [one] => Some(one),
                    [a, b] if drives(&a) != drives(&b) => Some(if drives(&a) { a } else { b }),
                    _ => None,
                };
                chosen
                    .and_then(|(q, j)| pad_at(q, j, quarter[q], &centre))
                    .map(|at| [at[0] - shift[0], at[1] - shift[1]])
            } else {
                None
            };
            // Where it may want to be: by the pad it is for, if it is for one; else at the middle of what it is
            // joined to, and (a part joined to few, which is as well anywhere between them) at their mean and
            // by each of them. Room is looked for near each, then further (twice and four times as far); the
            // place that leaves its nets the shortest is the one tried.
            let mut wants: Vec<Pt> = Vec::new();
            match own_pad {
                Some(pad) => wants.push(pad),
                None => {
                    wants.extend(median(&mates));
                    if !mates.is_empty() && mates.len() <= FEW {
                        let n = mates.len() as f64;
                        wants.push([
                            mates.iter().map(|m| m[0]).sum::<f64>() / n,
                            mates.iter().map(|m| m[1]).sum::<f64>() / n,
                        ]);
                        wants.extend(mates.iter().copied());
                    }
                }
            }
            if wants.is_empty() || dist(wants[0], was) < SETTLE {
                continue;
            }
            let domain = domain_of_part[p];
            let ground = top[domain].rect;
            let mut shut: Vec<[f64; 4]> = site.closed(&gap_rects);
            shut.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
            let in_the_way = |q: &&usize| **q != p && !on_gap.contains_key(q) && cx.side_of(**q) & cx.side_of(p) != 0;
            for &q in top[domain].parts.iter().filter(in_the_way) {
                if let Some(at) = centre[q].filter(|_| stood.get(&q).is_some_and(|at| at.is_some())) {
                    shut.push(cx.taken(q, (at, quarter[q] % 2 == 1)));
                }
            }
            // (what is disturbed is not moved onto the ground near what disturbs)
            if victim[p] {
                shut.extend(halos.iter().copied());
            }
            let [w, h] = cx.body(p);
            let lying = if quarter[p] % 2 == 1 { [h, w] } else { [w, h] };
            let stack = |at: Pt| [at[0] + shift[0], at[1] + shift[1]];
            let now = boxes(p, stack(was), &centre);
            let mut tried: Option<(f64, Pt, bool)> = None;
            for want in wants {
                let window = |reach: f64| {
                    [
                        ground[0].max(want[0] - reach),
                        ground[1].max(want[1] - reach),
                        ground[2].min(want[0] + reach),
                        ground[3].min(want[1] + reach),
                    ]
                };
                let room = [1.0, 2.0, 4.0].iter().find_map(|&times| {
                    let within = window(times * REFINE_MM);
                    let has_size = within[2] - within[0] >= SETTLE && within[3] - within[1] >= SETTLE;
                    if has_size {
                        settled(within, &[(lying, want)], &shut)[0]
                    } else {
                        None
                    }
                });
                if let Some((at, turn)) = room {
                    let then = boxes(p, stack(at), &centre);
                    if tried.map_or(true, |best| then < best.0 - 1e-6) {
                        tried = Some((then, at, turn));
                    }
                }
            }
            let Some((then, at, turn)) = tried else { continue };
            let nearer = own_pad.is_some_and(|pad| dist(at, pad) < dist(was, pad) - SETTLE && then <= now + OWN_PAD_MM);
            if then < now - 1e-6 || nearer {
                let turn = turn != (quarter[p] % 2 == 1);
                (centre[p], quarter[p]) = (Some(at), u8::from(turn));
                stood.insert(p, Some((at, turn)));
                better = true;
            }
        }
        if !better {
            break;
        }
    }
    face(&centre, &mut quarter, &pins);
    // (the sheets that repeat, as they came out: a copy is laid out like the others if it was placed whole)
    // How long a net is: the box of its pads, as the boards stack.
    let span_of = |net: usize| -> f64 {
        let mut spots: Vec<Pt> = Vec::new();
        for &q in &reach[net] {
            let on = (0..parts[q].pads.len()).filter(|&i| parts[q].pads[i] == Some(net));
            spots.extend(on.filter_map(|i| pad_at(q, i, quarter[q], &centre)));
        }
        if spots.len() < 2 {
            return 0.0;
        }
        let low = |k: usize| spots.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let high = |k: usize| spots.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        high(0) - low(0) + high(1) - low(1)
    };
    let mut repeats: Vec<Repeat> = unit_groups
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
                        shared_mm += span_of(net);
                        continue;
                    }
                    let spans: Vec<f64> = theirs.iter().map(|&of| span_of(of)).collect();
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
    // What disturbs and stands near what is disturbed, part by part: by how far their bodies are apart, as the
    // boards stack (on one board: another board is another matter).
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
            if far < QUIET_MM {
                close.push((cx.name(a), cx.name(v), round1(far)));
            }
        }
    }
    close.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    let disturbing = [aggressor.iter().filter(|&&is| is).count(), victim.iter().filter(|&&is| is).count()];
    // (the sensitive nets: how many, how long in all, and the longest)
    let mut sensitive: (usize, f64, f64, String) = (0, 0.0, 0.0, String::new());
    for net in (0..all_nets.len()).filter(|&net| all_nets[net].sensitive && cx.is_signal(net)) {
        let far = span_of(net);
        sensitive.0 += 1;
        sensitive.1 += far;
        if far > sensitive.2 {
            (sensitive.2, sensitive.3) = (far, cx.net_name(net));
        }
    }
    let sensitive = (sensitive.0, sensitive.1.round(), round1(sensitive.2), sensitive.3);
    repeats.sort_by(|a, b| (b.sheets.len() * b.parts).cmp(&(a.sheets.len() * a.parts)).then(a.sheets.cmp(&b.sheets)));
    let pads_mm = all_reach(&centre, &quarter);
    // (a capacitor is by its pad within BESIDE_PIN; a large one within its own length and a millimetre: its
    // pad cannot be nearer than its body allows)
    let by_pad = |c: usize| BESIDE_PIN.max(cx.body(c)[0].max(cx.body(c)[1]) + 1.0);
    let of_supplies =
        |c: &usize| parts[*c].capacitor && parts[*c].pads.iter().flatten().any(|net| found.rails.contains_key(net));
    let beside = pins
        .iter()
        .filter(|(c, _)| of_supplies(c))
        .filter(|(&c, &(mine, chip, pin))| {
            match (pad_at(c, mine, quarter[c], &centre), pad_at(chip, pin, quarter[chip], &centre)) {
                (Some(here), Some(there)) => dist(here, there) <= by_pad(c),
                _ => false,
            }
        })
        .count();
    // The loops, as they came out: the box of the pads on the node and on the supplies of the loop.
    let mut loops: Vec<Loop> = switchers
        .iter()
        .map(|(chip, node, by_it, at_pads)| {
            let mut spots: Vec<Pt> = Vec::new();
            let mut of_loop: BTreeSet<usize> = [*node].into_iter().collect();
            // (of the capacitors on one supply, the one nearest the chip closes the loop: the others are behind it)
            let here = in_stack(*chip, &centre);
            let mut nearest: BTreeMap<usize, (f64, usize)> = BTreeMap::new();
            for &m in by_it.iter().filter(|&&m| parts[m].capacitor) {
                let far = match (here, in_stack(m, &centre)) {
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
                spots.extend((0..parts[m].pads.len()).filter_map(|i| pad_at(m, i, quarter[m], &centre)));
            }
            for (i, net) in parts[*chip].pads.iter().enumerate() {
                if net.is_some_and(|net| of_loop.contains(&net)) {
                    spots.extend(pad_at(*chip, i, quarter[*chip], &centre));
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
    // (what stands for one pad, and how much of it is by that pad)
    let served_by = pins
        .iter()
        .filter(|(c, _)| served_parts.contains(c))
        .filter(|(&c, &(mine, head, pin))| {
            match (pad_at(c, mine, quarter[c], &centre), pad_at(head, pin, quarter[head], &centre)) {
                (Some(here), Some(there)) => dist(here, there) <= by_pad(c),
                _ => false,
            }
        })
        .count();
    let served = [pins.keys().filter(|c| served_parts.contains(c)).count(), served_by];
    // The chains: from the pad of a head on a net where it is the only head, through parts of two pads in
    // series (each on to a net with no head and one such part more), to the next head. One doubles back if
    // the way through the middles of its parts is more than BACK times as long as straight and LOCAL_MM more.
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
        pad_at(q, i, quarter[q], &centre)
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
            let room: f64 =
                through.iter().map(|&m| 2.0 * by_pad(m)).sum::<f64>() - (through.len() - 1) as f64 * BESIDE_PIN;
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
    let bypass = [pins.keys().filter(|c| of_supplies(c)).count(), beside];
    let mut capacitors: Vec<(String, String, f64, usize)> = pins
        .iter()
        .filter_map(|(&c, &(mine, chip, pin))| {
            let (here, there) = (pad_at(c, mine, quarter[c], &centre)?, pad_at(chip, pin, quarter[chip], &centre)?);
            let shared = at_pad.get(&(chip, pin)).copied().unwrap_or(1);
            Some((cx.name(c), cx.name(chip), (dist(here, there) * 10.0).round() / 10.0, shared))
        })
        .collect();
    capacitors.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
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
    let part_shift: BTreeMap<usize, Pt> =
        fine_leaves.iter().flat_map(|(domain, _, own)| own.iter().map(|&p| (p, shifts[to[*domain]]))).collect();
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
            let home = boards.iter().position(|b| parts[p].at[0] >= b[0] && parts[p].at[0] <= b[2]);
            let shift = home.map_or(part_shift[&p], |b| shifts[b]);
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
    Placed {
        blocks,
        alike,
        disturbing,
        close,
        sensitive,
        repeats,
        served,
        chains,
        back,
        loops,
        places,
        parts_now_mm,
        parts_then_mm,
        against,
        pads_first_mm,
        pads_mm,
        bypass,
        capacitors,
    }
}
