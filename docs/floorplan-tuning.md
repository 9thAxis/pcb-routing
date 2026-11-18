# What the floorplan counts and weighs by

The floorplan (`pcb_routing::floorplan`, the `floorplan` question, `tools/kicad_floorplan.py`) is made with
some fifty numbers: distances that are habits of placing, shares of a board, and the weights by which one
laying out is held better than another. None is a rule of a board. This page says which of them a board may
set, and how far the values they have were checked.

## What a board may set

In the question, as `tune`; in the hints file beside a board (`BOARD.routing.json`), as `"tune": {...}`. What
is left out has its default; a value that makes no sense is refused, and told which.

| name | default | what it is |
|---|---|---|
| `quiet_mm` | 15 | What disturbs (a crystal, a switch node, a part said to be `noisy`) is to be this far from what is disturbed. |
| `beside_pin_mm` | 3 | A capacitor is by the pad it is for if its own pad on that supply is no further from it than this (a large one: its own length and a millimetre). |
| `block_reach_mm` | 8 | How far round a chip its block may reach, and how far from its pad a part of the block may stand. |
| `large_mm2` | 64 | A body of this area or more is settled before the others, over the whole ground of its domain. |
| `packed` | 0.5 | How much of the ground they are laid out on the parts of one of the smallest zones fill, on a board with room to spare. |
| `board_full` | 0.6 | With several boards: the most of a board's room the parts of its domains may take. |
| `board_most` | 0.85 | And the most where no parting of the domains leaves every board under `board_full`. |
| `road_most` | 0.2 | The most of a piece of the board that the road across it takes (no more than 0.5). |
| `between_copies_mm` | 1 | What is left between two copies of a sheet that stand side by side. |
| `beside_mm` | 1 | What is left between two parts side by side on a gap between two grounds. |
| `row_worth` | 1.25 | With `rows: cheap`, copies stand side by side if their pads are then no more than this many times as far from what they join as with each copy by itself. |
| `refine_mm` | 8 | How far from where a small part wants to be room is looked for when the small parts are gone over. |
| `sheeted` | 0, 2, 4 | What ties the groups drawn on one sheet to each other when zones are gathered, as so many nets: the board is laid out with each in turn and the best plan kept. One value lays it out once. |

```json
{"tune": {"quiet_mm": 20, "packed": 0.6, "sheeted": [0, 4]}}
```

The others are in `src/floorplan/constants.rs`, each with what it is. They are the weights of the search and
the sizes of its steps: what they do to a plan cannot be said of one board in advance, which is why they are
not offered.

## How the values were checked

Against boards whose designers placed them. The plan of a board is made from its nets and the bodies of its
parts, as if nothing stood yet; then the signal nets are measured by where the plan puts their parts, and by
where the designers put them. 118 boards: nine of 146 to 932 parts (eight published designs: a logic
analyser, an oscilloscope, a battery manager, a carrier for a compute module, an FPGA board, a solar charge
controller, a miner, a radio; and the board this was built for), and 109 small published boards of 2 to 38
parts. Each twice: every part free, and every connector and switch held where its designers put it.

Every constant was then halved and doubled, one at a time (a share: halved, and half as much again), and all
236 plans made again: 97 variants, with `tools/dev/constants_sweep.py`.

What came of it:

- **No value is better than the one it has.** With the values as they are, the plan's nets on the nine larger
  boards are 1.07 of the designers' at the geometric mean with every part free, and 1.11 with the connectors
  held. That is the median of the 97 variants on both counts. The best variant reaches 1.055 and 1.078; only
  two are better on both counts at once, by less than 0.01 each. On the small boards the nets are 0.71 of
  the designers' in all (0.81 held), and 68 of the 97 variants leave that as it is.
- **A plan is sensitive in ways that are not the constant's doing.** Halving and doubling one value both make
  the larger boards worse by 0.05 for several of them (the lanes a supply counts for, how far down cuts are
  tried the other way, the level of road a part with holes keeps off). A change of less than 0.02 in the mean
  says nothing; one board alone moves by 0.1 to 0.3 from a small change. This is why the board is laid out
  several ways and the best kept, and why a default was not moved for a gain of 0.01.
- **What costs most when it is wrong** (0.04 or more on the mean of the larger boards, one way or the other):
  how many capacitors make a net a supply, the lanes a supply takes in a road, what a way under a zone and a
  layer change count for, how far a connection runs on the layer of its parts, how far down cuts are tried
  across and what a road asked for too much counts for, the ties of a shared supply, how large a net may be
  for a part to stand for one pad of it, what counts as a large body, and how fine the ground is looked at
  when parts are settled (coarser than 0.25 mm costs 0.03 to 0.08; finer gains under 0.01 for four times
  the work).
- **What these boards do not try.** `board_most` and `board_full` matter only with several boards in one
  box (one board here has two). What a part too tall for its ground counts for, how often roads are widened,
  and how often the parts of a sheet are coloured made no difference to any plan. The length a chain is
  followed for and what makes a loop tight are measures told of a plan and move no part.
- **No variant put one body over another**, on any board, either way; four made the plan of one board fail
  (a bound worked out twice came out apart by the last digit), which is mended.

The measure is the length of nets against a human placing: it says nothing of whether a plan can be routed,
and little of the criteria a designer weighs beside length. What it does say is that the defaults are not
fitted to one board.
