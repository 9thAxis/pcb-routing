#!/usr/bin/env python3
"""Where the parts of a KiCad board belong together. It reads the board and moves nothing.

    kicad_floorplan.py BOARD.kicad_pcb [--depth N] [--all] [--json FILE] [--hints FILE] [--no-guesses]

Run it with the Python that ships with KiCad, as kicad_route.py.

What it says:

- the zones of the board: sheets of the schematic that are tied to each other by signal nets more than to the
  rest, and zones of such zones, each with its parts, the ground they need, the nets that stay within it and
  where its parts stand now;
- what flows between zones: the nets from one to another. The heaviest flows say which zones are to be
  neighbours, and where a way is to be kept free between them;
- the parts that are tied more to another sheet than to the one they are drawn on (the filter of an input,
  drawn with the input and working with the converter): they are counted with the sheet they are tied to;
- the parts that stand far from the rest of their zone.

Supplies and grounds tie no parts together here: they reach everything.
"""
import argparse, dataclasses, json, os, sys
from pathlib import Path

from bridge.engine import Engine, EngineFailed
from bridge.floorplan import floorplan, floorplan_told
from bridge.intent import read_hints
from bridge.kicad import KicadBoard, ignored_checks, kicad_is_here, load_board
from bridge.rules import Rules, Settings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("--all", action="store_true", help="every flow, misfit and stray, and the parts of each zone")
    parser.add_argument("--depth", type=int, default=2, help="how many levels of zones to tell (default 2)")
    parser.add_argument("--json", help="write the floorplan here as JSON")
    parser.add_argument("--hints", help="the hints file (default: BOARD.routing.json beside the board, if there)")
    parser.add_argument("--no-guesses", action="store_true", help="take nothing from the names of nets and parts")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"))
    args = parser.parse_args(argv)
    if not kicad_is_here():
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")
    if not Path(args.board).is_file():
        parser.error(f"no such board: {args.board}")
    beside = Path(args.board).with_suffix(".routing.json")
    try:
        hints = read_hints(args.hints) if args.hints else read_hints(beside) if beside.is_file() else None
    except ValueError as error:
        parser.error(str(error))
    settings = Settings(hints=hints, guesses=not args.no_guesses)
    try:
        picture = KicadBoard(load_board(args.board), ignored_checks(args.board), Engine(args.engine, 0, None, None).ask,
                             settings.pours).read()
    except EngineFailed as failure:
        print(failure, file=sys.stderr)
        return 2
    plan = floorplan(picture, Rules.of(picture, settings))
    print("\n".join(floorplan_told(plan, args.depth, every=args.all)))
    if args.json:
        Path(args.json).write_text(json.dumps(dataclasses.asdict(plan), indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
