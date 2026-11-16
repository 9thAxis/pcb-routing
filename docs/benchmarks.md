# Benchmarks

How pcb-routing does on public boards, against two other open routers and against the boards' own designers.
The short version is in the [README](../README.md).

## Results on public boards

The boards are those of the PCBWorld benchmark (LG AI Research, arXiv:2607.05915; real open-source KiCad
boards from PCBench with their tracks removed). A board counts as a clean pass when every connection is made
and KiCad's design rule check reports no error that the unrouted board did not have.

| Set | Boards | Nets per board | Clean pass | Connections made | Time per board |
|---|---|---|---|---|---|
| D3-A (small) | 99 | 2 to 13 | 99 of 99 | 1,301 of 1,301 | 0.6 s |
| D3-B (medium) | 10 | 5 to 42 | 10 of 10 | 445 of 445 | 2.3 s |
| D3-C (large) | 10 | 27 to 79 | 10 of 10 | 1,178 of 1,178 | 19 s |

Run on 15 November 2026 through the KiCad bridge with its defaults, on an Apple M5 Max laptop with little
else running; the time is the whole run, including loading and saving the board in KiCad. A single attempt
uses one core, so the thread count does not change these figures.

The benchmark strips the boards of their copper zones as well as their tracks. With the designers' zones put
back, as the boards were drawn, the bridge follows them (its default):

| Set | Boards | Clean pass | Connections made | Track laid | Vias | Time per board |
|---|---|---|---|---|---|---|
| D3-A (small) | 99 | 99 of 99 | 980 of 980 | 11,500 mm (14,733 without zones) | 85 (71) | 0.7 s |
| D3-B (medium) | 10 | 10 of 10 | 325 of 325 | 3,990 mm (5,112) | 36 (34) | 2.9 s |
| D3-C (large) | 10 | 10 of 10 | 852 of 852 | 10,735 mm (13,249) | 333 (278) | 36 s |

(The lengths move by a few millimetres from one preparation of the boards to the next, as KiCad's fills do;
a board as saved routes the same every time.) Fewer connections are left to make, a fifth less track is
laid, and a few more vias (a pad on one side taken down to a zone on the other). The large boards take
longer: after each run the zones are filled again and what the new tracks cut is mended, and a board that is
not all joined at once is laid again without pairs and without what was guessed from the nets' names.

## Against other open routers, on the same machine

The same 119 stripped boards, the same KiCad 10 rule check, one board at a time on the same machine (the
other two routers were timed earlier the same day, with the machine idle).
FreeRouting 2.5.0 ran headless with its defaults from KiCad's Specctra export. KiCadRoutingTools (commit
435ffbb) ran through its router `route.py` on all nets, which takes track, clearance and via sizes from the
board's project file. Times are each tool's whole run.

| Set | Router | Clean pass | Clean pass, track width forgiven | Connections made | Time per board |
|---|---|---|---|---|---|
| D3-A (99) | pcb-routing | 99 | 99 | 100% | 0.58 s |
| | KiCadRoutingTools | 94 | 95 | 100% | 0.69 s |
| | FreeRouting | 46 | 92 | 99.5% | 2.8 s |
| D3-B (10) | pcb-routing | 10 | 10 | 100% | 2.3 s |
| | KiCadRoutingTools | 8 | 8 | 100% | 2.0 s |
| | FreeRouting | 4 | 8 | 99.3% | 3.7 s |
| D3-C (10) | pcb-routing | 10 | 10 | 100% | 19 s |
| | KiCadRoutingTools | 6 | 6 | 99.5% | 8.2 s |
| | FreeRouting | 0 | 7 | 96.5% | 12.4 s |

How to read it:

- **Track width.** Both other routers narrow some tracks near pads (FreeRouting to 75% of the class
  width), and on many of these boards that falls below the board's minimum track width,
  which KiCad reports as an error. The fourth column counts a board as clean when those are its only errors.
  With FreeRouting's `automatic_neckdown` turned off it leaves more connections open and does worse in both
  columns (69, 2 and 1 clean), so the table shows it as shipped.
- **Speed on large boards.** KiCadRoutingTools is the faster router there, more than twice as fast. The times
  of pcb-routing include what keeps buses together and links capacitors to their pins, which added about a
  seventh; and a board that is not all joined at once, or on which a pair is not all laid, is routed again:
  without those links, with the pair begun further from its pads, or without what was guessed from the names
  of its nets. Three of the ten take most of the time that way.
