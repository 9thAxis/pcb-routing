//! Small cases with known answers.

use pcb_routing::{route, Problem, Solution, Status};
use serde_json::{json, Value};

fn net(key: &str, a: [f64; 2], b: [f64; 2], layers: &[&str]) -> Value {
    json!({"key": key, "net": key, "width": 0.15, "via": [0.45, 0.2], "layers": layers, "via_penalty_mm": 3.0,
           "clearance": {"default": 0.17},
           "terminals": [{"xy": a, "layers": [layers[0]]}, {"xy": b, "layers": [layers[0]]}]})
}

fn wall(x: f64, y0: f64, y1: f64, layers: &[&str]) -> Value {
    json!({"t": "seg", "a": [x, y0], "b": [x, y1], "w": 0.2, "net": "WALL", "layers": layers})
}

fn problem(layers: &[&str], copper: Vec<Value>, nets: Vec<Value>) -> Problem {
    serde_json::from_value(
        json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": layers, "copper": copper, "nets": nets}),
    )
    .unwrap()
}

/// Smallest distance between the track centre lines of two nets on a shared layer, and between any via of one
/// and the copper of the other.
fn spacing(s: &Solution, a: usize, b: usize) -> f64 {
    let seg = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let (vx, vy, wx, wy) = (b[0] - a[0], b[1] - a[1], p[0] - a[0], p[1] - a[1]);
        let l2 = vx * vx + vy * vy;
        let t = if l2 > 0.0 { ((wx * vx + wy * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        ((wx - t * vx).powi(2) + (wy - t * vy).powi(2)).sqrt()
    };
    let mut best = f64::INFINITY;
    for ta in &s.nets[a].tracks {
        for tb in &s.nets[b].tracks {
            if ta.layer != tb.layer {
                continue;
            }
            for sa in ta.points.windows(2) {
                for sb in tb.points.windows(2) {
                    // sample one segment finely against the other
                    for k in 0..=200 {
                        let f = k as f64 / 200.0;
                        let p = [sa[0][0] + f * (sa[1][0] - sa[0][0]), sa[0][1] + f * (sa[1][1] - sa[0][1])];
                        best = best.min(seg(p, sb[0], sb[1]));
                    }
                }
            }
        }
    }
    best
}

fn three_lanes() -> Problem {
    // a wall with a gap that takes two tracks; the wall ends at y 1 and y 5, so a third track can go round
    problem(
        &["F"],
        vec![wall(5.0, 1.0, 2.4, &["F"]), wall(5.0, 3.6, 5.0, &["F"])],
        vec![
            net("a", [2.0, 2.6], [8.0, 2.6], &["F"]),
            net("b", [2.0, 3.0], [8.0, 3.0], &["F"]),
            net("c", [2.0, 3.4], [8.0, 3.4], &["F"]),
        ],
    )
}

#[test]
fn two_nets_share_a_gap() {
    // both want the middle of the gap; each must move aside
    let p = problem(
        &["F"],
        vec![wall(5.0, 0.0, 2.4, &["F"]), wall(5.0, 3.6, 6.0, &["F"])],
        vec![net("a", [2.0, 2.8], [8.0, 2.8], &["F"]), net("b", [2.0, 3.2], [8.0, 3.2], &["F"])],
    );
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{:?}", s.conflicts);
    assert!(spacing(&s, 0, 1) >= 0.15 + 0.15 - 1e-6, "centre lines {} mm apart", spacing(&s, 0, 1));
    assert!(s.nets.iter().all(|n| n.length_mm < 6.6), "{:?}", s.nets.iter().map(|n| n.length_mm).collect::<Vec<_>>());
}

#[test]
fn third_net_goes_round() {
    let s = route(&three_lanes()).unwrap();
    assert_eq!(s.status, Status::Routed, "{:?}", s.conflicts);
    let mut lengths: Vec<f64> = s.nets.iter().map(|n| n.length_mm).collect();
    lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(lengths[1] < 6.6 && lengths[2] > 7.5, "{lengths:?}");
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert!(spacing(&s, a, b) >= 0.3 - 1e-6);
    }
}

#[test]
fn a_wall_is_crossed_on_another_layer() {
    let p =
        problem(&["F", "B"], vec![wall(5.0, -1.0, 7.0, &["F"])], vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F", "B"])]);
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed);
    assert_eq!(s.nets[0].vias.len(), 2);
    assert!(s.nets[0].tracks.iter().any(|t| t.layer == "B"));
    assert!((s.nets[0].length_mm - 6.0).abs() < 0.01);
}

#[test]
fn two_nets_cross_with_a_layer_change() {
    let mut nets = vec![net("a", [0.3, 3.0], [9.7, 3.0], &["F", "B"]), net("b", [5.0, 0.3], [5.0, 5.7], &["F", "B"])];
    nets[0]["layers"] = json!(["F"]); // the first may not leave the top layer; both span the window, so neither can go round the other: the second must go under
    let s = route(&problem(&["F", "B"], vec![], nets)).unwrap();
    assert_eq!(s.status, Status::Routed, "{:?}", s.conflicts);
    assert_eq!(s.nets[0].vias.len(), 0);
    assert_eq!(s.nets[1].vias.len(), 2);
}

#[test]
fn too_many_nets_for_a_gap_is_reported_by_name() {
    // the gap takes one track and the wall has no end: the two nets cannot both pass
    let mut p = problem(
        &["F"],
        vec![wall(5.0, -1.0, 2.6, &["F"]), wall(5.0, 3.4, 7.0, &["F"])],
        vec![net("a", [2.0, 2.8], [8.0, 2.8], &["F"]), net("b", [2.0, 3.2], [8.0, 3.2], &["F"])],
    );
    p.options.max_iterations = 25;
    p.options.drop_conflicts = false;
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Conflicts);
    assert!(s.conflicts.iter().any(|c| c.key == "a" && c.with == vec!["b".to_string()]));
    assert!(s.conflicts.iter().any(|c| c.key == "b" && c.with == vec!["a".to_string()]));
}

