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

    def test_it_is_told_with_the_power_first(self):
        lines = k.power_told(self.found) + k.floorplan_told(self.found, 3)
        self.assertEqual(lines[0], "1 domains (grounds); 0 parts stand on two or more; 0 parts that nothing places in one")
        self.assertTrue(any(line.startswith("GND: 15 parts; 14 of 15 placed") for line in lines))
        self.assertTrue(any(line.startswith("  amp-1: 6 parts") and "given x" in line for line in lines))
        self.assertTrue(any(line.startswith("    amp-1 (C10, R10, U1): 3 parts") for line in lines))
        self.assertTrue(any(line.startswith("the signal nets between the smallest zones: ") for line in lines))


if __name__ == "__main__":
    unittest.main()
