//! The copper laid for an entry that was joined to a zone, as the pieces that hang together without the
//! zone, and where each of them ends on it.

use super::shape::{hang_together, head_of, micro};
use super::{End, Pad, Part};
use crate::geom::dist;
use crate::model::{Net, NetResult, Pt};
use std::collections::{BTreeSet, HashSet};

/// The copper laid for one entry as the pieces that hang together: each with its own tracks and vias, keyed
/// `KEY # n` (the entry's own key if it is all of a piece). Two tracks hang together where one ends on the
/// other on the same layer, a track and a via where the via stands on the track.
fn hanging_together(laid: &NetResult) -> Vec<Part> {
    let (tracks, vias) = (&laid.tracks, &laid.vias);
    let mut leader: Vec<usize> = (0..tracks.len() + vias.len()).collect();
    let track = |k: usize| (Some(tracks[k].layer.as_str()), &tracks[k].points[..]);
    for i in 0..tracks.len() {
        for j in (0..i).filter(|&j| hang_together(track(i), track(j))) {
            let (a, b) = (head_of(&leader, i), head_of(&leader, j));
            leader[a] = b;
        }
        for j in (0..vias.len()).filter(|&j| hang_together(track(i), (None, &vias[j..=j]))) {
            let (a, b) = (head_of(&leader, i), head_of(&leader, tracks.len() + j));
            leader[a] = b;
        }
    }
    let heads: BTreeSet<usize> = (0..leader.len()).map(|i| head_of(&leader, i)).collect();
    let whole = |key: String, keep: &dyn Fn(usize) -> bool| Part {
        key,
        of: laid.key.clone(),
        net: laid.net.clone(),
        tracks: (0..tracks.len()).filter(|&i| keep(i)).map(|i| tracks[i].clone()).collect(),
        vias: (0..vias.len()).filter(|&j| keep(tracks.len() + j)).map(|j| vias[j]).collect(),
        ends: Vec::new(),
    };
    if heads.len() < 2 {
        return vec![whole(laid.key.clone(), &|_| true)];
    }
    heads
        .iter()
        .enumerate()
        .map(|(n, &head)| whole(format!("{} # {}", laid.key, n + 1), &|i| head_of(&leader, i) == head))
        .collect()
}

