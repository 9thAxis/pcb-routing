//! The problem and the solution, as data.
//!
//! The router knows nothing about a particular board or CAD program: everything it needs is in a [`Problem`],
//! and everything it found is in a [`Solution`]. Both read and write as JSON with the field names used here.
//! All lengths are in millimetres.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A point, `[x, y]` in mm.
pub type Pt = [f64; 2];

/// Everything the router needs to know: where it may route, what is in the way, and what is to be joined.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    /// The area routed in: `[x0, y0, x1, y1]`. Cell (row j, column i) is at `(x0 + i * step, y0 + j * step)`.
    pub window: [f64; 4],
    /// Grid step. Tracks run through cell centres; one that ends on a terminal off the grid is ended on its
    /// exact place.
    pub step: f64,
    /// Layers that may carry routes, in the order used for every per-layer list. At most seven.
    pub layers: Vec<String>,
    /// Copper already on the board. Its layer names may include layers that carry no routes (planes): a through
    /// via must clear copper on those as well.
    #[serde(default)]
    pub copper: Vec<Copper>,
    /// Holes already on the board (vias and through-hole pins).
    #[serde(default)]
    pub holes: Vec<Hole>,
    /// Ground closed to every net.
    #[serde(default)]
    pub regions: Vec<Region>,
    /// Ground that is open but costs more.
    #[serde(default)]
    pub dear: Vec<Dear>,
    /// Distances that hold for all nets.
    #[serde(default)]
    pub rules: Rules,
    /// The pieces of work: one entry per connection (or tree of connections) to lay.
    pub nets: Vec<Net>,
    /// Pairs of nets to be laid side by side.
    #[serde(default)]
    pub pairs: Vec<Pair>,
    /// How the run is conducted.
    #[serde(default)]
    pub options: Options,
}

/// Two nets laid side by side.
///
/// The pair is laid as one wide track, the `trunk`: an entry of [`Problem::nets`] with two terminals (the
/// points where the pair gathers and where it parts, both on the grid; or several for each, as two pieces,
/// see [`Terminal::piece`]: the trunk then runs between whichever two come cheapest), an electrical net name
/// of its own (so that copper of either real net is an obstacle to it), a `width` of at least
/// `pitch + leg_width`, and `no_vias` set.
///
/// When the nets have settled the trunk is split into two legs `pitch` apart, which lie within it: on the outside
/// of a corner a leg takes what the trunk is wider than `pitch + leg_width` (0.04 mm takes a turn of 45 degrees at
/// a pitch up to 0.48 mm) and is cut straight across where that is not enough; a stretch of the trunk too short for
/// a leg to run beside it on the inside of its corners is left out of that leg. Each leg is joined to its own pads
/// by a stub at either end: four more entries of `nets`, each marked `last`, with the pad as its only terminal (the
/// end of the leg is added to it by the router).
///
/// In the solution each net of the pair appears as `KEY/NET`: its leg and its two stubs.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    /// Name of the pair in the output.
    pub key: String,
    /// Key of the trunk entry.
    pub trunk: String,
    /// The two electrical nets.
    pub legs: [String; 2],
    /// Track width of each leg.
    pub leg_width: f64,
    /// Centre to centre distance of the legs.
    pub pitch: f64,
    /// Keys of the stub entries: `stubs[end][leg]`, end 0 at the trunk's first terminal.
    pub stubs: [[String; 2]; 2],
}

/// A piece of copper already on the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Copper {
    /// Its electrical net, if it has one. Copper of a net is no obstacle to entries of the same net.
    pub net: Option<String>,
    /// The layers it is on.
    pub layers: Vec<String>,
    /// A solder land: a via drill keeps [`Rules::pad_drill_margin_mm`] from it even on its own net.
    #[serde(default)]
    pub pad: bool,
    /// Its outline.
    #[serde(flatten)]
    pub shape: Shape,
}

/// The outline of a piece of copper. In JSON the kind is the field `t`: `"seg"`, `"circle"` or `"poly"`.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t")]
pub enum Shape {
    /// A track: a line from `a` to `b` of width `w` with round ends.
    #[serde(rename = "seg")]
    Seg {
        /// One end.
        a: Pt,
        /// The other end.
        b: Pt,
        /// Width.
        w: f64,
    },
    /// A disc.
    #[serde(rename = "circle")]
    Circle {
        /// Centre.
        c: Pt,
        /// Radius.
        r: f64,
    },
    /// A polygon.
    #[serde(rename = "poly")]
    Poly {
        /// Rings of one polygon, even-odd: the first is the outline, the others are holes in it.
        rings: Vec<Vec<Pt>>,
    },
}

