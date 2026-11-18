"""Each constant of the floorplan halved and doubled, the boards planned again, and what that comes to told.

    constants_sweep.py COPY OUT NAME=DIR [NAME=DIR ...] [--only CONSTANT ...]

COPY: a copy of this repository that may be written to (`git archive HEAD | tar -x -C COPY`): its
src/floorplan/constants.rs is rewritten for each variant, built into COPY/target, and put back at the end.
OUT: a directory for results.jsonl (a run that is stopped goes on from there). NAME=DIR: a set of floorplan
questions as JSON files, as tools/floorplan_survey.py --json or the bridge asks them (free=..., held=...).

For each set and variant: the questions that failed, the bodies over one another (other than two fixed parts,
which stand as their designer put them), and over the boards of 100 parts or more the geometric mean and the
worst of the plan's nets against the board as it stands, the parts without room, the capacitors and the parts
for one pad that are by their pad, the chains that double back, the pairs too near; over the smaller boards
the nets in all and the parts without room. See docs/floorplan-tuning.md for what the last run came to."""
import argparse, glob, json, math, os, re, subprocess, time
from concurrent.futures import ThreadPoolExecutor

parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
parser.add_argument("copy"); parser.add_argument("out"); parser.add_argument("sets", nargs="+", metavar="NAME=DIR")
parser.add_argument("--only", nargs="*", default=[])
args = parser.parse_args()
S = os.path.abspath(args.out); os.makedirs(S, exist_ok=True)
COPY = os.path.abspath(args.copy)
CONST = f"{COPY}/src/floorplan/constants.rs"
ORIG = open(CONST).read()
ENGINE = f"{COPY}/target/release/pcb-routing"
SETS = {said.split("=", 1)[0]: sorted(glob.glob(said.split("=", 1)[1] + "/*.json")) for said in args.sets}
def board(q):
    if isinstance(q, dict):
        if isinstance(q.get("parts"), list) and q["parts"] and isinstance(q["parts"][0], dict) and "area" in q["parts"][0]: return q
        for v in q.values():
            r = board(v)
            if r: return r
FIXED, BIG = {}, set()
for s, qs in SETS.items():
    for p in qs:
        parts = board(json.load(open(p)))["parts"]
        FIXED[(s, os.path.basename(p))] = {x["name"] for x in parts if x.get("fixed")}
        if len(parts) >= 100:
            BIG.add(os.path.basename(p))
def run(job):
    s, q = job
    try:
        out = subprocess.run([ENGINE, "ask", "--threads", "3"], stdin=open(q), capture_output=True, timeout=300)
        return s, os.path.basename(q), json.loads(out.stdout) if out.returncode == 0 else None
    except Exception:
        return s, os.path.basename(q), None
def measured():
    with ThreadPoolExecutor(6) as pool:
        got = list(pool.map(run, [(s, q) for s, qs in SETS.items() for q in qs]))
    row = {}
    for s in SETS:
        mine = [(n, f) for t, n, f in got if t == s]
        fails = sum(1 for _, f in mine if f is None); mine = [(n, f) for n, f in mine if f]
        over = 0
        for n, f in mine:
            fx = FIXED[(s, n)]
            rects = [(r, x.get("back", False), x["part"]) for x in f["places"] if x["rect"] and x["fits"] for r in (x.get("pieces") or [x["rect"]])]
            rects.sort(key=lambda t: t[0][0])
            for i, (a, sa, pa) in enumerate(rects):
                for b, sb, pb in rects[i + 1:]:
                    if b[0] >= a[2] - 0.05: break
                    if pa != pb and sa == sb and min(a[3], b[3]) - max(a[1], b[1]) > 0.05 and not (pa in fx and pb in fx): over += 1
        big = [f for n, f in mine if n in BIG]; small = [f for n, f in mine if n not in BIG]
        ratio = [f["parts_then_mm"] / f["parts_now_mm"] for f in big if f["parts_now_mm"] > 0]
        frac = lambda key: sum(f[key][1] for f in big) / max(1, sum(f[key][0] for f in big))
        row.update({f"{s}_fails": fails, f"{s}_over": over,
                    f"{s}_geo": round(math.exp(sum(map(math.log, ratio)) / max(1, len(ratio))), 3) if ratio else None,
                    f"{s}_worst": round(max(ratio), 2) if ratio else None,
                    f"{s}_noroom": sum(1 for f in big for x in f["places"] if not x["fits"]),
                    f"{s}_pub": round(sum(f["parts_then_mm"] for f in small) / max(1, sum(f["parts_now_mm"] for f in small)), 3),
                    f"{s}_pubnoroom": sum(1 for f in small for x in f["places"] if not x["fits"]),
                    f"{s}_bypass": round(frac("bypass"), 3), f"{s}_served": round(frac("served"), 3),
                    f"{s}_back": sum(len(f["back"]) for f in big), f"{s}_close": sum(len(f["close"]) for f in big),
                    f"{s}_roadsover": sum(f["travel"]["over"] for f in big)})
    return row
consts = re.findall(r"pub\(super\) const (\w+): (f64|usize) = ([0-9.]+);", ORIG)
def variants(name, kind, value):
    v = float(value)
    if kind == "usize": return sorted({max(1, int(v) // 2), int(v) * 2} - {int(v)})
    if v < 1.0: return [v * 0.5, min(0.95, v * 1.5)]
    return [v * 0.5, v * 2.0]
def build(text):
    open(CONST, "w").write(text)
    out = subprocess.run(["cargo", "build", "--release"], cwd=COPY, env={**os.environ, "CARGO_TARGET_DIR": f"{COPY}/target"}, capture_output=True)
    return out.returncode == 0
done = {}
if os.path.exists(f"{S}/results.jsonl"):
    for line in open(f"{S}/results.jsonl"): r = json.loads(line); done[(r["name"], r["value"])] = r
todo = [("base", "", "")] + [(n, k, v) for n, k, v in consts]
only = set(args.only)
with open(f"{S}/results.jsonl", "a") as log:
    for name, kind, value in todo:
        if only and name not in only and name != "base": continue
        for new in ([""] if name == "base" else variants(name, kind, value)):
            shown = "" if name == "base" else (str(new) if kind == "usize" else f"{new:g}" + ("" if "." in f"{new:g}" else ".0"))
            if (name, shown) in done: continue
            text = ORIG if name == "base" else re.sub(rf"(const {name}: {kind} = )[0-9.]+;", rf"\g<1>{shown};", ORIG)
            start = time.time()
            row = {"name": name, "value": shown, "was": value}
            if build(text): row.update(measured())
            else: row["build"] = "failed"
            row["secs"] = round(time.time() - start)
            log.write(json.dumps(row) + "\n"); log.flush()
            print(row, flush=True)
open(CONST, "w").write(ORIG)
