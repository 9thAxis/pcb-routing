"""build_problem on hand-written pictures: the output is what the engine's model (src/model.rs) accepts, and
says what the rules say. No KiCad."""
import io, json, math, unittest
from contextlib import redirect_stderr

import support

k = support.bridge()

# The fields of src/model.rs: (all, required). The engine refuses unknown ones.
PROBLEM = ({"window", "step", "layers", "copper", "holes", "regions", "rules", "nets", "pairs", "options"}, {"window", "step", "layers", "nets"})
COPPER = {"seg": {"a", "b", "w"}, "circle": {"c", "r"}, "poly": {"rings"}}
COPPER_COMMON = ({"t", "net", "layers", "pad"}, {"t", "net", "layers"})
HOLE = ({"xy", "net"}, {"xy", "net"})
REGION = ({"rings", "layers", "tracks", "vias"}, {"rings"})
RULES = ({"hole_centre_mm", "pad_drill_margin_mm", "via_in_pad", "region_margin_mm", "new_clearance_mm", "quiet_clearance_mm"}, set())
NET = ({"key", "net", "width", "via", "layers", "clearance", "terminals", "via_penalty_mm", "layer_cost", "yield", "weight", "max_mm",
        "branch_anywhere", "regions", "last", "no_vias", "quiet", "bus", "bus_lane"}, {"key", "net", "width", "via", "layers", "clearance", "terminals"})
CLEARANCE = ({"default", "nets"}, {"default"})
TERMINAL = ({"xy", "layers"}, {"xy", "layers"})
OPTIONS = ({"max_iterations", "present_start", "present_growth", "present_max", "history_step", "refine_passes", "variants", "negotiate",
            "own_hole_free", "bend_mm", "two_pass", "sweep_margin", "exact_after", "stall_after", "patience", "drop_conflicts",
            "threads", "max_states", "bus_pull", "bus_cross"}, set())


def is_point(p):
    return isinstance(p, list) and len(p) == 2 and all(isinstance(v, (int, float)) and math.isfinite(v) for v in p)


class Shape(unittest.TestCase):
    def fields(self, thing, spec, what):
        allowed, required = spec
        self.assertLessEqual(set(thing), allowed, f"unknown field in {what}")
        self.assertLessEqual(required, set(thing), f"field missing in {what}")

    def rings(self, rings):
        self.assertTrue(rings and all(len(ring) >= 3 and all(is_point(p) for p in ring) for ring in rings))

    def check(self, problem):
        """Everything the engine's reader asks of a problem."""
        json.loads(json.dumps(problem, allow_nan=False))  # plain JSON, no NaN or infinity
        self.fields(problem, PROBLEM, "problem")
        self.assertEqual(len(problem["window"]), 4)
        self.assertLess(problem["window"][0], problem["window"][2])
        self.assertLess(problem["window"][1], problem["window"][3])
        self.assertGreater(problem["step"], 0)
        layers = problem["layers"]
        self.assertTrue(0 < len(layers) <= 7 and len(set(layers)) == len(layers))
        for c in problem["copper"]:
            self.assertIn(c["t"], COPPER)
            self.fields(c, (COPPER_COMMON[0] | COPPER[c["t"]], COPPER_COMMON[1] | COPPER[c["t"]]), "copper")
            self.assertTrue(c["net"] is None or isinstance(c["net"], str))
            self.assertTrue(all(isinstance(l, str) for l in c["layers"]))
            self.assertIsInstance(c.get("pad", False), bool)
            if c["t"] == "seg":
                self.assertTrue(is_point(c["a"]) and is_point(c["b"]) and c["w"] >= 0)
            elif c["t"] == "circle":
                self.assertTrue(is_point(c["c"]) and c["r"] >= 0)
            else:
                self.rings(c["rings"])
        for h in problem["holes"]:
            self.fields(h, HOLE, "hole")
            self.assertTrue(is_point(h["xy"]) and (h["net"] is None or isinstance(h["net"], str)))
        for g in problem["regions"]:
            self.fields(g, REGION, "region")
            self.rings(g["rings"])
            self.assertLessEqual(set(g.get("layers") or []), set(layers))
            self.assertIsInstance(g.get("tracks", True), bool)
            self.assertIsInstance(g.get("vias", True), bool)
        self.fields(problem["rules"], RULES, "rules")
        for name, v in problem["rules"].items():
            self.assertTrue(isinstance(v, bool) if name == "via_in_pad" else v >= 0, name)
        self.fields(problem["options"], OPTIONS, "options")
        self.assertLess(len(problem["nets"]), 250)
        keys = [n["key"] for n in problem["nets"]]
        self.assertEqual(len(keys), len(set(keys)))
        for n in problem["nets"]:
            self.fields(n, NET, "net")
            self.assertGreater(n["width"], 0)
            self.assertTrue(len(n["via"]) == 2 and n["via"][0] > 0 and n["via"][1] > 0)
            self.assertTrue(n["layers"] and set(n["layers"]) <= set(layers))
            self.fields(n["clearance"], CLEARANCE, "clearance")
            self.assertGreaterEqual(n["clearance"]["default"], 0)
            self.assertTrue(all(isinstance(o, str) and v >= 0 for o, v in n["clearance"]["nets"].items()))
            self.assertGreaterEqual(len(n["terminals"]), 2)
            for t in n["terminals"]:
                self.fields(t, TERMINAL, "terminal")
                self.assertTrue(is_point(t["xy"]) and t["layers"] and set(t["layers"]) <= set(n["layers"]))
            self.assertGreaterEqual(n["via_penalty_mm"], 0)
        return problem

    def build(self, picture, settings=None, **more):
        # the distances these tests expect are worked out with a slack of 0.02 mm
        return self.check(k.build_problem(picture, k.Rules.of(picture, settings or k.Settings(slack=0.02)), **more))


