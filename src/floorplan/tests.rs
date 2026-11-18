use super::constants::*;
use super::settle::{quartered, settled, spread, strip_between};
use super::*;
use crate::geom::dist;
use crate::model::Pt;
use std::collections::BTreeMap;

fn ties(of: &[(usize, usize, f64)]) -> BTreeMap<(usize, usize), f64> {
    of.iter().map(|&(a, b, w)| ((a, b), w)).collect()
}

#[test]
fn a_ground_is_a_net_named_as_one() {
    for name in ["GND", "VIB_GND", "FIELD_0V", "AGND", "DGND1", "/power/GND", "VSS", "HOST_GND"] {
        assert!(is_ground(name), "{name}");
    }
    for name in ["VIB_3V3", "GNDX", "SIGNAL", "10V", "AGNDA", "GROUNDED", "V0V1"] {
        assert!(!is_ground(name), "{name}");
    }
}

#[test]
fn nodes_tied_more_within_than_without_are_a_group() {
    // two triangles joined by one weak tie
    let two = ties(&[(0, 1, 1.0), (0, 2, 1.0), (1, 2, 1.0), (3, 4, 1.0), (3, 5, 1.0), (4, 5, 1.0), (2, 3, 0.2)]);
    assert_eq!(communities(&[0, 1, 2, 3, 4, 5], &two), [vec![0, 1, 2], vec![3, 4, 5]]);
    assert_eq!(communities(&[5, 4, 3, 2, 1, 0], &two), [vec![5, 4, 3], vec![2, 1, 0]]);
    // nothing tied: every node by itself; and one round gathers what is tied
    assert_eq!(communities(&[7, 8], &BTreeMap::new()), [vec![7], vec![8]]);
    assert_eq!(gathered(&[vec![0], vec![1], vec![2]], &ties(&[(0, 1, 1.0)])), [vec![0, 1], vec![2]]);
}

#[test]
fn things_on_a_line_stand_in_the_order_they_want_none_over_another() {
    // three that want one place stand round it; two at the end are pushed in; too many stick out evenly
    assert_eq!(spread(&[5.0, 5.0, 5.0], &[2.0, 2.0, 2.0], 0.0, 20.0), [3.0, 5.0, 7.0]);
    assert_eq!(spread(&[1.0, 1.0], &[4.0, 4.0], 0.0, 20.0), [2.0, 6.0]);
    assert_eq!(spread(&[15.0, 3.0], &[2.0, 2.0], 0.0, 20.0), [15.0, 3.0]);
    assert_eq!(spread(&[1.0, 2.0, 3.0], &[4.0, 4.0, 4.0], 0.0, 10.0), [1.0, 5.0, 9.0]);
    assert!(spread(&[], &[], 0.0, 1.0).is_empty());
}

#[test]
fn ground_where_no_part_may_stand_is_no_room_when_a_board_is_cut() {
    // the left quarter of the board is shut: the first domain has its share (90 in 145) of what is left
    let mut board = two_grounds();
    board.keepouts = vec![[0.0, 0.0, 10.0, 10.0]];
    let found = floorplan(&board).unwrap();
    let (a, field) = (found.zones[0].rect, found.zones[1].rect);
    let free = |r: [f64; 4]| (r[2] - r[0].max(10.0)) * 10.0;
    assert!((free(a) / (free(a) + free(field)) - 90.0 / 145.0).abs() < 0.01, "{a:?} {field:?}");
}

#[test]
fn a_supply_is_poured_on_the_plane_under_the_zones_it_feeds_and_takes_no_room_in_the_road_between_them() {
    // the regulator on a sheet of its own: the 3V3 it makes reaches two zones, its input stays within one
    let mut board = two_grounds();
    for part in board.parts.iter_mut().filter(|part| ["U2", "C4", "C5"].contains(&part.name.as_str())) {
        part.sheet = "regulator".to_string();
    }
    let within = |found: &Floorplan| found.roads.iter().find(|road| !road.gap).map(|road| road.rails);
    // with no plane to pour it on, it crosses the road between the two as a track
    let plain = floorplan(&board).unwrap();
    assert_eq!((within(&plain), plain.pours.len()), (Some(1), 0));
    board.stack = square().stack;
    let found = floorplan(&board).unwrap();
    // (the supply of the other ground reaches the isolator on the gap: poured too, under its one zone)
    let [pour, other] = &found.pours[..] else { panic!("{:?}", found.pours) };
    assert_eq!((other.net.as_str(), other.rects.len(), other.pieces), ("B_VCC", 1, 1));
    assert_eq!((pour.net.as_str(), pour.ground.as_str(), &pour.layers), ("A_3V3", "A_GND", &vec!["In4".to_string()]));
    assert_eq!((pour.rects.len() >= 2, pour.parts, pour.pieces, pour.by_track.len()), (true, 5, 1, 0));
    assert_eq!((within(&found), found.travel.split), (Some(0), 0));
    // within a zone a bypass capacitor stands by the chip it is for: nearer it than the zone's other parts are
    let at = |name: &str| found.places.iter().find(|place| place.part == name).unwrap().at;
    assert!(
        dist(at("C4"), at("U2")) < 6.0 && dist(at("C1"), at("U1")) < dist(at("C1"), at("R1")) + 6.0,
        "{:?}",
        found.places
    );
    // the isolator has its place on the gap, with its body there: across the gap, on both grounds
    let isolator = found.places.iter().find(|place| place.part == "U3").unwrap();
    assert_eq!(isolator.at, found.bridges[0].at);
    let (body, gap) = (isolator.rect.unwrap(), found.roads.iter().find(|road| road.gap).unwrap().rect);
    assert!(body[0] < gap[2] && body[2] > gap[0], "{body:?} {gap:?}");
    assert_eq!(found.places.len(), 14);
    // every layer change planned where roads meet is told with the ground its via wants
    let square = floorplan(&square()).unwrap();
    // and every part has a place within the ground of its zone, none over another
    assert_eq!(square.places.len(), 4);
    let grounds: Vec<[f64; 4]> = square.places.iter().filter_map(|place| place.rect).collect();
    for (i, a) in grounds.iter().enumerate() {
        for b in &grounds[i + 1..] {
            assert!(a[2].min(b[2]) - a[0].max(b[0]) < 1e-6 || a[3].min(b[3]) - a[1].max(b[1]) < 1e-6, "{a:?} {b:?}");
        }
    }
    assert!(square.places.iter().all(|place| place.fits && place.off.is_some()), "{:?}", square.places);
    let changes: usize = square.junctions.iter().map(|junction| junction.changes).sum();
    assert!(changes <= square.travel.vias, "{:?} {:?}", square.junctions, square.travel);
    for junction in &square.junctions {
        assert_eq!(junction.needs, 2.0 * junction.changes as f64);
        assert_eq!(junction.side > 0.0, junction.needs > junction.has, "{junction:?}");
    }
}

