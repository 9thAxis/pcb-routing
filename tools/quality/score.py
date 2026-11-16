#!/usr/bin/env python3
"""Compare routings of the same board with a reference routing of it, from the dumps written by dump.py.

    score.py REFERENCE.json NAME=ROUTED.json [NAME=ROUTED.json ...]
    score.py --dir DIR --reference REF NAME [NAME ...]

The first form scores one board. The second scores every board in DIR: a board is every BOARD for which
DIR/BOARD.REF.json exists, and its routings are DIR/BOARD.NAME.json. Needs Python 3 with shapely and numpy.

It prints four tables (general, buses, circuit, pad exits) with one row for the reference and one for each
routing; --json FILE writes the same figures, the boards used and the boards skipped as JSON (FILE - for
standard output in place of the tables). measures.py and README.md define every figure.

Which boards count: a board is used only if every named routing of it is there, has the reference's pads, and
is complete, so that all rows are sums over the same boards and nets. A routing is complete when its dump says
`unconnected: 0`, which is KiCad's own count at the time of the dump. Dumps of older make do not say; state it
yourself with --assume-complete (and choose the boards with --boards if only some are). The reference is not
required to be complete. Skipped boards are named on standard error with the reason.

The clearance for the spacing figures and the class widths for the power figures come from the reference
dump's rules, else from the first routing that has any; --clearance overrides the clearance. Without either the
clearance is 0.2 mm and no power track counts as narrow.

Exit status: 0 with at least one board compared, 1 with none.
"""
import argparse, json, math, sys
from pathlib import Path

import measures as m


# --------------------------------------------------------------------------------------------------------------
# Comparing: dumps in, figures out

def compare(boards, names, assume_complete=False, clearance=None, per_net=False):
    """Score boards. `boards` yields (board name, reference dump, {routing name: dump or None for a missing
    one}); `names` are the reference's name and then the routings', in table order. Returns a dict:
    names, boards (those used), skipped {board: reason}, general, buses, circuit, exits (each with a `rows`
    dict of figures by name, and its counts), and with `per_net` the per-net rows of the general table."""
    reference_name, routing_names = names[0], names[1:]
    totals = {table: {name: m.Tally() for name in names} for table in ("general", "buses", "circuit", "exits")}
    used, skipped, nets_rows = [], {}, []
    general_boards = nets_used = buses = 0
    for board, reference, routings in boards:
        missing = [name for name in routing_names if routings.get(name) is None]
        routings = {name: routings[name] for name in routing_names if name not in missing}
        reason = f"no dump of {', '.join(missing)}" if missing else m.unusable(reference, routings, assume_complete)
        if reason:
            skipped[board] = reason
            continue
        used.append(board)
        dumps = {reference_name: reference, **routings}
        rules = m.board_rules(reference, routings)
        gap = clearance or m.clearance_of(rules)
        poured, tracked = set(reference["zone_nets"]), m.tracked_nets(reference)

        nets = m.compared_nets(reference)
        if nets:
            general_boards += 1
            nets_used += len(nets)
            for name, dump in dumps.items():
                tally, counts = m.general_measures(dump, gap, nets)
                totals["general"][name].add(tally)
                if per_net:
                    nets_rows += [dict(board=board, net=net, routing=name, tree=tree, connections=connections,
                                       **counts[net])
                                  for net, (tree, connections) in nets.items()]
        pairs, groups = m.bus_pairs(reference)
        buses += groups
        circuit = m.circuit_nets(reference)
        for name, dump in dumps.items():
            if pairs:
                totals["buses"][name].add(m.bus_measures(dump, pairs))
            totals["circuit"][name].add(m.circuit_measures(dump, circuit, poured, rules,
                                                           is_reference=name == reference_name))
            totals["exits"][name].add(m.exit_measures(dump, tracked))

    general = totals["general"]
    result = dict(
        names=list(names), boards=used, skipped=skipped,
        general=dict(boards=general_boards, nets=nets_used, connections=general[reference_name]["connections"],
                     rows={name: m.general_figures(general[name], general[reference_name]["mm"]) for name in names}),
        buses=dict(buses=buses, rows={name: m.bus_figures(totals["buses"][name]) for name in names}),
        circuit=dict(rows={name: m.circuit_figures(totals["circuit"][name]) for name in names}),
        exits=dict(rows={name: m.exit_figures(totals["exits"][name]) for name in names}),
    )
    if per_net:
        result["nets"] = nets_rows
    return result


# --------------------------------------------------------------------------------------------------------------
# The tables as text

