//! Where a group of joined pads may be reached: at its pads, and on the zone that joins it.

use super::shape::{along, apart, bounds, rounded, Bounds, Piece};
use super::{Fill, Pad, Place};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::HashMap;

/// Pads of a group offered to each pad of another group of its net: the nearest.
const PIECE_PADS: usize = 2;
/// From a pad, in mm and in four directions: where a pour is tried for a point to end on.
const POUR_AROUND: [f64; 3] = [0.0, 1.0, 2.0];
/// Pads of another group that a pour is tried round: those nearest it.
const POUR_PADS: usize = 4;
/// Places along the edge of another group's pour that a pour is tried from, at most.
const POUR_EDGE: usize = 60;
/// Two pours further apart than this (by their boxes) are not tried along their edges, in mm.
const POUR_NEAR: f64 = 3.0;
/// The least distance between two points offered on one pour, in mm.
pub(super) const POUR_SPREAD: f64 = 1.0;
/// Most points of one group.
const PIECE_POINTS: usize = 64;
const FOUR_WAYS: [(f64, f64); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];

/// The point of a pour to take a track from `p` to: `p` itself if it is on the pour, else the nearest point
/// of its edge moved `inset` further in. None if that is not on the pour (a sliver too thin to end a track on).
pub(super) fn into_pour(pour: &Piece, p: Pt, inset: f64) -> Option<Pt> {
    if pour.contains(p) {
        return Some(p);
    }
    let (d, q) = pour.nearest_edge(p).filter(|&(d, _)| d > 0.0)?;
    let q = rounded([q[0] + (q[0] - p[0]) / d * inset, q[1] + (q[1] - p[1]) / d * inset]);
    pour.contains(q).then_some(q)
}

/// A pour of the net at hand, as the search for places on it uses it.
struct Pour<'a> {
    piece: Piece<'a>,
    layer: &'a str,
    /// The box round its outline.
    outline: Bounds,
}

/// A place found for a group.
struct Found<'a> {
    at: Pt,
    layers: Vec<&'a str>,
    /// How far it is from what it was found for.
    far: f64,
    /// Whether it is a point of a pour (not a pad).
    poured: bool,
}

/// The places found for one group, each once: the nearer finding of a place stands.
#[derive(Default)]
struct Offers<'a> {
    found: Vec<Found<'a>>,
    /// (the place in millionths of a millimetre, its layers) -> its index in `found`
    known: HashMap<(i64, i64, Vec<&'a str>), usize>,
}

impl<'a> Offers<'a> {
    fn offer(&mut self, far: f64, at: Pt, layers: Vec<&'a str>, poured: bool) {
        if layers.is_empty() {
            return;
        }
        let at = rounded(at);
        let key = ((at[0] * 1e6).round() as i64, (at[1] * 1e6).round() as i64, layers.clone());
        let found = Found { at, layers, far, poured };
        match self.known.get(&key) {
            Some(&k) if far < self.found[k].far => self.found[k] = found,
            Some(_) => {}
            None => {
                self.known.insert(key, self.found.len());
                self.found.push(found);
            }
        }
    }

    /// The point of a pour nearest to `near`, if it is well within the pour: not on a sliver of it.
    fn offer_pour(&mut self, pour: &Pour<'a>, near: Pt, inset: f64, step: Option<f64>) {
        let Some(mut at) = into_pour(&pour.piece, near, inset) else { return };
        if let Some(step) = step {
            at = rounded([(at[0] / step).round_ties_even() * step, (at[1] / step).round_ties_even() * step]);
        }
        let room = |&(ex, ey): &(f64, f64)| pour.piece.contains([at[0] + 0.7 * inset * ex, at[1] + 0.7 * inset * ey]);
        if FOUR_WAYS.iter().all(room) {
            self.offer(dist(at, near), at, vec![pour.layer], true);
        }
    }

