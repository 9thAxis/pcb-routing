//! The supplies on the plane layers: which is poured under which of the smallest zones.

use super::ground::Work;
use super::known::Known;
use super::settle::strip_between;
use super::types::Pour;
use std::collections::BTreeMap;

/// The supplies on the plane layers, by the smallest zones of the zone tree.
pub(super) struct Planes<'w> {
    /// The smallest zones: the domain and the parts of each.
    pub(super) leaf_parts: Vec<(usize, &'w Vec<usize>)>,
    /// The smallest zone each part is of.
    pub(super) leaf_at: BTreeMap<usize, usize>,
    /// The supplies, and the domain of each.
    pub(super) rail_nets: Vec<usize>,
    pub(super) rail_domain: Vec<Option<usize>>,
    /// The supplies poured under each of the smallest zones.
    pub(super) on_plane: Vec<Vec<usize>>,
    /// Of each supply, the parts that stand on its pour, and those that do not.
    pub(super) rail_parts: Vec<(Vec<usize>, Vec<usize>)>,
    /// For each supply: a part of each of the smallest zones it is poured under.
    pub(super) kin: Vec<Vec<usize>>,
}

/// The supplies on the plane layers (every plane after the first, the ground's): under each of the smallest
/// zones, as many supplies as there are such layers, those with the most parts there, of the supplies of
/// its ground that reach several zones (one that stays within a zone is laid there, on the layers of its
/// parts). A part on two grounds stands on a gap, beside the pours of both.
pub(super) fn planes<'w>(cx: &Known, top: &'w [Work]) -> Planes<'w> {
    let (found, reach, domain_at, power_layers) = (&cx.found, &cx.reach, &cx.domain_at, &cx.power_layers);
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
    Planes { leaf_parts, leaf_at, rail_nets, rail_domain, on_plane, rail_parts, kin }
}

impl Planes<'_> {
    /// The pieces each supply's pour is in, given the ground of every smallest zone: zones that are neighbours
    /// are one piece, and so are zones joined through ground that has a plane layer to spare.
    pub(super) fn pieces(&self, cx: &Known, rects: &[[f64; 4]]) -> Vec<usize> {
        let (board, power_layers) = (cx.board, &cx.power_layers);
        let Planes { leaf_parts, rail_nets, rail_domain, on_plane, .. } = self;
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
    }
}

/// The pours, as they are told: of each supply that is poured, the ground it has under the zones of the
/// layout chosen.
pub(super) fn pours(cx: &Known, planes: &Planes, top: &[Work]) -> Vec<Pour> {
    let (found, reach, power_layers) = (&cx.found, &cx.reach, &cx.power_layers);
    let Planes { leaf_at, rail_nets, on_plane, rail_parts, .. } = planes;
    let mut rects: Vec<[f64; 4]> = Vec::new();
    fn grounds_of(work: &Work, out: &mut Vec<[f64; 4]>) {
        if work.inside.is_empty() {
            out.push(work.rect);
        }
        work.inside.iter().for_each(|inner| grounds_of(inner, out));
    }
    top.iter().for_each(|work| grounds_of(work, &mut rects));
    let in_pieces = planes.pieces(cx, &rects);
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
    pours
}