#[test]
fn a_net_with_no_room_is_left_out_and_the_rest_is_clean() {
    // the same gap for one: by default the result breaks no rule, one net is laid and the other is listed
    let mut p = problem(
        &["F"],
        vec![wall(5.0, -1.0, 2.6, &["F"]), wall(5.0, 3.4, 7.0, &["F"])],
        vec![net("a", [2.0, 2.8], [8.0, 2.8], &["F"]), net("b", [2.0, 3.2], [8.0, 3.2], &["F"])],
    );
    p.options.max_iterations = 25;
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Unroutable);
    assert!(s.conflicts.is_empty());
    assert_eq!(s.unrouted.len(), 1, "{:?}", s.unrouted);
    assert_eq!(s.nets.iter().filter(|n| !n.tracks.is_empty()).count(), 1);
}

#[test]
fn a_walled_in_terminal_is_reported() {
    let p = problem(&["F"], vec![wall(5.0, -1.0, 7.0, &["F"])], vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Unroutable);
    assert!(s.message.starts_with("a: no path"), "{}", s.message);
}

#[test]
fn a_net_with_three_terminals_is_one_tree() {
    let mut n = net("a", [2.0, 3.0], [8.0, 3.0], &["F"]);
    n["terminals"].as_array_mut().unwrap().push(json!({"xy": [5.0, 5.0], "layers": ["F"]}));
    let s = route(&problem(&["F"], vec![], vec![n])).unwrap();
    assert_eq!(s.status, Status::Routed);
    // a straight run of 6 mm and a branch to the third terminal that leaves it at 45 degrees: 6 + 2 * sqrt(2) - 2 + ... = 7.66 mm,
    // shorter than the 8 mm of a square branch from the middle
    assert!(
        (s.nets[0].length_mm - (4.0 + 2.0 * 2.0f64.sqrt() + 2.0 * (2.0f64.sqrt() - 1.0))).abs() < 0.01,
        "{}",
        s.nets[0].length_mm
    );
}

