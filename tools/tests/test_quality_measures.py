"""The route quality measures, each on a hand-made dump. Needs shapely and numpy; skipped without them."""
import math, unittest

import quality_support as q
from quality_support import board, pad, track, via

m = q.scoring()


def shapes(tracks, vias=(), pads=(), net="A", clearance=0.2):
    """What net_shapes counts for one net."""
    counts, gaps = m.net_shapes(board(pads, tracks, vias), clearance, {net: None})
    return counts[net], gaps[net]


class Geometry(unittest.TestCase):
    def test_points_closer_than_a_micron_are_one_place(self):
        self.assertEqual(m.key([1.00004, 2.0]), m.key([1.0, 2.00003]))
        self.assertNotEqual(m.key([1.001, 2.0]), m.key([1.0, 2.0]))

    def test_tree_of_two_pads_is_their_distance(self):
        self.assertEqual(m.tree_length([[0, 0], [3, 4]]), (5.0, 2))

    def test_tree_joins_each_place_once_by_the_shortest_way(self):
        length, count = m.tree_length([[0, 0], [10, 0], [10, 1], [0, 0]])
        self.assertEqual(count, 3)
        self.assertAlmostEqual(length, 11.0)

    def test_tree_of_one_place_is_nothing(self):
        self.assertEqual(m.tree_length([[1, 1], [1, 1]]), (0.0, 1))
        self.assertEqual(m.tree_length([]), (0.0, 0))

    def test_turn(self):
        self.assertAlmostEqual(m.turn((-1, 0), (1, 0)), 0)   # straight on
        self.assertAlmostEqual(m.turn((-1, 0), (0, 1)), 90)
        self.assertAlmostEqual(m.turn((1, 0), (1, 0)), 180)  # back on itself

    def test_median_by_length(self):
        self.assertEqual(m.median_by_length([(1.0, 1.0), (5.0, 3.0)]), 5.0)
        self.assertEqual(m.median_by_length([(1.0, 3.0), (5.0, 1.0)]), 1.0)
        self.assertEqual(m.median_by_length([]), 0)

    def test_upper_median(self):
        self.assertEqual(m.upper_median([3, 1, 2, 4]), 3)
        self.assertTrue(math.isnan(m.upper_median([])))


class NetSelection(unittest.TestCase):
    def reference(self, **more):
        pads = [pad("A", 0, 0), pad("A", 3, 4), pad("B", 0, 9), pad("B", 5, 9), pad("LONE", 9, 9), pad("", 20, 20), pad("POUR", 0, 20), pad("POUR", 5, 20)]
        tracks = [track("A", (0, 0), (3, 4)), track("LONE", (9, 9), (12, 9)), track("POUR", (0, 20), (5, 20))]
        return board(pads, tracks, zone_nets=["POUR"], **more)

    def test_compared_nets_are_those_the_reference_routed_with_tracks_between_two_or_more_pads(self):
        # B has no track, LONE one pad, POUR a zone, and pads without a net do not count
        self.assertEqual(m.compared_nets(self.reference()), {"A": (5.0, 1)})

    def test_pads_at_one_place_are_one_pad(self):
        d = board([pad("A", 0, 0, layers=("F.Cu",)), pad("A", 0, 0, layers=("B.Cu",))], [track("A", (0, 0), (1, 0))])
        self.assertEqual(m.compared_nets(d), {})

    def test_a_net_of_n_pads_has_n_less_one_connections(self):
        d = board([pad("A", 0, 0), pad("A", 4, 0), pad("A", 4, 3)], [track("A", (0, 0), (4, 0))])
        self.assertEqual(m.compared_nets(d), {"A": (7.0, 2)})

    def test_tracked_nets(self):
        self.assertEqual(m.tracked_nets(self.reference()), {"A", "LONE"})

    def test_same_pads(self):
        a = self.reference()
        self.assertTrue(m.same_pads(a, board(reversed(a["pads"]))))
        self.assertFalse(m.same_pads(a, board(a["pads"][1:])))
        self.assertFalse(m.same_pads(a, board([pad("X", 0, 0)] + a["pads"][1:])))

    def test_a_board_is_usable_when_every_routing_says_it_is_complete(self):
        ref = self.reference()
        self.assertIsNone(m.unusable(ref, {"x": self.reference(unconnected=0)}))
        self.assertIn("3 open", m.unusable(ref, {"x": self.reference(unconnected=0), "y": self.reference(unconnected=3)}))

    def test_a_dump_that_does_not_say_is_not_taken_as_complete(self):
        self.assertIn("does not say", m.unusable(self.reference(), {"x": self.reference()}))

    def test_the_caller_can_vouch_for_completeness(self):
        self.assertIsNone(m.unusable(self.reference(), {"x": self.reference(unconnected=3)}, assume_complete=True))

    def test_the_reference_need_not_be_complete(self):
        self.assertIsNone(m.unusable(self.reference(unconnected=4), {"x": self.reference(unconnected=0)}))

    def test_another_board_is_never_usable(self):
        other = board([pad("A", 0, 0)], unconnected=0)
        self.assertIn("pads differ", m.unusable(self.reference(), {"x": other}, assume_complete=True))


