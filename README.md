# pcb-routing

A grid router for printed circuit boards that lays all the nets of a problem together and lets them settle
between themselves who goes where (negotiated congestion), instead of laying them one after the other in a
fixed order.

It knows nothing about any particular board or CAD program. A problem describes the fixed copper, the closed
ground and the nets with their rules; the answer is a solution with tracks and vias. Both are plain data that
read and write as JSON, so the router can be used as a Rust library or as a command-line program fed by an
exporter written in any language.

**Check every result against the real geometry before using it** (the design rule check of your CAD program,
or a check of your own). The router's picture of the board is the problem it was given.

## As a library

```toml
[dependencies]
pcb-routing = "0.1"
```

```rust
use pcb_routing::{route, Problem, Status};

let problem: Problem = serde_json::from_str(&std::fs::read_to_string("problem.json")?)?;
let solution = route(&problem)?;
assert_eq!(solution.status, Status::Routed);
for net in &solution.nets {
    println!("{}: {:.2} mm, {} vias", net.key, net.length_mm, net.vias.len());
}
```

`route` returns an error only when the problem cannot be run at all (a size or cost that makes no sense, a
name that does not exist, a grid larger than `options.max_states`). A problem that was run and could not be
completed comes back as a solution whose status is `conflicts` (the nets still too close are named, with the
place) or `unroutable` (a terminal cannot be reached even with nothing else in the way; the net and terminal
are named).

`Router` gives more control: a callback per round of negotiation, a self-check of the search, one attempt
alone. See `examples/route_file.rs` and the API documentation.

## As a program

    cargo install pcb-routing
    pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N] [--trace] [--verify] [--only ATTEMPT]
    pcb-routing masks PROBLEM.json --net KEY -o MASK.bin

`route` exits with status 0 if every net is laid and none are too close, 1 otherwise, 2 if the problem could
not be run. `--trace` prints one line per round and the time spent per kind of work. `--verify` runs every
search a second time without the first pass (see below) and reports any whose cost differs. `masks` writes the
router's picture of the board for one net, one byte per cell, for comparison with another tool's.

Try it on `examples/two_nets.json`.

## The problem

Every field is documented on the types in `src/model.rs`. In short:

* `window`, `step`, routing `layers` (at most seven);
* `copper` already on the board (shapes `seg`, `circle`, `poly`, with net and layers; `pad` for solder lands);
* `holes` already on the board; `regions` closed to tracks and/or vias;
* `rules`: hole spacing, clearance between new nets, a larger clearance between `quiet` nets and others;
* `nets`: key, electrical net, width, via, layers, clearance to existing copper, terminals, via penalty, layer
  costs, `yield` (how readily it gives way), `max_mm` and `weight` (for ranking attempts), `last`, `no_vias`;
* `pairs`: two nets laid side by side (see below);
* `options`: rounds, corner cost, number of attempts, threads, the largest grid accepted.

All lengths are in mm. A misspelt field is an error, not ignored.

## How it works

1. **Fixed masks** (`src/mask.rs`). Per net, per routing layer: the cells whose centre is too close to copper
   of another net, or inside closed ground, for a track of this net; and the cells closed to a via (a through
   via must clear copper on every layer, solder lands by its drill, other holes by the hole spacing).
   Distances are exact, to segments, circles and polygons.
