//! The spokes that tie a pad to its zone: how many it has, the ground a track would cut them on, and the
//! track that ties a pad with too few.

use super::joined::Touch;
use super::shape::{between, bounds, crossing, meet, rounded, sides, stroke, Piece};
use super::{Layout, Polygon, Spoked, Thermal};
use crate::geom::{dist, within};
use crate::model::Pt;
use std::collections::{BTreeSet, HashMap};

/// How far from a pad's middle a track that ties it to its zone may end, in mm.
const STUB_REACH: f64 = 3.0;
/// The directions such a track is tried in.
const STUB_DIRECTIONS: usize = 8;
/// How far apart the places are that are tried along each direction, in mm.
const STUB_STEP: f64 = 0.05;

/// The places where the edge of a piece of zone crosses the line round a pad, each once. Two of them are one
/// spoke.
fn crossings(island: &Piece, line: &[Pt]) -> usize {
    let near = bounds(line);
    let mut places: BTreeSet<(i64, i64)> = BTreeSet::new();
    // (the outline alone: a filled zone comes with its holes cut into it)
    let outline = island.rings.first().map_or(&[][..], |ring| &ring[..]);
    for (a, b) in sides(outline).filter(|(a, b)| meet(bounds([a, b]), near)) {
        for (c, d) in sides(line) {
            if let Some((t, _)) = crossing(a, b, c, d) {
                let at = between(a, b, t);
                places.insert(((at[0] * 1e4).round() as i64, (at[1] * 1e4).round() as i64));
            }
        }
    }
    places.len()
}

/// The ground a track of another net would cut the spokes of a pad on: everything within `around` of where
/// the line round the pad runs on the piece of zone.
fn ground(island: &Piece, line: &[Pt], around: f64) -> Vec<Polygon> {
    let mut out = Vec::new();
    for (a, b) in sides(line).filter(|&(a, b)| dist(a, b) > 0.0) {
        let mut cuts: Vec<f64> =
            island.sides_by(bounds([&a, &b])).filter_map(|(c, d)| crossing(a, b, c, d)).map(|(t, _)| t).collect();
        cuts.extend([0.0, 1.0]);
        cuts.sort_by(f64::total_cmp);
        for pair in cuts.windows(2).filter(|pair| pair[1] - pair[0] > 1e-9) {
            if island.contains(between(a, b, (pair[0] + pair[1]) / 2.0)) {
                let ring = stroke(between(a, b, pair[0]), between(a, b, pair[1]), around);
                out.push(vec![ring.into_iter().map(rounded).collect()]);
            }
        }
    }
    out
}

/// Every pad of the layout that a filled zone ties by spokes, with its spokes counted as KiCad's design rule
/// check counts them, in the order the pads were given.
///
/// A spoke is two crossings of the zone's edge with the line half the gap from the pad. Those of a piece of
/// the zone that joins nothing but this pad count only if the pad has no others; a track that begins by the
/// pad and ends on the zone counts as one, for a pad that has too few without. A pad the zone covers without
/// a gap, or does not reach, is not tied by spokes at all, and is not among those returned.
pub(super) fn thermals(layout: &Layout, pieces: &[Piece], touch: &[Touch]) -> Vec<Thermal> {
    let mut of_zone: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
    for (f, fill) in layout.fills.iter().enumerate() {
        of_zone.entry((fill.zone, &fill.layer)).or_default().push(f);
    }
    let thermal = |spoked: &Spoked| -> Option<Thermal> {
        let pad = &layout.pads[spoked.pad];
        let islands = of_zone.get(&(spoked.zone, spoked.layer.as_str()))?;
        let copper = pad.shapes.get(&spoked.layer).map(|polygons| bounds(polygons.iter().flatten().flatten()));
        let by_pad = copper.unwrap_or_else(|| bounds(&spoked.line));
        let (mut spokes, mut ignored) = (0u32, 0u32);
        let (mut tied, mut reach) = (Vec::new(), Vec::new());
        for &i in islands.iter().filter(|&&i| meet(pieces[i].bounds, by_pad)) {
            let across = (crossings(&pieces[i], &spoked.line) / 2) as u32;
            if across == 0 {
                continue; // not reached, or covered without a gap
            }
            if touch[i].joins_one() {
                ignored += across;
            } else {
                spokes += across;
            }
            tied.push(i);
            // (from a fifth of the gap off the pad to four fifths, and round that the gap, or the clearance
            // the zone keeps from a track if that is more)
            reach.extend(ground(&pieces[i], &spoked.line, 0.3 * spoked.gap + spoked.gap.max(spoked.clearance)));
        }
        if spokes + ignored == 0 {
            return None;
        }
        if spokes < spoked.least {
            let by_pad = |p: Pt| within(std::slice::from_ref(&spoked.line), p);
            let on_zone = |p: Pt| islands.iter().any(|&i| pieces[i].contains(p));
            let ties = |&&[a, b]: &&[Pt; 2]| (by_pad(a) && on_zone(b)) || (!by_pad(a) && by_pad(b) && on_zone(a));
            // The spokes of pieces that join this pad alone count only for a pad that has nothing else: so
            // a track on another layer, which KiCad counts, can leave a pad with fewer than without it.
            let with = |tracks: u32| if spokes + tracks == 0 { ignored } else { spokes + tracks };
            let here = spoked.tracks.iter().filter(ties).count() as u32;
            let elsewhere = spoked.elsewhere.iter().filter(ties).count() as u32;
            spokes = with(here).min(with(here + elsewhere));
        }
        let starved = spokes < spoked.least;
        // (the pieces that join more than this pad first: a track is better ended on those)
        tied.sort_by_key(|&i| touch[i].joins_one());
        let outline = |i: usize| vec![layout.fills[i].rings.first().cloned().unwrap_or_default()];
        Some(Thermal {
            net: layout.fills[islands[0]].net.clone(),
            pad: spoked.pad,
            layer: spoked.layer.clone(),
            least: spoked.least,
            spokes,
            reach,
            outline: if starved { spoked.line.clone() } else { Vec::new() },
            fill: if starved { tied.into_iter().map(outline).collect() } else { Vec::new() },
        })
    };
    use rayon::prelude::*;
    layout.spoked.par_iter().map(thermal).collect::<Vec<_>>().into_iter().flatten().collect()
}