class Rules(unittest.TestCase):
    RULES = dict(clearance=0.15, track_widths={"Default": 0.25, "POWER": 0.5, "Empty": None},
                 patterns=[["VCC*", "POWER"], ["SIG", "Empty"]], assignments={"GND": ["POWER"], "OLD": "POWER"})

    def test_the_reference_s_rules_come_first(self):
        self.assertEqual(m.board_rules(board(rules={"clearance": 0.1}), {"x": board(rules={"clearance": 0.3})}), {"clearance": 0.1})

    def test_else_those_of_the_first_routing_that_has_any(self):
        self.assertEqual(m.board_rules(board(), {"x": board(), "y": board(rules={"clearance": 0.3})}), {"clearance": 0.3})
        self.assertEqual(m.board_rules(board(), {"x": board()}), {})

    def test_clearance(self):
        self.assertEqual(m.clearance_of(self.RULES), 0.15)
        self.assertEqual(m.clearance_of({}), 0.2)
        self.assertEqual(m.clearance_of({"clearance": None}), 0.2)

    def test_class_width_by_pattern_assignment_and_default(self):
        self.assertEqual(m.class_width(self.RULES, "VCC_3V3"), 0.5)
        self.assertEqual(m.class_width(self.RULES, "GND"), 0.5)
        self.assertEqual(m.class_width(self.RULES, "OLD"), 0.5)
        self.assertEqual(m.class_width(self.RULES, "OTHER"), 0.25)
        self.assertEqual(m.class_width(self.RULES, "SIG"), 0.25)  # its class names no width
        self.assertEqual(m.class_width({}, "GND"), 0.0)


class Corners(unittest.TestCase):
    def test_an_l_shaped_track_has_one_corner_and_it_is_sharp(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (5, 5))])
        self.assertEqual((counts["corners"], counts["sharp"]), (1, 1))

    def test_a_bend_of_45_degrees_is_a_corner_but_not_sharp(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (8, 3))])
        self.assertEqual((counts["corners"], counts["sharp"]), (1, 0))

    def test_a_turn_back_of_135_degrees_is_sharp(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (2, 3))])
        self.assertEqual((counts["corners"], counts["sharp"]), (1, 1))

    def test_a_t_junction_is_not_a_corner(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (5, 5)), track("A", (5, 0), (9, 0))])
        self.assertEqual(counts["corners"], 0)

    def test_two_segments_in_line_are_not_a_corner(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (9, 0))])
        self.assertEqual(counts["corners"], 0)

    def test_the_segments_are_found_whichever_way_they_are_drawn(self):
        counts, _ = shapes([track("A", (5, 0), (0, 0)), track("A", (5, 5), (5, 0))])
        self.assertEqual(counts["corners"], 1)

    def test_a_change_of_layer_is_not_a_corner(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (5, 5), layer="B.Cu")], [via("A", 5, 0)])
        self.assertEqual((counts["corners"], counts["vias"]), (0, 1))

    def test_another_net_s_segment_at_the_same_place_does_not_count(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("B", (5, 0), (5, 5))])
        self.assertEqual(counts["corners"], 0)

    def test_an_arc_is_not_a_corner(self):
        counts, _ = shapes([track("A", (0, 0), (5, 0)), track("A", (5, 0), (6, 1), mid=[5.7, 0.3])])
        self.assertEqual(counts["corners"], 0)

    def test_a_staircase_counts_every_bend(self):
        points = [(0, 0), (2, 0), (2, 2), (4, 2), (4, 4)]
        counts, _ = shapes([track("A", a, b) for a, b in zip(points, points[1:])])
        self.assertEqual((counts["corners"], counts["sharp"]), (3, 3))