#[test]
fn bodies_are_settled_none_over_another_as_near_what_they_want_as_may_be() {
    let ground = [0.0, 0.0, 10.0, 4.0];
    // two that want the same place: the larger has it, the other stands beside
    let found = settled(ground, &[([2.0, 2.0], [5.0, 2.0]), ([4.0, 2.0], [5.0, 2.0])], &[]);
    assert_eq!(found[1], Some(([5.0, 2.0], false)));
    let (beside, _) = found[0].unwrap();
    assert!((beside[0] - 5.0).abs() >= 3.0 - 1e-9 && beside[1] == 2.0, "{beside:?}");
    // one too long for the ground as it stands is turned; one too large either way has no place
    assert_eq!(settled([0.0, 0.0, 3.0, 10.0], &[([8.0, 2.0], [1.5, 5.0])], &[]), [Some(([1.5, 5.0], true))]);
    assert_eq!(settled(ground, &[([12.0, 5.0], [5.0, 2.0])], &[]), [None]);
    // and none stands on ground that is shut
    let found = settled(ground, &[([2.0, 2.0], [1.0, 1.0])], &[[0.0, 0.0, 4.0, 4.0]]);
    assert_eq!(found, [Some(([5.0, 1.0], false))]);
}

#[test]
fn a_value_says_what_a_capacitor_holds_and_a_place_is_turned_by_quarters() {
    let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < b * 1e-9);
    assert!(near(farads("100n"), 100e-9) && near(farads("4u7 16V"), 4.7e-6) && near(farads("10uF"), 10e-6));
    assert!(near(farads("22 pF"), 22e-12) && near(farads("2.2µF X7R"), 2.2e-6));
    assert_eq!((farads("10k"), farads("BOARD LINK"), farads("")), (None, None, None));
    // (east, turned a quarter counter-clockwise as the board is looked at, is north: y runs down)
    assert_eq!(
        (quartered(1, [1.0, 0.0]), quartered(2, [1.0, 0.5]), quartered(3, [1.0, 0.0])),
        ([0.0, -1.0], [-1.0, -0.5], [0.0, 1.0])
    );
}

#[test]
fn a_part_is_turned_so_its_pads_face_what_they_join_and_a_bypass_capacitor_stands_by_its_pad() {
    // the chip's supply pad is on its west side; its capacitors have their supply pad first
    let mut board = two_grounds();
    board.outline = [0.0, 0.0, 60.0, 30.0];
    for part in board.parts.iter_mut() {
        match part.name.as_str() {
            "U1" => {
                (part.size, part.area) = (Some([5.0, 5.0]), 25.0);
                part.offsets = vec![[-2.0, 0.0], [2.0, 0.0], [0.0, 2.0], [0.0, -2.0]];
            }
            "C1" | "C2" => {
                (part.size, part.area) = (Some([2.0, 1.0]), 2.0);
                part.offsets = vec![[-0.5, 0.0], [0.5, 0.0]];
                part.value = if part.name == "C1" { "100n" } else { "4u7" }.to_string();
            }
            _ => {}
        }
    }
    let found = floorplan(&board).unwrap();
    let place = |name: &str| found.places.iter().find(|place| place.part == name).unwrap();
    let chip = place("U1");
    let pad = {
        let from = quartered((chip.turn / 90) as u8, [-2.0, 0.0]);
        [chip.at[0] + from[0], chip.at[1] + from[1]]
    };
    // (the chips on the supply take its capacitors in turn: the first is U1's, and stands by its supply pad)
    assert!(
        found.bypass[0] >= 1 && found.bypass[0] == found.bypass[1],
        "{:?} {chip:?} {:?}",
        found.bypass,
        place("C1")
    );
    // with its own supply pad toward the chip's: nearer it than its ground pad is
    let cap = place("C1");
    let end = |offset: Pt| {
        let from = quartered((cap.turn / 90) as u8, offset);
        dist([cap.at[0] + from[0], cap.at[1] + from[1]], pad)
    };
    assert!(end([-0.5, 0.0]) <= BESIDE_PIN && end([-0.5, 0.0]) < end([0.5, 0.0]), "{cap:?} {pad:?}");
    assert!(found.pads_mm <= found.pads_first_mm);
}

