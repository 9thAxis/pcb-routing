//! What a floorplan is asked of, and what it tells.

use crate::model::Pt;
use serde::{Deserialize, Serialize};

/// A part of the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// Its reference.
    pub name: String,
    /// The sheet of the schematic it is drawn on.
    #[serde(default)]
    pub sheet: String,
    /// Where it stands.
    pub at: Pt,
    /// Whether it stands on the board (a part not placed yet stands off it).
    pub placed: bool,
    /// Whether it is to stay where it stands: it pulls its zone to itself.
    #[serde(default)]
    pub fixed: bool,
    /// Whether it belongs on an edge of the board (a connector, a switch: what is plugged into or pressed).
    /// It moves with its zone, and pulls the zone to the nearest edge.
    #[serde(default)]
    pub edge: bool,
    /// Whether it is a capacitor: two of them from a net straight to a ground make the net a supply.
    #[serde(default)]
    pub capacitor: bool,
    /// Whether it is an inductor: a chip with a pad on a net that goes to one switches it.
    #[serde(default)]
    pub inductor: bool,
    /// Whether it is a diode: from a connector's net to a ground or a supply it protects, and stands there.
    #[serde(default)]
    pub diode: bool,
    /// Whether it disturbs what is near it (a relay, an oscillator; a switching regulator is found without
    /// being told): true or false if that is said, else as found.
    #[serde(default)]
    pub noisy: Option<bool>,
    /// Whether it is disturbed by what is near it (an amplifier's input, a reference); a part on a sensitive
    /// net is so without being told.
    #[serde(default)]
    pub quiet: Option<bool>,
    /// The ground it takes on the board, in mm2.
    pub area: f64,
    /// The net of each of its pads, as an index into [`Board::nets`]; None for a pad on no net.
    pub pads: Vec<Option<usize>>,
    /// How high it stands above the board, in mm, if that is known.
    #[serde(default)]
    pub height: Option<f64>,
    /// A point it is to stand near (what it mates with on another board, a header below), as boards stack:
    /// it pulls its zone there.
    #[serde(default)]
    pub near: Option<Pt>,
    /// The part it mates with, by name (the other half of a link from board to board): the two are pulled
    /// to the same place of the stack. Said of one, it holds for both.
    #[serde(default)]
    pub mate: Option<String>,
    /// How wide and how high its body is on the board, in mm, as it stands; none given: a square of its area.
    #[serde(default)]
    pub size: Option<[f64; 2]>,
    /// Where each of its pads is from `at`, as it stands, in the order of `pads`; none given: none is known.
    #[serde(default)]
    pub offsets: Vec<Pt>,
    /// What the schematic says each of its pads is, in the order of `pads`; none given: none is known.
    #[serde(default)]
    pub pins: Vec<Pin>,
    /// Whether it has a pad with a hole: it takes its ground on every layer, and stands on no road.
    #[serde(default)]
    pub through: bool,
    /// Its body in pieces, each [x0, y0, x1, y1] from `at` as it stands, where it does not take the whole of
    /// its box: a part with pads round its outline and nothing drawn between (a module on two rows of
    /// pins) leaves the ground between them to other parts. None given: its box is its body.
    #[serde(default)]
    pub pieces: Vec<[f64; 4]>,
    /// Whether it stands on the back of the board. It stays on its side: a part on the one side takes no
    /// ground from a part on the other, unless either has holes.
    #[serde(default)]
    pub back: bool,
    /// Where its own origin is from `at` (the middle of its body), as it stands: what its file places it by.
    #[serde(default)]
    pub anchor: Pt,
    /// How it is turned as it stands, in degrees.
    #[serde(default)]
    pub angle: f64,
    /// Its value, and the name of its footprint: two parts with both alike are of one kind.
    #[serde(default)]
    pub value: String,
    /// See `value`.
    #[serde(default)]
    pub footprint: String,
}

/// What the schematic says a pad is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Pin {
    /// It takes a signal in.
    Input,
    /// It drives one.
    Output,
    /// Either, as it is used.
    Both,
    /// Neither: the end of a resistor, a capacitor, a connector.
    Passive,
    /// It is fed a supply.
    PowerIn,
    /// It feeds one.
    PowerOut,
    /// The schematic does not say.
    #[default]
    Unknown,
}

