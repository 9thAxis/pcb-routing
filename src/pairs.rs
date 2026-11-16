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
const FURTHER: [f64; 5] = [1.0, 1.5, 2.0, 3.0, 4.5];

/// A trunk of a pair: the stretch laid as one, between two of its ends.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Trunk {
    /// The two ends it runs between, as indices into [`Plan::ends`].
    pub ends: [usize; 2],
    /// Where it may begin and where it may end: for each of its two ends, places on the grid a little off
    /// the pads (the grid's cells are taken to be at multiples of the step), by how far off they are: the
    /// nearest first, each half as far again, and for each distance two lots: the place square to the line
    /// of the two pads, then those beside it that lead towards the other end. They are for terminals of one
    /// piece that take their turn in that order ([`Terminal::turn`](crate::Terminal::turn)): the trunk then
    /// begins as near its pads as there is room, and the pair runs side by side for longer.
    ///
    /// A pair cannot be turned over on one layer: the leg that leaves one end on the left arrives at the
    /// other on the left. So the places of the two ends go together: a trunk from any place of the one to
    /// any place of the other brings each leg to the side of its own pad.
    pub gather: [Vec<Vec<Place>>; 2],
    /// The same from the other side of both ends: the pair leaves each end the other way and goes round. For
    /// a second try, where the first side of an end is shut in. A trunk between a place of `gather` and one
    /// of `round` would arrive with its legs changed over, and is never to be offered.
    pub round: [Vec<Vec<Place>>; 2],
    /// The ends it passes on its way from the first of `ends` to the second, in that order: it runs on
    /// between the two pads of each. None for a trunk of [`Plan::trunks`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<Pass>,
}

