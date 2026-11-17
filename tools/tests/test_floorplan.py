"""Where the parts of a board belong together: groups within sheets, zones of them, what flows between."""
import unittest

import support
from test_decompose import board

k = support.bridge()


class Communities(unittest.TestCase):
    def test_nodes_tied_more_within_than_without_are_a_group(self):
        # two triangles joined by one weak tie
        ties = {("a", "b"): 1.0, ("a", "c"): 1.0, ("b", "c"): 1.0, ("d", "e"): 1.0, ("d", "f"): 1.0, ("e", "f"): 1.0,
                ("c", "d"): 0.2}
        self.assertEqual(k.communities(list("abcdef"), ties), [["a", "b", "c"], ["d", "e", "f"]])
        self.assertEqual(k.communities(list("fedcba"), ties), [["f", "e", "d"], ["c", "b", "a"]])
        # nothing tied: every node by itself; and one round gathers what a second would gather further
        self.assertEqual(k.communities(["x", "y"], {}), [["x"], ["y"]])
        self.assertEqual(k.gathered([["a"], ["b"], ["c"]], {("a", "b"): 1.0}), [["a", "b"], ["c"]])


class Floorplan(unittest.TestCase):
    def setUp(self):
        self.picture = board()
        self.plan = k.floorplan(self.picture, k.Rules.of(self.picture, k.Settings()))

    def test_the_parts_of_a_sheet_that_are_tied_together_are_a_group_and_groups_are_gathered_into_zones(self):
        # the amplifier with its input parts, and its filter by the converter: two groups of the one sheet,
        # gathered again by the pair between them
        amp = next(zone for zone in self.plan.zones if zone.name == "amp-1")
        self.assertEqual(sorted(inner.parts for inner in amp.inside), [["C10", "R10", "U1"], ["C11", "R11", "R12"]])
        self.assertEqual((amp.within, amp.placed, amp.sheets), (4, 6, ["amp-1"]))
        self.assertEqual(self.plan.apart, [])
        # the clock ties the converter, the connector and the part not placed yet
        other = next(zone for zone in self.plan.zones if "adc" in zone.sheets)
        self.assertEqual((other.sheets, other.parts, other.placed), (["adc", "io", "later"], ["J1", "U20", "U9"], 2))
        self.assertEqual(self.plan.flows[0][2:], (2, 1))  # the filtered pair of an amplifier to the converter

    def test_zones_have_their_parts_what_they_need_and_what_stays_within(self):
        every = []
        def walk(zone):
            every.append(zone)
            for inner in zone.inside:
                walk(inner)
        for zone in self.plan.zones:
            walk(zone)
        # every part is in one top zone; ground ties nothing, so the connector with only the clock is by itself
        self.assertEqual(sorted(part for zone in self.plan.zones for part in zone.parts), sorted(self.picture.parts))
        self.assertTrue(all(zone.area > 0 and (zone.box is None) == (zone.placed == 0) for zone in every))

    def test_what_flows_between_zones_is_counted_in_nets_and_told(self):
        names = {zone.name for zone in self.plan.zones}
        for a, b, count, pairs in self.plan.flows:
            self.assertTrue(a in names and b in names and a != b and count >= 1 and pairs <= count)
        lines = k.floorplan_told(self.plan)
        self.assertTrue(lines[0].startswith(f"{len(self.plan.zones)} zones; "))
        self.assertTrue(any(line.startswith("amp-1: 6 parts") for line in lines))
        self.assertTrue(any(line.startswith("  amp-1 (C10, R10, U1): 3 parts") for line in lines))


if __name__ == "__main__":
    unittest.main()