2. **Search** (`src/engine/search.rs`; queue in `src/queue.rs`). A* over (direction of arrival, layer, cell),
   eight directions. A 45 degree corner costs `bend_mm`, a right angle three times that, sharper turns are not
   made. It runs in two passes:
   * **First pass**: the cost from every (layer, cell) to the goal at the costs of this moment (other nets'
     claims and standing costs included), without corners and so without directions. It runs backwards from
     the goal, steered towards the start by a fixed look-ahead (the cost past the fixed copper alone, computed
     once per net and terminal set and shared by all attempts; `src/engine/lookahead.rs`). It follows one of
     its cheapest paths forwards to learn what a real path costs with its corners, and stops once every state
     that could lie on a path that cheap has its cost (at most `sweep_margin` corners beyond the cheapest).
   * **Second pass**: the path with corners, looking ahead with the first pass's cost. It only widens by the
     cost of the corners, instead of by everything the other nets add.

   The result is the cheapest path (to within the queue's grid of 1/64 step per state); `two_pass: false`
   gives the one-pass search for comparison.
3. **Nets with several terminals** grow as a tree: from the copper laid so far to the nearest remaining
   terminal (`src/engine/lay.rs`).
4. **Claims** (`src/engine/claims.rs`). A laid net claims, for each size of track and via another net may
   have, the cells where the centre of such a track or via would be too close to it. A net is in conflict where
   its own centre line or via stands in a cell claimed by another.
5. **Negotiation** (`src/engine/negotiate.rs`). Round by round, the nets in conflict are laid again, one after
   the other. Ground claimed by others is not closed, only dearer: by a factor that grows each round, plus a
   standing cost left on cells where conflicts were. The rounds end when no net is in conflict. Two nets
   squeezed into a gap with room for one can stay there round after round while the settled nets are no longer
   moved; when the count of nets in conflict has not fallen for `stall_after` rounds the standing cost is
   spread over the whole width a track claims round the conflict, more each round, until one of them leaves
   and pushes a third net instead. After `patience` such rounds the attempt is given up.
6. **Closing passes**. Each net is laid once more with all the others fixed, and the new path is kept only if
   it is cheaper. Repeated until nothing improves.
7. **Pairs** (`src/engine/pairs.rs`). A pair takes part in the negotiation as one wide track, its trunk. Once
   the nets have settled the trunk is split into two legs a fixed pitch apart (mitred at corners), and each
   leg is joined to its own pad at either end by a short stub, laid with everything else fixed. A stub arrives
   on the cell nearest its leg's end that still keeps the rules' distance from the other leg. The four stubs
   are tried in every order until all fit.
8. **Nets marked `last`** are laid after the others have settled, with them fixed: for a path that must be
   shown to exist but may not push the other nets about.
9. **Attempts** (`src/router.rs`). `variants` runs differently ordered attempts side by side on separate cores
   and keeps the best complete one: lowest sum of weighted lengths, with each mm over a net's `max_mm` counted
   a hundredfold, plus one per via. The solution lists every attempt's lengths. The same problem always gives
   the same answer, whatever the thread count.

## Limits

* The grid carries part of the clearance: a `routed` result keeps the rules between new nets at cell centres,
  and a diagonal step can come nearer by about 0.002 mm at a 0.05 mm step.
* The legs of a pair are not tested against fixed copper. The trunk is, at its own width, which covers the
  legs on straight runs and 45 degree corners when it is at least `pitch + leg_width + 0.04` wide; the outside
  of a right-angle corner reaches a little further.
* Memory is about 400 bytes per (layer, cell) state for the first attempt (most of it look-ahead tables that
  all attempts share) and 200 for each further attempt running at once: 1.4 GB for one attempt and 14.5 GB for
  18 side by side on a grid of 3.9 million states. A problem beyond `options.max_states` (default 10 million)
  is refused.
* At most seven routing layers and 249 net entries per run.
* No length matching within a pair beyond what the common trunk gives, and no via positions shared out as a
  resource of their own.

## Evidence

Measured on 14 November 2026 on the static bus of a four-layer mixed-signal board (16 nets; 964,371 cells by
4 layers), against that board project's older sequential router and an independent geometric check.

| Proof | Result |
|---|---|
| Masks against the older router, cell by cell: 4 nets with different rules (plain logic, quiet-rule inner-only, 0.3 mm rail, outer keep-out), 964,371 cells, 4 layers and vias | 0 cells open here that the older router closes; 91 to 1,435 extra closed cells per layer, every one beside a cell the older router closes (exact distance here, against a polygon drawn round each shape there) |
| Single connections against the older router, each alone on the board state the older router saw: 29 connections, negotiation off, `bend_mm` 0, `own_hole_free` false | 29 of 29 within 2 %; worst 0.4 % on connections over 5 mm |
| Independent clearance check without the grid | 0 violations on 29 nets, 73 tracks, 45 vias against 4,725 fixed shapes; the control (one track moved 0.12 mm) is caught with 8 violations |
| Two-pass search against one-pass, every search of a full run on the bus (`--verify`) | 0 differ by more than 0.25 grid step |
| Known-answer cases (`tests/`) | Two passes against one pass on five walled connections; two nets sharing a gap; a third forced round; a wall crossed on another layer; a crossing that needs a layer change; a gap too narrow for two (reported as a conflict by name); a walled-in terminal (reported); a three-terminal tree; one corner on a slanted run; same answer on repeat and with 1 or 6 threads; a pair round a wall beside an ordinary net; and the regressions below |

On reading the second row: laying the 29 connections in sequence with this router and comparing each with the
older router's sequence does *not* agree (one line 91 % longer, others 5 % shorter): both routers pick among
equal-cost paths differently, and in a fixed order every later connection inherits that. That is the order
dependence this router exists to remove; it is why the comparison is made one connection at a time on an
identical board.

### Review before publication

An independent review of the code found the following, each now fixed and kept as a test in
`tests/regressions.rs`:

* A net could be reported unroutable when a path existed. The first pass took the cost of the path it had
  followed from its own table, accepting each step within a tolerance; over a line of 1,200 steps the
  tolerances added up to more than the difference between real paths, the bound came out below every real
  path, and the second pass cut them all off. The bound is now the sum of the steps actually taken.
* New vias of different nets kept only the copper clearance from each other, not the hole spacing.
* The legs of a pair could be put on the wrong sides when both pads lay on one side of the trunk, and its
  stubs assumed a 0.05 mm grid.
* A negative corner or layer cost, or a zero step, sent a search round for ever and the machine out of memory;
  more than seven layers or 249 nets, a stub without a terminal, or an attempt number out of range, panicked.
  All are refused by name before anything is allocated.
* Misspelt fields were ignored; duplicate keys were accepted.

## Measured behaviour and what was tried

Speed, one attempt on the bus, one core. The attempts are chaotic (a different order of equal-cost states gives
a different, equally valid, negotiation), so the number of rounds varies between rows; times are for what that
build did.

| Change | Time |
|---|---|
| No corner cost (about 1,000 corners on the bus) | 16 s |
| Corner cost with direction of arrival in the search state (about 190 corners) | 219 s |
| Look-ahead = true cost to the goal past the fixed copper (was straight-line distance) | 107 s |
| One cost table per net and round instead of four tables read per step; the nine directions of a cell side by side in memory | 76 s |
| Radix heap instead of the standard binary heap (the profile showed 57 % of the time in its `pop`) | 40 s |
| Step rules past the fixed copper as one byte per (layer, cell), corner costs as a table; both shared by all attempts | 40.9 s, same answer |
| Bucket queue (each entry was moved about 9 times inside the radix heap) | 30 % less processor time over 18 attempts |
| Two passes: a direction-free first pass at current costs as the look-ahead (each cell was being expanded in about 7 of its 9 directions) | 28 s |
| The same for later connections of a net with several terminals (look-ahead towards its copper: out to a radius for short hops, kept and reused otherwise) | 25 s |
| Search record 16 to 8 bytes, first-pass record with the step cost in it; margin from the corner-free path | 21 s |

Many attempts side by side on 18 cores (wall time): 18 attempts 41 s (was 199 s), 54 attempts 89 s (was about
430 s). Side by side each attempt takes about 1.8 times the processor time it takes alone: memory traffic, and
some of the cores being slower ones. Where the time goes (18 attempts): first pass 52 %, second pass 32 %,
look-ahead towards copper 11 %, cost tables 5 %.

Tried and not kept:

* Laying all nets of a round at once against the previous round's picture (order independent, and one net's
  time per round): does not settle on this bus; after 80 rounds five nets were still in conflict.