class Structure(Shape):
    def board(self):
        pads = [support.land(k, "A", 2, 2), support.land(k, "A", 8, 2), support.pin(k, "B", 2, 6), support.land(k, "B", 8, 6, layer="B.Cu"),
                support.land(k, "C", 12, 2), support.land(k, "C", 14, 2), support.land(k, None, 16, 8),
                support.pin(k, None, 18, 8, drill=2.0)]
        groups = {"A": [[0], [1]], "B": [[2], [3]], "C": [[4, 5]]}  # C is joined already
        p = support.picture(k, pads, groups)
        p.tracks = [k.Track([12, 2], [14, 2], 0.25, "C", "F.Cu")]
        p.vias = [k.Via([13, 2], 0.6, 0.3, "C")]
        p.polygons = [k.Polygon(support.square(k, 5, 8), "F.Cu"), k.Polygon(support.square(k, 13, 3, 0.1), "B.Cu", "C")]
        p.outline = [[[[0, 0], [20, 0], [20, 10], [0, 10]], [[9, 4], [11, 4], [11, 5], [9, 5]]]]
        p.areas = [k.Area(support.square(k, 16, 3)[0:1], ["F.Cu", "In1.Cu"], tracks=True, vias=False)]
        return p

    def test_everything_the_engine_requires_and_nothing_it_refuses(self):
        problem = self.build(self.board(), options=dict(variants=2, max_states=1000))
        self.assertEqual(problem["layers"], ["F.Cu", "B.Cu"])
        self.assertEqual(problem["window"], [-1.0, -1.0, 21.0, 11.0])
        self.assertEqual(problem["step"], 0.05)
        self.assertEqual(problem["options"], dict(variants=2, max_states=1000, bus_pull=0.2))
        self.assertNotIn("pairs", problem)

    def test_only_unjoined_nets_become_entries(self):
        problem = self.build(self.board())
        self.assertEqual([n["key"] for n in problem["nets"]], ["A", "B"])
        a, b = problem["nets"]
        self.assertEqual(a["terminals"], [dict(xy=[2, 2], layers=["F.Cu"]), dict(xy=[8, 2], layers=["F.Cu"])])
        self.assertEqual(b["terminals"], [dict(xy=[2, 6], layers=["F.Cu", "B.Cu"]), dict(xy=[8, 6], layers=["B.Cu"])])
        self.assertEqual((a["width"], a["via"], a["net"]), (0.25, [0.6, 0.3], "A"))
        self.assertEqual(a["clearance"], dict(default=0.22, nets={}))
        self.assertEqual(a["via_penalty_mm"], 10.0)

    def test_only_names_the_nets_wanted(self):
        self.assertEqual([n["key"] for n in self.build(self.board(), only={"B", "C"})["nets"]], ["B"])
        self.assertEqual(self.build(self.board(), only=set())["nets"], [])

    def test_rules(self):
        rules = self.build(self.board())["rules"]
        self.assertEqual(rules, dict(hole_centre_mm=0.57, pad_drill_margin_mm=0.0, region_margin_mm=0.32, new_clearance_mm=0.22))

    def test_copper_of_every_kind(self):
        copper = self.build(self.board())["copper"]
        lands = [c for c in copper if c.get("pad")]
        self.assertEqual(len(lands), 6)  # the surface-mount pads; not the two through-hole ones
        self.assertTrue(all(c["t"] == "poly" and len(c["layers"]) == 1 for c in lands))
        self.assertIn(dict(t="seg", a=[12, 2], b=[14, 2], w=0.25, net="C", layers=["F.Cu"]), copper)
        self.assertIn(dict(t="circle", c=[13, 2], r=0.3, net="C", layers=["F.Cu", "B.Cu"]), copper)
        self.assertIn(dict(t="poly", rings=support.square(k, 5, 8), net=None, layers=["F.Cu"]), copper)
        self.assertIn(dict(t="poly", rings=support.square(k, 13, 3, 0.1), net="C", layers=["B.Cu"]), copper)

    def test_outline_and_rule_areas_become_regions(self):
        regions = self.build(self.board())["regions"]
        frame = [[-2.0, -2.0], [22.0, -2.0], [22.0, 12.0], [-2.0, 12.0]]
        self.assertEqual(regions[0], dict(rings=[frame, [[0, 0], [20, 0], [20, 10], [0, 10]], [[9, 4], [11, 4], [11, 5], [9, 5]]]))
        # a rule area names only the layers that carry routes
        self.assertEqual(regions[1], dict(rings=support.square(k, 16, 3), layers=["F.Cu"], tracks=True, vias=False))

    def test_no_outline_no_frame(self):
        p = self.board()
        p.outline = None
        self.assertFalse(any("layers" not in g for g in self.build(p)["regions"]))

    def test_restricted_layers(self):
        with redirect_stderr(io.StringIO()) as said:
            problem = self.build(self.board(), k.Settings(slack=0.02, layers={"F.Cu"}))
        self.assertEqual(problem["layers"], ["F.Cu"])
        # B has a pad on the bottom only: it cannot be reached and is left out, by name
        self.assertEqual([n["key"] for n in problem["nets"]], ["A"])
        self.assertIn("skipped B", said.getvalue())
        # copper on the layer that carries no routes is still told: a through via must clear it
        self.assertTrue(any(c["layers"] == ["B.Cu"] for c in problem["copper"]))

    def test_stricter_class_and_slack(self):
        p = self.board()
        p.classes["B"] = k.NetClass(0.5, 0.4, 0.8, 0.4)
        a, b = self.build(p, k.Settings(slack=0.03))["nets"]
        self.assertEqual(a["clearance"], dict(default=0.23, nets={"B": 0.53}))
        self.assertEqual(b["clearance"], dict(default=0.53, nets={}))
        self.assertEqual(b["via"], [0.8, 0.4])

    def test_narrow_second_try(self):
        p = self.board()
        p.pads[1] = support.land(k, "A", 8, 2, half=0.15)  # a fine-pitch pad 0.3 wide
        problem = self.build(p, narrow=True)
        a, b = problem["nets"]
        self.assertEqual(a["width"], 0.2)
        self.assertEqual(b["width"], 0.25)  # no pad narrower than its class width
        self.assertIs(problem["rules"]["via_in_pad"], True)
        self.assertNotIn("via_in_pad", self.build(p)["rules"])

    def test_nothing_to_route(self):
        p = self.board()
        p.groups = {"C": [[4, 5]]}
        problem = self.build(p)
        self.assertEqual(problem["nets"], [])
        self.assertEqual(problem["rules"]["hole_centre_mm"], 0.57)


