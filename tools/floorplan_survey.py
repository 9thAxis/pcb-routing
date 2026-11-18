#!/usr/bin/env python3
"""The floorplan of many KiCad boards, judged: does the planner place every part of each, none over another, and
how do its nets compare with the boards as their designers placed them? It reads the boards and moves nothing.

    floorplan_survey.py BOARD.kicad_pcb ... [--json DIR] [--no-guesses]

Run it with the Python that ships with KiCad, as kicad_floorplan.py. Each board's hints (BOARD.routing.json beside
it) are used if there. With --json, each board's whole answer is written to DIR/NAME.json.
"""
import argparse, json, os, sys
from pathlib import Path

from bridge.engine import Engine, EngineFailed
from bridge.floorplan import floorplan
from bridge.intent import read_hints
from bridge.judged import judged, judged_told
from bridge.kicad import KicadBoard, ignored_checks, kicad_is_here, load_board
from bridge.rules import Rules, Settings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("boards", nargs="+")
    parser.add_argument("--json", help="write each board's floorplan to this directory")
    parser.add_argument("--no-guesses", action="store_true", help="take nothing from the names of nets and parts")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"))
    args = parser.parse_args(argv)
    if not kicad_is_here():
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")
    rows = {}
    for path in args.boards:
        name = Path(path).name.removesuffix(".kicad_pcb")
        rows[name] = None
        if not Path(path).is_file():
            print(f"no such board: {path}", file=sys.stderr)
            continue
        beside = Path(path).with_suffix(".routing.json")
        try:
            settings = Settings(hints=read_hints(beside) if beside.is_file() else None, guesses=not args.no_guesses)
            engine = Engine(args.engine, 0, None, None)
            picture = KicadBoard(load_board(path), ignored_checks(path), engine.ask, settings.pours).read()
            found = floorplan(picture, Rules.of(picture, settings), engine.ask)
        except (EngineFailed, ValueError) as failure:
            print(f"{name}: {failure}", file=sys.stderr)
            continue
        if not isinstance(found, dict) or "places" not in found:
            print(f"{name}: no floorplan", file=sys.stderr)
            continue
        rows[name] = judged(found)
        if args.json:
            Path(args.json).mkdir(parents=True, exist_ok=True)
            Path(args.json, f"{name}.json").write_text(json.dumps(found, indent=1))
    print("\n".join(judged_told(rows)))
    return 0 if all(rows.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
