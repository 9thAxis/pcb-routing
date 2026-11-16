//! Cases that once went wrong, and inputs that must be refused.

use pcb_routing::{route, Error, Problem, Router, Status};
use serde_json::{json, Value};

fn net(key: &str, a: [f64; 2], b: [f64; 2], layers: &[&str]) -> Value {
    json!({"key": key, "net": key, "width": 0.15, "via": [0.45, 0.2], "layers": layers, "via_penalty_mm": 3.0,
           "clearance": {"default": 0.17},
           "terminals": [{"xy": a, "layers": [layers[0]]}, {"xy": b, "layers": [layers[0]]}]})
}

fn small(nets: Vec<Value>) -> Value {
    json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F", "B"], "nets": nets})
}

fn problem(v: Value) -> Problem {
    serde_json::from_value(v).unwrap()
}

/// A long line beside a net that hardly gives way. The first pass used to take the cost of the path it had
/// followed from its own table, within a tolerance per step; over 1,200 steps the tolerances added up to more
/// than the difference between real paths, the bound came out below every real path, and the second pass cut
/// them all off: the net was reported as unroutable.
#[test]
fn a_long_line_beside_a_stiff_net_is_laid() {
    for give in [0.01, 0.02, 0.05, 0.3, 1.0] {
        let mut b = net("b", [1.0, 3.1], [61.0, 3.1], &["F"]);
        b["yield"] = json!(give);
        let p = problem(
            json!({"window": [0.0, 0.0, 62.0, 6.0], "step": 0.05, "layers": ["F"], "nets": [net("a", [5.0, 3.0], [57.0, 3.0], &["F"]), b]}),
        );
        let s = Router::new(&p).verify_search(true).run().unwrap();
        assert_eq!(s.status, Status::Routed, "yield {give}: {}", s.message);
        assert_eq!(s.stats.mismatches, 0, "yield {give}: two-pass searches that differ from one-pass");
    }
}

/// Two nets that each change layer to pass a wall: their vias keep the hole spacing from each other, not only
/// the copper clearance.
#[test]
fn vias_of_different_nets_keep_the_hole_spacing() {
    let mut v =
        small(vec![net("a", [2.0, 2.7], [8.0, 2.7], &["F", "B"]), net("b", [2.0, 3.4], [8.0, 3.4], &["F", "B"])]);
    v["copper"] = json!([{"t": "seg", "a": [5.0, -1.0], "b": [5.0, 7.0], "w": 0.2, "net": "WALL", "layers": ["F"]}]);
    v["rules"] = json!({"hole_centre_mm": 1.5});
    let s = route(&problem(v)).unwrap();
    assert_eq!(s.status, Status::Routed, "{:?}", s.conflicts);
    for a in &s.nets[0].vias {
        for b in &s.nets[1].vias {
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            assert!(d >= 1.5 - 1e-6, "vias of two nets {d} mm apart");
        }
    }
}

