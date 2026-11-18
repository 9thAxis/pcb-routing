"""Where the parts of a board belong: the question the bridge puts to the engine, and the answer as told."""
import unittest

import support
from test_decompose import board

k = support.bridge()


class Floorplan(unittest.TestCase):
    def setUp(self):
        self.picture = board()
        self.found = k.floorplan(self.picture, k.Rules.of(self.picture, k.Settings()), support.ask)

    def test_the_board_is_one_domain_with_the_groups_of_each_sheet_gathered_into_zones(self):
        (ground,) = self.found["zones"]  # (one ground: one domain, with the zones within it)
        self.assertEqual((ground["name"], len(ground["parts"]), ground["rect"]), ("GND", 15, [0.0, 0.0, 20.0, 10.0]))
        # the amplifier with its input parts, and its filter by the converter: two groups of the one sheet,
        # gathered again by the pair between them
        amp = next(zone for zone in ground["inside"] if zone["name"] == "amp-1")
        self.assertEqual(sorted(inner["parts"] for inner in amp["inside"]), [["C10", "R10", "U1"], ["C11", "R11", "R12"]])
        self.assertEqual((amp["within"], amp["placed"], amp["sheets"]), (4, 6, ["amp-1"]))
        # the clock ties the converter, the connector and the part not placed yet
        other = next(zone for zone in ground["inside"] if "adc" in zone["sheets"])
        self.assertEqual((other["sheets"], other["parts"], other["placed"]), (["adc", "io", "later"], ["J1", "U20", "U9"], 2))
        self.assertEqual(self.found["flows"][0][2:], [2, 1])  # the filtered pair of an amplifier to the converter

    def test_every_zone_is_given_ground_within_that_of_the_zone_it_is_in(self):
        def walk(zone, outer):
            x0, y0, x1, y1 = zone["rect"]
            self.assertTrue(outer[0] <= x0 < x1 <= outer[2] and outer[1] <= y0 < y1 <= outer[3], (zone["name"], zone["rect"]))
            self.assertTrue(0.0 <= zone["at_home"] <= 1.0)
            for inner in zone["inside"]:
                walk(inner, zone["rect"])
        for zone in self.found["zones"]:
            walk(zone, self.picture.box)

    def test_the_question_names_what_belongs_on_an_edge_and_what_is_a_capacitor(self):
        asked = []
        k.floorplan(self.picture, k.Rules.of(self.picture, k.Settings()), lambda question: asked.append(question) or {})
        parts = {part["name"]: part for part in asked[0]["parts"]}
        self.assertEqual((asked[0]["ask"], asked[0]["box"]), ("floorplan", [0.0, 0.0, 20.0, 10.0]))
        self.assertEqual((parts["J1"]["edge"], parts["U1"]["edge"], parts["C10"]["capacitor"], parts["R10"]["capacitor"]),
                         (True, False, True, False))
        self.assertEqual((parts["U20"]["placed"], parts["U1"]["placed"], len(parts["U1"]["pads"])), (False, True, 4))
        # without guesses nothing is said to belong on an edge
        k.floorplan(self.picture, k.Rules.of(self.picture, k.Settings(guesses=False)), lambda question: asked.append(question) or {})
        self.assertFalse(any(part["edge"] for part in asked[1]["parts"]))

    def test_a_road_is_kept_along_each_cut_that_something_crosses_and_no_zone_stands_on_it(self):
        roads = self.found["roads"]
        self.assertTrue(roads and all(road["width"] > 0 and road["nets"] + road["rails"] > 0 and not road["gap"] for road in roads))
        self.assertTrue(all(road["used"] <= road["lanes"] for road in roads))
        self.assertEqual((self.found["travel"]["over"], self.found["travel"]["through"]), (0, 0))
        self.assertEqual([road["level"] for road in roads], sorted(road["level"] for road in roads))
        asked = []
        k.floorplan(self.picture, k.Rules.of(self.picture, k.Settings()), lambda question: asked.append(question) or {})
        # (a 0.25 mm track and the 0.202 mm it keeps; both layers of this board carry routes)
        self.assertEqual((asked[0]["pitch"], asked[0]["layers"]), (0.452, 2))
        self.assertEqual(asked[0]["stack"], [dict(name="F.Cu", way="any"), dict(name="B.Cu", way="any")])
        def leaves(zone):
            return [zone] if not zone["inside"] else [leaf for inner in zone["inside"] for leaf in leaves(inner)]
        meet = lambda a, b: min(a[2], b[2]) - max(a[0], b[0]) > 1e-6 and min(a[3], b[3]) - max(a[1], b[1]) > 1e-6
        for zone in self.found["zones"]:
            for leaf in leaves(zone):
                self.assertFalse(any(meet(leaf["rect"], road["rect"]) for road in roads), leaf["name"])
        told = k.floorplan_told(self.found)
        self.assertTrue(any(line.startswith("ground kept free of parts: 0 gaps") for line in told))
        self.assertTrue(any(line.startswith("every net between zones of one domain given a way") for line in told))

    def test_the_hints_say_how_boards_stack_how_high_parts_stand_and_what_a_part_is_to_be_near(self):
        import json, tempfile
        from pathlib import Path
        picture = board()
        # two boards side by side, the right one mounted over the left one's right end, 6 mm above it
        picture.outline = [[[[0, 0], [14, 0], [14, 10], [0, 10]]], [[[15, 0], [20, 0], [20, 10], [15, 10]]]]
        for part in picture.parts.values():
            part.footprint = "lib:R_0603_1608Metric"
        picture.parts["U9"].footprint = "lib:CP_Elec_8x10.5"
        said = dict(boards=[{}, dict(shift=[-6.0, 0.0], clear=6.0)], heights={"R_0603": 0.5, "CP_Elec": 10.5},
                    parts={"J1": dict(near=[3.0, 4.0]), "U1": dict(mate="U9")}, keepouts=[[1.0, 1.0, 2.0, 3.0]])
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "b.routing.json").write_text(json.dumps(said))
            hints = k.read_hints(Path(tmp, "b.routing.json"))
        asked = []
        k.floorplan(picture, k.Rules.of(picture, k.Settings(hints=hints)), lambda question: asked.append(question) or {})
        (q,) = asked
        self.assertEqual((q["regions"], q["shifts"]), ([[0, 0, 14, 10], [15, 0, 20, 10]], [[0.0, 0.0], [-6.0, 0.0]]))
        self.assertEqual(q["keepouts"], [[1.0, 1.0, 2.0, 3.0]])
        # the height under the right board is not held to (the standoffs are chosen for it): nothing is asked
        self.assertNotIn("limits", q)
        parts = {part["name"]: part for part in q["parts"]}
        self.assertEqual((parts["U9"]["height"], parts["R10"]["height"], parts["J1"]["near"], "near" in parts["U1"]),
                         (10.5, 0.5, [3.0, 4.0], False))
        self.assertEqual((parts["U1"]["mate"], "mate" in parts["U9"]), ("U9", False))
        # each part is said by the middle of its body, with where its pads and its own origin are from there
        for part in q["parts"]:
            self.assertEqual((len(part["offsets"]), len(part["pins"])), (len(part["pads"]), len(part["pads"])), part["name"])
            self.assertEqual(len(part["anchor"]), 2)
            self.assertIn("through", part)
        # but the plan says what the ground under it needs: the right board lies over x 9 to 14 of the left one
        found = k.floorplan(picture, k.Rules.of(picture, k.Settings(hints=hints)), support.ask)
        (under,) = found["clear"]
        self.assertEqual((under["rect"], under["has"]), ([9.0, 0, 14.0, 10], 6.0))
        self.assertIn(under["needs"], (0.5, 10.5))
        self.assertTrue(any(line.startswith("  under another board (x 9 to 14") for line in k.floorplan_told(found)))
        for wrong in (dict(boards=[dict(shift=[1])]), dict(heights={"R": "tall"}), dict(parts={"J1": dict(near=3)}), dict(parts={"J1": dict(mate=3)}), dict(parts={"J1": dict(noisy="very")}),
                      dict(keepouts=[[3, 0, 1, 2]]), dict(rows="sometimes")):
            with tempfile.TemporaryDirectory() as tmp:
                Path(tmp, "b.routing.json").write_text(json.dumps(wrong))
                with self.assertRaises(ValueError):
                    k.read_hints(Path(tmp, "b.routing.json"))

    def test_it_is_told_with_the_power_first(self):
        lines = k.power_told(self.found) + k.floorplan_told(self.found, 3)
        self.assertEqual(lines[0], "1 domains (grounds); 0 parts stand on two or more; 0 parts that nothing places in one")
        self.assertTrue(any(line.startswith("GND: 15 parts; 14 of 15 placed") for line in lines))
        self.assertTrue(any(line.startswith("  amp-1: 6 parts") and "given x" in line for line in lines))
        self.assertTrue(any(line.startswith("    amp-1 (C10, R10, U1): 3 parts") for line in lines))
        self.assertTrue(any(line.startswith("the signal nets between the smallest zones: ") for line in lines))


if __name__ == "__main__":
    unittest.main()
