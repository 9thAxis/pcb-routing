//! Ground for each part within its zone, and the bodies settled on it.

use super::Site;
use crate::floorplan::ground::{arrange, Ground, Work};
use crate::floorplan::settle::middle;
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
            arrange(&mut each, *rect, 0, false, &mut on);
            stand = std::mem::take(&mut on.at);
            grounds.extend(each.iter().map(|work| (work.parts[0], work.rect)));
        }
    }
    grounds
}