class PadsAndHoles(Shape):
    def test_grown_pad_is_its_polygon_and_a_stroke_along_each_edge(self):
        p = support.picture(k, [support.land(k, "A", 2, 2, clearance=0.5), support.land(k, "A", 8, 2)])
        copper = self.build(p)["copper"]
        strokes = [c for c in copper if c["t"] == "seg"]
        self.assertEqual(len(strokes), 4)
        self.assertTrue(all(c["w"] == 0.6 and c["net"] == "A" and c["layers"] == ["F.Cu"] and not c.get("pad") for c in strokes))
        ring = support.square(k, 2, 2)[0]
        self.assertEqual({(tuple(c["a"]), tuple(c["b"])) for c in strokes}, {(tuple(a), tuple(b)) for a, b in zip(ring, ring[1:] + ring[:1])})
        self.assertEqual(len([c for c in copper if c["t"] == "poly"]), 2)

    def test_mask_opening_without_copper_under_it(self):
        pad = support.land(k, "A", 2, 2, openings={"F.Cu": 0.05, "B.Cu": 0.05})
        pad.shapes["B.Cu"] = [support.square(k, 2, 2)]
        p = support.picture(k, [pad, support.land(k, "A", 8, 2)])
        back = [c for c in self.build(p)["copper"] if c["layers"] == ["B.Cu"]]
        self.assertEqual(back, [dict(t="poly", rings=support.square(k, 2, 2), net="A", layers=["B.Cu"], pad=False)])
        p.ignored = frozenset({"solder_mask_bridge"})
        self.assertEqual([c for c in self.build(p)["copper"] if c["layers"] == ["B.Cu"]], [])

    def test_pad_with_an_opening_and_no_copper_layer(self):
        """Defect 1: such a pad used to stop the bridge with an IndexError."""
        aperture = k.Pad(net=None, at=[5, 5], layers=[], shapes={"F.Cu": [support.square(k, 5, 5)]}, openings={"F.Cu": 0.3})
        p = support.picture(k, [support.land(k, "A", 2, 2), support.land(k, "A", 8, 2), aperture])
        copper = self.build(p)["copper"]
        mine = [c for c in copper if c["net"] is None]
        self.assertEqual(mine[0], dict(t="poly", rings=support.square(k, 5, 5), net=None, layers=["F.Cu"], pad=False))
        self.assertEqual(len(mine), 5)  # and its opening beyond the clearance, as strokes
        self.assertTrue(all(c["t"] == "seg" and c["w"] == 0.18 for c in mine[1:]))
        p.ignored = frozenset({"solder_mask_bridge"})
        self.assertEqual([c for c in self.build(p)["copper"] if c["net"] is None], [])
        self.assertEqual(len(self.build(p)["nets"]), 1)

    def test_pad_drills_and_vias_go_through_one_hole_rule(self):
        """Defect 4: a via's hole is kept clear of like a pad's."""
        p = support.picture(k, [support.land(k, "A", 2, 2), support.land(k, "A", 8, 2), support.pin(k, "B", 5, 5, drill=0.3, half=0.2)])
        p.vias = [k.Via([12, 5], 0.5, 0.3, "C")]
        problem = self.build(p)
        self.assertEqual(problem["holes"], [dict(xy=[5, 5], net="B"), dict(xy=[12, 5], net="C")])
        discs = {tuple(c["c"]): c for c in problem["copper"] if c["t"] == "circle" and c["r"] == 0.2}
        # drill radius 0.15, and the hole clearance 0.25 beyond the 0.2 every net keeps anyway
        self.assertEqual(set(discs), {(5, 5), (12, 5)})
        self.assertEqual((discs[(12, 5)]["net"], discs[(12, 5)]["layers"]), ("C", ["F.Cu", "B.Cu"]))
        self.assertEqual(problem["regions"], [])  # no hole wider than the new vias' drill

    def test_a_pads_own_clearance_grows_its_hole(self):
        p = support.picture(k, [support.land(k, "A", 2, 2), support.land(k, "A", 8, 2), support.pin(k, "B", 5, 5, drill=1.0, clearance=0.6)])
        self.assertIn(dict(t="circle", c=[5, 5], r=0.9, net="B", layers=["F.Cu", "B.Cu"]), self.build(p)["copper"])

    def test_wide_hole_is_closed_to_vias_round_it(self):
        """Defect 5: the engine's one hole spacing is for holes of the new vias' drill; round a wider hole a
        via-only region makes up the difference, for the hole's own net too."""
        p = support.picture(k, [support.pin(k, "A", 2, 2, drill=1.0), support.land(k, "A", 8, 2), support.land(k, None, 15, 5)])
        problem = self.build(p)
        self.assertEqual(problem["rules"]["hole_centre_mm"], 0.57)
        closed = [g for g in problem["regions"] if g.get("tracks") is False]
        self.assertEqual(len(closed), 1)
        region = closed[0]
        self.assertEqual((region["layers"], region["vias"]), ([], True))
        # The engine closes to a via's centre whatever is within the via's radius and the region margin of the
        # polygon. A via centre nearer than (hole radius + via drill radius + hole-to-hole + slack) must be closed.
        needed = 0.5 + 0.15 + 0.25 + 0.02
        reach = 0.6 / 2 + problem["rules"]["region_margin_mm"]
        ring = region["rings"][0]
        inner = min(self.edge_distance([2, 2], a, b) for a, b in zip(ring, ring[1:] + ring[:1]))
        self.assertGreaterEqual(inner + reach, needed - 1e-6)
        self.assertLess(inner + reach, needed + 0.05)  # and not much more than that

    def test_wide_hole_with_a_wide_margin_still_closed(self):
        p = support.picture(k, [support.pin(k, "A", 2, 2, drill=0.4), support.land(k, "A", 8, 2)])
        p.minima.edge_clearance = 2.0
        closed = [g for g in self.build(p)["regions"] if g.get("tracks") is False]
        self.assertEqual(len(closed), 1)  # the margin alone covers it: a token polygon on the hole

    @staticmethod
    def edge_distance(p, a, b):
        ax, ay, bx, by = a[0] - p[0], a[1] - p[1], b[0] - p[0], b[1] - p[1]
        dx, dy = bx - ax, by - ay
        t = max(0.0, min(1.0, -(ax * dx + ay * dy) / (dx * dx + dy * dy)))
        return math.hypot(ax + t * dx, ay + t * dy)


