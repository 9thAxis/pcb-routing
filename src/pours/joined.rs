//! What each piece of a filled zone touches, and the groups of pads that the zones join.

use super::shape::Piece;
use super::{Layout, Wire};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What a piece of a filled zone touches of its own net, as indices into the layout.
#[derive(Default, Clone, Debug, PartialEq)]
pub(super) struct Touch {
    /// The pads whose copper on its layer it has ground in common with (through thermal spokes too; a pad it
    /// keeps clear of is not among them).
    pub pads: Vec<usize>,
    /// The vias that stand on it and the tracks on its layer that end on it.
    pub wires: Vec<usize>,
}

impl Touch {
    /// True if the piece joins one thing at most: it holds nothing together.
    pub fn joins_one(&self) -> bool {
        self.pads.len() + self.wires.len() < 2
    }
}

/// What every piece of fill touches, in the order of the fills.
pub(super) fn touches(layout: &Layout, pieces: &[Piece]) -> Vec<Touch> {
    let mut pads_of: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, pad) in layout.pads.iter().enumerate() {
        if let Some(net) = &pad.net {
            pads_of.entry(net).or_default().push(i);
        }
    }
    let mut wires_of: HashMap<&str, Vec<usize>> = HashMap::new();
    for (w, wire) in layout.wires.iter().enumerate() {
        wires_of.entry(&wire.net).or_default().push(w);
    }
    let on = |wire: &Wire, layer: &str| wire.layer.as_deref().map_or(true, |own| own == layer);
    let touch = |(fill, piece): (&super::Fill, &Piece)| Touch {
        pads: pads_of.get(fill.net.as_str()).map_or(Vec::new(), |pads| {
            let touched = |&&i: &&usize| {
                let pad = &layout.pads[i];
                let shapes = pad.shapes.get(&fill.layer).filter(|_| pad.layers.contains(&fill.layer));
                shapes.is_some_and(|polygons| polygons.iter().any(|polygon| piece.touches(polygon)))
            };
            pads.iter().filter(touched).copied().collect()
        }),
        wires: wires_of.get(fill.net.as_str()).map_or(Vec::new(), |wires| {
            let stands = |&&w: &&usize| {
                let wire = &layout.wires[w];
                on(wire, &fill.layer) && wire.ends.iter().any(|&end| piece.contains(end))
            };
            wires.iter().filter(stands).copied().collect()
        }),
    };
    use rayon::prelude::*;
    layout.fills.par_iter().zip(pieces).map(touch).collect()
}

/// The groups of pads of each net as the zones join them, and the pieces of zone that join each group.
pub(super) type Joined = (BTreeMap<String, Vec<Vec<usize>>>, BTreeMap<String, Vec<Vec<usize>>>);

/// The groups of the layout with those that a filled zone joins made one, and for each net with zones the
/// pieces that join each of its groups (a piece that joins no pad is nobody's).
///
/// A piece joins the groups of the pads it touches and of the vias and track ends that stand on it; copper
/// that reaches no pad counts as well, where it joins two pieces of zone.
pub(super) fn joined(layout: &Layout, touch: &[Touch]) -> Joined {
    let (mut groups, mut pours) = (BTreeMap::new(), BTreeMap::new());
    for (net, own) in &layout.groups {
        let fills: Vec<usize> = (0..layout.fills.len()).filter(|&f| &layout.fills[f].net == net).collect();
        if fills.is_empty() {
            groups.insert(net.clone(), own.clone());
            continue;
        }
        // what a piece can join: the groups of pads, then the copper that reaches no pad
        let group_of: HashMap<usize, usize> =
            own.iter().enumerate().flat_map(|(g, group)| group.iter().map(move |&pad| (pad, g))).collect();
        let hangs_on = |w: usize| layout.wires[w].node;
        let count = fills.iter().flat_map(|&f| touch[f].wires.iter().filter_map(|&w| hangs_on(w))).max();
        let mut leader: Vec<usize> = (0..own.len().max(count.map_or(0, |most| most + 1))).collect();
        fn find(leader: &[usize], mut g: usize) -> usize {
            while leader[g] != g {
                g = leader[g];
            }
            g
        }
        let mut joining: Vec<(usize, usize)> = Vec::new(); // (something it joins, the piece)
        for &f in &fills {
            let by_pads = touch[f].pads.iter().filter_map(|pad| group_of.get(pad).copied());
            let touched: BTreeSet<usize> = by_pads.chain(touch[f].wires.iter().filter_map(|&w| hangs_on(w))).collect();
            let Some(&least) = touched.first() else { continue };
            let first = find(&leader, least);
            for &g in &touched {
                let head = find(&leader, g);
                leader[head] = first;
            }
            joining.push((first, f));
        }
        let heads: Vec<usize> = (0..own.len()).map(|g| find(&leader, g)).collect::<BTreeSet<_>>().into_iter().collect();
        let pads_under = |head: usize| -> Vec<usize> {
            (0..own.len()).filter(|&g| find(&leader, g) == head).flat_map(|g| own[g].iter().copied()).collect()
        };
        groups.insert(net.clone(), heads.iter().map(|&head| pads_under(head)).collect());
        let mut of_group: Vec<Vec<usize>> = vec![Vec::new(); heads.len()];
        for (g, f) in joining {
            if let Ok(group) = heads.binary_search(&find(&leader, g)) {
                of_group[group].push(f);
            }
        }
        if of_group.iter().any(|own| !own.is_empty()) {
            pours.insert(net.clone(), of_group);
        }
    }
    (groups, pours)
}

