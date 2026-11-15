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
