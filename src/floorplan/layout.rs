//! The laying out of the zones: ground for each, the roads between them, and the search for the best.

use super::boards::Boards;
use super::constants::{
    BESIDE, CORNER, CRAMPED_MM, CROSS_LEVELS, MOVE_LEVELS, NEAR, NOISE, OVER_MM, QUIET_MM, STRANDED_MM, TALL_MM,
    TURNINGS, VIA_MM, WIDENINGS,
};
use super::ground::{arrange, Ground, Work};
use super::known::Known;
use super::planes::Planes;
use super::settle::{area, median, middle, overlap, spread, strip_between};
use super::types::{Across, Bridge, Road, Travel};
use super::ways::travelled;
use crate::geom::dist;
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// One laying out: ground for every zone, the roads, a way along them for every net between zones, the zones
/// with parts too tall for their ground, and the parts that stand on two grounds.
pub(super) type Laid = (Vec<Work>, Vec<Road>, Travel, Vec<(String, f64, Vec<String>, f64)>, Vec<Bridge>);
type Sided = BTreeMap<(String, String), bool>;
/// A part that stands on two grounds: the part, the domain it is counted with, the others it stands on, and
/// the parts its signal nets tie it to.
type Span = (usize, usize, Vec<usize>, Vec<usize>);

/// The parts that stand on two grounds: each with the domain it is counted with, the others it stands on
/// (the one most of its pads are on first), and the parts its signal nets tie it to.
fn spans(cx: &Known) -> Vec<Span> {
    let (parts, found, nets, domain_at) = (cx.parts, &cx.found, &cx.nets, &cx.domain_at);
    let mut spans: Vec<(usize, usize, Vec<usize>, Vec<usize>)> = Vec::new();
    for p in (0..parts.len()).filter(|&p| found.stands[p].len() > 1 && !cx.held(p)) {
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
    spans
}

/// What every laying out is made from: the zone tree, the supplies on the planes, the boards.
struct Laying<'a> {
    cx: &'a Known<'a>,
    top: &'a [Work],
    planes: &'a Planes<'a>,
    on: &'a Boards,
    /// Where each part is taken to stand before any zone has ground: a fixed part where it stands.
    at: Vec<Option<Pt>>,
    spans: Vec<Span>,
}