fn pair_problem(step: f64, pads: [[f64; 2]; 4], ends: [[f64; 2]; 2]) -> Problem {
    let entry = |key: &str, net: &str, width: f64, terminals: Value, more: Value| {
        let mut v = json!({"key": key, "net": net, "width": width, "via": [0.45, 0.2], "layers": ["F", "B"], "via_penalty_mm": 3.0,
                           "clearance": {"default": 0.17}, "terminals": terminals});
        for (k, x) in more.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    let stub = |key: &str, net: &str, at: [f64; 2]| {
        entry(key, net, 0.15, json!([{"xy": at, "layers": ["F"]}]), json!({"last": true, "quiet": true}))
    };
    problem(json!({
        "window": [0.0, 0.0, 10.0, 6.0], "step": step, "layers": ["F", "B"],
        "nets": [
            entry("pair", "~pair", 0.59, json!([{"xy": ends[0], "layers": ["F"]}, {"xy": ends[1], "layers": ["F"]}]), json!({"no_vias": true, "quiet": true})),
            stub("p0", "P", pads[0]), stub("m0", "M", pads[1]), stub("p1", "P", pads[2]), stub("m1", "M", pads[3]),
        ],
        "pairs": [{"key": "in1", "trunk": "pair", "legs": ["P", "M"], "leg_width": 0.15, "pitch": 0.3, "stubs": [["p0", "m0"], ["p1", "m1"]]}],
    }))
}

/// Both pads of the first end lie on the same side of the trunk. The side of each leg used to be decided from
/// the first pad alone, which could put the legs the wrong way round, and one stub then went round the other pad.
#[test]
fn pair_legs_take_the_side_of_their_pads_when_both_pads_are_on_one_side() {
    let p = pair_problem(0.05, [[1.0, 1.5], [1.0, 3.0], [9.0, 3.0], [9.0, 4.5]], [[2.0, 3.6], [8.0, 2.4]]);
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    // the stubs do not cross: every stub is a short piece, none goes round a pad
    for n in &s.nets {
        for stub in &n.tracks[1..] {
            let length: f64 = stub
                .points
                .windows(2)
                .map(|w| ((w[0][0] - w[1][0]).powi(2) + (w[0][1] - w[1][1]).powi(2)).sqrt())
                .sum();
            assert!(length < 3.2, "{}: a stub of {length:.2} mm {:?}", n.key, stub.points);
        }
    }
}

/// The cell a stub arrives on was a fixed 0.03 mm outside the leg's end, which only suits a 0.05 mm grid.
#[test]
fn a_pair_is_laid_on_a_coarser_grid() {
    let p = pair_problem(0.1, [[1.0, 2.0], [1.0, 4.0], [9.0, 2.5], [9.0, 3.5]], [[2.0, 3.0], [8.0, 3.0]]);
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
}

/// A wall across the layer of the pads: the pair changes layer to pass it and comes back, through two vias
/// side by side each time, and its legs keep their distance from each other all the way.
#[test]
fn a_pair_changes_layer_through_two_vias_side_by_side() {
    let p = pair_problem(0.05, [[1.0, 2.5], [1.0, 3.5], [9.0, 2.5], [9.0, 3.5]], [[2.0, 3.0], [8.0, 3.0]]);
    let mut v = serde_json::to_value(&p).unwrap();
    v["copper"] = json!([{"t": "seg", "a": [5.0, -1.0], "b": [5.0, 7.0], "w": 0.2, "net": "WALL", "layers": ["F"]}]);
    v["nets"][0]["no_vias"] = json!(false);
    v["nets"][0]["via"] = json!([1.05, 0.8]);
    v["pairs"][0]["via_pitch"] = json!(0.6);
    let s = route(&problem(v.clone())).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    let far = |p: [f64; 2], q: [f64; 2]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
    let from = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let (u, w) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
        let t = ((u[0] * w[0] + u[1] * w[1]) / (u[0] * u[0] + u[1] * u[1]).max(1e-12)).clamp(0.0, 1.0);
        far(p, [a[0] + t * u[0], a[1] + t * u[1]])
    };
    let legs: Vec<_> = s.nets.iter().filter(|n| n.key.starts_with("in1/")).collect();
    assert_eq!(legs.len(), 2);
    for (me, other) in [(legs[0], legs[1]), (legs[1], legs[0])] {
        assert_eq!(me.vias.len(), 2, "{}: {:?}", me.key, me.vias);
        assert!(me.tracks.iter().any(|t| t.layer == "B"), "{}: nothing on the other layer", me.key);
        for &via in &me.vias {
            // one on either side of the wall, 0.6 mm from its fellow, and clear of the other leg's track
            assert!(other.vias.iter().any(|&o| (far(via, o) - 0.6).abs() < 1e-5), "{via:?} {:?}", other.vias);
            assert!(other.vias.iter().all(|&o| far(via, o) > 0.6 - 1e-5));
            let track = other.tracks.iter().flat_map(|t| t.points.windows(2)).map(|w| from(via, w[0], w[1]));
            assert!(
                track.fold(f64::INFINITY, f64::min) > 0.225 + 0.15 + 0.075 - 1e-5,
                "{}: a via by the other leg",
                me.key
            );
        }
    }
    assert!(legs[0].vias.iter().any(|v| v[0] < 5.0) && legs[0].vias.iter().any(|v| v[0] > 5.0));
    // without a distance for its vias a trunk may not change layer, and its via must have room for both
    v["pairs"][0]["via_pitch"] = json!(0.0);
    refused(v.clone(), "pairs[0] (in1).trunk");
    v["pairs"][0]["via_pitch"] = json!(0.7);
    refused(v, "pairs[0] (in1).trunk");
}