class LengthAndShortSegments(unittest.TestCase):
    def test_length_is_the_sum_of_the_segments(self):
        counts, _ = shapes([track("A", (0, 0), (3, 0)), track("A", (3, 0), (3, 4)), track("B", (0, 9), (9, 9))])
        self.assertAlmostEqual(counts["mm"], 7.0)

    def test_a_segment_shorter_than_twice_its_width_is_short(self):
        counts, _ = shapes([track("A", (0, 0), (0.39, 0), w=0.2), track("A", (0.39, 0), (0.79, 0.01), w=0.2), track("A", (5, 5), (5.5, 5), w=0.3)])
        self.assertEqual(counts["short"], 2)

    def test_vias_of_the_net_are_counted(self):
        counts, _ = shapes([track("A", (0, 0), (3, 0))], [via("A", 3, 0), via("A", 1, 0), via("B", 2, 2)])
        self.assertEqual(counts["vias"], 2)


class Spacing(unittest.TestCase):
    def figures(self, other_at, other_net="B", clearance=0.2):
        d = board(tracks=[track("A", (0, 0), (10, 0), w=0.2), track(other_net, (0, other_at), (10, other_at), w=0.2)])
        tally, _ = m.general_measures(d, clearance, {"A": (10.0, 1)})
        return m.general_figures(tally, 10.0)

    def test_a_track_at_the_clearance_from_another_net_is_tight(self):
        f = self.figures(0.4)  # 0.2 mm of air between two tracks 0.2 mm wide
        self.assertAlmostEqual(f["tight_percent"], 100.0)
        self.assertAlmostEqual(f["median_gap"], 1.0, places=5)

    def test_a_track_three_clearances_away_is_not(self):
        f = self.figures(0.8)
        self.assertAlmostEqual(f["tight_percent"], 0.0)
        self.assertAlmostEqual(f["median_gap"], 3.0, places=5)

    def test_gaps_are_capped_at_five_clearances(self):
        self.assertEqual(self.figures(30.0)["median_gap"], 5.0)

    def test_copper_of_the_same_net_is_not_a_neighbour(self):
        self.assertEqual(self.figures(0.4, other_net="A")["median_gap"], 5.0)

    def test_copper_on_another_layer_is_not_a_neighbour(self):
        d = board(tracks=[track("A", (0, 0), (10, 0)), track("B", (0, 0.4), (10, 0.4), layer="B.Cu")])
        tally, _ = m.general_measures(d, 0.2, {"A": (10.0, 1)})
        self.assertEqual(m.general_figures(tally, 10.0)["tight_percent"], 0.0)

    def test_a_via_of_another_net_stands_on_every_layer(self):
        d = board(tracks=[track("A", (0, 0), (10, 0), layer="B.Cu")], vias=[via("B", 5, 0.5, d=0.6)])
        _, gaps = m.net_shapes(d, 0.2, {"A": None})
        self.assertAlmostEqual(min(g for g, _ in gaps["A"]), 0.5, places=2)  # 0.5 - 0.3 - 0.1 = 0.1 mm of air

    def test_a_pad_of_another_net_is_a_neighbour_and_the_net_s_own_pad_is_not(self):
        tracks = [track("A", (0, 0), (10, 0))]
        _, near = shapes(tracks, pads=[pad("B", 5, 0.8)])
        _, own = shapes(tracks, pads=[pad("A", 5, 0.8)])
        self.assertAlmostEqual(min(g for g, _ in near), 1.0, places=5)  # 0.8 - 0.5 - 0.1 = 0.2 mm
        self.assertEqual(min(g for g, _ in own), 5.0)

    def test_tracks_are_measured_in_pieces_of_half_a_millimetre(self):
        _, gaps = shapes([track("A", (0, 0), (2.2, 0))])
        self.assertEqual(len(gaps), 5)
        self.assertAlmostEqual(sum(mm for _, mm in gaps), 2.2)

    def test_tight_share_is_by_length_in_whole_pieces(self):
        d = board(tracks=[track("A", (0, 0), (10, 0)), track("B", (0, 0.4), (2.5, 0.4))])
        tally, _ = m.general_measures(d, 0.2, {"A": (10.0, 1)})
        # five pieces beside the other track, and the sixth still within 1.1 clearances of its round end
        self.assertAlmostEqual(m.general_figures(tally, 10.0)["tight_percent"], 30.0)


