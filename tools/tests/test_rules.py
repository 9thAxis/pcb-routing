"""The board-to-problem rules, each on its own. No KiCad."""
import json, tempfile, unittest
from pathlib import Path

import support

k = support.bridge()


def rules(ignored=(), slack=0.02, step=0.05, layers=None, **minima):
    base = dict(clearance=0.15, hole_clearance=0.25, edge_clearance=0.3, track_width=0.15, via_size=0.5,
                through_drill=0.3, annular_width=0.1, hole_to_hole=0.25)
    base.update(minima)
    classes = {"A": k.NetClass(0.2, 0.25, 0.6, 0.3), "B": k.NetClass(0.2, 0.25, 0.6, 0.3),
               "HV": k.NetClass(0.8, 0.5, 1.0, 0.5), "FINE": k.NetClass(0.1, 0.1, 0.3, 0.1)}
    return k.Rules(k.Minima(**base), classes, frozenset(ignored), k.Settings(step=step, slack=slack, layers=layers))


class Where(unittest.TestCase):
    def test_a_few_nets_of_a_large_board_are_routed_in_the_ground_round_their_pads(self):
        board = [0.0, 0.0, 200.0, 100.0]
        # half of what they span on every side, 10 mm at the least, and never beyond the board
        self.assertEqual(rules().about([[50.0, 40.0], [90.0, 44.0]], board), [30.0, 20.0, 110.0, 64.0])
        self.assertEqual(rules().about([[5.0, 40.0], [8.0, 44.0]], board), [0.0, 30.0, 18.0, 54.0])
        self.assertEqual(rules().about([[5.0, 5.0], [195.0, 95.0]], board), board)
        self.assertEqual(rules().about([], board), board)


class Clearances(unittest.TestCase):
    def test_class_clearance_with_slack(self):
        self.assertAlmostEqual(rules().clearance("A"), 0.22)

    def test_board_minimum_wins_over_a_smaller_class(self):
        self.assertAlmostEqual(rules().clearance("FINE"), 0.17)

    def test_least_clearance_is_the_smallest_of_any_net(self):
        self.assertAlmostEqual(rules().least_clearance, 0.17)
        self.assertAlmostEqual(k.Rules(k.Minima(clearance=0.2), {}, settings=k.Settings(slack=0.02)).least_clearance,
                               0.22)

    def test_clearance_to_a_stricter_class(self):
        r = rules()
        self.assertEqual(set(r.stricter("A")), {"HV"})
        self.assertAlmostEqual(r.stricter("A")["HV"], 0.82)
        self.assertEqual(set(r.stricter("FINE")), {"A", "B", "HV"})
        self.assertEqual(r.stricter("HV"), {})

    def test_new_copper_keeps_the_most_any_routed_net_asks(self):
        r = rules()
        self.assertAlmostEqual(r.new_clearance(["A", "FINE"]), 0.22)
        self.assertAlmostEqual(r.new_clearance(["A", "HV"]), 0.82)
        self.assertAlmostEqual(r.new_clearance([]), 0.17)

    def test_slack_is_added_once_everywhere(self):
        tight, loose = rules(slack=0.0), rules(slack=0.05)
        hole = k.Hole([0, 0], 1.0, None)
        for name, value in (("clearance", lambda r: r.clearance("A")), ("stricter", lambda r: r.stricter("A")["HV"]),
                            ("new", lambda r: r.new_clearance(["A"])), ("edge", lambda r: r.edge_margin()),
                            ("spacing", lambda r: r.hole_spacing(0.3, 1.0)), ("least", lambda r: r.least_clearance)):
            with self.subTest(name):
                self.assertAlmostEqual(value(loose) - value(tight), 0.05)
        # growth is beyond the least clearance, which has the slack too: it does not move with it
        self.assertAlmostEqual(loose.hole_growth(hole), tight.hole_growth(hole))


