# pcb-routing

A grid router for printed circuit boards that lays all the nets of a problem together and lets them settle
between themselves who goes where (negotiated congestion), instead of laying them one after the other in a
fixed order.

It knows nothing about any particular board. A problem file describes the fixed copper, the closed ground and
the nets with their rules; the answer is a solution file with tracks and vias. The board project exports the
problem, checks the answer independently and runs its own acceptance (KiCad DRC and the rest) before anything
is installed.

    cargo build --release
    target/release/pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N]
    target/release/pcb-routing masks PROBLEM.json --net KEY -o MASK.bin
    cargo test --release

`PCB_ROUTING_TRACE=1` prints one line per round and the time spent per kind of work.
`PCB_ROUTING_CHECK=1` runs every search a second time without the first pass (see below) and reports any
whose cost differs. `PCB_ROUTING_ONLY=v` runs attempt `v` of a problem alone.

## How it works

1. **Fixed masks** (`src/raster.rs`). Per net, per routing layer: the cells whose centre is too close to copper
   of another net, or inside closed ground, for a track of this net; and the cells closed to a via (a through
   via must clear copper on every layer, solder lands by its drill, other holes by the hole spacing).
   Distances are exact, to segments, circles and polygons.
2. **Search** (`src/router.rs`, `search`; queue in `src/queue.rs`). A* over (direction of arrival, layer, cell),
   eight directions. A 45 degree corner costs `bend_mm`, a right angle three times that, sharper turns are not
   made. It runs in two passes:
   * **First pass** (`sweep`): the cost from every (layer, cell) to the goal at the costs of this moment (other
     nets' claims and standing costs included), without corners and so without directions. It runs backwards
     from the goal, steered towards the start by a fixed look-ahead (the cost past the fixed copper alone,
     computed once per net and terminal set and shared by all attempts), and stops once every state that could
     lie on a path up to `sweep_margin` corners dearer than the cheapest has its cost.
   * **Second pass**: the path with corners, looking ahead with the first pass's cost. It only widens by the
     cost of the corners, instead of by everything the other nets add.

   The result is the cheapest path (to within the queue's 1/64 grid step); `two_pass: false` gives the one-pass
   search for comparison.
3. **Nets with several terminals** grow as a tree: from the copper laid so far to the nearest remaining terminal.
4. **Claims**. A laid net claims, for each size of track and via another net may have, the cells where the
   centre of such a track or via would be too close to it. A net is in conflict where its own centre line or
   via stands in a cell claimed by another.
5. **Negotiation** (`solve`). Round by round, the nets in conflict are laid again, one after the other. Ground
   claimed by others is not closed, only dearer: by a factor that grows each round, plus a standing cost left
   on cells where conflicts were. The rounds end when no net is in conflict. Two nets squeezed into a gap with
   room for one can stay there round after round while the settled nets are no longer moved; when the count of
   nets in conflict has not fallen for `stall_after` rounds the standing cost is spread over the whole width a
   track claims round the conflict, more each round, until one of them leaves and pushes a third net instead.
   After `patience` such rounds the attempt is given up.
6. **Closing passes**. Each net is laid once more with all the others fixed, and the new path is kept only if
   it is cheaper. Repeated until nothing improves.
7. **Attempts**. `variants` runs differently ordered attempts side by side on separate cores and keeps the best
   complete one: lowest sum of weighted lengths, with each mm over a net's `max_mm` counted a hundredfold, plus
   one per via. The solution lists every attempt's lengths. Nets marked `last` are laid after the others have
   settled, with them fixed. The
   same problem always gives the same answer, whatever the thread count.

A result is `routed`, `conflicts` (the nets still too close are named, with the place) or `unroutable` (a
terminal cannot be reached even with nothing else in the way; the net and terminal are named).

## Problem file

See `src/model.rs`; every field is documented there. In short: `window`, `step`, routing `layers`; `copper`
(shapes `seg`, `circle`, `poly` with net and layers, `pad` for solder lands); `holes`; `regions` (closed
ground, optionally per layer, for tracks and/or vias); `rules`; `nets` (key, electrical net, width, via,
layers, clearance table, terminals, via penalty, layer costs, `yield`, `max_mm`, own regions); `options`.

## Evidence (14 Nov 2026, the Sonata proving board, static bus of the acquisition core)

Board side scripts are in the board repository, `devkit/tools/`.

| Proof | Result |
|---|---|
| Masks against the board project's older router, cell by cell (`compare_engine_masks.py`), 4 nets with different rules (plain logic, quiet-rule inner-only, 0.3 mm rail, outer keep-out), 964,371 cells, 4 layers and vias | 0 cells open in the engine that the older router closes; 91 to 1,435 extra closed cells per layer, every one beside a cell the older router closes (exact distance against a polygon drawn round each shape) |
| Single connections against the older router, each alone on the board state the older router saw (`compare_engine_lengths.py`), 29 connections, negotiation off, `bend_mm` 0, `own_hole_free` false | 29 of 29 within 2 %; worst 0.4 % on connections over 5 mm (the reference lengths are printed to 0.1 mm) |
| Independent clearance check without the grid (`check_engine_solution.py`) | 0 violations on 29 nets, 73 tracks, 45 vias against 4,725 fixed shapes; the control (one track moved 0.12 mm) is caught with 8 violations |
| Two-pass search against one-pass, every search of a full run on the bus (`PCB_ROUTING_CHECK=1`, 376 searches) | 0 differ by more than 0.25 grid step (0.0125 mm of cost). The first run found 6, all on a four-terminal net: the fixed look-ahead charged a via where the net had a free hole of its own from an earlier connection; fixed at the cause (a net's own vias are starting points on every layer; other free holes are part of the look-ahead's identity) |
| Known-answer cases (`tests/negotiation.rs`, `src/queue.rs`) | 11 pass: two passes against one pass on five walled connections, two nets sharing a gap, a third forced round, a wall crossed on another layer, a crossing that needs a layer change, a gap too narrow for two (reported as a conflict by name), a walled-in terminal (reported), a three-terminal tree, one corner on a slanted run, same answer on repeat and with 1 or 6 threads |

Notes on reading the second row: laying the 29 connections in sequence with the engine and comparing each with
the older router's sequence does *not* agree (one line 91 % longer, others 5 % shorter): both routers pick
among equal-cost paths differently, and in a fixed order every later connection inherits that. That is the
order dependence the engine exists to remove; it is why the comparison is made one connection at a time on an
identical board.

## Measured behaviour and what was tried

Speed, one attempt on the bus (16 nets; 964,371 cells by 4 layers by 9 directions of arrival), one core. The
attempts are chaotic (a different order of equal-cost states gives a different, equally valid, negotiation),
so the number of rounds varies between rows; times are for what that build did.

| Change | Time |
|---|---|
| No corner cost (about 1,000 corners on the bus) | 16 s |
| Corner cost with direction of arrival in the search state (about 190 corners) | 219 s |
| Look-ahead = true cost to the goal past the fixed copper (was straight-line distance) | 107 s |
| One cost table per net and round instead of four tables read per step; the nine directions of a cell side by side in memory | 76 s |
| Radix heap instead of the standard binary heap (the profile showed 57 % of the time in its `pop`) | 40 s (45.8 s on the 16-net problem used below) |
| Step rules past the fixed copper as one byte per (layer, cell), corner costs as a table; both shared by all attempts | 40.9 s, same answer |
| Bucket queue (each entry was moved about 9 times inside the radix heap) | 30 % less processor time over 18 attempts |
| Two passes: a direction-free first pass at current costs as the look-ahead (each cell was being expanded in about 7 of its 9 directions) | 28 s |
| The same for later connections of a net with several terminals (look-ahead towards its copper: out to a radius for short hops, kept and reused otherwise) | 25 s |
| Search record 16 to 8 bytes, first-pass record with the step cost in it; margin from the corner-free path | 21 s |

Many attempts side by side on 18 cores (wall time):

| | Before | Now |
|---|---|---|
| 18 attempts, 16-net bus | 199 s (set by one attempt stuck for 80 rounds) | 41 s, all 18 complete |
| 54 attempts, bus with two lines laid last | about 430 s | 89 s |

Side by side each attempt still takes about 1.8 times the processor time it takes alone (was about 3): memory
traffic, and some of the cores being slower ones. Where the time goes now (18 attempts): first pass 52 %,
second pass 32 %, look-ahead towards copper 11 %, cost tables 5 %.

Tried and not kept as defaults:

* Laying all nets of a round at once against the previous round's picture (order independent, and one net's
  time per round): does not settle on this bus; after 80 rounds five nets were still in conflict.
* Laying a batch of six side by side and re-laying those that collide with a batch-mate: settles, but nearly
  every net of the bus collides, so no time is gained (removed).
* `haste` 1.2 (paths up to 20 % dearer for less search): 3 % faster, worse lines; default 1.
* `stiffen` 0.7 (a net over its wanted length gives way less each round): with several tight wanted lengths
  the stiffened nets hold their ground and an SPI line went round by 110 mm in all 18 attempts; default 1 (off).
* `repairs` (lay the net furthest over its wanted length on its shortest path, hold it, let the others settle
  again): three repairs tried on one attempt each made the sum of excesses worse (123 mm to 147 .. 192 mm) and
  the run took 400 s; default 0.

* `haste` again, with the two-pass search and a small margin for the first pass: slower (the second pass
  floods where the first pass's costs end; 1.02 took 103 s against 61 s for 18 attempts).
* On a stall, laying all nets again at a low price for claimed ground with the standing costs kept: the same
  two nets ended in the same gap both times it was tried (replaced by spreading the standing cost, above).
* A first-pass margin fixed at 1, 3, 8, 20, 50, 150 corners: 26, 26, 24, 22, 25, 33 s; the default is 20 as the
  cap, less where the corner-free path has fewer corners (it rarely has).

What the attempts showed about the bus itself: in each of the first 54, exactly one line of the group that
passes east of ADC A takes a way round of about 125 mm; which one depends on the attempt. The ranking (`weight`)
chooses which: with the SPI lines weighted highest it is a line that never changes during a capture. With 216
attempts (390 s) one was found with every GPIO and SPI line in target and the detour on the enable line, shorter
at 122 mm; of the 57 complete attempts none has every line within its wanted length.

Open, in order of expected gain: the first pass still covers about a quarter of the board per connection (the
margin of 20 corners is wide on a long line; a look-ahead that knows corners would remove the need for it);
matched pairs and length matching (wanted next for analog input pairs); via positions as a resource of their
own; attempts whose `last` nets cannot be laid are found out only at the end (41 of 54 on the bus). Sources
that shaped the design: McMurchie and Ebeling, PathFinder (1995); Chen et al., Dr.CU (TCAD 2020) for extra
search state and conflict-only rip-up; FreeRouting's keep-if-better closing pass; Boyne, Acorn (2025) for
clearance-wide claims and the order-independent variant tried above.