#[cfg(test)]
mod tests {
    use super::super::{Fill, Pad};
    use super::*;
    use crate::model::Pt;

    fn square(x: f64, y: f64, half: f64) -> Vec<Pt> {
        vec![[x - half, y - half], [x + half, y - half], [x + half, y + half], [x - half, y + half]]
    }

    fn land(net: &str, x: f64, y: f64) -> Pad {
        let shapes = HashMap::from([("F.Cu".to_string(), vec![vec![square(x, y, 0.5)]])]);
        Pad { net: Some(net.into()), at: [x, y], layers: vec!["F.Cu".into()], shapes }
    }

    fn fill(net: &str, layer: &str, ring: Vec<Pt>) -> Fill {
        Fill { net: net.into(), layer: layer.into(), rings: vec![ring], zone: 0 }
    }

    fn survey(layout: &Layout) -> (Vec<Touch>, Joined) {
        let pieces: Vec<Piece> = layout.fills.iter().map(|f| Piece::new(&f.rings)).collect();
        let touch = touches(layout, &pieces);
        let joined = joined(layout, &touch);
        (touch, joined)
    }

    /// Four pads of GND in a row, 4 mm apart, each a group of its own, and one of another net.
    fn row() -> Layout {
        let mut pads: Vec<Pad> = (0..4).map(|k| land("GND", 2.0 + 4.0 * k as f64, 2.0)).collect();
        pads.push(land("SIG", 6.0, 2.0));
        Layout {
            pads,
            groups: BTreeMap::from([("GND".to_string(), vec![vec![0], vec![1], vec![2], vec![3]])]),
            ..Layout::default()
        }
    }

    #[test]
    fn a_piece_joins_the_pads_it_touches_of_its_own_net_on_its_layer() {
        let mut layout = row();
        layout.fills = vec![fill("GND", "F.Cu", vec![[0.0, 0.0], [8.0, 0.0], [8.0, 4.0], [0.0, 4.0]])];
        let (touch, (groups, pours)) = survey(&layout);
        assert_eq!(touch[0].pads, [0, 1]); // (not the pad of the other net that lies on it)
        assert_eq!(groups["GND"], [vec![0, 1], vec![2], vec![3]]);
        assert_eq!(pours["GND"], [vec![0], vec![], vec![]]);
        // on another layer it touches none of them
        layout.fills[0].layer = "B.Cu".into();
        let (touch, (groups, pours)) = survey(&layout);
        assert!(touch[0].pads.is_empty());
        assert_eq!(groups["GND"].len(), 4);
        assert!(pours.is_empty());
    }

    #[test]
    fn two_pieces_are_one_group_through_a_via_that_stands_on_both() {
        let mut layout = row();
        layout.fills = vec![
            fill("GND", "F.Cu", vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]), // round the first pad
            fill("GND", "B.Cu", vec![[0.0, 5.0], [20.0, 5.0], [20.0, 9.0], [0.0, 9.0]]), // below, touching no pad
            fill("GND", "F.Cu", vec![[12.0, 0.0], [16.0, 0.0], [16.0, 9.0], [12.0, 9.0]]), // round the last
        ];
        // copper that reaches no pad: a via on the first and the second piece, and one on the second and third
        let via =
            |x: f64, y: f64, node: usize| Wire { net: "GND".into(), layer: None, ends: vec![[x, y]], node: Some(node) };
        layout.wires = vec![via(1.0, 3.5, 4), via(1.0, 6.0, 4), via(14.0, 7.0, 5)];
        // (the first via is of a track that runs down to the second: one piece of loose copper)
        let (touch, (groups, pours)) = survey(&layout);
        assert_eq!(touch[1].wires, [1, 2]);
        assert_eq!(groups["GND"], [vec![1], vec![2], vec![0, 3]]);
        assert_eq!(pours["GND"], [vec![], vec![], vec![0, 1, 2]]);
    }

    #[test]
    fn a_track_end_counts_on_its_own_layer_only_and_a_piece_that_joins_no_pad_is_nobodys() {
        let mut layout = row();
        layout.fills = vec![fill("GND", "B.Cu", vec![[0.0, 5.0], [20.0, 5.0], [20.0, 9.0], [0.0, 9.0]])];
        let track = |layer: &str| Wire {
            net: "GND".into(),
            layer: Some(layer.into()),
            ends: vec![[2.0, 2.0], [2.0, 6.0]],
            node: Some(0),
        };
        layout.wires = vec![track("F.Cu")];
        let (touch, (_, pours)) = survey(&layout);
        assert!(touch[0].wires.is_empty() && touch[0].joins_one());
        assert!(pours.is_empty());
        layout.wires = vec![track("B.Cu")];
        let (touch, (groups, pours)) = survey(&layout);
        assert_eq!(touch[0].wires, [0]);
        assert_eq!(groups["GND"].len(), 4); // it joins the first pad's group to nothing else
        assert_eq!(pours["GND"], [vec![0], vec![], vec![], vec![]]);
    }

    #[test]
    fn a_net_without_zones_keeps_its_groups() {
        let mut layout = row();
        layout.groups.insert("SIG".into(), vec![vec![4]]);
        layout.fills = vec![fill("GND", "F.Cu", vec![[0.0, 0.0], [8.0, 0.0], [8.0, 4.0], [0.0, 4.0]])];
        let (_, (groups, _)) = survey(&layout);
        assert_eq!(groups["SIG"], [vec![4]]);
    }
}