impl Laying<'_> {
    /// One laying out: ground for every zone, the roads, and a way along them for every net between zones.
    /// The cuts named in `turned` have their lots changed over, those in `crossed` are made across the shorter
    /// side; `sided` puts a zone in a lot of a cut, and `asked` is what a cut's road is to hold at the least.
    fn lay(
        &self,
        turned: &BTreeSet<String>,
        crossed: &BTreeSet<String>,
        sided: &Sided,
        asked: &BTreeMap<String, usize>,
    ) -> Laid {
        let (cx, top, planes) = (self.cx, self.top, self.planes);
        let (board, nets, mate_of, aggressor, victim) = (cx.board, &cx.nets, &cx.mate_of, &cx.aggressor, &cx.victim);
        let (boards, shifts, to) = (&self.on.boards, &self.on.shifts, &self.on.to);
        let mut on = Ground {
            board,
            nets,
            rails: &planes.rail_parts,
            at: self.at.clone(),
            roads: Vec::new(),
            region: boards[0],
            shift: shifts[0],
            turned,
            crossed,
            sided,
            asked,
            mates: mate_of,
            fine: false,
            kin: &planes.kin,
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
        travel.split = planes.pieces(cx, &rects).iter().map(|&n| n.saturating_sub(1)).sum();
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
        let bridges = self.bridged(&laid, &leaves, &mut travel);
        let tall = self.too_tall(&laid);
        (laid, roads, travel, tall, bridges)
    }

    /// A place for each part that stands on two grounds, and what the laying out comes to by where its
    /// parts stand: how far what goes from domain to domain reaches, and how far what is to be near a point
    /// is from it (by the middles of the smallest zones, as the boards stack).
    fn bridged(&self, laid: &[Work], leaves: &[(usize, [f64; 4], &Vec<usize>)], travel: &mut Travel) -> Vec<Bridge> {
        let (cx, top, spans) = (self.cx, self.top, &self.spans);
        let (board, parts, nets, mate_of) = (cx.board, cx.parts, &cx.nets, &cx.mate_of);
        let (shifts, to) = (&self.on.shifts, &self.on.to);
        // (how far what goes from domain to domain reaches, and how far what is to be near a point is from it:
        // by the middles of the smallest zones, as the boards stack)
        let mut spot_of: BTreeMap<usize, Pt> = BTreeMap::new();
        for &(domain, rect, own) in leaves {
            let (mid, shift) = (middle(rect), shifts[to[domain]]);
            spot_of.extend(own.iter().map(|&p| (p, [mid[0] + shift[0], mid[1] + shift[1]])));
        }
        for p in (0..parts.len()).filter(|&p| cx.held(p)) {
            let shift = self.on.shift_at(parts[p].at);
            spot_of.insert(p, [parts[p].at[0] + shift[0], parts[p].at[1] + shift[1]]);
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
            for (p, home, others, mates) in spans {
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
            for (bridge, (p, home, _, _)) in bridges.iter().zip(spans) {
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
        for (bridge, (p, _, _, _)) in bridges.iter_mut().zip(spans) {
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
        // (and what is held where it stands is joined from there: each of its nets to the nearest zone of that net)
        for net in nets.iter() {
            let free: Vec<Pt> = net.iter().filter(|&&p| !cx.held(p)).filter_map(|p| spot_of.get(p).copied()).collect();
            for at in net.iter().filter(|&&p| cx.held(p)).filter_map(|p| spot_of.get(p)) {
                let near =
                    free.iter().map(|to| (to[0] - at[0]).abs() + (to[1] - at[1]).abs()).fold(f64::INFINITY, f64::min);
                if near.is_finite() {
                    travel.held_mm += near;
                }
            }
        }
        travel.held_mm = travel.held_mm.round();
        (travel.through_mm, travel.near_mm) = (travel.through_mm.round(), travel.near_mm.round());
        bridges
    }

    /// The zones given ground where there is little height, with parts too tall for it.
    fn too_tall(&self, laid: &[Work]) -> Vec<(String, f64, Vec<String>, f64)> {
        let (board, parts) = (self.cx.board, self.cx.parts);
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
        tall
    }
}

/// What a laying out is weighed by: its track, each layer change as VIA_MM, and dearly what does not fit.
fn weight(laid: &Laid) -> f64 {
    let (roads, travel) = (&laid.1, &laid.2);
    let over: usize = roads.iter().filter(|road| !road.gap).map(|road| road.used.saturating_sub(road.lanes)).sum();
    let too_tall: f64 = laid.3.iter().map(|(_, under, too, _)| under * too.len() as f64).sum();
    travel.mm
        + travel.through_mm
        + NEAR * travel.near_mm
        + travel.held_mm
        + NEAR * travel.mate_mm
        + CRAMPED_MM * travel.cramped
        + STRANDED_MM * travel.apart as f64
        + OVER_MM * travel.split as f64
        + NOISE * travel.noise
        + VIA_MM * travel.vias as f64
        + OVER_MM * over as f64
        + STRANDED_MM * travel.stranded as f64
        + TALL_MM * too_tall
}

/// The laying out that is chosen, and what the first one came to: ground for each zone from the whole
/// board down, then roads widened and cuts tried other ways for as long as that leaves the whole the better.
pub(super) fn laid_out(cx: &Known, top: &[Work], planes: &Planes, on: &Boards) -> (Laid, Travel) {
    let parts = cx.parts;
    let at: Vec<Option<Pt>> = parts.iter().map(|part| (part.fixed && part.placed).then_some(part.at)).collect();
    let laying = Laying { cx, top, planes, on, at, spans: spans(cx) };
    let (mut turned, mut crossed, mut asked) = (BTreeSet::new(), BTreeSet::new(), BTreeMap::new());
    let mut sided: Sided = BTreeMap::new();
    let mut best = laying.lay(&turned, &crossed, &sided, &asked);
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
        best = laying.lay(&turned, &crossed, &sided, &asked);
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
                let now = laying.lay(&other, &across, &sided, &asked);
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
                    let now = laying.lay(&turned, &crossed, &other, &asked);
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
        let now = laying.lay(&turned, &crossed, &sided, &asked);
        // (kept if less is left over than before: that is what it was done for)
        let left_over = |laid: &Laid| -> usize {
            laid.1.iter().filter(|road| !road.gap).map(|road| road.used.saturating_sub(road.lanes)).sum()
        };
        if left_over(&now) >= left_over(&best) {
            break;
        }
        best = now;
    }
    (best, first)
}