class PadGrowth(unittest.TestCase):
    def pad(self, **more):
        return support.land(k, "A", 1.0, 1.0, **more)

    def test_plain_pad_is_not_grown(self):
        self.assertEqual(rules().pad_growth(self.pad(), "F.Cu"), 0.0)

    def test_nothing_on_a_layer_without_copper_or_opening(self):
        self.assertIsNone(rules().pad_growth(self.pad(), "B.Cu"))

    def test_own_clearance_override(self):
        # 0.5 asked, 0.15 (the least of any net) kept anyway; the slack is in both
        self.assertAlmostEqual(rules().pad_growth(self.pad(clearance=0.5), "F.Cu"), 0.5 - 0.15)

    def test_an_override_under_the_least_clearance_does_not_shrink_the_pad(self):
        self.assertEqual(rules().pad_growth(self.pad(clearance=0.05), "F.Cu"), 0.0)

    def test_mask_opening_wider_than_the_clearance(self):
        self.assertAlmostEqual(rules().pad_growth(self.pad(openings={"F.Cu": 0.3}), "F.Cu"), 0.3 + 0.01 - 0.17)

    def test_small_mask_opening_changes_nothing(self):
        self.assertEqual(rules().pad_growth(self.pad(openings={"F.Cu": 0.05}), "F.Cu"), 0.0)

    def test_the_larger_of_override_and_opening(self):
        pad = self.pad(clearance=0.5, openings={"F.Cu": 0.3})
        self.assertAlmostEqual(rules().pad_growth(pad, "F.Cu"), 0.35)

    def test_opening_on_the_side_without_copper(self):
        pad = self.pad(openings={"F.Cu": 0.05, "B.Cu": 0.3})
        self.assertAlmostEqual(rules().pad_growth(pad, "B.Cu"), 0.14)
        # the pad's own clearance is about its copper: it does not grow the bare opening
        pad.clearance = 0.9
        self.assertAlmostEqual(rules().pad_growth(pad, "B.Cu"), 0.14)

    def test_ignored_mask_check(self):
        r = rules(ignored={"solder_mask_bridge"})
        pad = self.pad(openings={"F.Cu": 0.3, "B.Cu": 0.3})
        self.assertEqual(r.pad_growth(pad, "F.Cu"), 0.0)
        self.assertIsNone(r.pad_growth(pad, "B.Cu"))

    def test_pad_with_an_opening_and_no_copper_at_all(self):
        pad = k.Pad(net=None, at=[1, 1], layers=[], shapes={"F.Cu": [support.square(k, 1, 1)]}, openings={"F.Cu": 0.0})
        self.assertEqual(rules().pad_growth(pad, "F.Cu"), 0.0)
        self.assertIsNone(rules().pad_growth(pad, "B.Cu"))
        self.assertIsNone(rules(ignored={"solder_mask_bridge"}).pad_growth(pad, "F.Cu"))


class Holes(unittest.TestCase):
    def test_hole_clearance_beyond_the_least_clearance(self):
        self.assertAlmostEqual(rules().hole_growth(k.Hole([0, 0], 1.0, "A")), 0.25 - 0.15)

    def test_the_pads_own_clearance_if_it_is_more(self):
        self.assertAlmostEqual(rules().hole_growth(k.Hole([0, 0], 1.0, "A", clearance=0.6)), 0.45)

    def test_ignored_hole_check(self):
        r = rules(ignored={"hole_clearance"})
        self.assertEqual(r.hole_growth(k.Hole([0, 0], 1.0, "A")), 0.0)
        self.assertAlmostEqual(r.hole_growth(k.Hole([0, 0], 1.0, "A", clearance=0.6)), 0.45)

    def test_spacing_counts_both_diameters(self):
        r = rules()
        self.assertAlmostEqual(r.hole_spacing(0.3, 0.3), 0.3 + 0.25 + 0.02)
        self.assertAlmostEqual(r.hole_spacing(1.0, 0.3), 0.65 + 0.25 + 0.02)

    def test_edge_margin(self):
        self.assertAlmostEqual(rules().edge_margin(), 0.32)
        self.assertEqual(rules(edge_clearance=0.0).edge_margin(), 0.0)
        self.assertEqual(rules(ignored={"copper_edge_clearance"}).edge_margin(), 0.0)