/// A hole already on the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Hole {
    /// Its centre.
    pub xy: Pt,
    /// Its electrical net, if plated and connected. An entry of that net may change layer on it.
    pub net: Option<String>,
}

/// Ground closed to routing.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Region {
    /// Rings of one polygon, even-odd.
    pub rings: Vec<Vec<Pt>>,
    /// Layers closed to tracks; all routing layers if absent.
    #[serde(default)]
    pub layers: Option<Vec<String>>,
    /// Closed to tracks (on `layers`).
    #[serde(default = "yes")]
    pub tracks: bool,
    /// Closed to vias.
    #[serde(default = "yes")]
    pub vias: bool,
}

/// Ground that is open to routing but costs more: a filled zone that a track would cut, the spokes that
/// tie a pad to it. A track pays `factor` times its length there, and a via as a track of its own diameter
/// would on the dearest layer it passes. Where several overlap, a net pays for the dearest that is not its
/// own. The middle of a track or via is what is tested: a polygon that is to keep their edges off as well is
/// given grown by that much.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Dear {
    /// Rings of one polygon, even-odd.
    pub rings: Vec<Vec<Pt>>,
    /// The layers it is on; all routing layers if absent.
    #[serde(default)]
    pub layers: Option<Vec<String>>,
    /// What a unit of track length costs there, as a factor: 1 or more.
    pub factor: f64,
    /// The electrical net whose ground it is, if any: its entries pay nothing more there.
    #[serde(default)]
    pub net: Option<String>,
}

fn yes() -> bool {
    true
}

/// Distances that hold for all nets.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    /// Smallest distance between two hole centres.
    pub hole_centre_mm: f64,
    /// A via drill edge keeps this far from any solder land (unless `via_in_pad` lets it into one of its own net).
    pub pad_drill_margin_mm: f64,
    /// A net may put a via in a solder land of its own net. Off by default: a hole in a land draws solder away
    /// unless it is filled, so it is for lands that cannot be left any other way (one enclosed by another pad).
    pub via_in_pad: bool,
    /// A track or via edge keeps this far from a closed region.
    pub region_margin_mm: f64,
    /// Edge-to-edge distance between copper of two nets routed in the same run.
    pub new_clearance_mm: f64,
    /// The same between a net marked `quiet` and one that is not (0: as `new_clearance_mm`).
    pub quiet_clearance_mm: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            hole_centre_mm: 0.56,
            pad_drill_margin_mm: 0.11,
            via_in_pad: false,
            region_margin_mm: 0.025,
            new_clearance_mm: 0.17,
            quiet_clearance_mm: 0.0,
        }
    }
}

/// Edge-to-edge distances a net keeps from copper already on the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Clearance {
    /// To copper of any other net.
    pub default: f64,
    /// To copper of the named nets, where it differs from the default.
    #[serde(default)]
    pub nets: HashMap<String, f64>,
}

/// A point a net is to reach.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    /// Where.
    pub xy: Pt,
    /// Layers on which the net may arrive here. More than one means a through connection (a via or a pin).
    pub layers: Vec<String>,
    /// The piece of copper it stands on, for terminals that copper already on the board joins (the pads on a
    /// filled zone, and points of the zone itself): terminals of an entry with the same number are joined
    /// already, and the entry is joined to their piece where it reaches any one of them. A terminal without
    /// one stands alone.
    #[serde(default)]
    pub piece: Option<u32>,
    /// Its turn among the terminals of its piece: of those that fixed copper leaves open, only the ones with
    /// the lowest number are offered. 0 where not given. For places in an order of preference, of which a
    /// later one is wanted only where the earlier are closed (where the trunk of a [`Pair`] may begin: as
    /// near its pads as there is room).
    #[serde(default)]
    pub turn: u32,
}

/// One piece of work: terminals to be joined, and the rules for the copper that joins them.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// Name of this piece of work in the output. Unique within a problem.
    pub key: String,
    /// Electrical net: copper of this net is not an obstacle. Several entries may share one.
    pub net: String,
    /// Track width.
    pub width: f64,
    /// Via pad diameter and drill.
    pub via: [f64; 2],
    /// The layers this net may use.
    pub layers: Vec<String>,
    /// Distances to copper already on the board.
    pub clearance: Clearance,
    /// The points to join: two pieces or more (see [`Terminal::piece`]; one for the stub of a [`Pair`]).
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
    /// A quiet net: an ordinary net and a quiet one keep [`Rules::quiet_clearance_mm`] between their new copper
    /// instead of [`Rules::new_clearance_mm`].
    #[serde(default)]
    pub quiet: bool,
    /// The bus this net is of, if any: nets of one bus are drawn to run beside each other (see
    /// [`Options::bus_pull`]). Any name; nets with the same name are of the same bus.
    #[serde(default)]
    pub bus: Option<String>,
    /// Its place across its bus, counted from one side: nets one lane apart are drawn to run next to each
    /// other, nets `k` lanes apart to leave room for the `k - 1` between them, and nets more than three
    /// lanes apart are not drawn to each other at all. Where all nets of a bus have the same lane (0 by
    /// default) they are drawn together in no order. The lanes say how far apart, not on which side.
    #[serde(default)]
    pub bus_lane: i32,
}