/// A net of the board.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// Its name.
    pub name: String,
    /// Whether it is a supply by its name (one found by its capacitors need not be said).
    #[serde(default)]
    pub supply: bool,
    /// Whether it is one half of a differential pair.
    #[serde(default)]
    pub pair: bool,
    /// Whether it is sensitive: to be short, and kept from what disturbs.
    #[serde(default)]
    pub sensitive: bool,
}

/// A board, as far as its floorplan needs it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Board {
    /// The board: [x0, y0, x1, y1].
    #[serde(rename = "box")]
    pub outline: [f64; 4],
    /// Its parts.
    pub parts: Vec<Part>,
    /// Its nets.
    pub nets: Vec<Net>,
    /// The boards within that box, where it holds several (two boards of a stack, drawn side by side), each
    /// [x0, y0, x1, y1]; none given: one board, the box. A domain is given ground on one of them.
    #[serde(default)]
    pub regions: Vec<[f64; 4]>,
    /// For each of `regions`, in their order: what is added to a place on it to say where that is once the
    /// boards are stacked (two boards drawn side by side and mounted one over the other). None given: none
    /// is moved. What ties parts on two boards pulls them to the same place of the stack.
    #[serde(default)]
    pub shifts: Vec<Pt>,
    /// Ground with little height above it (under another board of the stack), each with the most a part
    /// there may stand: a zone with a taller part is not to be given ground there.
    #[serde(default)]
    pub limits: Vec<Limit>,
    /// Ground where no part may stand (round a mounting hole, under what passes through the board), each
    /// [x0, y0, x1, y1]: it is not counted as room.
    #[serde(default)]
    pub keepouts: Vec<[f64; 4]>,
    /// What a signal track takes across its way, in mm: its width and the clearance beside it.
    #[serde(default = "usual_pitch")]
    pub pitch: f64,
    /// How many layers carry routes (the planes are none of them).
    #[serde(default = "two")]
    pub layers: usize,
    /// The copper layers, top to bottom, each with what it is for; none given: `layers` layers that run any way.
    #[serde(default)]
    pub stack: Vec<Layer>,
    /// How wide the ground between two domains is kept, in mm: no track crosses it but through the parts that
    /// stand on it, and the planes are parted there.
    #[serde(default = "usual_gap")]
    pub gap: f64,
    /// What a via takes, in mm: its pad and the clearance round it.
    #[serde(default = "usual_via")]
    pub via: f64,
    /// Whether the copies of a sheet that repeats stand side by side as one body.
    #[serde(default)]
    pub rows: Rows,
}

/// Whether the copies of a sheet that repeats stand side by side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Rows {
    /// Wherever there is room for them so: for channels that are to match, and to be read at a glance.
    Always,
    /// Where their pads are then not much further from what they join than with each copy by itself.
    #[default]
    Cheap,
    /// Never: each copy where its own nets are shortest.
    Never,
}

fn usual_via() -> f64 {
    1.0
}

fn usual_gap() -> f64 {
    4.0
}

/// Ground with little height above it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    /// The ground: [x0, y0, x1, y1], as the board is drawn.
    pub rect: [f64; 4],
    /// The most a part there may stand above the board, in mm.
    pub height: f64,
}

/// A copper layer and what it is for.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Its name.
    pub name: String,
    /// What it carries.
    pub way: Way,
}

/// What a layer carries.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Way {
    /// No routes: a plane.
    Plane,
    /// Long runs across the board (along x).
    Across,
    /// Long runs up and down it (along y).
    Up,
    /// Routes any way: an outer layer, which is free where no part stands.
    Any,
}

impl Board {
    /// How many layers carry a run up and down (`up`) or across.
    pub(super) fn layers_that_run(&self, up: bool) -> usize {
        if self.stack.is_empty() {
            return self.layers.max(1);
        }
        let runs = |layer: &&Layer| layer.way == Way::Any || layer.way == if up { Way::Up } else { Way::Across };
        self.stack.iter().filter(runs).count().max(1)
    }
}

fn usual_pitch() -> f64 {
    0.4
}

fn two() -> usize {
    2
}

