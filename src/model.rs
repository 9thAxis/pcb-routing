//! The problem and solution files. The engine knows nothing about a particular board: everything it needs is here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Pt = [f64; 2];

#[derive(Deserialize, Clone, Debug)]
pub struct Problem {
    /// x0, y0, x1, y1 in mm. Cell (j, i) is at (x0 + i * step, y0 + j * step).
    pub window: [f64; 4],
    pub step: f64,
    /// Layers that may carry routes, in the order used for every per-layer list.
    pub layers: Vec<String>,
    /// Copper already on the board. Its layer names may include layers that carry no routes (planes): a through
    /// via must clear copper on those as well.
    #[serde(default)]
    pub copper: Vec<Copper>,
    /// Centres of holes already on the board (vias and through-hole pins).
    #[serde(default)]
    pub holes: Vec<Hole>,
    /// Ground closed to every net.
    #[serde(default)]
    pub regions: Vec<Region>,
    #[serde(default)]
    pub rules: Rules,
    pub nets: Vec<Net>,
    /// Pairs of nets to be laid side by side.
    #[serde(default)]
    pub pairs: Vec<Pair>,
    #[serde(default)]
    pub options: Options,
}

/// Two nets laid side by side. The pair is laid as one wide track, the `trunk` (an entry of `nets` with two
/// terminals, the points where the pair gathers and where it parts, both on the grid; its `net` is a name of
/// its own, so that copper of either net is an obstacle to it; `width` is at least `pitch` + `leg_width` + 0.04,
/// the last for the outside of corners; `no_vias` set). When the nets have settled the trunk is split into two
/// legs `pitch` apart, and each leg is joined to its own pads by a stub at either end: four more entries of
/// `nets`, each marked `last` and with the pad as its only terminal; the end of the leg is added to it.
#[derive(Deserialize, Clone, Debug)]
pub struct Pair {
    pub key: String,
    /// Key of the trunk entry.
    pub trunk: String,
    /// The two electrical nets.
    pub legs: [String; 2],
    pub leg_width: f64,
    /// Centre to centre distance of the legs.
    pub pitch: f64,
    /// Keys of the stub entries: `stubs[end][leg]`, end 0 at the trunk's first terminal.
    pub stubs: [[String; 2]; 2],
}

#[derive(Deserialize, Clone, Debug)]
pub struct Copper {
    pub net: Option<String>,
    pub layers: Vec<String>,
    /// A solder land: a via drill keeps `pad_drill_margin_mm` from it even on its own net.
    #[serde(default)]
    pub pad: bool,
    #[serde(flatten)]
    pub shape: Shape,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "t")]
pub enum Shape {
    #[serde(rename = "seg")]
    Seg { a: Pt, b: Pt, w: f64 },
    #[serde(rename = "circle")]
    Circle { c: Pt, r: f64 },
    /// Rings of one polygon, even-odd: the first is the outline, the others are holes.
    #[serde(rename = "poly")]
    Poly { rings: Vec<Vec<Pt>> },
}