class Buses(unittest.TestCase):
    """Which nets are taken for a bus: neighbours on the same two parts, three or more."""

    def row(self, part, nets, x, pitch=1.0):
        return [support.land(k, net, x, i * pitch, half=0.3, part=part) for i, net in enumerate(nets)]

    def test_three_nets_side_by_side_on_two_parts_are_a_bus(self):
        pads = self.row("U1", ["a", "b", "c"], 0) + self.row("U2", ["a", "b", "c"], 20)
        bus = k.buses(pads)
        self.assertEqual(set(bus), {"a", "b", "c"})
        self.assertEqual(len({name for name, _ in bus.values()}), 1)

    def test_lanes_follow_the_row_of_pads(self):
        pads = self.row("U1", ["c", "a", "d", "b"], 0) + self.row("U2", ["c", "a", "d", "b"], 20)
        self.assertEqual({net: lane for net, (_, lane) in k.buses(pads).items()}, dict(c=0, a=1, d=2, b=3))
        # a row along x
        across = [support.land(k, net, i, 0, half=0.3, part="U1") for i, net in enumerate("abcd")]
        there = [support.land(k, net, i, 9, half=0.3, part="U2") for i, net in enumerate("abcd")]
        self.assertEqual({net: lane for net, (_, lane) in k.buses(across + there).items()}, dict(a=0, b=1, c=2, d=3))

    def test_two_rows_take_their_lanes_column_by_column(self):
        header = lambda part, x: [support.land(k, net, x + i % 2, i // 2, half=0.3, part=part) for i, net in enumerate("abcdef")]
        lanes = {net: lane for net, (_, lane) in k.buses(header("J1", 0) + header("J2", 20)).items()}
        self.assertEqual(lanes, dict(a=0, b=1, c=2, d=3, e=4, f=5))

    def test_an_array_of_pads_and_a_bus_not_all_on_one_part_have_no_lanes(self):
        grid = lambda part, x: [support.land(k, net, x + i % 3, i // 3, half=0.3, part=part) for i, net in enumerate("abcdefghi")]
        self.assertEqual({lane for _, lane in k.buses(grid("U1", 0) + grid("U2", 20)).values()}, {0})
        # b, c, d run between U2 and U3 as well as a, b, c between U1 and U2: one bus, no part with all of it
        pads = self.row("U1", ["a", "b", "c"], 0) + self.row("U2", ["a", "b", "c", "d"], 20) + self.row("U3", ["b", "c", "d", "e"], 40) + self.row("U4", ["c", "d", "e"], 60)
        bus = k.buses(pads)
        self.assertEqual(len({name for name, _ in bus.values()}), 1)
        self.assertEqual({lane for _, lane in bus.values()}, {0})

    def test_two_nets_are_not_a_bus(self):
        self.assertEqual(k.buses(self.row("U1", ["a", "b"], 0) + self.row("U2", ["a", "b"], 20)), {})

    def test_nets_side_by_side_on_one_part_only_are_not_a_bus(self):
        pads = self.row("U1", ["a", "b", "c"], 0) + [support.land(k, n, 20 + 10 * i, 0, part=f"R{i}") for i, n in enumerate("abc")]
        self.assertEqual(k.buses(pads), {})

    def test_nets_far_apart_on_a_part_are_not_neighbours(self):
        # on U2 the three are spread among other pads: no two of them are neighbours there
        spread = self.row("U2", ["a", "x", "b", "y", "c"], 20)
        self.assertEqual(k.buses(self.row("U1", ["a", "b", "c"], 0) + spread), {})

    def test_pads_of_no_net_between_them_keep_nets_apart_too(self):
        spread = self.row("U2", ["a", None, "b", None, "c"], 20)
        self.assertEqual(k.buses(self.row("U1", ["a", "b", "c"], 0) + spread), {})

    def test_a_net_that_is_not_to_be_routed_is_of_no_bus(self):
        pads = self.row("U1", ["a", "b", "c", "d"], 0) + self.row("U2", ["a", "b", "c", "d"], 20)
        self.assertEqual(set(k.buses(pads, {"a", "b", "c"})), {"a", "b", "c"})
        self.assertEqual(k.buses(pads, {"a", "b"}), {})  # two are not a bus, whatever lies beside them
        self.assertEqual(k.buses(pads, {"a", "c", "d"}), {})  # a is no neighbour of c

    def test_the_buses_do_not_depend_on_the_order_of_the_pads(self):
        pads = self.row("U1", ["a", "b", "c"], 0) + self.row("U2", ["a", "b", "c"], 20)
        self.assertEqual(k.buses(pads), k.buses(pads[::-1]))

    def test_the_bridge_and_the_quality_measure_agree_on_what_a_bus_is(self):
        import sys
        sys.path.insert(0, str(support.TOOLS / "quality"))
        try:
            import measures
        except ImportError:
            self.skipTest("the quality measures need shapely")
        pads = self.row("U1", ["a", None, "b", "c", "d"], 0) + self.row("U2", ["a", "b", "c", "d"], 20)
        near = measures.neighbour_nets(dict(pads=[dict(ref=p.part, net=p.net, at=p.at) for p in pads]))
        on_both = {pair for pair, where in near.items() if len(where) == 2}
        self.assertEqual(on_both, {("b", "c"), ("c", "d")})
        self.assertEqual(set(k.buses(pads)), {net for pair in on_both for net in pair})

    def test_two_buses_between_other_parts_stay_two(self):
        pads = (self.row("U1", ["a", "b", "c"], 0) + self.row("U2", ["a", "b", "c"], 20)
                + self.row("U3", ["d", "e", "f"], 40) + self.row("U4", ["d", "e", "f"], 60))
        bus = k.buses(pads)
        self.assertEqual(bus["a"][0], bus["c"][0])
        self.assertEqual(bus["d"][0], bus["f"][0])
        self.assertNotEqual(bus["a"][0], bus["d"][0])

    def test_the_problem_names_the_bus_of_each_net_and_none_without_pull(self):
        pads = self.row("U1", ["a", "b", "c"], 0, pitch=2.0) + self.row("U2", ["a", "b", "c"], 20, pitch=2.0)
        picture = support.picture(k, pads)
        problem = k.build_problem(picture, k.Rules.of(picture))
        self.assertEqual(len({net["bus"] for net in problem["nets"]}), 1)
        self.assertEqual(sorted(net["bus_lane"] for net in problem["nets"]), [0, 1, 2])
        problem = k.build_problem(picture, k.Rules.of(picture, k.Settings(bus_pull=0.0)))
        self.assertTrue(all("bus" not in net for net in problem["nets"]))


class Decoupling(unittest.TestCase):
    """A capacitor beside a pin of an IC on its net is joined to that pin in an entry of its own."""

    def board(self, far=False, **settings):
        pads = [support.land(k, "VCC", 0, 0, part="U1"), support.land(k, "VCC", 30 if far else 2, 0, part="C1"),
                support.land(k, "GND", 30 if far else 2, 2, part="C1"), support.land(k, "VCC", 20, 0, part="J1"),
                support.land(k, "GND", 20, 2, part="J1"), support.land(k, "GND", 0, 2, part="U1")]
        picture = support.picture(k, pads)
        return k.build_problem(picture, k.Rules.of(picture, k.Settings(**settings)))

    def test_the_capacitor_goes_to_the_pin_and_the_rest_is_joined_without_it(self):
        nets = {net["key"]: net for net in self.board()["nets"]}
        self.assertEqual(sorted(nets), ["GND", "GND | C1", "VCC", "VCC | C1"])
        link, rest = nets["VCC | C1"], nets["VCC"]
        self.assertEqual(link["net"], "VCC")
        self.assertEqual([t["xy"] for t in link["terminals"]], [[2, 0], [0, 0]])
        self.assertEqual(sorted(t["xy"] for t in rest["terminals"]), [[0, 0], [20, 0]])
        self.assertGreater(link["via_penalty_mm"], rest["via_penalty_mm"])

    def test_a_capacitor_far_from_any_pin_and_a_board_asked_not_to_are_left_as_they_are(self):
        self.assertEqual(sorted(net["key"] for net in self.board(far=True)["nets"]), ["GND", "VCC"])
        self.assertEqual(sorted(net["key"] for net in self.board(decoupling=False)["nets"]), ["GND", "VCC"])

    def test_a_net_of_only_the_capacitor_and_the_pin_is_one_entry(self):
        pads = [support.land(k, "VREF", 0, 0, part="U1"), support.land(k, "VREF", 2, 0, part="C1"), support.land(k, "GND", 2, 2, part="C1")]
        picture = support.picture(k, pads)
        nets = k.build_problem(picture, k.Rules.of(picture))["nets"]
        self.assertEqual([net["key"] for net in nets], ["VREF | C1"])

    def test_a_resistor_or_a_capacitor_already_joined_is_no_decoupling(self):
        pads = [support.land(k, "VCC", 0, 0, part="U1"), support.land(k, "VCC", 2, 0, part="R1"), support.land(k, "X", 2, 2, part="R1"),
                support.land(k, "VCC", 4, 0, part="C1"), support.land(k, "GND", 4, 2, part="C1"), support.land(k, "VCC", 20, 0, part="J1")]
        self.assertEqual(k.decoupling(pads, [[0], [1], [3, 5]]), [])
        self.assertEqual(k.decoupling(pads, [[0], [1], [3], [5]]), [(2, 0)])


if __name__ == "__main__":
    unittest.main()
