//! Filled zones ("pours"): what a board's zones join, where a net may be joined to its zone, how well each
//! pad is tied to it, and what of a run's copper hangs on a zone that the run itself cut away.
//!
//! The router itself knows nothing of zones: a [`Problem`](crate::Problem) has terminals that stand on one
//! piece of copper ([`Terminal::piece`](crate::Terminal::piece)) and ground that costs more
//! ([`Dear`](crate::Dear)). This module is what turns a board with filled zones into those, and what looks at
//! the board again afterwards. It works on plain geometry: the pieces of the zones as they were filled, as
//! polygons, and the pads; it does not fill a zone, which is the CAD program's to do, before a run and again
//! after it.
//!
//! One round with it goes like this:
//!
//! 1. [`survey`] a [`Layout`]: the groups of pads the zones join (a net whose pads are all in one group
//!    needs nothing), the pieces of zone that join each group, and every pad that a zone ties by thermal
//!    spokes, with its spokes counted.
//! 2. For each net still in several groups, [`reach`] gives the places each group may be reached at: these
//!    are the terminals of its entry, those of one group with the same `piece`. The pieces of zone, and the
//!    [`Thermal::ground`] of each pad, are the problem's dear ground.
//! 3. Route. [`parts`] gives what was laid as the pieces that can be taken back one by one, each with the
//!    points of a zone it ended on.
//! 4. Fill the zones again and survey again: a part that ended on a piece of zone the new tracks cut away
//!    ([`on_zone`] tells) is taken back; nets the new tracks parted are joined again; a pad left with too few spokes gets a straight
//!    track into its zone, ending on one of its [`tie_targets`] and kept if it [`ties`] the pad.
//!
//! Every one of these is also a question the `pcb-routing pours` command answers, as JSON: see [`Ask`].
//!
//! Two of them are of use without any zone: [`reach`] also chooses, for a group of several pads that tracks
//! already join, the pads nearest the rest of the net, and [`parts`] tells what a run laid apart into the
//! pieces that hang together.
//!
//! Lengths are in millimetres. A polygon is a list of rings, even-odd: its outline, then its holes. (KiCad
//! gives a filled zone as outlines with the holes cut into them; both forms are taken.)

mod ask;
mod joined;
mod parts;
mod reach;
mod shape;
mod spokes;

pub use ask::{answer, Answer, Ask, ReachNet, TiePad, TieTrack};
pub use parts::parts;
pub use reach::reach;
pub use spokes::{tie_targets, ties};

use crate::error::{invalid, Error};
use crate::model::{Pt, Track};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A polygon: its outline, then its holes (even-odd).
pub type Polygon = Vec<Vec<Pt>>;

/// A pad.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Pad {
    /// Its electrical net, if it has one.
    pub net: Option<String>,
    /// Where a track meets it.
    pub at: Pt,
    /// The copper layers it is on.
    pub layers: Vec<String>,
    /// Its copper on each of those layers, as polygons. Needed by [`survey`] only.
    #[serde(default)]
    pub shapes: HashMap<String, Vec<Polygon>>,
}

/// One piece of a filled zone: copper of a net that joins what touches it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Fill {
    /// Its electrical net.
    pub net: String,
    /// The layer it is on.
    pub layer: String,
    /// Its outline, then its holes.
    pub rings: Polygon,
    /// The zone it is a piece of: any number, the same for all pieces of one zone.
    #[serde(default)]
    pub zone: usize,
}

/// A via or a straight piece of track of a net that has zones.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Wire {
    /// Its electrical net.
    pub net: String,
    /// The layer of a track; absent for a via, which is on every layer.
    #[serde(default)]
    pub layer: Option<String>,
    /// The two ends of a track, or the one place of a via.
    pub ends: Vec<Pt>,
    /// The group of pads it is joined to, as an index into the groups of its net (see [`Layout::groups`]);
    /// absent for copper that reaches no pad. Such copper still counts where it joins two pieces of zone:
    /// wires that hang together (an end of one on the other on its layer, a via on a track) are one.
    #[serde(default)]
    pub group: Option<usize>,
}