#[test]
fn a_switching_regulator_is_found_by_its_inductor_and_its_loop_stands_by_it() {
    // the regulator of the two grounds made a buck: a switch node to an inductor, the inductor to the 3V3
    let mut board = two_grounds();
    board.outline = [0.0, 0.0, 80.0, 40.0];
    board.nets.push(Net { name: "SW".to_string(), supply: false, pair: false, sensitive: false });
    let node = board.nets.len() - 1;
    let net = |board: &Board, name: &str| board.nets.iter().position(|net| net.name == name);
    let (raw, out, ground) = (net(&board, "RAW"), net(&board, "A_3V3"), net(&board, "A_GND"));
    for part in board.parts.iter_mut() {
        match part.name.as_str() {
            "U2" => {
                part.pads = vec![raw, Some(node), ground, ground];
                part.offsets = vec![[-1.5, -1.0], [1.5, -1.0], [-1.5, 1.0], [1.5, 1.0]];
                (part.size, part.area) = (Some([4.0, 3.0]), 12.0);
            }
            "C4" | "C5" => {
                part.offsets = vec![[-0.5, 0.0], [0.5, 0.0]];
                (part.size, part.area) = (Some([2.0, 1.0]), 2.0);
                part.value = "10u".to_string();
            }
            _ => {}
        }
    }
    let mut coil = board.parts[0].clone();
    (coil.name, coil.inductor, coil.pads) = ("L1".to_string(), true, vec![Some(node), out]);
    (coil.offsets, coil.size, coil.area, coil.at) =
        (vec![[-2.0, 0.0], [2.0, 0.0]], Some([6.0, 6.0]), 36.0, [10.0, 5.0]);
    board.parts.push(coil);
    let found = floorplan(&board).unwrap();
    let [found_loop] = &found.loops[..] else { panic!("{:?}", found.loops) };
    assert_eq!((found_loop.chip.as_str(), found_loop.node.as_str()), ("U2", "SW"));
    assert!(
        found_loop.parts.contains(&"L1".to_string()) && found_loop.parts.contains(&"C4".to_string()),
        "{found_loop:?}"
    );
    // the inductor stands against the chip, and the whole loop in a box not much larger than its parts
    let place = |name: &str| found.places.iter().find(|place| place.part == name).unwrap();
    assert!(dist(place("L1").at, place("U2").at) <= 6.5, "{:?} {:?}", place("L1"), place("U2"));
    assert!(found_loop.mm2 <= 150.0 && found_loop.bodies > 0.0, "{found_loop:?}");
    // the regulator and its inductor disturb; what is on a sensitive net is disturbed, and where the board
    // has room for it none of that stands within QUIET_MM of them
    assert_eq!(found.disturbing, [2, 0]);
    let mut quiet = board.clone();
    quiet.nets.iter_mut().find(|net| net.name == "FILT").unwrap().sensitive = true;
    let found = floorplan(&quiet).unwrap();
    assert_eq!(found.disturbing, [2, 3]);
    assert!(found.close.is_empty(), "{:?}", found.close);
    // (a part said to disturb does; one said not to be disturbed is not)
    quiet.parts.iter_mut().find(|part| part.name == "R2").unwrap().noisy = Some(true);
    quiet.parts.iter_mut().find(|part| part.name == "R1").unwrap().quiet = Some(false);
    assert_eq!(floorplan(&quiet).unwrap().disturbing, [3, 2]);
    // a divider on a small net of the chip is told with it, and stands by the chip
    let mut board = board.clone();
    board.nets.push(Net { name: "FB".to_string(), supply: false, pair: false, sensitive: false });
    let sense = board.nets.len() - 1;
    board.parts.iter_mut().find(|part| part.name == "U2").unwrap().pads[3] = Some(sense);
    for (name, other) in [("R8", out), ("R9", ground)] {
        let mut divider = board.parts.iter().find(|part| part.name == "R1").unwrap().clone();
        (divider.name, divider.pads, divider.at) = (name.to_string(), vec![Some(sense), other], [70.0, 35.0]);
        (divider.offsets, divider.size, divider.area) = (vec![[-0.5, 0.0], [0.5, 0.0]], Some([2.0, 1.0]), 2.0);
        board.parts.push(divider);
    }
    let found = floorplan(&board).unwrap();
    assert_eq!(found.loops[0].beside, ["R8", "R9"]);
    let place = |name: &str| found.places.iter().find(|place| place.part == name).unwrap();
    assert!(dist(place("R8").at, place("U2").at) <= 6.0 && dist(place("R9").at, place("U2").at) <= 6.0);
}