def tables(result):
    """The four tables of a result of compare, as text."""
    wide = max([12] + [len(name) for name in result["names"]])
    general, buses, circuit, exits = result["general"], result["buses"], result["circuit"], result["exits"]
    out = [f"{len(result['boards'])} boards compared, {len(result['skipped'])} skipped", ""]

    out.append(f"general: {general['boards']} boards, {general['nets']} nets, {general['connections']} connections")
    out.append(f"{'':{wide}} {'mm':>8} {'detour':>7} {'vs ref':>8} {'vias/100':>8} {'corners':>8} {'sharp/100':>9} {'short/100':>9} {'tight %':>8} {'gap':>5}")
    for name, f in general["rows"].items():
        out.append(f"{name:{wide}} {f['mm']:8.0f} {f['detour']:7.3f} {f['length_vs_reference']:8.3f} {f['vias_per_100']:8.1f} {f['corners_per_connection']:8.2f} "
                   f"{f['sharp_per_100']:9.1f} {f['short_per_100']:9.1f} {f['tight_percent']:8.1f} {f['median_gap']:5.2f}")

    out += ["", f"buses: {buses['buses']} buses"]
    out.append(f"{'':{wide}} {'pairs':>6} {'alongside %':>12} {'same vias %':>12} {'crossing %':>11} {'length mismatch %':>18}")
    for name, f in buses["rows"].items():
        out.append(f"{name:{wide}} {f['pairs']:6} {f['alongside_percent']:12.1f} {f['same_vias_percent']:12.1f} {f['crossing_percent']:11.1f} {f['length_mismatch_percent']:18.1f}")

    out += ["", "circuit"]
    out.append(f"{'':{wide}} | power: nets  narrow%  vias/100  detour | decaps  path/straight(median)  via between%  not joined% | xtal nets  detour  vias | pairs  coupled%  mismatch%  same vias%")
    for name, f in circuit["rows"].items():
        out.append(f"{name:{wide}} | {f['power_nets']:11} {f['power_narrow_percent']:8.1f} {f['power_vias_per_100']:9.1f} {f['power_detour']:7.2f} | "
                   f"{f['decaps']:6} {f['decap_path_over_straight_median']:22.2f} {f['decap_via_between_percent']:13.1f} {f['decap_not_joined_directly_percent']:12.1f} | "
                   f"{f['crystal_nets']:9} {f['crystal_detour']:7.2f} {f['crystal_vias']:5} | "
                   f"{f['pairs']:5} {f['pair_coupled_percent']:9.1f} {f['pair_mismatch_percent']:10.1f} {f['pair_same_vias_percent']:11.1f}")

    out += ["", "pad exits"]
    out.append(f"{'':{wide}} {'exits':>7} {'grazing %':>10} {'long-pad exits':>15} {'through long side %':>20}")
    for name, f in exits["rows"].items():
        out.append(f"{name:{wide}} {f['exits']:7} {f['grazing_percent']:10.1f} {f['long_pad_exits']:15} {f['through_long_side_percent']:20.1f}")
    return "\n".join(out)


def plain(value):
    """A result as JSON can hold it: nan (a median of nothing) as null."""
    if isinstance(value, dict):
        return {k: plain(v) for k, v in value.items()}
    if isinstance(value, list):
        return [plain(v) for v in value]
    return None if isinstance(value, float) and math.isnan(value) else value


# --------------------------------------------------------------------------------------------------------------
# Files and the command line

def load(path):
    return json.loads(Path(path).read_text())


def boards_in(directory, reference_name, routing_names, only=None):
    """The boards of a directory of BOARD.NAME.json dumps, read one at a time, in name order."""
    ending = f".{reference_name}.json"
    for path in sorted(Path(directory).glob(f"*{ending}")):
        board = path.name[:-len(ending)]
        if only is not None and board not in only:
            continue
        routings = {}
        for name in routing_names:
            other = path.with_name(f"{board}.{name}.json")
            routings[name] = load(other) if other.exists() else None
        yield board, load(path), routings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("dumps", nargs="+", metavar="DUMP",
                        help="REFERENCE.json NAME=ROUTED.json ..., or with --dir the names of the routings")
    parser.add_argument("--dir", help="a directory of BOARD.NAME.json dumps: score every board in it")
    parser.add_argument("--reference",
                        help="with --dir: the name of the reference routing; without: the name of its row (default: reference)")
    parser.add_argument("--boards", help="with --dir: a file naming the boards to score, one a line (default: all)")
    parser.add_argument("--assume-complete", action="store_true",
                        help="take every routing as complete, whatever its dump says")
    parser.add_argument("--clearance", type=float,
                        help="the clearance in mm for the spacing figures (default: the Default class's from the dumps' rules, else 0.2)")
    parser.add_argument("--json", metavar="FILE",
                        help="write the figures as JSON here; - for standard output in place of the tables")
    parser.add_argument("--per-net", action="store_true",
                        help="with --json: also the per-net rows of the general table")
    args = parser.parse_args(argv)

    if args.dir:
        if not args.reference:
            parser.error("--dir needs --reference, the name of the reference routing")
        names = [args.reference] + args.dumps
        if any("=" in name or name.endswith(".json") for name in args.dumps):
            parser.error("with --dir, name the routings (the NAME of BOARD.NAME.json), not files")
        only = set(Path(args.boards).read_text().split("\n")) - {""} if args.boards else None
        boards = boards_in(args.dir, args.reference, args.dumps, only)
    else:
        if args.boards:
            parser.error("--boards goes with --dir")
        if len(args.dumps) < 2 or "=" in args.dumps[0] or not all("=" in d for d in args.dumps[1:]):
            parser.error("give the reference dump, then each routing as NAME=DUMP.json")
        named = [d.split("=", 1) for d in args.dumps[1:]]
        names = [args.reference or "reference"] + [name for name, _ in named]
        boards = [(Path(args.dumps[0]).stem, load(args.dumps[0]), {name: load(path) for name, path in named})]
    if len(set(names)) != len(names):
        parser.error("the reference and the routings need different names")

    result = compare(boards, names, assume_complete=args.assume_complete, clearance=args.clearance,
                     per_net=args.per_net)
    for board, reason in result["skipped"].items():
        print(f"skipped {board}: {reason}", file=sys.stderr)
    if args.json == "-":
        json.dump(plain(result), sys.stdout, indent=1)
        print()
    elif args.json:
        Path(args.json).write_text(json.dumps(plain(result), indent=1) + "\n")
    if not result["boards"]:
        print("no board to compare", file=sys.stderr)
        return 1
    if args.json != "-":
        print(tables(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
