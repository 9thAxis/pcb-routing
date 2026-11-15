# Changelog

## 0.1.0 (unreleased)

First version as a library.

- `route(&Problem) -> Result<Solution, Error>` and `Router` (events per round, verification of the two-pass
  search against the one-pass search, one attempt alone). The command line is unchanged in its two commands,
  `route` and `masks`, and gains `--trace`, `--verify` and `--only`, and a third command, `pours` (below).
- A problem is checked before it is run (`Problem::validate`): sizes and costs that make sense, names that
  exist, a grid that fits `options.max_states`. Misspelt fields in a problem file are an error.
- `Solution::status` is an enum (written as `routed`, `conflicts`, `unroutable`, as before); an incomplete
  solution has no score (`null`).
- What can be laid is laid: a net with a terminal out of reach keeps its other connections, also without
  negotiation and for the nets kept for last; a pair that cannot be split or whose stubs do not all fit no
  longer ends the attempt; a net left out for want of room is tried again after the closing passes have moved
  the others. Incomplete attempts are ranked by how much is missing. A failure names both sides of the
  connection that could not be made.
- The fixed masks of the nets and the costs of the ground are drawn up page by page, only where a search
  goes, and nets with the same sizes and clearances hold one page wherever none of their own copper is near.
  No mask is drawn up for the whole board before the first net is laid.
- `options.exact_after` is `null` by default: no exact look-ahead tables. They cost more than they saved on
  every board measured (the largest of the hard set: 695 to 439 thousand million instructions and 3.4 to
  1.0 GB without them, the same result).
- A track ends at the exact place of a pad or hole only if the piece that leads there from the cell keeps
  its distances; else it ends on the cell. (A track wider than its pad could come too near the next pad.)
- A track that ends on a pad or hole off the grid is brought there by sliding its last corner along the
  stretch before it, so that it keeps its directions, instead of by a crooked stub shorter than the track
  is wide. What moves is tested off the grid against the fixed copper and the other nets; a stub is laid
  where the track may not move, and left out where it would come too near another net. On 111 public
  boards: turns of 90 degrees or more per 100 connections 75 to 11, segments shorter than twice their
  width 158 to 36.
- An entry that changes layer on a via of another entry of the same net holds that via too: the other entry
  could be laid again with its via elsewhere, and the result then had a layer change with no via.
- Buses: nets given the same `bus` name are drawn to run beside each other. A net of a bus pays
  `options.bus_pull` (0.2) more per unit length wherever it is not beside a laid net of its bus, in the lane
  its `bus_lane` gives it, so that the nets between two others find room; and `options.bus_cross` (6) more
  where it runs over or under one on another layer. The KiCad bridge names as a bus three or more nets with
  neighbouring pads on the same two parts, with lanes in the order of the pads (`--bus-pull`, 0 for none).
  On 199 pairs of bus neighbours on public boards: 63 to 72 % of a net's length beside its neighbour (the
  designers: 72 %), and 22 to 13 % of the pairs crossing (19 %).
- KiCad bridge: a capacitor with a pin of an IC of its net within 5 mm is joined to that pin by an entry of
  its own with five times the via penalty, and the rest of the net is joined without it (`--no-decoupling`
  for none). A capacitor counts if both its nets have three pads or more (not one in series with a signal).
  If a net with such links is not all laid, nothing of the run is kept and it is done again with that net
  whole. On 51 such capacitors on public boards: a via between capacitor and pin for 14 to 6 %
  (the designers: 6 %), for 1.4 % more track.
- KiCad bridge: zones are followed (`--no-pours` for the old way, where a net with a zone is routed as
  tracks). The zones are filled first, and pads that a filled zone joins count as joined; a net with a zone
  gets tracks only from the pads its zone does not reach, each to a pad or to a point of the zone, whichever
  comes cheapest. A track of another net pays double for its length across a zone and four times across
  the spokes of a pad (`--pour-cost`, `--spoke-cost`). After each run the zones are filled again: copper that
  ended on a piece of zone the other tracks cut away is taken back, pads parted from their zone are joined
  again, and where that cannot be done past the tracks that did it, the run is taken back and done again
  with those pads joined by a track. At the end the spokes of every pad are counted as KiCad's check
  counts them, and a pad with too few gets a straight track into its zone, which counts as one; a pad
  that no such track fits by is named. On the 119 public boards with their designers' zones put back: all
  complete with no new error in KiCad's check, 26,000 mm of track and 464 vias where ignoring the zones
  takes 33,000 mm and 388 (the vias: pads taken down to a zone on the other side).
- Zones, in the crate (`pcb_routing::pours`, and the command `pcb-routing pours`, which answers the same
  questions as JSON): from the pieces of the filled zones as polygons and the pads, which groups of pads
  the zones join (`survey`), the spokes of every pad the zones tie, counted as KiCad counts them, with the
  ground where a track would cut them; the places a net may be joined to its zone (`reach`); the copper of
  a run that hangs on a piece of zone (`parts`); and the straight track that ties a pad left with too few
  spokes (`tie_targets`, `ties`). The bridge did all of this in Python; it now reads the board, fills the
  zones and asks. On the largest of the public boards that takes the time spent outside the router's own
  runs from 12 s to under 2 s.
- Engine: terminals of an entry may name the piece of copper they stand on (`piece`). Those of one piece are
  joined already, and the entry is joined to the piece at whichever of them comes cheapest. The bridge
  gives every group of pads that copper already joins as one piece, with its nearest pads to choose from,
  where it gave the group's first pad.
- Engine: ground that costs more without being closed (`dear`): polygons with a factor on the length of
  track in them, and on a via as a track of its diameter; the net a piece belongs to pays nothing more
  there, and where pieces overlap a net pays the dearest that is not its own.
- KiCad bridge: round pads and arcs are given as polygons that hold the true shape (they were drawn inside
  it, up to 0.005 mm short), and the margin added to every clearance is 0.002 mm instead of the 0.02 mm that
  had been covering for that. A board laid out at its minimum clearance has room again.
- The result types (`Solution`, `NetResult`, `Track`, `Conflict`, `ConflictKind`, `Attempt`, `Stats`) and
  `Shape` are `#[non_exhaustive]`: fields and kinds can be added without breaking callers. The tuning fields
  of `Options` are documented as not stable.
- Matched pairs (`Problem::pairs`): a trunk split into two legs, joined to their pads by stubs.