#[test]
fn what_stands_for_one_pad_is_found_by_how_the_signal_flows() {
    // an input on the field side: connector, a diode to ground, a resistor in series, a capacitor to
    // ground at the chip
    let mut board = two_grounds();
    for name in ["IN", "MID"] {
        board.nets.push(Net { name: name.to_string(), supply: false, pair: false, sensitive: false });
    }
    let net = |board: &Board, name: &str| board.nets.iter().position(|net| net.name == name);
    let (input, mid, ground) = (net(&board, "IN"), net(&board, "MID"), net(&board, "FIELD_0V"));
    // (the chip's and the connector's pads are known by where they are: so is which pad a part stands by)
    let chip = board.parts.iter_mut().find(|part| part.name == "U4").unwrap();
    chip.pads.push(mid);
    chip.offsets = vec![[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
    let like = board.parts.iter().find(|part| part.name == "R2").unwrap().clone();
    for (name, pads) in [("J2", [input, ground]), ("D1", [input, ground]), ("R5", [input, mid]), ("C9", [mid, ground])]
    {
        let mut part = like.clone();
        (part.name, part.pads) = (name.to_string(), pads.to_vec());
        (part.edge, part.diode, part.capacitor) = (name == "J2", name == "D1", name == "C9");
        part.offsets = vec![[-0.5, 0.0], [0.5, 0.0]];
        board.parts.push(part);
    }
    let found = floorplan(&board).unwrap();
    let of: BTreeMap<&str, &str> = found.capacitors.iter().map(|c| (c.0.as_str(), c.1.as_str())).collect();
    // the diode and the resistor are the connector's (its protection comes first), the capacitor the chip's
    assert_eq!((of.get("D1"), of.get("R5"), of.get("C9")), (Some(&"J2"), Some(&"J2"), Some(&"U4")), "{of:?}");
    assert_eq!(found.served[0], 3);
    // and from the connector through the resistor to the chip is a chain
    assert!(found.chains >= 1, "{}", found.chains);
    // a resistor across two pads of one chip, on nets of nothing else, is that chip's too
    for name in ["FB_A", "FB_B"] {
        board.nets.push(Net { name: name.to_string(), supply: false, pair: false, sensitive: false });
    }
    let (a, b) = (net(&board, "FB_A"), net(&board, "FB_B"));
    let amp = board.parts.iter_mut().find(|part| part.name == "U1").unwrap();
    amp.pads.extend([a, b]);
    amp.offsets = (0..6).map(|i| [i as f64 - 2.5, 0.0]).collect();
    let mut across = like.clone();
    (across.name, across.pads) = ("R7".to_string(), vec![a, b]);
    board.parts.push(across);
    let found = floorplan(&board).unwrap();
    assert!(found.capacitors.iter().any(|c| c.0 == "R7" && c.1 == "U1"), "{:?}", found.capacitors);
}

#[test]
fn a_chip_and_the_parts_by_its_pads_are_a_block_and_two_of_one_make_up_are_laid_out_alike() {
    // two amplifiers of one kind, each with a capacitor on its supply and a resistor across two of its
    // pads; the second stands a quarter turned in the file
    let names = ["GND", "VCC", "A1", "B1", "A2", "B2"];
    let net = |name: &str| names.iter().position(|n| *n == name);
    let part = |name: &str, kind: &str, at: Pt, size: [f64; 2], pads: &[&str], offsets: &[Pt], turned: u8| Part {
        name: name.to_string(),
        sheet: "amplifiers".to_string(),
        at,
        placed: true,
        fixed: false,
        edge: false,
        capacitor: name.starts_with('C'),
        inductor: false,
        diode: false,
        noisy: None,
        quiet: None,
        area: size[0] * size[1],
        pads: pads.iter().map(|n| net(n)).collect(),
        height: None,
        near: None,
        mate: None,
        size: Some(if turned % 2 == 1 { [size[1], size[0]] } else { size }),
        offsets: offsets.iter().map(|&offset| quartered(turned, offset)).collect(),
        pins: Vec::new(),
        through: false,
        pieces: Vec::new(),
        back: false,
        anchor: [0.0, 0.0],
        angle: 90.0 * f64::from(turned),
        value: kind.to_string(),
        footprint: kind.to_string(),
    };
    let chip = [[-2.0, -1.0], [-2.0, 1.0], [2.0, -1.0], [2.0, 1.0]];
    let small = [[-0.5, 0.0], [0.5, 0.0]];
    let board = Board {
        outline: [0.0, 0.0, 60.0, 30.0],
        regions: Vec::new(),
        shifts: Vec::new(),
        limits: Vec::new(),
        keepouts: Vec::new(),
        pitch: 0.4,
        layers: 2,
        stack: Vec::new(),
        gap: 4.0,
        via: 1.0,
        rows: Rows::Cheap,
        nets: names.iter().map(|n| Net { name: n.to_string(), supply: false, pair: false, sensitive: false }).collect(),
        parts: vec![
            part("U1", "amp", [10.0, 10.0], [5.0, 4.0], &["VCC", "GND", "A1", "B1"], &chip, 0),
            part("U2", "amp", [40.0, 10.0], [5.0, 4.0], &["VCC", "GND", "A2", "B2"], &chip, 1),
            part("C1", "100n", [12.0, 20.0], [2.0, 1.0], &["VCC", "GND"], &small, 0),
            part("C2", "100n", [42.0, 20.0], [2.0, 1.0], &["VCC", "GND"], &small, 0),
            part("R1", "10k", [15.0, 20.0], [2.0, 1.0], &["A1", "B1"], &small, 0),
            part("R2", "10k", [45.0, 20.0], [2.0, 1.0], &["A2", "B2"], &small, 1),
        ],
    };
    let found = floorplan(&board).unwrap();
    // two blocks of three parts each, of one make-up, laid out alike
    assert_eq!(found.blocks, [2, 6]);
    assert_eq!(found.alike, [(2, 2, 3, vec!["U1".to_string(), "U2".to_string()])]);
    // every part has its place, none over another, and each chip's parts are by it
    assert!(found.places.iter().all(|place| place.fits), "{:?}", found.places);
    let rects: Vec<[f64; 4]> = found.places.iter().filter_map(|place| place.rect).collect();
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            assert!(a[2].min(b[2]) - a[0].max(b[0]) < 1e-6 || a[3].min(b[3]) - a[1].max(b[1]) < 1e-6, "{a:?} {b:?}");
        }
    }
    let at = |name: &str| found.places.iter().find(|place| place.part == name).unwrap().at;
    for (chip, cap, resistor) in [("U1", "C1", "R1"), ("U2", "C2", "R2")] {
        assert!(dist(at(chip), at(cap)) < 6.0 && dist(at(chip), at(resistor)) < 6.0, "{:?}", found.places);
    }
    assert_eq!(found.bypass, [2, 2]);
    // drawn on a sheet each, with one part more that stands by no pad, the two are sheets of one make-up:
    // the first is laid out by itself, and both are placed whole, as it lies
    let mut board = board;
    board.nets.extend(["X1", "X2"].map(|name| Net {
        name: name.to_string(),
        supply: false,
        pair: false,
        sensitive: false,
    }));
    let (x1, x2) = (Some(board.nets.len() - 2), Some(board.nets.len() - 1));
    for (name, of, to, turned) in [("R3", "A1", x1, 0), ("R4", "A2", x2, 1)] {
        let mut more = board.parts.iter().find(|part| part.name == "R1").unwrap().clone();
        (more.name, more.pads) = (name.to_string(), vec![net(of), to]);
        (more.angle, more.size) = (90.0 * f64::from(turned), Some(if turned == 1 { [1.0, 2.0] } else { [2.0, 1.0] }));
        more.offsets = small.iter().map(|&offset| quartered(turned, offset)).collect();
        board.parts.push(more);
    }
    let mut far = board.parts.iter().find(|part| part.name == "U1").unwrap().clone();
    (far.name, far.value, far.pads) = ("U9".to_string(), "mux".to_string(), vec![net("VCC"), net("GND"), x1, x2]);
    board.parts.push(far);
    for part in board.parts.iter_mut() {
        part.sheet = match part.name.as_str() {
            "U1" | "C1" | "R1" | "R3" => "channel 1",
            "U2" | "C2" | "R2" | "R4" => "channel 2",
            _ => "the rest",
        }
        .to_string();
    }
    let found = floorplan(&board).unwrap();
    let [repeat] = &found.repeats[..] else { panic!("{:?}", found.repeats) };
    assert_eq!((repeat.sheets.len(), repeat.parts, repeat.alike, repeat.like.as_str()), (2, 4, 2, "channel 1"));
    // their own nets are matched copy against copy (the resistor's two, the one to the multiplexer), and
    // told with how much the copies differ; nothing is shared by both but the supply, which is no signal
    assert_eq!((repeat.matched, repeat.shared), (3, 0));
    assert!(repeat.spread_mm >= 0.0 && repeat.worst_mm >= repeat.spread_mm && repeat.longest_mm > 0.0, "{repeat:?}");
    // side by side wherever there is room, if that is asked; never, if that is
    let mut asked = board.clone();
    asked.rows = Rows::Always;
    assert_eq!(floorplan(&asked).unwrap().repeats[0].together, 2);
    asked.rows = Rows::Never;
    let apart = floorplan(&asked).unwrap();
    assert_eq!((apart.repeats[0].together, apart.repeats[0].alike), (0, 2));
    // alike: every two parts of the one are as far apart as the same two of the other
    let at = |name: &str| found.places.iter().find(|place| place.part == name).unwrap().at;
    let (one, other) = (["U1", "C1", "R1", "R3"], ["U2", "C2", "R2", "R4"]);
    for i in 0..4 {
        for j in i + 1..4 {
            let (a, b) = (dist(at(one[i]), at(one[j])), dist(at(other[i]), at(other[j])));
            assert!((a - b).abs() < 0.02, "{} {}: {a} {b}", one[i], one[j]);
        }
    }
    assert!(found.places.iter().all(|place| place.fits));
    // Two chips that each fill the board have room only on a side each; a part joined to nothing stays
    // where it stands and is in the others' way; a mark with no pad at all is in no one's.
    let wide = [[-3.0, 0.0], [3.0, 0.0]];
    let mut small = board.clone();
    small.outline = [0.0, 0.0, 10.0, 10.0];
    small.parts = vec![
        part("U1", "amp", [5.0, 5.0], [8.0, 8.0], &["A1", "B1"], &wide, 0),
        part("U2", "amp", [5.0, 5.0], [8.0, 8.0], &["A1", "B1"], &wide, 0),
    ];
    assert_eq!(floorplan(&small).unwrap().places.iter().filter(|place| place.fits).count(), 1);
    small.parts[1].back = true;
    let both = floorplan(&small).unwrap();
    assert!(both.places.iter().all(|place| place.fits) && both.places[1].back, "{:?}", both.places);
    let mut hole = part("H1", "hole", [5.0, 5.0], [3.0, 3.0], &["A2"], &[[0.0, 0.0]], 0);
    hole.through = true;
    small.parts.push(hole);
    let held = floorplan(&small).unwrap();
    let of = |name: &str| held.places.iter().find(|place| place.part == name).unwrap();
    assert_eq!((of("H1").at, of("H1").rect, of("H1").off), ([5.0, 5.0], None, Some(0.0)));
    assert!(!of("U1").fits && !of("U2").fits, "{:?}", held.places);
    small.parts[2].pads.clear();
    small.parts[2].offsets.clear();
    assert!(floorplan(&small).unwrap().places.iter().all(|place| place.fits));
    // A part given in pieces (two rows of pins far apart) leaves the ground between them to another.
    small.parts.truncate(1);
    small.parts[0].through = true;
    small.parts[0].pieces = vec![[-4.0, -4.0, -3.0, 4.0], [3.0, -4.0, 4.0, 4.0]];
    small.parts.push(part("R1", "10k", [5.0, 5.0], [3.0, 2.5], &["A1", "B1"], &[[-0.5, 0.0], [0.5, 0.0]], 0));
    let between = floorplan(&small).unwrap();
    assert!(between.places.iter().all(|place| place.fits), "{:?}", between.places);
    assert_eq!(between.places.iter().find(|place| place.part == "U1").unwrap().pieces.len(), 2);
    small.parts[0].pieces.clear();
    assert!(floorplan(&small).unwrap().places.iter().any(|place| !place.fits));
}

