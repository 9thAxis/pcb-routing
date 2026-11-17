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
        (ground,) = self.plan.zones  # (one ground: one domain, with the zones within it)
        self.assertEqual((ground.name, len(ground.parts)), ("GND", 15))
        amp = next(zone for zone in ground.inside if zone.name == "amp-1")
        self.assertEqual(sorted(inner.parts for inner in amp.inside), [["C10", "R10", "U1"], ["C11", "R11", "R12"]])
        self.assertEqual((amp.within, amp.placed, amp.sheets), (4, 6, ["amp-1"]))
        self.assertEqual(self.plan.apart, [])
        # the clock ties the converter, the connector and the part not placed yet
        other = next(zone for zone in ground.inside if "adc" in zone.sheets)
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
        names = {f"GND: {zone.name}" for zone in self.plan.zones[0].inside}
        for a, b, count, pairs in self.plan.flows:
            self.assertTrue(a in names and b in names and a != b and count >= 1 and pairs <= count)
        lines = k.floorplan_told(self.plan)
        self.assertTrue(lines[0].startswith("1 zones; "))
        self.assertTrue(any(line.startswith("  amp-1: 6 parts") for line in lines))
        self.assertTrue(any(line.startswith("    amp-1 (C10, R10, U1): 3 parts") for line in k.floorplan_told(self.plan, 3)))


def two_grounds():
    """A chip on each of two grounds with its supply and bypass capacitors, an isolator on both, a regulator
    from one supply to another, and a filtered signal with one capacitor to ground."""
    parts, pads = {}, []
    def part(reference, x, y, *nets):
        parts[reference] = k.Part(at=[x, y], sheet="all", file="all.kicad_sch")
        pads.extend(support.land(k, net, x + 0.6 * n, y, half=0.2, part=reference) for n, net in enumerate(nets))
    part("U1", 2.0, 2.0, "A_3V3", "A_GND", "SIG", "FILT")
    part("C1", 2.0, 3.0, "A_3V3", "A_GND")
    part("C2", 3.0, 3.0, "A_3V3", "A_GND")
    part("C3", 4.0, 3.0, "FILT", "A_GND")
    part("R1", 5.0, 3.0, "FILT", "SIG")
    part("U2", 2.0, 5.0, "RAW", "A_3V3", "A_GND")
    part("C4", 3.0, 5.0, "RAW", "A_GND")
    part("C5", 4.0, 5.0, "RAW", "A_GND")
    part("U3", 10.0, 2.0, "SIG", "A_3V3", "A_GND", "OUT", "B_VCC", "FIELD_0V")
    part("U4", 16.0, 2.0, "OUT", "B_VCC", "FIELD_0V")
    part("C6", 16.0, 3.0, "B_VCC", "FIELD_0V")
    part("C7", 17.0, 3.0, "B_VCC", "FIELD_0V")
    part("R2", 18.0, 3.0, "OUT", "LED")
    part("D1", 19.0, 3.0, "LED", "FIELD_0V")
    picture = support.picture(k, pads)
    picture.parts = parts
    return picture


class Power(unittest.TestCase):
    def setUp(self):
        self.picture = two_grounds()
        self.found = k.power(self.picture)

    def test_grounds_are_named_and_a_supply_is_what_two_capacitors_bypass_whatever_it_is_called(self):
        self.assertEqual(k.grounds_of(self.picture), ["A_GND", "FIELD_0V"])
        rails = {domain.ground: [(rail.net, len(rail.parts), rail.capacitors) for rail in domain.rails]
                 for domain in self.found.domains}
        # (the filtered signal has one capacitor to ground: no supply)
        self.assertEqual(rails, {"A_GND": [("A_3V3", 5, 2), ("RAW", 3, 2)], "FIELD_0V": [("B_VCC", 4, 2)]})

    def test_parts_are_of_the_ground_they_stand_on_and_one_on_two_is_between(self):
        domains = {domain.ground: domain.parts for domain in self.found.domains}
        # the resistor with no pad on a ground or a supply goes with the parts it shares nets with
        self.assertEqual(domains["FIELD_0V"], ["C6", "C7", "D1", "R2", "U4"])
        self.assertEqual(domains["A_GND"], ["C1", "C2", "C3", "C4", "C5", "R1", "U1", "U2", "U3"])
        self.assertEqual(self.found.between, [("U3", ["A_GND", "FIELD_0V"])])
        self.assertEqual(self.found.adrift, [])
        # the regulator stands on two supplies; the isolator on two as well, one of each domain
        self.assertEqual(self.found.links, [("A_3V3", "B_VCC", ["U3"]), ("A_3V3", "RAW", ["U2"])])

    def test_the_zones_are_found_within_each_domain_and_supplies_tie_nothing(self):
        plan = k.floorplan(self.picture, k.Rules.of(self.picture, k.Settings()))
        self.assertEqual([zone.name for zone in plan.zones], ["A_GND", "FIELD_0V"])
        self.assertEqual(plan.zones[1].parts, ["C6", "C7", "D1", "R2", "U4"])
        lines = k.power_told(self.found, self.picture)
        self.assertEqual(lines[0], "2 domains (grounds); 1 parts stand on two or more; 0 parts that nothing places in one")
        self.assertIn("  A_GND | FIELD_0V: 1 parts: U3", lines)


if __name__ == "__main__":
    unittest.main()
