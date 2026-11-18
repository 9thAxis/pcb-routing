//! Ground for each part within its zone, and the bodies settled on it.

use super::blocks::Block;
use super::Site;
use crate::floorplan::constants::{AS_NEAR, BLOCK_NEAR, MAIN, PACKED, QUIET_MM, ROW_WORTH};
use crate::floorplan::ground::{arrange, taken_by, Ground, Work};
use crate::floorplan::known::Known;
use crate::floorplan::settle::{middle, quartered, settled, settled_shape};
use crate::floorplan::types::Rows;
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// Ground for every part within its zone, pulled and cut like the zones (`fine_nets`: the nets of the
/// parts). The zones are gone over twice: the second time each part knows where those of the other zones
/// came to stand.
pub(super) fn grounds(site: &Site, fine_nets: &[Vec<usize>]) -> BTreeMap<usize, [f64; 4]> {
    let (cx, fine_leaves, on_gap) = (site.cx, &site.leaves, &site.on_gap);
    let (board, parts, mate_of) = (cx.board, cx.parts, &cx.mate_of);
    let (boards, shifts, to) = (&site.on.boards, &site.on.shifts, &site.on.to);
    let mut stand: Vec<Option<Pt>> = vec![None; parts.len()];
    for (domain, rect, own) in fine_leaves {
        let (mid, shift) = (middle(*rect), shifts[to[*domain]]);
        own.iter().for_each(|&p| stand[p] = Some([mid[0] + shift[0], mid[1] + shift[1]]));
    }
    for (domain, _, own) in fine_leaves {
        let shift = shifts[to[*domain]];
        for (&p, at) in own.iter().filter_map(|p| on_gap.get(p).map(|at| (p, at))) {
            stand[p] = Some([at[0] + shift[0], at[1] + shift[1]]);
        }
    }
    for p in (0..parts.len()).filter(|&p| cx.held(p)) {
        let shift = site.shift_of[p];
        stand[p] = Some([parts[p].at[0] + shift[0], parts[p].at[1] + shift[1]]);
    }
    let (none, no_sides, no_asks, no_kin) = (BTreeSet::new(), BTreeMap::new(), BTreeMap::new(), Vec::new());
    let no_rails: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut grounds: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
    for _ in 0..2 {
        for (domain, rect, own) in fine_leaves {
            let mut each: Vec<Work> = own
                .iter()
                .filter(|p| !on_gap.contains_key(p))
                .map(|&p| Work { name: cx.name(p), parts: vec![p], inside: Vec::new(), rect: [0.0; 4], more: 0.0 })
                .collect();
            let mut on = Ground {
                board,
                nets: fine_nets,
                rails: &no_rails,
                at: std::mem::take(&mut stand),
                roads: Vec::new(),
                region: boards[to[*domain]],
                shift: shifts[to[*domain]],
                turned: &none,
                crossed: &none,
                sided: &no_sides,
                asked: &no_asks,
                mates: mate_of,
                fine: true,
                kin: &no_kin,
            };
            let rect = &packed(site, fine_nets, &on.at, (*domain, *rect, own));
            arrange(&mut each, *rect, 0, false, &mut on);
            stand = std::mem::take(&mut on.at);
            grounds.extend(each.iter().map(|work| (work.parts[0], work.rect)));
        }
    }
    grounds
}