#[test]
fn two_grounds_are_neighbours_across_a_gap_as_long_as_they_are_beside_each_other() {
    let (a, b) = ([0.0, 0.0, 10.0, 10.0], [14.0, 4.0, 30.0, 20.0]);
    assert_eq!(strip_between(a, b, 4.0), Some(([10.0, 4.0, 14.0, 10.0], true)));
    assert_eq!(strip_between(b, a, 4.0), Some(([10.0, 4.0, 14.0, 10.0], true)));
    assert_eq!(
        strip_between([0.0, 0.0, 10.0, 10.0], [2.0, 12.0, 8.0, 20.0], 4.0),
        Some(([2.0, 10.0, 8.0, 12.0], false))
    );
    // too far apart; and corner to corner
    assert_eq!(strip_between(a, b, 3.0), None);
    assert_eq!(strip_between(a, [14.0, 10.0, 30.0, 20.0], 4.0), None);
}

/// A chip on each of two grounds with its supply and bypass capacitors, an isolator on both, a regulator
/// from one supply to another, and a filtered signal with one capacitor to ground.
fn two_grounds() -> Board {
    let nets = ["A_GND", "FIELD_0V", "A_3V3", "RAW", "B_VCC", "SIG", "FILT", "OUT", "LED"];
    let net = |name: &str| nets.iter().position(|n| *n == name);
    let part = |name: &str, x: f64, on: &[&str]| Part {
        name: name.to_string(),
        sheet: "all".to_string(),
        at: [x, 5.0],
        placed: true,
        fixed: false,
        edge: name.starts_with('J'),
        capacitor: name.starts_with('C'),
        inductor: name.starts_with('L'),
        diode: name.starts_with('D'),
        noisy: None,
        quiet: None,
        area: 10.0,
        pads: on.iter().map(|n| net(n)).collect(),
        height: None,
        near: None,
        mate: None,
        size: None,
        offsets: Vec::new(),
        pins: Vec::new(),
        anchor: [0.0, 0.0],
        through: false,
        pieces: Vec::new(),
        back: false,
        angle: 0.0,
        value: String::new(),
        footprint: String::new(),
    };
    Board {
        outline: [0.0, 0.0, 40.0, 10.0],
        regions: Vec::new(),
        shifts: Vec::new(),
        limits: Vec::new(),
        keepouts: Vec::new(),
        pitch: 0.4,
        layers: 2,
        stack: Vec::new(),
        gap: 4.0,
        via: 1.0,
        rows: Rows::Cheap,
        nets: nets.iter().map(|n| Net { name: n.to_string(), supply: false, pair: false, sensitive: false }).collect(),
        parts: vec![
            part("U1", 2.0, &["A_3V3", "A_GND", "SIG", "FILT"]),
            part("C1", 3.0, &["A_3V3", "A_GND"]),
            part("C2", 4.0, &["A_3V3", "A_GND"]),
            part("C3", 5.0, &["FILT", "A_GND"]),
            part("R1", 6.0, &["FILT", "SIG"]),
            part("U2", 7.0, &["RAW", "A_3V3", "A_GND"]),
            part("C4", 8.0, &["RAW", "A_GND"]),
            part("C5", 9.0, &["RAW", "A_GND"]),
            part("U3", 20.0, &["SIG", "A_3V3", "A_GND", "OUT", "B_VCC", "FIELD_0V"]),
            part("U4", 30.0, &["OUT", "B_VCC", "FIELD_0V"]),
            part("C6", 31.0, &["B_VCC", "FIELD_0V"]),
            part("C7", 32.0, &["B_VCC", "FIELD_0V"]),
            part("R2", 33.0, &["OUT", "LED"]),
            part("J1", 38.0, &["LED", "FIELD_0V"]),
        ],
    }
}