#[test]
fn a_slanted_connection_has_one_corner() {
    // 6 mm across and 2 mm up: many staircases have the same length; with a corner cost the path is two straight pieces
    let s = route(&problem(&["F"], vec![], vec![net("a", [2.0, 2.0], [8.0, 4.0], &["F"])])).unwrap();
    assert_eq!(s.nets[0].tracks.len(), 1);
    assert_eq!(s.nets[0].tracks[0].points.len(), 3, "{:?}", s.nets[0].tracks[0].points);
    assert!((s.nets[0].length_mm - (4.0 + 2.0 * 2.0f64.sqrt())).abs() < 0.01);
}

#[test]
fn same_problem_same_answer() {
    let mut p = three_lanes();
    let strip = |mut s: Solution| {
        s.stats = Default::default(); // times differ from run to run
        s
    };
    assert_eq!(strip(route(&p).unwrap()), strip(route(&p).unwrap()));
    p.options.variants = 6;
    p.options.threads = 1;
    let one = route(&p).unwrap();
    p.options.threads = 6;
    let mut many = route(&p).unwrap();
    many.stats = one.stats.clone(); // times differ from run to run
    assert_eq!(one, many);
    assert_eq!(one.status, Status::Routed);
}

/// What a laid net costs the search: length, 3 mm a via, 0.3 mm a 45 degree corner and three times that a right angle.
fn cost(s: &Solution, n: usize) -> f64 {
    let mut total = 3.0 * s.nets[n].vias.len() as f64;
    for t in &s.nets[n].tracks {
        for seg in t.points.windows(2) {
            total += ((seg[1][0] - seg[0][0]).powi(2) + (seg[1][1] - seg[0][1]).powi(2)).sqrt();
        }
        for c in t.points.windows(3) {
            let (u, v) = ([c[1][0] - c[0][0], c[1][1] - c[0][1]], [c[2][0] - c[1][0], c[2][1] - c[1][1]]);
            let turn = ((u[0] * v[0] + u[1] * v[1])
                / ((u[0] * u[0] + u[1] * u[1]).sqrt() * (v[0] * v[0] + v[1] * v[1]).sqrt()))
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
            total += if turn < 1.0 {
                0.0
            } else if turn < 46.0 {
                0.3
            } else {
                0.9
            };
        }
    }
    total
}

#[test]
fn two_passes_find_what_one_pass_finds() {
    // walls that force detours, corners and layer changes; each connection laid alone, with and without the first pass
    let copper = vec![
        wall(2.5, 0.0, 4.5, &["F", "B"]),
        wall(4.0, 1.5, 6.0, &["F"]),
        wall(4.0, 0.0, 4.0, &["B"]),
        wall(6.0, 0.0, 3.2, &["F"]),
        wall(6.0, 2.0, 6.0, &["B"]),
        wall(7.5, 1.0, 5.0, &["F", "B"]),
    ];
    let ends = [
        ([1.0, 1.0], [9.0, 5.0]),
        ([1.0, 5.0], [9.0, 1.0]),
        ([1.0, 3.0], [9.0, 3.0]),
        ([3.2, 0.5], [6.8, 5.5]),
        ([0.5, 0.5], [5.0, 3.0]),
    ];
    for (k, (a, b)) in ends.iter().enumerate() {
        let mut costs = Vec::new();
        for two_pass in [true, false] {
            let mut p = problem(&["F", "B"], copper.clone(), vec![net("n", *a, *b, &["F", "B"])]);
            p.options.two_pass = two_pass;
            let s = route(&p).unwrap();
            assert_eq!(s.status, Status::Routed, "connection {k}, two_pass {two_pass}: {}", s.message);
            costs.push(cost(&s, 0));
        }
        assert!(
            (costs[0] - costs[1]).abs() < 0.02,
            "connection {k}: two passes {:.3} mm, one pass {:.3} mm",
            costs[0],
            costs[1]
        );
    }
}

