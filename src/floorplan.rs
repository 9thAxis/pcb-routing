//! Where the parts of a board belong: its power domains, the zones within them, an area of the board for each,
//! and how far the parts stand from that today. Nothing here moves a part.
//!
//! A board is first of all its grounds: the parts on one ground are a domain, a part on two stands between
//! two domains. A supply is a net that two or more capacitors join straight to a ground, whatever it is
//! called. Within a domain, parts are tied by the signal nets they share (not by supplies and grounds, which
//! reach everything): the parts of a sheet of the schematic that are tied to each other are a group, and
//! groups are gathered into zones, and zones into larger ones, level by level, by the method of Blondel et
//! al. (Louvain), whatever sheet they are drawn on.
//!
//! Then each zone is given ground. Where a zone is pulled is worked out first: by its fixed parts (connectors)
//! where they stand, by the zones it is tied to, and by what already has its ground. The board is cut across
//! its longer side, the zones in the order they are pulled along it, into two lots as near the same size as
//! that order allows, each with the share of the board its zones need; each half is cut again, down to the
//! smallest zones.

use crate::error::{invalid, Error};
use crate::geom::dist;
use crate::model::Pt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    fn layers_that_run(&self, up: bool) -> usize {
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
    cut: String,
}

/// What the connections between zones come to when each is given a way along the roads (see [`floorplan`]).
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
    left: Vec<usize>,
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
    turns: Vec<(Pt, usize, f64)>,
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
    /// The ground its body takes there: [x0, y0, x1, y1]. None for a part on a gap, which is not settled.
    pub rect: Option<[f64; 4]>,
    /// How far it is turned from how it stands today, in degrees (0, 90, 180 or 270), counter-clockwise as
    /// the board is looked at: by a quarter where its body lies the better so, and so that its pads face
    /// what they are joined to.
    pub turn: u16,
    /// Whether there was room for its body, one way or the other, on the ground of its domain. Where there
    /// was not, its place is where it was pulled, over others.
    pub fits: bool,
    /// How far that is from where it stands today, in mm; None if it is not placed.
    pub off: Option<f64>,
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

/// What [`floorplan`] finds.
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
    /// How far the pads of the signal nets are from the nearest other pad of their net in all, in mm, with
    /// each part at its place and turned only as its body lies; and the same once each is turned so that
    /// its pads face what they are joined to.
    pub pads_first_mm: f64,
    /// See `pads_first_mm`.
    pub pads_mm: f64,
    /// The capacitors from a supply to the ground that stand for a chip of their zone, and how many of them
    /// have their pad on the supply within BESIDE_PIN of the pad of that chip they are for.
    pub bypass: [usize; 2],
    /// Each of those capacitors: (the capacitor, its chip, how far its pad on the supply is from the pad of
    /// the chip it is for in mm, how many capacitors that pad of the chip has in all).
    pub capacitors: Vec<(String, String, f64, usize)>,
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

/// Capacitors from a net straight to a ground that make it a supply.
const BYPASSED: usize = 2;
/// What a fixed part pulls its zone with, as so many nets.
const ANCHOR: f64 = 4.0;
/// How many signal tracks a supply counts for in a road: it is laid wide.
const RAIL_LANES: f64 = 4.0;
/// The most of a board's room that the parts of its domains may take: the rest is for roads and for routing.
const BOARD_FULL: f64 = 0.6;
/// And the most they may take where no parting of the domains leaves every board under BOARD_FULL.
const BOARD_MOST: f64 = 0.85;
/// What a way under the parts of a zone that is not its end counts for, as so many times its length.
const THROUGH: f64 = 2.0;
/// What a layer change counts for when a way is chosen, in mm of track.
const VIA_MM: f64 = 10.0;
/// How far along a road a connection may run on the layer of its parts, without a layer change, in mm.
const LOCAL_MM: f64 = 5.0;
/// What a mm between a part and the point it is to stand near counts for, as so many mm of track.
const NEAR: f64 = 5.0;
/// How far down each zone of a cut is tried in the other lot.
const MOVE_LEVELS: usize = 1;
/// A ground is cut across its shorter side only if that side is this much of the longer.
const SQUARISH: f64 = 0.4;
/// How far down the cuts are tried across the shorter side of their ground.
const CROSS_LEVELS: usize = 3;
/// What a part too tall for the ground its zone is given counts for when layings out are weighed, in mm.
const TALL_MM: f64 = 500.0;
/// What a track more than a road holds counts for when layings out are weighed, in mm; and a net given no way.
const OVER_MM: f64 = 50.0;
const STRANDED_MM: f64 = 200.0;
/// What ties the smallest zones under the pour of one supply to each other, as so many nets for each.
const POURED: f64 = 8.0;
/// And what ties them when zones are gathered into larger ones.
const GATHERED: f64 = 4.0;
/// What ties a capacitor from a supply to a ground to a chip of its sheet on that supply when the sheet's
/// parts are grouped, as so much of a net: enough that it is of the chip's group.
const BESIDE_CHIP: f64 = 0.25;
/// What ties a capacitor from a supply to the ground to the chip it stands by, as so many nets.
const BYPASS: usize = 3;
/// What a mm of parts more than their gap is long counts for when layings out are weighed, in mm.
const CRAMPED_MM: f64 = 20.0;
/// Two grounds no further apart than this many gaps, corner to corner, are neighbours.
const CORNER: f64 = 1.5;
/// What is left between two parts side by side on a gap, in mm.
const BESIDE: f64 = 1.0;
/// How often the roads asked for too much are widened, and how many cuts are tried the other way round.
const WIDENINGS: usize = 8;
const TURNINGS: usize = 400;
/// The most of a piece of the board that the road across it takes.
const ROAD_MOST: f64 = 0.2;

/// Whether a net is a ground by its name: GND, VSS, 0V or GROUND as a word of it (VIB_GND, FIELD_0V, AGND1).
pub fn is_ground(name: &str) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        let word = word.to_ascii_uppercase();
        let named = |w: &str| matches!(w, "GND" | "VSS" | "0V" | "GROUND");
        let bare = word.trim_end_matches(|c: char| c.is_ascii_digit());
        let kind = |w: &str| named(w) || (w.len() > 1 && matches!(&w[..1], "A" | "D" | "P") && named(&w[1..]));
        kind(&word) || kind(bare)
    })
}

/// Groups of nodes after one round of local moves: each group, as one, goes to the neighbouring group that
/// raises the modularity most, until none moves. `ties`: the weight between two nodes, the lower first. The
/// groups that end together are one, in the order of the first.
pub fn gathered(groups: &[Vec<usize>], ties: &BTreeMap<(usize, usize), f64>) -> Vec<Vec<usize>> {
    let mut index: BTreeMap<usize, usize> = BTreeMap::new();
    for (g, members) in groups.iter().enumerate() {
        for &node in members {
            index.insert(node, g);
        }
    }
    let mut between: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for (&(a, b), &weight) in ties {
        if let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) {
            *between.entry((i.min(j), i.max(j))).or_insert(0.0) += weight;
        }
    }
    let home = moved(groups.len(), &between);
    let mut merged: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (g, &to) in home.iter().enumerate() {
        merged.entry(to).or_default().push(g);
    }
    // (in the order of the first group of each)
    let mut lots: Vec<Vec<usize>> = merged.into_values().collect();
    lots.sort_by_key(|lot| lot[0]);
    lots.into_iter().map(|lot| lot.into_iter().flat_map(|g| groups[g].iter().copied()).collect()).collect()
}

/// Nodes in groups that are tied more within than without, the largest first: rounds of [`gathered`], each on
/// the groups of the round before, until a round changes nothing. The order of the nodes decides between
/// equals, so the same question has the same answer.
pub fn communities(nodes: &[usize], ties: &BTreeMap<(usize, usize), f64>) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = nodes.iter().map(|&node| vec![node]).collect();
    loop {
        let found = gathered(&groups, ties);
        if found.len() == groups.len() {
            let place = |node: usize| nodes.iter().position(|&n| n == node).unwrap_or(usize::MAX);
            groups.sort_by_key(|members| (usize::MAX - members.len(), place(members[0])));
            return groups;
        }
        groups = found;
    }
}

/// Which group each of `count` nodes ends in after local moves; `between`: the weight between two of them, the
/// lower first (a node with itself: what it, a group of an earlier round, has within).
fn moved(count: usize, between: &BTreeMap<(usize, usize), f64>) -> Vec<usize> {
    let mut beside: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); count];
    let mut degree = vec![0.0f64; count];
    for (&(i, j), &weight) in between {
        if i == j {
            degree[i] += 2.0 * weight;
        } else {
            *beside[i].entry(j).or_insert(0.0) += weight;
            *beside[j].entry(i).or_insert(0.0) += weight;
            degree[i] += weight;
            degree[j] += weight;
        }
    }
    let total: f64 = degree.iter().sum();
    let mut home: Vec<usize> = (0..count).collect();
    if total <= 0.0 {
        return home;
    }
    let mut weight_of = degree.clone();
    for _ in 0..100 {
        let mut changed = false;
        for node in 0..count {
            let here = home[node];
            let mut to: BTreeMap<usize, f64> = BTreeMap::new();
            for (&other, &weight) in &beside[node] {
                *to.entry(home[other]).or_insert(0.0) += weight;
            }
            weight_of[here] -= degree[node];
            let gain = |group: usize| to.get(&group).copied().unwrap_or(0.0) - weight_of[group] * degree[node] / total;
            let mut best = (here, f64::NEG_INFINITY);
            for &group in to.keys() {
                if gain(group) > best.1 {
                    best = (group, gain(group));
                }
            }
            let to_group = if best.1 > gain(here) + 1e-12 { best.0 } else { here };
            weight_of[to_group] += degree[node];
            if to_group != here {
                home[node] = to_group;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    home
}

/// Ties between things from nets (each the parts it reaches; `of`: the thing a part is of, if any): a net of k
/// things ties each two of them by 1 / (k - 1), so that it counts once for each thing it reaches.
fn tied(nets: &[Vec<usize>], of: &dyn Fn(usize) -> Option<usize>) -> BTreeMap<(usize, usize), f64> {
    let mut ties = BTreeMap::new();
    for parts in nets {
        let things: BTreeSet<usize> = parts.iter().filter_map(|&part| of(part)).collect();
        let things: Vec<usize> = things.into_iter().collect();
        for (i, &a) in things.iter().enumerate() {
            for &b in &things[i + 1..] {
                *ties.entry((a, b)).or_insert(0.0) += 1.0 / (things.len() - 1) as f64;
            }
        }
    }
    ties
}

/// What the power analysis finds, by index: the grounds (net indices), the rails with the ground of each, the
/// ground a part is counted with, and the grounds each part stands on.
struct Powered {
    home: Vec<Option<usize>>,
    stands: Vec<BTreeSet<usize>>,
    rails: BTreeMap<usize, (Option<usize>, usize)>,
    grounds: Vec<usize>,
}

fn powered(board: &Board) -> Powered {
    let (parts, nets) = (&board.parts, &board.nets);
    let ground: Vec<bool> = nets.iter().map(|net| is_ground(&net.name)).collect();
    let mut parts_on: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nets.len()];
    for (p, part) in parts.iter().enumerate() {
        for &net in part.pads.iter().flatten() {
            parts_on[net].insert(p);
        }
    }
    // A capacitor from a net straight to a ground bypasses the net.
    let mut bypass: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); nets.len()];
    for part in parts.iter().filter(|part| part.capacitor && part.pads.len() == 2) {
        if let [Some(a), Some(b)] = part.pads[..] {
            for (net, other) in [(a, b), (b, a)] {
                if a != b && ground[other] && !ground[net] {
                    *bypass[net].entry(other).or_insert(0) += 1;
                }
            }
        }
    }
    let mut rails: BTreeMap<usize, (Option<usize>, usize)> = BTreeMap::new();
    for net in (0..nets.len()).filter(|&net| !ground[net]) {
        let mut most: Option<(usize, usize)> = None;
        for (&to, &count) in &bypass[net] {
            if most.map_or(true, |m| count > m.1) {
                most = Some((to, count));
            }
        }
        if most.is_some_and(|m| m.1 >= BYPASSED) || nets[net].supply {
            rails.insert(net, (most.map(|m| m.0), most.map_or(0, |m| m.1)));
        }
    }
    // A part is on the grounds it has a pad on; without one, on those of the supplies it has a pad on;
    let mut stands: Vec<BTreeSet<usize>> = Vec::new();
    for part in parts {
        let own: BTreeSet<usize> = part.pads.iter().flatten().copied().filter(|&net| ground[net]).collect();
        let by_rail = || part.pads.iter().flatten().filter_map(|net| rails.get(net).and_then(|rail| rail.0)).collect();
        stands.push(if own.is_empty() { by_rail() } else { own });
    }
    // and what neither says, on the ground of most of the parts it shares a net with, as far as that goes.
    for _ in 0..10 {
        let mut found: Vec<(usize, usize)> = Vec::new();
        for p in (0..parts.len()).filter(|&p| stands[p].is_empty()) {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            let own: BTreeSet<usize> = parts[p].pads.iter().flatten().copied().collect();
            for &net in &own {
                for &other in &parts_on[net] {
                    for &g in &stands[other] {
                        *count.entry(g).or_insert(0) += 1;
                    }
                }
            }
            let mut best: Option<(usize, usize)> = None;
            for (&g, &n) in &count {
                if best.map_or(true, |b| n > b.1) {
                    best = Some((g, n));
                }
            }
            if let Some((g, _)) = best {
                found.push((p, g));
            }
        }
        if found.is_empty() {
            break;
        }
        for (p, g) in found {
            stands[p].insert(g);
        }
    }
    // A part on several grounds is counted with the one most of its pads are on, or lead to.
    let home: Vec<Option<usize>> = (0..parts.len())
        .map(|p| {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            for net in parts[p].pads.iter().flatten() {
                let g = if ground[*net] { Some(*net) } else { rails.get(net).and_then(|rail| rail.0) };
                if let Some(g) = g.filter(|g| stands[p].contains(g)) {
                    *count.entry(g).or_insert(0) += 1;
                }
            }
            let mut best: Option<(usize, usize)> = None;
            for &g in &stands[p] {
                let n = count.get(&g).copied().unwrap_or(0);
                if best.map_or(true, |b| n > b.1) {
                    best = Some((g, n));
                }
            }
            best.map(|b| b.0)
        })
        .collect();
    let mut grounds: Vec<usize> = (0..nets.len()).filter(|&net| ground[net]).collect();
    grounds.sort_by(|&a, &b| nets[a].name.cmp(&nets[b].name));
    Powered { home, stands, rails, grounds }
}

/// A zone while it is worked out: its parts by index.
#[derive(Clone)]
struct Work {
    name: String,
    parts: Vec<usize>,
    inside: Vec<Work>,
    rect: [f64; 4],
    /// The ground it needs besides that of its parts, in mm2: for the parts of another domain that stand half
    /// on this one.
    more: f64,
}

fn middle(rect: [f64; 4]) -> Pt {
    [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0]
}