fn refused(v: Value, field: &str) {
    match route(&problem(v)) {
        Err(Error::Invalid { field: f, reason }) => {
            assert!(f.contains(field), "refused for {f} ({reason}), expected {field}")
        }
        other => panic!("expected to be refused for {field}, got {other:?}"),
    }
}

/// Numbers that used to send a search round for ever (and the machine out of memory) or the grid out of bounds.
#[test]
fn numbers_that_make_no_sense_are_refused() {
    let nets = || vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F", "B"])];
    let with = |path: &[&str], value: Value| {
        let mut v = small(nets());
        let mut at = &mut v;
        for key in &path[..path.len() - 1] {
            if at.get(*key).is_none() {
                at[*key] = json!({});
            }
            at = at.get_mut(*key).unwrap();
        }
        at[path[path.len() - 1]] = value;
        v
    };
    refused(with(&["options", "bend_mm"], json!(-0.3)), "options.bend_mm");
    refused(with(&["options", "present_growth"], json!(0.5)), "options.present_growth");
    refused(with(&["step"], json!(0.0)), "step");
    refused(with(&["step"], json!(-0.05)), "step");
    refused(with(&["window"], json!([0.0, 0.0, -1.0, 6.0])), "window");
    refused(with(&["rules", "hole_centre_mm"], json!(-1.0)), "rules.hole_centre_mm");
    let mut v = small(nets());
    v["nets"][0]["layer_cost"] = json!({"F": -1.0});
    refused(v, "layer_cost.F");
    let mut v = small(nets());
    v["nets"][0]["width"] = json!(0.0);
    refused(v, "width");
    let mut v = small(nets());
    v["nets"][0]["yield"] = json!(-1.0);
    refused(v, "yield");
}

#[test]
fn names_that_do_not_exist_are_refused() {
    refused(small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"]), net("a", [2.0, 4.0], [8.0, 4.0], &["F"])]), "key");
    refused(small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["Top"])]), "layers");
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["layers"] = json!(["1", "2", "3", "4", "5", "6", "7", "8"]);
    refused(v, "layers");
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["nets"][0]["terminals"] = json!([{"xy": [2.0, 3.0], "layers": ["F"]}]);
    refused(v, "terminals");
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["pairs"] = json!([{"key": "p", "trunk": "nowhere", "legs": ["P", "M"], "leg_width": 0.15, "pitch": 0.3, "stubs": [["a", "a"], ["a", "a"]]}]);
    refused(v, "trunk");
    let many: Vec<Value> = (0..250).map(|k| net(&format!("n{k}"), [2.0, 3.0], [8.0, 3.0], &["F"])).collect();
    refused(small(many), "nets");
}

#[test]
fn a_misspelt_field_is_an_error() {
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["options"] = json!({"bend_m": 0.0});
    let e = serde_json::from_value::<Problem>(v).unwrap_err().to_string();
    assert!(e.contains("bend_m"), "{e}");
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["nets"][0]["yeild"] = json!(0.5);
    assert!(serde_json::from_value::<Problem>(v).is_err());
}

