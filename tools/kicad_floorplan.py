#!/usr/bin/env python3
"""Where the parts of a KiCad board belong together. It reads the board and moves nothing.

    kicad_floorplan.py BOARD.kicad_pcb [--depth N] [--all] [--json FILE] [--hints FILE] [--no-guesses]

Run it with the Python that ships with KiCad, as kicad_route.py.

What it says:

- the power of the board: its grounds, each a domain of the parts that stand on it, with its supplies (a net
  that two or more capacitors bypass to the ground, whatever it is called); the parts that stand on two
  grounds, which belong on the gap between two domains; and the supplies that a part joins;
- for each zone the ground it is given on the board (the board is cut in two again and again, the zones parted
  so that as little as may be is tied across each cut and each has the room its parts need; a connector or a
  switch stays where it stands and pulls its zone to itself), and how far its parts stand from that today;
- within each domain, the zones: sheets of the schematic that are tied to each other by signal nets more than to the
  rest, and zones of such zones, each with its parts, the ground they need, the nets that stay within it and
  where its parts stand now;
- what flows between zones: the nets from one to another. The heaviest flows say which zones are to be
  neighbours, and where a way is to be kept free between them;
- the parts that are tied more to another sheet than to the one they are drawn on (the filter of an input,
  drawn with the input and working with the converter): they are counted with the sheet they are tied to;
- the parts that stand far from the rest of their zone.

Supplies and grounds tie no parts together here: they reach everything.
"""
import argparse, json, os, sys
from pathlib import Path

from bridge.engine import Engine, EngineFailed
from bridge.floorplan import floorplan, floorplan_told, power_told
from bridge.intent import read_hints
from bridge.kicad import KicadBoard, ignored_checks, kicad_is_here, load_board
from bridge.rules import Rules, Settings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("--all", action="store_true", help="every flow, misfit and stray, and the parts of each zone")
    parser.add_argument("--depth", type=int, default=3, help="how many levels of zones to tell (default 3: domains, the zones within them, and theirs)")
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
    engine = Engine(args.engine, 0, None, None)
    try:
        picture = KicadBoard(load_board(args.board), ignored_checks(args.board), engine.ask, settings.pours).read()
        found = floorplan(picture, Rules.of(picture, settings), engine.ask)
    except EngineFailed as failure:
        print(failure, file=sys.stderr)
        return 2
    print("\n".join(power_told(found, every=args.all) + ["", ""] + floorplan_told(found, args.depth, every=args.all)))
    if args.json:
        Path(args.json).write_text(json.dumps(found, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
