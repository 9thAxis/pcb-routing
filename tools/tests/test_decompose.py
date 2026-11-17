"""The work of a board as steps: blocks from the sheets of the schematic, nets by kind, in a designer's order."""
import unittest

import support

k = support.bridge()


def board():
    """Two amplifiers (sheets amp-1 and amp-2), each with its filter drawn on its own sheet but placed by the
    converter (sheet adc) 15 mm away; a clock from the converter to a connector; a part not placed yet."""
    parts, pads = {}, []
    def part(reference, sheet, x, y, *nets):
        parts[reference] = k.Part(at=[x, y], sheet=sheet, file=sheet + ".kicad_sch")
        for n, net in enumerate(nets):
            pads.append(support.land(k, net, x + 0.6 * n, y, half=0.2, part=reference))
    for n, x in ((1, 2.0), (2, 9.0)):
        part(f"U{n}", f"amp-{n}", x, 8.0, f"IN{n}", f"MID{n}", f"OUT{n}_P", f"OUT{n}_N")
        part(f"R{n}0", f"amp-{n}", x + 1.5, 8.0, f"IN{n}", f"MID{n}")
        part(f"C{n}0", f"amp-{n}", x + 1.5, 9.0, f"MID{n}", "GND")
        part(f"R{n}1", f"amp-{n}", x, 1.5, f"OUT{n}_P", f"F{n}_P")
        part(f"R{n}2", f"amp-{n}", x + 0.8, 1.5, f"OUT{n}_N", f"F{n}_N")
        part(f"C{n}1", f"amp-{n}", x + 0.4, 2.3, f"F{n}_P", f"F{n}_N")
    part("U9", "adc", 16.0, 2.0, "F1_P", "F1_N", "F2_P", "F2_N", "CLK", "GND")
    part("J1", "io", 18.0, 9.0, "CLK", "GND")
    part("U20", "later", 90.0, 90.0, "CLK", "SPARE")
    picture = support.picture(k, pads)
    picture.parts = parts
    return picture


class Decompose(unittest.TestCase):
    def setUp(self):
        self.picture = board()
        self.plan = k.decompose(self.picture, k.Rules.of(self.picture, k.Settings()))
        self.steps = {step.name: step for step in self.plan.steps}

    def test_a_sheet_whose_parts_stand_in_two_places_is_two_blocks_and_parts_off_the_board_are_not_placed(self):
        names = [block.name for block in self.plan.blocks]
        self.assertIn("amp-1 (at 3, 8)", names)
        self.assertIn("amp-1 (at 2, 2)", names)
        self.assertIn("adc", names)
        later = next(block for block in self.plan.blocks if block.name == "later")
        self.assertEqual((later.parts, later.parked), ([], ["U20"]))

    def test_the_nets_within_a_block_come_first_then_pairs_then_what_is_careful_then_the_rest(self):
        kinds = [step.kind for step in self.plan.steps]
        self.assertEqual(kinds, sorted(kinds, key=k.STEP_ORDER.index))
        # the input and the middle of an amplifier never leave its block
        self.assertEqual(self.steps["amp-1 (at 3, 8)"].nets, ["IN1", "MID1"])
        self.assertEqual((self.steps["amp-1 (at 3, 8)"].kind, self.steps["amp-1 (at 3, 8)"].open), ("local", 3))
        # its outputs go to its filter by the converter: a pair between two blocks
        pair = self.steps["OUT1_N = OUT1_P"]
        self.assertEqual((pair.kind, pair.blocks, pair.open), ("pair", ["amp-1 (at 2, 2)", "amp-1 (at 3, 8)"], 2))
        self.assertAlmostEqual(pair.span, 2 * 6.5, delta=1.5)
        # the filtered pair from there to the converter is another, and longer: it comes before
        order = [step.name for step in self.plan.steps if step.kind == "pair"]
        self.assertLess(order.index("F1_N = F1_P"), order.index("OUT1_N = OUT1_P"))
        # ground, joined by no zone here, is last
        self.assertEqual((self.plan.steps[-1].kind, self.plan.steps[-1].nets), ("supply", ["GND"]))

    def test_a_net_with_a_pad_on_a_part_that_is_not_placed_waits(self):
        self.assertEqual(self.plan.waiting, {"CLK": ["U20"], "SPARE": ["U20"]})
        self.assertNotIn("CLK", [net for step in self.plan.steps for net in step.nets])

    def test_the_plan_is_told_with_its_steps_numbered_and_what_waits(self):
        lines = k.plan_told(self.plan, every=True)
        self.assertEqual(lines[0], "14 parts on the board in 6 blocks; 1 parts not placed yet")
        self.assertTrue(any(line.startswith("pair: ") for line in lines))
        self.assertTrue(any("OUT2_N, OUT2_P" in line for line in lines))
        self.assertIn("  CLK: U20", lines)


if __name__ == "__main__":
    unittest.main()
