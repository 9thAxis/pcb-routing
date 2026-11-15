# Changelog

## 0.1.0 (unreleased)

First version as a library.

- `route(&Problem) -> Result<Solution, Error>` and `Router` (events per round, verification of the two-pass
  search against the one-pass search, one attempt alone). The command line is unchanged in its two commands,
  `route` and `masks`, and gains `--trace`, `--verify` and `--only`.
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
- KiCad bridge: round pads and arcs are given as polygons that hold the true shape (they were drawn inside
  it, up to 0.005 mm short), and the margin added to every clearance is 0.002 mm instead of the 0.02 mm that
  had been covering for that. A board laid out at its minimum clearance has room again.
- The result types (`Solution`, `NetResult`, `Track`, `Conflict`, `ConflictKind`, `Attempt`, `Stats`) and
  `Shape` are `#[non_exhaustive]`: fields and kinds can be added without breaking callers. The tuning fields
  of `Options` are documented as not stable.
- Matched pairs (`Problem::pairs`): a trunk split into two legs, joined to their pads by stubs.
