//! Zones from outside: a round with `pcb_routing::pours` as a caller without KiCad would make it, and the
//! same questions as JSON, to the library and to the program.

use pcb_routing::pours::{self, Answer, Ask, Fill, Layout, Pad};
use pcb_routing::{route, Problem, Status};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

/// A pad 1 mm square on the front, as a question names it.
fn land(net: &str, x: f64, y: f64) -> Value {
    json!({"net": net, "at": [x, y], "layers": ["F"], "shapes": {"F": [[square(x - 0.5, y - 0.5, x + 0.5, y + 0.5)]]}})
}

/// A zone of GND on the front with two pads on it and one beside it, each pad a group of its own.
fn board() -> Value {
    json!({
        "pads": [land("GND", 2.0, 2.0), land("GND", 8.0, 8.0), land("GND", 14.0, 5.0)],
        "fills": [{"net": "GND", "layer": "F", "rings": [square(0.0, 0.0, 10.0, 10.0)]}],
        "groups": {"GND": [[0], [1], [2]]},
    })
}

fn asked(question: Value) -> Value {
    let ask: Ask = serde_json::from_value(question).unwrap();
    serde_json::to_value(pours::answer(&ask).unwrap()).unwrap()
}

#[test]
fn a_net_is_taken_to_its_zone_and_what_was_laid_hangs_on_it() {
    let layout: Layout = serde_json::from_value(board()).unwrap();
    let survey = pours::survey(&layout).unwrap();
    assert_eq!(survey.groups["GND"], [vec![0, 1], vec![2]]); // the zone joins the two pads on it
    assert_eq!(survey.pours["GND"], [vec![0], vec![]]);

    let layers = vec!["F".to_string()];
    let places =
        pours::reach(&layout.pads, &layout.fills, &survey.groups["GND"], &survey.pours["GND"], &layers, 0.2, Some(0.1));
    assert!(places[0].len() > 2, "the pads of the group and points of the zone: {:?}", places[0]);
    assert_eq!(places[1].len(), 1);

    // the problem: the places of each group are terminals of one piece
    let terminals: Vec<Value> = places
        .iter()
        .enumerate()
        .flat_map(|(g, group)| group.iter().map(move |place| (g, group.len(), place)))
        .map(|(g, count, place)| match count {
            1 => json!({"xy": place.xy, "layers": place.layers}),
            _ => json!({"xy": place.xy, "layers": place.layers, "piece": g}),
        })
        .collect();
    let problem: Problem = serde_json::from_value(json!({
        "window": [-1.0, -1.0, 16.0, 11.0], "step": 0.1, "layers": ["F"],
        "dear": [{"rings": [square(0.0, 0.0, 10.0, 10.0)], "layers": ["F"], "factor": 2.0, "net": "GND"}],
        "nets": [{"key": "GND", "net": "GND", "width": 0.25, "via": [0.6, 0.3], "layers": ["F"],
                  "clearance": {"default": 0.2}, "terminals": terminals}],
    }))
    .unwrap();
    let solution = route(&problem).unwrap();
    assert_eq!(solution.status, Status::Routed, "{}", solution.message);
    // the pad beside the zone goes to the zone's edge, not to a pad on it: 4 mm and the way in
    assert!(solution.nets[0].length_mm < 4.6, "{} mm", solution.nets[0].length_mm);

    let parts = pours::parts(&layout.pads, &problem.nets, &solution.nets);
    assert_eq!(parts.len(), 1);
    let [end] = &parts[0].ends[..] else { panic!("one end on the zone: {:?}", parts[0].ends) };
    assert_eq!((end.net.as_str(), end.layer.as_str()), ("GND", "F"));
    assert!((9.5..10.0).contains(&end.xy[0]) && end.xy[1] == 5.0, "{:?}", end.xy);
}

#[test]
fn a_survey_as_json_names_groups_pours_and_thermals() {
    let mut question = board();
    question["ask"] = json!("survey");
    let found = asked(question);
    assert_eq!(found["groups"], json!({"GND": [[0, 1], [2]]}));
    assert_eq!(found["pours"], json!({"GND": [[0], []]}));
    assert_eq!(found["thermals"], json!([]));
}

