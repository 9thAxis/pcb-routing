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
//! The same problem always gives the same answer, whatever the thread count. A `routed` result has no two new
//! nets closer than the rules on the router's grid; the grid carries about 0.02 mm of the clearance at a
//! 0.05 mm step. **Check every result against the real geometry before using it** (the design rule check of
//! your CAD program, or a check of your own): the router's picture of the board is the problem file, and the
//! legs of a [`Pair`] in particular are not tested against fixed copper.
//!
//! # How it works
//!
//! See the `README` of the repository for the method, the evidence it was accepted on, and measurements.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod engine;
mod error;
mod geom;
mod grid;
pub mod mask;
pub mod model;
mod queue;
mod router;
mod validate;

pub use error::Error;
pub use grid::Grid;
pub use model::{
    Attempt, Clearance, Conflict, ConflictKind, Copper, Hole, Net, NetResult, Options, Pair, Problem, Pt, Region,
    Rules, Shape, Solution, Stats, Status, Terminal, Track,
};
pub use router::{route, Event, Router};
