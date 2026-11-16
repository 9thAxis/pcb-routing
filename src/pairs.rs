//! Differential pairs for a front end: which pads of two nets face each other, and where the pair is to
//! gather and to part between them.
//!
//! The router lays a pair as one wide track, the trunk, between two points, splits it into two legs and
//! joins each leg to its pads by stubs (see [`Pair`](crate::Pair)). What a front end has is two nets and
//! their pads. [`plan`] is the step between: it finds the ends of the pair (a pad of each net side by
//! side), says between which ends a trunk is to run, and offers places for each trunk to begin and end.
//! The places of one end are the terminals of one piece ([`Terminal::piece`](crate::Terminal::piece)), the
//! nearer to the pads the earlier their turn ([`Terminal::turn`](crate::Terminal::turn)): a place that
//! turns out closed costs nothing, and the next is tried.

use crate::board::{Pad, Place};
use crate::error::{invalid, Error};
use crate::geom::dist;
use crate::grid::round6;
use crate::model::Pt;
use serde::{Deserialize, Serialize};

/// Two pads further apart than this, in mm, are not an end of a pair.
const END_REACH: f64 = 5.0;
/// A trunk is offered that many times further out from its pads, at most, than the nearest place.
const FURTHER: [f64; 3] = [1.0, 1.5, 2.0];

/// A trunk of a pair: the stretch laid as one, between two of its ends.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Trunk {
    /// The two ends it runs between, as indices into [`Plan::ends`].
    pub ends: [usize; 2],
    /// Where it may begin and where it may end: for each of its two ends, places on the grid a little off
    /// the pads towards the other end (the grid's cells are taken to be at multiples of the step), by how
    /// far off they are: the nearest first, each lot of places half as far again. They are for terminals of
    /// one piece that take their turn in that order ([`Terminal::turn`](crate::Terminal::turn)): the trunk
    /// then begins as near its pads as there is room, and the pair runs side by side for longer.
    pub gather: [Vec<Vec<Place>>; 2],
}

/// How two nets are to be laid as a pair.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Plan {
    /// The ends of the pair: a pad of the first net and a pad of the second that stand side by side, as
    /// indices into the pads of each.
    pub ends: Vec<[usize; 2]>,
    /// The trunks. The ends are joined by the shortest way, each to the nearest of those joined before it,
    /// and there is a trunk for every such stretch between two ends that are far enough apart to gather
    /// between and have a routing layer between them; two ends without one are joined as the pads of any
    /// net are. None at all: the nets are no pair to lay.
    pub trunks: Vec<Trunk>,
    /// The pads of each net that are at no end: they are joined to their net as the pads of any net are.
    pub loose: [Vec<usize>; 2],
}

/// How two nets may be laid as a pair.
///
/// A pad of each net are an end of the pair if each is the other's nearest pad of the other net and they
/// are no more than 5 mm apart. `pitch` is the distance between the legs, centre to centre; `layers` the
/// routing layers; places are on the grid of `step`. A trunk is offered on the routing layers that the
/// pads of both its ends are on, or, where there is none, on those of either: the stubs at one end then
/// change layer.
pub fn plan(a: &[Pad], b: &[Pad], pitch: f64, layers: &[String], step: f64) -> Result<Plan, Error> {
    for (field, value) in [("pitch", pitch), ("step", step)] {
        if !(value > 0.0 && value.is_finite()) {
            return Err(invalid(field, format!("is {value}: more than 0")));
        }
    }
    let ends = ends_of(a, b);
    let loose = [
        (0..a.len()).filter(|i| ends.iter().all(|end| end[0] != *i)).collect(),
        (0..b.len()).filter(|j| ends.iter().all(|end| end[1] != *j)).collect(),
    ];
    let middle = |end: [usize; 2]| between(a[end[0]].at, b[end[1]].at);
    // How far off its pads a trunk begins at the least: room for the stubs to come in at 45 degrees from the
    // pads' distance to the legs'.
    let off = |end: [usize; 2]| dist(a[end[0]].at, b[end[1]].at) / 2.0 + pitch;
    let middles: Vec<Pt> = ends.iter().map(|&end| middle(end)).collect();
    let mut trunks = Vec::new();
    for [i, j] in shortest_tree(&middles) {
        let (from, to) = (middles[i], middles[j]);
        // Between the places where it gathers and where it parts, a trunk is as long again as both are off
        // their pads at the least: so the furthest places are offered only where the ends are far enough apart,
        // and two ends too close for the nearest are joined as the pads of any net are.
        let reach = off(ends[i]) + off(ends[j]);
        let lots: Vec<f64> = FURTHER.into_iter().filter(|further| dist(from, to) >= (further + 1.0) * reach).collect();
        let on = trunk_layers([&a[ends[i][0]], &b[ends[i][1]]], [&a[ends[j][0]], &b[ends[j][1]]], layers);
        if lots.is_empty() || on.is_empty() {
            continue;
        }
        let gather = |end: usize, from: Pt, to: Pt| places(from, to, off(ends[end]), &lots, &on, step);
        trunks.push(Trunk { ends: [i, j], gather: [gather(i, from, to), gather(j, to, from)] });
    }
    Ok(Plan { ends, trunks, loose })
}