/// Ground kept free of parts for what runs between zones: the strip along one cut of the board.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Road {
    /// The strip: [x0, y0, x1, y1].
    pub rect: [f64; 4],
    /// How wide it is, in mm.
    pub width: f64,
    /// The signal nets that cross the cut it runs along.
    pub nets: usize,
    /// And the supplies that do.
    pub rails: usize,
    /// How far down it is: 0 for the cut of the whole board, 1 for the cuts of its halves, and so on. The
    /// lower the number, the more of the board it serves.
    pub level: usize,
    /// The zones on one side of it, by name, and those on the other.
    pub between: [Vec<String>; 2],
    /// Whether it runs up and down the board (else across it): its tracks are on the layers that run so.
    pub up: bool,
    /// Whether it is the ground between two domains: no road, nothing runs along it.
    pub gap: bool,
    /// How many tracks it holds side by side, on all the layers that run its way.
    pub lanes: usize,
    /// How many are planned along it where it is fullest (see [`Floorplan::travel`]).
    pub used: usize,
    /// For a gap: the parts that stand on both sides of it. They are placed on it.
    pub parts: Vec<String>,
    /// What its cut is known by (see `cut_of`).
    #[serde(skip)]
    pub(super) cut: String,
}

/// What the connections between zones come to when each is given a way along the roads (see [`floorplan`](super::floorplan)).
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Travel {
    /// The nets given a way.
    pub nets: usize,
    /// How long their ways are in all, in mm, from the middle of a zone to the middle of the next.
    pub mm: f64,
    /// The layer changes they take: two for a way that runs along a road (down to the layer that runs that
    /// way and up again), one more for every turn from a road into one that runs the other way; none for a
    /// way that only crosses a road to the zone beside.
    pub vias: usize,
    /// The nets between two domains: they go through the parts that stand on the gap, and are given no way.
    pub through: usize,
    /// The nets with a zone that no road comes to, given no way.
    pub stranded: usize,
    /// Which those are, as indices into the nets given.
    #[serde(skip)]
    pub(super) left: Vec<usize>,
    /// The roads with more planned along them than they hold.
    pub over: usize,
    /// How far the nets between domains reach in all, in mm, from the middle of a zone to the middle of the
    /// next as the boards stack: what goes through the parts on the gaps, and from board to board.
    pub through_mm: f64,
    /// How far the parts that are to stand near a point are from it in all, in mm, by the middle of their zone.
    pub near_mm: f64,
    /// How far apart the parts that mate are in all, in mm, as the boards stack: what a link has to span sideways.
    pub mate_mm: f64,
    /// The parts on two grounds whose two domains are on one board and no neighbours.
    pub apart: usize,
    /// Where ways turn from one road into another, each with the layer changes made there and what the two
    /// roads have of ground where they meet, in mm2.
    #[serde(skip)]
    pub(super) turns: Vec<(Pt, usize, f64)>,
    /// How much nearer than QUIET_MM the zones with parts that disturb are to the zones with parts that are
    /// disturbed, in mm, summed over every such pair of the smallest zones on one board.
    pub noise: f64,
    /// The pieces more than one that the supplies on the plane layers are in: each wants a way to the rest.
    pub split: usize,
    /// How much longer the parts that stand on the gaps are, side by side, than the gaps they stand on, in mm.
    pub cramped: f64,
}

/// A supply on a plane layer: the ground it is poured on.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Pour {
    /// The supply.
    pub net: String,
    /// Its ground: the domain it is poured in.
    pub ground: String,
    /// The plane layers it is one of the supplies of.
    pub layers: Vec<String>,
    /// The grounds of the smallest zones it is poured under, each [x0, y0, x1, y1].
    pub rects: Vec<[f64; 4]>,
    /// What those come to, in mm2.
    pub area: f64,
    /// How many of its parts stand there: they reach it by a via.
    pub parts: usize,
    /// Its parts that stand elsewhere, under the pour of another supply: they are reached by a track.
    pub by_track: Vec<String>,
    /// The pieces it is in (zones that are neighbours, or joined by ground no supply is poured on, are one).
    pub pieces: usize,
}

/// A part with the place it is given within its zone.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Place {
    /// The part.
    pub part: String,
    /// Where its middle is to stand.
    pub at: Pt,
    /// The ground its body takes there: [x0, y0, x1, y1]. None for a part that stays where it stands.
    pub rect: Option<[f64; 4]>,
    /// How far it is turned from how it stands today, in degrees (0, 90, 180 or 270), counter-clockwise as
    /// the board is looked at: by a quarter where its body lies the better so, and so that its pads face
    /// what they are joined to.
    pub turn: u16,
    /// Whether it stands on the back of the board.
    pub back: bool,
    /// For a part whose body is in pieces: the ground of each, where it is placed. Else none: `rect` is it.
    pub pieces: Vec<[f64; 4]>,
    /// Whether there was room for its body, one way or the other, on the ground of its domain. Where there
    /// was not, its place is where it was pulled, over others.
    pub fits: bool,
    /// How far that is from where it stands today, in mm; None if it is not placed.
    pub off: Option<f64>,
}

