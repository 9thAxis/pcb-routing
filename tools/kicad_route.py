#!/usr/bin/env python3
"""Route the unconnected nets of a KiCad board with pcb-routing.

    kicad_route.py BOARD.kicad_pcb -o ROUTED.kicad_pcb [--nets A,B] [--step 0.05] [--variants N]
                   [--layers F.Cu,B.Cu] [--problem FILE] [--solution FILE] [--engine PATH]

Run it with the Python that ships with KiCad (it needs the `pcbnew` module), for example on macOS:

    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 kicad_route.py ...

What it does:

1. Reads the board: pads, tracks, vias, drilled holes, the outline and the rule areas become the fixed picture;
   every net with pads that are not yet joined becomes one entry, with one terminal per group of joined pads.
   Widths, via sizes and clearances come from each net's class and the board's minima.
2. Runs the `pcb-routing` program on that problem.
3. Adds the tracks and vias found to the board, fills the zones again, and saves it under the new name.

Run KiCad's design rule check on the result before using it. What this bridge does not do yet is listed in
the README of the repository.
"""
import argparse, json, math, os, subprocess, sys, tempfile
from pathlib import Path

import pcbnew

MM = pcbnew.ToMM


def xy(p):
    return [round(MM(p.x), 6), round(MM(p.y), 6)]


def rings_of(poly_set):
    """Every outline of a SHAPE_POLY_SET as its rings: the outline first, then its holes."""
    out = []
    for i in range(poly_set.OutlineCount()):
        chain = poly_set.Outline(i)
        rings = [[xy(chain.CPoint(k)) for k in range(chain.PointCount())]]
        for h in range(poly_set.HoleCount(i)):
            hole = poly_set.Hole(i, h)
            rings.append([xy(hole.CPoint(k)) for k in range(hole.PointCount())])
        out.append(rings)
    return out


def pad_polygon(pad, layer):
    try:
        return pad.GetEffectivePolygon(layer, pcbnew.ERROR_INSIDE)
    except TypeError:
        return pad.GetEffectivePolygon(layer)


def mask_opening(pad, layer):
    """How far the solder mask opening of a pad reaches beyond its outline on an outer copper layer, in mm; a
    large negative number where the mask covers it there (or on an inner layer)."""
    mask = pcbnew.F_Mask if layer == pcbnew.F_Cu else pcbnew.B_Mask if layer == pcbnew.B_Cu else None
    if mask is None or not pad.IsOnLayer(mask):
        return -10.0
    try:
        return max(MM(pad.GetSolderMaskExpansion(layer)), 0.0)
    except TypeError:
        return max(MM(pad.GetSolderMaskExpansion()), 0.0)


def local_clearance(pad):
    """A pad's own clearance in mm, where it or its footprint overrides the net class; else None."""
    for owner in (pad, pad.GetParentFootprint()):
        try:
            value = owner.GetLocalClearance()
        except Exception:
            value = None
        if isinstance(value, tuple):
            value = value[0] if value else None
        if value:
            return MM(value)
    return None


def drawing_polygon(item, layer):
    """A drawing or text on a copper layer as polygons; its bounding box if KiCad cannot give its outline."""
    poly = pcbnew.SHAPE_POLY_SET()
    try:
        item.TransformShapeToPolygon(poly, layer, 0, pcbnew.FromMM(0.005), pcbnew.ERROR_OUTSIDE)
    except Exception:
        poly = pcbnew.SHAPE_POLY_SET()
    if poly.OutlineCount() == 0:
        box = item.GetBoundingBox()
        poly.NewOutline()
        for x, y in ((box.GetLeft(), box.GetTop()), (box.GetRight(), box.GetTop()), (box.GetRight(), box.GetBottom()), (box.GetLeft(), box.GetBottom())):
            poly.Append(x, y)
    return poly