/// For a pad with too few spokes: the points of its zone that a straight track from the pad's middle `at`
/// may end on, one per direction of eight, the nearest first. Each is `inset` into the zone from where the
/// line first meets it beyond the gap. `outline` and `fill` are the pad's [`Thermal::outline`] and
/// [`Thermal::fill`]; `tried`: points not to give again.
pub fn tie_targets(outline: &[Pt], fill: &[Polygon], at: Pt, inset: f64, tried: &[Pt]) -> Vec<Pt> {
    let line = [outline.to_vec()];
    let by_pad = |p: Pt| within(&line, p);
    let on_zone = |p: Pt| fill.iter().any(|rings| within(rings, p));
    let mut found: Vec<(f64, Pt)> = Vec::new();
    for k in 0..STUB_DIRECTIONS {
        let angle = k as f64 * 2.0 * std::f64::consts::PI / STUB_DIRECTIONS as f64;
        let along = |t: f64| rounded([at[0] + t * angle.cos(), at[1] + t * angle.sin()]);
        let mut t = STUB_STEP;
        while t + inset <= STUB_REACH {
            if !by_pad(along(t)) && [0.0, 0.5, 1.0, 2.0].iter().all(|f| on_zone(along(t + f * inset))) {
                let end = along(t + inset);
                if !tried.iter().any(|&known| dist(known, end) < 1e-6) {
                    found.push((t, end));
                }
                break;
            }
            t += STUB_STEP;
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1[0].total_cmp(&b.1[0])).then(a.1[1].total_cmp(&b.1[1])));
    found.into_iter().map(|(_, point)| point).collect()
}

/// True if a track through `points` counts as a spoke of the pad for KiCad: one of its straight pieces
/// begins within half the gap of the pad and ends on the pad's zone. `outline` and `fill` as for
/// [`tie_targets`].
pub fn ties(outline: &[Pt], fill: &[Polygon], points: &[Pt]) -> bool {
    let line = [outline.to_vec()];
    let by_pad = |p: Pt| within(&line, p);
    let on_zone = |p: Pt| fill.iter().any(|rings| within(rings, p));
    points.windows(2).any(|pair| (by_pad(pair[0]) && on_zone(pair[1])) || (by_pad(pair[1]) && on_zone(pair[0])))
}

#[cfg(test)]
mod tests {
    use super::super::{Fill, Pad};
    use super::*;

    // a pad 1 mm square about (5, 5); the line half the gap from it; a zone that begins 0.5 mm from the pad
    const LINE: [Pt; 4] = [[4.25, 4.25], [5.75, 4.25], [5.75, 5.75], [4.25, 5.75]];

