#!/usr/bin/env python3
"""Small boards made with KiCad itself, for the tests of the code that reads zones (test_real_kicad.py runs
this with KiCad's Python, one scene per process, and reads what it prints).

    kicad_scenes.py SCENE DIRECTORY ENGINE

Prints one line of JSON: what the bridge reads from the scene, and where it saved the board.
"""
import json, math, sys
from pathlib import Path

import pcbnew

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from bridge import engine, kicad, outcome, questions, route, rules, ties

MM = pcbnew.FromMM


def at(x, y):
    return pcbnew.VECTOR2I(MM(x), MM(y))


class Scene:
    """A two-layer board 30 mm by 20 mm."""

    def __init__(self):
        self.board = pcbnew.BOARD()
        self.nets = {}
        for a, b in (((0, 0), (30, 0)), ((30, 0), (30, 20)), ((30, 20), (0, 20)), ((0, 20), (0, 0))):
            edge = pcbnew.PCB_SHAPE(self.board)
            edge.SetShape(pcbnew.SHAPE_T_SEGMENT)
            edge.SetStart(at(*a)); edge.SetEnd(at(*b)); edge.SetLayer(pcbnew.Edge_Cuts); edge.SetWidth(MM(0.1))
            self.board.Add(edge)

    def net(self, name):
        if name not in self.nets:
            self.nets[name] = pcbnew.NETINFO_ITEM(self.board, name)
            self.board.Add(self.nets[name])
        return self.nets[name]

    def pad(self, ref, net, x, y, through=True, size=1.6, drill=0.8):
        """A part of one round pad: through the board, or a land on the front."""
        footprint = pcbnew.FOOTPRINT(self.board)
        footprint.SetReference(ref)
        footprint.SetPosition(at(x, y))
        self.board.Add(footprint)
        pad = pcbnew.PAD(footprint)
        pad.SetNumber("1")
        pad.SetShape(pcbnew.F_Cu, pcbnew.PAD_SHAPE_CIRCLE)
        pad.SetSize(pcbnew.F_Cu, pcbnew.VECTOR2I(MM(size), MM(size)))
        if through:
            pad.SetAttribute(pcbnew.PAD_ATTRIB_PTH)
            pad.SetDrillSize(pcbnew.VECTOR2I(MM(drill), MM(drill)))
            pad.SetLayerSet(pad.PTHMask())
        else:
            pad.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
            pad.SetLayerSet(pad.SMDMask())
        pad.SetPosition(at(x, y))
        pad.SetNet(self.net(net))
        footprint.Add(pad)
        return pad

    def zone(self, net, layer, box=(1, 1, 29, 19)):
        zone = pcbnew.ZONE(self.board)
        zone.SetNet(self.net(net))
        zone.SetLayer(layer)
        outline = zone.Outline()  # (filled in place: a new outline set on the zone is lost)
        outline.NewOutline()
        for x, y in ((box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])):
            outline.Append(MM(x), MM(y))
        zone.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        zone.SetThermalReliefGap(MM(0.5)); zone.SetThermalReliefSpokeWidth(MM(0.5))
        zone.SetLocalClearance(MM(0.3)); zone.SetMinThickness(MM(0.25))
        zone.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_NEVER)
        self.board.Add(zone)
        return zone

    def track(self, net, layer, *points, width=0.25):
        for a, b in zip(points, points[1:]):
            track = pcbnew.PCB_TRACK(self.board)
            track.SetStart(at(*a)); track.SetEnd(at(*b)); track.SetWidth(MM(width)); track.SetLayer(layer)
            track.SetNet(self.net(net))
            self.board.Add(track)

    def read(self):
        pcbnew.ZONE_FILLER(self.board).Fill(self.board.Zones())
        self.board.BuildConnectivity()
        return kicad.read_board(self.board, frozenset(), ASK)


def spokes(scene, tie=True):
    """A pad in the corner of a zone, where it has two spokes; then with a track of another net across one of
    them; then with a track of its own to the zone. And a pad in the middle, with its four."""
    scene.zone("GND", pcbnew.F_Cu)
    scene.pad("J1", "GND", 2.2, 2.2)
    scene.pad("J2", "GND", 15, 10)
    count = lambda picture: {picture.pads[t.pad].part: t.spokes for t in picture.thermals}
    whole = count(scene.read())
    scene.pad("R1", "SIG", 3.6, 1.0, through=False, size=0.5); scene.pad("R2", "SIG", 3.6, 4.4, through=False, size=0.5)
    scene.track("SIG", pcbnew.F_Cu, (3.6, 1.0), (3.6, 4.4))
    picture = scene.read()
    cut = count(picture)
    starved = [t for t in picture.thermals if t.spokes < t.least]
    targets = questions.tie_targets(picture, [(starved[0], picture.pads[starved[0].pad].at, 0.25, [])], ASK)[0] if starved else []
    if targets and tie:
        scene.track("GND", pcbnew.F_Cu, (2.2, 2.2), tuple(targets[0]))
    return dict(whole=whole, cut=cut, targets=targets, tied=count(scene.read()),
                starved=[picture.pads[t.pad].part for t in starved], least=[t.least for t in picture.thermals],
                dear=sorted({t.layer for t in picture.thermals if t.ground}))


