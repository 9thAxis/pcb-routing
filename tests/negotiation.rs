//! Small cases with known answers.

use pcb_routing::{route, Problem, Router, Solution, Status};
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

/// A trunk offered several places at either end runs between the two that come cheapest: here the nearer
/// place at the first end is closed by copper, and the pair is laid all the same.
#[test]
fn a_trunk_with_places_to_choose_from_at_each_end_is_laid_from_one_that_is_open() {
    let places = json!([
        {"xy": [2.0, 3.0], "layers": ["F"], "piece": 0}, {"xy": [3.2, 3.0], "layers": ["F"], "piece": 0},
        {"xy": [8.0, 3.0], "layers": ["F"], "piece": 1}, {"xy": [7.4, 3.0], "layers": ["F"], "piece": 1}
    ]);
    let stub = |key: &str, net: &str, at: [f64; 2]| {
        entry(key, net, 0.15, json!([{"xy": at, "layers": ["F"]}]), json!({"last": true, "layers": ["F"]}))
    };
    let nets = vec![
        entry("pair", "~pair", 0.59, places, json!({"no_vias": true, "layers": ["F"]})),
        stub("p0", "P", [1.0, 2.5]),
        stub("m0", "M", [1.0, 3.5]),
        stub("p1", "P", [9.0, 2.5]),
        stub("m1", "M", [9.0, 3.5]),
    ];
    let v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F"], "nets": nets,
                   "copper": [{"t": "circle", "c": [2.0, 3.0], "r": 0.2, "net": "X", "layers": ["F"]}],
                   "pairs": [{"key": "in1", "trunk": "pair", "legs": ["P", "M"], "leg_width": 0.15, "pitch": 0.3, "stubs": [["p0", "m0"], ["p1", "m1"]]}]});
    let p: Problem = serde_json::from_value(v).unwrap();
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{} {:?}", s.message, s.conflicts);
    assert!(joined(&s, 0, [1.0, 2.5], [9.0, 2.5]) && joined(&s, 1, [1.0, 3.5], [9.0, 3.5]));
}