* Laying a batch of six side by side and re-laying those that collide with a batch-mate: settles, but nearly
  every net of the bus collides, so no time is gained.
* Accepting paths up to 20 % dearer for less search: 3 % faster, worse lines; with the two-pass search,
  slower (the second pass floods where the first pass's costs end).
* A net over its wanted length giving way less each round: with several tight wanted lengths the stiffened
  nets hold their ground and one line went round by 110 mm in all 18 attempts.
* Repairs (lay the net furthest over its wanted length on its shortest path, hold it, let the others settle
  again): three repairs tried on one attempt each made the sum of excesses worse, and the run took 400 s.
* On a stall, laying all nets again at a low price for claimed ground with the standing costs kept: the same
  two nets ended in the same gap both times (replaced by spreading the standing cost).
* A first-pass margin fixed at 1, 3, 8, 20, 50, 150 corners: 26, 26, 24, 22, 25, 33 s; the default is 20 as
  the cap.

Open, in order of expected gain: the first pass still covers about a quarter of the board per connection (a
look-ahead that knows corners would remove the need for a margin); attempts whose `last` nets cannot be laid
are found out only at the end; the bucket queue rescans its far list each time its window empties, which could
matter late in a long negotiation; via positions as a resource of their own; length matching.

## Sources

McMurchie and Ebeling, PathFinder (1995), for negotiated congestion; Chen et al., Dr.CU (TCAD 2020), for extra
search state and conflict-only rip-up; FreeRouting's keep-if-better closing pass; Boyne, Acorn (2025), for
clearance-wide claims and the order-independent variant tried above.

## Licence

Licensed under either of the Apache License, Version 2.0 (`LICENSE-APACHE`) or the MIT licence
(`LICENSE-MIT`), at your option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as
above, without any additional terms or conditions.
