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

The bridge (`tools/kicad_route.py`) runs under the Python that ships with KiCad (9 or 10; the path above is
the macOS one). It reads pads, tracks, vias, holes, the outline, rule areas and the net classes, routes every
net whose pads are not yet joined, writes the tracks and vias back and refills the zones. What is already
routed is kept and routed around. `--nets A,B` routes only those nets; `--layers F.Cu,B.Cu` restricts the
layers; `--coarse-first` trades a little quality for about three times the speed. `--help` lists the rest.

## How it compares

### Results on public boards

The boards are those of the PCBWorld benchmark (LG AI Research, arXiv:2607.05915; real open-source KiCad
boards from PCBench with their tracks removed). A board counts as a clean pass when every connection is made
and KiCad's design rule check reports no error that the unrouted board did not have.

| Set | Boards | Nets per board | Clean pass | Connections made | Time per board |
|---|---|---|---|---|---|
| D3-A (small) | 99 | 2 to 13 | 99 of 99 | 1,301 of 1,301 | 0.5 s |
| D3-B (medium) | 10 | 5 to 42 | 10 of 10 | 445 of 445 | 2.0 s |
| D3-C (large) | 10 | 27 to 79 | 10 of 10 | 1,178 of 1,178 | 10.9 s |

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
| D3-A (99) | pcb-routing | 99 | 99 | 100% | 0.47 s |
| | KiCadRoutingTools | 94 | 95 | 100% | 0.69 s |
| | FreeRouting | 46 | 92 | 99.5% | 2.8 s |
| D3-B (10) | pcb-routing | 10 | 10 | 100% | 2.0 s |
| | KiCadRoutingTools | 8 | 8 | 100% | 2.0 s |
| | FreeRouting | 4 | 8 | 99.3% | 3.7 s |
| D3-C (10) | pcb-routing | 10 | 10 | 100% | 10.9 s |
| | KiCadRoutingTools | 6 | 6 | 99.5% | 8.2 s |
| | FreeRouting | 0 | 7 | 96.5% | 12.4 s |

How to read it:

- **Track width.** Both other routers narrow some tracks near pads (FreeRouting to 75% of the class
  width), and on many of these boards that falls below the board's minimum track width,
  which KiCad reports as an error. The fourth column counts a board as clean when those are its only errors.
  With FreeRouting's `automatic_neckdown` turned off it leaves more connections open and does worse in both
  columns (69, 2 and 1 clean), so the table shows it as shipped.
- **Vias.** On the boards both routers made clean, pcb-routing used 59 vias where KiCadRoutingTools used 263
  (D3-A, 94 boards), 26 against 137 (D3-B, 8 boards) and 141 against 421 (D3-C, 6 boards), for 0.6 to 4% less
  track.
- **Speed on large boards.** KiCadRoutingTools is the faster router there: on the six D3-C boards both made
  clean it took 20 s in all where pcb-routing took 66 s.
- **Front-ends.** KiCadRoutingTools' `autoroute.py` front-end, run with no arguments, routed every board with
  0.1 mm tracks and clearances instead of the board's own and made no board clean; the table uses `route.py`.
- **Not the paper's protocol.** The benchmark's authors prepare the boards with a KiCad 9 pipeline, pin older
  versions of both routers and score differently; they report clean pass 0.80 (D3-A) and 0.78 (D3-B) for
  FreeRouting 2.1.0 and 0.74 and 0.20 for KiCadRoutingTools. The figures above are this repository's
  preparation and are comparable with each other, not with the paper's.

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
* **Routing off the grid, arcs, teardrops.** Tracks run through cell centres (0.025 to 0.1 mm apart), so a
  crowded board loses a little room that a gridless router keeps.
* **KiCad's custom rule language and planes.** The bridge reads net classes, board minima, pad clearances and
  rule areas, not custom rules; a net meant to be joined by a zone is routed as tracks unless left out.
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

## As a program

    pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N] [--trace] [--verify] [--only ATTEMPT]
    pcb-routing masks PROBLEM.json --net KEY -o MASK.bin

`route` exits with status 0 if every net is laid and none are too close, 1 otherwise, 2 if the problem could
not be run. `--trace` prints one line per round and the time spent per kind of work. `--verify` runs every
search a second time without the first pass (see below) and reports any whose cost differs. `masks` writes
the router's picture of the board for one net, one byte per cell.

Try it on `examples/two_nets.json`.

## The problem

Every field is documented on the types in `src/model.rs`. In short:

* `window`, `step`, routing `layers` (at most seven);
* `copper` already on the board (shapes `seg`, `circle`, `poly`, with net and layers; `pad` for solder lands);
* `holes` already on the board; `regions` closed to tracks and/or vias;
* `rules`: hole spacing, clearance between new nets, a larger clearance between `quiet` nets and others,
  whether a via may stand in a land of its own net;
* `nets`: key, electrical net, width, via, layers, clearance to existing copper, terminals, via penalty, layer
  costs, `yield` (how readily it gives way), `max_mm` and `weight` (for ranking attempts), `last`, `no_vias`;
* `pairs`: two nets laid side by side;
* `options`: rounds, corner cost, number of attempts, threads, the largest grid accepted, whether nets still
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
7. **Pairs** (`src/engine/pairs.rs`). A pair negotiates as one wide track; once the nets have settled it is
   split into two legs a fixed pitch apart, each joined to its pads by a short stub.
8. **Attempts** (`src/router.rs`). `variants` runs differently ordered attempts side by side and keeps the
   best: complete before incomplete, then least missing, then the lowest sum of weighted lengths plus vias.

`docs/design-notes.md` records what each of these bought when it was added, and what was tried and dropped.

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
| Tests (`cargo test`, `python3 -m unittest discover -s tools/tests`) | 54 in the engine, 91 in the bridge |

## Sources

McMurchie and Ebeling, PathFinder (1995), for negotiated congestion; Chen et al., Dr.CU (TCAD 2020), for extra
search state and conflict-only rip-up; FreeRouting's keep-if-better closing pass; Boyne, Acorn (2025), for
clearance-wide claims.

## Licence

Licensed under either of the Apache License, Version 2.0 (`LICENSE-APACHE`) or the MIT licence
(`LICENSE-MIT`), at your option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as
above, without any additional terms or conditions.
