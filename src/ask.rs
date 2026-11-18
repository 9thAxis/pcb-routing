//! The questions a front end puts to the crate, as JSON: what the `pcb-routing ask` command takes and answers.
//!
//! They are the functions of [`pours`](crate::pours) (zones) and [`pairs`](crate::pairs) (differential pairs), each
//! for several nets, pads or tracks at once, and [`floorplan`](crate::floorplan) (where the parts belong).

use crate::error::{invalid, Error};
use crate::model::{Net, NetResult, Pt};
use crate::pairs::{plan, Plan};
use crate::pours::{on_zone, parts, reach, survey, tie_targets, ties, End, Fill, Layout, Pad, Part, Place, Survey};
use serde::{Deserialize, Serialize};

/// A question: JSON with the kind of question in the field `ask`, answered with the JSON of an [`Answer`].
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "ask", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ask {
    /// [`survey`]: answered with a [`Survey`].
    Survey(Layout),
    /// [`reach`], for several nets at once: answered with the places of each group of each net.
    Reach {
        /// The pads (their places and layers).
        pads: Vec<Pad>,
        /// The pieces of zone the nets name.
        #[serde(default)]
        fills: Vec<Fill>,
        /// The routing layers.
        layers: Vec<String>,
        /// The grid step, if points of a zone are to be on the grid.
        #[serde(default)]
        step: Option<f64>,
        /// The nets.
        nets: Vec<ReachNet>,
    },
    /// [`parts`]: answered with the parts.
    Parts {
        /// The pads (their nets, places and layers).
        pads: Vec<Pad>,
        /// The entries of the problem that was run.
        entries: Vec<Net>,
        /// What the run laid: the nets of its solution.
        laid: Vec<NetResult>,
    },
    /// [`on_zone`]: answered with true or false for each point.
    OnZone {
        /// The pieces of zone, as the zones are now filled.
        fills: Vec<Fill>,
        /// The points.
        points: Vec<End>,
    },
    /// [`plan`] of several pairs at once: answered with the plan of each.
    Pairs {
        /// The routing layers.
        layers: Vec<String>,
        /// The grid step.
        step: f64,
        /// The pairs.
        pairs: Vec<PairNets>,
    },
    /// [`tie_targets`], for several pads at once: answered with the points for each.
    TieTargets {
        /// The pieces of zone the pads name.
        fills: Vec<Fill>,
        /// The pads.
        pads: Vec<TiePad>,
    },
    /// [`floorplan`](crate::floorplan::floorplan): answered with the floorplan.
    Floorplan(Box<crate::floorplan::Board>),
    /// [`ties`], for several tracks at once: answered with true or false for each.
    Ties {
        /// The pieces of zone the tracks name.
        fills: Vec<Fill>,
        /// The tracks, each with its pad.
        tracks: Vec<TieTrack>,
    },
}

/// One net of a question [`Ask::Reach`].
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ReachNet {
    /// Its groups of joined pads, as indices into the pads.
    pub groups: Vec<Vec<usize>>,
    /// For each group, in the same order, the pieces of zone that join it, as indices into the fills.
    /// Groups it does not reach have none.
    #[serde(default)]
    pub pours: Vec<Vec<usize>>,
    /// How far within a zone a track is to end: half its width and a little.
    pub inset: f64,
}

/// The two nets of a question [`Ask::Pairs`].
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct PairNets {
    /// The pads of the first net (their places and layers).
    pub a: Vec<Pad>,
    /// The pads of the second.
    pub b: Vec<Pad>,
    /// The distance between the legs, centre to centre.
    pub pitch: f64,
}

/// One pad with too few spokes, of a question [`Ask::TieTargets`].
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TiePad {
    /// The pad's [`Thermal::outline`](crate::pours::Thermal::outline).
    pub outline: Vec<Pt>,
    /// The pieces of zone it is tied to ([`Thermal::fill`](crate::pours::Thermal::fill)), as indices into the
    /// question's fills.
    pub fill: Vec<usize>,
    /// The pad's middle.
    pub at: Pt,
    /// How far within the zone the track is to end.
    pub inset: f64,
    /// Points not to give again.
    #[serde(default)]
    pub tried: Vec<Pt>,
}

