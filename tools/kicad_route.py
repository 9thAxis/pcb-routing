#!/usr/bin/env python3
"""Route the unconnected nets of a KiCad board with pcb-routing.

    kicad_route.py BOARD.kicad_pcb -o ROUTED.kicad_pcb [--nets A,B] [--step auto|MM] [--variants N]
                   [--layers F.Cu,B.Cu] [--problem FILE] [--solution FILE] [--engine PATH]

Run it with the Python that ships with KiCad (it needs the `pcbnew` module), for example on macOS:

    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 kicad_route.py ...

What it does:

1. Reads the board: pads, tracks, vias, drilled holes, drawings on copper, the outline and the rule areas become
   the fixed picture; every net with pads that are not yet joined becomes one entry, with one terminal per group
   of joined pads. Widths, via sizes and clearances come from each net's class and the board's minima.
2. Runs the `pcb-routing` program on that problem. Nets it could not join are tried once more against the board
   as it then stands, with a track narrower than the net's narrowest pad. The engine takes fewer than 250 nets
   in one run: a board with more is routed in batches of at most 200, each against the board as the batch
   before left it.
3. Adds the tracks and vias found to the board, fills the zones again, and saves it under the new name.

Zones: the zones are filled first, and pads that a filled zone joins count as joined (--no-pours: only tracks,
vias and pads that touch do, and a net meant to be joined by a zone is routed as tracks).

- A net with a zone gets tracks only from the pads its zone does not reach, each to a pad of the net or to a
  point of the zone, whichever comes cheapest.
- A track of another net pays for its length across a zone (--pour-cost), and more where it would cut the
  spokes that tie a pad to its zone (--spoke-cost): it goes round where that is not far.
- After each run the zones are filled again. A track that ended on a piece of zone the other new tracks cut
  away is taken back, and pads that the new tracks parted from their zone are joined again. Where that cannot
  be done past the tracks that did it, the run is taken back and done again with those pads joined by a track.
- At the end every pad that is tied to its zone by fewer spokes than the board asks for (counted as KiCad's
  design rule check counts them) gets a straight track from its middle into the zone, which counts as a
  spoke. Where no such track fits by because new tracks of other nets pass close by the pad, those nets are
  laid again clear of it (and put back as they were if they cannot be). A pad still left with too few is named.

What the nets are for: taken from their names and parts, and from a hints file beside the board
(BOARD.routing.json, or --hints FILE; see docs/hints.md). Two nets named as the halves of a differential pair
are laid side by side where their pads stand side by side at both ends, and as two nets where that cannot be
done; clocks and crystals are slow to change layer; widths for a current, clearances for a voltage, layers,
"no vias" and the like come from the hints. What was taken from where is said at the start.

Run KiCad's design rule check on the result before using it.

What it does not do:

- It routes on a grid, with tracks at multiples of 45 degrees and through vias only (no blind, buried or micro
  vias, no arcs).
- No length tuning, and no impedance: a pair is laid at the width and gap it is told, or at its class's width
  and the clearance.

Exit status: 0 every net joined, 1 some nets left unjoined, 2 the engine failed (what it had laid before that
is saved all the same).
"""
import argparse, os, sys
from pathlib import Path