    /// The places to give: the nearest first, [`PIECE_POINTS`] at most, and no two on one pour closer than
    /// [`POUR_SPREAD`].
    fn chosen(mut self) -> Vec<Place> {
        let order = |a: &Found, b: &Found| {
            let by_place = a.at[0].total_cmp(&b.at[0]).then(a.at[1].total_cmp(&b.at[1]));
            a.far.total_cmp(&b.far).then(by_place).then_with(|| a.layers.cmp(&b.layers))
        };
        self.found.sort_by(order);
        let mut chosen: Vec<Found> = Vec::new();
        for found in self.found {
            let crowded =
                |other: &Found| other.poured && other.layers == found.layers && dist(other.at, found.at) < POUR_SPREAD;
            if chosen.len() < PIECE_POINTS && !(found.poured && chosen.iter().any(crowded)) {
                chosen.push(found);
            }
        }
        let names = |layers: Vec<&str>| layers.into_iter().map(String::from).collect();
        chosen.into_iter().map(|found| Place { xy: found.at, layers: names(found.layers) }).collect()
    }
}

/// Where each group of joined pads of a net may be reached: per group, its places. A router joins a group
/// at whichever of them comes cheapest (see [`crate::Terminal::piece`]).
///
/// A lone pad is reached at the pad. A group of several has, for each pad of every other group, its own
/// pads nearest to that pad. If a pour joins it, it also has points of the pour near the pads of each other
/// group that are nearest the pour: the pad's own place if the pour is there (on another layer: under the
/// pad), places round it, and the nearest point of the pour's edge moved `inset` in. Where the other group
/// has a pour close by (one pour cut in two), it has instead the points of its pour nearest to places along
/// the edge of the other: the two can be joined anywhere along the cut. A point of a pour is moved onto the
/// grid of `step` if one is given (a track ends on a cell; only a pad is worth leaving the grid for), and
/// two on one pour are 1 mm apart at least. The nearest come first, and there are 64 at most.
///
/// `groups`: the net's groups as indices into `pads`; `pours`: for each group, in the same order, the pours
/// that join it as indices into `fills` (groups it does not reach have none); `layers`: the routing layers. A group none of whose pads is on a routing layer
/// has no place.
pub fn reach(
    pads: &[Pad],
    fills: &[Fill],
    groups: &[Vec<usize>],
    pours: &[Vec<usize>],
    layers: &[String],
    inset: f64,
    step: Option<f64>,
) -> Vec<Vec<Place>> {
    let on =
        |pad: &Pad| -> Vec<&str> { layers.iter().filter(|l| pad.layers.contains(l)).map(String::as_str).collect() };
    let round_about: Vec<(f64, f64)> = POUR_AROUND
        .iter()
        .flat_map(|&d| FOUR_WAYS[..if d > 0.0 { 4 } else { 1 }].iter().map(move |&(dx, dy)| (d * dx, d * dy)))
        .collect();
    // the pours of each group that are on a routing layer
    let usable: Vec<Vec<Pour>> = (0..groups.len())
        .map(|g| {
            let own = pours.get(g).map_or(&[][..], |own| &own[..]);
            let routed = own.iter().map(|&f| &fills[f]).filter(|fill| layers.contains(&fill.layer));
            routed
                .map(|fill| Pour {
                    piece: Piece::new(&fill.rings),
                    layer: &fill.layer,
                    outline: bounds(fill.rings.first().into_iter().flatten()),
                })
                .collect()
        })
        .collect();
    let mut out = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        let own = &usable[g];
        if group.len() == 1 && own.is_empty() {
            let pad = &pads[group[0]];
            let place = Place { xy: pad.at, layers: on(pad).into_iter().map(String::from).collect() };
            out.push(if place.layers.is_empty() { Vec::new() } else { vec![place] });
            continue;
        }
        let mut offers = Offers::default();
        let others = || (0..groups.len()).filter(move |&h| h != g);
        for other in others().flat_map(|h| &groups[h]).map(|&i| &pads[i]) {
            let mut near: Vec<(f64, usize)> =
                group.iter().filter(|&&i| !on(&pads[i]).is_empty()).map(|&i| (dist(pads[i].at, other.at), i)).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(far, i) in near.iter().take(PIECE_PADS) {
                offers.offer(far, pads[i].at, on(&pads[i]), false);
            }
        }
        // Each pour of the group against each other group: along the edges of that group's pours where it
        // has any near this one (one pour cut in two), else round the pads of it nearest this pour.
        for pour in own {
            for h in others() {
                let beside: Vec<&Pour> =
                    usable[h].iter().filter(|theirs| apart(pour.outline, theirs.outline) <= POUR_NEAR).collect();
                for theirs in &beside {
                    let outline = theirs.piece.rings.first().map_or(&[][..], |ring| &ring[..]);
                    for place in along(outline, POUR_SPREAD, POUR_EDGE) {
                        offers.offer_pour(pour, place, inset, step);
                    }
                }
                if beside.is_empty() {
                    let far =
                        |i: usize| apart(pour.outline, [pads[i].at[0], pads[i].at[1], pads[i].at[0], pads[i].at[1]]);
                    let mut nearest = groups[h].clone();
                    nearest.sort_by(|&a, &b| far(a).total_cmp(&far(b))); // (of equals the earlier in the group)
                    for &i in nearest.iter().take(POUR_PADS) {
                        for &(dx, dy) in &round_about {
                            offers.offer_pour(pour, [pads[i].at[0] + dx, pads[i].at[1] + dy], inset, step);
                        }
                    }
                }
            }
        }
        if others().all(|h| groups[h].is_empty()) {
            // (the net's only group: nothing to join it to)
            let pad = &pads[group[0]];
            let place = Place { xy: pad.at, layers: on(pad).into_iter().map(String::from).collect() };
            out.push(if place.layers.is_empty() { Vec::new() } else { vec![place] });
            continue;
        }
        out.push(offers.chosen());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYERS: [&str; 2] = ["F.Cu", "B.Cu"];

    fn land(x: f64, y: f64, layer: &str) -> Pad {
        Pad { net: Some("N".into()), at: [x, y], layers: vec![layer.into()], shapes: Default::default() }
    }

    fn pour(layer: &str, ring: Vec<Pt>) -> Fill {
        Fill { net: "N".into(), layer: layer.into(), rings: vec![ring], zone: 0 }
    }

    fn square() -> Vec<Pt> {
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
    }

    fn layers(names: &[&str]) -> Vec<String> {
        names.iter().map(|l| l.to_string()).collect()
    }

    fn on(piece: &Piece, p: Pt, inset: f64) -> Option<Pt> {
        into_pour(piece, p, inset)
    }

    #[test]
    fn a_point_on_the_pour_is_its_own_target() {
        let rings = vec![square()];
        assert_eq!(on(&Piece::new(&rings), [3.0, 3.0], 0.2), Some([3.0, 3.0]));
    }

    #[test]
    fn a_point_outside_goes_to_the_nearest_edge_and_a_little_in() {
        let rings = vec![square()];
        assert_eq!(on(&Piece::new(&rings), [12.0, 5.0], 0.2), Some([9.8, 5.0]));
        assert_eq!(on(&Piece::new(&rings), [5.0, -3.0], 0.25), Some([5.0, 0.25]));
    }

    #[test]
    fn a_point_in_a_hole_goes_out_of_the_hole() {
        // (a pad the pour keeps clear of: the hole round it)
        let rings = vec![square(), vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]]];
        assert_eq!(on(&Piece::new(&rings), [5.0, 5.5], 0.2), Some([5.0, 6.2]));
    }

    #[test]
    fn a_sliver_too_thin_is_no_target() {
        let rings = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 0.1], [0.0, 0.1]]];
        assert_eq!(on(&Piece::new(&rings), [5.0, -1.0], 0.2), None);
    }

    #[test]
    fn a_group_of_one_pad_is_one_place() {
        let pads = [land(2.0, 2.0, "F.Cu"), land(8.0, 2.0, "F.Cu")];
        let found = reach(&pads, &[], &[vec![0], vec![1]], &[], &layers(&LAYERS), 0.2, None);
        let one = |x: f64| vec![Place { xy: [x, 2.0], layers: layers(&["F.Cu"]) }];
        assert_eq!(found, [one(2.0), one(8.0)]);
    }

    #[test]
    fn a_group_of_many_pads_offers_those_nearest_the_others() {
        let mut pads: Vec<Pad> = (0..10).map(|k| land(2.0 * k as f64, 0.0, "F.Cu")).collect();
        pads.push(land(30.0, 0.0, "F.Cu"));
        let found = reach(&pads, &[], &[(0..10).collect(), vec![10]], &[], &layers(&["F.Cu"]), 0.2, None);
        assert_eq!(found[0].iter().map(|p| p.xy).collect::<Vec<_>>(), [[18.0, 0.0], [16.0, 0.0]]);
        assert_eq!(found[1], [Place { xy: [30.0, 0.0], layers: layers(&["F.Cu"]) }]);
    }

    #[test]
    fn a_pour_is_reached_near_each_pad_off_it() {
        // two pads on the pour, one beside it, one under it on the other side
        let pads = [land(2.0, 2.0, "F.Cu"), land(8.0, 8.0, "F.Cu"), land(14.0, 5.0, "F.Cu"), land(5.0, 5.0, "B.Cu")];
        let fills = [pour("F.Cu", square())];
        let groups = [vec![0, 1], vec![2], vec![3]];
        let found = reach(&pads, &fills, &groups, &[vec![0]], &layers(&LAYERS), 0.225, Some(0.05));
        assert_eq!(found[1], [Place { xy: [14.0, 5.0], layers: layers(&["F.Cu"]) }]);
        assert_eq!(found[2], [Place { xy: [5.0, 5.0], layers: layers(&["B.Cu"]) }]);
        let places: Vec<Pt> = found[0].iter().map(|p| p.xy).collect();
        assert!(places.contains(&[5.0, 5.0])); // the pour over the pad on the other side
        assert!(places.contains(&[2.0, 2.0])); // a pad of the group
        assert!(places.iter().any(|p| 9.5 < p[0] && p[0] < 10.0 && p[1] == 5.0)); // just inside the edge, beside the pad off it
        for (k, a) in found[0].iter().enumerate() {
            assert!(found[0][..k].iter().all(|b| b != a), "{a:?} twice");
        }
    }

    #[test]
    fn a_small_pour_of_another_group_is_a_place_to_reach_it() {
        let pads = [land(2.0, 2.0, "F.Cu"), land(21.0, 1.0, "F.Cu")];
        let fills = [pour("F.Cu", square()), pour("F.Cu", vec![[20.0, 0.0], [22.0, 0.0], [22.0, 2.0], [20.0, 2.0]])];
        let pours = [vec![0], vec![1]];
        let found = reach(&pads, &fills, &[vec![0], vec![1]], &pours, &layers(&LAYERS), 0.2, None);
        assert!(found[0].iter().any(|p| 9.5 < p.xy[0] && p.xy[0] < 10.0)); // the edge of the large one nearest the small
        assert!(found[1].iter().any(|p| 20.0 < p.xy[0] && p.xy[0] < 20.5)); // and of the small one nearest the large
    }

    #[test]
    fn two_pieces_of_a_cut_pour_are_offered_along_the_cut() {
        let pads = [land(2.0, 2.0, "F.Cu"), land(8.0, 8.0, "F.Cu")];
        let upper = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 4.5], [0.0, 4.5]];
        let lower = vec![[0.0, 5.5], [10.0, 5.5], [10.0, 10.0], [0.0, 10.0]];
        let fills = [pour("F.Cu", upper), pour("F.Cu", lower)];
        let pours = [vec![0], vec![1]];
        let found = reach(&pads, &fills, &[vec![0], vec![1]], &pours, &layers(&["F.Cu"]), 0.2, Some(0.05));
        let by_cut: Vec<Pt> = found[0].iter().map(|p| p.xy).filter(|p| p[1] == 4.3).collect(); // 0.2 mm in from the cut
        assert!(by_cut.len() >= 8);
        for a in &by_cut {
            assert!(by_cut.iter().all(|b| a == b || (a[0] - b[0]).abs() >= POUR_SPREAD - 1e-9));
            assert!(((a[0] / 0.05).round() * 0.05 - a[0]).abs() < 1e-9); // on the grid
        }
    }

    #[test]
    fn a_pour_on_a_layer_that_carries_no_routes_is_no_place() {
        let pads = [land(2.0, 2.0, "F.Cu"), land(14.0, 5.0, "F.Cu")];
        let fills = [pour("In1.Cu", square())];
        let found = reach(&pads, &fills, &[vec![0], vec![1]], &[vec![0]], &layers(&LAYERS), 0.2, None);
        assert_eq!(found[0], [Place { xy: [2.0, 2.0], layers: layers(&["F.Cu"]) }]);
        assert_eq!(found[1], [Place { xy: [14.0, 5.0], layers: layers(&["F.Cu"]) }]);
    }

    #[test]
    fn a_pad_on_no_routing_layer_leaves_its_group_without_a_place() {
        let pads = [land(2.0, 2.0, "In1.Cu"), land(8.0, 2.0, "F.Cu")];
        let found = reach(&pads, &[], &[vec![0], vec![1]], &[], &layers(&["F.Cu"]), 0.2, None);
        assert!(found[0].is_empty());
    }
}