/// One track laid to tie a pad, of a question [`Ask::Ties`].
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TieTrack {
    /// The pad's [`Thermal::outline`](crate::pours::Thermal::outline).
    pub outline: Vec<Pt>,
    /// The pieces of zone it is tied to, as indices into the question's fills.
    pub fill: Vec<usize>,
    /// The track's points.
    pub points: Vec<Pt>,
}

/// The answer to an [`Ask`]. In JSON it is the answer alone, without a name for its kind.
#[derive(Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum Answer {
    /// To [`Ask::Survey`].
    Survey(Survey),
    /// To [`Ask::Reach`]: per net, per group, its places.
    Reach(Vec<Vec<Vec<Place>>>),
    /// To [`Ask::Parts`].
    Parts(Vec<Part>),
    /// To [`Ask::OnZone`]: per point, whether it is on a zone of its net.
    OnZone(Vec<bool>),
    /// To [`Ask::Pairs`]: per pair, its plan.
    Pairs(Vec<Plan>),
    /// To [`Ask::TieTargets`]: per pad, the points.
    TieTargets(Vec<Vec<Pt>>),
    /// To [`Ask::Ties`]: per track, whether it ties its pad.
    Ties(Vec<bool>),
    /// To [`Ask::Floorplan`].
    Floorplan(Box<crate::floorplan::Floorplan>),
}

/// The pieces of zone that indices name, as polygons; an error names `field` if one of them names none.
fn pieces<'a>(fills: &'a [Fill], named: &[usize], field: String) -> Result<Vec<&'a [Vec<Pt>]>, Error> {
    let piece = |&f: &usize| fills.get(f).map(|fill| &fill.rings[..]);
    named
        .iter()
        .map(piece)
        .collect::<Option<_>>()
        .ok_or_else(|| invalid(field, format!("names a fill of {}", fills.len())))
}

/// An error with the place in the question where it arose before its field.
fn within(place: String, error: Error) -> Error {
    match error {
        Error::Invalid { field, reason } => invalid(format!("{place}.{field}"), reason),
        other => other,
    }
}

/// Answer a question. An error if the question names a pad or a piece that is not there, or gives a
/// distance that is none.
pub fn answer(ask: &Ask) -> Result<Answer, Error> {
    Ok(match ask {
        Ask::Survey(layout) => Answer::Survey(survey(layout)?),
        Ask::Reach { pads, fills, layers, step, nets } => {
            use rayon::prelude::*;
            let places = |(n, net): (usize, &ReachNet)| {
                let found = reach(pads, fills, &net.groups, &net.pours, layers, net.inset, *step);
                found.map_err(|error| within(format!("nets[{n}]"), error))
            };
            Answer::Reach(nets.par_iter().enumerate().map(places).collect::<Result<_, _>>()?)
        }
        Ask::Parts { pads, entries, laid } => Answer::Parts(parts(pads, entries, laid)),
        Ask::OnZone { fills, points } => Answer::OnZone(on_zone(fills, points)),
        Ask::Pairs { layers, step, pairs } => {
            let planned = |(k, pair): (usize, &PairNets)| {
                plan(&pair.a, &pair.b, pair.pitch, layers, *step).map_err(|error| within(format!("pairs[{k}]"), error))
            };
            Answer::Pairs(pairs.iter().enumerate().map(planned).collect::<Result<_, _>>()?)
        }
        Ask::Floorplan(board) => Answer::Floorplan(Box::new(crate::floorplan::floorplan(board)?)),
        Ask::TieTargets { fills, pads } => {
            let targets = |(k, pad): (usize, &TiePad)| {
                let fill = pieces(fills, &pad.fill, format!("pads[{k}].fill"))?;
                let found = tie_targets(&pad.outline, &fill, pad.at, pad.inset, &pad.tried);
                found.map_err(|error| within(format!("pads[{k}]"), error))
            };
            Answer::TieTargets(pads.iter().enumerate().map(targets).collect::<Result<_, _>>()?)
        }
        Ask::Ties { fills, tracks } => {
            let tied = |(k, track): (usize, &TieTrack)| {
                Ok(ties(&track.outline, &pieces(fills, &track.fill, format!("tracks[{k}].fill"))?, &track.points))
            };
            Answer::Ties(tracks.iter().enumerate().map(tied).collect::<Result<_, Error>>()?)
        }
    })
}