#[test]
fn a_grid_too_large_is_refused_before_anything_is_allocated() {
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["window"] = json!([0.0, 0.0, 5000.0, 5000.0]);
    v["step"] = json!(0.01);
    assert!(matches!(route(&problem(v)), Err(Error::TooLarge { .. })));
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["options"] = json!({"max_states": 1000});
    assert!(matches!(route(&problem(v)), Err(Error::TooLarge { limit: 1000, .. })));
}

#[test]
fn an_attempt_that_does_not_exist_is_refused() {
    let p = problem(small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]));
    assert!(Router::new(&p).only_attempt(3).run().is_err());
    assert_eq!(Router::new(&p).only_attempt(0).run().unwrap().status, Status::Routed);
}

/// A solution that is not complete has no score; it must still write and read back as JSON.
#[test]
fn every_solution_survives_json() {
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["copper"] =
        json!([{"t": "seg", "a": [5.0, -1.0], "b": [5.0, 7.0], "w": 0.2, "net": "WALL", "layers": ["F", "B"]}]);
    let s = route(&problem(v)).unwrap();
    assert_eq!(s.status, Status::Unroutable);
    let back: pcb_routing::Solution = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    // and a problem writes and reads back too
    let p = problem(small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]));
    let again: Problem = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(route(&again).unwrap().nets, route(&p).unwrap().nets);
}

/// With the others fixed a net used to be laid whole or not at all: one terminal out of reach cost the net
/// the connections it could make. And the failure named only the far side, whichever side was shut in.
#[test]
fn what_can_be_joined_of_a_net_is_kept_when_a_terminal_is_out_of_reach() {
    let mut a = net("a", [2.0, 3.0], [4.0, 3.0], &["F"]);
    a["terminals"].as_array_mut().unwrap().push(json!({"xy": [8.0, 3.0], "layers": ["F"]}));
    let mut v = small(vec![a]);
    v["copper"] =
        json!([{"t": "seg", "a": [5.0, -1.0], "b": [5.0, 7.0], "w": 0.2, "net": "WALL", "layers": ["F", "B"]}]);
    for negotiate in [true, false] {
        v["options"] = json!({"negotiate": negotiate});
        let s = route(&problem(v.clone())).unwrap();
        assert_eq!(s.status, Status::Unroutable);
        assert_eq!(s.unrouted.len(), 1, "{:?}", s.unrouted);
        assert!((s.nets[0].length_mm - 2.0).abs() < 1e-6, "negotiate {negotiate}: {} mm", s.nets[0].length_mm);
        for side in ["[2.0, 3.0]", "[4.0, 3.0]", "[8.0, 3.0]"] {
            assert!(s.unrouted[0].contains(side), "{} does not name {side}", s.unrouted[0]);
        }
    }
}

/// A net of which nothing is laid has length 0, not the -0.0 that the sum of no numbers is.
#[test]
fn nothing_laid_has_length_zero() {
    let mut v = small(vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    v["copper"] =
        json!([{"t": "seg", "a": [5.0, -1.0], "b": [5.0, 7.0], "w": 0.2, "net": "WALL", "layers": ["F", "B"]}]);
    let s = route(&problem(v)).unwrap();
    assert!(!serde_json::to_string(&s).unwrap().contains("-0.0"));
}

/// A pair whose trunk could not be laid used to end the attempt with nothing at all to show. Now the pair is
/// said to be missing and the other nets are laid as if it were not there.
#[test]
fn a_pair_that_cannot_be_laid_does_not_take_the_other_nets_with_it() {
    let mut p = pair_problem(0.05, [[1.0, 1.5], [1.0, 3.0], [9.0, 3.0], [9.0, 4.5]], [[2.0, 3.6], [8.0, 2.4]]);
    p.copper.push(
        serde_json::from_value(
            json!({"t": "seg", "a": [5.0, -1.0], "b": [5.0, 7.0], "w": 0.2, "net": "WALL", "layers": ["F", "B"]}),
        )
        .unwrap(),
    );
    p.nets.push(serde_json::from_value(net("other", [1.0, 5.2], [4.0, 5.2], &["F"])).unwrap());
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Unroutable);
    assert!(!s.unrouted.is_empty());
    let other = s.nets.iter().find(|n| n.key == "other").expect("the other net is reported");
    assert!((other.length_mm - 3.0).abs() < 1e-6, "{} mm", other.length_mm);
    assert!(s.nets.iter().all(|n| n.key == "other" || n.tracks.is_empty()), "copper of the pair is left");
}

