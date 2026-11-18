#!/usr/bin/env python3
"""Place the parts of a KiCad board as its floorplan says, on a copy. The board given is never written.

    kicad_place.py BOARD.kicad_pcb -o PLACED.kicad_pcb [--hints FILE] [--json FILE] [--no-guesses]

Run it with the Python that ships with KiCad, as kicad_floorplan.py. It asks the engine for the floorplan (see
kicad_floorplan.py for what that is), stands every part where the plan gives it a place, turned as the plan
says, and saves the result as PLACED. Then it reads PLACED back and says how the parts stand there.

What it does not do: it lays no track and moves none. Tracks and zones that were laid to the parts where they
stood are left where they are and no longer reach them: place a board before it is routed, or take its tracks
out first. A part the plan found no room for is left where it stood and named.
"""
import argparse, json, os, sys
from pathlib import Path

from bridge.engine import Engine, EngineFailed
from bridge.floorplan import bodies, floorplan
from bridge.intent import read_hints
from bridge.judged import judged
from bridge.kicad import KicadBoard, ignored_checks, kicad_is_here, load_board, move_parts, save_board
from bridge.placing import moves
from bridge.rules import Rules, Settings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("-o", "--output", required=True, help="the copy to write: not the board itself")
    parser.add_argument("--json", help="write the floorplan here as JSON")
    parser.add_argument("--hints", help="the hints file (default: BOARD.routing.json beside the board, if there)")
    parser.add_argument("--no-guesses", action="store_true", help="take nothing from the names of nets and parts")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"))
    args = parser.parse_args(argv)
    if not kicad_is_here():
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")
    if not Path(args.board).is_file():
        parser.error(f"no such board: {args.board}")
    if Path(args.output).resolve() == Path(args.board).resolve():
        parser.error("the copy is the board itself: name another file to write")
    beside = Path(args.board).with_suffix(".routing.json")
    try:
        hints = read_hints(args.hints) if args.hints else read_hints(beside) if beside.is_file() else None
    except ValueError as error:
        parser.error(str(error))
    settings = Settings(hints=hints, guesses=not args.no_guesses)
    engine = Engine(args.engine, 0, None, None)
    try:
        board = load_board(args.board)
        picture = KicadBoard(board, ignored_checks(args.board), engine.ask, settings.pours).read()
        found = floorplan(picture, Rules.of(picture, settings), engine.ask)
    except EngineFailed as failure:
        print(failure, file=sys.stderr)
        return 2
    if not isinstance(found, dict) or "places" not in found:
        print("the engine gave no floorplan", file=sys.stderr)
        return 2
    asked, kept = moves(picture, found)
    moved = move_parts(board, asked)
    save_board(board, args.output, fill=False)
    if args.json:
        Path(args.json).write_text(json.dumps(found, indent=1))
    # Read the copy back: each part's body is to stand where the plan put it.
    again = KicadBoard(load_board(args.output), ignored_checks(args.board), engine.ask, settings.pours).read()
    stands = {reference: [(box[0] + box[2]) / 2, (box[1] + box[3]) / 2] for reference, box in bodies(again).items()}
    wanted = {place["part"]: place["at"] for place in found["places"] if place["fits"]}
    off = sorted(reference for reference, at in wanted.items()
                 if reference in stands and max(abs(stands[reference][k] - at[k]) for k in (0, 1)) > 0.02)
    row = judged(found)
    print(f"{len(moved)} of {len(picture.parts)} parts moved; {len(wanted) - len(moved)} stand where the plan has them already")
    print(f"written to {args.output}; read back: {len(wanted) - len(off)} parts stand where the plan says"
          + (f", {len(off)} do not: {', '.join(off[:12])}{' ...' if len(off) > 12 else ''}" if off else ""))
    if kept:
        print(f"left where they stood (the plan found them no room): {', '.join(kept)}")
    if picture.tracks:
        print(f"the board has {len(picture.tracks)} tracks: they are where they were, and no longer reach the parts that moved")
    print(f"the plan: {row['parts']} parts, {len(row['no_room'])} without room, {row['over']} bodies over one another; its "
          f"signal nets {row['then_mm']:.0f} mm for {row['now_mm']:.0f} as the board stood")
    return 1 if off else 0


if __name__ == "__main__":
    sys.exit(main())
