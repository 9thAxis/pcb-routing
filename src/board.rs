//! What a front end tells of a board, and is told back, in more than one of its questions.

use crate::model::Pt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Its copper on each of those layers, as polygons. Needed by [`survey`](crate::pours::survey) only.
    #[serde(default)]
    pub shapes: HashMap<String, Vec<Polygon>>,
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