#[test]
fn reach_as_json_gives_the_places_of_each_group_of_each_net() {
    let found = asked(json!({
        "ask": "reach", "layers": ["F"], "step": 0.1,
        "pads": [{"net": "GND", "at": [2.0, 2.0], "layers": ["F"]}, {"net": "GND", "at": [14.0, 5.0], "layers": ["F"]},
                 {"net": "A", "at": [1.0, 12.0], "layers": ["F"]}, {"net": "A", "at": [9.0, 12.0], "layers": ["B"]}],
        "fills": [{"net": "GND", "layer": "F", "rings": [square(0.0, 0.0, 10.0, 10.0)]}],
        "nets": [{"groups": [[0], [1]], "pours": [[0]], "inset": 0.2}, {"groups": [[2], [3]], "inset": 0.2}],
    }));
    assert!(found[0][0].as_array().unwrap().contains(&json!({"xy": [9.8, 5.0], "layers": ["F"]})), "{}", found[0][0]);
    assert_eq!(found[0][1], json!([{"xy": [14.0, 5.0], "layers": ["F"]}]));
    // a pad on no routing layer leaves its group without a place
    assert_eq!(found[1], json!([[{"xy": [1.0, 12.0], "layers": ["F"]}], []]));
}

#[test]
fn parts_and_ties_as_json() {
    let entry = json!({"key": "GND", "net": "GND", "width": 0.25, "via": [0.6, 0.3], "layers": ["F"], "clearance": {"default": 0.2},
                       "terminals": [{"xy": [14.0, 5.0], "layers": ["F"]}, {"xy": [9.8, 5.0], "layers": ["F"], "piece": 0}]});
    let laid = json!({"key": "GND", "net": "GND", "length_mm": 4.2, "tracks": [{"layer": "F", "points": [[14.0, 5.0], [9.8, 5.0]]}], "vias": []});
    let found = asked(
        json!({"ask": "parts", "pads": [{"net": "GND", "at": [14.0, 5.0], "layers": ["F"]}], "entries": [entry], "laid": [laid]}),
    );
    assert_eq!(found[0]["key"], "GND");
    assert_eq!(found[0]["ends"], json!([{"net": "GND", "layer": "F", "xy": [9.8, 5.0]}]));

    // a pad at (5, 5) with the zone 1 mm off all round: the line half way, and a track out to the zone
    let pad = json!({"outline": square(4.5, 4.5, 5.5, 5.5), "fill": [[square(0.0, 0.0, 10.0, 10.0), square(4.0, 4.0, 6.0, 6.0)]]});
    let mut target = pad.clone();
    target["at"] = json!([5.0, 5.0]);
    target["inset"] = json!(0.2);
    let found = asked(json!({"ask": "tie_targets", "pads": [target]}));
    assert_eq!(found[0].as_array().unwrap().len(), 8);
    let (mut tying, mut short) = (pad.clone(), pad);
    tying["points"] = json!([[5.0, 5.0], found[0][0]]);
    short["points"] = json!([[5.0, 5.0], [5.8, 5.0]]);
    assert_eq!(asked(json!({"ask": "ties", "tracks": [tying, short]})), json!([true, false]));
}

#[test]
fn a_question_that_names_what_is_not_there_is_refused() {
    let refused = |question: Value| {
        let ask: Ask = serde_json::from_value(question).unwrap();
        pours::answer(&ask).unwrap_err().to_string()
    };
    let mut survey = board();
    survey["ask"] = json!("survey");
    survey["groups"]["GND"][2] = json!([7]);
    assert!(refused(survey).contains("groups[GND][2]"));
    let reach = json!({"ask": "reach", "layers": ["F"], "pads": [], "nets": [{"groups": [[0]], "inset": 0.2}]});
    assert!(refused(reach).contains("nets[0].groups"));
    assert!(serde_json::from_value::<Ask>(json!({"ask": "nothing"})).is_err());
}

#[test]
fn the_answers_are_plain_lists_and_maps() {
    // (what a caller in another language reads: no tags, no wrappers)
    let pads =
        vec![Pad { net: Some("A".into()), at: [1.0, 1.0], layers: vec!["F".into()], shapes: Default::default() }];
    let fills: Vec<Fill> = Vec::new();
    let places = pours::reach(&pads, &fills, &[vec![0]], &[], &["F".to_string()], 0.2, None);
    assert_eq!(
        serde_json::to_value(Answer::Reach(vec![places])).unwrap(),
        json!([[[{"xy": [1.0, 1.0], "layers": ["F"]}]]])
    );
    assert_eq!(serde_json::to_value(Answer::Ties(vec![true])).unwrap(), json!([true]));
}

#[test]
fn the_program_answers_from_standard_input_and_says_why_it_cannot() {
    let run = |question: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pcb-routing"))
            .arg("pours")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(question.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let mut question = board();
    question["ask"] = json!("survey");
    let out = run(&question.to_string());
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let found: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(found["groups"]["GND"], json!([[0, 1], [2]]));
    let out = run(
        r#"{"ask": "survey", "pads": [], "spoked": [{"pad": 3, "layer": "F", "line": [], "gap": 0.5, "least": 2}]}"#,
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("spoked[0].pad"));
}