/// An end that a trunk passes: it runs on between the end's two pads, and each leg gets a branch to its own.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pass {
    /// The end, as an index into [`Plan::ends`].
    pub end: usize,
    /// Where the trunk passes: half way between the two pads, on the grid, on the routing layers both are on
    /// that the trunk is offered on. For a terminal with [`Terminal::pass`](crate::Terminal::pass) set.
    pub place: Place,
    /// The way a trunk that leaves its ends by [`Trunk::gather`] goes through, as a direction: square to the
    /// line of the two pads, to the side that takes each leg past its own pad.
    pub gather: Pt,
    /// And one that leaves them by [`Trunk::round`]: the other way.
    pub round: Pt,
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
    /// The trunks of a pair that may run on past an end in its middle, between that end's two pads (a part
    /// both nets pass on their way: the pair then stays side by side from its first end to its last). Where
    /// two stretches of the shortest way meet at an end that no third comes to, one trunk takes the place of
    /// both and passes that end ([`Trunk::passes`]); it is offered even where one of the two is too short for
    /// a trunk of its own. The other trunks are as in `trunks`. Empty where no end can be passed.
    ///
    /// These are instead of `trunks`, for a front end to choose: a trunk is as wide as both legs and the
    /// distance between them, and passes only where the two pads leave it that much room and its clearance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub through: Vec<Trunk>,
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
    let off = |end: usize| dist(a[ends[end][0]].at, b[ends[end][1]].at) / 2.0 + pitch;
    let middles: Vec<Pt> = ends.iter().map(|&end| middle(end)).collect();
    // The line of the two pads of an end, from the second net's to the first's.
    let across = |end: usize| {
        let (p, q) = (b[ends[end][1]].at, a[ends[end][0]].at);
        let apart = dist(p, q).max(1e-9);
        [(q[0] - p[0]) / apart, (q[1] - p[1]) / apart]
    };
    let pads = |end: usize| [&a[ends[end][0]], &b[ends[end][1]]];
    let way = |from: usize, to: usize| {
        let (p, q) = (middles[from], middles[to]);
        let apart = dist(p, q).max(1e-9);
        [(q[0] - p[0]) / apart, (q[1] - p[1]) / apart]
    };
    // An end is left with the first net's leg on the left or on the right; the other end must then be left
    // with it on the other hand, for the trunk arrives there the other way about. Of the two ways to choose,
    // the first is the one whose square places face the way the trunk sets out from either end (`next`
    // and `before`: the ends it comes to first from its first end, and last before its last).
    let first_hand = |i: usize, next: usize, before: usize, j: usize| {
        let faces = |hand: f64| {
            let (one, other) = (square(across(i), hand), square(across(j), -hand));
            let (out, back) = (way(i, next), way(j, before));
            one[0] * out[0] + one[1] * out[1] + other[0] * back[0] + other[1] * back[1]
        };
        if faces(1.0) >= faces(-1.0) {
            1.0
        } else {
            -1.0
        }
    };
    let tree = shortest_tree(&middles);
    let mut trunks = Vec::new();
    for &[i, j] in &tree {
        // Between the places where it gathers and where it parts, a trunk is as long again as both are off
        // their pads at the least: so the furthest places are offered only where the ends are far enough apart,
        // and two ends too close for the nearest are joined as the pads of any net are.
        let reach = off(i) + off(j);
        let apart = dist(middles[i], middles[j]);
        let lots: Vec<f64> = FURTHER.into_iter().filter(|further| apart >= (further + 1.0) * reach).collect();
        let on = trunk_layers(pads(i), pads(j), layers);
        if lots.is_empty() || on.is_empty() {
            continue;
        }
        let hand = first_hand(i, j, i, j);
        let gather = |end: usize, to: usize, hand: f64| {
            places(middles[end], middles[to], across(end), hand, off(end), &lots, &on, step)
        };
        trunks.push(Trunk {
            ends: [i, j],
            gather: [gather(i, j, hand), gather(j, i, -hand)],
            round: [gather(i, j, -hand), gather(j, i, hand)],
            passes: Vec::new(),
        });
    }
    // A trunk that runs on past the ends in the middle of a line of them.
    let passing = |line: &[usize]| -> Option<Trunk> {
        let (i, next, before, j) = (line[0], line[1], line[line.len() - 2], line[line.len() - 1]);
        // (on the layers of its two ends, as any trunk; and where an end it passes is on none of those, on
        // the layers of either end: it changes layer on its way)
        let on_both = |on: &[String], end: usize| on.iter().any(|l| pads(end).iter().all(|pad| pad.layers.contains(l)));
        let on_either =
            |layer: &&String| [i, j].iter().any(|&end| pads(end).iter().all(|pad| pad.layers.contains(layer)));
        let mut on = trunk_layers(pads(i), pads(j), layers);
        if !line[1..line.len() - 1].iter().all(|&end| on_both(&on, end)) {
            on = layers.iter().filter(on_either).cloned().collect();
        }
        // (an end is left no further off than leaves the trunk two pitches before the end it passes next)
        let lots = |end: usize, to: usize| -> Vec<f64> {
            let apart = dist(middles[end], middles[to]);
            FURTHER.into_iter().filter(|further| apart >= further * off(end) + 2.0 * pitch).collect()
        };
        let (near, far) = (lots(i, next), lots(j, before));
        let hand = first_hand(i, next, before, j);
        let mut passes = Vec::new();
        for &end in &line[1..line.len() - 1] {
            let both: Vec<String> =
                on.iter().filter(|layer| pads(end).iter().all(|pad| pad.layers.contains(layer))).cloned().collect();
            if both.is_empty() {
                return None;
            }
            let on_grid = |v: f64| round6((v / step).round() * step);
            passes.push(Pass {
                end,
                place: Place { xy: [on_grid(middles[end][0]), on_grid(middles[end][1])], layers: both },
                gather: square(across(end), hand),
                round: square(across(end), -hand),
            });
        }
        if near.is_empty() || far.is_empty() || on.is_empty() {
            return None;
        }
        let gather = |end: usize, to: usize, hand: f64, lots: &[f64]| {
            places(middles[end], middles[to], across(end), hand, off(end), lots, &on, step)
        };
        Some(Trunk {
            ends: [i, j],
            gather: [gather(i, next, hand, &near), gather(j, before, -hand, &far)],
            round: [gather(i, next, -hand, &near), gather(j, before, hand, &far)],
            passes,
        })
    };
    let mut through = Vec::new();
    let mut passed = false;
    for line in lines_of(&tree, ends.len()) {
        let stretches = line.windows(2).map(|w| [w[0], w[1]]);
        let own = |stretch: [usize; 2]| trunks.iter().find(|t| t.ends == stretch || t.ends == [stretch[1], stretch[0]]);
        match Some(&line).filter(|line| line.len() > 2).and_then(|line| passing(line)) {
            Some(trunk) => {
                passed = true;
                through.push(trunk);
            }
            None => through.extend(stretches.filter_map(own).cloned()),
        }
    }
    if !passed {
        through.clear();
    }
    Ok(Plan { ends, trunks, loose, through })
}

