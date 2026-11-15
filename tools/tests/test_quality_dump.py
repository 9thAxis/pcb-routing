"""The dump of a board, against a stand-in for KiCad's pcbnew. No KiCad, no shapely."""
import contextlib, io, json, tempfile, unittest
from pathlib import Path

import support
import quality_support as q

pcbnew = support.stub_pcbnew()
dump = q.load("dump")
MM = 1_000_000  # KiCad's units in a mm


class Outline:
    def __init__(self, points): self.points = points
    def PointCount(self): return len(self.points)
    def CPoint(self, k): return support.VECTOR2I(*self.points[k])


class PolySet:
    def __init__(self): self.outlines = []
    def OutlineCount(self): return len(self.outlines)
    def Outline(self, i): return self.outlines[i]


class Part:
    def __init__(self, reference): self.reference = reference
    def GetReference(self): return self.reference


class Pad:
    def __init__(self, net, x, y, layers, ref="U1", num="1"):
        self.net, self.at, self.layers, self.part, self.num = net, (x * MM, y * MM), layers, Part(ref) if ref else None, num

    def GetNetname(self): return self.net
    def GetPosition(self): return support.VECTOR2I(*self.at)
    def IsOnLayer(self, layer): return layer in self.layers
    def GetParentFootprint(self): return self.part
    def GetNumber(self): return self.num

    def TransformShapeToPolygon(self, polygons, layer, clearance, error, where):
        self.asked = (clearance, error, where)
        x, y = self.at
        polygons.outlines.append(Outline([(x - MM // 2, y - MM // 2), (x + MM // 2, y - MM // 2), (x + MM // 2, y + MM // 2), (x - MM // 2, y + MM // 2)]))


class Track:
    def __init__(self, net, a, b, width, layer, mid=None):
        self.net, self.a, self.b, self.width, self.layer, self.mid = net, a, b, width, layer, mid

    def Type(self): return pcbnew.PCB_ARC_T if self.mid else pcbnew.PCB_TRACE_T
    def GetNetname(self): return self.net
    def GetStart(self): return support.VECTOR2I(self.a[0] * MM, self.a[1] * MM)
    def GetEnd(self): return support.VECTOR2I(self.b[0] * MM, self.b[1] * MM)
    def GetMid(self): return support.VECTOR2I(self.mid[0] * MM, self.mid[1] * MM)
    def GetWidth(self): return int(self.width * MM)
    def GetLayer(self): return self.layer
    def GetLength(self): return 5 * MM


class Via:
    def __init__(self, net, x, y, size): self.net, self.at, self.size = net, (x * MM, y * MM), size
    def Type(self): return pcbnew.PCB_VIA_T
    def GetNetname(self): return self.net
    def GetPosition(self): return support.VECTOR2I(*self.at)
    def GetWidth(self, layer): return int(self.size * MM)


class Zone:
    def __init__(self, net, rule_area=False): self.net, self.rule_area = net, rule_area
    def GetNetname(self): return self.net
    def GetIsRuleArea(self): return self.rule_area


class Board:
    def __init__(self, pads=(), tracks=(), zones=(), unconnected=0):
        self.pads, self.tracks, self.zones, self.unconnected = list(pads), list(tracks), list(zones), unconnected

    def GetEnabledLayers(self): return self
    def CuStack(self): return [pcbnew.F_Cu, pcbnew.B_Cu]
    def GetLayerName(self, layer): return {pcbnew.F_Cu: "F.Cu", pcbnew.B_Cu: "B.Cu"}[layer]
    def GetPads(self): return self.pads
    def GetTracks(self): return self.tracks
    def Zones(self): return self.zones
    def GetConnectivity(self): return self
    def GetUnconnectedCount(self, visible_only): return self.unconnected


def with_poly_sets(test):
    """The stand-in pcbnew with what reading pad shapes needs, for the length of a test."""
    for name, value in (("SHAPE_POLY_SET", PolySet), ("ERROR_OUTSIDE", "outside")):
        setattr(pcbnew, name, value)
        test.addCleanup(delattr, pcbnew, name)


PROJECT = dict(net_settings=dict(
    classes=[dict(name="Default", clearance=0.15, track_width=0.25), dict(name="POWER", clearance=0.3, track_width=0.5)],
    netclass_patterns=[dict(netclass="POWER", pattern="VCC*")], netclass_assignments={"GND": ["POWER"]}))


class Rules(unittest.TestCase):
    def test_what_is_kept_of_a_project(self):
        self.assertEqual(dump.project_rules(PROJECT), dict(clearance=0.15, track_widths={"Default": 0.25, "POWER": 0.5},
                                                           patterns=[["VCC*", "POWER"]], assignments={"GND": ["POWER"]}))

    def test_a_project_without_net_classes(self):
        self.assertEqual(dump.project_rules({}), dict(clearance=None, track_widths={}, patterns=[], assignments={}))
        self.assertEqual(dump.project_rules(dict(net_settings=dict(classes=None, netclass_patterns=None, netclass_assignments=None)))["patterns"], [])

    def test_a_project_that_is_not_there_or_not_json_has_no_rules(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(dump.read_rules(Path(tmp) / "none.kicad_pro"))
            (Path(tmp) / "bad.kicad_pro").write_text("(kicad_pcb")
            self.assertIsNone(dump.read_rules(Path(tmp) / "bad.kicad_pro"))
            (Path(tmp) / "good.kicad_pro").write_text(json.dumps(PROJECT))
            self.assertEqual(dump.read_rules(Path(tmp) / "good.kicad_pro")["clearance"], 0.15)


class ReadBoard(unittest.TestCase):
    def setUp(self):
        with_poly_sets(self)
        self.pads = [Pad("A", 1, 2, {pcbnew.F_Cu}, ref="R1", num="2"), Pad("B", 5, 5, {pcbnew.F_Cu, pcbnew.B_Cu}, ref=None),
                     Pad("PASTE", 9, 9, {pcbnew.F_Mask})]
        tracks = [Track("A", (1, 2), (4, 6), 0.25, pcbnew.F_Cu), Via("A", 4, 6, 0.6), Track("A", (4, 6), (5, 7), 0.2, pcbnew.B_Cu, mid=(4.7, 6.3))]
        self.board = Board(self.pads, tracks, [Zone("GND"), Zone("VCC"), Zone("GND"), Zone("", False), Zone("KEEP", rule_area=True)], unconnected=2)
        self.dump = dump.read_board(self.board)

    def test_layers_by_name(self):
        self.assertEqual(self.dump["layers"], ["F.Cu", "B.Cu"])

    def test_pads_on_copper_with_their_outline_on_each_layer(self):
        first, second = self.dump["pads"]
        self.assertEqual({k: first[k] for k in ("net", "at", "ref", "num")}, dict(net="A", at=[1.0, 2.0], ref="R1", num="2"))
        self.assertEqual(first["shapes"], {"F.Cu": [[[0.5, 1.5], [1.5, 1.5], [1.5, 2.5], [0.5, 2.5]]]})
        self.assertEqual(sorted(second["shapes"]), ["B.Cu", "F.Cu"])
        self.assertEqual(second["ref"], "")

    def test_round_shapes_are_drawn_just_outside(self):
        self.assertEqual(self.pads[0].asked, (0, 2000, "outside"))

    def test_tracks_arcs_and_vias(self):
        straight, arc = self.dump["tracks"]
        self.assertEqual(straight, dict(net="A", a=[1.0, 2.0], b=[4.0, 6.0], w=0.25, layer="F.Cu", len=5.0))
        self.assertEqual((arc["layer"], arc["mid"]), ("B.Cu", [4.7, 6.3]))
        self.assertEqual(self.dump["vias"], [dict(net="A", at=[4.0, 6.0], d=0.6)])

    def test_zone_nets_once_each_without_rule_areas(self):
        self.assertEqual(self.dump["zone_nets"], ["GND", "VCC"])

    def test_kicad_s_count_of_open_connections(self):
        self.assertEqual(self.dump["unconnected"], 2)

    def test_the_dump_is_json(self):
        self.assertEqual(json.loads(json.dumps(self.dump)), self.dump)


class CommandLine(unittest.TestCase):
    def setUp(self):
        with_poly_sets(self)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.loaded = []
        pcbnew.LoadBoard = lambda path: self.loaded.append(path) or Board([Pad("A", 1, 2, {pcbnew.F_Cu})])
        self.addCleanup(delattr, pcbnew, "LoadBoard")

    def test_the_dump_goes_beside_the_board_with_its_project_s_rules(self):
        (self.dir / "b.kicad_pro").write_text(json.dumps(PROJECT))
        self.assertEqual(dump.main([str(self.dir / "b.kicad_pcb")]), 0)
        written = json.loads((self.dir / "b.json").read_text())
        self.assertEqual(self.loaded, [str(self.dir / "b.kicad_pcb")])
        self.assertEqual(written["rules"]["clearance"], 0.15)
        self.assertEqual(len(written["pads"]), 1)

    def test_a_board_without_a_project_has_no_rules(self):
        dump.main([str(self.dir / "b.kicad_pcb"), "-o", str(self.dir / "out.json")])
        self.assertNotIn("rules", json.loads((self.dir / "out.json").read_text()))

    def test_another_project_can_be_named(self):
        (self.dir / "b.kicad_pro").write_text(json.dumps({}))
        (self.dir / "other.kicad_pro").write_text(json.dumps(PROJECT))
        dump.main([str(self.dir / "b.kicad_pcb"), "--project", str(self.dir / "other.kicad_pro")])
        self.assertEqual(json.loads((self.dir / "b.json").read_text())["rules"]["track_widths"]["POWER"], 0.5)

    def test_a_named_project_that_cannot_be_read_is_an_error(self):
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            dump.main([str(self.dir / "b.kicad_pcb"), "--project", str(self.dir / "none.kicad_pro")])
        self.assertEqual(self.loaded, [])


if __name__ == "__main__":
    unittest.main()
