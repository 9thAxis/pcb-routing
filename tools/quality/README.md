# Route quality against a reference routing

Passing the design rule check says nothing of how a route reads to a designer. These tools measure routed
KiCad boards against a reference routing of the same board, usually the designer's own: how much track, how
many vias and corners, how close to other copper, whether buses stay together, how capacitors reach their
pins, how tracks leave pads. The "What the routes look like" table in the main README is computed with them.

The measures are counts and ratios of geometry. They do not replace the design rule check and they do not
know what the circuit needs; read them as a comparison between routings of the same boards, not as marks.

## In one command

    python3 tools/quality/quality.py DESIGNER.kicad_pcb mine=ROUTED.kicad_pcb other=ROUTED2.kicad_pcb

All the boards must be the same board (the same parts in the same places), routed differently. The command
needs a Python 3 with `shapely` and `numpy` (`pip install shapely numpy`; `matplotlib` too for `--picture`),
and KiCad 9 or 10 for reading the boards: it runs the dump under KiCad's own Python, taken from
`$KICAD_PYTHON` if set, else the one inside KiCad.app on macOS, else `/usr/bin/python3` or `python3` if they
can import `pcbnew` (as on Linux).

Options: `--json FILE` writes the figures as JSON, `--picture OUT.png` draws the routings side by side,
`--dumps DIR` keeps the dumps, `--project FILE.kicad_pro` names the project to read the reference's net
classes from, `--clearance MM` sets the clearance for the spacing figures, `--assume-complete` scores routings
that KiCad counts as incomplete.

## Step by step

For many boards, or to keep the KiCad step apart from the scoring:

1. **Dump** each board, with KiCad's Python (no other dependency):

       KIPY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
       $KIPY tools/quality/dump.py board1/designer.kicad_pcb -o dumps/board1.designer.json
       $KIPY tools/quality/dump.py board1/routed.kicad_pcb   -o dumps/board1.mine.json

   A dump holds the copper layers, the pads with their outlines, the tracks, the vias, the nets that have a
   zone, KiCad's count of open connections, and the net classes of the board's project (`--project` names
   another project file, for a board that has none beside it). `dump.py --help` lists the fields.

2. **Score**, with a Python that has shapely and numpy. One board:

       python3 tools/quality/score.py dumps/board1.designer.json mine=dumps/board1.mine.json

   or every board of a directory of `BOARD.NAME.json` dumps:

       python3 tools/quality/score.py --dir dumps --reference designer mine other --json figures.json

   The tables go to standard output, the boards left out and why to standard error. `--json FILE` writes the
   same figures, with the boards used and skipped; `--per-net` adds the general figures net by net.

3. **Look** (optional, needs matplotlib):

       python3 tools/quality/show.py out.png designer=dumps/board1.designer.json mine=dumps/board1.mine.json

## What is compared

**Boards.** A board is scored only if every named routing of it is there, has the reference's pads (same
places, same nets) and is complete; the rows are then sums over the same boards and the same nets. Whether a
routing is complete is read from its dump: `unconnected` is KiCad's count of open connections on the board as
it was saved (with its zones as they were filled), and 0 means complete. It is not worked out again from the
geometry. For dumps without that field, or to score incomplete routings anyway, the caller states it with
`--assume-complete`; with `--dir`, `--boards FILE` then names the boards to take. The reference is not
required to be complete.

**Nets.** The reference decides which nets are measured, and the same nets are measured in every routing,
the reference included. A net is *routed with tracks* when the reference has at least one track segment of
it, and *poured* when the reference has a zone of it: what a designer joined with a plane is not track to
compare. Each table then takes:

| Table | Nets |
|---|---|
| General | pads at two or more places, routed with tracks and not poured in the reference |
| Buses | pairs of such nets with neighbouring pads on two parts, in groups of three nets or more |
| Circuit: power | named as a supply or ground, pads at two or more places, not poured in the reference, with tracks in the routing measured |
| Circuit: decoupling | the supply net of a capacitor next to a chip pin; a supply poured in the reference is left out for the reference's own row only |
| Circuit: crystals | on parts `Y..`, `X..`, `XTAL..`, not power, with tracks in the routing measured |
| Circuit: pairs | two nets named as halves of a pair, not power, not poured in the reference, both with tracks in the routing measured |
| Pad exits | pads of nets routed with tracks and not poured in the reference |

**Rules.** The clearance (of the `Default` net class) and the track width of each net's class are read from
the reference dump's project rules, else from the first routing that has any; `--clearance` overrides the
clearance. Without any, the clearance is 0.2 mm and no power track counts as narrow.

Two points are the same place when they agree to 0.001 mm.

## The figures

### General

A net with pads at n places has n - 1 *connections*. Every figure is a sum over all compared nets of all
boards divided by another such sum, not a mean of per-net ratios.

| Column | JSON | Meaning |
|---|---|---|
| mm | `mm` | Track length: the sum of KiCad's segment lengths (arcs along the arc) |
| detour | `detour` | Track length over the length of the shortest tree through each net's pad centres (Euclidean minimum spanning tree, no added points). It can fall under 1: a route may branch between pads, and tracks may stop short of pad centres |
| vs ref | `length_vs_reference` | Track length over the reference's |
| vias/100 | `vias_per_100` | Vias per 100 connections |
| corners | `corners_per_connection` | Corners per connection. A corner is a place where exactly two straight segments of the net end on the same layer and the direction changes by more than 1°. Three segments meeting (a T) is a branch and not a corner; a change of layer is not one; a segment meeting an arc is not counted |
| sharp/100 | `sharp_per_100` | Corners where the direction changes by more than 89° (turns of 90° or more), per 100 connections |
| short/100 | `short_per_100` | Segments shorter than twice their own width, per 100 connections |
| tight % | `tight_percent` | Share of track length with less than 1.1 clearances of air to other nets' copper |
| gap | `median_gap` | The median of that air over the track length, in clearances, capped at 5 |