#[test]
fn grounds_are_domains_and_a_supply_is_what_two_capacitors_bypass() {
    let found = floorplan(&two_grounds()).unwrap();
    let power = &found.power;
    type Rails<'a> = Vec<(&'a str, Vec<(&'a str, usize, usize)>)>;
    let rails: Rails = power
        .domains
        .iter()
        .map(|d| (d.ground.as_str(), d.rails.iter().map(|r| (r.net.as_str(), r.parts.len(), r.capacitors)).collect()))
        .collect();
    // (the filtered signal has one capacitor to ground: no supply)
    assert_eq!(rails, [("A_GND", vec![("A_3V3", 5, 2), ("RAW", 3, 2)]), ("FIELD_0V", vec![("B_VCC", 4, 2)])]);
    // the resistor with no pad on a ground or a supply goes with the parts it shares nets with
    assert_eq!(power.domains[1].parts, ["C6", "C7", "J1", "R2", "U4"]);
    assert_eq!(power.domains[0].parts, ["C1", "C2", "C3", "C4", "C5", "R1", "U1", "U2", "U3"]);
    assert_eq!(power.between, [("U3".to_string(), vec!["A_GND".to_string(), "FIELD_0V".to_string()])]);
    assert!(power.adrift.is_empty());
    let links: Vec<(&str, &str, &[String])> =
        power.links.iter().map(|(a, b, by)| (a.as_str(), b.as_str(), &by[..])).collect();
    assert_eq!(links, [("A_3V3", "B_VCC", &["U3".to_string()][..]), ("A_3V3", "RAW", &["U2".to_string()][..])]);
}

#[test]
fn each_domain_is_given_ground_by_what_it_needs_wherever_its_parts_stand_today() {
    let found = floorplan(&two_grounds()).unwrap();
    let [a, field] = &found.zones[..] else { panic!("{:?}", found.zones.len()) };
    assert_eq!((a.name.as_str(), field.name.as_str()), ("A_GND", "FIELD_0V"));
    // nine parts to five of the same size and half the isolator, which stands on both grounds: 90 parts in
    // 145 of what the gap between the two domains leaves: 4 mm that nothing runs along
    assert_eq!(a.rect, [0.0, 0.0, 22.34, 10.0]);
    assert_eq!(field.rect, [26.34, 0.0, 40.0, 10.0]);
    let top = &found.roads[0];
    assert_eq!((top.rect, top.width, top.gap, top.lanes, top.level), ([22.34, 0.0, 26.34, 10.0], 4.0, true, 0, 0));
    assert_eq!(top.between, [vec!["A_GND".to_string()], vec!["FIELD_0V".to_string()]]);
    // the roads within a domain come after; the nets from one domain to the other go through the isolator
    assert!(found.roads[1..].iter().all(|road| !road.gap && road.level > 0 && road.width <= 2.0), "{:?}", found.roads);
    assert_eq!(found.travel.through, 1, "{:?}", found.travel);
    // and the isolator, on both grounds, stands on the gap: across its middle
    assert_eq!(top.parts, ["U3"]);
    let [bridge] = &found.bridges[..] else { panic!("{:?}", found.bridges) };
    assert_eq!((bridge.part.as_str(), bridge.on, bridge.reach), ("U3", Across::Gap, 0.0));
    assert_eq!(bridge.grounds, ["A_GND".to_string(), "FIELD_0V".to_string()]);
    assert!((bridge.at[0] - 24.34).abs() < 0.1, "{bridge:?}");
    assert!(bridge.at[1] > 0.0 && bridge.at[1] < 10.0 && bridge.off.is_some(), "{bridge:?}");
    assert_eq!(found.travel.cramped, 0.0);
    // (as it happens the parts stand where their zones are given ground)
    assert_eq!((a.at_home, field.at_home, a.placed, field.placed), (1.0, 1.0, 9, 5));
    assert!(a.off.unwrap() < 8.0 && field.off.unwrap() < 2.0, "{:?} {:?}", a.off, field.off);
    // every part is in one zone at each level
    for zone in &found.zones {
        if !zone.inside.is_empty() {
            let mut inner: Vec<&String> = zone.inside.iter().flat_map(|z| &z.parts).collect();
            inner.sort();
            assert_eq!(inner, zone.parts.iter().collect::<Vec<_>>());
        }
    }
    // nothing is fixed: turned round, the board is given the same ground (the connector moves with its zone)
    let mut turned = two_grounds();
    turned.parts.iter_mut().for_each(|part| part.at[0] = 40.0 - part.at[0]);
    let again = floorplan(&turned).unwrap();
    assert_eq!((again.zones[0].rect, again.zones[1].rect), (a.rect, field.rect));
}

#[test]
fn two_boards_in_one_box_each_hold_whole_domains_and_a_keep_out_is_no_room() {
    let mut board = two_grounds();
    // a large board and a small one, a gap between them; a keep-out on the large one
    board.regions = vec![[0.0, 0.0, 26.0, 10.0], [28.0, 0.0, 40.0, 10.0]];
    board.keepouts = vec![[10.0, 0.0, 12.0, 10.0]];
    let found = floorplan(&board).unwrap();
    let [large, small] = &found.regions[..] else { panic!("{:?}", found.regions) };
    assert_eq!((large.rect, large.room, &large.zones), ([0.0, 0.0, 26.0, 10.0], 240.0, &vec!["A_GND".to_string()]));
    assert_eq!((small.room, small.needed, &small.zones), (120.0, 55.0, &vec!["FIELD_0V".to_string()]));
    // what stands on both is the link between the boards: on its own, at the edge nearest the other
    let [link] = &found.bridges[..] else { panic!("{:?}", found.bridges) };
    assert_eq!((link.part.as_str(), link.on, link.reach), ("U3", Across::Link, 2.0));
    // (its body within its board, against that edge)
    assert!(link.at[0] > 24.0 && link.at[0] < 26.0 - 1.5, "{link:?}");
    // (the keep-out is no room: within the large board each lot has its share of what is left)
    // each domain has the whole of its board, and there is no road across the gap between boards
    assert_eq!((found.zones[0].rect, found.zones[1].rect), ([0.0, 0.0, 26.0, 10.0], [28.0, 0.0, 40.0, 10.0]));
    let within = |road: &Road| road.rect[2] <= 26.0 + 1e-6 || road.rect[0] >= 28.0 - 1e-6;
    assert!(found.roads.iter().all(|road| !road.gap && within(road)), "{:?}", found.roads);
}

/// Four chips in a square on one ground, each tied to the next round the square by eight nets.
fn square() -> Board {
    let mut nets = vec![Net { name: "GND".to_string(), supply: false, pair: false, sensitive: false }];
    let mut pads: Vec<Vec<Option<usize>>> = vec![vec![Some(0)]; 4];
    for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        for k in 0..8 {
            nets.push(Net { name: format!("N{a}{b}_{k}"), supply: false, pair: false, sensitive: false });
            pads[a].push(Some(nets.len() - 1));
            pads[b].push(Some(nets.len() - 1));
        }
    }
    let part = |n: usize, x: f64, y: f64| Part {
        name: format!("U{n}"),
        sheet: format!("sheet {n}"),
        at: [x, y],
        placed: true,
        fixed: false,
        edge: false,
        capacitor: false,
        inductor: false,
        diode: false,
        noisy: None,
        quiet: None,
        area: 100.0,
        pads: pads[n].clone(),
        height: None,
        near: None,
        mate: None,
        size: None,
        offsets: Vec::new(),
        pins: Vec::new(),
        anchor: [0.0, 0.0],
        through: false,
        pieces: Vec::new(),
        back: false,
        angle: 0.0,
        value: String::new(),
        footprint: String::new(),
    };
    Board {
        outline: [0.0, 0.0, 40.0, 40.0],
        regions: Vec::new(),
        shifts: Vec::new(),
        limits: Vec::new(),
        keepouts: Vec::new(),
        pitch: 0.4,
        layers: 4,
        stack: ["F", "In1", "In2", "In3", "In4", "B"]
            .iter()
            .zip([Way::Any, Way::Plane, Way::Up, Way::Across, Way::Plane, Way::Any])
            .map(|(name, way)| Layer { name: name.to_string(), way })
            .collect(),
        gap: 4.0,
        via: 1.0,
        rows: Rows::Cheap,
        nets,
        parts: vec![part(0, 10.0, 10.0), part(1, 30.0, 10.0), part(2, 30.0, 30.0), part(3, 10.0, 30.0)],
    }
}