    fn zone() -> Polygon {
        vec![
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]],
        ]
    }

    fn count(ring: Vec<Pt>) -> usize {
        let rings = vec![ring];
        crossings(&Piece::new(&rings), &LINE)
    }

    #[test]
    fn two_crossings_of_the_line_are_one_spoke() {
        assert_eq!(count(vec![[5.6, 4.9], [8.0, 4.9], [8.0, 5.1], [5.6, 5.1]]), 2); // from by the pad out to the right
        assert_eq!(count(zone().remove(0)), 0); // the zone's edge is nowhere near
    }

    #[test]
    fn a_crossing_at_a_corner_of_the_edge_is_counted_once() {
        assert_eq!(count(vec![[5.75, 5.0], [8.0, 4.0], [8.0, 6.0]]), 1); // its point on the line
    }

    #[test]
    fn the_targets_are_one_per_direction_just_inside_the_zone_the_nearest_first() {
        let targets = tie_targets(&LINE, &[zone()], [5.0, 5.0], 0.2, &[]);
        assert_eq!(targets.len(), 8);
        // straight out first: where the zone begins, 1 mm off (or a step further), and 0.2 mm in
        for t in &targets[..4] {
            let (dx, dy) = ((t[0] - 5.0).abs(), (t[1] - 5.0).abs());
            assert!(dx.min(dy) < 1e-9 && (1.2 - 1e-9..=1.25 + 1e-9).contains(&(dx + dy)), "{t:?}");
        }
        assert!(targets[4..].iter().all(|t| ((t[0] - 5.0).abs() - (t[1] - 5.0).abs()).abs() < 1e-6));
        // then the diagonals
    }

    #[test]
    fn a_target_tried_is_not_given_again_and_a_zone_out_of_reach_gives_none() {
        let first = tie_targets(&LINE, &[zone()], [5.0, 5.0], 0.2, &[])[0];
        let targets = tie_targets(&LINE, &[zone()], [5.0, 5.0], 0.2, &[first]);
        assert_eq!(targets.len(), 7);
        assert!(!targets.contains(&first));
        let far = vec![vec![[20.0, 0.0], [30.0, 0.0], [30.0, 10.0], [20.0, 10.0]]];
        assert!(tie_targets(&LINE, &[far], [5.0, 5.0], 0.2, &[]).is_empty());
    }

    #[test]
    fn a_zone_too_thin_to_end_a_track_on_is_no_target() {
        let thin = vec![vec![[6.0, 4.0], [6.3, 4.0], [6.3, 6.0], [6.0, 6.0]]];
        assert!(tie_targets(&LINE, &[thin], [5.0, 5.0], 0.2, &[]).is_empty());
    }

    #[test]
    fn a_track_ties_the_pad_if_one_straight_piece_goes_from_the_pad_onto_the_zone() {
        let tied = |points: &[Pt]| ties(&LINE, &[zone()], points);
        assert!(tied(&[[5.0, 5.0], [6.25, 5.0]]));
        assert!(tied(&[[6.25, 5.0], [5.0, 5.0]])); // whichever way it was laid
        assert!(tied(&[[5.0, 5.0], [5.1, 5.1], [6.5, 5.1]])); // a jog within the pad first
        assert!(!tied(&[[5.0, 5.0], [5.9, 5.0]])); // ends in the gap
        assert!(!tied(&[[5.0, 5.0], [5.9, 5.0], [5.9, 6.5]])); // a corner in the gap: neither piece does both
    }

    /// A pad at (5, 5) in a zone that keeps 0.5 mm from it but for spokes 0.4 mm wide, to the sides named:
    /// the zone with the ground round the pad as a hole, and each spoke a piece of its own from the pad to
    /// beyond the hole, with a via on it (so that it joins two things, as a spoke of the zone does).
    fn spoked_pad(sides_with_spokes: &str, tracks: Vec<[Pt; 2]>) -> Layout {
        let square = |x: f64, y: f64, half: f64| {
            vec![[x - half, y - half], [x + half, y - half], [x + half, y + half], [x - half, y + half]]
        };
        let piece = |rings: Polygon| Fill { net: "GND".into(), layer: "F.Cu".into(), rings, zone: 0 };
        let mut fills = vec![piece(vec![square(5.0, 5.0, 5.0), square(5.0, 5.0, 1.0)])];
        let mut wires = Vec::new();
        for side in sides_with_spokes.chars() {
            let (ring, via) = match side {
                'e' => (vec![[5.4, 4.8], [6.2, 4.8], [6.2, 5.2], [5.4, 5.2]], [6.1, 5.0]),
                'w' => (vec![[3.8, 4.8], [4.6, 4.8], [4.6, 5.2], [3.8, 5.2]], [3.9, 5.0]),
                'n' => (vec![[4.8, 3.8], [5.2, 3.8], [5.2, 4.6], [4.8, 4.6]], [5.0, 3.9]),
                _ => (vec![[4.8, 5.4], [5.2, 5.4], [5.2, 6.2], [4.8, 6.2]], [5.0, 6.1]),
            };
            fills.push(piece(vec![ring]));
            wires.push(super::super::Wire { net: "GND".into(), layer: None, ends: vec![via], node: None });
        }
        let shapes = HashMap::from([("F.Cu".to_string(), vec![vec![square(5.0, 5.0, 0.5)]])]);
        let pad = Pad { net: Some("GND".into()), at: [5.0, 5.0], layers: vec!["F.Cu".into()], shapes };
        let spoked = Spoked {
            pad: 0,
            layer: "F.Cu".into(),
            zone: 0,
            line: LINE.to_vec(),
            gap: 0.5,
            clearance: 0.2,
            least: 2,
            tracks,
            elsewhere: Vec::new(),
        };
        Layout { pads: vec![pad], fills, wires, spoked: vec![spoked], ..Layout::default() }
    }

    fn survey(layout: &Layout) -> Vec<Thermal> {
        let pieces: Vec<Piece> = layout.fills.iter().map(|f| Piece::new(&f.rings)).collect();
        let touch = super::super::joined::touches(layout, &pieces);
        thermals(layout, &pieces, &touch)
    }

    #[test]
    fn spokes_are_counted_and_a_pad_with_enough_has_no_outline() {
        let found = survey(&spoked_pad("ewn", vec![]));
        assert_eq!((found.len(), found[0].spokes, found[0].least), (1, 3, 2));
        assert!(found[0].outline.is_empty() && found[0].fill.is_empty());
        assert_eq!(found[0].reach.len(), 3);
    }

    #[test]
    fn the_ground_of_a_spoke_is_round_where_it_crosses_the_line() {
        let found = survey(&spoked_pad("e", vec![]));
        let on_ground = |p: Pt| found[0].reach.iter().any(|rings| within(rings, p));
        // the spoke crosses the line at x = 5.75 from y = 4.8 to 5.2; the ground reaches 0.15 + 0.5 mm from that
        assert!(on_ground([5.75, 5.0]) && on_ground([5.2, 5.0]) && on_ground([6.3, 5.0]) && on_ground([5.75, 5.8]));
        assert!(!on_ground([5.75, 5.9]) && !on_ground([6.5, 5.0]) && !on_ground([4.25, 5.0]));
    }

    #[test]
    fn a_pad_with_too_few_spokes_comes_with_its_outline_and_the_zone_it_is_tied_to() {
        let found = survey(&spoked_pad("e", vec![]));
        assert_eq!((found[0].spokes, found[0].outline.len(), found[0].fill.len()), (1, 4, 1));
    }

    #[test]
    fn a_track_from_the_pad_onto_the_zone_counts_as_a_spoke() {
        let onto = [[5.0, 5.0], [5.0, 6.5]]; // out through the gap to the zone beyond the hole
        let short = [[5.0, 5.0], [5.0, 5.9]]; // ends in the gap
        assert_eq!(survey(&spoked_pad("e", vec![onto]))[0].spokes, 2);
        assert_eq!(survey(&spoked_pad("e", vec![[onto[1], onto[0]]]))[0].spokes, 2);
        assert_eq!(survey(&spoked_pad("e", vec![short]))[0].spokes, 1);
    }

    #[test]
    fn the_spokes_of_a_piece_that_joins_the_pad_alone_count_only_if_it_has_no_others() {
        let mut layout = spoked_pad("ew", vec![]);
        layout.wires.remove(1); // the piece to the west joins nothing but the pad
        assert_eq!(survey(&layout)[0].spokes, 1);
        layout.wires.clear(); // nor the other: now they are all it has
        assert_eq!(survey(&layout)[0].spokes, 2);
    }

    #[test]
    fn a_track_on_another_layer_counts_as_kicad_counts_it_where_that_leaves_the_pad_with_fewer() {
        // two spokes of pieces that join the pad alone: they are all it has, and count
        let mut layout = spoked_pad("ew", vec![]);
        layout.wires.clear();
        assert_eq!(survey(&layout)[0].spokes, 2);
        // with a track on the other side that ends over the zone, KiCad counts that one and not the two
        layout.spoked[0].elsewhere = vec![[[5.0, 5.0], [5.0, 6.5]]];
        assert_eq!(survey(&layout)[0].spokes, 1);
        // where the pad has a spoke that counts, the track adds nothing to what is said of this layer
        let mut layout = spoked_pad("e", vec![]);
        layout.spoked[0].elsewhere = vec![[[5.0, 5.0], [5.0, 6.5]]];
        assert_eq!(survey(&layout)[0].spokes, 1);
    }

    #[test]
    fn a_pad_the_zone_does_not_reach_is_not_tied_by_spokes() {
        assert!(survey(&spoked_pad("", vec![])).is_empty());
    }
}