/// The middle value of each coordinate of some points.
fn median(points: &[Pt]) -> Option<Pt> {
    if points.is_empty() {
        return None;
    }
    let of = |k: usize| {
        let mut values: Vec<f64> = points.iter().map(|p| p[k]).collect();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    Some([of(0), of(1)])
}

/// Things in the order their ties alone give: each goes half way to the middle of those it is tied to, over
/// and over, with the middle of all taken out, which leaves those tied to each other side by side.
fn in_line(count: usize, ties: &BTreeMap<(usize, usize), f64>) -> Vec<f64> {
    let mut line: Vec<f64> = (0..count).map(|z| z as f64).collect();
    for _ in 0..500 {
        let before = line.clone();
        for z in 0..count {
            let (mut sum, mut weights) = (0.0, 0.0);
            for (&(a, b), &weight) in ties.iter().filter(|(&(a, b), _)| a == z || b == z) {
                sum += weight * before[if a == z { b } else { a }];
                weights += weight;
            }
            if weights > 0.0 {
                line[z] = (before[z] + sum / weights) / 2.0;
            }
        }
        let mean = line.iter().sum::<f64>() / count.max(1) as f64;
        let size = line.iter().map(|v| (v - mean).abs()).fold(0.0, f64::max).max(1e-12);
        line.iter_mut().for_each(|v| *v = (*v - mean) / size);
    }
    line
}

/// What the giving of ground works on, and what it leaves.
struct Ground<'a> {
    board: &'a Board,
    /// The signal nets, each as its parts.
    nets: &'a [Vec<usize>],
    /// The supplies: of each, the parts that stand on its pour on a plane layer, and those that do not (all of
    /// them, where there is no such layer).
    rails: &'a [(Vec<usize>, Vec<usize>)],
    /// Where each part is taken to stand so far.
    at: Vec<Option<Pt>>,
    roads: Vec<Road>,
    /// The board being laid out, and what is added to a place on it to say where that is in the stack.
    region: [f64; 4],
    shift: Pt,
    /// The cuts whose two lots change sides (a cut is known by the zones it parts: see `cut_of`).
    turned: &'a BTreeSet<String>,
    /// The cuts made across the shorter side of their ground, not the longer.
    crossed: &'a BTreeSet<String>,
    /// Zones put in the first lot of a cut (true) or the second, whatever they are pulled to: by (cut, zone).
    sided: &'a BTreeMap<(String, String), bool>,
    /// And the tracks a cut's road is asked to hold at the least.
    asked: &'a BTreeMap<String, usize>,
    /// The part each part mates with, if any.
    mates: &'a [Option<usize>],
    /// Whether it is parts that are given ground, within a zone: no road is kept between them.
    fine: bool,
    /// For each supply poured on a plane layer: a part of each of the smallest zones it is poured under. Those
    /// zones are pulled together, that the pour be of one piece.
    kin: &'a [Vec<usize>],
}

/// The ground between two grounds that are neighbours, no further apart than `most`, and whether it runs up
/// and down (they are side by side) or across (one is above the other): as long as they are beside each other.
fn strip_between(a: [f64; 4], b: [f64; 4], most: f64) -> Option<([f64; 4], bool)> {
    for k in [0usize, 1] {
        let j = 1 - k;
        let (low, high) = (a[j].max(b[j]), a[j + 2].min(b[j + 2]));
        let (first, second) = if a[k] <= b[k] { (a, b) } else { (b, a) };
        let wide = second[k] - first[k + 2];
        if high - low > 1e-6 && wide > -1e-6 && wide <= most + 1e-6 {
            let mut strip = [0.0; 4];
            (strip[k], strip[k + 2], strip[j], strip[j + 2]) = (first[k + 2], second[k], low, high);
            return Some((strip, k == 0));
        }
    }
    None
}

/// Places along a line from `low` to `high` for things of the sizes given, each of which wants one: in the
/// order of what they want, none over another, each as near what it wants as that allows (the squares of how
/// far each is from it are the least they can be). What does not fit sticks out as much at both ends.
fn spread(want: &[f64], size: &[f64], low: f64, high: f64) -> Vec<f64> {
    struct Lot {
        first: usize,
        count: usize,
        size: f64,
        asks: f64,
        start: f64,
    }
    let room = high - low;
    let settled = |mut lot: Lot| {
        let start = lot.asks / lot.count as f64;
        lot.start = if lot.size >= room { low - (lot.size - room) / 2.0 } else { start.clamp(low, high - lot.size) };
        lot
    };
    let mut order: Vec<usize> = (0..want.len()).collect();
    order.sort_by(|&a, &b| want[a].total_cmp(&want[b]).then(a.cmp(&b)));
    let mut lots: Vec<Lot> = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        let mut lot = settled(Lot { first: k, count: 1, size: size[i], asks: want[i] - size[i] / 2.0, start: 0.0 });
        // (what would stand over the lot before it is one lot with it, placed as one)
        while let Some(last) = lots.pop() {
            if last.start + last.size <= lot.start + 1e-9 {
                lots.push(last);
                break;
            }
            lot = settled(Lot {
                first: last.first,
                count: last.count + lot.count,
                size: last.size + lot.size,
                asks: last.asks + lot.asks - lot.count as f64 * last.size,
                start: 0.0,
            });
        }
        lots.push(lot);
    }
    let mut at = vec![0.0; want.len()];
    for lot in &lots {
        let mut from = lot.start;
        for &i in &order[lot.first..lot.first + lot.count] {
            at[i] = from + size[i] / 2.0;
            from += size[i];
        }
    }
    at
}

/// A capacitor is by the pad it is for if its own pad on that supply is no further from it than this, in mm.
const BESIDE_PIN: f64 = 3.0;

/// A place turned so many quarters counter-clockwise as the board is looked at (its y runs down).
fn quartered(quarters: u8, at: Pt) -> Pt {
    match quarters % 4 {
        0 => at,
        1 => [at[1], -at[0]],
        2 => [-at[0], -at[1]],
        _ => [-at[1], at[0]],
    }
}

/// What a capacitor's value says it holds, in farads: `100n`, `4u7 16V`, `10uF`, `22 pF`; None if it says none.
pub fn farads(value: &str) -> Option<f64> {
    let text: String = value.trim().replace(['µ', 'μ'], "u");
    let mut digits = String::new();
    let mut scale = None;
    for c in text.chars() {
        match c {
            '0'..='9' | '.' => digits.push(c),
            'p' | 'n' | 'u' | 'm' if scale.is_none() && !digits.is_empty() => {
                scale = Some(match c {
                    'p' => 1e-12,
                    'n' => 1e-9,
                    'u' => 1e-6,
                    _ => 1e-3,
                });
                if !digits.contains('.') {
                    digits.push('.');
                }
            }
            ' ' if scale.is_none() => {}
            _ => break,
        }
    }
    let number: f64 = digits.trim_end_matches('.').parse().ok()?;
    scale.map(|scale| number * scale)
}

/// A body of this many mm2 or more is settled before the others, over the whole ground of its domain.
const LARGE: f64 = 64.0;
/// A part with holes stands on no road down to this level (see [`Road::level`]).
const MAIN: usize = 3;
/// How fine the ground of a zone is looked at when its parts are settled on it, in mm.
const SETTLE: f64 = 0.25;

/// Bodies settled on a ground: each ([wide, high], where its middle wants to be) is given a place for its
/// middle and whether it is turned a quarter, the largest first, each as near what it wants as what is
/// settled before it and the ground `shut` allow, and as it stands rather than turned where both are as near.
/// None for one there is no room left for.
fn settled(rect: [f64; 4], bodies: &[([f64; 2], Pt)], shut: &[[f64; 4]]) -> Vec<Option<(Pt, bool)>> {
    let cells = |mm: f64| (mm / SETTLE - 1e-6).ceil().max(1.0) as usize;
    let (wide, high) = (cells(rect[2] - rect[0]), cells(rect[3] - rect[1]));
    let mut taken = vec![false; wide * high];
    let mark = |taken: &mut Vec<bool>, r: [f64; 4]| {
        let low = |v: f64, from: f64, most: usize| (((v - from) / SETTLE + 1e-6).floor().max(0.0) as usize).min(most);
        let top = |v: f64, from: f64, most: usize| (((v - from) / SETTLE - 1e-6).ceil().max(0.0) as usize).min(most);
        for y in low(r[1], rect[1], high)..top(r[3], rect[1], high) {
            for x in low(r[0], rect[0], wide)..top(r[2], rect[0], wide) {
                taken[y * wide + x] = true;
            }
        }
    };
    shut.iter().for_each(|&k| mark(&mut taken, k));
    let mut order: Vec<usize> = (0..bodies.len()).collect();
    order.sort_by(|&a, &b| {
        let area = |i: usize| bodies[i].0[0] * bodies[i].0[1];
        area(b).total_cmp(&area(a)).then(a.cmp(&b))
    });
    let mut found: Vec<Option<(Pt, bool)>> = vec![None; bodies.len()];
    let mut sums = vec![0u32; (wide + 1) * (high + 1)];
    for i in order {
        // (how much is taken up to each cell, that any body's ground is asked in one go)
        for y in 0..high {
            let mut row = 0u32;
            for x in 0..wide {
                row += u32::from(taken[y * wide + x]);
                sums[(y + 1) * (wide + 1) + x + 1] = sums[y * (wide + 1) + x + 1] + row;
            }
        }
        let ([w, h], want) = bodies[i];
        let mut best: Option<(f64, usize, usize, bool)> = None;
        for turn in [false, true] {
            let (bw, bh) = if turn { (cells(h), cells(w)) } else { (cells(w), cells(h)) };
            if bw > wide || bh > high || (turn && bw == bh) {
                continue;
            }
            for y in 0..=high - bh {
                for x in 0..=wide - bw {
                    let at = |x: usize, y: usize| sums[y * (wide + 1) + x];
                    if at(x + bw, y + bh) + at(x, y) != at(x + bw, y) + at(x, y + bh) {
                        continue;
                    }
                    let mid = [
                        rect[0] + (x as f64 + bw as f64 / 2.0) * SETTLE,
                        rect[1] + (y as f64 + bh as f64 / 2.0) * SETTLE,
                    ];
                    let far = (mid[0] - want[0]).powi(2) + (mid[1] - want[1]).powi(2) + if turn { 1e-3 } else { 0.0 };
                    if best.map_or(true, |b| far < b.0) {
                        best = Some((far, x, y, turn));
                    }
                }
            }
        }
        if let Some((_, x, y, turn)) = best {
            let (bw, bh) = if turn { (cells(h), cells(w)) } else { (cells(w), cells(h)) };
            let r = [
                rect[0] + x as f64 * SETTLE,
                rect[1] + y as f64 * SETTLE,
                rect[0] + (x + bw) as f64 * SETTLE,
                rect[1] + (y + bh) as f64 * SETTLE,
            ];
            mark(&mut taken, r);
            found[i] = Some((middle(r), turn));
        }
    }
    found
}

/// What a cut is known by from one laying out to the next: the zones it parts, by name.
fn cut_of(zones: &[Work]) -> String {
    let mut names: Vec<&str> = zones.iter().map(|zone| zone.name.as_str()).collect();
    names.sort_unstable();
    names.join(" | ")
}

