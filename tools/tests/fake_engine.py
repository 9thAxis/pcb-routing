#!/usr/bin/env python3
"""A stand-in for the pcb-routing program, for the bridge's tests.

    fake_engine.py route PROBLEM -o SOLUTION [--threads N]
    fake_engine.py pours

It joins the terminals of every net by one straight track on the first terminal's first layer. What it does
otherwise is told by files in the directory named by FAKE_ENGINE_DIR:

    runs/N.json     written: the problem of run N (from 1)
    stubborn.json   {"nets": [...], "below": W}: these nets are left unrouted unless their width is under W
    fail_on         a run number: that run writes no solution and ends with status 2

A question about zones (`pours`) is the program's own to answer: it is passed on to $PCB_ROUTING, else to the
repository's release build.
"""
import json, os, sys
from pathlib import Path

args = sys.argv[1:]
if args[:1] == ["pours"]:
    program = os.environ.get("PCB_ROUTING") or str(Path(__file__).resolve().parents[2] / "target/release/pcb-routing")
    os.execv(program, [program] + args)
assert args[0] == "route" and args[2] == "-o", args
problem = json.loads(Path(args[1]).read_text())
control = Path(os.environ["FAKE_ENGINE_DIR"])
runs = control / "runs"
runs.mkdir(exist_ok=True)
number = len(list(runs.iterdir())) + 1
(runs / f"{number}.json").write_text(json.dumps(problem))

fail_on = control / "fail_on"
if fail_on.exists() and int(fail_on.read_text()) == number:
    print("invalid problem: the fake engine was told to fail", file=sys.stderr)
    sys.exit(2)

stubborn = json.loads((control / "stubborn.json").read_text()) if (control / "stubborn.json").exists() else {"nets": [], "below": 0}
nets, unrouted = [], []
for net in problem["nets"]:
    if net["key"] in stubborn["nets"] and net["width"] >= stubborn["below"]:
        unrouted.append(f"{net['key']}: no path")
        nets.append(dict(key=net["key"], net=net["net"], length_mm=0.0, tracks=[], vias=[]))
        continue
    points = [t["xy"] for t in net["terminals"]]
    nets.append(dict(key=net["key"], net=net["net"], length_mm=1.0, tracks=[dict(layer=net["terminals"][0]["layers"][0], points=points)], vias=[]))
status = "unroutable" if unrouted else "routed"
solution = dict(status=status, iterations=1, variant=0, score=None, nets=nets, conflicts=[], message=unrouted[0] if unrouted else "", unrouted=unrouted)
Path(args[3]).write_text(json.dumps(solution))
print(f"{status}: fake run {number}, {len(problem['nets'])} nets")
sys.exit(1 if unrouted else 0)