fn entry(key: &str, net: &str, width: f64, terminals: Value, more: Value) -> Value {
    let mut v = json!({"key": key, "net": net, "width": width, "via": [0.45, 0.2], "layers": ["F", "B"], "via_penalty_mm": 3.0,
                       "clearance": {"default": 0.17}, "terminals": terminals});
    for (k, x) in more.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

/// True if the tracks of a net join the two points (ends that coincide are joined; so are ends on a via).
fn joined(s: &Solution, n: usize, a: [f64; 2], b: [f64; 2]) -> bool {
    let same = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6;
    let mut reached = vec![a];
    let mut grew = true;
    while grew {
        grew = false;
        for t in &s.nets[n].tracks {
            let on: Vec<bool> = t.points.iter().map(|p| reached.iter().any(|r| same(*r, *p))).collect();
            if on.iter().any(|&x| x) && !on.iter().all(|&x| x) {
                for p in &t.points {
                    if !reached.iter().any(|r| same(*r, *p)) {
                        reached.push(*p);
                    }
                }
                grew = true;
            }
        }
    }
    reached.iter().any(|r| same(*r, b))
}

#[test]
fn a_pair_runs_side_by_side_and_reaches_its_pads() {
    // pads 2 mm apart at either end, a wall the pair must go round, and an ordinary net that wants the same way
    let stub = |key: &str, net: &str, at: [f64; 2]| {
        entry(key, net, 0.15, json!([{"xy": at, "layers": ["F"]}]), json!({"last": true, "quiet": true}))
    };
    let nets = vec![
        entry(
            "pair",
            "~pair",
            0.59,
            json!([{"xy": [2.0, 3.0], "layers": ["F", "B"]}, {"xy": [8.0, 3.0], "layers": ["F", "B"]}]),
            json!({"no_vias": true, "quiet": true, "yield": 0.2}),
        ),
        stub("p0", "P", [1.0, 2.0]),
        stub("m0", "M", [1.0, 4.0]),
        stub("p1", "P", [9.0, 2.5]),
        stub("m1", "M", [9.0, 3.5]),
        net("other", [2.0, 3.4], [8.0, 3.4], &["F", "B"]),
    ];
    let mut v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F", "B"], "copper": [wall(5.0, 2.0, 6.0, &["F", "B"])], "nets": nets,
                       "rules": {"quiet_clearance_mm": 0.52},
                       "pairs": [{"key": "in1", "trunk": "pair", "legs": ["P", "M"], "leg_width": 0.15, "pitch": 0.3, "stubs": [["p0", "m0"], ["p1", "m1"]]}]});
    v["options"] = json!({"variants": 1});
    let p: Problem = serde_json::from_value(v).unwrap();
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{} {:?}", s.message, s.conflicts);
    let keys: Vec<&str> = s.nets.iter().map(|n| n.key.as_str()).collect();
    assert_eq!(keys, ["other", "in1/P", "in1/M"]);
    assert!(joined(&s, 1, [1.0, 2.0], [9.0, 2.5]), "P is not one piece from pad to pad");
    assert!(joined(&s, 2, [1.0, 4.0], [9.0, 3.5]), "M is not one piece from pad to pad");
    let between = spacing(&s, 1, 2);
    assert!(between > 0.2999, "legs {between} mm apart, centre to centre");
    // the legs keep exactly their pitch along the trunk: the first track of each net is its leg
    assert!(s.nets[1].tracks[0].points.len() >= 3, "the leg should have gone round the wall");
    // the ordinary net keeps the quiet distance from both: 0.52 mm edge to edge, 0.67 centre to centre
    for leg in [1, 2] {
        let d = spacing(&s, 0, leg);
        assert!(d > 0.6699, "ordinary net {d} mm from a leg, centre to centre");
    }
}
