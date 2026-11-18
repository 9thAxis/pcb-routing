//! The floorplan of a board, from its power down to a place for every part.

use super::constants::*;
use super::ground::{arrange, Ground, Work};
use super::known::{standing, Known};
use super::power::power_told;
use super::settle::{
    area, farads, median, middle, off_rect, overlap, quartered, round1, round2, settled, settled_shape, spread,
    strip_between,
};
use super::ties::{in_line, tied};
use super::types::*;
use super::ways::travelled;
use super::zones::zone_tree;
use crate::error::Error;
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// The floorplan of a board (see the module). An error if a pad names a net that is not there, or the board
/// has no size.
pub fn floorplan(board: &Board) -> Result<Floorplan, Error> {
    let (stays, held) = standing(board);
    let board = held.as_ref().unwrap_or(board);
    let cx = &Known::of(board, stays)?;
    let power = power_told(cx);
    let (top, apart) = zone_tree(cx);
    let (parts, all_nets, found, reach, nets, signal) =
        (cx.parts, cx.all_nets, &cx.found, &cx.reach, &cx.nets, &cx.signal);
    let (mate_of, aggressor, victim, power_layers, domain_at) =
        (&cx.mate_of, &cx.aggressor, &cx.victim, &cx.power_layers, &cx.domain_at);

    // Ground for each zone, from the whole board down.
    let at: Vec<Option<Pt>> = parts.iter().map(|part| (part.fixed && part.placed).then_some(part.at)).collect();
    // The supplies on the plane layers (every plane after the first, the ground's): under each of the smallest
    // zones, as many supplies as there are such layers, those with the most parts there, of the supplies of
    // its ground that reach several zones (one that stays within a zone is laid there, on the layers of its
    // parts). A part on two grounds stands on a gap, beside the pours of both.
    let mut leaf_parts: Vec<(usize, &Vec<usize>)> = Vec::new();
    fn smallest_of<'w>(work: &'w Work, domain: usize, out: &mut Vec<(usize, &'w Vec<usize>)>) {
        if work.inside.is_empty() {
            out.push((domain, &work.parts));
        }
        work.inside.iter().for_each(|inner| smallest_of(inner, domain, out));
    }
    top.iter().enumerate().for_each(|(domain, work)| smallest_of(work, domain, &mut leaf_parts));
    let leaf_at: BTreeMap<usize, usize> =
        leaf_parts.iter().enumerate().flat_map(|(l, (_, own))| own.iter().map(move |&p| (p, l))).collect();
    let rail_nets: Vec<usize> = found.rails.keys().copied().collect();
    let in_leaf: Vec<BTreeMap<usize, usize>> = rail_nets
        .iter()
        .map(|&net| {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            reach[net].iter().filter_map(|p| leaf_at.get(p)).for_each(|&l| *count.entry(l).or_insert(0) += 1);
            count
        })
        .collect();
    let rail_domain: Vec<Option<usize>> =
        rail_nets.iter().map(|net| found.rails[net].0.and_then(|g| domain_at.get(&g).copied())).collect();
    let mut on_plane: Vec<Vec<usize>> = vec![Vec::new(); leaf_parts.len()];
    for (l, (domain, _)) in leaf_parts.iter().enumerate() {
        let mut asking: Vec<(usize, usize, usize)> = (0..rail_nets.len())
            .filter(|&r| rail_domain[r] == Some(*domain) && in_leaf[r].len() > 1)
            .filter_map(|r| in_leaf[r].get(&l).map(|&count| (count, in_leaf[r].len(), r)))
            .collect();
        asking.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        on_plane[l] = asking.into_iter().take(power_layers.len()).map(|ask| ask.2).collect();
    }
    let rail_parts: Vec<(Vec<usize>, Vec<usize>)> = (0..rail_nets.len())
        .map(|r| {
            let poured = on_plane.iter().any(|rails| rails.contains(&r));
            let fed = |p: &usize| {
                poured
                    && (found.stands[*p].len() > 1
                        || leaf_at.get(p).is_some_and(|&l| on_plane[l].contains(&r))
                        || in_leaf[r].len() < 2)
            };
            reach[rail_nets[r]].iter().copied().partition(fed)
        })
        .collect();
    let kin: Vec<Vec<usize>> = (0..rail_nets.len())
        .map(|r| {
            let under = (0..leaf_parts.len()).filter(|&l| on_plane[l].contains(&r));
            under.filter_map(|l| leaf_parts[l].1.first().copied()).collect()
        })
        .collect();
    // (the pieces each supply's pour is in, given the ground of every smallest zone: zones that are neighbours
    // are one piece, and so are zones joined through ground that has a plane layer to spare)
    let pieces = |rects: &[[f64; 4]]| -> Vec<usize> {
        let count = leaf_parts.len();
        let mut beside: Vec<Vec<usize>> = vec![Vec::new(); count];
        for a in 0..count {
            for b in a + 1..count {
                if leaf_parts[a].0 == leaf_parts[b].0 && strip_between(rects[a], rects[b], board.gap).is_some() {
                    beside[a].push(b);
                    beside[b].push(a);
                }
            }
        }
        (0..rail_nets.len())
            .map(|r| {
                let open = |l: usize| on_plane[l].contains(&r) || on_plane[l].len() < power_layers.len();
                let mut seen = vec![false; count];
                let mut found = 0;
                for from in (0..count).filter(|&l| on_plane[l].contains(&r)) {
                    if seen[from] {
                        continue;
                    }
                    found += 1;
                    let mut edge = vec![from];
                    seen[from] = true;
                    while let Some(l) = edge.pop() {
                        for &next in
                            beside[l].iter().filter(|&&next| open(next) && rail_domain[r] == Some(leaf_parts[next].0))
                        {
                            if !seen[next] {
                                seen[next] = true;
                                edge.push(next);
                            }
                        }
                    }
                }
                found
            })
            .collect()
    };
    // Where the box holds several boards, the domains are parted between them first: in the order their ties
    // give, each board with as many as its room is of all the room (a domain is on one board).
    let mut with_shift: Vec<([f64; 4], Pt)> = if board.regions.is_empty() {
        vec![(board.outline, [0.0, 0.0])]
    } else {
        (0..board.regions.len())
            .map(|k| (board.regions[k], board.shifts.get(k).copied().unwrap_or([0.0, 0.0])))
            .collect()
    };
    with_shift.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]).then(a.0[1].total_cmp(&b.0[1])));
    let boards: Vec<[f64; 4]> = with_shift.iter().map(|board| board.0).collect();
    let shifts: Vec<Pt> = with_shift.iter().map(|board| board.1).collect();
    let mut to: Vec<usize> = vec![0; top.len()];
    if boards.len() > 1 && top.len() > 1 {
        let mut of_top: BTreeMap<usize, usize> = BTreeMap::new();
        for (z, work) in top.iter().enumerate() {
            of_top.extend(work.parts.iter().map(|&p| (p, z)));
        }
        let line = in_line(top.len(), &tied(nets, &|p| of_top.get(&p).copied()));
        let mut order: Vec<usize> = (0..top.len()).collect();
        order.sort_by(|&a, &b| line[a].total_cmp(&line[b]).then(a.cmp(&b)));
        let all_room: f64 = boards.iter().map(|&r| cx.room(r)).sum();
        let all_need: f64 = top.iter().map(|work| cx.need(work)).sum::<f64>().max(1.0);
        // (the order, or the order turned round: whichever leaves each board nearer what its room is of all)
        let parted = |order: &[usize]| -> (f64, Vec<usize>) {
            let (mut to, mut at_board, mut taken, mut off) = (vec![0usize; top.len()], 0usize, 0.0f64, 0.0f64);
            for (k, &z) in order.iter().enumerate() {
                let share = cx.room(boards[at_board]) / all_room * all_need;
                let left = order.len() - k;
                let must_move = left <= boards.len() - 1 - at_board;
                let over = taken + cx.need(&top[z]) - share;
                if at_board + 1 < boards.len() && taken > 0.0 && (must_move || over > share - taken) {
                    off += (taken - share).abs();
                    at_board += 1;
                    taken = 0.0;
                }
                to[z] = at_board;
                taken += cx.need(&top[z]);
            }
            off += (taken - cx.room(boards[at_board]) / all_room * all_need).abs();
            (off, to)
        };
        let turned: Vec<usize> = order.iter().rev().copied().collect();
        let (straight, back) = (parted(&order), parted(&turned));
        to = if straight.0 <= back.0 { straight.1 } else { back.1 };
        // What goes from one board to another goes through a connector between them: where the domains are
        // few enough to try every way, they are parted so that as little as may be is tied across boards,
        // each board holding no more than BOARD_FULL of its room; of ways as good, the most even.
        let ways = (boards.len() as f64).powi(top.len() as i32);
        if ways <= 20_000.0 {
            let mut between = tied(nets, &|p| of_top.get(&p).copied());
            // (a part on two grounds stands on the gap between its two domains: they are on one board)
            for p in (0..parts.len()).filter(|&p| found.stands[p].len() > 1) {
                let on: Vec<usize> = top
                    .iter()
                    .enumerate()
                    .filter(|(_, work)| found.stands[p].iter().any(|&g| all_nets[g].name == work.name))
                    .map(|(z, _)| z)
                    .collect();
                for (i, &a) in on.iter().enumerate() {
                    for &b in &on[i + 1..] {
                        *between.entry((a.min(b), a.max(b))).or_insert(0.0) += ANCHOR;
                    }
                }
            }
            let mut best: Option<(f64, f64, Vec<usize>)> = None;
            // (no board fuller than BOARD_FULL where that can be; else than BOARD_MOST; else as it may)
            for most in [BOARD_FULL, BOARD_MOST, f64::INFINITY] {
                if best.is_some() {
                    break;
                }
                for code in 0..ways as usize {
                    let (mut way, mut rest) = (Vec::with_capacity(top.len()), code);
                    for _ in 0..top.len() {
                        way.push(rest % boards.len());
                        rest /= boards.len();
                    }
                    let fill: Vec<f64> = (0..boards.len())
                        .map(|b| {
                            (0..top.len()).filter(|&z| way[z] == b).map(|z| cx.need(&top[z])).sum::<f64>()
                                / cx.room(boards[b])
                        })
                        .collect();
                    if fill.iter().any(|&f| f > most || f == 0.0) {
                        continue;
                    }
                    let across: f64 = between.iter().filter(|(&(a, b), _)| way[a] != way[b]).map(|(_, w)| *w).sum();
                    let uneven =
                        fill.iter().fold(0.0f64, |m, &f| m.max(f)) - fill.iter().fold(f64::INFINITY, |m, &f| m.min(f));
                    if best.as_ref().map_or(true, |b| (across, uneven) < (b.0, b.1)) {
                        best = Some((across, uneven, way));
                    }
                }
            }
            if let Some((_, _, way)) = best {
                to = way;
            }
        }
    }
    let held: Vec<Vec<String>> = (0..boards.len())
        .map(|b| (0..top.len()).filter(|&z| to[z] == b).map(|z| top[z].name.clone()).collect())
        .collect();
    // The parts that stand on two grounds: each with the domain it is counted with, the others it stands on
    // (the one most of its pads are on first), and the parts its signal nets tie it to.
    let mut spans: Vec<(usize, usize, Vec<usize>, Vec<usize>)> = Vec::new();
    for p in (0..parts.len()).filter(|&p| found.stands[p].len() > 1) {
        let Some(home) = found.home[p].and_then(|g| domain_at.get(&g).copied()) else { continue };
        let pads_on = |g: usize| {
            let on = |net: &&usize| **net == g || found.rails.get(net).is_some_and(|rail| rail.0 == Some(g));
            parts[p].pads.iter().flatten().filter(on).count()
        };
        let mut others: Vec<(usize, usize)> = found.stands[p]
            .iter()
            .filter_map(|&g| domain_at.get(&g).map(|&d| (pads_on(g), d)))
            .filter(|&(_, d)| d != home)
            .collect();
        others.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mates: Vec<usize> = nets
            .iter()
            .filter(|net| net.contains(&p))
            .flat_map(|net| net.iter().copied())
            .filter(|&q| q != p)
            .collect();
        if !others.is_empty() {
            spans.push((p, home, others.into_iter().map(|other| other.1).collect(), mates));
        }
    }
    // One laying out: ground for every zone, the roads, and a way along them for every net between zones.
    type Laid = (Vec<Work>, Vec<Road>, Travel, Vec<(String, f64, Vec<String>, f64)>, Vec<Bridge>);
    type Sided = BTreeMap<(String, String), bool>;
    let lay = |turned: &BTreeSet<String>,
               crossed: &BTreeSet<String>,
               sided: &Sided,
               asked: &BTreeMap<String, usize>|
     -> Laid {
        let mut on = Ground {
            board,
            nets,
            rails: &rail_parts,
            at: at.clone(),
            roads: Vec::new(),
            region: boards[0],
            shift: shifts[0],
            turned,
            crossed,
            sided,
            asked,
            mates: mate_of,
            fine: false,
            kin: &kin,
        };
        let mut lots: Vec<Vec<Work>> = (0..boards.len()).map(|_| Vec::new()).collect();
        let mut places: Vec<(usize, usize)> = Vec::new();
        for (z, work) in top.iter().enumerate() {
            places.push((to[z], lots[to[z]].len()));
            lots[to[z]].push(work.clone());
        }
        // (twice over: the second time each board knows where the others' zones came to stand)
        for round in 0..2 {
            for (b, lot) in lots.iter_mut().enumerate() {
                (on.region, on.shift) = (boards[b], shifts[b]);
                if round == 1 {
                    on.roads.retain(|road| overlap(road.rect, boards[b]) <= 0.0);
                }
                arrange(lot, boards[b], 0, true, &mut on);
            }
            if boards.len() < 2 {
                break;
            }
        }
        let mut back: Vec<Vec<Option<Work>>> =
            lots.into_iter().map(|lot| lot.into_iter().map(Some).collect()).collect();
        let laid: Vec<Work> = places.into_iter().filter_map(|(b, k)| back[b][k].take()).collect();
        let mut roads = std::mem::take(&mut on.roads);
        let mut leaves: Vec<(usize, [f64; 4], &Vec<usize>)> = Vec::new();
        fn smallest<'w>(work: &'w Work, domain: usize, leaves: &mut Vec<(usize, [f64; 4], &'w Vec<usize>)>) {
            if work.inside.is_empty() {
                leaves.push((domain, work.rect, &work.parts));
            }
            work.inside.iter().for_each(|inner| smallest(inner, domain, leaves));
        }
        for (domain, work) in laid.iter().enumerate() {
            smallest(work, domain, &mut leaves);
        }
        let mut travel = travelled(&mut roads, &leaves, nets);
        let rects: Vec<[f64; 4]> = leaves.iter().map(|leaf| leaf.1).collect();
        travel.split = pieces(&rects).iter().map(|&n| n.saturating_sub(1)).sum();
        // (what disturbs is kept from what is disturbed: by the grounds of their zones, on one board)
        for (a, &(domain, loud, own)) in leaves.iter().enumerate() {
            if !own.iter().any(|&p| aggressor[p]) {
                continue;
            }
            for (v, &(other, hushed, theirs)) in leaves.iter().enumerate() {
                if a != v && to[domain] == to[other] && theirs.iter().any(|&p| victim[p]) {
                    let apart = [0, 1].map(|k| (loud[k].max(hushed[k]) - loud[k + 2].min(hushed[k + 2])).max(0.0));
                    travel.noise += (QUIET_MM - dist([0.0, 0.0], apart)).max(0.0);
                }
            }
        }
        travel.noise = travel.noise.round();
        // (how far what goes from domain to domain reaches, and how far what is to be near a point is from it:
        // by the middles of the smallest zones, as the boards stack)
        let mut spot_of: BTreeMap<usize, Pt> = BTreeMap::new();
        for &(domain, rect, own) in &leaves {
            let (mid, shift) = (middle(rect), shifts[to[domain]]);
            spot_of.extend(own.iter().map(|&p| (p, [mid[0] + shift[0], mid[1] + shift[1]])));
        }
        let domain_of: BTreeMap<usize, usize> =
            leaves.iter().flat_map(|&(domain, _, own)| own.iter().map(move |&p| (p, domain))).collect();
        // What stands on two grounds is given its place: on the gap between its two domains where they are
        // neighbours, along it as near the middle of what it is tied to as the others on that gap allow;
        // else at the place of its own domain nearest the other, as the boards stack.
        // (twice over: the second time each knows where the others came to stand)
        let mut bridges: Vec<Bridge> = Vec::new();
        type Strip = (bool, [f64; 4], Vec<(usize, f64, f64)>);
        for _ in 0..2 {
            bridges.clear();
            travel.cramped = 0.0;
            let mut strips: BTreeMap<[i64; 4], Strip> = BTreeMap::new();
            for (p, home, others, mates) in &spans {
                let (p, home) = (*p, *home);
                let (here, mine) = (shifts[to[home]], laid[home].rect);
                let mut tied_to: Vec<Pt> =
                    mates.iter().filter_map(|q| spot_of.get(q)).map(|s| [s[0] - here[0], s[1] - here[1]]).collect();
                // (what it mates with counts for as much as all its nets together)
                if let Some(s) = mate_of[p].and_then(|other| spot_of.get(&other)) {
                    tied_to.extend(vec![[s[0] - here[0], s[1] - here[1]]; tied_to.len().max(1)]);
                }
                let want = median(&tied_to).unwrap_or(middle(mine));
                let beside = others.iter().find_map(|&o| {
                    let strip = if to[o] == to[home] { strip_between(mine, laid[o].rect, board.gap) } else { None };
                    strip.map(|strip| (o, strip))
                });
                let grounds = |o: usize| [top[home].name.clone(), top[o].name.clone()];
                let (on, at, reach, other) = match beside {
                    Some((o, (strip, up))) => {
                        let key = strip.map(|v| (v * 1000.0).round() as i64);
                        let entry = strips.entry(key).or_insert((up, strip, Vec::new()));
                        entry.2.push((bridges.len(), want[usize::from(up)], parts[p].area.sqrt() + BESIDE));
                        (Across::Gap, middle(strip), 0.0, o)
                    }
                    None => {
                        let (o, into) =
                            (others[0], |p: Pt, r: [f64; 4]| [p[0].clamp(r[0], r[2]), p[1].clamp(r[1], r[3])]);
                        let (there, r) = (shifts[to[o]], laid[o].rect);
                        let d = [there[0] - here[0], there[1] - here[1]];
                        let theirs = [r[0] + d[0], r[1] + d[1], r[2] + d[0], r[3] + d[1]];
                        let at = into(into(want, theirs), mine);
                        // (two grounds corner to corner across the gap are neighbours enough: it stands at the corner)
                        let (far, same) = (dist(at, into(at, theirs)), to[o] == to[home]);
                        if same && far <= CORNER * board.gap {
                            let to = into(at, theirs);
                            (Across::Gap, [(at[0] + to[0]) / 2.0, (at[1] + to[1]) / 2.0], 0.0, o)
                        } else {
                            (if same { Across::Apart } else { Across::Link }, at, far, o)
                        }
                    }
                };
                bridges.push(Bridge { part: parts[p].name.clone(), grounds: grounds(other), on, at, reach, off: None });
            }
            for (up, strip, on_it) in strips.into_values() {
                let along = usize::from(up);
                let (want, size): (Vec<f64>, Vec<f64>) = on_it.iter().map(|&(_, want, size)| (want, size)).unzip();
                travel.cramped += (size.iter().sum::<f64>() - (strip[along + 2] - strip[along])).max(0.0);
                for (&(b, _, _), place) in on_it.iter().zip(spread(&want, &size, strip[along], strip[along + 2])) {
                    bridges[b].at[along] = place;
                }
            }
            for (bridge, (p, home, _, _)) in bridges.iter().zip(&spans) {
                let here = shifts[to[*home]];
                spot_of.insert(*p, [bridge.at[0] + here[0], bridge.at[1] + here[1]]);
            }
        }
        travel.cramped = (travel.cramped * 10.0).round() / 10.0;
        travel.apart = bridges.iter().filter(|bridge| bridge.on == Across::Apart).count();
        for (p, other) in mate_of.iter().enumerate().filter_map(|(p, other)| other.filter(|&o| o > p).map(|o| (p, o))) {
            if let (Some(&a), Some(&b)) = (spot_of.get(&p), spot_of.get(&other)) {
                travel.mate_mm += dist(a, b);
            }
        }
        travel.mate_mm = travel.mate_mm.round();
        for (bridge, (p, _, _, _)) in bridges.iter_mut().zip(&spans) {
            bridge.off = parts[*p].placed.then(|| (dist(parts[*p].at, bridge.at) * 10.0).round() / 10.0);
            bridge.at = bridge.at.map(|v| (v * 100.0).round() / 100.0);
            bridge.reach = (bridge.reach * 10.0).round() / 10.0;
        }
        for net in nets.iter() {
            let domains: BTreeSet<usize> = net.iter().filter_map(|p| domain_of.get(p).copied()).collect();
            if domains.len() > 1 {
                let spots: Vec<Pt> = net.iter().filter_map(|p| spot_of.get(p).copied()).collect();
                let low = |k: usize| spots.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
                let high = |k: usize| spots.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
                travel.through_mm += high(0) - low(0) + high(1) - low(1);
            }
        }
        for (p, part) in parts.iter().enumerate() {
            if let (Some(near), Some(&spot)) = (part.near, spot_of.get(&p)) {
                travel.near_mm += dist(near, spot);
            }
        }
        (travel.through_mm, travel.near_mm) = (travel.through_mm.round(), travel.near_mm.round());
        // (a zone given ground where there is little height, with parts too tall for it)
        let mut tall: Vec<(String, f64, Vec<String>, f64)> = Vec::new();
        fn named<'w>(work: &'w Work, out: &mut Vec<&'w Work>) {
            if work.inside.is_empty() {
                out.push(work);
            }
            work.inside.iter().for_each(|inner| named(inner, out));
        }
        let mut smallest_zones: Vec<&Work> = Vec::new();
        laid.iter().for_each(|work| named(work, &mut smallest_zones));
        for zone in smallest_zones {
            let all = area(zone.rect).max(1e-9);
            for limit in &board.limits {
                let under = overlap(zone.rect, limit.rect) / all;
                let mut too: Vec<String> = zone
                    .parts
                    .iter()
                    .filter(|&&p| parts[p].height.is_some_and(|h| h > limit.height))
                    .map(|&p| parts[p].name.clone())
                    .collect();
                if under > 1e-6 && !too.is_empty() {
                    too.sort();
                    let highest = zone.parts.iter().filter_map(|&p| parts[p].height).fold(0.0, f64::max);
                    tall.push((zone.name.clone(), (under * 100.0).round() / 100.0, too, highest));
                }
            }
        }
        (laid, roads, travel, tall, bridges)
    };
    // What a laying out is weighed by: its track, each layer change as VIA_MM, and dearly what does not fit.
    let weight = |laid: &Laid| -> f64 {
        let (roads, travel) = (&laid.1, &laid.2);
        let over: usize = roads.iter().filter(|road| !road.gap).map(|road| road.used.saturating_sub(road.lanes)).sum();
        let too_tall: f64 = laid.3.iter().map(|(_, under, too, _)| under * too.len() as f64).sum();
        travel.mm
            + travel.through_mm
            + NEAR * travel.near_mm
            + NEAR * travel.mate_mm
            + CRAMPED_MM * travel.cramped
            + STRANDED_MM * travel.apart as f64
            + OVER_MM * travel.split as f64
            + NOISE * travel.noise
            + VIA_MM * travel.vias as f64
            + OVER_MM * over as f64
            + STRANDED_MM * travel.stranded as f64
            + TALL_MM * too_tall
    };
    let (mut turned, mut crossed, mut asked) = (BTreeSet::new(), BTreeSet::new(), BTreeMap::new());
    let mut sided: Sided = BTreeMap::new();
    let mut best = lay(&turned, &crossed, &sided, &asked);
    let first = best.2.clone();
    // A road asked for more than it holds is made to hold it, and the board laid out again: a few times, for
    // a wider road moves what is beside it.
    for _ in 0..WIDENINGS {
        let over: Vec<(String, usize)> = best
            .1
            .iter()
            .filter(|road| !road.gap && road.used > road.lanes)
            .map(|road| (road.cut.clone(), road.used + road.used / 4 + 1))
            .collect();
        if over.is_empty() {
            break;
        }
        for (cut, lanes) in over {
            let held = asked.entry(cut).or_insert(0);
            *held = (*held).max(lanes);
        }
        best = lay(&turned, &crossed, &sided, &asked);
    }
    // Then each cut in turn, the widest roads first, tried three other ways: its two lots changed over, made
    // across the shorter side of its ground, and both; what leaves the whole the better stays. The cuts that
    // part the most of the board are gone over twice, for one changes what another is worth.
    for round in 0..2 {
        let mut cuts: Vec<(usize, String)> = best.1.iter().map(|road| (road.level, road.cut.clone())).collect();
        cuts.sort();
        cuts.dedup();
        let most = if round == 0 { TURNINGS } else { TURNINGS / 8 };
        let mut better = false;
        for (level, cut) in cuts.into_iter().take(most) {
            for way in 1..4usize {
                if way > 1 && level > CROSS_LEVELS {
                    break;
                }
                let (mut other, mut across) = (turned.clone(), crossed.clone());
                if way & 1 == 1 && !other.remove(&cut) {
                    other.insert(cut.clone());
                }
                if way & 2 == 2 && !across.remove(&cut) {
                    across.insert(cut.clone());
                }
                let now = lay(&other, &across, &sided, &asked);
                if weight(&now) < weight(&best) - 1e-6 {
                    (turned, crossed, best, better) = (other, across, now, true);
                }
            }
        }
        // And at the cuts that part the most of the board, each zone in turn is put in the other lot.
        let wide_cuts: Vec<(String, [Vec<String>; 2])> = best
            .1
            .iter()
            .filter(|road| road.level <= MOVE_LEVELS)
            .map(|road| (road.cut.clone(), road.between.clone()))
            .collect();
        for (cut, lots) in wide_cuts {
            for (side, names) in lots.iter().enumerate() {
                for name in names {
                    let mut other = sided.clone();
                    other.insert((cut.clone(), name.clone()), side == 1);
                    let now = lay(&turned, &crossed, &other, &asked);
                    if weight(&now) < weight(&best) - 1e-6 {
                        (sided, best, better) = (other, now, true);
                    }
                }
            }
        }
        if !better {
            break;
        }
    }
    // (what the changes asked of the roads is seen to once more)
    for _ in 0..WIDENINGS {
        let over: Vec<(String, usize)> = best
            .1
            .iter()
            .filter(|road| !road.gap && road.used > road.lanes)
            .map(|road| (road.cut.clone(), road.used + road.used / 4 + 1))
            .collect();
        if over.is_empty() {
            break;
        }
        for (cut, lanes) in over {
            let held = asked.entry(cut).or_insert(0);
            *held = (*held).max(lanes);
        }
        let now = lay(&turned, &crossed, &sided, &asked);
        // (kept if less is left over than before: that is what it was done for)
        let left_over = |laid: &Laid| -> usize {
            laid.1.iter().filter(|road| !road.gap).map(|road| road.used.saturating_sub(road.lanes)).sum()
        };
        if left_over(&now) >= left_over(&best) {
            break;
        }
        best = now;
    }
    let (laid, mut roads, travel, tall, mut bridges) = best;
    let top = laid;
    // Ground for the vias where ways turn: each takes its own and as much again, in a square where the roads meet.
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
    // A place for every part within its zone. The zones are gone over twice: the second time each part knows
    // where those of the other zones came to stand.
    let mut on_gap: BTreeMap<usize, Pt> = bridges
        .iter()
        .filter_map(|bridge| parts.iter().position(|part| part.name == bridge.part).map(|p| (p, bridge.at)))
        .collect();
    // A part on a gap has a body like any other, and it is the first to have its ground: turned so that its
    // pads lie on the side of the gap their nets belong to, and along the gap clear of the others there. No
    // other part may stand on it.
    let domain_named = |name: &str| top.iter().position(|work| work.name == name);
    let mut gap_quarter: BTreeMap<usize, u8> = BTreeMap::new();
    let mut gap_rects: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
    {
        type Strip = (bool, [f64; 4], Vec<(usize, usize, f64, f64)>); // (bridge, part, want along, size)
        let mut strips: BTreeMap<[i64; 4], Strip> = BTreeMap::new();
        let mut gap_along: BTreeMap<usize, usize> = BTreeMap::new(); // a part on a gap -> the way its gap runs
        for (b, bridge) in bridges.iter().enumerate() {
            let Some(p) = parts.iter().position(|part| part.name == bridge.part) else { continue };
            let Some(home) = domain_named(&bridge.grounds[0]) else { continue };
            let here = bridge.at;
            let across = |quarters: u8| -> f64 {
                let mut far = 0.0;
                for (i, net) in parts[p].pads.iter().enumerate() {
                    let (Some(net), Some(&offset)) = (net, parts[p].offsets.get(i)) else { continue };
                    let Some(domain) = cx.net_domain(*net).filter(|&domain| to[domain] == to[home]) else { continue };
                    let from = quartered(quarters, offset);
                    far += off_rect([here[0] + from[0], here[1] + from[1]], top[domain].rect);
                }
                far
            };
            let quarters = (0..4u8)
                .fold((f64::INFINITY, 0u8), |best, k| if across(k) < best.0 - 1e-6 { (across(k), k) } else { best })
                .1;
            gap_quarter.insert(p, quarters);
            let other = domain_named(&bridge.grounds[1]);
            let strip = other
                .filter(|_| bridge.on == Across::Gap)
                .and_then(|other| strip_between(top[home].rect, top[other].rect, board.gap));
            if let Some((strip, up)) = strip {
                let along = usize::from(up);
                let key = strip.map(|v| (v * 1000.0).round() as i64);
                let entry = strips.entry(key).or_insert((up, strip, Vec::new()));
                entry.2.push((b, p, here[along], cx.lying(p, quarters)[along] + BESIDE));
            }
        }
        for (up, strip, on_it) in strips.into_values() {
            let along = usize::from(up);
            let (want, size): (Vec<f64>, Vec<f64>) = on_it.iter().map(|&(_, _, want, size)| (want, size)).unzip();
            for (&(b, p, _, _), place) in on_it.iter().zip(spread(&want, &size, strip[along], strip[along + 2])) {
                let mut at = middle(strip);
                at[along] = place;
                bridges[b].at = at;
                on_gap.insert(p, at);
                gap_along.insert(p, along);
            }
        }
        // (and each within its board: a long connector on a short gap reaches past the gap, not past the edge;
        // and off what the board shuts and the bodies on the other gaps: the largest first, each moved along
        // its gap to the nearest room there is)
        let mut order: Vec<usize> = (0..bridges.len()).collect();
        let their_parts: Vec<Option<usize>> =
            bridges.iter().map(|bridge| parts.iter().position(|part| part.name == bridge.part)).collect();
        let part_of = |b: usize| their_parts[b];
        order.sort_by(|&a, &b| {
            let takes = |b: usize| part_of(b).map_or(0.0, |p| cx.body(p)[0] * cx.body(p)[1]);
            takes(b).total_cmp(&takes(a)).then(a.cmp(&b))
        });
        for b in order {
            let Some(p) = part_of(b) else { continue };
            let Some(home) = domain_named(&bridges[b].grounds[0]) else { continue };
            let (quarters, region) = (gap_quarter.get(&p).copied().unwrap_or(0), boards[to[home]]);
            let [w, h] = cx.lying(p, quarters);
            let hold = |v: f64, low: f64, high: f64, size: f64| {
                if high - low >= size {
                    v.clamp(low + size / 2.0, high - size / 2.0)
                } else {
                    (low + high) / 2.0
                }
            };
            let mut at =
                [hold(bridges[b].at[0], region[0], region[2], w), hold(bridges[b].at[1], region[1], region[3], h)];
            let shut: Vec<[f64; 4]> = board.keepouts.iter().chain(gap_rects.values()).copied().collect();
            let band = match gap_along.get(&p) {
                Some(&along) => {
                    let mut band = cx.body_at(p, at, quarters);
                    (band[along], band[along + 2]) = (region[along], region[along + 2]);
                    band
                }
                None => region,
            };
            if let Some((free, false)) = settled(band, &[([w, h], at)], &shut)[0] {
                at = free;
            }
            bridges[b].at = at.map(|v| (v * 100.0).round() / 100.0);
            bridges[b].off = parts[p].placed.then(|| (dist(parts[p].at, at) * 10.0).round() / 10.0);
            on_gap.insert(p, at);
            gap_rects.insert(p, cx.body_at(p, at, quarters));
        }
    }
    // (the ground that is shut to every part: what the board shuts, and the bodies on the gaps)
    let closed = |gaps: &BTreeMap<usize, [f64; 4]>| -> Vec<[f64; 4]> {
        board.keepouts.iter().chain(gaps.values()).copied().collect()
    };
    let mut fine_leaves: Vec<(usize, [f64; 4], Vec<usize>)> = Vec::new();
    fn parts_of_leaves(work: &Work, domain: usize, out: &mut Vec<(usize, [f64; 4], Vec<usize>)>) {
        if work.inside.is_empty() {
            out.push((domain, work.rect, work.parts.clone()));
        }
        work.inside.iter().for_each(|inner| parts_of_leaves(inner, domain, out));
    }
    top.iter().enumerate().for_each(|(domain, work)| parts_of_leaves(work, domain, &mut fine_leaves));
    let mut stand: Vec<Option<Pt>> = vec![None; parts.len()];
    for (domain, rect, own) in &fine_leaves {
        let (mid, shift) = (middle(*rect), shifts[to[*domain]]);
        own.iter().for_each(|&p| stand[p] = Some([mid[0] + shift[0], mid[1] + shift[1]]));
    }
    for (domain, _, own) in &fine_leaves {
        let shift = shifts[to[*domain]];
        for (&p, at) in own.iter().filter_map(|p| on_gap.get(p).map(|at| (p, at))) {
            stand[p] = Some([at[0] + shift[0], at[1] + shift[1]]);
        }
    }
    // (the nets of the parts: the signals, and for each capacitor from a supply to a ground a tie to a chip of
    // its zone on that supply, the chips taking them in turn)
    let mut fine_nets: Vec<Vec<usize>> = nets.clone();
    let mut bypasses: Vec<(usize, usize, usize)> = Vec::new(); // (the capacitor, its chip, the supply)
    let mut taken: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (domain, _, own) in &fine_leaves {
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
    // A chip with a pad on a net that goes to an inductor and to little else switches it: that net is its
    // switch node, and what carries the switched current stands by the chip, the loop as small as may be:
    // the inductor and what else of two pads is on the node (a diode) by the chip's pad on the node; and the
    // capacitors of the chip's sheet from the far side of each of those to a ground (two at the most, the
    // smallest first) by that part's pad there. These go before the other capacitors.
    type Switcher = (usize, usize, Vec<usize>, Vec<usize>); // (the chip, its node, its loop's parts, those at its other pads)
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
    let in_loops: BTreeSet<usize> = of_loops.iter().map(|of| of.0).collect();
    bypasses.retain(|bypass| !in_loops.contains(&bypass.0));
    for &(c, by, _) in &of_loops {
        fine_nets.extend(vec![vec![c, by]; BYPASS]);
    }
    let loop_parts = of_loops.len();
    bypasses.splice(0..0, of_loops);
    // A signal is laid out in the order it flows, and what is there for one pad stands by it. On a small net
    // with one chip or connector on it (its head): a capacitor from the net to a ground or a supply, or
    // across to another net of that chip, filters what the chip takes in there and stands by its pad; a
    // diode from it to a ground or a supply clamps it and stands there too; and at a connector what is in
    // series with its pin comes first (a fuse, a resistor, a bead).
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
    let (none, no_sides, no_asks, no_kin) = (BTreeSet::new(), BTreeMap::new(), BTreeMap::new(), Vec::new());
    let no_rails: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut grounds: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
    for _ in 0..2 {
        for (domain, rect, own) in &fine_leaves {
            let mut each: Vec<Work> = own
                .iter()
                .filter(|p| !on_gap.contains_key(p))
                .map(|&p| Work { name: cx.name(p), parts: vec![p], inside: Vec::new(), rect: [0.0; 4], more: 0.0 })
                .collect();
            let mut on = Ground {
                board,
                nets: &fine_nets,
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
            arrange(&mut each, *rect, 0, false, &mut on);
            stand = std::mem::take(&mut on.at);
            grounds.extend(each.iter().map(|work| (work.parts[0], work.rect)));
        }
    }
    // Then the bodies themselves are settled on the ground of each zone, each as near the middle of the ground
    // that came to it as the others allow, the largest first; no part on ground that is shut.
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
    // A chip (or a connector) and the parts that stand by its pads are a block: laid out once, by themselves
    // on empty ground (the chip in the middle, each part beside the pad it is for, in the order they are
    // taken), and then placed as one body. Where each is taken after its neighbours, the ground by a chip's
    // pads is gone before its own parts come to it. Two blocks of the same make-up come out alike: the
    // steps that lay them out are the same.
    let mut order = bypasses.clone();
    // (the parts of the loops first, as they were found: each inductor before the capacitors by it)
    order[placed_first..].sort_by(|a, b| {
        let held = |c: usize| farads(&parts[c].value).unwrap_or(f64::INFINITY);
        held(a.0).total_cmp(&held(b.0)).then(parts[a.0].name.cmp(&parts[b.0].name))
    });
    let part_domain: BTreeMap<usize, usize> =
        fine_leaves.iter().flat_map(|(domain, _, own)| own.iter().map(move |&p| (p, *domain))).collect();
    let mut block_head: BTreeMap<usize, usize> = BTreeMap::new();
    let mut members_of: BTreeMap<usize, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for &(c, by, net) in &order {
        let head = block_head.get(&by).copied().unwrap_or(by);
        let free = c != head && !block_head.contains_key(&c) && !members_of.contains_key(&c);
        let together = grounds.contains_key(&head)
            && grounds.contains_key(&c)
            && part_domain.get(&c) == part_domain.get(&head)
            // (a part in pieces is no head of a block: what stands by it may stand between its pieces)
            && parts[head].pieces.is_empty();
        if free && together && !parts[by].offsets.is_empty() && !parts[head].offsets.is_empty() {
            block_head.insert(c, head);
            members_of.entry(head).or_default().push((c, by, net));
        }
    }
    #[derive(Clone)]
    struct Block {
        /// Each of its parts: where its middle is from the block's, and how many quarters it is turned.
        at: Vec<(usize, Pt, u8)>,
        /// The same as the chip is drawn: where each is from the chip, and its quarters from its own drawing.
        drawn: Vec<(usize, Pt, u8)>,
        through: bool,
        /// For each part that stands by a pad: (the part, (its pad, the part it stands by, that one's pad)).
        pins: Vec<(usize, (usize, usize, usize))>,
    }
    let lay_block = |head: usize, list: &[(usize, usize, usize)]| -> Block {
        // (where each part is from the chip, and how many quarters it is turned from how it is drawn)
        let mut pos: BTreeMap<usize, (Pt, u8)> = [(head, ([0.0, 0.0], 0u8))].into_iter().collect();
        let [hw, hh] = cx.body(head);
        let reach = hw.max(hh) / 2.0 + BLOCK_REACH;
        let ground = [-reach, -reach, reach, reach];
        // (what lies already, with the sides of the board it takes ground on)
        let mut shut: Vec<([f64; 4], u8)> = vec![(cx.drawn_rect(head, [0.0, 0.0], 0), cx.side_of(head))];
        let mut at_pad: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        let mut pins: Vec<(usize, (usize, usize, usize))> = Vec::new();
        for &(c, by, net) in list {
            let Some(&(by_at, by_q)) = pos.get(&by) else { continue };
            let pad_of = |i: usize| {
                let from = quartered(by_q, cx.drawn_pad(by, i));
                [by_at[0] + from[0], by_at[1] + from[1]]
            };
            let on_net = (0..parts[by].pads.len()).filter(|&i| parts[by].pads[i] == Some(net));
            // (a part whose other pad is on a net of the same part stands between the two pads: of the pads
            // on this net, the one nearest a pad on that other net; else the pads in turn)
            let partners: Vec<Pt> = parts[c]
                .pads
                .iter()
                .flatten()
                .filter(|&&other| other != net && cx.is_signal(other))
                .flat_map(|&other| (0..parts[by].pads.len()).filter(move |&i| parts[by].pads[i] == Some(other)))
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
            let Some(pin) = on_net.min_by(|&a, &b| {
                let has = |i: usize| at_pad.get(&(by, i)).copied().unwrap_or(0);
                across(a).cmp(&across(b)).then(has(a).cmp(&has(b))).then(a.cmp(&b))
            }) else {
                continue;
            };
            // (just off the body of what it stands by, on the side the pad is nearest, lying away from it)
            let pad = pad_of(pin);
            let by_body = cx.drawn_rect(by, by_at, by_q);
            let [cw, ch] = cx.drawn_body(c, 0);
            let (long, short) = (cw.max(ch), cw.min(ch));
            let half = long / 2.0;
            let sides = [
                (pad[0] - by_body[0], [by_body[0] - half, pad[1]], true),
                (by_body[2] - pad[0], [by_body[2] + half, pad[1]], true),
                (pad[1] - by_body[1], [pad[0], by_body[1] - half], false),
                (by_body[3] - pad[1], [pad[0], by_body[3] + half], false),
            ];
            let (_, want, along_x) =
                sides.iter().fold(sides[0], |best, side| if side.0 < best.0 { *side } else { best });
            // (a part on the other side of the board from what it stands by stands under the pad itself, as
            // its footprint is drawn or a quarter from that: there is nothing of the other in its way)
            let under = cx.side_of(c) & cx.side_of(by) == 0;
            let (want, lying) =
                if under { (pad, [cw, ch]) } else { (want, if along_x { [long, short] } else { [short, long] }) };
            let quarter_more = (lying[0] - cw).abs() > 1e-9;
            let in_its_way: Vec<[f64; 4]> =
                shut.iter().filter(|lies| lies.1 & cx.side_of(c) != 0).map(|lies| lies.0).collect();
            let Some((at, turn)) = settled(ground, &[(lying, want)], &in_its_way)[0] else { continue };
            if dist(at, pad) > BLOCK_REACH {
                continue;
            }
            // (of the two ways it may lie there, the one with its own pad on the net toward the pad)
            let mut q = u8::from(turn != quarter_more);
            if let Some(mine) = parts[c].pads.iter().position(|&of| of == Some(net)) {
                if mine < parts[c].offsets.len() {
                    let far = |q: u8| {
                        let from = quartered(q, cx.drawn_pad(c, mine));
                        dist([at[0] + from[0], at[1] + from[1]], pad)
                    };
                    if far(q + 2) < far(q) - 1e-9 {
                        q += 2;
                    }
                }
                pins.push((c, (mine, by, pin)));
            }
            *at_pad.entry((by, pin)).or_insert(0) += 1;
            pos.insert(c, (at, q));
            shut.push((cx.drawn_rect(c, at, q), cx.side_of(c)));
        }
        let low = |k: usize| shut.iter().map(|lies| lies.0[k]).fold(f64::INFINITY, f64::min);
        let high = |k: usize| shut.iter().map(|lies| lies.0[k + 2]).fold(f64::NEG_INFINITY, f64::max);
        let mid = [(low(0) + high(0)) / 2.0, (low(1) + high(1)) / 2.0];
        Block {
            // (told as each part stands in the file: turned from that by what it is turned from its drawing,
            // less what it stands turned already)
            at: pos
                .iter()
                .map(|(&p, &(at, q))| (p, [at[0] - mid[0], at[1] - mid[1]], (q + 4 - cx.stands_turned(p)) % 4))
                .collect(),
            drawn: pos.iter().map(|(&p, &(at, q))| (p, at, q)).collect(),
            through: pos.keys().any(|&p| parts[p].through),
            pins,
        }
    };
    let mut laid_blocks: BTreeMap<usize, Block> = members_of
        .iter()
        .map(|(&head, list)| (head, lay_block(head, list)))
        .filter(|(_, block)| block.at.len() > 1)
        .collect();
    // The blocks of one make-up, and whether they are laid out alike. A make-up is the kind of the chip (its
    // footprint and its value) and, for each part by it, its kind and the pads it stands between; a layout is
    // where each such part lies from the chip and how it is turned.
    // (the block a part stands in, by its chip)
    let in_blocks = |p: usize, blocks: &BTreeMap<usize, Block>| -> Option<usize> {
        blocks.iter().find(|(_, block)| block.at.iter().any(|placed| placed.0 == p)).map(|(&head, _)| head)
    };
    type Made = BTreeMap<(String, Vec<String>), Vec<(usize, Vec<String>)>>; // make-up -> [(chip, layout)]
    let mut made: Made = BTreeMap::new();
    for (&head, block) in &laid_blocks {
        let by_pad: BTreeMap<usize, (usize, usize, usize)> = block.pins.iter().copied().collect();
        let mut make: Vec<String> = Vec::new();
        let mut lies: Vec<String> = Vec::new();
        for &(m, at, q) in block.drawn.iter().filter(|placed| placed.0 != head) {
            let pads = by_pad.get(&m).map_or(String::new(), |&(mine, by, pin)| format!("{mine}>{}:{pin}", cx.kind(by)));
            make.push(format!("{} {pads}", cx.kind(m)));
            lies.push(format!("{} {pads} {:.2} {:.2} {q}", cx.kind(m), at[0] + 0.0, at[1] + 0.0));
        }
        make.sort();
        lies.sort();
        made.entry((cx.kind(head), make)).or_default().push((head, lies));
    }
    let mut alike: Vec<(usize, usize, usize, Vec<String>)> = made
        .values()
        .filter(|group| group.len() > 1)
        .map(|group| {
            let mut count: BTreeMap<&Vec<String>, usize> = BTreeMap::new();
            group.iter().for_each(|(_, lies)| *count.entry(lies).or_insert(0) += 1);
            let most = count.values().copied().max().unwrap_or(0);
            (group.len(), most, group[0].1.len() + 1, cx.names(&group.iter().map(|of| of.0).collect::<Vec<_>>()))
        })
        .collect();
    alike.sort_by(|a, b| (b.0 * b.2).cmp(&(a.0 * a.2)).then(a.3.cmp(&b.3)));
    let blocks = [laid_blocks.len(), laid_blocks.values().map(|block| block.at.len()).sum()];
    // Sheets that repeat are laid out alike: one is laid out, by itself on empty ground, and every copy is
    // placed as that one lies, as one body. Two sheets repeat if their parts are of the same kinds, joined the
    // same way: told by colouring the parts, each by its kind, then again and again by what its pads are
    // joined to (the kinds and pads of the parts on each net within the sheet; whether the net is a ground,
    // a supply, or leaves the sheet), until two parts have one colour only if they sit alike in their
    // sheets. The parts of two such sheets are matched colour by colour.
    // (the sheets, the parts of each that are placed whole, a part of each, and the parts of each in one order)
    type Copies = (Vec<String>, usize, Vec<usize>, Vec<Vec<usize>>);
    let mut unit_groups: Vec<Copies> = Vec::new();
    // (for a body of several copies, by its first part: what is tried if it finds no room, in turn; each a
    // set of bodies, by a part of each)
    let mut alternatives: BTreeMap<usize, Vec<Vec<(usize, Block)>>> = BTreeMap::new();
    let mut in_rows: BTreeMap<usize, usize> = BTreeMap::new(); // a body of copies, by its first part -> the copies side by side
    {
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
                    let block = in_blocks(p, &laid_blocks);
                    let inner =
                        block.map_or(true, |head| laid_blocks[&head].at.iter().all(|placed| own.contains(&placed.0)));
                    grounds.contains_key(&p)
                        && part_domain.get(&p) == part_domain.get(&own[0])
                        && !parts[p].offsets.is_empty()
                        && inner
                };
                let whole: Vec<usize> = (0..copies[0].1.len())
                    .filter(|&j| copies.iter().all(|(_, own)| within_sheet(own, own[j])))
                    .collect();
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
                let chosen: BTreeSet<usize> = whole.iter().map(|&j| model[j]).collect();
                let mut items: Vec<Vec<(usize, Pt, u8)>> = Vec::new(); // each: its parts, where from its middle, quarters from their drawing
                let mut taken_up: BTreeSet<usize> = BTreeSet::new();
                for &p in &chosen {
                    if taken_up.contains(&p) {
                        continue;
                    }
                    match in_blocks(p, &laid_blocks) {
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
                let takes = |item: &Vec<(usize, Pt, u8)>| -> f64 {
                    item.iter().map(|&(p, _, _)| cx.body(p)[0] * cx.body(p)[1]).sum()
                };
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
                let mut fits = true;
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
                            if let Some(net) =
                                net.filter(|&net| cx.is_signal(net) && reach[net].iter().any(|p| mine.contains(p)))
                            {
                                theirs.entry(net).or_default().push(pad_of(q, i, at, turn));
                            }
                        }
                    }
                    let all: Vec<Pt> = theirs.values().flatten().copied().collect();
                    let want = median(&all).unwrap_or([0.0, 0.0]);
                    let ways: Vec<Vec<[f64; 4]>> = (0..4u8)
                        .map(|way| {
                            item.iter().map(|&(p, at, q)| cx.drawn_rect(p, quartered(way, at), (q + way) % 4)).collect()
                        })
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
                        .fold(
                            None::<(f64, u8, Pt)>,
                            |best, now| if best.is_some_and(|b| b.0 <= now.0) { best } else { Some(now) },
                        );
                    let Some((_, way, here)) = best else {
                        fits = false;
                        break;
                    };
                    for &(p, at, q) in &item {
                        let from = quartered(way, at);
                        let (to, turn) = ([here[0] + from[0], here[1] + from[1]], (q + way) % 4);
                        lies.insert(p, (to, turn));
                        shut.push(cx.drawn_rect(p, to, turn));
                    }
                }
                if !fits || shut.is_empty() {
                    unit_groups.push((
                        names,
                        copies[0].1.len(),
                        Vec::new(),
                        copies.iter().map(|copy| copy.1.clone()).collect(),
                    ));
                    continue;
                }
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
                            ((n % across) as f64 - (across - 1) as f64 / 2.0) * (size[0] + BETWEEN_COPIES),
                            ((n / across) as f64 - (down - 1) as f64 / 2.0) * (size[1] + BETWEEN_COPIES),
                        ];
                        let moved = |placed: &(usize, Pt, u8)| {
                            (placed.0, [placed.1[0] + step[0], placed.1[1] + step[1]], placed.2)
                        };
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
    }
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
            let mut base: Vec<[f64; 4]> = closed(&gap_rects);
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
                let mut over_roads: Vec<[f64; 4]> = closed(&gap_rects);
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
        let mut shut: Vec<[f64; 4]> = closed(&gap_rects);
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
            let mut shut: Vec<[f64; 4]> = closed(&gap_rects);
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
                    let mut open: Vec<[f64; 4]> = closed(&gap_rects);
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
    let mut shift_of: Vec<Pt> = vec![[0.0, 0.0]; parts.len()];
    let mut domain_of_part: Vec<usize> = vec![0; parts.len()];
    for (domain, _, own) in &fine_leaves {
        for &p in own {
            (shift_of[p], domain_of_part[p]) = (shifts[to[*domain]], *domain);
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
    for (c, chip, rail) in order {
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
        let mut shut: Vec<[f64; 4]> = closed(&gap_rects);
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
            let (Some(home), Some(other)) = (domain_named(&bridge.grounds[0]), domain_named(&bridge.grounds[1])) else {
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
            let mut shut: Vec<[f64; 4]> = closed(&gap_rects);
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
    for (_, _, own) in &fine_leaves {
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
    // The pours, as they are told.
    let mut rects: Vec<[f64; 4]> = Vec::new();
    fn grounds_of(work: &Work, out: &mut Vec<[f64; 4]>) {
        if work.inside.is_empty() {
            out.push(work.rect);
        }
        work.inside.iter().for_each(|inner| grounds_of(inner, out));
    }
    top.iter().for_each(|work| grounds_of(work, &mut rects));
    let in_pieces = pieces(&rects);
    let mut pours: Vec<Pour> = (0..rail_nets.len())
        .filter(|&r| in_pieces[r] > 0)
        .map(|r| {
            let under: Vec<[f64; 4]> =
                (0..rects.len()).filter(|&l| on_plane[l].contains(&r)).map(|l| rects[l]).collect();
            let fed = |p: &&usize| leaf_at.get(p).is_some_and(|&l| on_plane[l].contains(&r));
            Pour {
                net: cx.net_name(rail_nets[r]),
                ground: found.rails[&rail_nets[r]].0.map(|net| cx.net_name(net)).unwrap_or_default(),
                layers: power_layers.clone(),
                area: under.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum::<f64>().round(),
                rects: under.into_iter().map(|r| r.map(|v| (v * 100.0).round() / 100.0)).collect(),
                parts: reach[rail_nets[r]].iter().filter(fed).count(),
                by_track: cx.names(&rail_parts[r].1),
                pieces: in_pieces[r],
            }
        })
        .collect();
    pours.sort_by(|a, b| b.area.total_cmp(&a.area).then(a.net.cmp(&b.net)));
    // (what stands on a gap: the parts on a ground of either side)
    for road in roads.iter_mut().filter(|road| road.gap) {
        let side = |p: usize, names: &Vec<String>| found.stands[p].iter().any(|&g| names.contains(&all_nets[g].name));
        let mut on_it: Vec<String> = (0..parts.len())
            .filter(|&p| side(p, &road.between[0]) && side(p, &road.between[1]))
            .map(|p| parts[p].name.clone())
            .collect();
        on_it.sort();
        road.parts = on_it;
    }
    let regions: Vec<Region> = boards
        .iter()
        .zip(&held)
        .map(|(&rect, names)| Region {
            rect,
            room: cx.room(rect).round(),
            needed: top.iter().filter(|work| names.contains(&work.name)).map(|work| cx.need(work)).sum::<f64>().round(),
            roads: roads.iter().map(|road| overlap(road.rect, rect)).sum::<f64>().round(),
            zones: names.clone(),
        })
        .collect();
    roads.sort_by(|a, b| a.level.cmp(&b.level).then(b.width.total_cmp(&a.width)));

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
    let zones: Vec<Zone> = top.iter().map(|work| told(work, board, nets, &mut leaf_of, &mut leaves)).collect();
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
    Ok(Floorplan {
        power,
        zones,
        flows,
        apart,
        now_mm: now_mm.round(),
        then_mm: then_mm.round(),
        regions,
        stranded: travel.left.iter().map(|&k| cx.net_name(signal[k])).collect(),
        tall,
        travel,
        first,
        bridges,
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
        parts_now_mm: parts_now_mm.round(),
        parts_then_mm: parts_then_mm.round(),
        against: against.into_iter().map(|(kind, count, now, then)| (kind, count, now.round(), then.round())).collect(),
        pads_first_mm: pads_first_mm.round(),
        pads_mm: pads_mm.round(),
        bypass,
        capacitors,
        junctions,
        pours,
        roads,
    })
}