/// A track wider than its own pad, on pads at a fine pitch: the cell nearest the pad's middle is clear of the
/// neighbouring pad, but the last piece of track from that cell to the exact middle, off the grid, was not
/// tested and came nearer the neighbour than the clearance. The track now ends on the cell.
#[test]
fn the_end_of_a_track_at_a_pad_keeps_its_distance_from_the_next_pad() {
    let pad = |net: &str, x0: f64, x1: f64| json!({"t": "poly", "rings": [[[x0, 5.591], [x1, 5.591], [x1, 6.391], [x0, 6.391]]], "net": net, "layers": ["F"], "pad": true});
    let mut a = net("a", [8.5555, 5.991], [8.6, 2.0], &["F"]);
    a["width"] = json!(0.3);
    a["clearance"] = json!({"default": 0.1724});
    let mut v = small(vec![a]);
    v["step"] = json!(0.1);
    v["copper"] = json!([pad("a", 8.436, 8.675), pad("b", 8.035, 8.275)]);
    let s = route(&problem(v)).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    for track in &s.nets[0].tracks {
        for point in &track.points {
            let gap = point[0] - 0.15 - 8.275;
            assert!(point[1] < 5.591 - 0.3 || gap >= 0.1724 - 1e-9, "{point:?} is {gap:.4} mm from the next pad");
        }
    }
}