class General(unittest.TestCase):
    def test_detour_of_a_two_pad_net(self):
        pads = [pad("A", 0, 0), pad("A", 3, 4)]
        reference = board(pads, [track("A", (0, 0), (3, 4))])
        routed = board(pads, [track("A", (0, 0), (3, 0)), track("A", (3, 0), (3, 4))], [via("A", 3, 0)])
        nets = m.compared_nets(reference)
        straight = m.general_figures(m.general_measures(reference, 0.2, nets)[0], 5.0)
        bent = m.general_figures(m.general_measures(routed, 0.2, nets)[0], 5.0)
        self.assertAlmostEqual(straight["detour"], 1.0)
        self.assertAlmostEqual(bent["detour"], 1.4)
        self.assertAlmostEqual(bent["length_vs_reference"], 1.4)
        self.assertAlmostEqual(bent["vias_per_100"], 100.0)
        self.assertAlmostEqual(bent["corners_per_connection"], 1.0)
        self.assertAlmostEqual(bent["sharp_per_100"], 100.0)
        self.assertAlmostEqual(bent["short_per_100"], 0.0)

    def test_figures_are_sums_over_nets_not_means_of_ratios(self):
        pads = [pad("A", 0, 0), pad("A", 10, 0), pad("B", 0, 5), pad("B", 1, 5)]
        d = board(pads, [track("A", (0, 0), (10, 0)), track("B", (0, 5), (0, 7)), track("B", (0, 7), (1, 7)), track("B", (1, 7), (1, 5))])
        tally, counts = m.general_measures(d, 0.2, m.compared_nets(d))
        self.assertAlmostEqual(m.general_figures(tally, 1.0)["detour"], 15.0 / 11.0)
        self.assertEqual(tally["connections"], 2)
        self.assertEqual(counts["B"]["corners"], 2)

    def test_tallies_add_up(self):
        a, b = m.Tally(), m.Tally()
        a["mm"], b["mm"], b["vias"] = 1.5, 2.0, 3
        a.samples.append((1.0, 1.0))
        b.samples.append((2.0, 1.0))
        a.add(b)
        self.assertEqual((a["mm"], a["vias"], a.samples), (3.5, 3, [(1.0, 1.0), (2.0, 1.0)]))


def bus_board(tracks, nets=("A", "B", "C"), vias=()):
    """Two connectors ten mm apart with their pads in a row at 1 mm, joined net by net."""
    pads = [pad(n, x, i, ref=ref, num=str(i + 1), w=0.5, h=0.5) for ref, x in (("J1", 0), ("J2", 10)) for i, n in enumerate(nets)]
    return board(pads, tracks, vias)


def straight(nets=("A", "B", "C")):
    return [track(n, (0, i), (10, i)) for i, n in enumerate(nets)]


