# How it works

The problem the router takes, the method, and its limits. What each step bought when it was added, and
what was tried and dropped, is in the [design notes](design-notes.md).

## The problem

Every field is documented on the types in `src/model.rs`. In short:

* `window`, `step`, routing `layers` (at most seven);
* `copper` already on the board (shapes `seg`, `circle`, `poly`, with net and layers; `pad` for solder lands);
* `holes` already on the board; `regions` closed to tracks and/or vias; `dear` ground, open but at a price
  (a factor on the length of track in it; nothing more for the net it belongs to, or for one net `only`);
* `rules`: hole spacing, clearance between new nets, a larger clearance between `quiet` nets and others,
  whether a via may stand in a land of its own net;
* `nets`: key, electrical net, width, via, layers, clearance to existing copper, terminals (those that
  share a `piece` number stand on one piece of copper already, and the net is joined to it at whichever
  comes cheapest; a `turn` puts them in an order of preference), via penalty, layer costs, `yield` (how
  readily it gives way), `max_mm` and `weight` (for ranking attempts), `last`, `no_vias`, `bus` and
  `bus_lane` (the bus it runs with and its place across it);
* `pairs`: two nets laid side by side, each as a trunk entry and four stub entries of `nets`;
* `options`: rounds, corner cost, number of attempts, threads, the largest grid accepted, how strongly the
  nets of a bus are drawn together and kept from crossing (`bus_pull`, `bus_cross`), whether nets still
  in conflict at the end are taken out (`drop_conflicts`, the default) or returned as they stand.

All lengths are in mm. A misspelt field is an error, not ignored.

## The method

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
   split into two legs a fixed pitch apart, each joined to its pads by a short stub. The legs are laid only
   if they come out no nearer each other than the pitch and within the track they were split from.
8. **Ends** (`src/engine/ends.rs`). A pad seldom stands on a cell centre. The last corner of a track slides
   along the stretch before it so that the track ends on the pad's own point and keeps its directions. What
   moves is tested off the grid against the fixed copper and the other nets. Where the corner cannot slide,
   the end of the last stretch turns onto the point by a few degrees. Where nothing may move a short stub is
   laid, and where that would come too near another net the track ends on its cell, within the pad.
9. **Attempts** (`src/router.rs`). `variants` runs differently ordered attempts side by side and keeps the
   best: complete before incomplete, then least missing, then the lowest sum of weighted lengths plus vias.

## The program

    pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N] [--trace] [--verify] [--only ATTEMPT]
    pcb-routing masks PROBLEM.json --net KEY -o MASK.bin
    pcb-routing ask [QUESTION.json] [-o ANSWER.json] [--threads N]

`--trace` prints one line per round and the time spent per kind of work. `--verify` runs every search a second
time without the first pass and reports any whose cost differs. `masks` writes the router's picture of the
board for one net, one byte per cell. `ask` answers one question a front end has before or after routing, as
JSON (from standard input to standard output where no file is named): the field `ask` names it (`survey`,
`reach`, `parts`, `on_zone`, `tie_targets`, `ties` about filled zones, `pairs` about differential pairs), and
the rest is as documented on `pcb_routing::ask::Ask`.

## Limits

* The grid carries part of the clearance: a `routed` result keeps the rules between new nets at cell centres,
  and a diagonal step can come nearer by about 0.002 mm at a 0.05 mm step. The KiCad bridge adds 0.002 mm to
  every clearance for this (`--slack`).
* The legs of a pair are not tested against fixed copper themselves. The trunk they are split from is, at
  its own width, and a pair is laid only if its legs come out within the trunk and no nearer each other
  than their pitch; else it is reported as not laid.
* Memory is about 200 bytes per (layer, cell) state for each attempt running at once. A problem beyond
  `options.max_states` (default 10 million) is refused.
* At most seven routing layers and 249 net entries per run.

## What it has

In the engine:

* All nets negotiated together; no dependence on net order; several differently ordered attempts side by
  side, the best kept.