/// The point half way between two.
fn between(p: Pt, q: Pt) -> Pt {
    [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]
}

/// The ends of a pair: a pad of each net that are each the other's nearest, no more than [`END_REACH`] apart.
fn ends_of(a: &[Pad], b: &[Pad]) -> Vec<[usize; 2]> {
    let nearest = |p: &Pad, others: &[Pad]| {
        (0..others.len()).min_by(|&i, &j| dist(p.at, others[i].at).total_cmp(&dist(p.at, others[j].at)))
    };
    let faces = |i: usize, j: usize| nearest(&b[j], a) == Some(i) && dist(a[i].at, b[j].at) <= END_REACH;
    (0..a.len()).filter_map(|i| nearest(&a[i], b).filter(|&j| faces(i, j)).map(|j| [i, j])).collect()
}

/// The points joined by the shortest way: each to the nearest of those joined before it, beginning with the
/// first. As pairs of their indices, the one joined earlier first.
fn shortest_tree(points: &[Pt]) -> Vec<[usize; 2]> {
    let mut joined: Vec<usize> = (0..points.len()).take(1).collect();
    let mut tree = Vec::new();
    while joined.len() < points.len() {
        let apart = |&[i, j]: &[usize; 2]| dist(points[i], points[j]);
        // (in this order: of several that are as near, the one joined earliest, then the lowest)
        let left = || (0..points.len()).filter(|j| !joined.contains(j));
        let ways = joined.iter().flat_map(|&i| left().map(move |j| [i, j]));
        let next = ways.min_by(|x, y| apart(x).total_cmp(&apart(y)));
        let Some([i, j]) = next else { break };
        tree.push([i, j]);
        joined.push(j);
    }
    tree
}

/// The routing layers a trunk between two ends (the two pads of each) is offered on: those all four pads are
/// on; failing that, those the pads of either end are both on.
fn trunk_layers(one: [&Pad; 2], other: [&Pad; 2], layers: &[String]) -> Vec<String> {
    let of_end = |end: [&Pad; 2], layer: &String| end.iter().all(|pad| pad.layers.contains(layer));
    let both: Vec<String> = layers.iter().filter(|l| of_end(one, l) && of_end(other, l)).cloned().collect();
    if !both.is_empty() {
        return both;
    }
    layers.iter().filter(|l| of_end(one, l) || of_end(other, l)).cloned().collect()
}