class Buses(unittest.TestCase):
    def test_three_nets_side_by_side_on_two_parts_are_a_bus_of_two_pairs(self):
        self.assertEqual(m.bus_pairs(bus_board(straight())), ([("A", "B", 1.0), ("B", "C", 1.0)], 1))

    def test_two_nets_are_not_a_bus(self):
        self.assertEqual(m.bus_pairs(bus_board(straight(("A", "B")), nets=("A", "B"))), ([], 0))

    def test_nets_side_by_side_on_one_part_only_are_not_a_bus(self):
        d = bus_board(straight())
        d["pads"] = [p for p in d["pads"] if p["ref"] == "J1"] + [pad(n, 10 + 5 * i, 7 * i, ref=f"R{i}") for i, n in enumerate("ABC")]
        self.assertEqual(m.bus_pairs(d), ([], 0))

    def test_a_net_the_reference_did_not_route_or_poured_breaks_the_bus(self):
        self.assertEqual(m.bus_pairs(bus_board(straight()[:2])), ([], 0))
        poured = bus_board(straight())
        poured["zone_nets"] = ["B"]
        self.assertEqual(m.bus_pairs(poured), ([], 0))

    def test_the_pitch_is_the_larger_of_the_two_parts(self):
        d = bus_board(straight())
        for p in d["pads"]:
            if p["ref"] == "J2":
                p["at"][1] *= 2
        self.assertEqual([pitch for _, _, pitch in m.bus_pairs(d)[0]], [2.0, 2.0])

    def test_a_pair_that_is_side_by_side_on_three_parts_counts_once(self):
        d = bus_board(straight())
        d["pads"] += [pad(n, 20, i, ref="J3") for i, n in enumerate("ABC")]
        self.assertEqual(sorted(m.bus_pairs(d)[0]), [("A", "B", 1.0), ("B", "C", 1.0)])
        self.assertEqual(m.bus_pairs(d)[1], 1)

    def test_parallel_tracks_run_alongside_all_the_way(self):
        f = m.bus_figures(m.bus_measures(bus_board(straight()), [("A", "B", 1.0), ("B", "C", 1.0)]))
        self.assertEqual(f["pairs"], 2)
        self.assertAlmostEqual(f["alongside_percent"], 100.0)
        self.assertAlmostEqual(f["same_vias_percent"], 100.0)
        self.assertAlmostEqual(f["crossing_percent"], 0.0)
        self.assertAlmostEqual(f["length_mismatch_percent"], 0.0)

    def test_alongside_share_is_of_the_first_net_s_length_within_one_and_a_half_pitches(self):
        tracks = [track("A", (0, 0), (10, 0)), track("B", (0, 1), (5, 1))]
        t = m.bus_measures(bus_board(tracks), [("A", "B", 1.0)])
        self.assertAlmostEqual(t["alongside"], (5 + math.sqrt(1.5 ** 2 - 1)) / 10, places=2)
        self.assertAlmostEqual(t["mismatch"], 0.5)

    def test_tracks_on_different_layers_are_not_alongside(self):
        tracks = [track("A", (0, 0), (10, 0)), track("B", (0, 1), (10, 1), layer="B.Cu")]
        self.assertEqual(m.bus_measures(bus_board(tracks), [("A", "B", 1.0)])["alongside"], 0.0)

    def test_a_crossing_is_on_different_layers(self):
        over = [track("A", (0, 0), (10, 0)), track("B", (0, 1), (10, -1), layer="B.Cu")]
        self.assertEqual(m.bus_measures(bus_board(over), [("A", "B", 1.0)])["crossing"], 1)
        apart = [track("A", (0, 0), (10, 0)), track("B", (0, 1), (10, 1), layer="B.Cu")]
        self.assertEqual(m.bus_measures(bus_board(apart), [("A", "B", 1.0)])["crossing"], 0)

    def test_same_vias(self):
        d = bus_board(straight(), vias=[via("A", 5, 0)])
        t = m.bus_measures(d, [("A", "B", 1.0), ("B", "C", 1.0)])
        self.assertEqual(t["same vias"], 1)

    def test_a_pair_with_a_net_left_without_tracks_is_missing(self):
        t = m.bus_measures(bus_board(straight()[:1]), [("A", "B", 1.0)])
        self.assertEqual((t["pairs"], t["missing"]), (0, 1))


def decoupled(tracks, vias=(), chip_layers=("F.Cu",)):
    """A capacitor C1 between VCC and GND, 3 mm from pin 1 of U1 on VCC."""
    pads = [pad("VCC", 0, 0, ref="C1", num="1", w=0.6, h=0.6), pad("GND", 1, 0, ref="C1", num="2", w=0.6, h=0.6),
            pad("VCC", 0, 3, ref="U1", num="1", w=0.6, h=0.6, layers=chip_layers), pad("GND", 5, 3, ref="U1", num="2", w=0.6, h=0.6)]
    return board(pads, tracks, vias)


