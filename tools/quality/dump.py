#!/usr/bin/env python3
"""Write the copper of a KiCad board as JSON, for score.py and show.py.

    dump.py BOARD.kicad_pcb [-o DUMP.json] [--project FILE.kicad_pro]

Run it with the Python that ships with KiCad (it needs the `pcbnew` module and nothing else), for example on
macOS:

    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 dump.py ...

Without -o the dump goes beside the board as BOARD.json. What it holds (lengths in mm, points [x, y], layers by
name):

    layers       the enabled copper layers, top to bottom
    pads         every pad on copper: net, at (its position), shapes {layer: [ring, ...]} (its outline as
                 polygons, round parts drawn at most 0.002 mm outside), ref (its part), num
    tracks       net, a, b (ends), w (width), layer, len (KiCad's length), and mid for an arc
    vias         net, at, d (diameter on the top layer); where a via starts and stops is not kept
    zone_nets    the nets that have a copper zone (not rule areas)
    unconnected  how many connections KiCad counts as open on the board as saved (zones as they were filled).
                 score.py takes a routing as complete when this is 0
    rules        from the project file (BOARD.kicad_pro unless --project names another), absent without one:
                 clearance (the Default class's), track_widths {class: mm}, patterns [[pattern, class]],
                 assignments {net: class}

A board whose tracks were removed and laid again by a router often has no project of its own, or the designer's
original has none: name the one to read the rules from with --project.
"""
import argparse, json, sys
from pathlib import Path

try:
    import pcbnew
except ImportError:  # the rules are read without KiCad; only reading a board needs it
    pcbnew = None

ARC_ERROR = 0.002  # mm: how far the corners of a polygon drawn for a round shape lie outside it


def project_rules(project):
    """What the measures need of a project file's net classes (the .kicad_pro as parsed JSON)."""
    settings = project.get("net_settings") or {}
    classes = settings.get("classes") or []
    return dict(
        clearance=next((c.get("clearance") for c in classes if c.get("name") == "Default"), None),
        track_widths={c["name"]: c.get("track_width") for c in classes if "name" in c},
        patterns=[[p.get("pattern", ""), p.get("netclass")] for p in settings.get("netclass_patterns") or []],
        assignments=settings.get("netclass_assignments") or {},
    )


def read_rules(path):
    """The rules of the project file at `path`; None if it is not there or cannot be read."""
    try:
        return project_rules(json.loads(Path(path).read_text()))
    except (OSError, ValueError, AttributeError, TypeError):
        return None


def _xy(p):
    return [round(pcbnew.ToMM(p.x), 5), round(pcbnew.ToMM(p.y), 5)]


def _rings(polygons):
    rings = []
    for i in range(polygons.OutlineCount()):
        outline = polygons.Outline(i)
        rings.append([_xy(outline.CPoint(k)) for k in range(outline.PointCount())])
    return rings


def read_board(board):
    """A loaded board as a dump, without the rules."""
    layers = [l for l in board.GetEnabledLayers().CuStack()]
    name = board.GetLayerName
    pads, tracks, vias = [], [], []
    for pad in board.GetPads():
        on = [l for l in layers if pad.IsOnLayer(l)]
        if not on:
            continue
        shapes = {}
        for l in on:
            polygons = pcbnew.SHAPE_POLY_SET()
            pad.TransformShapeToPolygon(polygons, l, 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
            shapes[name(l)] = _rings(polygons)
        part = pad.GetParentFootprint()
        pads.append(dict(net=pad.GetNetname(), at=_xy(pad.GetPosition()), shapes=shapes, ref=part.GetReference() if part else "", num=pad.GetNumber()))
    for t in board.GetTracks():
        if t.Type() == pcbnew.PCB_VIA_T:
            vias.append(dict(net=t.GetNetname(), at=_xy(t.GetPosition()), d=pcbnew.ToMM(t.GetWidth(layers[0]))))
        else:
            track = dict(net=t.GetNetname(), a=_xy(t.GetStart()), b=_xy(t.GetEnd()), w=pcbnew.ToMM(t.GetWidth()), layer=name(t.GetLayer()), len=pcbnew.ToMM(t.GetLength()))
            if t.Type() == pcbnew.PCB_ARC_T:
                track["mid"] = _xy(t.GetMid())
            tracks.append(track)
    zone_nets = sorted({z.GetNetname() for z in board.Zones() if not z.GetIsRuleArea() and z.GetNetname()})
    return dict(layers=[name(l) for l in layers], pads=pads, tracks=tracks, vias=vias, zone_nets=zone_nets,
                unconnected=board.GetConnectivity().GetUnconnectedCount(True))


def dump(board_path, project_path=None):
    """The dump of the board file at `board_path`, with the rules of its project (or of the one named)."""
    picture = read_board(pcbnew.LoadBoard(str(board_path)))
    rules = read_rules(project_path or Path(board_path).with_suffix(".kicad_pro"))
    if rules is not None:
        picture["rules"] = rules
    return picture


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("-o", "--output", help="the dump (default: the board's name with .json)")
    parser.add_argument("--project", help="the .kicad_pro to read the net classes from (default: the one beside the board)")
    args = parser.parse_args(argv)
    if pcbnew is None:
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")
    if args.project and read_rules(args.project) is None:
        parser.error(f"cannot read {args.project}")
    output = Path(args.output) if args.output else Path(args.board).with_suffix(".json")
    output.write_text(json.dumps(dump(args.board, args.project)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