from bridge.intent import read_hints
from bridge.rules import Settings, ignored_checks
from bridge.engine import Engine, EngineFailed
from bridge.route import route
from bridge.kicad import KicadBoard, kicad_is_here, load_board, save_board


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("-o", "--output", required=True)
    parser.add_argument("--nets", help="only these nets, comma separated (default: every net with pads not yet joined)")
    parser.add_argument("--layers", help="copper layers that may carry routes, comma separated (default: all)")
    parser.add_argument("--step", default="auto", help="grid step in mm, or 'auto' (the default): chosen from the track widths and clearances of the nets")
    parser.add_argument("--coarse-first", action="store_true", help="a first pass on a grid twice as coarse, then what is left on the fine one: about three times faster, a little more track and vias, and less room on a crowded board")
    parser.add_argument("--slack", type=float, default=0.002, help="added to every clearance for the grid, in mm")
    parser.add_argument("--variants", type=int, default=1, help="differently ordered attempts run side by side")
    parser.add_argument("--via-penalty", type=float, default=10.0, help="length of detour accepted before a via is preferred, in mm")
    parser.add_argument("--no-pours", action="store_true", help="do not count pads that a filled zone joins as joined: a net with a zone is routed as tracks, as if the zone were not there")
    parser.add_argument("--pour-cost", type=float, default=2.0, help="what a track pays for its length across another net's pour, as a factor (1: nothing more)")
    parser.add_argument("--spoke-cost", type=float, default=4.0, help="and across the ground where it would cut a spoke that ties a pad to its pour")
    parser.add_argument("--no-pairs", action="store_true", help="do not lay two nets named as the halves of a differential pair (_P/_N, +/-, DP/DM) side by side")
    parser.add_argument("--hints", help="what the nets are for, as a JSON file (default: BOARD.routing.json beside the board, if it is there; see read_hints)")
    parser.add_argument("--no-guesses", action="store_true", help="do not guess what a net is for from its name and parts (clocks, crystals, switch nodes, pairs): only the hints say")
    parser.add_argument("--no-decoupling", action="store_true", help="do not join a capacitor that stands by a pin of an IC to that pin itself")
    parser.add_argument("--bus-pull", type=float, default=0.2, help="how much longer a net that leaves and arrives beside others may run to stay beside them, as a share of its length (0: buses are not kept together)")
    parser.add_argument("--max-states", type=int, default=40_000_000, help="largest grid accepted, in cells times layers (memory grows with it and with the number of nets)")
    parser.add_argument("--problem", help="keep the problem file here (of the first run of the engine)")
    parser.add_argument("--solution", help="keep the solution file here (of the first run of the engine)")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"), help="the pcb-routing program (default: $PCB_ROUTING, else pcb-routing on the path)")
    parser.add_argument("--threads", type=int, default=0, help="threads the engine may use (default: as many as the machine has)")
    args = parser.parse_args(argv)
    if not kicad_is_here():
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")

    names = lambda text: set(text.split(",")) if text else None
    try:
        step = None if args.step == "auto" else float(args.step)
    except ValueError:
        parser.error("--step takes a number of mm, or auto")
    if not 0 <= args.bus_pull <= 10:
        parser.error("--bus-pull takes a number from 0 to 10")
    if not (1 <= args.pour_cost <= 100 and 1 <= args.spoke_cost <= 100):
        parser.error("--pour-cost and --spoke-cost take a number from 1 to 100")
    beside = Path(args.board).with_suffix(".routing.json")
    try:
        hints = read_hints(args.hints) if args.hints else read_hints(beside) if beside.is_file() else None
    except ValueError as error:
        parser.error(str(error))
    settings = Settings(hints=hints, guesses=not args.no_guesses, step=step, coarse_first=args.coarse_first, slack=args.slack, via_penalty=args.via_penalty, layers=names(args.layers), bus_pull=args.bus_pull, decoupling=not args.no_decoupling, pours=not args.no_pours, pairs=not args.no_pairs,
                        pour_cost=args.pour_cost, spoke_cost=args.spoke_cost)
    options = dict(variants=args.variants, max_states=args.max_states)
    engine = Engine(args.engine, args.threads, args.problem, args.solution)

    if not Path(args.board).is_file():
        parser.error(f"no such board: {args.board}")  # (pcbnew crashes on a file that is not there)
    board = load_board(args.board)
    try:
        outcome = route(KicadBoard(board, ignored_checks(args.board), engine.ask, settings.pours), settings, engine,
                        only=names(args.nets), options=options)
    except EngineFailed as failure:  # (a question about the zones that got no answer: nothing is saved)
        print(failure, file=sys.stderr)
        return 2

    if outcome.failure:
        print(outcome.failure, file=sys.stderr)
        if not outcome.laid:
            return 2  # nothing laid: nothing to save
    if outcome.wanted == 0:
        print("nothing to route: every net's pads are joined")
        save_board(board, args.output, fill=False)
        return 0
    save_board(board, args.output)
    joined = f"{outcome.wanted - outcome.left} of {outcome.wanted} nets joined, {outcome.tracks} track segments and {outcome.vias} vias added; saved {args.output}"
    if outcome.failure:
        print(f"the engine failed; kept what it had laid before: {joined}")
        return 2
    print(f"{outcome.status}: {joined}")
    return 0 if outcome.status == "routed" else 1


if __name__ == "__main__":
    sys.exit(main())
