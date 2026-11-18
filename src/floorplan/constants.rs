//! What the floorplan counts and weighs by.

/// Capacitors from a net straight to a ground that make it a supply.
pub(super) const BYPASSED: usize = 2;
/// What a fixed part pulls its zone with, as so many nets.
pub(super) const ANCHOR: f64 = 4.0;
/// How many signal tracks a supply counts for in a road: it is laid wide.
pub(super) const RAIL_LANES: f64 = 4.0;
/// The most of a board's room that the parts of its domains may take: the rest is for roads and for routing.
pub(super) const BOARD_FULL: f64 = 0.6;
/// And the most they may take where no parting of the domains leaves every board under BOARD_FULL.
pub(super) const BOARD_MOST: f64 = 0.85;
/// What a way under the parts of a zone that is not its end counts for, as so many times its length.
pub(super) const THROUGH: f64 = 2.0;
/// What a layer change counts for when a way is chosen, in mm of track.
pub(super) const VIA_MM: f64 = 10.0;
/// How far along a road a connection may run on the layer of its parts, without a layer change, in mm.
pub(super) const LOCAL_MM: f64 = 5.0;
/// What a mm between a part and the point it is to stand near counts for, as so many mm of track.
pub(super) const NEAR: f64 = 5.0;
/// What disturbs is to be this far from what is disturbed, in mm; and what a mm less than that between two
/// zones counts for when layings out are weighed, as so many mm of track.
pub(super) const QUIET_MM: f64 = 15.0;
pub(super) const NOISE: f64 = 10.0;
/// How far down each zone of a cut is tried in the other lot.
pub(super) const MOVE_LEVELS: usize = 1;
/// A ground is cut across its shorter side only if that side is this much of the longer.
pub(super) const SQUARISH: f64 = 0.4;
/// How far down the cuts are tried across the shorter side of their ground.
pub(super) const CROSS_LEVELS: usize = 3;
/// What a part too tall for the ground its zone is given counts for when layings out are weighed, in mm.
pub(super) const TALL_MM: f64 = 500.0;
/// What a track more than a road holds counts for when layings out are weighed, in mm; and a net given no way.
pub(super) const OVER_MM: f64 = 50.0;
pub(super) const STRANDED_MM: f64 = 200.0;
/// What ties the smallest zones under the pour of one supply to each other, as so many nets for each.
pub(super) const POURED: f64 = 8.0;
/// And what ties them when zones are gathered into larger ones.
pub(super) const GATHERED: f64 = 4.0;
/// What ties the groups drawn on one sheet to each other when zones are gathered, as so many nets for each:
/// the board is laid out with each of these in turn, and the plan that comes out best is kept. How much the
/// drawing says of what belongs together differs from board to board, and a plan is sensitive to it.
pub(super) const SHEETED: [f64; 3] = [0.0, 2.0, 4.0];
/// What ties a capacitor from a supply to a ground to a chip of its sheet on that supply when the sheet's
/// parts are grouped, as so much of a net: enough that it is of the chip's group.
pub(super) const BESIDE_CHIP: f64 = 0.25;
/// What ties a capacitor from a supply to the ground to the chip it stands by, as so many nets.
pub(super) const BYPASS: usize = 3;
/// What a mm of parts more than their gap is long counts for when layings out are weighed, in mm.
pub(super) const CRAMPED_MM: f64 = 20.0;
/// Two grounds no further apart than this many gaps, corner to corner, are neighbours.
pub(super) const CORNER: f64 = 1.5;
/// What is left between two parts side by side on a gap, in mm.
pub(super) const BESIDE: f64 = 1.0;
/// How often the roads asked for too much are widened, and how many cuts are tried the other way round.
pub(super) const WIDENINGS: usize = 8;
pub(super) const TURNINGS: usize = 400;
/// The most of a piece of the board that the road across it takes.
pub(super) const ROAD_MOST: f64 = 0.2;

/// A capacitor is by the pad it is for if its own pad on that supply is no further from it than this, in mm.
pub(super) const BESIDE_PIN: f64 = 3.0;

/// A net on which a part may stand for the one chip or connector there has no more parts than this.
pub(super) const SERVED: usize = 6;
/// A chain is followed through no more parts in series than this; and doubles back if the way from pad to
/// pad through its parts is more than BACK times as long as straight (and what its parts may stand off the
/// pads they are by more: see `chains`).
pub(super) const CHAIN_MOST: usize = 6;
pub(super) const BACK: f64 = 1.5;
/// A loop is tight if the box of its pads is no more than this many times the bodies of its parts.
pub(super) const LOOP_MOST: f64 = 2.0;
/// A net with an inductor and a chip on it is a switch node if it has no more parts than this.
pub(super) const SWITCHED: usize = 5;
/// How often the small parts are gone over to shorten their nets, and how far from where a part wants to be
/// room is looked for, in mm.
pub(super) const REFINES: usize = 4;
pub(super) const REFINE_MM: f64 = 8.0;
/// A part joined to no more parts than this is tried by each of them, and at their mean.
pub(super) const FEW: usize = 4;
/// What the nets of a part may grow by, in mm, when it is moved to the one pad it is there for.
pub(super) const OWN_PAD_MM: f64 = 1.0;
/// A sheet is looked at for repeats if it has this many parts; and its parts are coloured this many times.
pub(super) const REPEAT_LEAST: usize = 3;
/// Copies stand side by side if their pads are then no more than this many times as far from what they join
/// as with each copy by itself.
pub(super) const ROW_WORTH: f64 = 1.25;
/// What is left between two copies of a sheet that stand side by side, in mm.
pub(super) const BETWEEN_COPIES: f64 = 1.0;
/// How far from its middle a sheet that is laid out by itself may reach, in mm.
pub(super) const UNIT_REACH: f64 = 40.0;
pub(super) const COLOURINGS: usize = 4;
/// A block is first looked for room no further than this from where it is wanted, in mm; and of the ways it
/// may be turned, those that come within AS_NEAR of the nearest are weighed by what its pads join.
pub(super) const BLOCK_NEAR: f64 = 25.0;
pub(super) const AS_NEAR: f64 = 2.0;
/// How far round a chip its block may reach, in mm; and how far from its pad a part of the block may stand.
pub(super) const BLOCK_REACH: f64 = 8.0;
/// A body of this many mm2 or more is settled before the others, over the whole ground of its domain.
pub(super) const LARGE: f64 = 64.0;
/// A part with holes stands on no road down to this level (see [`Road::level`](super::Road::level)).
pub(super) const MAIN: usize = 3;
/// How fine the ground of a zone is looked at when its parts are settled on it, in mm.
pub(super) const SETTLE: f64 = 0.25;

/// How much of the ground its parts are laid out on the parts of one of the smallest zones fill: a zone has its
/// share of the whole board, and on a board with room to spare its parts are kept together within it, towards
/// what they are joined to, and not spread over all of it.
pub(super) const PACKED: f64 = 0.5;
