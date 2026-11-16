//! Differential pairs for a front end: which pads of two nets face each other, and where the pair is to
//! gather and to part between them.
//!
//! The router lays a pair as one wide track, the trunk, between two points, splits it into two legs and
//! joins each leg to its pads by stubs (see [`Pair`](crate::Pair)). What a front end has is two nets and
//! their pads. [`plan`] is the step between: it finds the ends of the pair (a pad of each net side by
//! side), says between which ends a trunk is to run, and offers places for each trunk to begin and end.
//! The places of one end are the terminals of one piece ([`Terminal::piece`](crate::Terminal::piece)): the
//! router takes whichever suits, and one that turns out closed costs nothing.

use crate::error::{invalid, Error};
use crate::geom::dist;
use crate::pours::{Pad, Place};
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
    /// the pads towards the other end, the nearest first.
    pub gather: [Vec<Place>; 2],
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
    // the ends: pads that are each the other's nearest
    let nearest = |p: &Pad, others: &[Pad]| {
        (0..others.len()).min_by(|&i, &j| dist(p.at, others[i].at).total_cmp(&dist(p.at, others[j].at)))
    };
    let mut ends: Vec<[usize; 2]> = Vec::new();
    for (i, pad) in a.iter().enumerate() {
        if let Some(j) = nearest(pad, b).filter(|&j| nearest(&b[j], a) == Some(i) && dist(pad.at, b[j].at) <= END_REACH)
        {
            ends.push([i, j]);
        }
    }
    let loose = [
        (0..a.len()).filter(|i| ends.iter().all(|end| end[0] != *i)).collect(),
        (0..b.len()).filter(|j| ends.iter().all(|end| end[1] != *j)).collect(),
    ];
    let middle =
        |end: [usize; 2]| [(a[end[0]].at[0] + b[end[1]].at[0]) / 2.0, (a[end[0]].at[1] + b[end[1]].at[1]) / 2.0];
    let apart = |end: [usize; 2]| dist(a[end[0]].at, b[end[1]].at);
    // (how far off the pads a trunk begins at the least: room for the stubs to come in from the pads' distance
    // to the legs' at 45 degrees)
    let off = |end: [usize; 2]| apart(end) / 2.0 + pitch;
    // the trunks: the ends joined by the shortest way, each new end to the nearest already joined
    let mut joined = vec![0usize];
    let mut between: Vec<[usize; 2]> = Vec::new();
    while joined.len() < ends.len() {
        let far = |&(i, j): &(usize, usize)| dist(middle(ends[i]), middle(ends[j]));
        let pairs = joined.iter().flat_map(|&i| (0..ends.len()).filter(|j| !joined.contains(j)).map(move |j| (i, j)));
        let (i, j) = pairs.min_by(|x, y| far(x).total_cmp(&far(y))).expect("an end is left");
        between.push([i, j]);
        joined.push(j);
    }
    let on = |pad: &Pad| -> Vec<&String> { layers.iter().filter(|l| pad.layers.contains(l)).collect() };
    let mut trunks = Vec::new();
    for [i, j] in between {
        let (from, to) = (middle(ends[i]), middle(ends[j]));
        if dist(from, to) < 2.0 * FURTHER[0] * (off(ends[i]) + off(ends[j])) {
            continue; // too close for a stretch side by side: these two ends are joined as the pads of any net are
        }
        // on the layers all four pads are on; failing that, on those of either end
        let of_end = |end: [usize; 2]| -> Vec<&String> {
            on(&a[end[0]]).into_iter().filter(|l| b[end[1]].layers.contains(l)).collect()
        };
        let common: Vec<&String> = of_end(ends[i]).into_iter().filter(|l| of_end(ends[j]).contains(l)).collect();
        let either: Vec<&String> =
            layers.iter().filter(|l| of_end(ends[i]).contains(l) || of_end(ends[j]).contains(l)).collect();
        let on_layers: Vec<String> = if common.is_empty() { either } else { common }.into_iter().cloned().collect();
        if on_layers.is_empty() {
            continue;
        }
        let places = |end: [usize; 2], from: [f64; 2], to: [f64; 2]| -> Vec<Place> {
            // towards the other end, in the nearest of the eight directions and the two beside it
            let towards = (to[1] - from[1]).atan2(to[0] - from[0]);
            let eighth = std::f64::consts::FRAC_PI_4;
            let straight = (towards / eighth).round();
            let nearer = if towards >= straight * eighth { 1.0 } else { -1.0 }; // (the side the true direction lies on)
            let mut found: Vec<Place> = Vec::new();
            for further in FURTHER {
                for turn in [0.0, nearer, -nearer] {
                    let angle = (straight + turn) * eighth;
                    let far = further * off(end);
                    let at = [from[0] + far * angle.cos(), from[1] + far * angle.sin()];
                    let xy = [
                        crate::grid::round6((at[0] / step).round() * step),
                        crate::grid::round6((at[1] / step).round() * step),
                    ];
                    if !found.iter().any(|place| place.xy == xy) {
                        found.push(Place { xy, layers: on_layers.clone() });
                    }
                }
            }
            found
        };
        trunks.push(Trunk { ends: [i, j], gather: [places(ends[i], from, to), places(ends[j], to, from)] });
    }
    Ok(Plan { ends, trunks, loose })
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
        assert_eq!(trunk.gather[0][0], Place { xy: [0.9, 0.5], layers: vec!["F".to_string()] });
        assert_eq!(trunk.gather[1][0].xy, [19.1, 0.5]);
        assert!(trunk.gather[0].len() > 3 && trunk.gather[0].iter().all(|p| p.xy[0] > 0.5 && p.xy[0] < 3.0));
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
    fn a_trunk_is_offered_on_the_layers_of_all_four_pads_or_of_either_end() {
        let a = [land(0.0, 0.0, "F"), land(20.0, 0.0, "B")];
        let b = [land(0.0, 1.0, "F"), land(20.0, 1.0, "B")];
        let found = plan(&a, &b, 0.4, &layers(), 0.1).unwrap();
        assert_eq!(found.trunks[0].gather[0][0].layers, ["F", "B"]);
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
        let first = &found.trunks[0].gather[0];
        assert!(first[0].xy[0] > 0.5 && first[0].xy[1] > 0.9, "{:?}", first[0]); // along the diagonal first
        assert!(first[1].xy[0] > first[2].xy[0], "{:?}", first); // then on the side the true direction lies: right before up
    }
}