/// A pad that a zone is to meet by thermal spokes, on one layer.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Spoked {
    /// The pad, as an index into [`Layout::pads`].
    pub pad: usize,
    /// The layer.
    pub layer: String,
    /// The zone (see [`Fill::zone`]).
    #[serde(default)]
    pub zone: usize,
    /// The line a spoke crosses: the pad's outline grown by half the gap, as a ring.
    pub line: Vec<Pt>,
    /// The gap the zone leaves round the pad.
    pub gap: f64,
    /// The clearance the zone keeps from copper of other nets.
    #[serde(default)]
    pub clearance: f64,
    /// The spokes asked of it.
    pub least: u32,
    /// The straight pieces of track that begin or end on the pad on this layer, each as its two ends.
    #[serde(default)]
    pub tracks: Vec<[Pt; 2]>,
    /// Those on other layers. They tie nothing to this layer's zone, but KiCad's check counts them, and with
    /// one of them counted no longer counts the spokes of a piece of zone that joins this pad alone.
    #[serde(default)]
    pub elsewhere: Vec<[Pt; 2]>,
}

/// What of a board the work on its zones needs. The zones must be filled.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// Every pad.
    pub pads: Vec<Pad>,
    /// The vias and tracks of the nets that have zones.
    #[serde(default)]
    pub wires: Vec<Wire>,
    /// Every piece of every filled zone.
    #[serde(default)]
    pub fills: Vec<Fill>,
    /// Per net: its groups of pads that tracks, vias and pads that touch already join, as indices into `pads`.
    #[serde(default)]
    pub groups: BTreeMap<String, Vec<Vec<usize>>>,
    /// The pads that zones are to meet by spokes.
    #[serde(default)]
    pub spoked: Vec<Spoked>,
}

/// A pad tied to a filled zone of its net by thermal spokes, on one layer.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Thermal {
    /// The zone's net.
    pub net: String,
    /// The pad, as an index into [`Layout::pads`].
    pub pad: usize,
    /// The layer.
    pub layer: String,
    /// The spokes asked of it.
    pub least: u32,
    /// Those it has: the fewer of what KiCad's check counts and of the same count without the tracks on
    /// other layers.
    pub spokes: u32,
    /// The ground where a track of another net would cut its spokes, as polygons: dear ground for a router.
    pub ground: Vec<Polygon>,
    /// For a pad with too few spokes: the line a spoke crosses ([`Spoked::line`]). Empty otherwise.
    pub outline: Vec<Pt>,
    /// For a pad with too few spokes: the pieces of the zone it is tied to, as indices into
    /// [`Layout::fills`], those that join more than this pad first. Empty otherwise.
    pub fill: Vec<usize>,
}

/// What [`survey`] finds.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Survey {
    /// The groups of [`Layout::groups`] with those that a filled zone joins made one.
    pub groups: BTreeMap<String, Vec<Vec<usize>>>,
    /// Per net that a zone joins a pad of: for each of its groups, in the order of `groups`, the pieces of
    /// zone that join it, as indices into [`Layout::fills`]. A piece that joins no pad is nobody's.
    pub pours: BTreeMap<String, Vec<Vec<usize>>>,
    /// Every pad a zone ties by spokes, in the order of [`Layout::spoked`]. A pad its zone covers without a
    /// gap, or does not reach, is not among them.
    pub thermals: Vec<Thermal>,
}

/// A place where a group of pads may be reached: a terminal for a router.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Place {
    /// Where.
    pub xy: Pt,
    /// The layers it may be reached on there.
    pub layers: Vec<String>,
}

/// A point of a zone that laid copper came to end on.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct End {
    /// The zone's net.
    pub net: String,
    /// The layer.
    pub layer: String,
    /// Where.
    pub xy: Pt,
}