#[test]
fn a_road_holds_what_fits_on_the_layers_that_run_its_way_and_every_net_is_given_a_way_along_the_roads() {
    let found = floorplan(&square()).unwrap();
    // one domain: no gap; a road across the whole and one within each half, at right angles to it
    assert!(found.roads.iter().all(|road| !road.gap));
    let first = &found.roads[0];
    assert_eq!(found.roads.len(), 3);
    assert!(found.roads[1..].iter().all(|road| road.up != first.up && road.level == 1));
    // sixteen nets cross the first cut: on three layers that run its way (the two outer and one inner
    // of the two between the planes), six tracks abreast
    assert_eq!((first.nets, first.width, first.lanes), (16, 2.4, 18));
    // every net goes to the zone beside its own: across a road, or a little along one
    let travel = &found.travel;
    assert_eq!((travel.nets, travel.through, travel.stranded), (32, 0, 0), "{travel:?}");
    assert!(travel.mm > 32.0 * 15.0 && travel.mm < 32.0 * 30.0, "{travel:?}");
    // no road is asked for more than it holds, and each tells how much runs along it
    assert_eq!(travel.over, 0, "{:?}", found.roads);
    // (what was tried after the first laying out left it no worse)
    let weigh = |t: &Travel| t.mm + VIA_MM * t.vias as f64;
    assert!(weigh(travel) <= weigh(&found.first) + 1e-6, "{travel:?} {:?}", found.first);
    assert!(found.roads.iter().all(|road| road.used <= road.lanes));
    // with one layer that runs each way and nothing else, the roads are three times as wide for the same
    let mut plain = square();
    plain.stack.retain(|layer| layer.way != Way::Any);
    let wide = floorplan(&plain).unwrap();
    assert_eq!((wide.roads[0].width, wide.roads[0].lanes), (6.4, 16));
}

#[test]
fn a_tall_part_keeps_its_zone_from_under_another_board_and_a_part_is_pulled_to_the_point_it_is_to_be_near() {
    // the four chips of the square; one of them tall, and the right half of the board has 5 mm of height
    let mut board = square();
    board.parts[0].height = Some(12.0);
    board.limits = vec![Limit { rect: [20.0, 0.0, 40.0, 40.0], height: 5.0 }];
    let found = floorplan(&board).unwrap();
    let zone = |found: &Floorplan, part: &str| -> [f64; 4] {
        fn of(zone: &Zone, part: &str) -> Option<[f64; 4]> {
            if zone.inside.is_empty() {
                return zone.parts.iter().any(|p| p == part).then_some(zone.rect);
            }
            zone.inside.iter().find_map(|inner| of(inner, part))
        }
        found.zones.iter().find_map(|z| of(z, part)).unwrap()
    };
    assert!(zone(&found, "U0")[2] <= 20.0 + 1e-6, "{:?}", zone(&found, "U0"));
    assert!(found.tall.is_empty(), "{:?}", found.tall);
    // with the limit on the other half, it is given ground on the other side
    board.limits[0].rect = [0.0, 0.0, 20.0, 40.0];
    let found = floorplan(&board).unwrap();
    assert!(zone(&found, "U0")[0] >= 20.0 - 1e-6 && found.tall.is_empty(), "{:?}", zone(&found, "U0"));
    // a part that is to be near a corner takes its zone there
    let mut board = square();
    board.parts[2].near = Some([0.0, 0.0]);
    let found = floorplan(&board).unwrap();
    let there = zone(&found, "U2");
    assert!(there[0] < 1e-6 && there[1] < 1e-6, "{there:?}");
}