/// Sheets of one make-up: the same parts joined the same way.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Repeat {
    /// The sheets.
    pub sheets: Vec<String>,
    /// The parts of each.
    pub parts: usize,
    /// How many of them are laid out alike: placed whole, as the first of them was laid out by itself. 0
    /// where they were left to be placed part by part (too few of their parts have ground in one domain, or
    /// the first found no room laid out by itself).
    pub alike: usize,
    /// The sheet that was laid out.
    pub like: String,
    /// How many of them stand together, side by side and turned alike, as one body; 0 if each stands by itself.
    pub together: usize,
    /// The nets that each copy has of its own, matched from copy to copy (by the pad of the matched part
    /// they are on): how many there are.
    pub matched: usize,
    /// How much longer the longest of a net's copies is than the shortest, in mm: on the average over those
    /// nets. The less, the more alike the copies are joined to what is round them: channels that match.
    pub spread_mm: f64,
    /// And at the worst, with the net of the first sheet that it is of.
    pub worst_mm: f64,
    /// See `worst_mm`.
    pub worst: String,
    /// The longest of all those nets, in mm.
    pub longest_mm: f64,
    /// The nets that are one and the same for every copy (a bias, a clock, what selects them): how many, and
    /// how long they are in all, in mm. Copies side by side leave these short and straight.
    pub shared: usize,
    /// See `shared`.
    pub shared_mm: f64,
}

/// A switching regulator with the parts of its loop, which stand by it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Loop {
    /// The chip that switches.
    pub chip: String,
    /// Its switch node: the net from it to its inductor.
    pub node: String,
    /// The parts that stand by it for the loop: its inductor, what else is on the switch node (a diode), and
    /// the capacitors that close the loop (on its supply, and on the far side of the diode or the inductor).
    pub parts: Vec<String>,
    /// How wide and how high the box of the pads of the loop is, in mm: the chip's pads on the switch node and
    /// the supplies of the loop, the pads of what is on the node, and of the capacitors those of each supply
    /// that stands nearest the chip.
    #[serde(rename = "box")]
    pub span: [f64; 2],
    /// And its ground, in mm2: what the loop encloses at the most.
    pub mm2: f64,
    /// What the bodies of the chip and of the parts in that box take, in mm2: the loop cannot be smaller.
    pub bodies: f64,
    /// Whether the box is no more than LOOP_MOST times that: the parts stand against each other.
    pub tight: bool,
    /// The parts of two pads on the chip's other small nets, which stand by its pads there (its divider,
    /// what sets its frequency, the capacitor it drives its switch from).
    pub beside: Vec<String>,
}

/// Where roads meet and ways turn from one into the other: each turn is a layer change, and its via wants
/// ground that is no track's.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Junction {
    /// Where the two roads meet.
    pub at: Pt,
    /// The layer changes planned there.
    pub changes: usize,
    /// The ground their vias take, in mm2: each as much again as itself, for the tracks to reach it.
    pub needs: f64,
    /// What the two roads have of ground where they meet, in mm2.
    pub has: f64,
    /// The side of the square kept free of parts there, in mm: what the vias need, where the roads have less.
    pub side: f64,
}

/// Where a part that stands on two grounds is given its place.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Across {
    /// On the gap between its two domains, which are neighbours.
    Gap,
    /// Its two domains are on two boards: it is the link between them, at the place of its own board nearest
    /// the other domain as the boards stack.
    Link,
    /// Its two domains are on one board and no neighbours: at the place of its own nearest the other.
    Apart,
}

/// A part that stands on two grounds, with the place it is given.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Bridge {
    /// The part.
    pub part: String,
    /// The ground it is counted with, and the other it is placed by.
    pub grounds: [String; 2],
    /// How it is placed.
    pub on: Across,
    /// Where: on a gap, across its middle, and along it as near what the part is tied to on both sides as
    /// the other parts of the gap allow.
    pub at: Pt,
    /// How far from there its other domain is, in mm, as the boards stack: none on a gap; what a link has to
    /// span sideways; what parts the two domains that should be neighbours.
    pub reach: f64,
    /// How far that is from where it stands today, in mm; None if it is not placed.
    pub off: Option<f64>,
}