/// The ground the parts of one of the smallest zones are laid out on: not all of the zone's, which has its
/// share of the whole board however empty that is, but as much as they fill to PACKED, of the zone's own
/// shape, as near as it can lie within the zone to what they are joined to outside it (`stand`: where every
/// part is taken to stand so far).
fn packed(site: &Site, fine_nets: &[Vec<usize>], stand: &[Option<Pt>], leaf: (usize, [f64; 4], &[usize])) -> [f64; 4] {
    let (cx, on_gap) = (site.cx, &site.on_gap);
    let (domain, rect, own) = leaf;
    let shift = site.on.shifts[site.on.to[domain]];
    let inside: Vec<usize> = own.iter().copied().filter(|p| !on_gap.contains_key(p)).collect();
    let (across, down) = (rect[2] - rect[0], rect[3] - rect[1]);
    let share = taken_by(cx.board, &inside) / PACKED / (across * down).max(1e-9);
    if share >= 1.0 || inside.is_empty() {
        return rect;
    }
    // (no narrower than its largest part)
    let most = inside.iter().map(|&p| cx.body(p)[0].max(cx.body(p)[1])).fold(0.0, f64::max);
    let (wide, high) = ((across * share.sqrt()).max(most).min(across), (down * share.sqrt()).max(most).min(down));
    let own: BTreeSet<usize> = inside.iter().copied().collect();
    let (mut sum, mut all) = ([0.0, 0.0], 0.0);
    for net in fine_nets.iter().filter(|net| net.len() > 1 && net.iter().any(|p| own.contains(p))) {
        let each = 1.0 / (net.len() - 1) as f64;
        for at in net.iter().filter(|p| !own.contains(p)).filter_map(|&p| stand[p]) {
            sum = [sum[0] + each * (at[0] - shift[0]), sum[1] + each * (at[1] - shift[1])];
            all += each;
        }
    }
    let mid = middle(rect);
    let pull = if all > 0.0 { [sum[0] / all, sum[1] / all] } else { mid };
    let x = pull[0].clamp(rect[0] + wide / 2.0, rect[2] - wide / 2.0);
    let y = pull[1].clamp(rect[1] + high / 2.0, rect[3] - high / 2.0);
    [x - wide / 2.0, y - high / 2.0, x + wide / 2.0, y + high / 2.0]
}

/// Where the bodies came to stand.
pub(super) struct Stood {
    /// Where each came to stand: its middle and whether it is turned; None where there was no room. A domain's
    /// bodies are settled together, off its roads and what is shut: a large part may reach over the ground of
    /// its zone, which was cut by what its parts take and not by their shapes.
    pub(super) stood: BTreeMap<usize, Option<(Pt, bool)>>,
    /// How many quarters each part of a block that was placed whole is turned.
    pub(super) block_quarter: BTreeMap<usize, u8>,
    /// The ground near each part that disturbs, as it came to stand.
    pub(super) halos: Vec<[f64; 4]>,
    /// For a body of copies, by its first part: the copies that came to stand side by side.
    pub(super) in_rows: BTreeMap<usize, usize>,
}

/// What a block is placed by: the ground of its domain, where it is wanted, and where the parts it is joined
/// to are taken to come.
struct Room<'a> {
    cx: &'a Known<'a>,
    on_gap: &'a BTreeMap<usize, Pt>,
    grounds: &'a BTreeMap<usize, [f64; 4]>,
    in_block: &'a BTreeMap<usize, usize>,
    /// The block's first part, the ground of its domain, and where that part is wanted.
    p: usize,
    rect: [f64; 4],
    want: Pt,
}