/// Give the zones of `zones` ground within `rect`, and theirs within that (see the module). `at`: where each
/// part is taken to stand so far (a fixed part where it stands; another at the middle of the ground its zone
/// has been given, once it has).
fn arrange(zones: &mut [Work], rect: [f64; 4], level: usize, domains: bool, on: &mut Ground) {
    let (board, nets) = (on.board, on.nets);
    let (shift, region) = (on.shift, on.region);
    let at = &mut on.at;
    let settle = |zone: &Work, rect: [f64; 4], at: &mut Vec<Option<Pt>>| {
        for &part in &zone.parts {
            if !(board.parts[part].fixed && board.parts[part].placed) {
                // (kept as places of the stack: what is on another board is pulled to the same place)
                let here = middle(rect);
                at[part] = Some([here[0] + shift[0], here[1] + shift[1]]);
            }
        }
    };
    match zones.len() {
        0 => return,
        1 => {
            zones[0].rect = rect;
            settle(&zones[0], rect, at);
            let (inner, _) = (&mut zones[0].inside, ());
            let mut taken = std::mem::take(inner);
            arrange(&mut taken, rect, level, false, on);
            zones[0].inside = taken;
            return;
        }
        _ => {}
    }
    let count = zones.len();
    let area: Vec<f64> = zones
        .iter()
        .map(|zone| (zone.parts.iter().map(|&p| board.parts[p].area).sum::<f64>() + zone.more).max(1.0))
        .collect();
    let total: f64 = area.iter().sum();
    let mut zone_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (z, zone) in zones.iter().enumerate() {
        for &part in &zone.parts {
            zone_of.insert(part, z);
        }
    }
    let mut ties = tied(nets, &|part| zone_of.get(&part).copied());
    // (zones under the pour of one supply are tied by it: each of the smallest to every other, POURED nets in all)
    let mut kin_here: Vec<(BTreeMap<usize, f64>, f64)> = Vec::new();
    for kin in on.kin.iter().filter(|kin| kin.len() > 1) {
        let each = POURED / (kin.len() - 1) as f64;
        let mut here: BTreeMap<usize, f64> = BTreeMap::new();
        kin.iter().filter_map(|p| zone_of.get(p)).for_each(|&z| *here.entry(z).or_insert(0.0) += 1.0);
        let list: Vec<(usize, f64)> = here.iter().map(|(&z, &n)| (z, n)).collect();
        for (i, &(a, of_a)) in list.iter().enumerate() {
            for &(b, of_b) in &list[i + 1..] {
                *ties.entry((a, b)).or_insert(0.0) += each * of_a * of_b;
            }
        }
        kin_here.push((here, each));
    }
    let ties = ties;
    // Where each zone is pulled: by its fixed parts where they stand, by what it is tied to outside these zones
    // where that has been given ground, and by the others of these zones, each where it is pulled itself. The
    // place that all of it together asks for is found by going round until nothing moves much.
    let mut outside: Vec<Vec<(Pt, f64)>> = vec![Vec::new(); count];
    for parts in nets {
        let weight = 1.0 / (parts.len().max(2) - 1) as f64;
        let touched: BTreeSet<usize> = parts.iter().filter_map(|p| zone_of.get(p).copied()).collect();
        for &other in parts.iter().filter(|&&p| !zone_of.contains_key(&p)) {
            if let Some(there) = at[other] {
                let there = [there[0] - shift[0], there[1] - shift[1]];
                touched.iter().for_each(|&z| outside[z].push((there, weight)));
            }
        }
    }
    for (kin, (here, each)) in on.kin.iter().filter(|kin| kin.len() > 1).zip(&kin_here) {
        for there in kin.iter().filter(|p| !zone_of.contains_key(p)).filter_map(|&p| at[p]) {
            let there = [there[0] - shift[0], there[1] - shift[1]];
            here.iter().for_each(|(&z, &count)| outside[z].push((there, each * count)));
        }
    }
    for (&part, &z) in zone_of.iter().filter(|(&p, _)| board.parts[p].fixed && board.parts[p].placed) {
        outside[z].push((board.parts[part].at, ANCHOR));
    }
    // (and a part that is to stand near a point pulls its zone there)
    for (&part, &z) in &zone_of {
        if let Some(near) = board.parts[part].near {
            outside[z].push(([near[0] - shift[0], near[1] - shift[1]], ANCHOR));
        }
    }
    // (and a part that mates with another, to where that one is taken to stand)
    for (&part, &z) in &zone_of {
        let other = on.mates[part].filter(|other| !zone_of.contains_key(other));
        if let Some(there) = other.and_then(|other| at[other]) {
            outside[z].push(([there[0] - shift[0], there[1] - shift[1]], ANCHOR));
        }
    }
    // (a zone with parts that belong on an edge is pulled to the nearest point of the board's edge, as it goes)
    let on_edge: Vec<f64> = (0..count)
        .map(|z| zones[z].parts.iter().filter(|&&p| board.parts[p].edge).count().min(3) as f64 * ANCHOR)
        .collect();
    let to_edge = |p: Pt| -> Pt {
        let [x0, y0, x1, y1] = region;
        let ways = [(p[0] - x0, [x0, p[1]]), (x1 - p[0], [x1, p[1]]), (p[1] - y0, [p[0], y0]), (y1 - p[1], [p[0], y1])];
        ways.iter().fold(ways[0], |best, way| if way.0 < best.0 { *way } else { best }).1
    };
    let mut spot: Vec<Pt> = vec![middle(rect); count];
    for _ in 0..200 {
        let mut far = 0.0f64;
        for z in 0..count {
            let (mut sum, mut weights) = ([0.0, 0.0], 0.0);
            if on_edge[z] > 0.0 {
                let there = to_edge(spot[z]);
                sum = [on_edge[z] * there[0], on_edge[z] * there[1]];
                weights = on_edge[z];
            }
            for &(there, weight) in &outside[z] {
                sum = [sum[0] + weight * there[0], sum[1] + weight * there[1]];
                weights += weight;
            }
            for (&(a, b), &weight) in ties.iter().filter(|(&(a, b), _)| a == z || b == z) {
                let there = spot[if a == z { b } else { a }];
                sum = [sum[0] + weight * there[0], sum[1] + weight * there[1]];
                weights += weight;
            }
            if weights > 0.0 {
                let to = [sum[0] / weights, sum[1] / weights];
                far = far.max(dist(to, spot[z]));
                spot[z] = to;
            }
        }
        if far < 1e-3 {
            break;
        }
    }
    // The board is cut across its longer side, the zones in the order they are pulled along it, where the two
    // lots are nearest the same size; each lot gets the share of the board its zones need.
    // (across its longer side; or across the shorter where that was found the better, unless the ground is so
    // long that its pieces would be slivers)
    let (across, down) = (rect[2] - rect[0], rect[3] - rect[1]);
    let squarish = across.min(down) >= SQUARISH * across.max(down);
    let wide = (across >= down) != (squarish && on.crossed.contains(&cut_of(zones)));
    let along = usize::from(!wide);
    // Where nothing pulls the zones apart along that side (nothing fixed, nothing outside), they are put in the
    // order their ties alone give.
    let low = spot.iter().map(|p| p[along]).fold(f64::INFINITY, f64::min);
    let high = spot.iter().map(|p| p[along]).fold(f64::NEG_INFINITY, f64::max);
    let mut line: Vec<f64> = spot.iter().map(|p| p[along]).collect();
    if high - low < 1e-6 * (rect[2] - rect[0] + rect[3] - rect[1]) {
        line = in_line(count, &ties);
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| line[a].total_cmp(&line[b]).then(a.cmp(&b)));
    let mut best = (1usize, f64::INFINITY);
    let mut held = 0.0;
    for (k, &z) in order.iter().enumerate().take(count - 1) {
        held += area[z];
        let lean = (2.0 * held - total).abs();
        if lean < best.1 {
            best = (k + 1, lean);
        }
    }
    let mut first = vec![false; count];
    order[..best.0].iter().for_each(|&z| first[z] = true);
    // (a zone is in the other lot where that was found the better, as long as neither lot is left empty)
    let this_cut = cut_of(zones);
    let as_pulled = first.clone();
    for z in 0..count {
        if let Some(&side) = on.sided.get(&(this_cut.clone(), zones[z].name.clone())) {
            first[z] = side;
        }
    }
    if first.iter().all(|&f| f) || !first.iter().any(|&f| f) {
        first = as_pulled;
    }
    let share = (0..count).filter(|&z| first[z]).map(|z| area[z]).sum::<f64>() / total;
    // Between the two lots a road is kept free of parts: what crosses the cut runs along it to where it is
    // going. It is as wide as the tracks that cross take side by side on the layers there are (a supply as
    // several), and no more than a share of the piece that is cut.
    let crosses = |parts: &Vec<usize>| {
        let sides: BTreeSet<bool> = parts.iter().filter_map(|p| zone_of.get(p)).map(|&z| first[z]).collect();
        sides.len() == 2
    };
    // (a supply crosses by a track where a lot has parts of it and none of them on its pour, and the other lot
    // has any of it)
    let rail_crosses = |(fed, left): &(Vec<usize>, Vec<usize>)| {
        let sides = |parts: &Vec<usize>| {
            let mut found = [false; 2];
            parts.iter().filter_map(|p| zone_of.get(p)).for_each(|&z| found[usize::from(first[z])] = true);
            found
        };
        let (fed, left) = (sides(fed), sides(left));
        (0..2).any(|k| left[k] && !fed[k] && (fed[1 - k] || left[1 - k]))
    };
    let (signals, supplies) =
        (nets.iter().filter(|n| crosses(n)).count(), on.rails.iter().filter(|n| rail_crosses(n)).count());
    let length = if wide { rect[2] - rect[0] } else { rect[3] - rect[1] };
    let lanes =
        (signals as f64 + RAIL_LANES * supplies as f64).max(on.asked.get(&this_cut).copied().unwrap_or(0) as f64);
    // (a road along a cut across the board's width runs up and down, and so on: it has the layers that run so;
    // between two domains it is no road but the gap they keep)
    let run_layers = board.layers_that_run(wide);
    // (whole tracks: a road for three nets on four layers is one track wide, not three quarters of one; and
    // one track at the least along every cut, or a zone with nothing to its neighbour has no way out past it)
    let abreast = (lanes / run_layers as f64).ceil().max(1.0);
    let road =
        if on.fine { 0.0 } else { if domains { board.gap } else { board.pitch * abreast }.min(ROAD_MOST * length) };
    let low = if wide { rect[0] } else { rect[1] };
    let piece = |from: f64, to: f64| if wide { [from, rect[1], to, rect[3]] } else { [rect[0], from, rect[2], to] };
    // (the lots change sides where that was found the better)
    let flip = on.turned.contains(&this_cut);
    let aim = if flip { 1.0 - share } else { share };
    let mut cut = low + aim * (length - road);
    // (where no part may stand is no room: with such ground here, each lot has its share of what is left)
    let shut = |r: [f64; 4], k: &[f64; 4]| {
        (r[2].min(k[2]) - r[0].max(k[0])).max(0.0) * (r[3].min(k[3]) - r[1].max(k[1])).max(0.0)
    };
    let free =
        |r: [f64; 4]| ((r[2] - r[0]) * (r[3] - r[1]) - board.keepouts.iter().map(|k| shut(r, k)).sum::<f64>()).max(0.0);
    if board.keepouts.iter().any(|k| shut(rect, k) > 0.0) {
        let (mut from, mut till) = (low, low + length - road);
        for _ in 0..40 {
            let mid = (from + till) / 2.0;
            let (one, other) = (free(piece(low, mid)), free(piece(mid + road, low + length)));
            if one < aim * (one + other) {
                from = mid;
            } else {
                till = mid;
            }
        }
        cut = (from + till) / 2.0;
    }
    let (before, after) = (piece(low, cut), piece(cut + road, low + length));
    let (of_first, of_second) = if flip { (after, before) } else { (before, after) };
    if road > 0.0 {
        let names = |side: bool| (0..count).filter(|&z| first[z] == side).map(|z| zones[z].name.clone()).collect();
        on.roads.push(Road {
            rect: piece(cut, cut + road).map(|v| (v * 100.0).round() / 100.0),
            width: (road * 100.0).round() / 100.0,
            nets: signals,
            rails: supplies,
            level,
            between: [names(true), names(false)],
            up: wide,
            gap: domains,
            lanes: if domains { 0 } else { (road / board.pitch + 1e-6).floor() as usize * run_layers },
            used: 0,
            parts: Vec::new(),
            cut: this_cut,
        });
    }
    // (each lot stands at the middle of its half until its zones have their own)
    for (z, zone) in zones.iter().enumerate() {
        settle(zone, if first[z] { of_first } else { of_second }, at);
    }
    let mut lots: [Vec<Work>; 2] = [Vec::new(), Vec::new()];
    let mut order: Vec<(usize, usize)> = Vec::new();
    for (z, zone) in zones.iter_mut().enumerate() {
        let side = usize::from(!first[z]);
        order.push((side, lots[side].len()));
        lots[side].push(Work {
            name: std::mem::take(&mut zone.name),
            parts: std::mem::take(&mut zone.parts),
            inside: std::mem::take(&mut zone.inside),
            rect: zone.rect,
            more: zone.more,
        });
    }
    let [mut one, mut other] = lots;
    arrange(&mut one, of_first, level + 1, domains, on);
    arrange(&mut other, of_second, level + 1, domains, on);
    let mut back: [Vec<Option<Work>>; 2] = [one.into_iter().map(Some).collect(), other.into_iter().map(Some).collect()];
    for (zone, (side, k)) in zones.iter_mut().zip(order) {
        if let Some(done) = back[side][k].take() {
            *zone = done;
        }
    }
}

/// Give each net that reaches several of the smallest zones (`leaves`: the domain, the ground and the parts of
/// each) a way along the roads, and say what that comes to; each road is told how many ways run along it
/// where it is fullest.
///
/// The roads are lines down their middles, joined where one ends on another; a zone comes onto every road
/// that runs along one of its sides, at the point nearest its middle. A way is the shortest from zone to
/// zone along those lines, a turn from one road into another counted as VIA_MM more (it is a change of
/// layer), a road that is full as four times its length, and a way under a zone that is not its end as
/// twice its length. A net of several zones is joined zone by
/// zone, each to the nearest of what is joined already. The gaps between domains are no roads.
fn travelled(roads: &mut [Road], leaves: &[(usize, [f64; 4], &Vec<usize>)], nets: &[Vec<usize>]) -> Travel {
    const NEAR: f64 = 0.03;
    let key = |p: Pt| ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64);
    let mut ids: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    let mut points: Vec<Pt> = Vec::new();
    let node = |p: Pt, ids: &mut BTreeMap<(i64, i64), usize>, points: &mut Vec<Pt>| -> usize {
        *ids.entry(key(p)).or_insert_with(|| {
            points.push(p);
            points.len() - 1
        })
    };
    let line = |road: &Road| -> (f64, f64, f64) {
        // (where its middle line is, and from where to where it runs)
        let r = road.rect;
        if road.up {
            ((r[0] + r[2]) / 2.0, r[1], r[3])
        } else {
            ((r[1] + r[3]) / 2.0, r[0], r[2])
        }
    };
    let real: Vec<usize> = (0..roads.len()).filter(|&i| !roads[i].gap).collect();
    // the points on each road: where another meets it, and where a zone comes onto it
    let mut on_road: Vec<Vec<(f64, usize)>> = vec![Vec::new(); roads.len()];
    for (a, &i) in real.iter().enumerate() {
        for &j in &real[a + 1..] {
            if roads[i].up == roads[j].up {
                continue;
            }
            let (up, across) = if roads[i].up { (i, j) } else { (j, i) };
            let (x, y0, y1) = line(&roads[up]);
            let (y, x0, x1) = line(&roads[across]);
            let (ru, ra) = (roads[up].rect, roads[across].rect);
            // (they meet where the one's strip touches or crosses the other's)
            let meets =
                ra[0] <= ru[2] + NEAR && ra[2] >= ru[0] - NEAR && ru[1] <= ra[3] + NEAR && ru[3] >= ra[1] - NEAR;
            if meets
                && y >= y0 - roads[across].width
                && y <= y1 + roads[across].width
                && x >= x0 - roads[up].width
                && x <= x1 + roads[up].width
            {
                let at = node([x, y], &mut ids, &mut points);
                on_road[up].push((y, at));
                on_road[across].push((x, at));
            }
        }
    }
    let mut edges: Vec<Vec<(usize, f64, Option<usize>)>> = Vec::new(); // (to, length, the road it runs along)
    let mut zone_node: Vec<usize> = Vec::new();
    let mut access: Vec<Vec<(usize, f64)>> = vec![Vec::new(); leaves.len()];
    for (z, &(_, r, _)) in leaves.iter().enumerate() {
        let mid = middle(r);
        zone_node.push(node([mid[0] + 1e-4 * z as f64, mid[1] + 1e-4], &mut ids, &mut points));
        for &i in &real {
            let (at, from, to) = line(&roads[i]);
            let road = roads[i].rect;
            let (beside, low, high, along) = if roads[i].up {
                ((r[2] - road[0]).abs() < NEAR || (r[0] - road[2]).abs() < NEAR, r[1].max(from), r[3].min(to), mid[1])
            } else {
                ((r[3] - road[1]).abs() < NEAR || (r[1] - road[3]).abs() < NEAR, r[0].max(from), r[2].min(to), mid[0])
            };
            if !beside || high - low <= NEAR {
                continue;
            }
            let along = along.clamp(low, high);
            let p = if roads[i].up { [at, along] } else { [along, at] };
            let there = node(p, &mut ids, &mut points);
            on_road[i].push((along, there));
            access[z].push((there, (p[0] - mid[0]).abs() + (p[1] - mid[1]).abs()));
        }
    }
    edges.resize(points.len(), Vec::new());
    for (z, onto) in access.iter().enumerate() {
        for &(there, far) in onto {
            edges[zone_node[z]].push((there, far, None));
            edges[there].push((zone_node[z], far, None));
        }
    }
    for &i in &real {
        on_road[i].sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        on_road[i].dedup_by_key(|point| point.1);
        for pair in on_road[i].windows(2) {
            let far = (pair[1].0 - pair[0].0).abs();
            edges[pair[0].1].push((pair[1].1, far, Some(i)));
            edges[pair[1].1].push((pair[0].1, far, Some(i)));
        }
    }
    let mut leaf_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (z, (_, _, parts)) in leaves.iter().enumerate() {
        leaf_of.extend(parts.iter().map(|&p| (p, z)));
    }
    let about: Vec<(bool, usize)> = roads.iter().map(|road| (road.up, road.lanes)).collect();
    let count = roads.len();
    let mut fullest = vec![0usize; count];
    let mut used: BTreeMap<(usize, usize), usize> = BTreeMap::new(); // per stretch of road: the ways along it
    let mut travel = Travel::default();
    let widths: Vec<f64> = roads.iter().map(|road| road.width).collect();
    let mut turned_at: BTreeMap<usize, (usize, f64)> = BTreeMap::new();
    // (the way from any of `from` to the nearest of `to`: its nodes; a state is (node, the road come along))
    let way = |from: &BTreeSet<usize>,
               to: &BTreeSet<usize>,
               used: &BTreeMap<(usize, usize), usize>|
     -> Option<Vec<(usize, Option<usize>)>> {
        let slot = |node: usize, road: Option<usize>| node * (count + 1) + road.unwrap_or(count);
        let mut best: BTreeMap<usize, (f64, Option<usize>)> = BTreeMap::new(); // state -> (cost, the state before)
        let mut open: BTreeSet<(u64, usize)> = BTreeSet::new();
        for &start in from {
            best.insert(slot(start, None), (0.0, None));
            open.insert((0, slot(start, None)));
        }
        while let Some(&(at_cost, state)) = open.iter().next() {
            open.remove(&(at_cost, state));
            let (here, came) = (state / (count + 1), state % (count + 1));
            let cost = best[&state].0;
            if to.contains(&here) {
                let mut path = vec![(here, (came < count).then_some(came))];
                let mut back = best[&state].1;
                while let Some(before) = back {
                    let came = before % (count + 1);
                    path.push((before / (count + 1), (came < count).then_some(came)));
                    back = best[&before].1;
                }
                path.reverse();
                return Some(path);
            }
            for &(next, far, road) in &edges[here] {
                // (a zone is the end of a way; a way through one runs under its parts on the inner layers,
                // which is dearer than a road: THROUGH times as far)
                let under =
                    (zone_node.contains(&next) && !to.contains(&next)) || (zone_node.contains(&here) && cost > 0.0);
                let turn = match (came < count, road) {
                    (true, Some(r)) if about[came].0 != about[r].0 => VIA_MM,
                    _ => 0.0,
                };
                let full = road
                    .is_some_and(|r| used.get(&(here.min(next), here.max(next))).copied().unwrap_or(0) >= about[r].1);
                let then = cost
                    + turn
                    + if full {
                        4.0 * far
                    } else if under {
                        THROUGH * far
                    } else {
                        far
                    };
                let to_state = slot(next, road);
                if best.get(&to_state).map_or(true, |b| then < b.0 - 1e-9) {
                    best.insert(to_state, (then, Some(state)));
                    open.insert(((then * 1000.0) as u64, to_state));
                }
            }
        }
        None
    };
    for (number, parts) in nets.iter().enumerate() {
        let reached: BTreeSet<usize> = parts.iter().filter_map(|p| leaf_of.get(p).copied()).collect();
        if reached.len() < 2 {
            continue;
        }
        let domains: BTreeSet<usize> = reached.iter().map(|&z| leaves[z].0).collect();
        if domains.len() > 1 {
            travel.through += 1;
            continue;
        }
        if reached.iter().any(|&z| access[z].is_empty()) {
            travel.stranded += 1;
            travel.left.push(number);
            continue;
        }
        travel.nets += 1;
        let mut joined: BTreeSet<usize> = [zone_node[*reached.iter().next().unwrap()]].into_iter().collect();
        let mut left: BTreeSet<usize> = reached.iter().skip(1).map(|&z| zone_node[z]).collect();
        while !left.is_empty() {
            let Some(path) = way(&joined, &left, &used) else {
                travel.stranded += 1;
                travel.left.push(number);
                break;
            };
            let (mut along, mut turns, mut last) = (0.0, 0usize, None::<usize>);
            let mut turned: Vec<(usize, f64)> = Vec::new();
            for pair in path.windows(2) {
                let ((a, _), (b, road)) = (pair[0], pair[1]);
                let far = (points[a][0] - points[b][0]).abs() + (points[a][1] - points[b][1]).abs();
                travel.mm += far;
                if let Some(r) = road {
                    along += far;
                    *used.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                    let now = used[&(a.min(b), a.max(b))];
                    fullest[r] = fullest[r].max(now);
                    if let Some(before) = last.filter(|&before| about[before].0 != about[r].0) {
                        turns += 1;
                        turned.push((a, widths[before] * widths[r]));
                    }
                    last = Some(r);
                }
            }
            if along > LOCAL_MM {
                travel.vias += 2 + turns;
                for (node, ground) in turned {
                    turned_at.entry(node).or_insert((0, ground)).0 += 1;
                }
            }
            let end = path[path.len() - 1].0;
            left.remove(&end);
            joined.extend(path.iter().map(|step| step.0));
        }
    }
    for (road, &most) in roads.iter_mut().zip(&fullest) {
        road.used = most;
    }
    travel.turns = turned_at.into_iter().map(|(node, (changes, ground))| (points[node], changes, ground)).collect();
    travel.mm = travel.mm.round();
    travel.over = roads.iter().filter(|road| !road.gap && road.used > road.lanes).count();
    travel
}