For the last two, each segment is cut into pieces of at most 0.5 mm. A piece's gap is the distance from its
centre line to the nearest copper of another net on its layer, less half its own width. The other copper is
tracks, vias (taken as standing on every layer) and pads; zone fills are not in the dump and do not count.
Arcs are measured along their chord. A piece counts whole.

### Buses

Two pads of a part are *neighbours* when their distance is at most 1.3 times the distance from the first to
its nearest pad. Two nets are a *pair* between parts F and G when they have neighbouring pads on F and
neighbouring pads on G. Pairs between the same two parts that hang together form a group, and only groups of
three nets or more count as buses. A pair that is side by side on more than two parts counts once. The
*pitch* of a pair is the larger of its pad distances. The first
net of a pair is the one whose name sorts first. Each figure is the mean over the pairs both of whose nets
have tracks in the routing:

| Column | JSON | Meaning |
|---|---|---|
| pairs | `pairs` | Pairs measured |
| alongside % | `alongside_percent` | Share of the first net's track within 1.5 pitches of the second's on the same layer |
| same vias % | `same_vias_percent` | Pairs whose two nets have the same number of vias |
| crossing % | `crossing_percent` | Pairs where a segment of one crosses a segment of the other on a different layer |
| length mismatch % | `length_mismatch_percent` | Difference of the two lengths over the longer |

### Circuit

These read what nets are for from net names and part references, so they are only as good as the names.

*Power* nets match the pattern `POWER` in `measures.py` (`GND`, `VCC`, `+3V3`, `VDD_IO`, `VBUS`, ...).

| Column | JSON | Meaning |
|---|---|---|
| power: nets | `power_nets` | Power nets measured in this routing (those it laid as tracks, so rows can differ) |
| narrow% | `power_narrow_percent` | Share of their length narrower than the net class's track width |
| vias/100 | `power_vias_per_100` | Vias per 100 connections |
| detour | `power_detour` | As in the general table |
| decaps | `decaps` | Decoupling capacitors: a two-pad part `C..` between a supply net and a ground net (`GND`, `VSS`, `VEE` in the name), with a pad of the same supply on a part `U..` or `IC..` within 5 mm; the nearest such pad is its pin |
| path/straight (median) | `decap_path_over_straight_median` | Length of the supply's copper from the capacitor's pad to the pin (the shortest way along its tracks; a via adds nothing) over the straight distance; the median (the upper of two middle values) over the capacitors that are joined that way; `nan` in the table and `null` in JSON without any |
| via between% | `decap_via_between_percent` | Of those, the share whose way passes a via |
| not joined% | `decap_not_joined_directly_percent` | Capacitors with no way along tracks to the pin: joined through another pad, a zone, or not at all |
| xtal nets | `crystal_nets` | Crystal nets with tracks in this routing |
| detour, vias | `crystal_detour`, `crystal_vias` | Their detour, and their vias (a count) |
| pairs | `pairs` | Differential pairs: two nets whose names differ only in an ending `_P`/`_N`, `-P`/`-N`, `P`/`N`, `+`/`-` or `DP`/`DM` |
| coupled% | `pair_coupled_percent` | Share of the first half's length (the name that sorts first) within 4 of its widest track widths of the other half on the same layer, mean over pairs |
| mismatch% | `pair_mismatch_percent` | Difference of the two lengths over the longer, mean over pairs |
| same vias% | `pair_same_vias_percent` | Pairs whose halves have the same number of vias |

A track end is on a pad when it lies inside the pad's outline grown by 0.01 mm, on that layer. Tracks are cut
where another track of the net ends on them or a via stands on them.

### Pad exits

For every straight segment that ends on the centre of a pad of its net, the track is followed (up to six
segments, as long as it does not branch) to the first segment that leaves the pad's outline; that is one
*exit*. Tracks that end on a pad elsewhere than its centre are not counted.

| Column | JSON | Meaning |
|---|---|---|
| exits | `exits` | Exits found |
| grazing % | `grazing_percent` | Exits that meet the edge of the outline they cross at under 40° |
| long-pad exits | `long_pad_exits` | Exits from pads at least 1.5 times as long as wide (by the smallest rectangle around the pad) |
| through long side % | `through_long_side_percent` | Of those, the share that leave through a long side |

## What to keep in mind

* Vias are taken as through vias; a blind or buried via counts as standing on every layer.
* Zone copper is not in the dump: a track hugging a pour is not "tight", and nets joined by a pour in a
  routing but by tracks in the reference are measured by their tracks only.
* The circuit table goes by names (`C1`, `U3`, `Y1`, `GND`, `USB_DP`). A net ending in `P` or `N` with a twin
  is taken as half of a pair whether it is one or not; check the pair count against the board.
* The reference's own row is measured like the others. A reference with open connections has less track than
  a complete routing of it; the dump says how many are open.
* Sums favour big boards: one large board can outweigh many small ones. `--per-net` gives the rows to weigh
  differently.

## Layout

| File | What | Needs |
|---|---|---|
| `dump.py` | a `.kicad_pcb` to a JSON dump | KiCad's Python (`pcbnew`) |
| `measures.py` | the measures: pure functions of dumps, with every definition in its docstring | shapely, numpy |
| `score.py` | reads dumps, sums over boards, prints tables, writes JSON | shapely, numpy |
| `show.py` | a picture of several dumps side by side | matplotlib |
| `quality.py` | dump and score in one command | all of the above |

Tests: `python3 -m unittest discover -s tools/tests` (the `test_quality_*` files; those that need shapely are
skipped without it).