/// Copper of one entry that hangs together.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Part {
    /// Its name: the entry's key if the entry's copper is all of a piece, else `KEY # n`.
    pub key: String,
    /// The key of the entry.
    pub of: String,
    /// The electrical net.
    pub net: String,
    /// Its tracks.
    pub tracks: Vec<Track>,
    /// Its vias.
    pub vias: Vec<Pt>,
    /// The points of a zone it ends on: it is joined to its net there only while the zone is.
    pub ends: Vec<End>,
}

/// What a board's filled zones join, and how each pad they meet by spokes is tied.
///
/// A piece of a zone joins the pads of its net whose copper on its layer it touches, and the vias and track
/// ends of its net that stand on it. Spokes are counted as KiCad's design rule check counts them: two
/// crossings of the zone's edge with the line half the gap from the pad are one spoke; those of a piece that
/// joins nothing but this pad count only if the pad has no others; and a track from by the pad onto the
/// zone counts as one, for a pad that has too few without. KiCad counts such a track on any layer; one on
/// another layer ties nothing to this zone, so a pad is given the fewer of the count with those and without.
pub fn survey(layout: &Layout) -> Result<Survey, Error> {
    layout.check()?;
    let pieces: Vec<shape::Piece> = layout.fills.iter().map(|fill| shape::Piece::new(&fill.rings)).collect();
    let touch = joined::touches(layout, &pieces);
    let (groups, pours) = joined::joined(layout, &touch);
    Ok(Survey { groups, pours, thermals: spokes::thermals(layout, &pieces, &touch) })
}

/// Whether each of some points is on a piece of zone of its net on its layer (`fills`: the pieces, as the
/// zones are now filled). A track that ended on a point of a zone hangs on nothing once the zone has drawn
/// back from it (see [`Part::ends`]).
pub fn on_zone(fills: &[Fill], points: &[End]) -> Vec<bool> {
    let pieces: Vec<Option<shape::Piece>> = fills
        .iter()
        .map(|fill| {
            points.iter().any(|p| p.net == fill.net && p.layer == fill.layer).then(|| shape::Piece::new(&fill.rings))
        })
        .collect();
    let stands = |point: &End| {
        let own = fills.iter().zip(&pieces).filter(|(fill, _)| fill.net == point.net && fill.layer == point.layer);
        own.flat_map(|(_, piece)| piece).any(|piece| piece.contains(point.xy))
    };
    points.iter().map(stands).collect()
}

impl Layout {
    fn check(&self) -> Result<(), Error> {
        let pad = |field: String, i: usize| match i < self.pads.len() {
            true => Ok(()),
            false => Err(invalid(field, format!("pad {i} of {}", self.pads.len()))),
        };
        for (net, groups) in &self.groups {
            for (g, group) in groups.iter().enumerate() {
                group.iter().try_for_each(|&i| pad(format!("groups[{net}][{g}]"), i))?;
            }
        }
        for (k, spoked) in self.spoked.iter().enumerate() {
            pad(format!("spoked[{k}].pad"), spoked.pad)?;
            if !(spoked.gap > 0.0 && spoked.gap.is_finite() && spoked.clearance.is_finite()) {
                return Err(invalid(format!("spoked[{k}].gap"), "the gap is more than 0, the clearance a number"));
            }
        }
        for (k, wire) in self.wires.iter().enumerate() {
            if wire.ends.is_empty() || wire.ends.len() > 2 {
                return Err(invalid(format!("wires[{k}].ends"), "one place for a via, two ends for a track"));
            }
            let most = self.groups.get(&wire.net).map_or(0, Vec::len);
            if wire.group.is_some_and(|group| group >= most) {
                return Err(invalid(format!("wires[{k}].group"), format!("names a group of {most}")));
            }
        }
        Ok(())
    }
}
