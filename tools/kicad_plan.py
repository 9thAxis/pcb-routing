#!/usr/bin/env python3
"""The work of a KiCad board as steps, in the order a designer would take them. It reads the board and lays
nothing.

    kicad_plan.py BOARD.kicad_pcb [--nets] [--json FILE] [--hints FILE] [--no-guesses] [--no-pours]

Run it with the Python that ships with KiCad (it needs the `pcbnew` module), as kicad_route.py.

What it says:

- the blocks of the board: its parts by the sheet of the schematic they are drawn on, and the parts that are
  not placed yet (they stand off the board);
- the steps, each with its nets, what it has still to join and the ground its pads span: first the nets within
  one block, block by block; then the pairs, the clocks and what is sensitive between blocks; then the buses;
  then the other nets between blocks; last the supplies and grounds that tracks must join;
- the nets that wait for a part to be placed.

What the nets are for is taken from their names and parts and from the hints file, as kicad_route.py does.
"""
import argparse, dataclasses, json, os, sys
from pathlib import Path

from bridge.decompose import decompose, plan_told
from bridge.engine import Engine, EngineFailed
from bridge.intent import read_hints
from bridge.kicad import KicadBoard, ignored_checks, kicad_is_here, load_board
from bridge.rules import Rules, Settings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("--nets", action="store_true", help="name the nets of each step, and those that wait")
    parser.add_argument("--json", help="write the plan here as JSON")
    parser.add_argument("--hints", help="the hints file (default: BOARD.routing.json beside the board, if there)")
    parser.add_argument("--no-guesses", action="store_true", help="take nothing from the names of nets and parts")
    parser.add_argument("--no-pours", action="store_true", help="what a zone joins does not count as joined")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"),
                        help="the pcb-routing program, for what the zones join (default: $PCB_ROUTING, else on the path)")
    args = parser.parse_args(argv)
    if not kicad_is_here():
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")
    if not Path(args.board).is_file():
        parser.error(f"no such board: {args.board}")  # (pcbnew crashes on a file that is not there)
    beside = Path(args.board).with_suffix(".routing.json")
    try:
        hints = read_hints(args.hints) if args.hints else read_hints(beside) if beside.is_file() else None
    except ValueError as error:
        parser.error(str(error))
    settings = Settings(hints=hints, guesses=not args.no_guesses, pours=not args.no_pours)
    engine = Engine(args.engine, 0, None, None)
    try:
        picture = KicadBoard(load_board(args.board), ignored_checks(args.board), engine.ask, settings.pours).read()
    except EngineFailed as failure:
        print(failure, file=sys.stderr)
        return 2
    plan = decompose(picture, Rules.of(picture, settings))
    print("\n".join(plan_told(plan, every=args.nets)))
    if args.json:
        Path(args.json).write_text(json.dumps(dataclasses.asdict(plan), indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