fn one() -> f64 {
    1.0
}

fn ten() -> f64 {
    10.0
}

/// How a run is conducted. Every field has a default.
///
/// `variants`, `threads`, `max_states`, `max_iterations`, `refine_passes`, `negotiate`, `bend_mm`,
/// `bus_pull`, `bus_cross` and `drop_conflicts` are meant to be set. The rest
/// tune the search and the negotiation; they are not a stable part of the interface, and a later version may
/// rename, remove or reinterpret them. Leave them at their defaults unless you are experimenting.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    /// Most rounds of negotiation per attempt.
    pub max_iterations: usize,
    /// Cost factor per other net claiming a cell, in the first round.
    pub present_start: f64,
    /// Its growth per round (at least 1).
    pub present_growth: f64,
    /// Its ceiling.
    pub present_max: f64,
    /// Added to the standing cost of a contested cell after each round.
    pub history_step: f64,
    /// After a clean result: passes that lay each net again with the others fixed, keeping only improvements.
    pub refine_passes: usize,
    /// Number of differently ordered attempts run side by side; the best complete one is kept.
    pub variants: usize,
    /// false: route each net once in the given order, earlier nets fixed (no negotiation).
    pub negotiate: bool,
    /// A layer change on a hole the net already has costs nothing (true) or the usual via penalty (false). No
    /// second via is made either way.
    pub own_hole_free: bool,
    /// Cost of a 45 degree corner, as a length. A right angle costs three times as much; sharper turns are not
    /// made. 0: corners are free (and equal-length staircases are as good as a straight run with one corner).
    pub bend_mm: f64,
    /// Search in two passes: first the cost to the goal from everywhere near, at the costs of the moment and
    /// without corners; then the path with corners, looking ahead with that. false: one pass with the fixed
    /// look-ahead only (slower; kept for comparison).
    pub two_pass: bool,
    /// The most the first pass goes beyond the cheapest corner-free path, in 45 degree corners' worth of cost.
    pub sweep_margin: f64,
    /// If given: how often a connection is laid with the straight way to its goal as look-ahead before the
    /// exact cost past the fixed copper is worked out for it (a table over much of the board, shared by all
    /// attempts); 0 for always the table. By default (`null`) never: on every board measured, working the
    /// tables out cost more than the searches they shortened, and several times the memory. Kept for boards
    /// where the straight way misleads badly (a maze of fixed copper). With several attempts side by side,
    /// which of them works a table out first can decide between paths of equal cost.
    pub exact_after: Option<usize>,
    /// How strongly the nets of a bus ([`Net::bus`]) are drawn together: a net of a bus pays this much more
    /// per unit length wherever it does not run beside another net of its bus that is already laid (0.2: a
    /// way beside the others may be a fifth longer than the shortest and is still taken). 0: not at all.
    pub bus_pull: f64,
    /// What a net of a bus pays more per unit length where it runs over or under another net of its bus on
    /// another layer, within one track pitch of it: 6 makes such a step cost seven times its length, so that
    /// the nets of a bus are slow to cross each other. Never less than `bus_pull`.
    pub bus_cross: f64,
    /// Rounds without fewer nets in conflict before the standing cost is spread round the places of conflict
    /// (0: never).
    pub stall_after: usize,
    /// Such rounds before the attempt is given up (0: never).
    pub patience: usize,
    /// When the rounds end with nets still too close to each other: take those nets out (and list them in
    /// [`Solution::unrouted`]) so that what is returned breaks no rule (true), or return everything as it stands
    /// with the places of conflict (false).
    pub drop_conflicts: bool,
    /// Threads for the attempts (0: as many as the machine has). One attempt uses one thread.
    pub threads: usize,
    /// The largest grid accepted, in (layer, cell) states. A run needs roughly 200 bytes per state for each
    /// attempt running at once (its search tables); the default of 10 million is about 2 GB for one attempt.
    pub max_states: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_iterations: 80,
            present_start: 0.5,
            present_growth: 1.4,
            present_max: 1.0e4,
            history_step: 0.25,
            refine_passes: 6,
            variants: 1,
            negotiate: true,
            own_hole_free: true,
            bend_mm: 0.3,
            two_pass: true,
            sweep_margin: 20.0,
            exact_after: None,
            bus_pull: 0.2,
            bus_cross: 6.0,
            stall_after: 3,
            patience: 20,
            drop_conflicts: true,
            threads: 0,
            max_states: 10_000_000,
        }
    }
}