/// Length of the first net's track that lies within `reach` of the second's, sampled every 0.05 mm.
fn beside(s: &Solution, a: usize, b: usize, reach: f64) -> f64 {
    let near = |q: [f64; 2]| {
        s.nets[b].tracks.iter().flat_map(|t| t.points.windows(2)).any(|w| {
            let (vx, vy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let t = (((q[0] - w[0][0]) * vx + (q[1] - w[0][1]) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
            (q[0] - w[0][0] - t * vx).hypot(q[1] - w[0][1] - t * vy) <= reach
        })
    };
    let mut total = 0.0;
    for w in s.nets[a].tracks.iter().flat_map(|t| t.points.windows(2)) {
        let len = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        let pieces = (len / 0.05).ceil().max(1.0) as usize;
        for k in 0..pieces {
            let f = (k as f64 + 0.5) / pieces as f64;
            if near([w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f]) {
                total += len / pieces as f64;
            }
        }
    }
    total
}

/// Two nets of a bus whose shortest ways lie a millimetre apart: drawn together, one comes over to run beside
/// the other for most of its length, at the price of a little more track; not drawn, each keeps its own way.
#[test]
fn nets_of_a_bus_run_beside_each_other() {
    // (in either order: the net laid first is drawn to the other in the closing passes)
    for b_first in [false, true] {
        let run = |pull: f64| {
            let mut a = net("a", [1.0, 1.0], [9.0, 1.0], &["F"]);
            let mut b = net("b", [1.0, 2.4], [9.0, 3.4], &["F"]);
            a["bus"] = json!("d");
            b["bus"] = json!("d");
            let nets = if b_first { vec![b, a] } else { vec![a, b] };
            let mut v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F"], "nets": nets});
            v["options"] = json!({"bus_pull": pull});
            let p: Problem = serde_json::from_value(v).unwrap();
            let s = Router::new(&p).verify_search(true).run().unwrap();
            assert_eq!(s.status, Status::Routed, "{}", s.message);
            assert_eq!(s.stats.mismatches, 0);
            (beside(&s, 1, 0, 0.7), s.nets.iter().map(|n| n.length_mm).sum::<f64>())
        };
        let (apart, shortest) = run(0.0);
        let (together, longer) = run(0.5);
        assert!(apart < 1.0, "not drawn together, {apart:.2} mm runs beside");
        assert!(together > 4.0, "drawn together (b first: {b_first}), only {together:.2} mm runs beside");
        assert!(longer > shortest && longer < shortest + 2.5, "{shortest:.2} mm apart, {longer:.2} mm together");
    }
}

#[test]
fn a_pull_beyond_reason_is_refused() {
    for pull in [-0.1, 11.0, f64::NAN] {
        let mut v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F"], "nets": [net("a", [1.0, 1.0], [9.0, 1.0], &["F"])]});
        v["options"] = json!({"bus_pull": pull});
        assert!(
            serde_json::from_value::<Problem>(v).map_or(true, |p| route(&p).is_err()),
            "a pull of {pull} was accepted"
        );
    }
}

/// A net of no bus, and nets of different buses, are not drawn to anything: the result is the same whatever
/// the pull.
#[test]
fn only_nets_of_one_bus_are_drawn_together() {
    let run = |pull: f64, buses: [Option<&str>; 2]| {
        let mut nets = vec![net("a", [1.0, 1.0], [9.0, 1.0], &["F"]), net("b", [1.0, 2.4], [9.0, 3.4], &["F"])];
        for (n, bus) in nets.iter_mut().zip(buses) {
            if let Some(bus) = bus {
                n["bus"] = json!(bus);
            }
        }
        let mut v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F"], "nets": nets});
        v["options"] = json!({"bus_pull": pull});
        let mut s = route(&serde_json::from_value::<Problem>(v).unwrap()).unwrap();
        s.stats = Default::default();
        s
    };
    let plain = run(0.0, [None, None]);
    assert_eq!(run(0.5, [None, None]), plain);
    assert_eq!(run(0.5, [Some("d"), None]), plain);
    assert_eq!(run(0.5, [Some("d"), Some("e")]), plain);
}

/// Three nets of a bus with through-hole pads a millimetre apart, free to use either layer. With their lanes
/// given, the outer two leave room for the middle one: all three run on one layer, in the order of their pads,
/// each beside the next.
#[test]
fn nets_of_a_bus_keep_their_lanes() {
    let both = ["F", "B"];
    let nets: Vec<Value> = [("a", 2.0), ("b", 3.0), ("c", 4.0)]
        .iter()
        .enumerate()
        .map(|(lane, &(key, y))| {
            let mut n = net(key, [1.0, y], [9.0, y], &both);
            n["terminals"] = json!([{"xy": [1.0, y], "layers": both}, {"xy": [9.0, y], "layers": both}]);
            n["bus"] = json!("d");
            n["bus_lane"] = json!(lane);
            n
        })
        .collect();
    let mut v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": both, "nets": nets});
    v["options"] = json!({"bus_pull": 0.5});
    let p: Problem = serde_json::from_value(v).unwrap();
    let s = route(&p).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    let layers: std::collections::HashSet<&str> =
        s.nets.iter().flat_map(|n| &n.tracks).map(|t| t.layer.as_str()).collect();
    assert_eq!(layers.len(), 1, "the bus is on {layers:?}");
    assert!(s.nets.iter().all(|n| n.vias.is_empty()));
    assert!(beside(&s, 1, 0, 0.7) > 4.0 && beside(&s, 1, 2, 0.7) > 4.0, "b is not beside both a and c");
    // half way along, in the order of the pads
    let at = |n: usize| -> f64 {
        let cross = s.nets[n]
            .tracks
            .iter()
            .flat_map(|t| t.points.windows(2))
            .find(|w| (w[0][0] - 5.0) * (w[1][0] - 5.0) <= 0.0 && w[0][0] != w[1][0]);
        let w = cross.expect("the track passes x = 5");
        w[0][1] + (w[1][1] - w[0][1]) * (5.0 - w[0][0]) / (w[1][0] - w[0][0])
    };
    assert!(at(0) < at(1) && at(1) < at(2), "half way: a at {}, b at {}, c at {}", at(0), at(1), at(2));
}

/// A net whose shorter way passes under another net of its bus on the other layer, and whose longer way stays
/// on its own side: with a price on running over or under the bus it takes the longer way; without, the shorter.
#[test]
fn a_net_of_a_bus_is_slow_to_pass_under_another() {
    let both = ["F", "B"];
    let run = |cross: f64| {
        let mut a = net("a", [1.0, 3.0], [9.0, 3.0], &["F"]);
        let mut b = net("b", [4.0, 4.0], [6.0, 4.0], &both);
        b["terminals"] = json!([{"xy": [4.0, 4.0], "layers": both}, {"xy": [6.0, 4.0], "layers": both}]);
        a["bus"] = json!("d");
        b["bus"] = json!("d");
        // A wall below a, on both layers: b gets past it above (under a, on the other layer) or below. And one
        // above a on its own layer, so that a cannot make room beside itself.
        let above = json!({"t": "seg", "a": [1.5, 2.6], "b": [8.5, 2.6], "w": 0.2, "net": "WALL", "layers": ["F"]});
        let mut v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": both,
                           "copper": [wall(5.0, 3.4, 5.0, &both), above], "nets": [a, b]});
        v["options"] = json!({"bus_cross": cross});
        let s = route(&serde_json::from_value::<Problem>(v).unwrap()).unwrap();
        assert_eq!(s.status, Status::Routed, "{}", s.message);
        s.nets[1].tracks.iter().flat_map(|t| &t.points).map(|p| p[1]).fold(f64::MAX, f64::min)
    };
    let (free, dear) = (run(0.0), run(6.0));
    assert!(free < 3.3, "free to pass under, b comes no nearer a than y = {free}");
    assert!(dear > 3.9, "with a price on passing under, b goes up to y = {dear}");
}

/// Lanes may be any numbers: nets far apart in lanes are simply not drawn to each other.
#[test]
fn lanes_of_any_size_are_taken() {
    let mut nets = vec![net("a", [1.0, 1.0], [9.0, 1.0], &["F"]), net("b", [1.0, 2.4], [9.0, 3.4], &["F"])];
    for (n, lane) in nets.iter_mut().zip([i32::MIN, i32::MAX]) {
        n["bus"] = json!("d");
        n["bus_lane"] = json!(lane);
    }
    let v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F"], "nets": nets, "options": {"bus_pull": 0.5}});
    let s = route(&serde_json::from_value::<Problem>(v).unwrap()).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    assert!(beside(&s, 1, 0, 0.7) < 1.0, "nets lanes apart were drawn together");
}

/// With no pull, a bus name changes nothing at all.
#[test]
fn bus_names_without_pull_change_nothing() {
    let run = |named: bool| {
        let mut nets = vec![
            net("a", [1.0, 1.0], [9.0, 1.2], &["F", "B"]),
            net("b", [1.0, 2.4], [9.0, 3.4], &["F", "B"]),
            net("c", [5.0, 0.5], [5.0, 5.5], &["F", "B"]),
        ];
        if named {
            nets.iter_mut().for_each(|n| n["bus"] = json!("d"));
        }
        let v = json!({"window": [0.0, 0.0, 10.0, 6.0], "step": 0.05, "layers": ["F", "B"], "nets": nets, "options": {"bus_pull": 0.0}});
        let mut s = route(&serde_json::from_value::<Problem>(v).unwrap()).unwrap();
        s.stats = Default::default();
        s
    };
    assert_eq!(run(true), run(false));
}

/// Track length of a result's entry, in mm.
fn length(s: &Solution, n: usize) -> f64 {
    let far = |a: &[f64; 2], b: &[f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    s.nets[n].tracks.iter().flat_map(|t| t.points.windows(2)).map(|w| far(&w[0], &w[1])).sum()
}

fn terminal(xy: [f64; 2], piece: Option<u32>) -> Value {
    json!({"xy": xy, "layers": ["F"], "piece": piece})
}

#[test]
fn a_piece_of_copper_is_joined_at_its_nearest_terminal_only() {
    // one piece with points at x 1 and x 8, a pad at x 9: joined to the point at 8, and nothing laid between
    // the two points of the piece
    let piece = vec![terminal([1.0, 3.0], Some(7)), terminal([8.0, 3.0], Some(7)), terminal([9.0, 3.0], None)];
    let s = route(&problem(&["F", "B"], vec![], vec![entry("a", "A", 0.15, json!(piece), json!({}))])).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    assert!((length(&s, 0) - 1.0).abs() < 1e-6, "{} mm laid", length(&s, 0));
    // whichever comes first in the list
    let piece = vec![terminal([9.0, 3.0], None), terminal([1.0, 3.0], Some(7)), terminal([8.0, 3.0], Some(7))];
    let s = route(&problem(&["F", "B"], vec![], vec![entry("a", "A", 0.15, json!(piece), json!({}))])).unwrap();
    assert!((length(&s, 0) - 1.0).abs() < 1e-6, "{} mm laid", length(&s, 0));
}

#[test]
fn a_piece_is_reached_where_it_is_open() {
    // the nearer point of the piece is walled in: the pad is joined to the other, and nothing is reported
    let box_ = |x: f64| vec![wall(x - 0.5, 2.5, 3.5, &["F", "B"]), wall(x + 0.5, 2.5, 3.5, &["F", "B"])];
    let mut copper = box_(8.0);
    copper.push(json!({"t": "seg", "a": [7.5, 2.5], "b": [8.5, 2.5], "w": 0.2, "net": "WALL", "layers": ["F", "B"]}));
    copper.push(json!({"t": "seg", "a": [7.5, 3.5], "b": [8.5, 3.5], "w": 0.2, "net": "WALL", "layers": ["F", "B"]}));
    let piece = vec![terminal([9.5, 3.0], None), terminal([8.0, 3.0], Some(1)), terminal([2.0, 3.0], Some(1))];
    let s = route(&problem(&["F", "B"], copper, vec![entry("a", "A", 0.15, json!(piece), json!({}))])).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    assert!(s.unrouted.is_empty(), "{:?}", s.unrouted);
    assert!(joined(&s, 0, [9.5, 3.0], [2.0, 3.0]));
}

#[test]
fn two_pieces_are_joined_once() {
    // two pieces of two points each, opposite each other: one connection, between the nearest two
    let terminals = vec![
        terminal([2.0, 1.0], Some(1)),
        terminal([2.0, 5.0], Some(1)),
        terminal([4.0, 1.0], Some(2)),
        terminal([8.0, 5.0], Some(2)),
    ];
    let s = route(&problem(&["F", "B"], vec![], vec![entry("a", "A", 0.15, json!(terminals), json!({}))])).unwrap();
    assert_eq!(s.status, Status::Routed, "{}", s.message);
    assert!((length(&s, 0) - 2.0).abs() < 1e-6, "{} mm laid", length(&s, 0));
}

#[test]
fn terminals_of_one_piece_alone_are_nothing_to_join() {
    let terminals = vec![terminal([2.0, 1.0], Some(1)), terminal([2.0, 5.0], Some(1))];
    let p = problem(&["F", "B"], vec![], vec![entry("a", "A", 0.15, json!(terminals), json!({}))]);
    let why = route(&p).unwrap_err().to_string();
    assert!(why.contains("not yet joined"), "{why}");
}

fn dear(x: [f64; 2], y: [f64; 2], factor: f64, net: Option<&str>) -> Value {
    json!({"rings": [[[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]]], "layers": ["F"], "factor": factor, "net": net})
}

/// A problem with one net straight across dear ground from x 4 to 6, which ends at y 4 (and at y 0, the window's edge).
fn across_dear_ground(factor: f64, of: Option<&str>) -> Problem {
    let mut p = problem(&["F"], vec![], vec![net("a", [2.0, 3.0], [8.0, 3.0], &["F"])]);
    p.dear = vec![serde_json::from_value(dear([4.0, 6.0], [-1.0, 4.0], factor, of)).unwrap()];
    p
}

#[test]
fn dear_ground_is_gone_round_when_that_is_cheaper() {
    // straight through: 2 mm of dear ground; round it: about 0.9 mm more track
    let through = route(&across_dear_ground(1.2, None)).unwrap();
    assert!((length(&through, 0) - 6.0).abs() < 1e-6, "{} mm laid", length(&through, 0));
    let round = route(&across_dear_ground(3.0, None)).unwrap();
    assert_eq!(round.status, Status::Routed);
    assert!(length(&round, 0) > 6.5, "{} mm laid", length(&round, 0));
    let highest = round.nets[0].tracks.iter().flat_map(|t| &t.points).map(|p| p[1]).fold(0.0, f64::max);
    assert!(highest > 4.0, "the track stays below y {highest}");
}

#[test]
fn dear_ground_costs_its_own_net_nothing() {
    let s = route(&across_dear_ground(3.0, Some("a"))).unwrap();
    assert!((length(&s, 0) - 6.0).abs() < 1e-6, "{} mm laid", length(&s, 0));
}

#[test]
fn a_via_is_kept_out_of_dear_ground() {
    // A wall on F from top to bottom at x 5, passed on B. Without dear ground the net comes back up at its
    // terminal; with the terminal in a patch of dear ground on F it comes up before the patch, and pays for
    // 0.2 mm of track in it rather than for a via.
    let mut p =
        problem(&["F", "B"], vec![wall(5.0, -1.0, 7.0, &["F"])], vec![net("a", [2.0, 3.0], [7.5, 3.0], &["F", "B"])]);
    let via_at = |p: &Problem| route(p).unwrap().nets[0].vias.clone();
    let plain = via_at(&p);
    assert_eq!(plain.len(), 2, "{plain:?}");
    p.dear = vec![serde_json::from_value(dear([7.3, 7.7], [2.8, 3.2], 20.0, None)).unwrap()];
    let kept_out = via_at(&p);
    assert_eq!(kept_out.len(), 2, "{kept_out:?}");
    assert!(plain.iter().any(|v| v[0] > 7.3 && v[0] < 7.7), "without dear ground the via is not there: {plain:?}");
    assert!(kept_out.iter().all(|v| v[0] < 7.3 || v[0] > 7.7), "{kept_out:?}");
}

#[test]
fn a_factor_below_one_is_refused() {
    let why = route(&across_dear_ground(0.5, None)).unwrap_err().to_string();
    assert!(why.contains("dear[0].factor"), "{why}");
}