class CircuitNets(unittest.TestCase):
    def test_power_nets_by_name(self):
        for name in ("GND", "VCC", "+3V3", "+5V", "3V3", "VDD_IO", "AGND", "/power/VBUS", "+12V", "VBAT"):
            self.assertTrue(m.is_supply(name), name)
        for name in ("SDA", "Net-(U1-Pad3)", "VSYNC", "RESET", "D5", "/usb/D+"):
            self.assertFalse(m.is_supply(name), name)

    def test_a_power_net_needs_pads_at_two_places(self):
        d = board([pad("VCC", 0, 0), pad("VCC", 4, 0), pad("GND", 9, 9), pad("SIG", 1, 1), pad("SIG", 2, 2)])
        self.assertEqual(m.circuit_nets(d)["power"], ["VCC"])

    def test_a_capacitor_between_supply_and_ground_near_a_chip_pin_decouples_it(self):
        found = m.circuit_nets(decoupled([]))["decaps"]
        self.assertEqual([(net, c["ref"], pin["ref"]) for net, c, pin in found], [("VCC", "C1", "U1")])

    def test_a_capacitor_far_from_any_chip_pin_does_not(self):
        d = decoupled([])
        d["pads"][2]["at"] = [0, 5.1]
        self.assertEqual(m.circuit_nets(d)["decaps"], [])

    def test_a_capacitor_between_two_signals_or_on_a_connector_does_not(self):
        d = decoupled([])
        d["pads"][1]["net"] = "SIG"
        self.assertEqual(m.circuit_nets(d)["decaps"], [])
        d = decoupled([])
        d["pads"][2]["ref"] = "J1"
        self.assertEqual(m.circuit_nets(d)["decaps"], [])

    def test_the_nearest_chip_pin_is_taken(self):
        d = decoupled([])
        d["pads"].append(pad("VCC", 0, 2, ref="U2", num="7"))
        self.assertEqual(m.circuit_nets(d)["decaps"][0][2]["ref"], "U2")

    def test_crystal_nets_are_those_on_y_or_xtal_parts_that_are_not_power(self):
        d = board([pad("XI", 0, 0, ref="Y1"), pad("XI", 3, 0, ref="U1"), pad("GND", 0, 1, ref="Y1"), pad("GND", 3, 1, ref="U1"),
                   pad("XO", 0, 2, ref="XTAL2"), pad("XO", 3, 2, ref="U1"), pad("NC", 0, 3, ref="Y1"),
                   pad("SIG", 0, 4, ref="X3"), pad("SIG", 3, 4, ref="U1")])  # (X3 is a connector as often)
        self.assertEqual(m.circuit_nets(d)["crystals"], ["XI", "XO"])

    def test_differential_pairs_by_name(self):
        self.assertEqual(m.pair_halves(["USB_DP", "USB_DM", "CLK_P", "CLK_N", "TX+", "TX-", "ETH-P", "ETH-N", "SDA", "LONE_P"]),
                         [("CLK_N", "CLK_P"), ("ETH-N", "ETH-P"), ("TX+", "TX-"), ("USB_DM", "USB_DP")])

    def test_power_nets_are_not_pairs(self):
        d = board([pad(n, x, y) for y, n in enumerate(("VDD_P", "VDD_N", "RX_P", "RX_N")) for x in (0, 5)])
        self.assertEqual(m.circuit_nets(d)["pairs"], [("RX_N", "RX_P")])


class CopperPath(unittest.TestCase):
    def path(self, d):
        (net, capacitor, pin), = m.circuit_nets(d)["decaps"]
        return m.copper_path(d, net, capacitor, pin)

    def test_straight_from_pad_to_pad(self):
        self.assertEqual(self.path(decoupled([track("VCC", (0, 0), (0, 3))])), (3.0, False))

    def test_through_a_via(self):
        d = decoupled([track("VCC", (0, 0), (0, 1)), track("VCC", (0, 1), (0, 3), layer="B.Cu")], [via("VCC", 0, 1)], chip_layers=("F.Cu", "B.Cu"))
        self.assertEqual(self.path(d), (3.0, True))

    def test_a_via_on_the_net_but_not_on_the_way_does_not_count(self):
        d = decoupled([track("VCC", (0, 0), (0, 3)), track("VCC", (0, 3), (-2, 3))], [via("VCC", -2, 3)])
        self.assertEqual(self.path(d), (3.0, False))

    def test_the_way_without_a_via_wins_at_equal_length(self):
        tracks = [track("VCC", (0, 0), (0, 3)), track("VCC", (0, 0), (0, 3), layer="B.Cu")]
        d = decoupled(tracks, [via("VCC", 0, 0), via("VCC", 0, 3)])
        self.assertEqual(self.path(d), (3.0, False))

    def test_the_length_is_along_the_track(self):
        d = decoupled([track("VCC", (0, 0), (-2, 0)), track("VCC", (-2, 0), (-2, 3)), track("VCC", (-2, 3), (0, 3))])
        self.assertEqual(self.path(d), (7.0, False))

    def test_a_track_that_ends_on_another_branches_from_it(self):
        d = decoupled([track("VCC", (-4, 1), (4, 1)), track("VCC", (0, 0), (0, 1)), track("VCC", (0, 1), (0, 3))])
        self.assertEqual(self.path(d), (3.0, False))
        tee = decoupled([track("VCC", (0, 0), (0, 5)), track("VCC", (0, 3), (3, 3))])  # the pin sits where the branch leaves
        self.assertEqual(self.path(tee), (3.0, False))

    def test_a_track_that_stops_short_or_on_the_wrong_layer_does_not_join(self):
        self.assertIsNone(self.path(decoupled([track("VCC", (0, 0), (0, 2))])))
        self.assertIsNone(self.path(decoupled([track("VCC", (0, 0), (0, 3), layer="B.Cu")])))

    def test_another_net_s_track_does_not_join(self):
        self.assertIsNone(self.path(decoupled([track("GND", (0, 0), (0, 3))])))

    def test_a_track_end_anywhere_on_the_pad_is_on_it(self):
        d = decoupled([track("VCC", (0.2, 0.2), (0.2, 2.8))])
        length, through = self.path(d)
        self.assertAlmostEqual(length, 2.6)


