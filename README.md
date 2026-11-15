# pcb-routing

An autorouter for printed circuit boards, as a Rust library and a command-line program, with a bridge that
routes a KiCad board in place.

It lays all the nets of a problem together and lets them settle between themselves who goes where
(negotiated congestion), instead of laying them one after the other in a fixed order. The result does not
depend on the order the nets are given in, and a net that would block three others gives way.

**Check every result with the design rule check of your CAD program before using it.** The router's picture
of the board is the problem it was given.

Status: 0.1, not yet on crates.io. The interfaces may still change.

## Routing a KiCad board

    git clone https://github.com/9thAxis/pcb-routing && cd pcb-routing
    cargo build --release
    PCB_ROUTING=target/release/pcb-routing \
      /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 \
      tools/kicad_route.py BOARD.kicad_pcb -o ROUTED.kicad_pcb

The bridge ([`tools/kicad_route.py`](https://github.com/9thAxis/pcb-routing/blob/main/tools/kicad_route.py)) is in the
repository, not in the crate on crates.io. It runs under the Python that ships with KiCad (9 or 10; the path
above is the macOS one). It reads pads, tracks, vias, holes, the outline, rule areas and the net classes, routes every
net whose pads are not yet joined, writes the tracks and vias back and refills the zones. What is already
routed is kept and routed around, and what a filled zone joins is left to the zone (see below). `--nets A,B` routes only those nets; `--layers F.Cu,B.Cu` restricts the
layers; `--coarse-first` trades a little quality for about three times the speed. `--help` lists the rest.

## How it compares

### Results on public boards

The boards are those of the PCBWorld benchmark (LG AI Research, arXiv:2607.05915; real open-source KiCad
boards from PCBench with their tracks removed). A board counts as a clean pass when every connection is made
and KiCad's design rule check reports no error that the unrouted board did not have.

| Set | Boards | Nets per board | Clean pass | Connections made | Time per board |
|---|---|---|---|---|---|
| D3-A (small) | 99 | 2 to 13 | 99 of 99 | 1,301 of 1,301 | 0.6 s |
| D3-B (medium) | 10 | 5 to 42 | 10 of 10 | 445 of 445 | 2.5 s |
| D3-C (large) | 10 | 27 to 79 | 10 of 10 | 1,178 of 1,178 | 15 s |

Run on 15 November 2026 through the KiCad bridge with its defaults, on an idle Apple M5 Max laptop; the time
is the whole run, including loading and saving the board in KiCad. A single attempt uses one core, so the
thread count does not change these figures.

### Against other open routers, on the same machine

The same 119 stripped boards, the same KiCad 10 rule check, one board at a time on the same idle machine.
FreeRouting 2.5.0 ran headless with its defaults from KiCad's Specctra export. KiCadRoutingTools (commit
435ffbb) ran through its router `route.py` on all nets, which takes track, clearance and via sizes from the
board's project file. Times are each tool's whole run.

| Set | Router | Clean pass | Clean pass, track width forgiven | Connections made | Time per board |
|---|---|---|---|---|---|
| D3-A (99) | pcb-routing | 99 | 99 | 100% | 0.60 s |
| | KiCadRoutingTools | 94 | 95 | 100% | 0.69 s |
| | FreeRouting | 46 | 92 | 99.5% | 2.8 s |
| D3-B (10) | pcb-routing | 10 | 10 | 100% | 2.5 s |
| | KiCadRoutingTools | 8 | 8 | 100% | 2.0 s |
| | FreeRouting | 4 | 8 | 99.3% | 3.7 s |
| D3-C (10) | pcb-routing | 10 | 10 | 100% | 15 s |
| | KiCadRoutingTools | 6 | 6 | 99.5% | 8.2 s |
| | FreeRouting | 0 | 7 | 96.5% | 12.4 s |

How to read it:

- **Track width.** Both other routers narrow some tracks near pads (FreeRouting to 75% of the class
  width), and on many of these boards that falls below the board's minimum track width,
  which KiCad reports as an error. The fourth column counts a board as clean when those are its only errors.
  With FreeRouting's `automatic_neckdown` turned off it leaves more connections open and does worse in both
  columns (69, 2 and 1 clean), so the table shows it as shipped.
- **Speed on large boards.** KiCadRoutingTools is the faster router there, by about half again. The times of
  pcb-routing include what keeps buses together and links capacitors to their pins, which added about a
  seventh; one crowded board is routed twice, the second time without those links.
- **Front-ends.** KiCadRoutingTools' `autoroute.py` front-end, run with no arguments, routed every board with
  0.1 mm tracks and clearances instead of the board's own and made no board clean; the table uses `route.py`.
- **Not the paper's protocol.** The benchmark's authors prepare the boards with a KiCad 9 pipeline, pin older
  versions of both routers and score differently; they report clean pass 0.80 (D3-A) and 0.78 (D3-B) for
  FreeRouting 2.1.0 and 0.74 and 0.20 for KiCadRoutingTools. The figures above are this repository's
  preparation and are comparable with each other, not with the paper's.

### What the routes look like

Passing the rule check says nothing of how a route reads to a designer. These boards are real designs with
their tracks removed, so the original routing is there to compare with. Measured on the 111 boards that all
three routers connected completely, over the 1,105 nets (2,723 connections) that the designer routed with
tracks and not with a copper pour:

| | Track length, designer = 1 | Detour | Vias per 100 connections | Corners per connection | Turns of 90° or more per 100 | Short segments per 100 |
|---|---|---|---|---|---|---|
| The designer | 1.00 | 1.25 | 18.7 | 2.5 | 25 | 90 |
| pcb-routing | 0.90 | 1.13 | 10.0 | 1.7 | 12 | 40 |
| KiCadRoutingTools | 0.90 | 1.13 | 36.9 | 1.8 | 7 | 35 |
| FreeRouting | 0.95 | 1.18 | 15.8 | 2.7 | 41 | 83 |

Detour is track length over the shortest tree through a net's pad centres. A short segment is one shorter
than twice its width. pcb-routing has the least track, the fewest vias and the fewest corners;
KiCadRoutingTools has fewer sharp turns.

Buses: nets with neighbouring pads on one part and neighbouring pads on another, three or more together (54
buses, 199 pairs of neighbours on these boards). Alongside is the share of a net's track within 1.5 pad
pitches of its neighbour on the same layer.

| | Alongside its neighbour | Same number of vias | The two cross | Difference in length |
|---|---|---|---|---|
| The designer | 72% | 80% | 19% | 25% |
| pcb-routing | 72% | 81% | 13% | 25% |
| pcb-routing, buses left alone (`--bus-pull 0`) | 63% | 80% | 22% | 23% |
| KiCadRoutingTools | 71% | 61% | 26% | 25% |
| FreeRouting | 64% | 77% | 29% | 24% |

Decoupling: a capacitor with a pin of an IC of the same net within 5 mm (51 on these boards). The designers
have a via between the capacitor and that pin for 6% of them; pcb-routing 6% (14% with `--no-decoupling`),
KiCadRoutingTools 7% of those it joins directly, FreeRouting 8%.

Not in these tables: what needs to know what the circuit does (which nets are quiet and which are noisy).

The table is computed with [`tools/quality`](https://github.com/9thAxis/pcb-routing/blob/main/tools/quality/README.md), which defines each figure and works on
any routed KiCad board that has a reference routing to compare with:

    python3 tools/quality/quality.py DESIGNER.kicad_pcb mine=ROUTED.kicad_pcb

It also has first measures of pad exits, supply nets, decoupling paths and crystals, which are not reported
here.

### What it has

* All nets negotiated together; no dependence on net order; several differently ordered attempts side by
  side, the best kept.
* Exact clearances to fixed copper (segments, circles, polygons), per net class and per pair of nets; hole
  to hole spacing; keep-out regions for tracks, vias or both; board edge.
* Tracks at multiples of 45 degrees with a cost per corner, so lines run straight; through vias with a cost,
  reuse of a net's existing holes.
* Nets with many terminals as trees; partial results: what cannot be joined is named, terminal by terminal,
  and everything else is laid.
* Matched pairs: two nets laid side by side at a fixed pitch as one trunk, then split (library and program;
  not yet through the KiCad bridge).
* Per net: width, via size, allowed layers, a cost per layer, a wanted maximum length, readiness to give
  way, "no vias", "lay last without disturbing the others".
* Deterministic: the same problem gives the same answer, whatever the thread count.
* A self-check of the search (`--verify`), a mask export for comparing with another tool, a trace per round.
* Decoupling capacitors joined to the pin they stand by, in the KiCad bridge: a link of its own that is slow
  to change layer, given up for a net that cannot be completed with it.
* Copper zones followed: pads that a filled zone joins are left to the zone, and a net
  with a zone gets tracks only from the pads the zone does not reach. Other nets pay for crossing a zone
  and more for crossing the spokes that tie a pad to it; a zone that new tracks cut is joined again; a pad
  left with fewer spokes than the board asks for gets a track of its own into the zone. On the 119 public
  boards with their designers' zones put back: all complete, no new error of any kind in KiCad's check
  (thermal relief included), and 21 % less track than with the zones ignored. What the zones join, where
  a net may be joined to one and how each pad is tied to it are worked out in the crate, from the filled
  zones as polygons (`pcb_routing::pours`, or `pcb-routing pours`); the KiCad bridge fills the zones and asks.
* Buses kept together: nets marked as one bus (`bus` and `bus_lane` on a net; the KiCad bridge marks nets
  that leave one part and arrive at another side by side) are drawn to run beside each other on one layer,
  in the order of their pads, and are slow to cross each other.

### What it lacks

Things other routers have and this one does not, roughly in order of how often they are missed:

* **Push and shove, and interactive routing.** It is a batch router. KiCad's own router is interactive and
  shoves; FreeRouting has both modes.
* **Length and skew tuning** (meanders, matched groups). A wanted maximum length per net only ranks attempts.
  KiCadRoutingTools and the commercial routers tune lengths.
* **Pairs coupled all the way to the pads**, and pairs through the KiCad bridge. The legs of a pair are also
  not tested against fixed copper (see Limits).
* **Blind, buried and micro vias.** Through vias only.
* **Fan-out of fine-pitch and BGA parts** as a step of its own. Pads are escaped by the ordinary search,
  with a narrower second try.
* **A clean-up pass after routing** beyond laying each net once more with the others fixed: no via
  minimisation as a pass, no smoothing off the grid. FreeRouting has an optimiser.
* **Routing off the grid, arcs, teardrops.** But for their ends on pads, tracks run through cell centres
  (0.025 to 0.1 mm apart), so a crowded board loses a little room that a gridless router keeps.
* **KiCad's custom rule language.** The bridge reads net classes, board minima, pad clearances, rule areas
  and zones, not custom rules.
* **A design rule check of its own.** Use the CAD program's.
* **Size.** At most seven routing layers and 249 nets per run (the bridge routes larger boards in batches),
  and memory grows with the grid: about 200 bytes per cell and layer for each attempt running at once.

## As a library

```rust
use pcb_routing::{route, Problem, Status};

let problem: Problem = serde_json::from_str(&std::fs::read_to_string("problem.json")?)?;
let solution = route(&problem)?;
if solution.status != Status::Routed {
    eprintln!("not joined: {:?}", solution.unrouted);
}
for net in &solution.nets {
    println!("{}: {:.2} mm, {} vias", net.key, net.length_mm, net.vias.len());
}
```

Until it is on crates.io: `pcb-routing = { git = "https://github.com/9thAxis/pcb-routing" }`.

`route` returns an error only when the problem cannot be run at all (a size or cost that makes no sense, a
name that does not exist, a grid larger than `options.max_states`). A problem that was run and could not be
completed comes back as a solution: `unroutable` with the connections that could not be made named in
`unrouted` and everything else laid, or (with `drop_conflicts: false`) `conflicts` with the places where
nets are still too close.

`Router` gives more control: a callback per round of negotiation, a self-check of the search, one attempt
alone. See `examples/route_file.rs` and the API documentation (`cargo doc --open`).

For a board with filled zones, `pcb_routing::pours` turns the zones as your CAD program filled them into
what a problem takes: `survey` tells which pads the zones already join and counts the thermal spokes of
each pad, `reach` gives the places a net may be joined to its zone (the terminals of one `piece`), and
after a run `parts` tells which of the new copper hangs on a piece of zone, to be looked at again once the
zones are refilled. The module documentation has the whole round.

## As a program

    pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N] [--trace] [--verify] [--only ATTEMPT]
    pcb-routing masks PROBLEM.json --net KEY -o MASK.bin
    pcb-routing pours [QUESTION.json] [-o ANSWER.json]

`route` exits with status 0 if every net is laid and none are too close, 1 otherwise, 2 if the problem could
not be run. `--trace` prints one line per round and the time spent per kind of work. `--verify` runs every
search a second time without the first pass (see below) and reports any whose cost differs. `masks` writes
the router's picture of the board for one net, one byte per cell. `pours` answers one question about a
board's filled zones, as JSON (from standard input to standard output where no file is named): the field
`ask` names it (`survey`, `reach`, `parts`, `tie_targets`, `ties`), and the rest is as documented on
`pcb_routing::pours::Ask`.

Try it on `examples/two_nets.json`.

## The problem

Every field is documented on the types in `src/model.rs`. In short:

* `window`, `step`, routing `layers` (at most seven);
* `copper` already on the board (shapes `seg`, `circle`, `poly`, with net and layers; `pad` for solder lands);
* `holes` already on the board; `regions` closed to tracks and/or vias; `dear` ground, open but at a price
  (a factor on the length of track in it; nothing more for the net it belongs to);
* `rules`: hole spacing, clearance between new nets, a larger clearance between `quiet` nets and others,
  whether a via may stand in a land of its own net;
* `nets`: key, electrical net, width, via, layers, clearance to existing copper, terminals (those that
  share a `piece` number stand on one piece of copper already, and the net is joined to it at whichever comes
  cheapest), via penalty, layer costs, `yield` (how readily it gives way), `max_mm` and `weight` (for ranking attempts), `last`, `no_vias`, `bus` and `bus_lane` (the
  bus it runs with and its place across it);
* `pairs`: two nets laid side by side;
* `options`: rounds, corner cost, number of attempts, threads, the largest grid accepted, how strongly the
  nets of a bus are drawn together and kept from crossing (`bus_pull`, `bus_cross`), whether nets still
  in conflict at the end are taken out (`drop_conflicts`, the default) or returned as they stand.

All lengths are in mm. A misspelt field is an error, not ignored.

## How it works

1. **Fixed masks** (`src/mask.rs`, `src/engine/fixed.rs`). Per net and routing layer: the cells whose centre
   is too close to copper of another net, or inside closed ground, for a track of this net; and the cells
   closed to a via. Distances are exact, to segments, circles and polygons. A mask is worked out in pages of
   256 cells, only where a search goes, and nets with the same rules share the pages that none of their own
   copper is near.
2. **Search** (`src/engine/search.rs`). A* over (direction of arrival, layer, cell), eight directions. A 45
   degree corner costs `bend_mm`, a right angle three times that, sharper turns are not made. It runs in two
   passes: the first finds the cost from every state to the goal at the costs of this moment, without corners
   and so without directions; the second finds the path with corners, looking ahead with the first pass's
   cost, and only has to widen by what the corners cost.
3. **Nets with several terminals** grow as a tree, from the copper laid so far to the nearest remaining
   terminal (`src/engine/lay.rs`). What cannot be joined is reported and the rest is joined among itself.
4. **Claims** (`src/engine/claims.rs`). A laid net claims, for each size of track and via another net may
   have, the cells where the centre of such a track or via would be too close to it.
5. **Negotiation** (`src/engine/negotiate.rs`). Round by round, the nets in conflict are laid again. Ground
   claimed by others is not closed, only dearer: by a factor that grows each round, plus a standing cost left
   where conflicts were. The rounds end when no net is in conflict. If they do not end, the nets still in
   conflict are taken out one by one, laid again with the others fixed, and reported if they still do not fit.
6. **Closing passes**. Each net is laid once more with all the others fixed; the new path is kept if it joins
   more of the net, or as much for less.
   **Buses**: a net with a `bus` name pays `options.bus_pull` more per unit length wherever it is not beside
   a laid net of the same bus, in the lane its `bus_lane` gives it (nets two lanes apart leave room for the
   one between), and `options.bus_cross` more where it runs over or under one on another layer. In the rounds
   and in the closing passes alike, so the nets of a bus gather on one layer in the order of their pads.
7. **Pairs** (`src/engine/pairs.rs`). A pair negotiates as one wide track; once the nets have settled it is
   split into two legs a fixed pitch apart, each joined to its pads by a short stub.
8. **Ends** (`src/engine/ends.rs`). A pad seldom stands on a cell centre. The last corner of a track slides
   along the stretch before it so that the track ends on the pad's own point and keeps its directions. What
   moves is tested off the grid against the fixed copper and the other nets. Where it may not move a short
   stub is laid, and where that would come too near another net the track ends on its cell, within the pad.
9. **Attempts** (`src/router.rs`). `variants` runs differently ordered attempts side by side and keeps the
   best: complete before incomplete, then least missing, then the lowest sum of weighted lengths plus vias.

[`docs/design-notes.md`](https://github.com/9thAxis/pcb-routing/blob/main/docs/design-notes.md) (in the
repository) records what each of these bought when it was added, and what was tried and dropped.

## Limits

* The grid carries part of the clearance: a `routed` result keeps the rules between new nets at cell centres,
  and a diagonal step can come nearer by about 0.002 mm at a 0.05 mm step. The KiCad bridge adds 0.002 mm to
  every clearance for this (`--slack`).
* The legs of a pair are not tested against fixed copper. The trunk is, at its own width, which covers the
  legs on straight runs and 45 degree corners when it is at least `pitch + leg_width + 0.04` wide.
* Memory is about 200 bytes per (layer, cell) state for each attempt running at once. A problem beyond
  `options.max_states` (default 10 million) is refused.
* At most seven routing layers and 249 net entries per run.

## Evidence

| Check | Result |
|---|---|
| 119 public boards through KiCad's design rule check (table above) | All fully connected with no new error |
| Masks against an older, independent router, cell by cell: 4 nets with different rules on 964,371 cells by 4 layers | No cell open here that the other closes |
| Single connections against that router, each alone on the same board: 29 connections | 29 of 29 within 2 % in length |
| Independent clearance check without the grid: 29 nets, 73 tracks, 45 vias against 4,725 fixed shapes | 0 violations; a track moved by 0.12 mm as a control is caught |
| Two-pass search against one-pass, every search of a full run (`--verify`) | None differs by more than a quarter of a grid step |
| Page-wise masks against whole-board masks, 150 random problems in the tests (4,900 in review) | Identical |
| Tests (`cargo test`, `python3 -m unittest discover -s tools/tests`) | 70 in the engine, 121 in the bridge, 141 in the quality tools |

## Sources

McMurchie and Ebeling, PathFinder (1995), for negotiated congestion; Chen et al., Dr.CU (TCAD 2020), for extra
search state and conflict-only rip-up; FreeRouting's keep-if-better closing pass; Boyne, Acorn (2025), for
clearance-wide claims.

## Licence

Licensed under either of the Apache License, Version 2.0 (`LICENSE-APACHE`) or the MIT licence
(`LICENSE-MIT`), at your option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as
above, without any additional terms or conditions.