/// Pads that do not stand on the grid: a track used to reach each with a crooked stub from the last cell,
/// shorter than the track is wide. Its last corner now slides so that it ends on the pad and every stretch
/// keeps one of the eight directions; a second net beside it keeps its distance all the same.
#[test]
fn a_track_ends_on_a_pad_off_the_grid_without_a_stub() {
    let a = net("a", [1.013, 1.021], [6.037, 4.018], &["F"]);
    let b = net("b", [1.013, 1.521], [5.537, 4.518], &["F"]);
    let s = route(&problem(small(vec![a, b]))).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    for (n, ends) in s.nets.iter().zip([[[1.013, 1.021], [6.037, 4.018]], [[1.013, 1.521], [5.537, 4.518]]]) {
        let points = &n.tracks[0].points;
        assert_eq!(n.tracks.len(), 1);
        assert!(ends.contains(&points[0]) && ends.contains(points.last().unwrap()), "{points:?}");
        for s in points.windows(2) {
            let (dx, dy) = ((s[1][0] - s[0][0]).abs(), (s[1][1] - s[0][1]).abs());
            assert!(dx.hypot(dy) > 0.15, "a stub in {points:?}");
            assert!(dx < 1e-9 || dy < 1e-9 || (dx - dy).abs() < 1e-9, "a crooked stretch in {points:?}");
        }
    }
    // the two tracks keep their clearance where they have moved off the grid
    let (ta, tb) = (&s.nets[0].tracks[0].points, &s.nets[1].tracks[0].points);
    for sa in ta.windows(2) {
        for sb in tb.windows(2) {
            for k in 0..=20 {
                let f = k as f64 / 20.0;
                let q = [sb[0][0] + (sb[1][0] - sb[0][0]) * f, sb[0][1] + (sb[1][1] - sb[0][1]) * f];
                let (vx, vy) = (sa[1][0] - sa[0][0], sa[1][1] - sa[0][1]);
                let t = (((q[0] - sa[0][0]) * vx + (q[1] - sa[0][1]) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
                let d = (q[0] - sa[0][0] - t * vx).hypot(q[1] - sa[0][1] - t * vy);
                assert!(d >= 0.15 + 0.17 - 1e-9, "the tracks come within {d:.4} mm");
            }
        }
    }
}

/// Small problems on which ending tracks on pads off the grid once went wrong (found by a randomised check):
/// a stub laid into room another net had just moved into, a track sent to the point of a pad on another
/// layer and away from the via it started on, and a net that changes layer in its own pad's cell left without
/// the piece of track to the pad's point. Tracks of different nets keep their distance (less the hair the
/// grid allows), and every via and every terminal is a point of a track of its net.
#[test]
fn tracks_ended_off_the_grid_keep_their_distance_and_their_vias() {
    let dist2 = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = vx * vx + vy * vy;
        let t = if l2 > 0.0 { (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        (p[0] - a[0] - t * vx).hypot(p[1] - a[1] - t * vy)
    };
    for seed in [11146, 13213, 3537, 3101, 38] {
        let text =
            std::fs::read_to_string(format!("{}/tests/data/ends-{seed}.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let p: Problem = serde_json::from_str(&text).unwrap();
        let s = route(&p).unwrap();
        assert_eq!(s.status, Status::Routed, "{seed}: {}", s.message);
        let need = p.nets[0].width + p.rules.new_clearance_mm;
        for (i, a) in s.nets.iter().enumerate() {
            for b in s.nets[i + 1..].iter().filter(|b| b.net != a.net) {
                for (ta, tb) in a.tracks.iter().flat_map(|ta| b.tracks.iter().map(move |tb| (ta, tb))) {
                    if ta.layer != tb.layer {
                        continue;
                    }
                    for sa in ta.points.windows(2) {
                        for sb in tb.points.windows(2) {
                            // (the ends of each against the other: the tracks do not cross)
                            let d = [
                                dist2(sa[0], sb[0], sb[1]),
                                dist2(sa[1], sb[0], sb[1]),
                                dist2(sb[0], sa[0], sa[1]),
                                dist2(sb[1], sa[0], sa[1]),
                            ];
                            let d = d.into_iter().fold(f64::MAX, f64::min);
                            assert!(
                                d >= need - 0.004,
                                "{seed}: {} and {} are {d:.4} mm apart, centre to centre",
                                a.key,
                                b.key
                            );
                        }
                    }
                }
            }
        }
        let same = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6;
        for n in &s.nets {
            // a via is where a track of the net ends on each of the two layers
            for via in &n.vias {
                let layers: std::collections::HashSet<&str> = s
                    .nets
                    .iter()
                    .filter(|m| m.net == n.net)
                    .flat_map(|m| &m.tracks)
                    .filter(|t| same(t.points[0], *via) || same(*t.points.last().unwrap(), *via))
                    .map(|t| t.layer.as_str())
                    .collect();
                assert!(!layers.is_empty(), "{seed}: no track of {} ends on its via at {via:?}", n.key);
            }
        }
    }
}

/// Two entries of one net: the second changed layer on a via of the first, and the first was then laid again
/// with its via elsewhere, so that the result had tracks of the net ending on each other on two layers with
/// no via there. A via one entry borrows is its own as well.
#[test]
fn a_layer_change_on_a_via_of_another_entry_of_the_net_keeps_its_via() {
    let text = std::fs::read_to_string(format!("{}/tests/data/siblings-396.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let p: Problem = serde_json::from_str(&text).unwrap();
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    let same = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6;
    let ends = |layer: &str, net: &str| -> Vec<[f64; 2]> {
        let tracks = s.nets.iter().filter(|n| n.net == net).flat_map(|n| &n.tracks).filter(|t| t.layer == layer);
        tracks.flat_map(|t| [t.points[0], *t.points.last().unwrap()]).collect()
    };
    for net in ["N0", "N1", "N2"] {
        let vias: Vec<[f64; 2]> = s.nets.iter().filter(|n| n.net == net).flat_map(|n| n.vias.clone()).collect();
        for at in ends("F", net) {
            if ends("B", net).iter().any(|b| same(*b, at)) {
                assert_eq!(
                    vias.iter().filter(|v| same(**v, at)).count(),
                    1,
                    "{net} changes layer at {at:?}: vias {vias:?}"
                );
            }
        }
    }
}

/// The trunk of a pair runs between two pieces: two terminals, or several that stand for two. One piece and a
/// terminal alone will do; three pieces, or one, will not.
#[test]
fn a_trunk_has_two_ends_however_many_places_each_is_offered() {
    let trunk = |terminals: Value| {
        let mut v = small(vec![
            json!({"key": "t", "net": "~t", "width": 0.5, "via": [0.45, 0.2], "layers": ["F"], "clearance": {"default": 0.17},
                   "no_vias": true, "terminals": terminals}),
        ]);
        for (key, net, at) in
            [("a0", "P", [1.0, 2.0]), ("b0", "M", [1.0, 4.0]), ("a1", "P", [9.0, 2.0]), ("b1", "M", [9.0, 4.0])]
        {
            v["nets"]
                .as_array_mut()
                .unwrap()
                .push(json!({"key": key, "net": net, "width": 0.15, "via": [0.45, 0.2], "layers": ["F"],
                "clearance": {"default": 0.17}, "last": true, "terminals": [{"xy": at, "layers": ["F"]}]}));
        }
        v["pairs"] = json!([{"key": "p", "trunk": "t", "legs": ["P", "M"], "leg_width": 0.15, "pitch": 0.3, "stubs": [["a0", "b0"], ["a1", "b1"]]}]);
        problem(v).validate()
    };
    let at = |x: f64, piece: Option<u32>| match piece {
        Some(piece) => json!({"xy": [x, 3.0], "layers": ["F"], "piece": piece}),
        None => json!({"xy": [x, 3.0], "layers": ["F"]}),
    };
    assert!(trunk(json!([at(2.0, None), at(8.0, None)])).is_ok());
    assert!(trunk(json!([at(2.0, Some(0)), at(2.5, Some(0)), at(8.0, Some(1)), at(7.5, Some(1))])).is_ok());
    assert!(trunk(json!([at(2.0, Some(0)), at(2.5, Some(0)), at(8.0, None)])).is_ok());
    for wrong in [
        json!([at(2.0, Some(0)), at(5.0, Some(1)), at(8.0, Some(2))]),
        json!([at(2.0, Some(0)), at(2.5, Some(0))]),
        json!([at(2.0, None)]),
    ] {
        assert!(trunk(wrong.clone()).is_err(), "{wrong}"); // (as a trunk, or already as an entry with nothing to join)
    }
}

/// A size given in the wrong unit is refused before anything is drawn up for it.
#[test]
fn a_width_or_a_clearance_wider_than_the_board_is_refused() {
    for (field, at) in [("width", "nets[0] (a).width"), ("clearance", "nets[0] (a).clearance.default")] {
        let mut v = small(vec![net("a", [1.0, 3.0], [9.0, 3.0], &["F"])]);
        match field {
            "width" => v["nets"][0]["width"] = json!(250.0),
            _ => v["nets"][0]["clearance"]["default"] = json!(250.0),
        }
        assert!(matches!(problem(v).validate(), Err(Error::Invalid { field, .. }) if field == at));
    }
    let mut v = small(vec![net("a", [1.0, 3.0], [9.0, 3.0], &["F"])]);
    v["rules"] = json!({"new_clearance_mm": 500.0});
    assert!(matches!(problem(v).validate(), Err(Error::Invalid { field, .. }) if field == "rules.new_clearance_mm"));
}