class Circuit(unittest.TestCase):
    def measure(self, d, reference=None, rules=None, is_reference=False):
        reference = reference or d
        return m.circuit_measures(d, m.circuit_nets(reference), set(reference["zone_nets"]), rules or {}, is_reference)

    def test_decoupling_path_over_straight_distance(self):
        d = decoupled([track("VCC", (0, 0), (-2, 0)), track("VCC", (-2, 0), (-2, 3)), track("VCC", (-2, 3), (0, 3))])
        f = m.circuit_figures(self.measure(d))
        self.assertEqual(f["decaps"], 1)
        self.assertAlmostEqual(f["decap_path_over_straight_median"], 7 / 3)
        self.assertEqual(f["decap_via_between_percent"], 0.0)

    def test_a_decoupling_path_through_a_via(self):
        d = decoupled([track("VCC", (0, 0), (0, 1)), track("VCC", (0, 1), (0, 3), layer="B.Cu")], [via("VCC", 0, 1)], chip_layers=("F.Cu", "B.Cu"))
        f = m.circuit_figures(self.measure(d))
        self.assertAlmostEqual(f["decap_path_over_straight_median"], 1.0)
        self.assertEqual(f["decap_via_between_percent"], 100.0)

    def test_a_capacitor_not_joined_to_its_pin_by_tracks(self):
        f = m.circuit_figures(self.measure(decoupled([])))
        self.assertEqual((f["decaps"], f["decap_not_joined_directly_percent"]), (1, 100.0))
        self.assertTrue(math.isnan(f["decap_path_over_straight_median"]))

    def test_a_poured_supply_is_left_out_for_the_reference_only(self):
        d = decoupled([track("VCC", (0, 0), (0, 3))])
        d["zone_nets"] = ["VCC"]
        self.assertEqual(self.measure(d, is_reference=True)["decaps"], 0)
        self.assertEqual(self.measure(d)["decaps"], 1)

    def test_power_track_narrower_than_its_class(self):
        d = decoupled([track("VCC", (0, 0), (0, 2), w=0.5), track("VCC", (0, 2), (0, 3), w=0.2)], [via("VCC", 0, 2)])
        rules = dict(track_widths={"Default": 0.25, "POWER": 0.5}, patterns=[["VCC", "POWER"]])
        f = m.circuit_figures(self.measure(d, rules=rules))
        self.assertEqual(f["power_nets"], 1)  # GND has no tracks here
        self.assertAlmostEqual(f["power_narrow_percent"], 100 / 3)
        self.assertAlmostEqual(f["power_vias_per_100"], 100.0)
        self.assertAlmostEqual(f["power_detour"], 1.0)

    def test_without_rules_no_power_track_is_narrow(self):
        d = decoupled([track("VCC", (0, 0), (0, 3), w=0.1)])
        self.assertEqual(m.circuit_figures(self.measure(d))["power_narrow_percent"], 0.0)

    def test_a_power_net_poured_in_the_reference_is_not_measured(self):
        d = decoupled([track("VCC", (0, 0), (0, 3))])
        d["zone_nets"] = ["VCC"]
        self.assertEqual(self.measure(d)["power nets"], 0)

    def test_crystal_detour_and_vias(self):
        pads = [pad("XI", 0, 0, ref="Y1"), pad("XI", 3, 4, ref="U1")]
        d = board(pads, [track("XI", (0, 0), (3, 0)), track("XI", (3, 0), (3, 4))], [via("XI", 3, 0)])
        f = m.circuit_figures(self.measure(d))
        self.assertEqual((f["crystal_nets"], f["crystal_vias"]), (1, 1))
        self.assertAlmostEqual(f["crystal_detour"], 1.4)

    def pair(self, second, vias=()):
        pads = [pad(n, x, y, w=0.3, h=0.3) for y, n in ((0, "D_N"), (0.5, "D_P")) for x in (0, 10)]
        return board(pads, [track("D_N", (0, 0), (10, 0), w=0.2)] + second, vias)

    def test_a_pair_side_by_side_is_coupled(self):
        f = m.circuit_figures(self.measure(self.pair([track("D_P", (0, 0.5), (10, 0.5), w=0.2)])))
        self.assertEqual(f["pairs"], 1)
        self.assertAlmostEqual(f["pair_coupled_percent"], 100.0)
        self.assertAlmostEqual(f["pair_mismatch_percent"], 0.0)
        self.assertEqual(f["pair_same_vias_percent"], 100.0)

    def test_a_pair_that_splits_is_coupled_for_part_of_its_length(self):
        second = [track("D_P", (0, 0.5), (4, 0.5), w=0.2), track("D_P", (4, 0.5), (4, 6), w=0.2), track("D_P", (4, 6), (10, 6), w=0.2),
                  track("D_P", (10, 6), (10, 0.5), w=0.2)]
        f = m.circuit_figures(self.measure(self.pair(second, [via("D_P", 4, 6)])))
        near = math.sqrt(0.8 ** 2 - 0.5 ** 2)  # how far past the other half's end the first is still within 0.8 mm of it
        self.assertAlmostEqual(f["pair_coupled_percent"], 100 * (4 + 2 * near) / 10, delta=0.1)
        self.assertAlmostEqual(f["pair_mismatch_percent"], 100 * (21 - 10) / 21)
        self.assertEqual(f["pair_same_vias_percent"], 0.0)


