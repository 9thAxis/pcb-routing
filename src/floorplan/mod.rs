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
//!
//! The work is done in stages, each a function with what it reads and what it leaves: what is known of the
//! board (`known`), its power (`power`), the zone tree (`zones`), the supplies on the plane layers (`planes`),
//! the boards of the box (`boards`), the laying out and the search for the best (`layout`, which gives ground
//! by `ground` and ways by `ways`), a place for every part (`place`, with stages of its own), and what is
//! told of the layout (`report`). `plan` runs them in turn; `ties` and `settle` are what they share.

mod boards;
mod constants;
mod ground;
mod known;
mod layout;
mod place;
mod plan;
mod planes;
mod power;
mod report;
mod settle;
mod ties;
mod types;
mod ways;
mod zones;

#[cfg(test)]
mod tests;

pub use plan::floorplan;
pub use power::is_ground;
pub use settle::farads;
pub use ties::{communities, gathered};
pub use types::*;