- **Front-ends.** KiCadRoutingTools' `autoroute.py` front-end, run with no arguments, routed every board with
  0.1 mm tracks and clearances instead of the board's own and made no board clean; the table uses `route.py`.
- **Not the paper's protocol.** The benchmark's authors prepare the boards with a KiCad 9 pipeline, pin older
  versions of both routers and score differently; they report clean pass 0.80 (D3-A) and 0.78 (D3-B) for
  FreeRouting 2.1.0 and 0.74 and 0.20 for KiCadRoutingTools. The figures above are this repository's
  preparation and are comparable with each other, not with the paper's.

## What the routes look like

Passing the rule check says nothing of how a route reads to a designer. These boards are real designs with
their tracks removed, so the original routing is there to compare with. Measured on the 111 boards that all
three routers connected completely, over the 1,105 nets (2,723 connections) that the designer routed with
tracks and not with a copper pour:

| | Track length, designer = 1 | Detour | Vias per 100 connections | Corners per connection | Turns of 90° or more per 100 | Short segments per 100 |
|---|---|---|---|---|---|---|
| The designer | 1.00 | 1.25 | 18.7 | 2.5 | 25 | 90 |
| pcb-routing | 0.90 | 1.13 | 10.0 | 1.8 | 13 | 44 |
| KiCadRoutingTools | 0.90 | 1.13 | 36.9 | 1.8 | 7 | 35 |
| FreeRouting | 0.95 | 1.18 | 15.8 | 2.7 | 41 | 83 |

Detour is track length over the shortest tree through a net's pad centres. A short segment is one shorter
than twice its width. pcb-routing has the fewest vias, and as little track and as few corners as
KiCadRoutingTools, which has fewer sharp turns.

Buses: nets with neighbouring pads on one part and neighbouring pads on another, three or more together (54
buses, 199 pairs of neighbours on these boards). Alongside is the share of a net's track within 1.5 pad
pitches of its neighbour on the same layer.

| | Alongside its neighbour | Same number of vias | The two cross | Difference in length |
|---|---|---|---|---|
| The designer | 72% | 80% | 19% | 25% |
| pcb-routing | 71% | 80% | 13% | 25% |
| pcb-routing, buses left alone (`--bus-pull 0`) | 65% | 82% | 23% | 24% |
| KiCadRoutingTools | 71% | 61% | 26% | 25% |
| FreeRouting | 64% | 77% | 29% | 24% |

Decoupling: a capacitor with a pin of an IC of the same net within 5 mm (51 on these boards). The designers
have a via between the capacitor and that pin for 6% of them; pcb-routing 6% (12% with `--no-decoupling`),
KiCadRoutingTools 7% of those it joins directly, FreeRouting 8%.

Pad exits: of the pads at least 1.5 times as long as wide, the designers leave 33% through a long side,
pcb-routing 31% (53% with `--exit-cost 0`), FreeRouting 29%.

Not in these tables: what needs to know what the circuit does (which nets are quiet and which are noisy).

The table is computed with
[`tools/quality`](../tools/quality/README.md), which defines
each figure and works on any routed KiCad board that has a reference routing to compare with:

    python3 tools/quality/quality.py DESIGNER.kicad_pcb mine=ROUTED.kicad_pcb

It also has first measures of pad exits, supply nets, decoupling paths and crystals, which are not reported
here.

## Evidence

| Check | Result |
|---|---|
| 119 public boards through KiCad's design rule check (first table) | All fully connected with no new error |
| Masks against an older, independent router, cell by cell: 4 nets with different rules on 964,371 cells by 4 layers | No cell open here that the other closes |
| Single connections against that router, each alone on the same board: 29 connections | 29 of 29 within 2 % in length |
| Independent clearance check without the grid: 29 nets, 73 tracks, 45 vias against 4,725 fixed shapes | 0 violations; a track moved by 0.12 mm as a control is caught |
| Two-pass search against one-pass, every search of a full run (`--verify`) | None differs by more than a quarter of a grid step |
| Page-wise masks against whole-board masks, 150 random problems in the tests (4,900 in review) | Identical |
| Tests (`cargo test`, `python3 -m unittest discover -s tools/tests`) | 164 in the engine and its modules for zones and pairs, 178 in the bridge, 141 in the quality tools |