def ignored_checks(board_path):
    """The design rule checks the board's project sets to 'ignore' (KiCad keeps them in the .kicad_pro)."""
    try:
        project = json.loads(Path(board_path).with_suffix(".kicad_pro").read_text())
        severities = project["board"]["design_settings"]["rule_severities"]
    except Exception:
        return set()
    return {name for name, level in severities.items() if level == "ignore"}


def joined_pads(connectivity, pad):
    """The pads that copper already joins to a pad: followed from item to item through tracks and vias."""
    found, pads, waiting = {pad.m_Uuid.AsString()}, [pad], [pad]
    while waiting:
        item = waiting.pop()
        for other in list(connectivity.GetConnectedTracks(item)) + list(connectivity.GetConnectedPads(item)):
            key = other.m_Uuid.AsString()
            if key in found:
                continue
            found.add(key)
            waiting.append(other)
            if other.Type() == pcbnew.PCB_PAD_T:
                pads.append(other)
    return pads


def build_problem(board, args, only=None, narrow=False):
    settings = board.GetDesignSettings()
    ignored = ignored_checks(args.board)
    slack = args.slack
    enabled = [l for l in board.GetEnabledLayers().CuStack()]
    name = board.GetLayerName
    wanted = set(args.layers.split(",")) if args.layers else None
    routing = [l for l in enabled if wanted is None or name(l) in wanted]
    all_names = [name(l) for l in enabled]
    layers = [name(l) for l in routing]

    box = board.GetBoardEdgesBoundingBox()
    if box.GetWidth() == 0 or box.GetHeight() == 0:
        box = board.ComputeBoundingBox(False)
    # a little beyond the board, with the cells on multiples of the step so that parts placed on a grid are met
    margin, step = 1.0, args.step
    low = lambda v: round(math.floor((MM(v) - margin) / step) * step, 6)
    high = lambda v: round(math.ceil((MM(v) + margin) / step) * step, 6)
    window = [low(box.GetLeft()), low(box.GetTop()), high(box.GetRight()), high(box.GetBottom())]

    min_clear = MM(settings.m_MinClearance)
    hole_clear = 0.0 if "hole_clearance" in ignored else MM(settings.m_HoleClearance)
    edge_clear = 0.0 if "copper_edge_clearance" in ignored else MM(settings.m_CopperEdgeClearance)
    copper, holes = [], []
    # the smallest clearance any net keeps from other copper, with the slack: what every track keeps from a pad anyway
    classes = {pad.GetNetname(): pad.GetNet().GetNetClassSlow() for pad in board.GetPads() if pad.GetNetCode() > 0}
    class_clear = {n: max(MM(c.GetClearance()), min_clear) for n, c in classes.items()}
    least_clear = min((max(MM(c.GetClearance()), min_clear) for c in classes.values()), default=min_clear) + slack

    def add_hole(at, drill, net, local=None):
        holes.append(dict(xy=xy(at), net=net or None))
        # copper keeps the hole clearance (or the pad's own) from the edge of a drilled hole: the hole stands
        # as a disc, grown by what that asks beyond the smallest clearance every track keeps anyway
        grow = max(max(hole_clear, local or 0.0) + slack - least_clear, 0.0)
        copper.append(dict(t="circle", c=xy(at), r=round(drill / 2 + grow, 6), net=net or None, layers=all_names))

    for pad in board.GetPads():
        net = pad.GetNetname() or None
        on = [l for l in enabled if pad.IsOnLayer(l)]
        smd = len(on) == 1
        # What a pad asks of other nets' copper beyond the smallest clearance every track keeps anyway: its own
        # clearance where it has one, and its solder mask opening (copper of another net there would be bare).
        local = local_clearance(pad)
        own = (local + slack - least_clear) if local is not None else 0.0
        for l in enabled:
            opening = -10.0 if "solder_mask_bridge" in ignored else mask_opening(pad, l)
            grow = max(own if l in on else -1.0, opening - least_clear + 0.01)
            if l not in on and grow <= -1.0:
                continue  # no copper and no opening on this layer
            if l in on and grow <= 0:
                shape = pad_polygon(pad, l)
            else:
                shape = pcbnew.SHAPE_POLY_SET()
                pad.TransformShapeToPolygon(shape, l if l in on else on[0], pcbnew.FromMM(max(grow, 0.0)), pcbnew.FromMM(0.005), pcbnew.ERROR_OUTSIDE)
            for rings in rings_of(shape):
                copper.append(dict(t="poly", rings=rings, net=net, layers=[name(l)], pad=smd and l in on))
        drill = pad.GetDrillSize()
        if drill.x > 0:
            add_hole(pad.GetPosition(), MM(max(drill.x, drill.y)), net, local)
    for track in board.GetTracks():
        net = track.GetNetname() or None
        if track.Type() == pcbnew.PCB_VIA_T:
            at = track.GetPosition()
            try:
                width = MM(track.GetWidth(pcbnew.F_Cu))
            except TypeError:
                width = MM(track.GetWidth())
            copper.append(dict(t="circle", c=xy(at), r=round(width / 2, 6), net=net, layers=all_names))
            holes.append(dict(xy=xy(at), net=net))
        elif track.Type() == pcbnew.PCB_ARC_T:
            # an arc as a few straight pieces
            poly = pcbnew.SHAPE_POLY_SET()
            track.TransformShapeToPolygon(poly, track.GetLayer(), 0, pcbnew.FromMM(0.005), pcbnew.ERROR_INSIDE)
            for rings in rings_of(poly):
                copper.append(dict(t="poly", rings=rings, net=net, layers=[name(track.GetLayer())]))
        else:
            copper.append(dict(t="seg", a=xy(track.GetStart()), b=xy(track.GetEnd()), w=round(MM(track.GetWidth()), 6),
                               net=net, layers=[name(track.GetLayer())]))

    # text and drawings on copper layers, on the board and in footprints: copper of no net
    drawn = list(board.GetDrawings())
    for footprint in board.GetFootprints():
        drawn.extend(footprint.GraphicalItems())
        drawn.extend(f for f in footprint.GetFields() if f.IsVisible())
    for item in drawn:
        for l in enabled:
            if not item.IsOnLayer(l):
                continue
            for rings in rings_of(drawing_polygon(item, l)):
                copper.append(dict(t="poly", rings=rings, net=None, layers=[name(l)]))

    regions = []
    outline = pcbnew.SHAPE_POLY_SET()
    try:
        has_outline = board.GetBoardPolygonOutlines(outline, False)
    except TypeError:
        has_outline = board.GetBoardPolygonOutlines(outline)
    if has_outline:
        frame = [[window[0] - 1, window[1] - 1], [window[2] + 1, window[1] - 1], [window[2] + 1, window[3] + 1], [window[0] - 1, window[3] + 1]]
        rings = [frame]
        for board_rings in rings_of(outline):
            rings.extend(board_rings)  # even-odd: closed outside the board and inside its cut-outs
        regions.append(dict(rings=rings))
    for zone in board.Zones():
        if not zone.GetIsRuleArea():
            continue  # filled copper is cut by new tracks when the zones are filled again
        if not (zone.GetDoNotAllowTracks() or zone.GetDoNotAllowVias()):
            continue
        on = [name(l) for l in enabled if zone.IsOnLayer(l)]
        for rings in rings_of(zone.Outline()):
            regions.append(dict(rings=rings, layers=[l for l in on if l in layers], tracks=bool(zone.GetDoNotAllowTracks()), vias=bool(zone.GetDoNotAllowVias())))

    # one terminal per group of pads that existing copper already joins
    connectivity = board.GetConnectivity()
    if only is None:
        only = set(args.nets.split(",")) if args.nets else None
    by_net = {}
    for pad in board.GetPads():
        if pad.GetNetname() and pad.GetNetCode() > 0 and any(pad.IsOnLayer(l) for l in enabled):
            by_net.setdefault(pad.GetNetname(), []).append(pad)
    nets, worst_clear = [], 0.0
    for net_name in sorted(by_net):
        if only is not None and net_name not in only:
            continue
        pads, seen, groups = by_net[net_name], set(), []
        for pad in pads:
            key = pad.m_Uuid.AsString()
            if key in seen:
                continue
            group = joined_pads(connectivity, pad)
            seen.update(p.m_Uuid.AsString() for p in group)
            groups.append(group)
        if len(groups) < 2:
            continue
        cls = classes[net_name]
        clear = max(MM(cls.GetClearance()), min_clear)
        width = max(MM(cls.GetTrackWidth()), MM(settings.m_TrackMinWidth))
        if narrow:
            # a second try for a net that could not be laid at its class width: narrower than its narrowest
            # pad (a track wider than a fine-pitch pad cannot enter it), never under the board's minimum
            slim = min(min(MM(pad.GetSize(l).x), MM(pad.GetSize(l).y)) for pad in pads for l in enabled if pad.IsOnLayer(l))
            # (a little under the pad's width: pads on an inch pitch do not sit on the grid, and the nearest cell
            # to a pad's middle is then a little nearer one neighbour)
            width = max(min(width, slim - 0.1), MM(settings.m_TrackMinWidth), 0.1)
        via = [max(MM(cls.GetViaDiameter()), MM(settings.m_ViasMinSize)), max(MM(cls.GetViaDrill()), MM(settings.m_MinThroughDrill))]
        via[0] = max(via[0], via[1] + 2 * MM(settings.m_ViasMinAnnularWidth))
        worst_clear = max(worst_clear, clear)
        terminals = []
        for group in groups:
            pad = group[0]
            on = [name(l) for l in routing if pad.IsOnLayer(l)]
            if not on:
                break
            terminals.append(dict(xy=xy(pad.GetPosition()), layers=on))
        if len(terminals) < len(groups):
            print(f"skipped {net_name}: a pad of it is on no routing layer", file=sys.stderr)
            continue
        more = {other: round(c + slack, 6) for other, c in class_clear.items() if c > clear and other != net_name}
        nets.append(dict(key=net_name, net=net_name, width=round(width, 6), via=[round(v, 6) for v in via], layers=layers,
                         clearance=dict(default=round(clear + slack, 6), nets=more), terminals=terminals, via_penalty_mm=args.via_penalty))

    via_drill = max((n["via"][1] for n in nets), default=0.3)
    rules = dict(hole_centre_mm=round(via_drill + MM(settings.m_HoleToHoleMin) + slack, 6), pad_drill_margin_mm=0.0,
                 region_margin_mm=round(edge_clear + (slack if edge_clear > 0 else 0.0), 6), new_clearance_mm=round(max(worst_clear, min_clear) + slack, 6))
    if narrow:
        rules["via_in_pad"] = True  # a land enclosed by another pad can only be left through a via in it
    options = dict(variants=args.variants, max_states=args.max_states)
    return dict(window=window, step=args.step, layers=layers, copper=copper, holes=holes, regions=regions, rules=rules, nets=nets, options=options)