class Sizes(unittest.TestCase):
    def test_track_width(self):
        r = rules()
        self.assertEqual(r.track_width("A"), 0.25)
        self.assertEqual(r.track_width("FINE"), 0.15)

    def test_narrow_width_is_under_the_narrowest_pad(self):
        self.assertAlmostEqual(rules().narrow_width("A", 0.3), 0.2)

    def test_narrow_width_never_under_the_board_minimum(self):
        self.assertAlmostEqual(rules().narrow_width("A", 0.2), 0.15)
        self.assertAlmostEqual(rules(track_width=0.0).narrow_width("A", 0.12), 0.1)

    def test_narrow_width_never_wider_than_the_class(self):
        self.assertEqual(rules().narrow_width("A", 2.0), 0.25)
        self.assertEqual(rules().narrow_width("A", None), 0.25)

    def test_via(self):
        r = rules()
        self.assertEqual(r.via("A"), [0.6, 0.3])
        self.assertEqual(r.via("FINE"), [0.5, 0.3])  # the board's smallest via and drill
        size, drill = rules(annular_width=0.2).via("A")
        self.assertAlmostEqual(size, 0.7)  # the drill and the smallest ring both sides


class Where(unittest.TestCase):
    def test_window_is_snapped_outwards_to_the_step(self):
        self.assertEqual(rules(step=0.25).window([0.1, 0.1, 20.1, 10.3]), [-1.0, -1.0, 21.25, 11.5])

    def test_window_on_the_grid_already(self):
        self.assertEqual(rules(step=0.5).window([0.0, 0.0, 20.0, 10.0]), [-1.0, -1.0, 21.0, 11.0])

    def test_window_cells_are_multiples_of_the_step(self):
        window = rules(step=0.05).window([3.333, -7.777, 41.111, 12.345])
        for v in window:
            self.assertAlmostEqual(v / 0.05, round(v / 0.05), places=6)
        self.assertLessEqual(window[0], 3.333 - 1.0)
        self.assertGreaterEqual(window[2], 41.111 + 1.0)

    def test_routing_layers(self):
        self.assertEqual(rules().routing_layers(["F.Cu", "In1.Cu", "B.Cu"]), ["F.Cu", "In1.Cu", "B.Cu"])
        self.assertEqual(rules(layers={"B.Cu", "F.Cu"}).routing_layers(["F.Cu", "In1.Cu", "B.Cu"]), ["F.Cu", "B.Cu"])


class IgnoredChecks(unittest.TestCase):
    def test_read_from_the_project_beside_the_board(self):
        with tempfile.TemporaryDirectory() as tmp:
            severities = {"solder_mask_bridge": "ignore", "hole_clearance": "error", "silk_overlap": "ignore",
                          "clearance": "warning"}
            (Path(tmp) / "b.kicad_pro").write_text(json.dumps({"board": {"design_settings": {"rule_severities": severities}}}))
            self.assertEqual(k.ignored_checks(Path(tmp) / "b.kicad_pcb"), {"solder_mask_bridge", "silk_overlap"})

    def test_no_project_or_a_broken_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(k.ignored_checks(Path(tmp) / "none.kicad_pcb"), frozenset())
            (Path(tmp) / "bad.kicad_pro").write_text("{")
            self.assertEqual(k.ignored_checks(Path(tmp) / "bad.kicad_pcb"), frozenset())
            (Path(tmp) / "bare.kicad_pro").write_text("{}")
            self.assertEqual(k.ignored_checks(Path(tmp) / "bare.kicad_pcb"), frozenset())


if __name__ == "__main__":
    unittest.main()