#[test]
fn two_boards_of_a_stack_bring_what_is_tied_across_them_to_the_same_place() {
    // the two grounds on a board each, the second drawn to the right and mounted over the first's left end
    let mut board = two_grounds();
    board.outline = [0.0, 0.0, 60.0, 10.0];
    board.regions = vec![[0.0, 0.0, 40.0, 10.0], [42.0, 0.0, 60.0, 10.0]];
    board.shifts = vec![[0.0, 0.0], [-42.0, 0.0]];
    let found = floorplan(&board).unwrap();
    fn of(zone: &Zone, part: &str) -> Option<[f64; 4]> {
        if zone.inside.is_empty() {
            return zone.parts.iter().any(|p| p == part).then_some(zone.rect);
        }
        zone.inside.iter().find_map(|inner| of(inner, part))
    }
    // the isolator, on the lower board, is tied to the chip on the upper one: its zone is at the left end
    // of the lower board, under the upper
    let isolator = found.zones.iter().find_map(|z| of(z, "U3")).unwrap();
    assert!(isolator[0] < 10.0, "{isolator:?}");
    // a part on each board that mates with the other: the regulator's zone comes under the small upper
    // board, at the right end of the lower one, where without that it stood at the left
    let mut board = two_grounds();
    board.outline = [0.0, 0.0, 52.0, 10.0];
    board.regions = vec![[0.0, 0.0, 40.0, 10.0], [42.0, 0.0, 52.0, 10.0]];
    board.shifts = vec![[0.0, 0.0], [-12.0, 0.0]];
    for part in board.parts.iter_mut().filter(|part| ["U2", "C4", "C5"].contains(&part.name.as_str())) {
        part.sheet = "regulator".to_string();
    }
    let before = floorplan(&board).unwrap();
    board.parts.iter_mut().find(|part| part.name == "U2").unwrap().mate = Some("J1".to_string());
    let after = floorplan(&board).unwrap();
    let at = |found: &Floorplan| found.zones.iter().find_map(|z| of(z, "U2")).unwrap();
    assert!(at(&before)[2] < 20.0 && at(&after)[0] > 20.0, "{:?} {:?}", at(&before), at(&after));
    assert!(after.travel.mate_mm < before.travel.mate_mm || before.travel.mate_mm == 0.0, "{:?}", after.travel);
    board.parts[0].mate = Some("U99".to_string());
    assert!(floorplan(&board).is_err());
    // (and what is said of a part's pads is said of each)
    let mut board = two_grounds();
    board.parts[0].offsets = vec![[0.0, 0.0]];
    assert!(floorplan(&board).is_err());
    board.parts[0].offsets = vec![[0.0, 0.0]; 4];
    board.parts[0].pins = vec![Pin::Input, Pin::PowerIn, Pin::Unknown, Pin::Passive];
    assert!(floorplan(&board).is_ok());
}

#[test]
fn a_board_without_size_or_a_pad_on_a_net_that_is_not_there_is_refused() {
    let mut board = two_grounds();
    board.outline = [0.0, 0.0, 0.0, 10.0];
    assert!(floorplan(&board).is_err());
    let mut board = two_grounds();
    board.parts[0].pads.push(Some(99));
    assert!(floorplan(&board).is_err());
}

#[test]
fn parts_on_the_two_sides_of_a_board_share_its_ground() {
    // all on the front: the sum; some on the back: the more of the two sides; one with holes: on both
    let mut board = two_grounds();
    let all: Vec<usize> = (0..board.parts.len()).collect();
    let sum: f64 = board.parts.iter().map(|part| part.area).sum();
    assert!((super::ground::taken_by(&board, &all) - sum).abs() < 1e-9);
    let (a, b, c) = (board.parts[0].area, board.parts[1].area, board.parts[2].area);
    board.parts[0].back = true;
    assert!((super::ground::taken_by(&board, &[0, 1, 2]) - a.max(b + c)).abs() < 1e-9);
    board.parts[1].through = true;
    assert!((super::ground::taken_by(&board, &[0, 1, 2]) - (b + a.max(c))).abs() < 1e-9);
}

#[test]
fn on_a_board_with_room_to_spare_the_parts_of_a_zone_are_kept_together() {
    // the same parts on a board sixteen times the size: those of the first domain's zone stand within a
    // ground they fill to PACKED and not over all of the zone's; and no body is over another
    let mut board = two_grounds();
    board.outline = [0.0, 0.0, 160.0, 40.0];
    let found = floorplan(&board).unwrap();
    let rect = |name: &str| found.places.iter().find(|place| place.part == name).unwrap().rect.unwrap();
    fn leaves<'a>(zone: &'a Zone, out: &mut Vec<&'a Zone>) {
        if zone.inside.is_empty() {
            out.push(zone);
        }
        zone.inside.iter().for_each(|inner| leaves(inner, out));
    }
    let mut smallest = Vec::new();
    found.zones.iter().for_each(|zone| leaves(zone, &mut smallest));
    let mut looked = 0;
    for zone in smallest.iter().filter(|zone| zone.parts.len() >= 3) {
        let within =
            |r: &[f64; 4]| r[0] >= zone.rect[0] && r[1] >= zone.rect[1] && r[2] <= zone.rect[2] && r[3] <= zone.rect[3];
        let own: Vec<[f64; 4]> = zone.parts.iter().map(|name| rect(name)).filter(within).collect();
        let low = |k: usize| own.iter().map(|r| r[k]).fold(f64::INFINITY, f64::min);
        let high = |k: usize| own.iter().map(|r| r[k]).fold(f64::NEG_INFINITY, f64::max);
        let over = (high(2) - low(0)) * (high(3) - low(1));
        let all = (zone.rect[2] - zone.rect[0]) * (zone.rect[3] - zone.rect[1]);
        assert!(
            over <= 1.5 * zone.area / PACKED && over < all / 2.0,
            "{}: {over} of {all}, parts {}",
            zone.name,
            zone.area
        );
        looked += 1;
    }
    assert!(looked >= 2);
    let all: Vec<[f64; 4]> = found.places.iter().filter_map(|place| place.rect).collect();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert!(a[2].min(b[2]) - a[0].max(b[0]) < 1e-6 || a[3].min(b[3]) - a[1].max(b[1]) < 1e-6, "{a:?} {b:?}");
        }
    }
}

#[test]
fn a_body_that_is_nearly_square_is_not_turned_a_quarter_onto_what_stands_against_it() {
    // bodies 2.9 by 2.7 with pads that would face better turned a quarter: each lies as it was settled or
    // turned half round, and none is over another
    let mut board = two_grounds();
    for part in &mut board.parts {
        part.size = Some([2.9, 2.7]);
        part.area = 2.9 * 2.7;
        part.offsets = (0..part.pads.len()).map(|i| [0.0, i as f64 - 1.0]).collect();
    }
    let found = floorplan(&board).unwrap();
    let all: Vec<[f64; 4]> = found.places.iter().filter(|place| place.fits).filter_map(|place| place.rect).collect();
    assert!(all.len() > 8);
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert!(a[2].min(b[2]) - a[0].max(b[0]) < 1e-6 || a[3].min(b[3]) - a[1].max(b[1]) < 1e-6, "{a:?} {b:?}");
        }
    }
}