/// One board of the box, with what it is given to hold.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Region {
    /// The board: [x0, y0, x1, y1].
    pub rect: [f64; 4],
    /// The room on it, in mm2: its ground but for the keep-outs.
    pub room: f64,
    /// What the parts of its domains take at the least, in mm2.
    pub needed: f64,
    /// And the roads kept free on it.
    pub roads: f64,
    /// The domains it holds.
    pub zones: Vec<String>,
}

/// A supply: a net that feeds parts.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Rail {
    /// The net.
    pub net: String,
    /// Its parts.
    pub parts: Vec<String>,
    /// The capacitors from it straight to its ground.
    pub capacitors: usize,
}

/// The parts on one ground.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Domain {
    /// The ground.
    pub ground: String,
    /// Its parts: those on this ground alone, or mostly (see [`Power::between`]).
    pub parts: Vec<String>,
    /// Its supplies, the one with the most parts first.
    pub rails: Vec<Rail>,
}

/// The power of a board.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Power {
    /// The domains, the largest first.
    pub domains: Vec<Domain>,
    /// The parts that stand on two grounds or more, each with them: what joins two domains.
    pub between: Vec<(String, Vec<String>)>,
    /// Two supplies with the parts that stand on both and are no capacitors: one feeds the other through them.
    pub links: Vec<(String, String, Vec<String>)>,
    /// The parts that no ground and no supply places in a domain.
    pub adrift: Vec<String>,
}

/// Parts that belong together on the board: a group of a sheet's parts, zones that are tied together, or a
/// domain.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Zone {
    /// What it is called: its ground, or the sheet most of its parts are drawn on.
    pub name: String,
    /// The sheets its parts are drawn on.
    pub sheets: Vec<String>,
    /// Its parts.
    pub parts: Vec<String>,
    /// How many of them stand on the board.
    pub placed: usize,
    /// The box of those: [x0, y0, x1, y1]; None with none.
    #[serde(rename = "box")]
    pub bounds: Option<[f64; 4]>,
    /// What its parts take, in mm2: the least it needs.
    pub area: f64,
    /// The signal nets that stay within it.
    pub within: usize,
    /// The zones it is made of, the largest first.
    pub inside: Vec<Zone>,
    /// The ground it is given: [x0, y0, x1, y1].
    pub rect: [f64; 4],
    /// The share of its placed parts that stand within that ground today.
    pub at_home: f64,
    /// How far the middle of its placed parts is from the middle of that ground, in mm; None with none placed.
    pub off: Option<f64>,
}

/// A sheet whose groups are not gathered together: its name, and each group (its parts) with the zone it is
/// gathered into.
pub type Apart = (String, Vec<(Vec<String>, String)>);