/// How a run ended.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Every net is laid and no two are too close.
    Routed,
    /// Every net is laid but some are still too close to each other; see [`Solution::conflicts`]. Only with
    /// [`Options::drop_conflicts`] off.
    Conflicts,
    /// Some terminal cannot be reached; see [`Solution::unrouted`]. The rest is laid.
    Unroutable,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Status::Routed => "routed",
            Status::Conflicts => "conflicts",
            Status::Unroutable => "unroutable",
        })
    }
}

/// What the router found.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Solution {
    /// How the run ended.
    pub status: Status,
    /// Rounds of negotiation of the attempt returned.
    pub iterations: usize,
    /// Which attempt this is (0 is the one in the given order).
    pub variant: usize,
    /// What the attempts are ranked by, lower is better: the sum over the nets of weight times (length plus a
    /// hundred times any excess over the wanted length), plus one per via. Absent (`null` in JSON) if the
    /// attempt is not complete.
    pub score: Option<f64>,
    /// The copper laid, per entry of the problem (pairs as `KEY/NET`, after the others).
    pub nets: Vec<NetResult>,
    /// Where nets are still too close.
    pub conflicts: Vec<Conflict>,
    /// For an unroutable result: the first of `unrouted`.
    pub message: String,
    /// Every terminal or net that could not be joined, with the reason. Everything else is laid all the same:
    /// a net of several terminals keeps the connections that could be made.
    #[serde(default)]
    pub unrouted: Vec<String>,
    /// Every attempt in brief (the solution itself is the best of them), to show what the choices were.
    #[serde(default)]
    pub attempts: Vec<Attempt>,
    /// What the run cost.
    #[serde(default)]
    pub stats: Stats,
}

/// What a run cost, all attempts together.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Processor seconds per kind of work, summed over the threads. The masks and the cost tables are drawn up
    /// within the passes of a search, so their seconds are counted under those too.
    pub seconds: Vec<(String, f64)>,
    /// States taken out of the queue in the first pass of the searches.
    pub swept: u64,
    /// The same in the second pass.
    pub expanded: u64,
    /// With [`Router::verify_search`](crate::Router::verify_search): searches whose two-pass cost differed from
    /// the one-pass cost.
    pub mismatches: u64,
}

/// One attempt in brief.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Attempt {
    /// Its number.
    pub variant: usize,
    /// How it ended.
    pub status: Status,
    /// Its score (see [`Solution::score`]).
    pub score: Option<f64>,
    /// Rounds of negotiation.
    pub rounds: usize,
    /// Length of each net the attempt laid in mm, in the order of its own [`Solution::nets`]: an attempt that
    /// could not join every terminal still lists what it laid. Empty only if it ended with nothing to show
    /// (a pair whose trunk could not be split).
    pub lengths_mm: Vec<f64>,
    /// New vias, all nets together.
    pub vias: usize,
}

/// The copper laid for one entry.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetResult {
    /// The entry's key (`KEY/NET` for a net of a pair).
    pub key: String,
    /// Its electrical net.
    pub net: String,
    /// Track length.
    pub length_mm: f64,
    /// The tracks, each on one layer.
    pub tracks: Vec<Track>,
    /// New vias. A layer change on a hole that already exists is not listed.
    pub vias: Vec<Pt>,
}

/// A run of track on one layer.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    /// The layer.
    pub layer: String,
    /// Its corner points, in order.
    pub points: Vec<Pt>,
}

/// What kind of copper is too close.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    /// A track of the net stands in ground another net claims.
    #[serde(rename = "track")]
    Track,
    /// A via of the net does.
    #[serde(rename = "via")]
    Via,
    /// Two vias of the same net are closer than the hole spacing.
    #[serde(rename = "two vias of the net too close")]
    OwnVias,
}

/// A place where nets are still too close.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conflict {
    /// What kind of copper.
    pub kind: ConflictKind,
    /// The net.
    pub key: String,
    /// The nets it is too close to.
    pub with: Vec<String>,
    /// The layer, for a track.
    pub layer: Option<String>,
    /// Where.
    pub xy: Pt,
}