def groups(scene):
    """Three pads on a zone, one beside it; then with a track across the zone from edge to edge."""
    scene.zone("GND", pcbnew.B_Cu, box=(1, 1, 24, 19))
    for ref, x, y in (("J1", 5, 5), ("J2", 20, 5), ("J3", 5, 15), ("J4", 27, 10)):
        scene.pad(ref, "GND", x, y)
    names = lambda picture: sorted(sorted(picture.pads[i].part for i in group) for group in picture.groups["GND"])
    whole = scene.read()
    scene.pad("R1", "SIG", 0.9, 10, size=0.9, drill=0.4); scene.pad("R2", "SIG", 29.1, 12, size=0.9, drill=0.4)
    scene.track("SIG", pcbnew.B_Cu, (0.9, 10), (25.2, 10), (25.2, 12), (29.1, 12))
    cut = scene.read()
    return dict(whole=names(whole), cut=names(cut), fills=[len(whole.fills), len(cut.fills)],
                pours=[sorted(len(own) for own in picture.pours["GND"].values()) for picture in (whole, cut)],
                unjoined=cut.unjoined(), without=names(kicad.read_board(scene.board, frozenset(), ASK, pours=False)))


def cleared(scene, room=True):
    """A pad in a zone, and a track of another net that this run laid round three sides of it: one spoke is
    left. The track is laid again clear of the pad; or, without `room` (its own pads stand too near the
    pad), it is put back as it was."""
    scene.zone("GND", pcbnew.F_Cu)
    scene.pad("J1", "GND", 15, 10)
    scene.pad("J2", "GND", 25, 15)
    # (a round pad has its spokes on the diagonals: the track crosses three of them just beyond the gap)
    ring = [[16.0, 9.0], [16.6, 10.0], [16.0, 11.0], [15.0, 11.6], [14.0, 11.0]]
    ends = ((17.6, 7.4), (12.4, 12.6)) if room else (tuple(ring[0]), tuple(ring[-1]))
    scene.pad("R1", "SIG", *ends[0], through=False, size=0.5); scene.pad("R2", "SIG", *ends[1], through=False, size=0.5)
    board = kicad.KicadBoard(scene.board, frozenset(), ASK)
    count = lambda picture: {picture.pads[t.pad].part: t.spokes for t in picture.thermals}
    laid_before = board.read().copper_keys()
    round_the_pad = ([list(ends[0])] if room else []) + ring + ([list(ends[1])] if room else [])
    board.write(dict(nets=[dict(key="SIG", net="SIG", vias=[], tracks=[dict(layer="F.Cu", points=round_the_pad)])]), {"SIG": (0.25, [0.6, 0.3])})
    picture = board.read()
    starved = [t for t in picture.thermals if t.spokes < t.least]
    counted, said = outcome.Outcome(tracks=len(round_the_pad) - 1), []
    done = ties.cleared(board, rules.Settings(), engine.Engine(sys.argv[3]), counted, picture, starved, laid_before, None, said.append)
    after = board.read()
    signal = sorted((t.a, t.b) for t in after.tracks if t.net == "SIG")
    return dict(cut=count(picture), after=count(after), done=done, said=said, unjoined=after.unjoined(), tracks=[counted.tracks, len(signal)],
                as_it_was=signal == sorted((a, b) for a, b in zip(round_the_pad, round_the_pad[1:])),
                nearest=min(math.dist(p, [15, 10]) for t in after.tracks if t.net == "SIG" for p in (t.a, t.b)))


def to_route(scene):
    """A board for the whole bridge: a zone with pads on it and one beside it, and a signal across the zone."""
    scene.zone("GND", pcbnew.B_Cu, box=(1, 1, 24, 19))
    for ref, x, y in (("J1", 5, 5), ("J2", 20, 5), ("J3", 5, 15), ("J4", 27, 10)):
        scene.pad(ref, "GND", x, y)
    scene.pad("R1", "SIG", 3, 10, through=False, size=1.0); scene.pad("R2", "SIG", 22, 10, through=False, size=1.0)
    scene.pad("R3", "CLK", 12, 3, through=False, size=1.0); scene.pad("R4", "CLK", 12, 17, through=False, size=1.0)
    return {}


SCENES = dict(spokes=spokes, starved=lambda scene: spokes(scene, tie=False), groups=groups, to_route=to_route,
              cleared=cleared, not_cleared=lambda scene: cleared(scene, room=False))

if __name__ == "__main__":
    name, directory = sys.argv[1], Path(sys.argv[2])
    ASK = lambda question: engine.ask_engine(question, sys.argv[3])
    scene = Scene()
    said = SCENES[name](scene)
    pcbnew.ZONE_FILLER(scene.board).Fill(scene.board.Zones())
    saved = directory / f"{name}.kicad_pcb"
    pcbnew.SaveBoard(str(saved), scene.board)
    print(json.dumps(dict(said, saved=str(saved))))