#[derive(Deserialize, Clone, Debug)]
pub struct Hole {
    pub xy: Pt,
    pub net: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Region {
    pub rings: Vec<Vec<Pt>>,
    /// Layers closed to tracks; all routing layers if absent.
    #[serde(default)]
    pub layers: Option<Vec<String>>,
    #[serde(default = "yes")]
    pub tracks: bool,
    #[serde(default = "yes")]
    pub vias: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Rules {
    /// Smallest distance between two hole centres.
    pub hole_centre_mm: f64,
    /// A via drill edge keeps this far from any solder land.
    pub pad_drill_margin_mm: f64,
    /// A track or via edge keeps this far from a closed region.
    pub region_margin_mm: f64,
    /// Edge-to-edge distance between copper of two nets routed in the same run.
    pub new_clearance_mm: f64,
    /// The same between a net marked `quiet` and one that is not (0: as `new_clearance_mm`).
    pub quiet_clearance_mm: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { hole_centre_mm: 0.56, pad_drill_margin_mm: 0.11, region_margin_mm: 0.025, new_clearance_mm: 0.17, quiet_clearance_mm: 0.0 }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct Clearance {
    pub default: f64,
    /// Edge-to-edge distance to copper of the named nets, where it differs from the default.
    #[serde(default)]
    pub nets: HashMap<String, f64>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Terminal {
    pub xy: Pt,
    /// Layers on which the net may arrive here. More than one means a through connection (a via or a pin).
    pub layers: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Net {
    /// Name of this piece of work in the output.
    pub key: String,
    /// Electrical net: copper of this net is not an obstacle.
    pub net: String,
    pub width: f64,
    /// Via pad diameter and drill.
    pub via: [f64; 2],
    pub layers: Vec<String>,
    pub clearance: Clearance,
    pub terminals: Vec<Terminal>,
    /// The length of detour accepted before a layer change is preferred.
    #[serde(default = "ten")]
    pub via_penalty_mm: f64,
    /// Cost per unit length by layer (1.0 where not given).
    #[serde(default)]
    pub layer_cost: HashMap<String, f64>,
    /// How readily this net gives way to others while paths are negotiated: 1 normal, towards 0 it holds its
    /// shortest path and the others go round.
    #[serde(default = "one", rename = "yield")]
    pub yield_: f64,
    /// Weight of this net when several complete results are compared: of its length and, a hundredfold, of any
    /// excess over `max_mm`.
    #[serde(default = "one")]
    pub weight: f64,
    /// Wanted greatest length; results that exceed it are ranked behind those that do not.
    #[serde(default)]
    pub max_mm: Option<f64>,
    /// A later connection may branch from anywhere on the net's copper (true) or only from its terminals.
    #[serde(default = "yes")]
    pub branch_anywhere: bool,
    /// Ground closed to this net only.
    #[serde(default)]
    pub regions: Vec<Region>,
    /// Laid after all the others have settled, with them fixed, and taking no part in the negotiation: for a
    /// path that must be shown to exist but may not push the other nets about.
    #[serde(default)]
    pub last: bool,
    /// May not change layer.
    #[serde(default)]
    pub no_vias: bool,
    /// A quiet net: an ordinary net and a quiet one keep `rules.quiet_clearance_mm` between their new copper
    /// instead of `rules.new_clearance_mm`.
    #[serde(default)]
    pub quiet: bool,
}

fn one() -> f64 {
    1.0
}
fn ten() -> f64 {
    10.0
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Options {
    pub max_iterations: usize,
    /// Cost factor per other net claiming a cell, at the first iteration, and its growth per iteration.
    pub present_start: f64,
    pub present_growth: f64,
    pub present_max: f64,
    /// Added to the standing cost of a contested cell after each iteration.
    pub history_step: f64,
    /// After a clean result: passes that lay each net again with the others fixed, keeping only improvements.
    pub refine_passes: usize,
    /// Number of differently ordered attempts run side by side; the best complete one is kept.
    pub variants: usize,
    /// false: route each net once in the given order, earlier nets fixed (no negotiation).
    pub negotiate: bool,
    /// A layer change on a hole the net already has costs nothing (true) or the usual via penalty (false, as the
    /// project's older router counts it). No second via is made either way.
    pub own_hole_free: bool,
    /// Cost of a 45 degree corner, as a length. A right angle costs three times as much; sharper turns are not
    /// made. 0: corners are free (and equal-length staircases are as good as a straight run with one corner).
    pub bend_mm: f64,
    /// While nets still negotiate, the search may accept a path up to this factor dearer than the cheapest in
    /// return for looking at far fewer cells (1 = always the cheapest). The closing passes, with the other nets
    /// fixed, always find the cheapest.
    pub haste: f64,
    /// Repairs tried once the nets have settled: the net furthest over its `max_mm` is laid on its own cheapest
    /// path and held, and the others settle round it; kept only if the sum of the excesses falls.
    pub repairs: usize,
    /// After each round, a net longer than its `max_mm` has its readiness to give way multiplied by this
    /// (1, the default: lengths wanted are only used to rank finished attempts). Measured on a bus where several
    /// nets have tight wanted lengths: 0.7 made those nets hold their ground and sent an SPI line round by 110 mm.
    pub stiffen: f64,
    /// Search in two passes (see `Engine::sweep`): first the cost to the goal from everywhere near, at the costs of
    /// the moment and without corners; then the path with corners, looking ahead with that. false: one pass with
    /// the fixed look-ahead only (slower; kept for comparison).
    pub two_pass: bool,
    /// How far beyond the cheapest corner-free path the first pass goes, in 45 degree corners' worth of cost.
    pub sweep_margin: f64,
    /// Rounds without fewer nets in conflict before the standing cost is spread round the places of conflict
    /// (0: never), and before the attempt is given up (0: never).
    pub stall_after: usize,
    pub patience: usize,
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_iterations: 80, present_start: 0.5, present_growth: 1.4, present_max: 1.0e4, history_step: 0.25, refine_passes: 6, variants: 1, negotiate: true, own_hole_free: true, bend_mm: 0.3, haste: 1.0, repairs: 0, stiffen: 1.0, two_pass: true, sweep_margin: 20.0, stall_after: 3, patience: 20, threads: 0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Solution {
    /// "routed", "conflicts" (some nets still too close to each other) or "unroutable" (a terminal cannot be reached at all).
    pub status: String,
    pub iterations: usize,
    pub variant: usize,
    pub score: f64,
    pub nets: Vec<NetResult>,
    pub conflicts: Vec<Conflict>,
    pub message: String,
    /// Every attempt in brief (the solution itself is the best of them), to show what the choices were.
    #[serde(default)]
    pub attempts: Vec<Attempt>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Attempt {
    pub variant: usize,
    pub status: String,
    pub score: f64,
    pub rounds: usize,
    /// Length of each net in mm, in the order of the problem's nets.
    pub lengths_mm: Vec<f64>,
    pub vias: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetResult {
    pub key: String,
    pub net: String,
    pub length_mm: f64,
    pub tracks: Vec<Track>,
    /// New vias. A layer change on a hole that already exists is not listed.
    pub vias: Vec<Pt>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub layer: String,
    pub points: Vec<Pt>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conflict {
    pub kind: String,
    pub key: String,
    pub with: Vec<String>,
    pub layer: Option<String>,
    pub xy: Pt,
}