/// What [`floorplan`](super::floorplan) finds.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Floorplan {
    /// The power.
    pub power: Power,
    /// The domains as zones, each with the zones within it; and the parts on no ground, if any, as one more.
    pub zones: Vec<Zone>,
    /// What flows between the zones just under the domains: (zone, zone, nets, of which pairs), the heaviest
    /// first. A zone is named `GROUND: NAME`.
    pub flows: Vec<(String, String, usize, usize)>,
    /// The sheets whose groups are not gathered together at the first level, each with its groups and the
    /// zone each is gathered into: drawn as one, tied apart.
    pub apart: Vec<Apart>,
    /// The length of the signal nets between the smallest zones, with each zone at the middle of its placed
    /// parts as they stand, in mm (zones with no part placed are left out).
    pub now_mm: f64,
    /// And the same with each of those zones at the middle of the ground it is given.
    pub then_mm: f64,
    /// The boards, each with the domains it holds and whether they have room there.
    pub regions: Vec<Region>,
    /// What the connections between zones come to, each given a way along the roads.
    pub travel: Travel,
    /// And what they came to as the board was first laid out, before roads were widened and lots changed sides.
    pub first: Travel,
    /// The nets that could be given no way, by name.
    pub stranded: Vec<String>,
    /// The smallest zones given ground where there is little height with parts too tall for it: (the zone,
    /// the share of its ground that is so, those parts, how high the tallest of them stands in mm).
    pub tall: Vec<(String, f64, Vec<String>, f64)>,
    /// The parts that stand on two grounds, each with the place it is given.
    pub bridges: Vec<Bridge>,
    /// A place for every part: within the ground of its zone, by the same pulling and cutting as the zones
    /// themselves (each part pulled by what it is tied to, wherever that is; a capacitor from a supply to the
    /// ground by a chip of its zone on that supply; a connector by the nearest edge); then the bodies of a
    /// domain are settled, none over another and none on a road, the largest first, each as near where it
    /// was pulled as the others allow, turned a quarter where that is nearer. Parts on two grounds have their place on a gap (see `bridges`).
    pub places: Vec<Place>,
    /// The length of all the signal nets with each placed part where it stands today, by the box of each net,
    /// in mm (nets with a part not placed are left out); and the same nets with each part at its place.
    pub parts_now_mm: f64,
    /// The same nets with each part at its place (see `parts_now_mm`).
    pub parts_then_mm: f64,
    /// And the same by how far apart in the plan the parts of each net are: (what they are, how many such
    /// nets, their length as the parts stand today in mm, and with each part at its place): within one of
    /// the smallest zones; between such zones of one larger zone; between larger zones of a domain; between
    /// domains. Where the plan is shorter or longer than the board as it stands, this says at which scale.
    pub against: Vec<(String, usize, f64, f64)>,
    /// How far the pads of the signal nets are from the nearest other pad of their net in all, in mm, with
    /// each part at its place and turned only as its body lies; and the same once each is turned so that
    /// its pads face what they are joined to.
    pub pads_first_mm: f64,
    /// See `pads_first_mm`.
    pub pads_mm: f64,
    /// The capacitors from a supply to the ground that stand for a chip of their zone, and how many of them
    /// have their pad on the supply by the pad of that chip they are for: within BESIDE_PIN, or for a large
    /// one within its own length and a millimetre.
    pub bypass: [usize; 2],
    /// Each of those capacitors: (the capacitor, its chip, how far its pad on the supply is from the pad of
    /// the chip it is for in mm, how many capacitors that pad of the chip has in all).
    pub capacitors: Vec<(String, String, f64, usize)>,
    /// The parts of two pads that stand for one pad of a chip or a connector, and how many of them are by it
    /// (as a capacitor is by its pad): on a small net with one chip or connector, a capacitor from it to a
    /// ground or a supply or across two nets of that chip (a filter at the input it is for) and the
    /// resistor in series with it; a diode likewise; at a connector what is in series with its pin (its
    /// protection comes first); and what is across two pads of one chip and of nothing else (its feedback).
    pub served: [usize; 2],
    /// How many chains there are from one chip or connector through parts of two pads in series to another.
    pub chains: usize,
    /// Those that double back: (from, to, the parts between in their order, how far it is straight from the
    /// pad of the one to the pad of the other in mm, how far from pad to pad through the parts), the worst
    /// first. A chain doubles back if that is more than BACK times the straight way and BESIDE_PIN for each
    /// of its hops.
    pub back: Vec<(String, String, Vec<String>, f64, f64)>,
    /// The blocks: a chip or a connector with the parts that stand by its pads, laid out by themselves and
    /// placed as one. How many there are, and how many parts they hold in all.
    pub blocks: [usize; 2],
    /// The blocks of one make-up (the same chip, the same parts by the same pads): (how many there are, how
    /// many of them came out laid out alike, the parts in each, the chips), the most numerous first. Those
    /// of one make-up are laid out by the same steps, and so alike.
    pub alike: Vec<(usize, usize, usize, Vec<String>)>,
    /// How many parts disturb what is near them, and how many are disturbed.
    pub disturbing: [usize; 2],
    /// Each part that disturbs with each part that is disturbed and stands nearer it than QUIET_MM, on one
    /// board: (the one, the other, how far their bodies are apart in mm), the nearest first.
    pub close: Vec<(String, String, f64)>,
    /// The sensitive nets: (how many, how long in all in mm, the longest in mm, its name).
    pub sensitive: (usize, f64, f64, String),
    /// The sheets that repeat, the largest first: of each make-up one is laid out by itself and every copy
    /// placed as it lies.
    pub repeats: Vec<Repeat>,
    /// The switching regulators, each with its loop: the largest loop first.
    pub loops: Vec<Loop>,
    /// The meetings of roads where ways turn, those with the most layer changes first: ground for their vias
    /// is kept free of parts there.
    pub junctions: Vec<Junction>,
    /// The supplies poured on the plane layers (every plane after the first, which is the ground's): under
    /// each of the smallest zones, the supply most of its parts are on, of those that reach several zones.
    /// A supply so poured takes no room in the roads but where a part of it stands under another's pour.
    pub pours: Vec<Pour>,
    /// The ground kept free between zones, the roads that serve the most of the board first: a road along
    /// every cut that something crosses, as wide as its tracks take side by side on the layers there are.
    pub roads: Vec<Road>,
}
