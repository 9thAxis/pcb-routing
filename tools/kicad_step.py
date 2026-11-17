#!/usr/bin/env python3
"""One step of a KiCad board's work (see kicad_plan.py): tried on each set of layers that makes sense for it,
the best try kept, checked, and saved as a new board. Then it stops: the next step is another run, on the
board this one saved.

    kicad_step.py BOARD.kicad_pcb -o NEXT.kicad_pcb --step N|NAME [--layers F.Cu,B.Cu] [--json FILE]
                  [--hints FILE] [--no-guesses] [--no-check]

Run it with the Python that ships with KiCad, as kicad_route.py. `--step` takes the number kicad_plan.py gives
the step on this board, or its name (or a part of it that names one step only).

What it does:

1. Works out the plan of the board as it stands and takes the step named.
2. Tries the step on its pads' own layers, on those with each other signal layer, and on every signal layer
   (--layers: on those layers only). An inner layer that zones cover is a plane and carries no routes. Each
   try routes the step's nets alone, on the board as it stands, with everything on it kept.
3. Says of each try what it laid: the nets left unjoined, the track in mm and how many times the least it
   could be, the vias, and for a pair how much of it runs side by side. The best is kept: the fewest nets
   left, then the least track, a via counted as 10 mm and a pair's track that runs alone counted twice more.
4. Saves the board with that try, and checks it with KiCad's design rule check against the board it began
   from (--no-check: not). Errors the board did not have, or a net left unjoined, are said.

Exit status: 0 the step is all joined with no new error, 1 not, 2 the engine failed or the step is not there.
"""
import argparse, collections, contextlib, io, json, os, shutil, subprocess, sys, tempfile
from dataclasses import replace
from pathlib import Path

from bridge.decompose import SIDE_BY_SIDE, decompose, plan_told
from bridge.engine import Engine, EngineFailed
from bridge.intent import read_hints
from bridge.kicad import KicadBoard, ignored_checks, kicad_is_here, load_board, save_board
from bridge.route import route
from bridge.rules import Rules, Settings
from bridge.steps import figures, habits, layer_options, planes, weight

KICAD_CLI = ["kicad-cli", "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"]


def errors_of(board):
    """What KiCad's design rule check finds wrong with a board, as a count of each kind of error; None if the
    check cannot be run."""
    program = next((p for p in KICAD_CLI if shutil.which(p)), None)
    if program is None:
        return None
    with tempfile.TemporaryDirectory() as tmp:
        report = Path(tmp) / "drc.json"
        subprocess.run([program, "pcb", "drc", "--severity-error", "--format", "json", "--refill-zones", "-o", str(report),
                        str(board)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            return collections.Counter(v["type"] for v in json.loads(report.read_text())["violations"])
        except (OSError, ValueError, KeyError):
            return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("-o", "--output", required=True, help="the board to save, with the step laid")
    parser.add_argument("--step", required=True, help="the step: its number in the plan of this board, or its name")
    parser.add_argument("--layers", help="try the step on these layers only")
    parser.add_argument("--json", help="write what every try came to here")
    parser.add_argument("--hints", help="the hints file (default: BOARD.routing.json beside the board, if there)")
    parser.add_argument("--no-guesses", action="store_true", help="take nothing from the names of nets and parts")
    parser.add_argument("--no-check", action="store_true", help="do not run KiCad's design rule check")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"))
    parser.add_argument("--threads", type=int, default=0)
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
    engine = Engine(args.engine, args.threads, None, None)
    ignored = ignored_checks(args.board)

    def opened():
        board = KicadBoard(load_board(args.board), ignored, engine.ask, settings.pours)
        return board, board.read()
    try:
        _, picture = opened()
    except EngineFailed as failure:
        print(failure, file=sys.stderr)
        return 2
    rules = Rules.of(picture, settings)
    plan = decompose(picture, rules)
    named = ([plan.steps[int(args.step) - 1]] if args.step.isdigit() and 0 < int(args.step) <= len(plan.steps)
             else [step for step in plan.steps if step.name == args.step]
             or [step for step in plan.steps if args.step in step.name])
    if len(named) != 1:
        print(f"{args.step} names {'no step' if not named else 'more than one step'} of this board:", file=sys.stderr)
        print("\n".join(plan_told(plan)), file=sys.stderr)
        return 2
    step = named[0]
    pairs = [(a, rules.intent(a).pair) for a in step.nets
             if isinstance(rules.intent(a).pair, str) and rules.intent(a).pair in step.nets and a < rules.intent(a).pair]
    print(f"step: {step.name} ({step.kind}): {', '.join(step.nets)}; {step.open} connections, {step.span:.0f} mm at the least")
    flat = planes(picture)
    if flat:
        print(f"planes, which carry no routes: {', '.join(flat)}")
    for layer, (mm, across, up) in sorted(habits(picture).items(), key=lambda item: picture.layers.index(item[0])):
        if layer not in flat and max(across, up) >= 0.7 and mm > 100:
            print(f"the tracks on {layer} run {'across' if across > up else 'up and down'} the board "
                  f"({100 * max(across, up):.0f} % of {mm:.0f} mm)")
    options = [("as asked", [l for l in picture.layers if l in args.layers.split(",")])] if args.layers else layer_options(picture, step)

    tries, best = [], None
    for name, layers in options:
        try:
            board, said = KicadBoard(load_board(args.board), ignored, engine.ask, settings.pours), []
            with contextlib.redirect_stdout(io.StringIO()):  # (the engine tells of each run: not here)
                outcome = route(board, replace(settings, layers=set(layers)), engine, only=set(step.nets), say=said.append)
                found = figures(picture, board.read(), step, pairs)
        except EngineFailed as failure:
            print(f"  {name}: {failure}")
            continue
        found.update(option=name, tried_on=layers, failure=outcome.failure,
                     notes=[line for line in said if "two nets" in line or "other way round" in line])
        tries.append(found)
        side = "".join(f", {pair} side by side for {100 * share:.0f} %" for pair, share in found["coupled"].items())
        left = f"{len(found['left'])} nets left unjoined ({', '.join(found['left'])}); " if found["left"] else ""
        print(f"  {name} ({', '.join(layers)}): {left}{found['mm']:.1f} mm, {found['detour']} times the least, "
              f"{found['vias']} vias, on {', '.join(found['layers']) or 'nothing'}{side}")
        if best is None or weight(found) < weight(best[0]):
            best = (found, board)
    if args.json:
        Path(args.json).write_text(json.dumps(dict(step=step.name, nets=step.nets, tries=tries), indent=1))
    if best is None:
        print("no try gave anything", file=sys.stderr)
        return 2
    found, board = best
    for line in found["notes"]:
        print(f"  {line}")
    save_board(board.board, args.output)
    for suffix in (".kicad_pro", ".kicad_dru"):  # (the same rules for the check, and for whoever opens it)
        source, target = Path(args.board).with_suffix(suffix), Path(args.output).with_suffix(suffix)
        if source.is_file() and source.resolve() != target.resolve():
            shutil.copy2(source, target)
    print(f"kept: {found['option']}; saved {args.output}")
    new = None
    if not args.no_check:
        was, now = errors_of(args.board), errors_of(args.output)
        if was is None or now is None:
            print("not checked: KiCad's design rule check could not be run (kicad-cli)")
        else:
            new = dict(now - was)
            print("KiCad's check: no new error" if not new else
                  "KiCad's check: new errors: " + ", ".join(f"{count} {kind}" for kind, count in sorted(new.items())))
    return 0 if not found["left"] and not new and not found["failure"] else 1


if __name__ == "__main__":
    sys.exit(main())
