#!/usr/bin/env python3
"""Compare routed KiCad boards with a reference routing of the same board, in one command.

    quality.py REFERENCE.kicad_pcb NAME=ROUTED.kicad_pcb [NAME=ROUTED.kicad_pcb ...]
               [--project FILE.kicad_pro] [--dumps DIR] [--picture OUT.png] [--json FILE]
               [--assume-complete] [--clearance MM]

It dumps each board with dump.py under KiCad's Python, then scores the dumps as score.py does and prints its
tables. Run it with a Python 3 that has shapely and numpy (and matplotlib for --picture). KiCad's Python is
$KICAD_PYTHON if set, else the first of these that can import pcbnew: the one inside KiCad.app on macOS,
/usr/bin/python3, python3 on the path, and the Python running this.

All boards must be the same board: the same parts in the same places, routed differently. The reference is
usually the designer's own routing. --project names the project file whose net classes give the clearance and
class widths, when the reference board has none beside it (without it, those of the first routed board that has
a project are used). --dumps keeps the dumps (REFERENCE.json, NAME.json) in a directory, for score.py or
show.py later; otherwise they are temporary.

Exit status: as score.py (0 compared, 1 not: a routing is incomplete or not of the reference's board), 2 if a
board could not be dumped.
"""
import argparse, os, shutil, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
MACOS_PYTHON = "/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3"


def has_pcbnew(python):
    """Whether this Python can import KiCad's pcbnew module."""
    try:
        return subprocess.run([python, "-c", "import pcbnew"], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL).returncode == 0
    except OSError:
        return False


def kicad_python(environ=None, works=has_pcbnew):
    """The Python to run dump.py with; None if none of the usual ones has pcbnew. $KICAD_PYTHON, when set, is
    the only one tried."""
    environ = os.environ if environ is None else environ
    if environ.get("KICAD_PYTHON"):
        return environ["KICAD_PYTHON"] if works(environ["KICAD_PYTHON"]) else None
    candidates = [MACOS_PYTHON, "/usr/bin/python3", shutil.which("python3"), sys.executable]
    return next((p for p in candidates if p and works(p)), None)


def dump_command(python, board, output, project=None):
    """The command that dumps one board."""
    return [python, str(HERE / "dump.py"), str(board), "-o",
            str(output)] + (["--project", str(project)] if project else [])


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("reference", metavar="REFERENCE.kicad_pcb")
    parser.add_argument("routed", nargs="+", metavar="NAME=ROUTED.kicad_pcb")
    parser.add_argument("--project",
                        help="the .kicad_pro to read the reference's net classes from (default: the one beside it)")
    parser.add_argument("--dumps", help="keep the dumps in this directory")
    parser.add_argument("--picture", metavar="OUT.png", help="also draw the routings side by side (needs matplotlib)")
    parser.add_argument("--json", metavar="FILE",
                        help="write the figures as JSON here; - for standard output in place of the tables")
    parser.add_argument("--assume-complete", action="store_true",
                        help="take every routing as complete, whatever KiCad counts as open")
    parser.add_argument("--clearance", type=float,
                        help="the clearance in mm for the spacing figures (default: the Default class's)")
    args = parser.parse_args(argv)
    if not all("=" in r for r in args.routed):
        parser.error("give each routed board as NAME=ROUTED.kicad_pcb")
    named = [r.split("=", 1) for r in args.routed]
    names = ["reference"] + [name for name, _ in named]
    if len(set(names)) != len(names) or any(Path(name).name != name for name in names):
        parser.error("the routings need different names, usable as file names (and not 'reference')")
    python = kicad_python()
    if python is None:
        parser.error("no Python with KiCad's pcbnew module found: set KICAD_PYTHON to the Python that ships with KiCad")

    import score  # here and not at the top: the checks above need neither shapely nor numpy

    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(args.dumps) if args.dumps else Path(temporary)
        directory.mkdir(parents=True, exist_ok=True)
        dumps = [(name, directory / f"{name}.json") for name in names]
        for (name, output), board in zip(dumps, [args.reference] + [path for _, path in named]):
            done = subprocess.run(dump_command(python, board, output, args.project if name == "reference" else None),
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            if done.returncode != 0 or not output.exists():
                print(f"could not dump {board}:\n{done.stderr}", file=sys.stderr)
                return 2
        if args.picture:
            import show
            show.picture([(name, score.load(path)) for name, path in dumps], args.picture)
        options = (["--json", args.json] if args.json else []) + (["--assume-complete"] if args.assume_complete else [])
        options += ["--clearance", str(args.clearance)] if args.clearance else []
        return score.main([str(dumps[0][1])] + [f"{name}={path}" for name, path in dumps[1:]] + options)


if __name__ == "__main__":
    sys.exit(main())
