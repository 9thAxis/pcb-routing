//! A grid router for printed circuit boards that lays all the nets of a problem together and lets them settle
//! between themselves who goes where (negotiated congestion), instead of laying them one after the other in a
//! fixed order.
//!
//! The router knows nothing about any particular board or CAD program. A [`Problem`] describes the fixed
//! copper, the closed ground and the nets with their rules; the answer is a [`Solution`] with tracks and vias.
//! Both are plain data that read and write as JSON.
//!
//! ```
//! use pcb_routing::{route, Problem, Status};
//!
//! // Two nets that must pass each other on one layer.
//! let problem: Problem = serde_json::from_str(r#"{
//!     "window": [0, 0, 10, 6], "step": 0.05, "layers": ["F"],
//!     "nets": [
//!         {"key": "a", "net": "A", "width": 0.15, "via": [0.45, 0.2], "layers": ["F"],
//!          "clearance": {"default": 0.17},
//!          "terminals": [{"xy": [1, 3], "layers": ["F"]}, {"xy": [9, 3], "layers": ["F"]}]},
//!         {"key": "b", "net": "B", "width": 0.15, "via": [0.45, 0.2], "layers": ["F"],
//!          "clearance": {"default": 0.17},
//!          "terminals": [{"xy": [2, 3.1], "layers": ["F"]}, {"xy": [8, 3.1], "layers": ["F"]}]}
//!     ]
//! }"#)?;
//! let solution = route(&problem)?;
//! assert_eq!(solution.status, Status::Routed);
//! for net in &solution.nets {
//!     println!("{}: {:.2} mm, {} vias", net.key, net.length_mm, net.vias.len());
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What it guarantees, and what it does not
//!
//! The same problem always gives the same answer, whatever the thread count (with `options.exact_after` set
//! and several attempts, paths of equal cost can be chosen differently from run to run). A `routed` result
//! has no two new nets closer than the rules at the centres of the router's cells; between them a slanted
//! step can come nearer by about 0.002 mm at a 0.05 mm step. **Check every result against the real geometry
//! before using it** (the design rule check of your CAD program, or a check of your own): the router's
//! picture of the board is the problem file, and the legs of a [`Pair`] are tested against fixed copper only
//! as the trunk they are split from.
//!
//! # For a front end
//!
//! What stands between a board in a CAD program and a [`Problem`] is mostly reading. Two things are not, and
//! are here as plain geometry, with no CAD program in them:
//!
//! - [`pours`]: filled copper zones. Which pads the zones already join, where a net may be joined to its
//!   zone, how many thermal spokes each pad is left with, and what of a run's copper hung on a piece of zone
//!   that the run cut away.
//! - [`pairs`]: differential pairs. Which pads of two nets face each other, and between which of them a pair
//!   is to be laid as one.
//!
//! [`ask`] has the same as questions and answers in JSON, for a front end in another language: the
//! `pcb-routing ask` command answers them. The KiCad bridge in the repository works that way.
//!
//! # How it works
//!
//! See [how it works](https://github.com/9thAxis/pcb-routing/blob/main/docs/how-it-works.md) for the method,
//! and [the benchmarks](https://github.com/9thAxis/pcb-routing/blob/main/docs/benchmarks.md) for measurements.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ask;
pub mod board;
mod engine;
mod error;
mod geom;
mod grid;
pub mod mask;
pub mod model;
pub mod pairs;
pub mod pours;
mod queue;
mod router;
mod validate;

pub use error::Error;
pub use grid::Grid;
pub use model::{
    Attempt, Clearance, Conflict, ConflictKind, Copper, Dear, Hole, Net, NetResult, Options, Pair, Problem, Pt, Region,
    Rules, Shape, Solution, Stats, Status, Terminal, Track,
};
pub use router::{route, Event, Router};