impl Room<'_> {
    /// Where a block would come to stand, of the ways it may be turned: placed by the bodies of its parts,
    /// not by a box round them (blocks then lie into each other's corners); of the ways that come about as
    /// near where it is wanted as the nearest, the one that leaves its pads nearest what they join outside
    /// it, with how far that is. It is looked for near where it is wanted; and `anywhere`, if there is no
    /// room near.
    fn best_for(&self, block: &Block, anywhere: bool, shut: &[[f64; 4]]) -> Option<(f64, u8, Pt)> {
        let (cx, on_gap, grounds, in_block) = (self.cx, self.on_gap, self.grounds, self.in_block);
        let (parts, reach) = (cx.parts, &cx.reach);
        let (p, want) = (self.p, self.want);
        // (where a part that is not yet settled is taken to come: the middle of the ground that came to it)
        let wanted = |q: usize| on_gap.get(&q).copied().or_else(|| grounds.get(&q).map(|&r| middle(r)));
        let pieces = |way: u8| -> Vec<[f64; 4]> {
            block.at.iter().map(|&(m, from, q)| cx.taken(m, (quartered(way, from), (q + way) % 2 == 1))).collect()
        };
        let ways: Vec<Vec<[f64; 4]>> = (0..4).map(pieces).collect();
        let outside = |way: u8, here: Pt| -> f64 {
            let mut all = 0.0;
            for &(m, from, q) in &block.at {
                for (i, net) in parts[m].pads.iter().enumerate() {
                    let (Some(net), Some(&offset)) = (net.filter(|&net| cx.is_signal(net)), parts[m].offsets.get(i))
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
        let mut tried = settled_shape(self.rect, &ways, want, shut, BLOCK_NEAR);
        if anywhere && tried.iter().all(|at| at.is_none()) {
            tried = settled_shape(self.rect, &ways, want, shut, f64::INFINITY);
        }
        let nearest = tried.iter().flatten().map(|&at| dist(at, want)).fold(f64::INFINITY, f64::min);
        tried
            .iter()
            .enumerate()
            .filter_map(|(way, at)| at.map(|at| (way as u8, at)))
            .filter(|&(_, at)| dist(at, want) <= nearest + AS_NEAR)
            .map(|(way, at)| (outside(way, at), way, at))
            .fold(None::<(f64, u8, Pt)>, |best, now| if best.is_some_and(|b| b.0 <= now.0) { best } else { Some(now) })
    }
}

/// The parts with holes, the large ones, what disturbs and the blocks are settled first, over the whole
/// ground of their domain: they are the hardest to find room for.
///
/// No track runs under a part with holes: it stands off the gaps and the roads that part much of the board
/// (a lesser road, a track or two, goes round it); the others may stand over a road, whose tracks run on
/// the layers beneath.
pub(super) fn settled_first(
    site: &Site,
    grounds: &BTreeMap<usize, [f64; 4]>,
    gap_rects: &BTreeMap<usize, [f64; 4]>,
    laid_blocks: &BTreeMap<usize, Block>,
    alternatives: &BTreeMap<usize, Vec<Vec<(usize, Block)>>>,
    in_block: &BTreeMap<usize, usize>,
) -> Stood {
    let (cx, top, roads, on_gap) = (site.cx, site.top, site.roads, &site.on_gap);
    let (board, parts, aggressor, victim) = (cx.board, cx.parts, &cx.aggressor, &cx.victim);
    // (where each came to stand: its middle and whether it is turned; None where there was no room. A domain's
    // bodies are settled together, off its roads and what is shut: a large part may reach over the ground of
    // its zone, which was cut by what its parts take and not by their shapes)
    let mut stood: BTreeMap<usize, Option<(Pt, bool)>> = BTreeMap::new();
    let mut in_rows: BTreeMap<usize, usize> = BTreeMap::new(); // a body of copies, by its first part -> the copies side by side
    let holed = |p: usize| laid_blocks.get(&p).map_or(parts[p].through, |block| block.through);
    let mut block_quarter: BTreeMap<usize, u8> = BTreeMap::new();
    // What disturbs is placed before all else, and what is disturbed then keeps QUIET_MM from it wherever
    // there is room for it so: the ground that near each part that disturbs, as it comes to stand.
    let mut halos: Vec<[f64; 4]> = Vec::new();
    let loud =
        |p: usize| laid_blocks.get(&p).map_or(aggressor[p], |block| block.at.iter().any(|placed| aggressor[placed.0]));
    let round_it = |r: [f64; 4]| [r[0] - QUIET_MM, r[1] - QUIET_MM, r[2] + QUIET_MM, r[3] + QUIET_MM];
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
            let mut base: Vec<[f64; 4]> = site.closed(gap_rects);
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
                let mut over_roads: Vec<[f64; 4]> = site.closed(gap_rects);
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
                let room = Room { cx, on_gap, grounds, in_block, p, rect: work.rect, want };
                for (n, option) in options.iter().enumerate() {
                    let mut placed_all = true;
                    for &(_, block) in option {
                        let anywhere = n == last || board.rows == Rows::Always;
                        let kept: Vec<[f64; 4]> = shut.iter().chain(&halos).copied().collect();
                        let hushed = block.at.iter().any(|placed| victim[placed.0]);
                        let away = if hushed { room.best_for(block, false, &kept) } else { None };
                        let mut chosen = away.or_else(|| room.best_for(block, anywhere, &shut)).or_else(|| {
                            if n == last {
                                room.best_for(block, true, &over_roads)
                            } else {
                                None
                            }
                        });
                        // (copies side by side are worth it if their pads are not much further from what
                        // they join than with each copy where it would stand by itself)
                        if let (true, Some((together, _, _))) = (n < last && board.rows == Rows::Cheap, chosen) {
                            let alone: f64 = options[last]
                                .iter()
                                .map(|&(_, one)| room.best_for(one, false, &shut).map_or(f64::INFINITY, |c| c.0))
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
                                        // (one that disturbs keeps what is disturbed off as any other does)
                                        if aggressor[m] {
                                            halos.push(round_it(cx.taken(m, at)));
                                        }
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
    Stood { stood, block_quarter, halos, in_rows }
}

/// Then the others, zone by zone, each on the ground of its own; what found no room there, on what is left
/// of the ground of its domain; and what found none anywhere stays where it stands today, if it may.
pub(super) fn settled_rest(
    site: &Site,
    grounds: &BTreeMap<usize, [f64; 4]>,
    gap_rects: &BTreeMap<usize, [f64; 4]>,
    in_block: &BTreeMap<usize, usize>,
    on: &mut Stood,
) {
    let (cx, top, roads, fine_leaves) = (site.cx, site.top, site.roads, &site.leaves);
    let (board, parts, victim) = (cx.board, cx.parts, &cx.victim);
    let (stood, halos) = (&mut on.stood, &on.halos);
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
                grounds.contains_key(&p)
                    && !cx.goes_first(p)
                    && !in_block.contains_key(&p)
                    && cx.side_of(p) == side
                    && !stood.get(&p).is_some_and(|at| at.is_some())
            })
            .collect();
        let mut shut: Vec<[f64; 4]> = site.closed(gap_rects);
        shut.extend(settled_first.iter().filter(|first| first.1 & side != 0).map(|first| first.0));
        // (what is disturbed first, off the ground near what disturbs; what of it finds no room so, with the rest)
        let hushed: Vec<usize> = mine.iter().copied().filter(|&p| victim[p]).collect();
        let kept: Vec<[f64; 4]> = shut.iter().chain(halos).copied().collect();
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
            let mut shut: Vec<[f64; 4]> = site.closed(gap_rects);
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
                    let mut open: Vec<[f64; 4]> = site.closed(gap_rects);
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
}

/// Where the middle of each part is (as its board is drawn), and how many quarters it is turned.
pub(super) fn lies(site: &Site, grounds: &BTreeMap<usize, [f64; 4]>, on: &Stood) -> (Vec<Option<Pt>>, Vec<u8>) {
    let (parts, fine_leaves) = (site.cx.parts, &site.leaves);
    let (on_gap, gap_quarter) = (&site.on_gap, &site.gap_quarter);
    let (stood, block_quarter) = (&on.stood, &on.block_quarter);
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
    // (and what is held where it stands is there, as it lies)
    for p in (0..parts.len()).filter(|&p| site.cx.held(p)) {
        (centre[p], quarter[p]) = (Some(parts[p].at), 0);
    }
    (centre, quarter)
}