class PadExits(unittest.TestCase):
    def exits(self, tracks, w=1.0, h=1.0, nets=("A",)):
        return m.exit_measures(board([pad("A", 0, 0, w=w, h=h)], tracks), set(nets))

    def test_a_track_leaving_square_to_the_edge(self):
        t = self.exits([track("A", (0, 0), (3, 0))])
        self.assertEqual((t["exits"], t["grazing"], t["long pads"]), (1, 0, 0))

    def test_a_track_leaving_a_long_pad_through_its_end(self):
        t = self.exits([track("A", (0, 0), (3, 0))], w=2.0)
        self.assertEqual((t["exits"], t["long pads"], t["through long side"]), (1, 1, 0))

    def test_a_track_leaving_a_long_pad_through_its_long_side(self):
        t = self.exits([track("A", (0, 0), (0, 3))], w=2.0)
        self.assertEqual((t["exits"], t["grazing"], t["long pads"], t["through long side"]), (1, 0, 1, 1))

    def test_a_track_meeting_the_edge_at_a_shallow_angle_grazes_it(self):
        t = self.exits([track("A", (0, 0), (0.9, 0.6))], w=2.0)  # 34 degrees to the long side it crosses
        self.assertEqual((t["exits"], t["grazing"], t["through long side"]), (1, 1, 1))

    def test_a_diagonal_out_of_a_square_pad_does_not_graze(self):
        t = self.exits([track("A", (0, 0), (2, 1.9))])
        self.assertEqual((t["exits"], t["grazing"]), (1, 0))

    def test_the_track_is_followed_out_of_the_pad(self):
        t = self.exits([track("A", (0, 0), (0.3, 0)), track("A", (0.3, 0), (0.4, 0.3)), track("A", (0.4, 0.3), (0.4, 3))])
        self.assertEqual((t["exits"], t["grazing"]), (1, 0))

    def test_a_track_that_branches_or_ends_inside_the_pad_is_not_an_exit(self):
        inside = [track("A", (0, 0), (0.3, 0))]
        self.assertEqual(self.exits(inside)["exits"], 0)
        branching = inside + [track("A", (0.3, 0), (3, 0)), track("A", (0.3, 0), (0.3, 3))]
        self.assertEqual(self.exits(branching)["exits"], 0)

    def test_two_tracks_from_one_pad_are_two_exits(self):
        self.assertEqual(self.exits([track("A", (0, 0), (3, 0)), track("A", (-3, 0), (0, 0))])["exits"], 2)

    def test_a_track_that_does_not_end_on_the_pad_s_centre_is_not_counted(self):
        self.assertEqual(self.exits([track("A", (0.2, 0), (3, 0))])["exits"], 0)

    def test_only_pads_of_the_nets_named_count(self):
        self.assertEqual(self.exits([track("A", (0, 0), (3, 0))], nets=("B",))["exits"], 0)

    def test_a_track_on_another_layer_does_not_leave_the_pad(self):
        self.assertEqual(self.exits([track("A", (0, 0), (3, 0), layer="B.Cu")])["exits"], 0)

    def test_figures(self):
        t = m.Tally()
        t.update({"exits": 8, "grazing": 2, "long pads": 4, "through long side": 1})
        self.assertEqual(m.exit_figures(t), dict(exits=8, grazing_percent=25.0, long_pad_exits=4, through_long_side_percent=25.0))
        self.assertEqual(m.exit_figures(m.Tally())["grazing_percent"], 0.0)


if __name__ == "__main__":
    unittest.main()