def add_solution(board, solution):
    codes = {net.GetNetname(): net for net in board.GetNetsByName().values()}
    layer_id = {board.GetLayerName(l): l for l in board.GetEnabledLayers().CuStack()}
    at = lambda p: pcbnew.VECTOR2I(pcbnew.FromMM(p[0]), pcbnew.FromMM(p[1]))
    sizes = {n["key"]: n for n in solution["_nets"]}
    tracks = vias = 0
    for laid in solution["nets"]:
        net, size = codes[laid["net"]], sizes[laid["key"]]
        for track in laid["tracks"]:
            for a, b in zip(track["points"], track["points"][1:]):
                item = pcbnew.PCB_TRACK(board)
                item.SetStart(at(a)); item.SetEnd(at(b)); item.SetWidth(pcbnew.FromMM(size["width"]))
                item.SetLayer(layer_id[track["layer"]]); item.SetNet(net)
                board.Add(item); tracks += 1
        for v in laid["vias"]:
            item = pcbnew.PCB_VIA(board)
            item.SetPosition(at(v)); item.SetViaType(pcbnew.VIATYPE_THROUGH)
            try:
                item.SetWidth(pcbnew.F_Cu, pcbnew.FromMM(size["via"][0]))
            except TypeError:
                item.SetWidth(pcbnew.FromMM(size["via"][0]))
            item.SetDrill(pcbnew.FromMM(size["via"][1])); item.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); item.SetNet(net)
            board.Add(item); vias += 1
    return tracks, vias


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("-o", "--output", required=True)
    parser.add_argument("--nets", help="only these nets, comma separated (default: every net with pads not yet joined)")
    parser.add_argument("--layers", help="copper layers that may carry routes, comma separated (default: all)")
    parser.add_argument("--step", type=float, default=0.05, help="grid step in mm")
    parser.add_argument("--slack", type=float, default=0.02, help="added to every clearance for the grid, in mm")
    parser.add_argument("--variants", type=int, default=1, help="differently ordered attempts run side by side")
    parser.add_argument("--via-penalty", type=float, default=10.0, help="length of detour accepted before a via is preferred, in mm")
    parser.add_argument("--max-states", type=int, default=40_000_000, help="largest grid accepted, in cells times layers (memory grows with it and with the number of nets)")
    parser.add_argument("--problem", help="keep the problem file here")
    parser.add_argument("--solution", help="keep the solution file here")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"), help="the pcb-routing program")
    parser.add_argument("--threads", type=int, default=0)
    args = parser.parse_args()

    board = pcbnew.LoadBoard(args.board)
    problem = build_problem(board, args)
    if not problem["nets"]:
        print("nothing to route: every net's pads are joined")
        pcbnew.SaveBoard(args.output, board)
        return 0
    wanted = len(problem["nets"])
    first_keys = {n["key"] for n in problem["nets"]}
    tracks = vias = 0
    status = "routed"
    for attempt in ("at the class widths", "narrower, for the nets not joined"):
        with tempfile.TemporaryDirectory() as tmp:
            first = attempt.startswith("at")
            problem_path = Path((args.problem if first else None) or Path(tmp) / "problem.json")
            solution_path = Path((args.solution if first else None) or Path(tmp) / "solution.json")
            problem_path.write_text(json.dumps(problem))
            command = [args.engine, "route", str(problem_path), "-o", str(solution_path)] + (["--threads", str(args.threads)] if args.threads else [])
            run = subprocess.run(command, capture_output=True, text=True)
            print(run.stdout, end="")
            if run.returncode == 2 or not solution_path.exists():
                print(run.stderr, file=sys.stderr, end="")
                return 2
            solution = json.loads(solution_path.read_text())
        solution["_nets"] = problem["nets"]
        added = add_solution(board, solution)
        tracks, vias = tracks + added[0], vias + added[1]
        status = solution["status"]
        if status == "routed" or not first:
            break
        # what could not be joined is tried once more against the board as it now stands
        board.BuildConnectivity()
        problem = build_problem(board, args, only={n["key"] for n in problem["nets"]}, narrow=True)
        if not problem["nets"]:
            status = "routed"
            break
        print(f"second try, narrower: {', '.join(n['key'] for n in problem['nets'])}")
    filler = pcbnew.ZONE_FILLER(board)
    filler.Fill(board.Zones())
    pcbnew.SaveBoard(args.output, board)
    board.BuildConnectivity()
    left = len(build_problem(board, args, only=first_keys)["nets"]) if status != "routed" else 0
    print(f"{status}: {wanted - left} of {wanted} nets joined, {tracks} track segments and {vias} vias added; saved {args.output}")
    return 0 if status == "routed" else 1


if __name__ == "__main__":
    sys.exit(main())