/// The floorplan of a board (see the module). An error if a pad names a net that is not there, or the board
/// has no size.
pub fn floorplan(board: &Board) -> Result<Floorplan, Error> {
    // A part on the board with no pad on any net is the board's own (a mounting hole, a mark): it stays where
    // it stands, and no other part may stand on its body.
    let stays = |part: &Part| part.placed && part.pads.iter().all(|net| net.is_none());
    let held: Board;
    let board = if board.parts.iter().any(stays) {
        let mut with = board.clone();
        for part in board.parts.iter().filter(|part| stays(part)) {
            let side = part.area.sqrt();
            let [w, h] = part.size.unwrap_or([side, side]);
            with.keepouts.push([
                part.at[0] - w / 2.0,
                part.at[1] - h / 2.0,
                part.at[0] + w / 2.0,
                part.at[1] + h / 2.0,
            ]);
        }
        held = with;
        &held
    } else {
        board
    };
    let (parts, all_nets) = (&board.parts, &board.nets);
    let [x0, y0, x1, y1] = board.outline;
    if !(x1 > x0 && y1 > y0) {
        return Err(invalid("box", "has no size"));
    }
    for (p, part) in parts.iter().enumerate() {
        if part.pads.iter().flatten().any(|&net| net >= all_nets.len()) {
            return Err(invalid(format!("parts[{p}].pads"), format!("names a net of {}", all_nets.len())));
        }
        for (what, count) in [("offsets", part.offsets.len()), ("pins", part.pins.len())] {
            if count != 0 && count != part.pads.len() {
                return Err(invalid(format!("parts[{p}].{what}"), format!("{count} for {} pads", part.pads.len())));
            }
        }
    }
    let mut mate_of: Vec<Option<usize>> = vec![None; parts.len()];
    for (p, part) in parts.iter().enumerate() {
        if let Some(mate) = &part.mate {
            let Some(other) = parts.iter().position(|other| &other.name == mate).filter(|&other| other != p) else {
                return Err(invalid(format!("parts[{p}].mate"), format!("names no other part: {mate}")));
            };
            (mate_of[p], mate_of[other]) = (Some(other), Some(p));
        }
    }
    let found = powered(board);
    // (what a capacitor may stand for: a part of more than two pads that is no capacitor and no connector)
    let is_chip = |q: usize| !parts[q].capacitor && !parts[q].edge && parts[q].pads.len() > 2;
    let name = |p: usize| parts[p].name.clone();
    let names = |list: &[usize]| -> Vec<String> {
        let mut out: Vec<String> = list.iter().map(|&p| name(p)).collect();
        out.sort();
        out
    };
    let net_name = |net: usize| all_nets[net].name.clone();

    // The signal nets: what is no ground and no supply, each as the parts it reaches.
    let mut reach: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); all_nets.len()];
    for (p, part) in parts.iter().enumerate() {
        for &net in part.pads.iter().flatten() {
            reach[net].insert(p);
        }
    }
    let is_signal = |net: usize| !found.grounds.contains(&net) && !found.rails.contains_key(&net);
    let signal: Vec<usize> = (0..all_nets.len()).filter(|&net| is_signal(net) && reach[net].len() > 1).collect();
    let nets: Vec<Vec<usize>> = signal.iter().map(|&net| reach[net].iter().copied().collect()).collect();
    let between_parts = tied(&nets, &Some);

    // The power, as it is told.
    let mut domains: Vec<(usize, Vec<usize>)> = found
        .grounds
        .iter()
        .map(|&g| (g, (0..parts.len()).filter(|&p| found.home[p] == Some(g)).collect::<Vec<usize>>()))
        .filter(|(_, own)| !own.is_empty())
        .collect();
    domains.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(net_name(a.0).cmp(&net_name(b.0))));
    let adrift: Vec<usize> = (0..parts.len()).filter(|&p| found.home[p].is_none() && !stays(&parts[p])).collect();
    let rails_of = |g: usize| -> Vec<Rail> {
        let mut rails: Vec<Rail> = found
            .rails
            .iter()
            .filter(|(_, rail)| rail.0 == Some(g))
            .map(|(&net, rail)| Rail {
                net: net_name(net),
                parts: names(&reach[net].iter().copied().collect::<Vec<_>>()),
                capacitors: rail.1,
            })
            .collect();
        rails.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.net.cmp(&b.net)));
        rails
    };
    let mut links: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (p, part) in parts.iter().enumerate().filter(|(_, part)| !part.capacitor) {
        let on: BTreeSet<String> =
            part.pads.iter().flatten().filter(|net| found.rails.contains_key(net)).map(|&net| net_name(net)).collect();
        let on: Vec<String> = on.into_iter().collect();
        for (i, a) in on.iter().enumerate() {
            for b in &on[i + 1..] {
                links.entry((a.clone(), b.clone())).or_default().push(name(p));
            }
        }
    }
    let mut between: Vec<(String, Vec<String>)> = (0..parts.len())
        .filter(|&p| found.stands[p].len() > 1)
        .map(|p| {
            let mut on: Vec<String> = found.stands[p].iter().map(|&g| net_name(g)).collect();
            on.sort();
            (name(p), on)
        })
        .collect();
    between.sort();
    let power = Power {
        domains: domains
            .iter()
            .map(|(g, own)| Domain { ground: net_name(*g), parts: names(own), rails: rails_of(*g) })
            .collect(),
        between,
        links: links
            .into_iter()
            .map(|((a, b), mut by)| {
                (a, b, {
                    by.sort();
                    by
                })
            })
            .collect(),
        adrift: names(&adrift),
    };

    // The zones within a lot of parts: the groups within each sheet, gathered level by level.
    let sheet = |p: usize| if parts[p].sheet.is_empty() { "(on no sheet)".to_string() } else { parts[p].sheet.clone() };
    let sheet_size = |name: &str| (0..parts.len()).filter(|&p| sheet(p) == name).count();
    let mut apart: Vec<Apart> = Vec::new();
    let power_layers: Vec<String> =
        board.stack.iter().filter(|layer| layer.way == Way::Plane).skip(1).map(|layer| layer.name.clone()).collect();
    let mut within = |lot: &[usize]| -> Vec<Work> {
        let mine: BTreeSet<usize> = lot.iter().copied().collect();
        let mut sheets: Vec<String> = mine.iter().map(|&p| sheet(p)).collect::<BTreeSet<_>>().into_iter().collect();
        sheets.sort();
        let mut atoms: Vec<(String, Vec<usize>)> = Vec::new();
        let mut atom_of: BTreeMap<usize, usize> = BTreeMap::new();
        for of_sheet in &sheets {
            let mut own: Vec<usize> = mine.iter().copied().filter(|&p| &sheet(p) == of_sheet).collect();
            own.sort_by_key(|&p| name(p));
            let here: BTreeSet<usize> = own.iter().copied().collect();
            let mut ties: BTreeMap<(usize, usize), f64> = between_parts
                .iter()
                .filter(|(&(a, b), _)| here.contains(&a) && here.contains(&b))
                .map(|(&k, &w)| (k, w))
                .collect();
            // (a capacitor from a supply to a ground is tied to a chip of the sheet on that supply, the chips
            // taking them in turn: it is there for the chip, though no signal says so)
            let mut has: BTreeMap<(usize, usize), usize> = BTreeMap::new();
            for &c in own.iter().filter(|&&c| parts[c].capacitor && parts[c].pads.len() == 2) {
                for &rail in parts[c].pads.iter().flatten().filter(|net| found.rails.contains_key(net)) {
                    let chips = own.iter().filter(|&&q| is_chip(q) && parts[q].pads.contains(&Some(rail)));
                    if let Some(&chip) = chips.min_by_key(|&&q| (has.get(&(rail, q)).copied().unwrap_or(0), q)) {
                        *has.entry((rail, chip)).or_insert(0) += 1;
                        *ties.entry((c.min(chip), c.max(chip))).or_insert(0.0) += BESIDE_CHIP;
                    }
                }
            }
            let is_tied: BTreeSet<usize> = ties.keys().flat_map(|&(a, b)| [a, b]).collect();
            let nodes: Vec<usize> = own.iter().copied().filter(|p| is_tied.contains(p)).collect();
            let mut groups = communities(&nodes, &ties);
            if groups.is_empty() {
                groups.push(Vec::new());
            }
            groups[0].extend(own.iter().copied().filter(|p| !is_tied.contains(p)));
            for mut group in groups {
                group.sort_by_key(|&p| name(p));
                for &p in &group {
                    atom_of.insert(p, atoms.len());
                }
                atoms.push((of_sheet.clone(), group));
            }
        }
        // (a zone by the sheet most of its parts are drawn on; and by its first parts, if it is not all of it)
        let named = |members: &[usize]| -> String {
            let mut count: BTreeMap<&str, usize> = BTreeMap::new();
            for &atom in members {
                *count.entry(atoms[atom].0.as_str()).or_insert(0) += atoms[atom].1.len();
            }
            let mut most: (&str, usize) = ("", 0);
            for (&of_sheet, &n) in &count {
                if n > most.1 {
                    most = (of_sheet, n);
                }
            }
            let mut of_it: Vec<String> = members
                .iter()
                .filter(|&&atom| atoms[atom].0 == most.0)
                .flat_map(|&atom| names(&atoms[atom].1))
                .collect();
            of_it.sort();
            let some = if most.1 == sheet_size(most.0) {
                String::new()
            } else {
                format!(" ({}{})", of_it[..of_it.len().min(3)].join(", "), if of_it.len() > 3 { " ..." } else { "" })
            };
            let more = count.len() - 1;
            format!("{}{}{}", most.0, some, if more > 0 { format!(" and {more} more") } else { String::new() })
        };
        let local: Vec<Vec<usize>> =
            nets.iter().map(|net| net.iter().copied().filter(|p| mine.contains(p)).collect()).collect();
        let mut ties = tied(&local, &|p| atom_of.get(&p).copied());
        // (a capacitor from a supply to a ground with no chip of its own sheet on that supply is for a chip
        // of another: its group is tied to that chip's, the chips taking them in turn)
        let mut has: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for &c in mine.iter().filter(|&&c| parts[c].capacitor && parts[c].pads.len() == 2) {
            for &rail in parts[c].pads.iter().flatten().filter(|net| found.rails.contains_key(net)) {
                let chips = || mine.iter().filter(|&&q| is_chip(q) && parts[q].pads.contains(&Some(rail)));
                if chips().any(|&q| sheet(q) == sheet(c)) {
                    continue;
                }
                if let Some(&chip) = chips().min_by_key(|&&q| (has.get(&(rail, q)).copied().unwrap_or(0), q)) {
                    *has.entry((rail, chip)).or_insert(0) += 1;
                    if let (Some(&a), Some(&b)) = (atom_of.get(&c), atom_of.get(&chip)) {
                        if a != b {
                            *ties.entry((a.min(b), a.max(b))).or_insert(0.0) += 1.0;
                        }
                    }
                }
            }
        }
        // (and by the supply each would have of the plane: the groups most of whose parts are on one supply
        // are tied by it, GATHERED nets for each, that what is fed alike be gathered together)
        let mut asking: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); atoms.len()];
        for &net in found.rails.keys() {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            reach[net].iter().filter_map(|p| atom_of.get(p)).for_each(|&a| *count.entry(a).or_insert(0) += 1);
            if count.len() > 1 {
                count.iter().for_each(|(&a, &n)| asking[a].push((n, count.len(), net)));
            }
        }
        let mut fed_by: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (a, asks) in asking.iter_mut().enumerate() {
            asks.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
            asks.iter().take(power_layers.len()).for_each(|ask| fed_by.entry(ask.2).or_default().push(a));
        }
        for alike in fed_by.values().filter(|alike| alike.len() > 1) {
            for (i, &a) in alike.iter().enumerate() {
                for &b in &alike[i + 1..] {
                    *ties.entry((a.min(b), a.max(b))).or_insert(0.0) += GATHERED / (alike.len() - 1) as f64;
                }
            }
        }
        let ties = ties;
        let mut levels: Vec<Vec<Vec<usize>>> =
            vec![(0..atoms.len()).filter(|&a| !atoms[a].1.is_empty()).map(|a| vec![a]).collect()];
        loop {
            let last = &levels[levels.len() - 1];
            let next = gathered(last, &ties);
            if next.len() == last.len() || next.len() < 2 {
                break;
            }
            levels.push(next);
        }
        let parts_of = |members: &[usize]| -> Vec<usize> {
            members.iter().flat_map(|&atom| atoms[atom].1.iter().copied()).collect()
        };
        let blank = [0.0; 4];
        let mut kept: Vec<(Vec<usize>, Work)> = levels[0]
            .iter()
            .map(|members| {
                (
                    members.clone(),
                    Work { name: named(members), parts: parts_of(members), inside: Vec::new(), rect: blank, more: 0.0 },
                )
            })
            .collect();
        for level in &levels[1..] {
            let mut made: Vec<(Vec<usize>, Work)> = Vec::new();
            for members in level {
                let of: BTreeSet<usize> = members.iter().copied().collect();
                let (mut inside, rest): (Vec<_>, Vec<_>) =
                    kept.into_iter().partition(|(under, _)| under.iter().all(|a| of.contains(a)));
                kept = rest;
                if inside.len() == 1 {
                    made.push((members.clone(), inside.remove(0).1));
                } else {
                    let mut inner: Vec<Work> = inside.into_iter().map(|(_, work)| work).collect();
                    inner.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.name.cmp(&b.name)));
                    made.push((
                        members.clone(),
                        Work { name: named(members), parts: parts_of(members), inside: inner, rect: blank, more: 0.0 },
                    ));
                }
            }
            kept = made;
        }
        // A sheet whose groups are not gathered together at the first level: drawn as one, tied apart.
        let first = if levels.len() > 1 { &levels[1] } else { &levels[0] };
        let mut first_of: BTreeMap<usize, usize> = BTreeMap::new();
        for (g, members) in first.iter().enumerate() {
            for &atom in members {
                first_of.insert(atom, g);
            }
        }
        for of_sheet in &sheets {
            let own: Vec<usize> =
                (0..atoms.len()).filter(|&a| &atoms[a].0 == of_sheet && !atoms[a].1.is_empty()).collect();
            let into: BTreeSet<usize> = own.iter().map(|a| first_of[a]).collect();
            if into.len() > 1 {
                apart.push((
                    of_sheet.clone(),
                    own.iter().map(|&a| (names(&atoms[a].1), named(&first[first_of[&a]]))).collect(),
                ));
            }
        }
        let mut zones: Vec<Work> = kept.into_iter().map(|(_, work)| work).collect();
        zones.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.name.cmp(&b.name)));
        zones
    };
    let mut top: Vec<Work> = Vec::new();
    for (g, own) in &domains {
        let inside = within(own);
        // (a part counted with another domain that stands on this ground too takes half its ground here)
        let half =
            |p: usize| if found.home[p] != Some(*g) && found.stands[p].contains(g) { parts[p].area / 2.0 } else { 0.0 };
        let more = (0..parts.len()).map(half).sum();
        top.push(Work { name: net_name(*g), parts: own.clone(), inside, rect: [0.0; 4], more });
    }
    if !adrift.is_empty() {
        let inside = within(&adrift);
        top.push(Work { name: "(on no ground)".to_string(), parts: adrift.clone(), inside, rect: [0.0; 4], more: 0.0 });
    }

    // Ground for each zone, from the whole board down.
    let at: Vec<Option<Pt>> = parts.iter().map(|part| (part.fixed && part.placed).then_some(part.at)).collect();
    // The supplies on the plane layers (every plane after the first, the ground's): under each of the smallest
    // zones, as many supplies as there are such layers, those with the most parts there, of the supplies of
    // its ground that reach several zones (one that stays within a zone is laid there, on the layers of its
    // parts). A part on two grounds stands on a gap, beside the pours of both.
    let mut leaf_parts: Vec<(usize, &Vec<usize>)> = Vec::new();
    fn smallest_of<'w>(work: &'w Work, domain: usize, out: &mut Vec<(usize, &'w Vec<usize>)>) {
        if work.inside.is_empty() {
            out.push((domain, &work.parts));
        }
        work.inside.iter().for_each(|inner| smallest_of(inner, domain, out));
    }
    top.iter().enumerate().for_each(|(domain, work)| smallest_of(work, domain, &mut leaf_parts));
    let leaf_at: BTreeMap<usize, usize> =
        leaf_parts.iter().enumerate().flat_map(|(l, (_, own))| own.iter().map(move |&p| (p, l))).collect();
    let rail_nets: Vec<usize> = found.rails.keys().copied().collect();
    let in_leaf: Vec<BTreeMap<usize, usize>> = rail_nets
        .iter()
        .map(|&net| {
            let mut count: BTreeMap<usize, usize> = BTreeMap::new();
            reach[net].iter().filter_map(|p| leaf_at.get(p)).for_each(|&l| *count.entry(l).or_insert(0) += 1);
            count
        })
        .collect();
    let domain_at: BTreeMap<usize, usize> = domains.iter().enumerate().map(|(d, (g, _))| (*g, d)).collect();
    let rail_domain: Vec<Option<usize>> =
        rail_nets.iter().map(|net| found.rails[net].0.and_then(|g| domain_at.get(&g).copied())).collect();
    let mut on_plane: Vec<Vec<usize>> = vec![Vec::new(); leaf_parts.len()];
    for (l, (domain, _)) in leaf_parts.iter().enumerate() {
        let mut asking: Vec<(usize, usize, usize)> = (0..rail_nets.len())
            .filter(|&r| rail_domain[r] == Some(*domain) && in_leaf[r].len() > 1)
            .filter_map(|r| in_leaf[r].get(&l).map(|&count| (count, in_leaf[r].len(), r)))
            .collect();
        asking.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        on_plane[l] = asking.into_iter().take(power_layers.len()).map(|ask| ask.2).collect();
    }
    let rail_parts: Vec<(Vec<usize>, Vec<usize>)> = (0..rail_nets.len())
        .map(|r| {
            let poured = on_plane.iter().any(|rails| rails.contains(&r));
            let fed = |p: &usize| {
                poured
                    && (found.stands[*p].len() > 1
                        || leaf_at.get(p).is_some_and(|&l| on_plane[l].contains(&r))
                        || in_leaf[r].len() < 2)
            };
            reach[rail_nets[r]].iter().copied().partition(fed)
        })
        .collect();
    let kin: Vec<Vec<usize>> = (0..rail_nets.len())
        .map(|r| {
            let under = (0..leaf_parts.len()).filter(|&l| on_plane[l].contains(&r));
            under.filter_map(|l| leaf_parts[l].1.first().copied()).collect()
        })
        .collect();
    // (the pieces each supply's pour is in, given the ground of every smallest zone: zones that are neighbours
    // are one piece, and so are zones joined through ground that has a plane layer to spare)
    let pieces = |rects: &[[f64; 4]]| -> Vec<usize> {
        let count = leaf_parts.len();
        let mut beside: Vec<Vec<usize>> = vec![Vec::new(); count];
        for a in 0..count {
            for b in a + 1..count {
                if leaf_parts[a].0 == leaf_parts[b].0 && strip_between(rects[a], rects[b], board.gap).is_some() {
                    beside[a].push(b);
                    beside[b].push(a);
                }
            }
        }
        (0..rail_nets.len())
            .map(|r| {
                let open = |l: usize| on_plane[l].contains(&r) || on_plane[l].len() < power_layers.len();
                let mut seen = vec![false; count];
                let mut found = 0;
                for from in (0..count).filter(|&l| on_plane[l].contains(&r)) {
                    if seen[from] {
                        continue;
                    }
                    found += 1;
                    let mut edge = vec![from];
                    seen[from] = true;
                    while let Some(l) = edge.pop() {
                        for &next in
                            beside[l].iter().filter(|&&next| open(next) && rail_domain[r] == Some(leaf_parts[next].0))
                        {
                            if !seen[next] {
                                seen[next] = true;
                                edge.push(next);
                            }
                        }
                    }
                }
                found
            })
            .collect()
    };
    // Where the box holds several boards, the domains are parted between them first: in the order their ties
    // give, each board with as many as its room is of all the room (a domain is on one board).
    let overlap = |a: [f64; 4], b: [f64; 4]| {
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0)
    };
    let ground = |r: [f64; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    let room = |r: [f64; 4]| (ground(r) - board.keepouts.iter().map(|&k| overlap(r, k)).sum::<f64>()).max(1.0);
    let mut with_shift: Vec<([f64; 4], Pt)> = if board.regions.is_empty() {
        vec![(board.outline, [0.0, 0.0])]
    } else {
        (0..board.regions.len())
            .map(|k| (board.regions[k], board.shifts.get(k).copied().unwrap_or([0.0, 0.0])))
            .collect()
    };
    with_shift.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]).then(a.0[1].total_cmp(&b.0[1])));
    let boards: Vec<[f64; 4]> = with_shift.iter().map(|board| board.0).collect();
    let shifts: Vec<Pt> = with_shift.iter().map(|board| board.1).collect();
    let need = |work: &Work| work.parts.iter().map(|&p| parts[p].area).sum::<f64>() + work.more;
    let mut to: Vec<usize> = vec![0; top.len()];
    if boards.len() > 1 && top.len() > 1 {
        let mut of_top: BTreeMap<usize, usize> = BTreeMap::new();
        for (z, work) in top.iter().enumerate() {
            of_top.extend(work.parts.iter().map(|&p| (p, z)));
        }
        let line = in_line(top.len(), &tied(&nets, &|p| of_top.get(&p).copied()));
        let mut order: Vec<usize> = (0..top.len()).collect();
        order.sort_by(|&a, &b| line[a].total_cmp(&line[b]).then(a.cmp(&b)));
        let all_room: f64 = boards.iter().map(|&r| room(r)).sum();
        let all_need: f64 = top.iter().map(need).sum::<f64>().max(1.0);
        // (the order, or the order turned round: whichever leaves each board nearer what its room is of all)
        let parted = |order: &[usize]| -> (f64, Vec<usize>) {
            let (mut to, mut at_board, mut taken, mut off) = (vec![0usize; top.len()], 0usize, 0.0f64, 0.0f64);
            for (k, &z) in order.iter().enumerate() {
                let share = room(boards[at_board]) / all_room * all_need;
                let left = order.len() - k;
                let must_move = left <= boards.len() - 1 - at_board;
                let over = taken + need(&top[z]) - share;
                if at_board + 1 < boards.len() && taken > 0.0 && (must_move || over > share - taken) {
                    off += (taken - share).abs();
                    at_board += 1;
                    taken = 0.0;
                }
                to[z] = at_board;
                taken += need(&top[z]);
            }
            off += (taken - room(boards[at_board]) / all_room * all_need).abs();
            (off, to)
        };
        let turned: Vec<usize> = order.iter().rev().copied().collect();
        let (straight, back) = (parted(&order), parted(&turned));
        to = if straight.0 <= back.0 { straight.1 } else { back.1 };
        // What goes from one board to another goes through a connector between them: where the domains are
        // few enough to try every way, they are parted so that as little as may be is tied across boards,
        // each board holding no more than BOARD_FULL of its room; of ways as good, the most even.
        let ways = (boards.len() as f64).powi(top.len() as i32);
        if ways <= 20_000.0 {
            let mut between = tied(&nets, &|p| of_top.get(&p).copied());
            // (a part on two grounds stands on the gap between its two domains: they are on one board)
            for p in (0..parts.len()).filter(|&p| found.stands[p].len() > 1) {
                let on: Vec<usize> = top
                    .iter()
                    .enumerate()
                    .filter(|(_, work)| found.stands[p].iter().any(|&g| all_nets[g].name == work.name))
                    .map(|(z, _)| z)
                    .collect();
                for (i, &a) in on.iter().enumerate() {
                    for &b in &on[i + 1..] {
                        *between.entry((a.min(b), a.max(b))).or_insert(0.0) += ANCHOR;
                    }
                }
            }
            let mut best: Option<(f64, f64, Vec<usize>)> = None;
            // (no board fuller than BOARD_FULL where that can be; else than BOARD_MOST; else as it may)
            for most in [BOARD_FULL, BOARD_MOST, f64::INFINITY] {
                if best.is_some() {
                    break;
                }
                for code in 0..ways as usize {
                    let (mut way, mut rest) = (Vec::with_capacity(top.len()), code);
                    for _ in 0..top.len() {
                        way.push(rest % boards.len());
                        rest /= boards.len();
                    }
                    let fill: Vec<f64> = (0..boards.len())
                        .map(|b| {
                            (0..top.len()).filter(|&z| way[z] == b).map(|z| need(&top[z])).sum::<f64>()
                                / room(boards[b])
                        })
                        .collect();
                    if fill.iter().any(|&f| f > most || f == 0.0) {
                        continue;
                    }
                    let across: f64 = between.iter().filter(|(&(a, b), _)| way[a] != way[b]).map(|(_, w)| *w).sum();
                    let uneven =
                        fill.iter().fold(0.0f64, |m, &f| m.max(f)) - fill.iter().fold(f64::INFINITY, |m, &f| m.min(f));
                    if best.as_ref().map_or(true, |b| (across, uneven) < (b.0, b.1)) {
                        best = Some((across, uneven, way));
                    }
                }
            }
            if let Some((_, _, way)) = best {
                to = way;
            }
        }
    }
    let held: Vec<Vec<String>> = (0..boards.len())
        .map(|b| (0..top.len()).filter(|&z| to[z] == b).map(|z| top[z].name.clone()).collect())
        .collect();
    // The parts that stand on two grounds: each with the domain it is counted with, the others it stands on
    // (the one most of its pads are on first), and the parts its signal nets tie it to.
    let mut spans: Vec<(usize, usize, Vec<usize>, Vec<usize>)> = Vec::new();
    for p in (0..parts.len()).filter(|&p| found.stands[p].len() > 1) {
        let Some(home) = found.home[p].and_then(|g| domain_at.get(&g).copied()) else { continue };
        let pads_on = |g: usize| {
            let on = |net: &&usize| **net == g || found.rails.get(net).is_some_and(|rail| rail.0 == Some(g));
            parts[p].pads.iter().flatten().filter(on).count()
        };
        let mut others: Vec<(usize, usize)> = found.stands[p]
            .iter()
            .filter_map(|&g| domain_at.get(&g).map(|&d| (pads_on(g), d)))
            .filter(|&(_, d)| d != home)
            .collect();
        others.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mates: Vec<usize> = nets
            .iter()
            .filter(|net| net.contains(&p))
            .flat_map(|net| net.iter().copied())
            .filter(|&q| q != p)
            .collect();
        if !others.is_empty() {
            spans.push((p, home, others.into_iter().map(|other| other.1).collect(), mates));
        }
    }
    // One laying out: ground for every zone, the roads, and a way along them for every net between zones.
    type Laid = (Vec<Work>, Vec<Road>, Travel, Vec<(String, f64, Vec<String>, f64)>, Vec<Bridge>);
    type Sided = BTreeMap<(String, String), bool>;
    let lay = |turned: &BTreeSet<String>,
               crossed: &BTreeSet<String>,
               sided: &Sided,
               asked: &BTreeMap<String, usize>|
     -> Laid {
        let mut on = Ground {
            board,
            nets: &nets,
            rails: &rail_parts,
            at: at.clone(),
            roads: Vec::new(),
            region: boards[0],
            shift: shifts[0],
            turned,
            crossed,
            sided,
            asked,
            mates: &mate_of,
            fine: false,
            kin: &kin,
        };
        let mut lots: Vec<Vec<Work>> = (0..boards.len()).map(|_| Vec::new()).collect();
        let mut places: Vec<(usize, usize)> = Vec::new();
        for (z, work) in top.iter().enumerate() {
            places.push((to[z], lots[to[z]].len()));
            lots[to[z]].push(work.clone());
        }
        // (twice over: the second time each board knows where the others' zones came to stand)
        for round in 0..2 {
            for (b, lot) in lots.iter_mut().enumerate() {
                (on.region, on.shift) = (boards[b], shifts[b]);
                if round == 1 {
                    on.roads.retain(|road| overlap(road.rect, boards[b]) <= 0.0);
                }
                arrange(lot, boards[b], 0, true, &mut on);
            }
            if boards.len() < 2 {
                break;
            }
        }
        let mut back: Vec<Vec<Option<Work>>> =
            lots.into_iter().map(|lot| lot.into_iter().map(Some).collect()).collect();
        let laid: Vec<Work> = places.into_iter().filter_map(|(b, k)| back[b][k].take()).collect();
        let mut roads = std::mem::take(&mut on.roads);
        let mut leaves: Vec<(usize, [f64; 4], &Vec<usize>)> = Vec::new();
        fn smallest<'w>(work: &'w Work, domain: usize, leaves: &mut Vec<(usize, [f64; 4], &'w Vec<usize>)>) {
            if work.inside.is_empty() {
                leaves.push((domain, work.rect, &work.parts));
            }
            work.inside.iter().for_each(|inner| smallest(inner, domain, leaves));
        }
        for (domain, work) in laid.iter().enumerate() {
            smallest(work, domain, &mut leaves);
        }
        let mut travel = travelled(&mut roads, &leaves, &nets);
        let rects: Vec<[f64; 4]> = leaves.iter().map(|leaf| leaf.1).collect();
        travel.split = pieces(&rects).iter().map(|&n| n.saturating_sub(1)).sum();
        // (how far what goes from domain to domain reaches, and how far what is to be near a point is from it:
        // by the middles of the smallest zones, as the boards stack)
        let mut spot_of: BTreeMap<usize, Pt> = BTreeMap::new();
        for &(domain, rect, own) in &leaves {
            let (mid, shift) = (middle(rect), shifts[to[domain]]);
            spot_of.extend(own.iter().map(|&p| (p, [mid[0] + shift[0], mid[1] + shift[1]])));
        }
        let domain_of: BTreeMap<usize, usize> =
            leaves.iter().flat_map(|&(domain, _, own)| own.iter().map(move |&p| (p, domain))).collect();
        // What stands on two grounds is given its place: on the gap between its two domains where they are
        // neighbours, along it as near the middle of what it is tied to as the others on that gap allow;
        // else at the place of its own domain nearest the other, as the boards stack.
        // (twice over: the second time each knows where the others came to stand)
        let mut bridges: Vec<Bridge> = Vec::new();
        type Strip = (bool, [f64; 4], Vec<(usize, f64, f64)>);
        for _ in 0..2 {
            bridges.clear();
            travel.cramped = 0.0;
            let mut strips: BTreeMap<[i64; 4], Strip> = BTreeMap::new();
            for (p, home, others, mates) in &spans {
                let (p, home) = (*p, *home);
                let (here, mine) = (shifts[to[home]], laid[home].rect);
                let mut tied_to: Vec<Pt> =
                    mates.iter().filter_map(|q| spot_of.get(q)).map(|s| [s[0] - here[0], s[1] - here[1]]).collect();
                // (what it mates with counts for as much as all its nets together)
                if let Some(s) = mate_of[p].and_then(|other| spot_of.get(&other)) {
                    tied_to.extend(vec![[s[0] - here[0], s[1] - here[1]]; tied_to.len().max(1)]);
                }
                let want = median(&tied_to).unwrap_or(middle(mine));
                let beside = others.iter().find_map(|&o| {
                    let strip = if to[o] == to[home] { strip_between(mine, laid[o].rect, board.gap) } else { None };
                    strip.map(|strip| (o, strip))
                });
                let grounds = |o: usize| [top[home].name.clone(), top[o].name.clone()];
                let (on, at, reach, other) = match beside {
                    Some((o, (strip, up))) => {
                        let key = strip.map(|v| (v * 1000.0).round() as i64);
                        let entry = strips.entry(key).or_insert((up, strip, Vec::new()));
                        entry.2.push((bridges.len(), want[usize::from(up)], parts[p].area.sqrt() + BESIDE));
                        (Across::Gap, middle(strip), 0.0, o)
                    }
                    None => {
                        let (o, into) =
                            (others[0], |p: Pt, r: [f64; 4]| [p[0].clamp(r[0], r[2]), p[1].clamp(r[1], r[3])]);
                        let (there, r) = (shifts[to[o]], laid[o].rect);
                        let d = [there[0] - here[0], there[1] - here[1]];
                        let theirs = [r[0] + d[0], r[1] + d[1], r[2] + d[0], r[3] + d[1]];
                        let at = into(into(want, theirs), mine);
                        // (two grounds corner to corner across the gap are neighbours enough: it stands at the corner)
                        let (far, same) = (dist(at, into(at, theirs)), to[o] == to[home]);
                        if same && far <= CORNER * board.gap {
                            let to = into(at, theirs);
                            (Across::Gap, [(at[0] + to[0]) / 2.0, (at[1] + to[1]) / 2.0], 0.0, o)
                        } else {
                            (if same { Across::Apart } else { Across::Link }, at, far, o)
                        }
                    }
                };
                bridges.push(Bridge { part: parts[p].name.clone(), grounds: grounds(other), on, at, reach, off: None });
            }
            for (up, strip, on_it) in strips.into_values() {
                let along = usize::from(up);
                let (want, size): (Vec<f64>, Vec<f64>) = on_it.iter().map(|&(_, want, size)| (want, size)).unzip();
                travel.cramped += (size.iter().sum::<f64>() - (strip[along + 2] - strip[along])).max(0.0);
                for (&(b, _, _), place) in on_it.iter().zip(spread(&want, &size, strip[along], strip[along + 2])) {
                    bridges[b].at[along] = place;
                }
            }
            for (bridge, (p, home, _, _)) in bridges.iter().zip(&spans) {
                let here = shifts[to[*home]];
                spot_of.insert(*p, [bridge.at[0] + here[0], bridge.at[1] + here[1]]);
            }
        }
        travel.cramped = (travel.cramped * 10.0).round() / 10.0;
        travel.apart = bridges.iter().filter(|bridge| bridge.on == Across::Apart).count();
        for (p, other) in mate_of.iter().enumerate().filter_map(|(p, other)| other.filter(|&o| o > p).map(|o| (p, o))) {
            if let (Some(&a), Some(&b)) = (spot_of.get(&p), spot_of.get(&other)) {
                travel.mate_mm += dist(a, b);
            }
        }
        travel.mate_mm = travel.mate_mm.round();
        for (bridge, (p, _, _, _)) in bridges.iter_mut().zip(&spans) {
            bridge.off = parts[*p].placed.then(|| (dist(parts[*p].at, bridge.at) * 10.0).round() / 10.0);
            bridge.at = bridge.at.map(|v| (v * 100.0).round() / 100.0);
            bridge.reach = (bridge.reach * 10.0).round() / 10.0;
        }
        for net in nets.iter() {
            let domains: BTreeSet<usize> = net.iter().filter_map(|p| domain_of.get(p).copied()).collect();
            if domains.len() > 1 {
                let spots: Vec<Pt> = net.iter().filter_map(|p| spot_of.get(p).copied()).collect();
                let low = |k: usize| spots.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
                let high = |k: usize| spots.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
                travel.through_mm += high(0) - low(0) + high(1) - low(1);
            }
        }
        for (p, part) in parts.iter().enumerate() {
            if let (Some(near), Some(&spot)) = (part.near, spot_of.get(&p)) {
                travel.near_mm += dist(near, spot);
            }
        }
        (travel.through_mm, travel.near_mm) = (travel.through_mm.round(), travel.near_mm.round());
        // (a zone given ground where there is little height, with parts too tall for it)
        let mut tall: Vec<(String, f64, Vec<String>, f64)> = Vec::new();
        fn named<'w>(work: &'w Work, out: &mut Vec<&'w Work>) {
            if work.inside.is_empty() {
                out.push(work);
            }
            work.inside.iter().for_each(|inner| named(inner, out));
        }
        let mut smallest_zones: Vec<&Work> = Vec::new();
        laid.iter().for_each(|work| named(work, &mut smallest_zones));
        for zone in smallest_zones {
            let all = ground(zone.rect).max(1e-9);
            for limit in &board.limits {
                let under = overlap(zone.rect, limit.rect) / all;
                let mut too: Vec<String> = zone
                    .parts
                    .iter()
                    .filter(|&&p| parts[p].height.is_some_and(|h| h > limit.height))
                    .map(|&p| parts[p].name.clone())
                    .collect();
                if under > 1e-6 && !too.is_empty() {
                    too.sort();
                    let highest = zone.parts.iter().filter_map(|&p| parts[p].height).fold(0.0, f64::max);
                    tall.push((zone.name.clone(), (under * 100.0).round() / 100.0, too, highest));
                }
            }
        }
        (laid, roads, travel, tall, bridges)
    };
    // What a laying out is weighed by: its track, each layer change as VIA_MM, and dearly what does not fit.
    let weight = |laid: &Laid| -> f64 {
        let (roads, travel) = (&laid.1, &laid.2);
        let over: usize = roads.iter().filter(|road| !road.gap).map(|road| road.used.saturating_sub(road.lanes)).sum();
        let too_tall: f64 = laid.3.iter().map(|(_, under, too, _)| under * too.len() as f64).sum();
        travel.mm
            + travel.through_mm
            + NEAR * travel.near_mm
            + NEAR * travel.mate_mm
            + CRAMPED_MM * travel.cramped
            + STRANDED_MM * travel.apart as f64
            + OVER_MM * travel.split as f64
            + VIA_MM * travel.vias as f64
            + OVER_MM * over as f64
            + STRANDED_MM * travel.stranded as f64
            + TALL_MM * too_tall
    };
    let (mut turned, mut crossed, mut asked) = (BTreeSet::new(), BTreeSet::new(), BTreeMap::new());
    let mut sided: Sided = BTreeMap::new();
    let mut best = lay(&turned, &crossed, &sided, &asked);
    let first = best.2.clone();
    // A road asked for more than it holds is made to hold it, and the board laid out again: a few times, for
    // a wider road moves what is beside it.
    for _ in 0..WIDENINGS {
        let over: Vec<(String, usize)> = best
            .1
            .iter()
            .filter(|road| !road.gap && road.used > road.lanes)
            .map(|road| (road.cut.clone(), road.used + road.used / 4 + 1))
            .collect();
        if over.is_empty() {
            break;
        }
        for (cut, lanes) in over {
            let held = asked.entry(cut).or_insert(0);
            *held = (*held).max(lanes);
        }
        best = lay(&turned, &crossed, &sided, &asked);
    }
    // Then each cut in turn, the widest roads first, tried three other ways: its two lots changed over, made
    // across the shorter side of its ground, and both; what leaves the whole the better stays. The cuts that
    // part the most of the board are gone over twice, for one changes what another is worth.
    for round in 0..2 {
        let mut cuts: Vec<(usize, String)> = best.1.iter().map(|road| (road.level, road.cut.clone())).collect();
        cuts.sort();
        cuts.dedup();
        let most = if round == 0 { TURNINGS } else { TURNINGS / 8 };
        let mut better = false;
        for (level, cut) in cuts.into_iter().take(most) {
            for way in 1..4usize {
                if way > 1 && level > CROSS_LEVELS {
                    break;
                }
                let (mut other, mut across) = (turned.clone(), crossed.clone());
                if way & 1 == 1 && !other.remove(&cut) {
                    other.insert(cut.clone());
                }
                if way & 2 == 2 && !across.remove(&cut) {
                    across.insert(cut.clone());
                }
                let now = lay(&other, &across, &sided, &asked);
                if weight(&now) < weight(&best) - 1e-6 {
                    (turned, crossed, best, better) = (other, across, now, true);
                }
            }
        }
        // And at the cuts that part the most of the board, each zone in turn is put in the other lot.
        let wide_cuts: Vec<(String, [Vec<String>; 2])> = best
            .1
            .iter()
            .filter(|road| road.level <= MOVE_LEVELS)
            .map(|road| (road.cut.clone(), road.between.clone()))
            .collect();
        for (cut, lots) in wide_cuts {
            for (side, names) in lots.iter().enumerate() {
                for name in names {
                    let mut other = sided.clone();
                    other.insert((cut.clone(), name.clone()), side == 1);
                    let now = lay(&turned, &crossed, &other, &asked);
                    if weight(&now) < weight(&best) - 1e-6 {
                        (sided, best, better) = (other, now, true);
                    }
                }
            }
        }
        if !better {
            break;
        }
    }
    // (what the changes asked of the roads is seen to once more)
    for _ in 0..WIDENINGS {
        let over: Vec<(String, usize)> = best
            .1
            .iter()
            .filter(|road| !road.gap && road.used > road.lanes)
            .map(|road| (road.cut.clone(), road.used + road.used / 4 + 1))
            .collect();
        if over.is_empty() {
            break;
        }
        for (cut, lanes) in over {
            let held = asked.entry(cut).or_insert(0);
            *held = (*held).max(lanes);
        }
        let now = lay(&turned, &crossed, &sided, &asked);
        // (kept if less is left over than before: that is what it was done for)
        let left_over = |laid: &Laid| -> usize {
            laid.1.iter().filter(|road| !road.gap).map(|road| road.used.saturating_sub(road.lanes)).sum()
        };
        if left_over(&now) >= left_over(&best) {
            break;
        }
        best = now;
    }
    let (laid, mut roads, travel, tall, bridges) = best;
    let top = laid;
    // Ground for the vias where ways turn: each takes its own and as much again, in a square where the roads meet.
    let mut junctions: Vec<Junction> = travel
        .turns
        .iter()
        .map(|&(at, changes, has)| {
            let needs = 2.0 * changes as f64 * board.via * board.via;
            Junction {
                at: at.map(|v| (v * 100.0).round() / 100.0),
                changes,
                needs: (needs * 10.0).round() / 10.0,
                has: (has * 100.0).round() / 100.0,
                side: if needs > has { (needs.sqrt() * 10.0).round() / 10.0 } else { 0.0 },
            }
        })
        .collect();
    junctions
        .sort_by(|a, b| b.changes.cmp(&a.changes).then(a.at[0].total_cmp(&b.at[0])).then(a.at[1].total_cmp(&b.at[1])));
    // A place for every part within its zone. The zones are gone over twice: the second time each part knows
    // where those of the other zones came to stand.
    let on_gap: BTreeMap<usize, Pt> = bridges
        .iter()
        .filter_map(|bridge| parts.iter().position(|part| part.name == bridge.part).map(|p| (p, bridge.at)))
        .collect();
    let mut fine_leaves: Vec<(usize, [f64; 4], Vec<usize>)> = Vec::new();
    fn parts_of_leaves(work: &Work, domain: usize, out: &mut Vec<(usize, [f64; 4], Vec<usize>)>) {
        if work.inside.is_empty() {
            out.push((domain, work.rect, work.parts.clone()));
        }
        work.inside.iter().for_each(|inner| parts_of_leaves(inner, domain, out));
    }
    top.iter().enumerate().for_each(|(domain, work)| parts_of_leaves(work, domain, &mut fine_leaves));
    let mut stand: Vec<Option<Pt>> = vec![None; parts.len()];
    for (domain, rect, own) in &fine_leaves {
        let (mid, shift) = (middle(*rect), shifts[to[*domain]]);
        own.iter().for_each(|&p| stand[p] = Some([mid[0] + shift[0], mid[1] + shift[1]]));
    }
    for (domain, _, own) in &fine_leaves {
        let shift = shifts[to[*domain]];
        for (&p, at) in own.iter().filter_map(|p| on_gap.get(p).map(|at| (p, at))) {
            stand[p] = Some([at[0] + shift[0], at[1] + shift[1]]);
        }
    }
    // (the nets of the parts: the signals, and for each capacitor from a supply to a ground a tie to a chip of
    // its zone on that supply, the chips taking them in turn)
    let mut fine_nets: Vec<Vec<usize>> = nets.clone();
    let mut bypasses: Vec<(usize, usize, usize)> = Vec::new(); // (the capacitor, its chip, the supply)
    let mut taken: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (domain, _, own) in &fine_leaves {
        for &c in own.iter().filter(|&&c| parts[c].capacitor && parts[c].pads.len() == 2) {
            for &rail in parts[c].pads.iter().flatten().filter(|net| found.rails.contains_key(net)) {
                // (a chip of its zone; failing that, one of its domain)
                let of = |among: &Vec<usize>| {
                    among
                        .iter()
                        .copied()
                        .filter(|&q| is_chip(q) && parts[q].pads.contains(&Some(rail)))
                        .min_by_key(|&q| (taken.get(&(rail, q)).copied().unwrap_or(0), q))
                };
                let chip = of(own).or_else(|| of(&top[*domain].parts));
                if let Some(chip) = chip {
                    *taken.entry((rail, chip)).or_insert(0) += 1;
                    fine_nets.extend(vec![vec![c, chip]; BYPASS]);
                    if bypasses.last().map_or(true, |last| last.0 != c) {
                        bypasses.push((c, chip, rail));
                    }
                }
            }
        }
    }
    let (none, no_sides, no_asks, no_kin) = (BTreeSet::new(), BTreeMap::new(), BTreeMap::new(), Vec::new());
    let no_rails: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut grounds: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
    for _ in 0..2 {
        for (domain, rect, own) in &fine_leaves {
            let mut each: Vec<Work> = own
                .iter()
                .filter(|p| !on_gap.contains_key(p))
                .map(|&p| Work { name: name(p), parts: vec![p], inside: Vec::new(), rect: [0.0; 4], more: 0.0 })
                .collect();
            let mut on = Ground {
                board,
                nets: &fine_nets,
                rails: &no_rails,
                at: std::mem::take(&mut stand),
                roads: Vec::new(),
                region: boards[to[*domain]],
                shift: shifts[to[*domain]],
                turned: &none,
                crossed: &none,
                sided: &no_sides,
                asked: &no_asks,
                mates: &mate_of,
                fine: true,
                kin: &no_kin,
            };
            arrange(&mut each, *rect, 0, false, &mut on);
            stand = std::mem::take(&mut on.at);
            grounds.extend(each.iter().map(|work| (work.parts[0], work.rect)));
        }
    }
    // Then the bodies themselves are settled on the ground of each zone, each as near the middle of the ground
    // that came to it as the others allow, the largest first; no part on ground that is shut.
    let round2 = |v: f64| (v * 100.0).round() / 100.0;
    let mut places: Vec<Place> = Vec::new();
    let mut spot: Vec<Option<Pt>> = vec![None; parts.len()];
    let body = |p: usize| {
        let side = parts[p].area.sqrt();
        parts[p].size.unwrap_or([side, side])
    };
    // (where each came to stand: its middle and whether it is turned; None where there was no room. A domain's
    // bodies are settled together, off its roads and what is shut: a large part may reach over the ground of
    // its zone, which was cut by what its parts take and not by their shapes)
    let mut stood: BTreeMap<usize, Option<(Pt, bool)>> = BTreeMap::new();
    let taken = |p: usize, (at, turn): (Pt, bool)| {
        let [w, h] = body(p);
        let (w, h) = if turn { (h, w) } else { (w, h) };
        [at[0] - w / 2.0, at[1] - h / 2.0, at[0] + w / 2.0, at[1] + h / 2.0]
    };
    // First the parts with holes and the large ones, over the whole ground of their domain: they are the
    // hardest to find room for. No track runs under a part with holes: it stands off the gaps and the roads
    // that part much of the board (a lesser road, a track or two, goes round it); the others may stand over
    // a road, whose tracks run on the layers beneath.
    let goes_first = |p: usize| parts[p].through || body(p)[0] * body(p)[1] >= LARGE;
    for work in top.iter() {
        let mine: Vec<usize> =
            work.parts.iter().copied().filter(|&p| grounds.contains_key(&p) && goes_first(p)).collect();
        let mut found: Vec<[f64; 4]> = Vec::new();
        for with_holes in [true, false] {
            let lot: Vec<usize> = mine.iter().copied().filter(|&p| parts[p].through == with_holes).collect();
            let mut shut: Vec<[f64; 4]> = board.keepouts.clone();
            shut.extend(
                roads.iter().filter(|road| road.gap || (with_holes && road.level <= MAIN)).map(|road| road.rect),
            );
            shut.extend(found.iter().copied());
            let bodies: Vec<([f64; 2], Pt)> = lot.iter().map(|&p| (body(p), middle(grounds[&p]))).collect();
            for (&p, at) in lot.iter().zip(settled(work.rect, &bodies, &shut)) {
                found.extend(at.map(|at| taken(p, at)));
                stood.insert(p, at);
            }
        }
    }
    // Then the others, zone by zone, each on the ground of its own.
    let settled_first: Vec<[f64; 4]> = stood.iter().filter_map(|(&p, at)| at.map(|at| taken(p, at))).collect();
    for (_, rect, own) in &fine_leaves {
        let mine: Vec<usize> = own.iter().copied().filter(|&p| grounds.contains_key(&p) && !goes_first(p)).collect();
        let mut shut: Vec<[f64; 4]> = board.keepouts.clone();
        shut.extend(settled_first.iter().copied());
        let bodies: Vec<([f64; 2], Pt)> = mine.iter().map(|&p| (body(p), middle(grounds[&p]))).collect();
        stood.extend(mine.iter().copied().zip(settled(*rect, &bodies, &shut)));
    }
    // What found no room in its zone (a zone cut by what its parts take, not by their shapes) is settled on
    // what is left of the ground of its domain, off every body that has its place: first the parts with
    // holes, off the roads and gaps too; then the others, which may stand over a road (its tracks run on
    // the layers beneath them) but over no gap.
    for work in top.iter() {
        for with_holes in [true, false] {
            let left: Vec<usize> = work
                .parts
                .iter()
                .copied()
                .filter(|&p| stood.get(&p) == Some(&None) && parts[p].through == with_holes)
                .collect();
            if left.is_empty() {
                continue;
            }
            let mut shut: Vec<[f64; 4]> = board.keepouts.clone();
            shut.extend(
                roads.iter().filter(|road| road.gap || (with_holes && road.level <= MAIN)).map(|road| road.rect),
            );
            shut.extend(work.parts.iter().filter_map(|&p| stood.get(&p).copied().flatten().map(|at| taken(p, at))));
            let bodies: Vec<([f64; 2], Pt)> = left.iter().map(|&p| (body(p), middle(grounds[&p]))).collect();
            stood.extend(left.iter().copied().zip(settled(work.rect, &bodies, &shut)));
        }
    }
    // Where the middle of each part is (as its board is drawn), how many quarters it is turned, and what is
    // added to its place to say where that is in the stack.
    let mut centre: Vec<Option<Pt>> = vec![None; parts.len()];
    let mut quarter: Vec<u8> = vec![0; parts.len()];
    let mut shift_of: Vec<Pt> = vec![[0.0, 0.0]; parts.len()];
    let mut domain_of_part: Vec<usize> = vec![0; parts.len()];
    for (domain, _, own) in &fine_leaves {
        for &p in own {
            (shift_of[p], domain_of_part[p]) = (shifts[to[*domain]], *domain);
            match (on_gap.get(&p), stood.get(&p)) {
                (Some(&at), _) => centre[p] = Some(at),
                (None, Some(&Some((at, turn)))) => (centre[p], quarter[p]) = (Some(at), u8::from(turn)),
                (None, Some(None)) => centre[p] = Some(middle(grounds[&p])),
                _ => {}
            }
        }
    }
    // Then each part is turned so that its pads face what they are joined to: of the ways its body may lie
    // (as it was settled, or that turned half round; any of the four for a square one or one on a gap), the
    // one that leaves its pads nearest the nearest other pad of their nets. A capacitor that stands for a
    // chip counts the way from its pad on the supply to the pad of the chip it is for.
    let mut pads_on: Vec<Vec<(usize, usize)>> = vec![Vec::new(); all_nets.len()];
    for (p, part) in parts.iter().enumerate() {
        for (i, net) in part.pads.iter().enumerate() {
            if let Some(net) = net.filter(|&net| is_signal(net)) {
                pads_on[net].push((p, i));
            }
        }
    }
    let pad_at = |p: usize, i: usize, quarters: u8, centre: &[Option<Pt>]| -> Option<Pt> {
        let at = centre[p]?;
        let from = parts[p].offsets.get(i).map_or([0.0, 0.0], |&offset| quartered(quarters, offset));
        Some([at[0] + shift_of[p][0] + from[0], at[1] + shift_of[p][1] + from[1]])
    };
    type Pins = BTreeMap<usize, (usize, usize, usize)>; // a capacitor -> (its pad, its chip, the chip's pad)
    let reach_of = |p: usize, quarters: u8, centre: &[Option<Pt>], quarter: &[u8], pins: &Pins| -> f64 {
        let mut all = 0.0;
        for (i, net) in parts[p].pads.iter().enumerate() {
            let (Some(net), Some(here)) = (net, pad_at(p, i, quarters, centre)) else { continue };
            let others = pads_on[*net].iter().filter(|other| other.0 != p);
            let nearest = others
                .filter_map(|&(q, j)| pad_at(q, j, quarter[q], centre))
                .map(|there| (there[0] - here[0]).abs() + (there[1] - here[1]).abs())
                .fold(f64::INFINITY, f64::min);
            if nearest.is_finite() {
                all += nearest;
            }
        }
        if let Some(&(i, chip, j)) = pins.get(&p) {
            if let (Some(here), Some(there)) = (pad_at(p, i, quarters, centre), pad_at(chip, j, quarter[chip], centre))
            {
                all += BYPASS as f64 * ((there[0] - here[0]).abs() + (there[1] - here[1]).abs());
            }
        }
        all
    };
    let face = |centre: &[Option<Pt>], quarter: &mut Vec<u8>, pins: &Pins| {
        for _ in 0..2 {
            for p in 0..parts.len() {
                if centre[p].is_none() || parts[p].offsets.is_empty() || stays(&parts[p]) {
                    continue;
                }
                let [w, h] = body(p);
                let any = on_gap.contains_key(&p) || (w - h).abs() < SETTLE;
                let ways: Vec<u8> = if any { vec![0, 1, 2, 3] } else { vec![quarter[p] % 2, quarter[p] % 2 + 2] };
                let mut best = (reach_of(p, quarter[p], centre, quarter, pins), quarter[p]);
                for way in ways {
                    let far = reach_of(p, way, centre, quarter, pins);
                    if far < best.0 - 1e-6 {
                        best = (far, way);
                    }
                }
                quarter[p] = best.1;
            }
        }
    };
    let no_pins: Pins = BTreeMap::new();
    let all_reach = |centre: &[Option<Pt>], quarter: &[u8]| -> f64 {
        (0..parts.len()).map(|p| reach_of(p, quarter[p], centre, quarter, &no_pins)).sum()
    };
    let pads_first_mm = all_reach(&centre, &quarter);
    face(&centre, &mut quarter, &no_pins);
    // A capacitor that stands for a chip is moved beside the pad of the chip that is on its supply (the pads
    // of the chip taking them in turn, the smallest capacitors first: they want to be the nearest), just off
    // the chip's body, as near that as what stands there leaves room; if that is nearer the pad than it stood.
    let mut pins: Pins = BTreeMap::new();
    let mut at_pad: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let with_capacitors: BTreeSet<usize> = bypasses.iter().map(|bypass| bypass.1).collect();
    let mut by_its_pad: BTreeSet<usize> = BTreeSet::new();
    let mut order = bypasses.clone();
    order.sort_by(|a, b| {
        let held = |c: usize| farads(&parts[c].value).unwrap_or(f64::INFINITY);
        held(a.0).total_cmp(&held(b.0)).then(parts[a.0].name.cmp(&parts[b.0].name))
    });
    for (c, chip, rail) in order {
        let (Some(was), Some(chip_at)) = (centre[c], centre[chip]) else { continue };
        if parts[chip].offsets.is_empty() || on_gap.contains_key(&c) {
            continue;
        }
        let pad_of = |i: usize| {
            let from = quartered(quarter[chip], parts[chip].offsets[i]);
            [chip_at[0] + from[0], chip_at[1] + from[1]]
        };
        let on_rail = (0..parts[chip].pads.len()).filter(|&i| parts[chip].pads[i] == Some(rail));
        let Some(pin) = on_rail.min_by(|&a, &b| {
            let taken = |i: usize| at_pad.get(&(chip, i)).copied().unwrap_or(0);
            taken(a).cmp(&taken(b)).then(dist(pad_of(a), was).total_cmp(&dist(pad_of(b), was))).then(a.cmp(&b))
        }) else {
            continue;
        };
        *at_pad.entry((chip, pin)).or_insert(0) += 1;
        if let Some(mine) = parts[c].pads.iter().position(|&net| net == Some(rail)) {
            pins.insert(c, (mine, chip, pin));
        }
        // (just off the chip's body, on the side its pad is nearest)
        let (pad, [w, h]) = (pad_of(pin), body(chip));
        let (w, h) = if quarter[chip] % 2 == 1 { (h, w) } else { (w, h) };
        let chip_body = [chip_at[0] - w / 2.0, chip_at[1] - h / 2.0, chip_at[0] + w / 2.0, chip_at[1] + h / 2.0];
        // (lying away from the chip, its own pad on the supply toward the chip's: so the track between is short)
        let [cw, ch] = body(c);
        let (long, short) = (cw.max(ch), cw.min(ch));
        let half = long / 2.0;
        let sides = [
            (pad[0] - chip_body[0], [chip_body[0] - half, pad[1]], true),
            (chip_body[2] - pad[0], [chip_body[2] + half, pad[1]], true),
            (pad[1] - chip_body[1], [pad[0], chip_body[1] - half], false),
            (chip_body[3] - pad[1], [pad[0], chip_body[3] + half], false),
        ];
        let (_, want, along_x) = sides.iter().fold(sides[0], |best, side| if side.0 < best.0 { *side } else { best });
        let lying = if along_x { [long, short] } else { [short, long] };
        // (whether lying so is a quarter from how its body is given)
        let quarter_more = (lying[0] - cw).abs() > 1e-9;
        // It takes the place it wants there though a small part stands on it: that one is settled again, as
        // near where it stood as may be. What is not moved for it: what was settled first, a chip that has
        // capacitors of its own, and a capacitor that has its place by its pad.
        let domain = domain_of_part[c];
        let (ground, mine) = (top[domain].rect, &top[domain].parts);
        let stands = |q: usize| !on_gap.contains_key(&q) && stood.get(&q).is_some_and(|at| at.is_some());
        let hard = |q: usize| goes_first(q) || with_capacitors.contains(&q) || by_its_pad.contains(&q);
        let rect_of = |q: usize, centre: &[Option<Pt>], quarter: &[u8]| {
            centre[q].filter(|_| stands(q)).map(|at| taken(q, (at, quarter[q] % 2 == 1)))
        };
        let mut shut: Vec<[f64; 4]> = board.keepouts.clone();
        shut.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
        let open = shut.clone();
        shut.extend(mine.iter().filter(|&&q| q != c && hard(q)).filter_map(|&q| rect_of(q, &centre, &quarter)));
        let Some((at, turn)) = settled(ground, &[(lying, want)], &shut)[0] else { continue };
        if dist(at, pad) >= dist(was, pad) - 1e-6 {
            continue;
        }
        let turn = turn != quarter_more;
        let (mut new_centre, mut new_quarter) = (centre.clone(), quarter.clone());
        (new_centre[c], new_quarter[c]) = (Some(at), u8::from(turn));
        let here = taken(c, (at, turn));
        let over =
            |r: [f64; 4]| r[2].min(here[2]) - r[0].max(here[0]) > 1e-6 && r[3].min(here[3]) - r[1].max(here[1]) > 1e-6;
        let mut moved: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|&q| q != c && !hard(q) && rect_of(q, &centre, &quarter).is_some_and(over))
            .collect();
        moved.sort_by(|&a, &b| parts[b].area.total_cmp(&parts[a].area).then(a.cmp(&b)));
        let mut done = true;
        for &q in &moved {
            let mut shut = open.clone();
            let others = mine.iter().filter(|&&other| other != q && (other == c || stands(other)));
            shut.extend(
                others.filter_map(|&other| new_centre[other].map(|at| taken(other, (at, new_quarter[other] % 2 == 1)))),
            );
            match (centre[q], settled(ground, &[(body(q), centre[q].unwrap_or(at))], &shut)[0]) {
                (Some(_), Some((to, turn))) => (new_centre[q], new_quarter[q]) = (Some(to), u8::from(turn)),
                _ => done = false,
            }
        }
        // (and only if what it moves and itself are the nearer what they are joined to for it, all told)
        let reach = |centre: &[Option<Pt>], quarter: &[u8]| -> f64 {
            moved.iter().chain([&c]).map(|&q| reach_of(q, quarter[q], centre, quarter, &pins)).sum()
        };
        if done && reach(&new_centre, &new_quarter) < reach(&centre, &quarter) - 1e-6 {
            for &q in moved.iter().chain([&c]) {
                if let Some(at) = new_centre[q] {
                    stood.insert(q, Some((at, new_quarter[q] % 2 == 1)));
                }
            }
            (centre, quarter) = (new_centre, new_quarter);
            by_its_pad.insert(c);
        }
    }
    face(&centre, &mut quarter, &pins);
    let pads_mm = all_reach(&centre, &quarter);
    let beside = pins
        .iter()
        .filter(|(&c, &(mine, chip, pin))| {
            match (pad_at(c, mine, quarter[c], &centre), pad_at(chip, pin, quarter[chip], &centre)) {
                (Some(here), Some(there)) => dist(here, there) <= BESIDE_PIN,
                _ => false,
            }
        })
        .count();
    let bypass = [pins.len(), beside];
    let mut capacitors: Vec<(String, String, f64, usize)> = pins
        .iter()
        .filter_map(|(&c, &(mine, chip, pin))| {
            let (here, there) = (pad_at(c, mine, quarter[c], &centre)?, pad_at(chip, pin, quarter[chip], &centre)?);
            let shared = at_pad.get(&(chip, pin)).copied().unwrap_or(1);
            Some((name(c), name(chip), (dist(here, there) * 10.0).round() / 10.0, shared))
        })
        .collect();
    capacitors.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    for (_, _, own) in &fine_leaves {
        for &p in own {
            let Some(at) = centre[p] else { continue };
            let settled_here = stood.get(&p).is_some_and(|at| at.is_some());
            let (rect, fits) = match (on_gap.contains_key(&p), settled_here) {
                (true, _) => (None, true),
                (false, true) => (Some(taken(p, (at, quarter[p] % 2 == 1))), true),
                (false, false) => (grounds.get(&p).copied(), false),
            };
            spot[p] = Some([at[0] + shift_of[p][0], at[1] + shift_of[p][1]]);
            places.push(Place {
                part: name(p),
                at: at.map(round2),
                rect: rect.map(|r| r.map(round2)),
                turn: 90 * u16::from(quarter[p] % 4),
                fits,
                off: parts[p].placed.then(|| (dist(parts[p].at, at) * 10.0).round() / 10.0),
            });
        }
    }
    // (what is the board's own stays where it stands)
    for part in parts.iter().filter(|part| stays(part)) {
        places.push(Place {
            part: part.name.clone(),
            at: part.at.map(round2),
            rect: None,
            turn: 0,
            fits: true,
            off: Some(0.0),
        });
    }
    places.sort_by(|a, b| a.part.cmp(&b.part));
    // (how long the signal nets are with the parts as they stand, and with each at its place: nets all of
    // whose parts stand on the board today; as the boards stack)
    let (mut parts_now_mm, mut parts_then_mm) = (0.0, 0.0);
    let part_shift: BTreeMap<usize, Pt> =
        fine_leaves.iter().flat_map(|(domain, _, own)| own.iter().map(|&p| (p, shifts[to[*domain]]))).collect();
    for net in &nets {
        if !net.iter().all(|&p| parts[p].placed && spot[p].is_some()) {
            continue;
        }
        let span = |points: Vec<Pt>| {
            let low = |k: usize| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| points.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            high(0) - low(0) + high(1) - low(1)
        };
        // (today the parts stand as the file draws them: on the board they are drawn on)
        let today = |p: usize| {
            let home = boards.iter().position(|b| parts[p].at[0] >= b[0] && parts[p].at[0] <= b[2]);
            let shift = home.map_or(part_shift[&p], |b| shifts[b]);
            [parts[p].at[0] + shift[0], parts[p].at[1] + shift[1]]
        };
        parts_now_mm += span(net.iter().map(|&p| today(p)).collect());
        parts_then_mm += span(net.iter().filter_map(|&p| spot[p]).collect());
    }
    // The pours, as they are told.
    let mut rects: Vec<[f64; 4]> = Vec::new();
    fn grounds_of(work: &Work, out: &mut Vec<[f64; 4]>) {
        if work.inside.is_empty() {
            out.push(work.rect);
        }
        work.inside.iter().for_each(|inner| grounds_of(inner, out));
    }
    top.iter().for_each(|work| grounds_of(work, &mut rects));
    let in_pieces = pieces(&rects);
    let mut pours: Vec<Pour> = (0..rail_nets.len())
        .filter(|&r| in_pieces[r] > 0)
        .map(|r| {
            let under: Vec<[f64; 4]> =
                (0..rects.len()).filter(|&l| on_plane[l].contains(&r)).map(|l| rects[l]).collect();
            let fed = |p: &&usize| leaf_at.get(p).is_some_and(|&l| on_plane[l].contains(&r));
            Pour {
                net: net_name(rail_nets[r]),
                ground: found.rails[&rail_nets[r]].0.map(net_name).unwrap_or_default(),
                layers: power_layers.clone(),
                area: under.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum::<f64>().round(),
                rects: under.into_iter().map(|r| r.map(|v| (v * 100.0).round() / 100.0)).collect(),
                parts: reach[rail_nets[r]].iter().filter(fed).count(),
                by_track: names(&rail_parts[r].1),
                pieces: in_pieces[r],
            }
        })
        .collect();
    pours.sort_by(|a, b| b.area.total_cmp(&a.area).then(a.net.cmp(&b.net)));
    // (what stands on a gap: the parts on a ground of either side)
    for road in roads.iter_mut().filter(|road| road.gap) {
        let side = |p: usize, names: &Vec<String>| found.stands[p].iter().any(|&g| names.contains(&all_nets[g].name));
        let mut on_it: Vec<String> = (0..parts.len())
            .filter(|&p| side(p, &road.between[0]) && side(p, &road.between[1]))
            .map(|p| parts[p].name.clone())
            .collect();
        on_it.sort();
        road.parts = on_it;
    }
    let regions: Vec<Region> = boards
        .iter()
        .zip(&held)
        .map(|(&rect, names)| Region {
            rect,
            room: room(rect).round(),
            needed: top.iter().filter(|work| names.contains(&work.name)).map(need).sum::<f64>().round(),
            roads: roads.iter().map(|road| overlap(road.rect, rect)).sum::<f64>().round(),
            zones: names.clone(),
        })
        .collect();
    roads.sort_by(|a, b| a.level.cmp(&b.level).then(b.width.total_cmp(&a.width)));

    // What flows between the zones just under the domains.
    let mut home: BTreeMap<usize, String> = BTreeMap::new();
    for zone in &top {
        if zone.inside.is_empty() {
            home.extend(zone.parts.iter().map(|&p| (p, zone.name.clone())));
        }
        for inner in &zone.inside {
            home.extend(inner.parts.iter().map(|&p| (p, format!("{}: {}", zone.name, inner.name))));
        }
    }
    let mut flows: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    for (k, net) in nets.iter().enumerate() {
        let reached: BTreeSet<&String> = net.iter().filter_map(|p| home.get(p)).collect();
        let reached: Vec<&String> = reached.into_iter().collect();
        for (i, a) in reached.iter().enumerate() {
            for b in &reached[i + 1..] {
                let entry = flows.entry(((*a).clone(), (*b).clone())).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += usize::from(all_nets[signal[k]].pair);
            }
        }
    }
    let mut flows: Vec<(String, String, usize, usize)> =
        flows.into_iter().map(|((a, b), (n, pairs))| (a, b, n, pairs / 2)).collect();
    flows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));

    // How far the parts stand from the ground their zone is given.
    let mut leaf_of: BTreeMap<usize, usize> = BTreeMap::new();
    let mut leaves: Vec<(Option<Pt>, Pt)> = Vec::new(); // (where its placed parts are, where it is given ground)
    fn told(
        work: &Work,
        board: &Board,
        nets: &[Vec<usize>],
        leaf_of: &mut BTreeMap<usize, usize>,
        leaves: &mut Vec<(Option<Pt>, Pt)>,
    ) -> Zone {
        let parts = &board.parts;
        let stands: Vec<Pt> = work.parts.iter().filter(|&&p| parts[p].placed).map(|&p| parts[p].at).collect();
        let bounds = (!stands.is_empty()).then(|| {
            let low = |k: usize| stands.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| stands.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            [low(0), low(1), high(0), high(1)]
        });
        let r = work.rect;
        let at_home = stands.iter().filter(|p| p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]).count();
        let among: BTreeSet<usize> = work.parts.iter().copied().collect();
        if work.inside.is_empty() {
            leaf_of.extend(work.parts.iter().map(|&p| (p, leaves.len())));
            leaves.push((median(&stands), middle(r)));
        }
        let mut names: Vec<String> = work.parts.iter().map(|&p| parts[p].name.clone()).collect();
        names.sort();
        let sheets: BTreeSet<String> = work
            .parts
            .iter()
            .map(|&p| if parts[p].sheet.is_empty() { "(on no sheet)".to_string() } else { parts[p].sheet.clone() })
            .collect();
        Zone {
            name: work.name.clone(),
            sheets: sheets.into_iter().collect(),
            parts: names,
            placed: stands.len(),
            bounds,
            area: work.parts.iter().map(|&p| parts[p].area).sum::<f64>().round(),
            within: nets.iter().filter(|net| net.iter().all(|p| among.contains(p))).count(),
            inside: work.inside.iter().map(|inner| told(inner, board, nets, leaf_of, leaves)).collect(),
            rect: r.map(|v| (v * 100.0).round() / 100.0),
            at_home: if stands.is_empty() {
                0.0
            } else {
                (at_home as f64 / stands.len() as f64 * 1000.0).round() / 1000.0
            },
            off: median(&stands).map(|m| (dist(m, middle(r)) * 10.0).round() / 10.0),
        }
    }
    let zones: Vec<Zone> = top.iter().map(|work| told(work, board, &nets, &mut leaf_of, &mut leaves)).collect();
    let (mut now_mm, mut then_mm) = (0.0, 0.0);
    for net in &nets {
        let reached: BTreeSet<usize> =
            net.iter().filter_map(|p| leaf_of.get(p).copied()).filter(|&l| leaves[l].0.is_some()).collect();
        if reached.len() < 2 {
            continue;
        }
        let span = |points: Vec<Pt>| {
            let low = |k: usize| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
            let high = |k: usize| points.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
            high(0) - low(0) + high(1) - low(1)
        };
        now_mm += span(reached.iter().filter_map(|&l| leaves[l].0).collect());
        then_mm += span(reached.iter().map(|&l| leaves[l].1).collect());
    }
    Ok(Floorplan {
        power,
        zones,
        flows,
        apart,
        now_mm: now_mm.round(),
        then_mm: then_mm.round(),
        regions,
        stranded: travel.left.iter().map(|&k| net_name(signal[k])).collect(),
        tall,
        travel,
        first,
        bridges,
        places,
        parts_now_mm: parts_now_mm.round(),
        parts_then_mm: parts_then_mm.round(),
        pads_first_mm: pads_first_mm.round(),
        pads_mm: pads_mm.round(),
        bypass,
        capacitors,
        junctions,
        pours,
        roads,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            (pour.net.as_str(), pour.ground.as_str(), &pour.layers),
            ("A_3V3", "A_GND", &vec!["In4".to_string()])
        );
        assert_eq!((pour.rects.len() >= 2, pour.parts, pour.pieces, pour.by_track.len()), (true, 5, 1, 0));
        assert_eq!((within(&found), found.travel.split), (Some(0), 0));
        // within a zone a bypass capacitor stands by the chip it is for: nearer it than the zone's other parts are
        let at = |name: &str| found.places.iter().find(|place| place.part == name).unwrap().at;
        assert!(
            dist(at("C4"), at("U2")) < 6.0 && dist(at("C1"), at("U1")) < dist(at("C1"), at("R1")) + 6.0,
            "{:?}",
            found.places
        );
        // the isolator has its place on the gap, and no ground of its own
        let isolator = found.places.iter().find(|place| place.part == "U3").unwrap();
        assert_eq!((isolator.rect, isolator.at), (None, found.bridges[0].at));
        assert_eq!(found.places.len(), 14);
        // every layer change planned where roads meet is told with the ground its via wants
        let square = floorplan(&square()).unwrap();
        // and every part has a place within the ground of its zone, none over another
        assert_eq!(square.places.len(), 4);
        let grounds: Vec<[f64; 4]> = square.places.iter().filter_map(|place| place.rect).collect();
        for (i, a) in grounds.iter().enumerate() {
            for b in &grounds[i + 1..] {
                assert!(
                    a[2].min(b[2]) - a[0].max(b[0]) < 1e-6 || a[3].min(b[3]) - a[1].max(b[1]) < 1e-6,
                    "{a:?} {b:?}"
                );
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
            nets: nets.iter().map(|n| Net { name: n.to_string(), supply: false, pair: false }).collect(),
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
            .map(|d| {
                (d.ground.as_str(), d.rails.iter().map(|r| (r.net.as_str(), r.parts.len(), r.capacitors)).collect())
            })
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
        assert!(
            found.roads[1..].iter().all(|road| !road.gap && road.level > 0 && road.width <= 2.0),
            "{:?}",
            found.roads
        );
        assert_eq!(found.travel.through, 1, "{:?}", found.travel);
        // and the isolator, on both grounds, stands on the gap: across its middle
        assert_eq!(top.parts, ["U3"]);
        let [bridge] = &found.bridges[..] else { panic!("{:?}", found.bridges) };
        assert_eq!((bridge.part.as_str(), bridge.on, bridge.reach), ("U3", Across::Gap, 0.0));
        assert_eq!(bridge.grounds, ["A_GND".to_string(), "FIELD_0V".to_string()]);
        assert_eq!(bridge.at[0], 24.34);
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
        assert_eq!((link.part.as_str(), link.on, link.at[0], link.reach), ("U3", Across::Link, 26.0, 2.0));
        // (the keep-out is no room: within the large board each lot has its share of what is left)
        // each domain has the whole of its board, and there is no road across the gap between boards
        assert_eq!((found.zones[0].rect, found.zones[1].rect), ([0.0, 0.0, 26.0, 10.0], [28.0, 0.0, 40.0, 10.0]));
        let within = |road: &Road| road.rect[2] <= 26.0 + 1e-6 || road.rect[0] >= 28.0 - 1e-6;
        assert!(found.roads.iter().all(|road| !road.gap && within(road)), "{:?}", found.roads);
    }

    /// Four chips in a square on one ground, each tied to the next round the square by eight nets.
    fn square() -> Board {
        let mut nets = vec![Net { name: "GND".to_string(), supply: false, pair: false }];
        let mut pads: Vec<Vec<Option<usize>>> = vec![vec![Some(0)]; 4];
        for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            for k in 0..8 {
                nets.push(Net { name: format!("N{a}{b}_{k}"), supply: false, pair: false });
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
}