/// The places a trunk may begin at, off the middle `from` of one of its ends towards the middle `to` of the
/// other: for each of `lots`, `off` times that far, in the nearest of the eight directions and the two beside
/// it (first the one on the side the true direction lies). On the grid of `step`, each place once.
fn places(from: Pt, to: Pt, off: f64, lots: &[f64], on: &[String], step: f64) -> Vec<Vec<Place>> {
    let eighth = std::f64::consts::FRAC_PI_4;
    let towards = (to[1] - from[1]).atan2(to[0] - from[0]);
    let straight = (towards / eighth).round();
    let nearer = if towards >= straight * eighth { 1.0 } else { -1.0 };
    let on_grid = |v: f64| round6((v / step).round() * step);
    let mut found: Vec<Vec<Place>> = Vec::new();
    for further in lots {
        let mut lot: Vec<Place> = Vec::new();
        for turn in [0.0, nearer, -nearer] {
            let (angle, far) = ((straight + turn) * eighth, further * off);
            let xy = [on_grid(from[0] + far * angle.cos()), on_grid(from[1] + far * angle.sin())];
            if !found.iter().flatten().chain(&lot).any(|place| place.xy == xy) {
                lot.push(Place { xy, layers: on.to_vec() });
            }
        }
        found.push(lot);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn land(x: f64, y: f64, layer: &str) -> Pad {
        Pad { net: None, at: [x, y], layers: vec![layer.into()], shapes: Default::default() }
    }

    fn layers() -> Vec<String> {
        vec!["F".to_string(), "B".to_string()]
    }

    #[test]
    fn of_ends_as_near_the_one_joined_earliest_is_joined_to() {
        let corners = [[0., 0.], [10., 0.], [10., 20.], [0., 20.]];
        assert_eq!(shortest_tree(&corners), [[0, 1], [0, 3], [3, 2]]);
    }

    #[test]
    fn two_nets_with_pads_side_by_side_at_both_ends_are_a_pair_with_one_trunk() {
        let a = [land(0.0, 0.0, "F"), land(20.0, 0.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 1.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        assert_eq!(
            (found.ends.clone(), found.trunks.len(), found.loose.clone()),
            (vec![[0, 0], [1, 1]], 1, [vec![], vec![]])
        );
        let trunk = &found.trunks[0];
        assert_eq!(trunk.ends, [0, 1]);
        // the nearest place is straight towards the other end, half the pads' distance and the pitch off
        assert_eq!(trunk.gather[0][0][0], Place { xy: [0.9, 0.5], layers: vec!["F".to_string()] });
        assert_eq!(trunk.gather[1][0][0].xy, [19.1, 0.5]);
        // three lots of three, each further off than the one before
        assert_eq!(trunk.gather[0].iter().map(Vec::len).collect::<Vec<_>>(), [3, 3, 3]);
        assert!(trunk.gather[0].iter().flatten().all(|p| p.xy[0] > 0.5 && p.xy[0] < 3.0));
        assert!(
            trunk.gather[0][1][0].xy[0] > trunk.gather[0][0][0].xy[0]
                && trunk.gather[0][2][0].xy[0] > trunk.gather[0][1][0].xy[0]
        );
    }

    #[test]
    fn a_pad_at_no_end_is_loose_and_three_ends_have_two_trunks() {
        // the first net has a pad of its own far off; both pass a part in the middle
        let a = [land(0.0, 0.0, "F"), land(20.0, 0.0, "F"), land(40.0, 0.0, "F"), land(10.0, 30.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 1.0, "F"), land(40.0, 1.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        assert_eq!(found.ends, [[0, 0], [1, 1], [2, 2]]);
        assert_eq!(found.loose, [vec![3], vec![]]);
        assert_eq!(found.trunks.iter().map(|t| t.ends).collect::<Vec<_>>(), [[0, 1], [1, 2]]);
    }

    #[test]
    fn nets_without_two_ends_or_with_ends_too_close_are_no_pair() {
        let a = [land(0.0, 0.0, "F"), land(20.0, 0.0, "F")];
        let far = [land(0.0, 1.0, "F"), land(20.0, 9.0, "F")]; // the second pads are 9 mm apart
        assert!(plan(&a, &far, 0.4, &layers(), 0.1).unwrap().trunks.is_empty());
        let near = [land(0.0, 0.0, "F"), land(2.0, 0.0, "F")];
        let beside = [land(0.0, 1.0, "F"), land(2.0, 1.0, "F")];
        let found = plan(&near, &beside, 0.4, &layers(), 0.1).unwrap();
        assert_eq!((found.ends.len(), found.trunks.len()), (2, 0));
    }

    #[test]
    fn a_short_hop_between_two_ends_is_no_trunk_and_the_long_one_beyond_it_is() {
        // a connector, a part 2 mm from it that both nets pass, and the far end
        let a = [land(0.0, 0.0, "F"), land(2.0, 0.0, "F"), land(30.0, 0.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(2.0, 1.0, "F"), land(30.0, 1.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        assert_eq!(found.ends.len(), 3);
        assert_eq!(found.trunks.iter().map(|t| t.ends).collect::<Vec<_>>(), [[1, 2]]);
    }

    #[test]
    fn the_places_of_the_two_ends_never_meet() {
        // ends 3.6 mm apart with pads 1 mm apart: twice as far off the pads, the places of both would be one
        let a = [land(0.0, 0.0, "F"), land(3.6, 0.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(3.6, 1.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        let [near, far] = &found.trunks[0].gather;
        assert!(!near.is_empty() && !far.is_empty());
        for p in near.iter().flatten() {
            assert!(far.iter().flatten().all(|q| q.xy[0] - p.xy[0] >= 1.7), "{p:?} and {far:?}");
        }
        // and a little nearer still there is no trunk at all
        let b = [land(0.0, 1.0, "F"), land(3.5, 1.0, "F")];
        let a = [land(0.0, 0.0, "F"), land(3.5, 0.0, "F")];
        assert!(plan(&a, &b, 0.4, &layers(), 0.1).unwrap().trunks.is_empty());
    }

    #[test]
    fn a_trunk_is_offered_on_the_layers_of_all_four_pads_or_of_either_end() {
        let a = [land(0.0, 0.0, "F"), land(20.0, 0.0, "B")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 1.0, "B")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        assert_eq!(found.trunks[0].gather[0][0][0].layers, ["F", "B"]);
        let inner = [land(0.0, 0.0, "In1"), land(20.0, 0.0, "In1")];
        assert!(plan(&inner, &b, 0.4, &layers(), 0.1).unwrap().trunks.is_empty()); // an end on no routing layer
        assert!(plan(&a, &b, 0.0, &layers(), 0.1).is_err());
    }

    #[test]
    fn the_places_lean_to_the_side_the_other_end_is_on() {
        // the other end lies up and to the right, nearer the diagonal than straight
        let a = [land(0.0, 0.0, "F"), land(20.0, 14.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 15.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        let first = &found.trunks[0].gather[0][0];
        assert!(first[0].xy[0] > 0.5 && first[0].xy[1] > 0.9, "{:?}", first[0]); // along the diagonal first
        assert!(first[1].xy[0] > first[2].xy[0], "{:?}", first); // then on the side the true direction lies: right before up
    }
}