* Exact clearances to fixed copper (segments, circles, polygons), per net class and per pair of nets; hole
  to hole spacing; keep-out regions for tracks, vias or both; board edge; ground that is open at a price.
* Tracks at multiples of 45 degrees with a cost per corner, so lines run straight; through vias with a cost,
  reuse of a net's existing holes.
* Nets with many terminals as trees; terminals that stand on one piece of copper already (a zone, pads that
  are joined), reached where it comes cheapest; partial results: what cannot be joined is named, terminal by
  terminal, and everything else is laid.
* Per net: width, via size, allowed layers, a cost per layer, a wanted maximum length, readiness to give
  way, "no vias", "lay last without disturbing the others".
* Differential pairs: two nets laid side by side at a fixed pitch as one trunk, then split into two legs
  that are checked before they are laid.
* Buses: nets marked as one bus are drawn to run beside each other on one layer, in the order given, and are
  slow to cross each other.
* Deterministic: the same problem gives the same answer, whatever the thread count.
* A self-check of the search (`--verify`), a mask export for comparing with another tool, a trace per round.

For a front end (the KiCad bridge is one), and measured through it:

* **Copper zones followed.** Pads that a filled zone joins are left to the zone, and a net with a zone gets
  tracks only from the pads the zone does not reach. Other nets pay for crossing a zone and more for
  crossing the spokes that tie a pad to it; a zone that new tracks cut is joined again; a pad left with fewer
  spokes than the board asks for gets a track of its own into the zone. On the 119 public boards with their
  designers' zones put back: all complete, no new error of any kind in KiCad's check (thermal relief
  included), and a fifth less track than with the zones ignored. What the zones join, where a net may be
  joined to one and how each pad is tied to it are worked out in the crate, from the filled zones as
  polygons (`pcb_routing::pours`); the bridge fills the zones and asks.
* **Differential pairs found and laid.** Two nets named as a pair (`_P`/`_N`, `+`/`-`, `DP`/`DM`), or said
  to be one in the hints, are laid side by side between the places where a pad of each stand side by side
  (`pcb_routing::pairs`), begun as near their pads as there is room, and as two nets where that cannot be
  done. On 20 pairs of 13 public boards: coupled for 51 % of their length, where the same router without
  pairs has 18 % and the designers 39 %; all boards complete, with no new error in KiCad's check.
* **What the nets are for.** Guessed from names and parts (clocks and crystals are slow to change layer),
  and said in an optional hints file beside the board
  ([docs/hints.md](hints.md)): roles, a width for a
  current, a clearance for a voltage, layers, "no vias", a greatest length, pairs with their gap, buses. A
  guess only leans a net, and is dropped where it would cost a connection; what binds a net comes from the
  file.
* **Buses kept together.** Nets that leave one part and arrive at another side by side are marked as a bus,
  with lanes in the order of their pads.
* **Long pads left through an end.** The ground beside the long sides of a pad costs the pad's own net more,
  so its track runs out along the pad before it turns: 31 % of long pads are left through a long side, where
  the designers have 33 % and the router without this 53 %.
* **Decoupling capacitors joined to the pin they stand by**: a link of its own that is slow to change
  layer, given up for a net that cannot be completed with it.

## What it lacks

Things other routers have and this one does not, roughly in order of how often they are missed:

* **Push and shove, and interactive routing.** It is a batch router. KiCad's own router is interactive and
  shoves; FreeRouting has both modes.
* **Length and skew tuning** (meanders, matched groups). A wanted maximum length per net only ranks attempts.
  KiCadRoutingTools and the commercial routers tune lengths.
* **Pairs coupled all the way to the pads, and pairs that change layer.** A pair is laid side by side on one
  layer between two points a little off its pads, and where its legs would have to cross to reach their pads
  it is laid as two nets. There is no impedance calculation: a pair has the width and gap it is told.
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

## Sources

McMurchie and Ebeling, PathFinder (1995), for negotiated congestion; Chen et al., Dr.CU (TCAD 2020), for extra
search state and conflict-only rip-up; FreeRouting's keep-if-better closing pass; Boyne, Acorn (2025), for
clearance-wide claims.