/// A tree of points (`tree`: its stretches, as pairs of indices below `count`) as its lines: each from a
/// point where the tree ends or branches, through points that two stretches meet at, to the next such point.
/// Every stretch is in one line.
fn lines_of(tree: &[[usize; 2]], count: usize) -> Vec<Vec<usize>> {
    let beside = |at: usize| tree.iter().filter_map(move |&[i, j]| (i == at).then_some(j).or((j == at).then_some(i)));
    let on_the_way = |at: usize| beside(at).count() == 2;
    let mut lines = Vec::new();
    for from in (0..count).filter(|&at| !on_the_way(at)) {
        for first in beside(from) {
            let mut line = vec![from, first];
            while on_the_way(line[line.len() - 1]) {
                let (before, at) = (line[line.len() - 2], line[line.len() - 1]);
                match beside(at).find(|&next| next != before) {
                    Some(next) => line.push(next),
                    None => break,
                }
            }
            // (each line is found from both its ends: the one from its lower end is kept)
            if line[0] < line[line.len() - 1] {
                lines.push(line);
            }
        }
    }
    lines
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

/// The direction square to the line of two pads (`across`, a unit vector) on one hand of it (1 or -1).
fn square(across: Pt, hand: f64) -> Pt {
    [-across[1] * hand, across[0] * hand]
}

/// The places a trunk may begin at, off the middle `from` of one of its ends, on one `hand` of the line of
/// its two pads (`across`: that line's direction), in lots by preference. For each of `lots`, `off` times
/// that far: first in the direction square to the pads, so that each leg comes straight to its own pad and
/// neither has to pass the other's; then, as a lot of their own, in the directions that lead towards the
/// far end (`to`) on that hand: the nearest of the eight to it and the two beside it. The directions are
/// those of the grid. On the grid of `step`, each place once.
#[allow(clippy::too_many_arguments)]
fn places(
    from: Pt,
    to: Pt,
    across: Pt,
    hand: f64,
    off: f64,
    lots: &[f64],
    on: &[String],
    step: f64,
) -> Vec<Vec<Place>> {
    let eighth = std::f64::consts::FRAC_PI_4;
    let towards = (to[1] - from[1]).atan2(to[0] - from[0]);
    let straight = (towards / eighth).round();
    let nearer = if towards >= straight * eighth { 1.0 } else { -1.0 };
    let out = square(across, hand);
    // (how far a direction leads out on this hand: 1 square to the pads, 0 along their line)
    let leads = |turns: f64| (turns * eighth).cos() * out[0] + (turns * eighth).sin() * out[1];
    let squarest = (out[1].atan2(out[0]) / eighth).round();
    let same = |one: f64, other: f64| (one - other).rem_euclid(8.0) < 1e-9;
    let beside: Vec<f64> = [straight, straight + nearer, straight - nearer]
        .into_iter()
        .filter(|&way| leads(way) > 0.3 && !same(way, squarest))
        .collect();
    let on_grid = |v: f64| round6((v / step).round() * step);
    let mut found: Vec<Vec<Place>> = Vec::new();
    for further in lots {
        for ways in [&[squarest][..], &beside[..]] {
            let mut lot: Vec<Place> = Vec::new();
            for &way in ways {
                let (angle, far) = (way * eighth, further * off);
                let xy = [on_grid(from[0] + far * angle.cos()), on_grid(from[1] + far * angle.sin())];
                if !found.iter().flatten().chain(&lot).any(|place| place.xy == xy) {
                    lot.push(Place { xy, layers: on.to_vec() });
                }
            }
            // (a lot for each, even an empty one: the lots go in twos, by distance)
            found.push(lot);
        }
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
        // the nearest place is square to the pads (here straight towards the other end too), half the pads'
        // distance and the pitch off
        assert_eq!(trunk.gather[0][0][0], Place { xy: [0.9, 0.5], layers: vec!["F".to_string()] });
        assert_eq!(trunk.gather[1][0][0].xy, [19.1, 0.5]);
        // at each of five distances: the place square to the pads, then as a lot of their own the two beside it
        assert_eq!(trunk.gather[0].iter().map(Vec::len).collect::<Vec<_>>(), [1, 2, 1, 2, 1, 2, 1, 2, 1, 2]);
        assert!(trunk.gather[0].iter().flatten().all(|p| p.xy[0] > 0.5 && p.xy[0] < 5.0));
        // the other way round both ends: away from each other
        assert_eq!((trunk.round[0][0][0].xy, trunk.round[1][0][0].xy), ([-0.9, 0.5], [20.9, 0.5]));
        assert!(
            trunk.gather[0][2][0].xy[0] > trunk.gather[0][0][0].xy[0]
                && trunk.gather[0][4][0].xy[0] > trunk.gather[0][2][0].xy[0]
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
    fn a_part_in_the_middle_of_a_pair_may_be_passed_by_one_trunk_from_end_to_end() {
        // a connector, a part 2 mm from it with its pads 1.6 mm apart, and the far end
        let a = [land(0.0, 0.0, "F"), land(2.0, -0.3, "F"), land(30.0, 0.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(2.0, 1.3, "F"), land(30.0, 1.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        assert_eq!(found.trunks.iter().map(|t| t.ends).collect::<Vec<_>>(), [[1, 2]]);
        assert_eq!(found.through.len(), 1);
        let trunk = &found.through[0];
        assert_eq!((trunk.ends, trunk.passes.len()), ([0, 2], 1));
        let pass = &trunk.passes[0];
        assert_eq!((pass.end, pass.place.xy, &pass.place.layers), (1, [2.0, 0.5], &vec!["F".to_string()]));
        // left towards the part, it is passed the same way; round the connector, the other way
        assert_eq!((trunk.gather[0][0][0].xy, pass.gather, pass.round), ([0.9, 0.5], [1.0, 0.0], [-1.0, 0.0]));
        assert!(trunk.round[0][0][0].xy[0] < 0.0 && trunk.gather[1][0][0].xy[0] < 30.0);
        // the connector is left no further off than leaves room before the part
        assert!(trunk.gather[0].iter().flatten().all(|place| place.xy[0] < 1.2), "{:?}", trunk.gather[0]);
        // two ends have nothing to pass, and a part on the other side of the board is not passed
        let found = plan(&a[..1], &b[..1], 0.4, &layers(), 0.1).unwrap();
        assert!(found.through.is_empty());
        let under = [land(0.0, 1.0, "F"), land(2.0, 1.3, "B"), land(30.0, 1.0, "F")];
        assert!(plan(&a, &under, 0.4, &layers(), 0.1).unwrap().through.is_empty());
    }

    #[test]
    fn the_lines_of_a_tree_run_between_its_ends_and_its_forks() {
        assert_eq!(lines_of(&[[0, 1], [1, 2]], 3), [vec![0, 1, 2]]);
        assert_eq!(lines_of(&[[0, 1], [0, 2]], 3), [vec![1, 0, 2]]);
        // a fork at 1: three lines from it, one of them through 3
        assert_eq!(lines_of(&[[0, 1], [1, 2], [1, 3], [3, 4]], 5), [vec![0, 1], vec![1, 2], vec![1, 3, 4]]);
        assert!(lines_of(&[], 1).is_empty());
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
    fn the_place_square_to_the_pads_comes_first_and_the_rest_lean_to_the_far_end() {
        // the pads stand one above the other; the other end lies up and to the right, nearer the diagonal
        let a = [land(0.0, 0.0, "F"), land(20.0, 14.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 15.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        let lots = &found.trunks[0].gather[0];
        // square to the pads: straight to the right, where each leg comes to its own pad
        assert_eq!(lots[0].iter().map(|place| place.xy).collect::<Vec<_>>(), [[0.9, 0.5]]);
        // then along the diagonal; not straight up, along the line of the pads and past the other's
        assert_eq!(lots[1].len(), 1);
        assert!(lots[1][0].xy[0] > 0.5 && lots[1][0].xy[1] > 0.9, "{:?}", lots[1]);
    }

    #[test]
    fn pads_that_stand_in_line_with_the_far_end_are_left_square_to_their_line() {
        // (a part turned by 45 degrees: its two pads on a diagonal, the far end to the left)
        let a = [land(20.0, 10.0, "F"), land(0.0, 10.0, "F")];
        let b = [land(20.35, 10.35, "F"), land(0.0, 11.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.05).unwrap();
        let (first, round) = (found.trunks[0].gather[0][0][0].xy, found.trunks[0].round[0][0][0].xy);
        let middle = [20.175, 10.175];
        // square to the diagonal the pads stand on: down and to the left, towards the far end; the other
        // way round, up and to the right
        assert!(first[0] < middle[0] - 0.2 && first[1] > middle[1] + 0.2, "{first:?}");
        assert!(round[0] > middle[0] + 0.2 && round[1] < middle[1] - 0.2, "{round:?}");
    }

    #[test]
    fn a_pair_whose_pads_are_changed_over_at_the_far_end_goes_round_one_of_them() {
        // the first net's pad is above at the near end and below at the far end: a trunk straight across
        // would arrive with its legs changed over
        let a = [land(0.0, 0.0, "F"), land(20.0, 1.0, "F")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 0.0, "F")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        let trunk = &found.trunks[0];
        let (near, far) = (trunk.gather[0][0][0].xy, trunk.gather[1][0][0].xy);
        // it leaves both ends to the same side: one of them is gone round
        assert!((near[0] > 0.0) == (far[0] > 20.0), "{near:?} {far:?}");
        let (near, far) = (trunk.round[0][0][0].xy, trunk.round[1][0][0].xy);
        assert!((near[0] > 0.0) == (far[0] > 20.0), "{near:?} {far:?}");
    }
}