/// What a run laid, as what can be taken back piece by piece: the copper of every entry that has points of a
/// zone among its terminals (see [`crate::Terminal::piece`]) as the pieces that hang together, each with the
/// points of a zone it came to end on; the copper of every other entry whole.
///
/// A track that ends on a point of a zone, or a via there, is joined to its net only as long as the zone is
/// there: where the tracks laid with it have cut that piece of the zone away, the part is loose. A terminal
/// at a pad of the net on that layer is no point of a zone; one under a pad that is on another layer is.
///
/// `entries` are the entries of the problem that was run, `laid` its solution's nets, `pads` the pads of the
/// board (their nets, places and layers). What was laid for a [`Pair`](crate::Pair) is under a key of its
/// own and comes back whole.
pub fn parts(pads: &[Pad], entries: &[Net], laid: &[NetResult]) -> Vec<Part> {
    let at_pads: HashSet<(&str, (i64, i64), &str)> = pads
        .iter()
        .filter_map(|pad| pad.net.as_deref().map(|net| (pad, net)))
        .flat_map(|(pad, net)| pad.layers.iter().map(move |layer| (net, micro(pad.at), layer.as_str())))
        .collect();
    let mut out = Vec::new();
    for copper in laid {
        let entry = entries.iter().find(|entry| entry.key == copper.key);
        let Some(entry) = entry.filter(|entry| entry.terminals.iter().any(|t| t.piece.is_some())) else {
            out.push(Part {
                key: copper.key.clone(),
                of: copper.key.clone(),
                net: copper.net.clone(),
                tracks: copper.tracks.clone(),
                vias: copper.vias.clone(),
                ends: Vec::new(),
            });
            continue;
        };
        for mut part in hanging_together(copper) {
            let ended_at = |layer: &str, at: Pt| {
                let tracks = part.tracks.iter().filter(|track| track.layer == layer && !track.points.is_empty());
                let mut ends = tracks.flat_map(|track| [track.points[0], track.points[track.points.len() - 1]]);
                ends.any(|p| dist(at, p) < 1e-6) || part.vias.iter().any(|&p| dist(at, p) < 1e-6)
            };
            let mut ends = Vec::new();
            for terminal in entry.terminals.iter().filter(|t| t.piece.is_some()) {
                for layer in &terminal.layers {
                    if !at_pads.contains(&(entry.net.as_str(), micro(terminal.xy), layer.as_str()))
                        && ended_at(layer, terminal.xy)
                    {
                        ends.push(End { net: entry.net.clone(), layer: layer.clone(), xy: terminal.xy });
                    }
                }
            }
            part.ends = ends;
            out.push(part);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Terminal, Track};

    fn laid(tracks: &[(&str, &[Pt])], vias: &[Pt]) -> NetResult {
        let tracks: Vec<serde_json::Value> =
            tracks.iter().map(|(layer, points)| serde_json::json!({"layer": layer, "points": points})).collect();
        serde_json::from_value(
            serde_json::json!({"key": "GND", "net": "GND", "length_mm": 0.0, "tracks": tracks, "vias": vias}),
        )
        .unwrap()
    }

    fn entry(terminals: &[(Pt, &str, Option<u32>)]) -> Net {
        let mut entry: Net = serde_json::from_value(serde_json::json!({
            "key": "GND", "net": "GND", "width": 0.25, "via": [0.6, 0.3], "layers": ["F.Cu", "B.Cu"],
            "clearance": {"default": 0.2}, "terminals": []
        }))
        .unwrap();
        entry.terminals =
            terminals.iter().map(|&(xy, layer, piece)| Terminal { xy, layers: vec![layer.into()], piece }).collect();
        entry
    }

    fn land(x: f64, y: f64) -> Pad {
        Pad { net: Some("GND".into()), at: [x, y], layers: vec!["F.Cu".into()], shapes: Default::default() }
    }

    fn layers(part: &Part) -> Vec<&str> {
        part.tracks.iter().map(|track: &Track| track.layer.as_str()).collect()
    }

    #[test]
    fn copper_all_of_a_piece_is_one_part_under_the_entrys_own_key() {
        // a branch from the middle of the first
        let copper = laid(&[("F.Cu", &[[0., 0.], [5., 0.]]), ("F.Cu", &[[2., 0.], [2., 3.]])], &[]);
        let found = hanging_together(&copper);
        assert_eq!(
            (found.len(), found[0].key.as_str(), found[0].of.as_str(), found[0].tracks.len()),
            (1, "GND", "GND", 2)
        );
    }

    #[test]
    fn two_stubs_that_do_not_touch_are_two_parts() {
        let copper = laid(
            &[("F.Cu", &[[0., 0.], [5., 0.]]), ("F.Cu", &[[0., 4.], [5., 4.]]), ("B.Cu", &[[5., 4.], [5., 9.]])],
            &[[5., 4.]],
        );
        let found = hanging_together(&copper);
        assert_eq!(
            (found[0].key.as_str(), found[0].of.as_str(), layers(&found[0]), found[0].vias.len()),
            ("GND # 1", "GND", vec!["F.Cu"], 0)
        );
        assert_eq!(
            (found[1].key.as_str(), layers(&found[1]), &found[1].vias),
            ("GND # 2", vec!["F.Cu", "B.Cu"], &vec![[5., 4.]])
        ); // joined by the via
    }

    #[test]
    fn a_via_in_the_middle_of_a_track_joins_it() {
        let copper = laid(&[("F.Cu", &[[0., 0.], [5., 0.], [10., 0.]]), ("B.Cu", &[[5., 0.], [5., 5.]])], &[[5., 0.]]);
        assert_eq!(hanging_together(&copper).len(), 1);
    }

    #[test]
    fn tracks_that_cross_on_different_layers_are_apart() {
        let copper = laid(&[("F.Cu", &[[0., 0.], [5., 0.]]), ("B.Cu", &[[5., 0.], [5., 5.]])], &[]);
        assert_eq!(hanging_together(&copper).len(), 2);
    }

    #[test]
    fn a_point_of_a_zone_under_a_pad_on_the_other_side_is_an_end_on_the_zone() {
        let pads = [land(0., 0.), land(5., 0.)]; // both on F.Cu
        let entries = [entry(&[([0., 0.], "F.Cu", None), ([5., 0.], "F.Cu", Some(0)), ([5., 0.], "B.Cu", Some(0))])];
        let on_the_pad = laid(&[("F.Cu", &[[0., 0.], [5., 0.]])], &[]);
        let under_it = laid(&[("B.Cu", &[[0., 0.], [5., 0.]])], &[[0., 0.]]);
        assert!(parts(&pads, &entries, &[on_the_pad])[0].ends.is_empty());
        let found = parts(&pads, &entries, &[under_it]);
        assert_eq!(found[0].ends, [End { net: "GND".into(), layer: "B.Cu".into(), xy: [5., 0.] }]);
    }

    #[test]
    fn the_ends_on_a_zone_are_found_per_part() {
        let pads = [land(0., 0.), land(0., 4.)];
        let terminals = [
            ([0., 0.], "F.Cu", None),
            ([0., 4.], "F.Cu", None),
            ([5., 0.], "F.Cu", Some(0)),
            ([5., 9.], "B.Cu", Some(0)),
        ];
        let copper = laid(
            &[("F.Cu", &[[0., 0.], [5., 0.]]), ("F.Cu", &[[0., 4.], [5., 4.]]), ("B.Cu", &[[5., 4.], [5., 9.]])],
            &[[5., 4.]],
        );
        let found = parts(&pads, &[entry(&terminals)], &[copper]);
        let end = |layer: &str, xy: Pt| vec![End { net: "GND".into(), layer: layer.into(), xy }];
        assert_eq!((found[0].key.as_str(), &found[0].ends), ("GND # 1", &end("F.Cu", [5., 0.])));
        assert_eq!((found[1].key.as_str(), &found[1].ends), ("GND # 2", &end("B.Cu", [5., 9.])));
    }

    #[test]
    fn an_entry_without_points_of_a_zone_is_one_part_whatever_it_laid() {
        let copper = laid(&[("F.Cu", &[[0., 0.], [5., 0.]]), ("F.Cu", &[[0., 4.], [5., 4.]])], &[]);
        let found = parts(&[], &[entry(&[([0., 0.], "F.Cu", None), ([5., 4.], "F.Cu", None)])], &[copper]);
        assert_eq!((found.len(), found[0].key.as_str(), found[0].tracks.len()), (1, "GND", 2));
    }
}
