"""What the nets are for: guesses from names and parts, the hints file, what follows from each for the rules and
the problem, and pairs. No KiCad; the engine itself plans the pairs and lays one."""
import io, json, tempfile, unittest
from contextlib import redirect_stdout
from pathlib import Path

import support
from test_route import FakeBoard, WithEngine

k = support.bridge()


def board(names, part=None):
    """Nets of two pads each, 2 mm apart in a column at either end of a board; `part`: {net: reference of its first pad's part}."""
    pads = []
    for i, name in enumerate(names):
        pads += [support.land(k, name, 2.0, 2.0 + 2.0 * i, part=(part or {}).get(name)),
                 support.land(k, name, 18.0, 2.0 + 2.0 * i)]
    picture = support.picture(k, pads)
    picture.box = [0.0, 0.0, 20.0, 4.0 + 2.0 * len(names)]
    return picture


def hints(**said):
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "board.routing.json"
        path.write_text(json.dumps(said))
        return k.read_hints(path)


class Names(unittest.TestCase):
    def test_pairs_are_names_that_differ_in_their_ending_only(self):
        nets = ["USB_DP", "USB_DM", "/eth/TX+", "/eth/TX-", "CLK_P", "CLK_N", "LVDS1P", "LVDS1N", "RX-P", "RX-N", "SDA",
                "SCL", "TOP", "TON"]
        self.assertEqual(sorted(k.pair_names(nets)),
                         [("/eth/TX+", "/eth/TX-"), ("CLK_N", "CLK_P"), ("LVDS1N", "LVDS1P"), ("RX-N", "RX-P"),
                                                      # (TOP and TON: a name alone does not tell)
                                                      ("TON", "TOP"), ("USB_DM", "USB_DP")])

    def test_supplies_are_no_pair_and_a_half_alone_is_none(self):
        self.assertEqual(k.pair_names(["V+", "V-", "+", "-", "+5V", "-5V", "VIN+", "VIN-", "GND_P", "GND_N", "D+",
                                       "A_P", "B_N"]), [])

    def test_roles_are_guessed_from_names_and_parts(self):
        picture = board(["GND", "+3V3", "/mcu/SPI_SCK", "XTAL1", "/psu/SW", "VREF", "Net-(Y1-Pad1)", "DATA3", "SWDIO",
                         "CLKOUT_EN", "Net-(X1-Pad3)"],
                        part={"Net-(Y1-Pad1)": "Y1", "Net-(X1-Pad3)": "X1"})
        roles = {net: intent.role for net, intent in k.intents_of(picture).items()}
        self.assertEqual(roles,
                         {"GND": "ground", "+3V3": "supply", "/mcu/SPI_SCK": "clock", "XTAL1": "crystal",
                          "/psu/SW": "noisy",
                                 "VREF": "supply", "Net-(Y1-Pad1)": "crystal", "DATA3": None, "SWDIO": None,
                                 "CLKOUT_EN": None, "Net-(X1-Pad3)": None})  # (X1 is a connector as often as a crystal)
        self.assertTrue(all(intent.role is None and intent.pair is None
                            for intent in k.intents_of(picture, guesses=False).values()))


class HintsFile(unittest.TestCase):
    def test_patterns_name_nets_and_later_ones_override(self):
        picture = board(["/adc/IN1", "/adc/IN2", "/adc/CLK", "+24V"])
        said = hints(nets={"/adc/*": dict(role="sensitive", clearance=0.4), "adc/CLK": dict(role="clock", vias="none"),
                           "+24V": dict(amps=3, volts=24)})
        intents = k.intents_of(picture, said)
        self.assertEqual([intents[n].role for n in ("/adc/IN1", "/adc/IN2", "/adc/CLK")],
                         ["sensitive", "sensitive", "clock"])
        self.assertEqual((intents["/adc/CLK"].clearance, intents["/adc/CLK"].vias, intents["+24V"].amps),
                         (0.4, "none", 3))
        self.assertEqual(intents["/adc/CLK"].source,
                         dict(role="board.routing.json", clearance="board.routing.json", vias="board.routing.json"))
        self.assertEqual(intents["+24V"].role, "supply")  # (what the hints do not speak of stays as guessed)

    def test_a_pair_is_said_by_either_net_or_in_the_list_and_unsaid_with_false(self):
        picture = board(["A", "B", "C", "D", "E_P", "E_N"])
        said = hints(nets={"A": dict(pair="B", gap=0.3), "E_P": dict(pair=False)},
                     pairs=[dict(nets=["C", "D"], width=0.15)])
        intents = k.intents_of(picture, said)
        self.assertEqual([intents[n].pair for n in "ABCD"], ["B", "A", "D", "C"])
        self.assertEqual((intents["B"].gap, intents["C"].width, intents["D"].width), (0.3, 0.15, 0.15))
        self.assertEqual((intents["E_P"].pair, intents["E_N"].pair), (False, None))  # one half says no: no pair

    def test_what_is_wrong_with_a_file_is_said(self):
        for said, why in ((dict(nest={}), "the file"), (dict(nets={"A": dict(widht=0.2)}), "nets[A].widht: not known"),
                          (dict(nets={"A": dict(width="wide")}), "nets[A].width: a number"),
                          (dict(nets={"A": dict(amps=-1)}), "nets[A].amps"),
                          (dict(nets={"A": dict(role="loud")}), "nets[A].role: one of"),
                          (dict(nets={"A": dict(vias="some")}), "nets[A].vias"),
                          (dict(pairs=[dict(nets=["A"])]), "pairs[0].nets"),
                          (dict(pairs=[dict(nets=["A", "B"], gap="x")]), "pairs[0]"),
                          (dict(pairs=[dict(nets=["A", "B"], width=True)]), "pairs[0]"), (dict(nets=["A"]), "the file"),
                          (dict(pairs=5), "the file"),
                          (dict(nets={"A": dict(bus=True)}), "nets[A].bus: a name, or false"),
                          (dict(nets={"A": dict(pair=True)}), "nets[A].pair"),
                          (dict(nets={"A": dict(layers=[1, 2])}), "nets[A].layers"),
                          (dict(nets={"A": dict(width=250)}), "nets[A].width: is 250"),
                          (dict(nets={"A": dict(volts=100000)}), "nets[A].volts"),
                          (dict(nets={"A": dict(amps=True)}), "nets[A].amps")):
            with self.assertRaises(ValueError) as raised:
                hints(**said)
            self.assertIn(why, str(raised.exception))
        with self.assertRaises(ValueError):
            k.read_hints("/no/such/file.json")

    def test_what_the_file_says_that_nothing_on_the_board_answers_to_is_told(self):
        picture = board(["D_P", "D_N", "A"])
        said = hints(nets={"D_P": dict(pair="D_M"), "/adc/*": dict(role="sensitive"),
                           "A": dict(layers=["F.Cu", "In7.Cu"])}, pairs=[dict(nets=["A", "B_NN"])])
        self.assertEqual(k.unheeded(picture, said),
                         ["board.routing.json: D_P is to be a pair with D_M, which is no net of the board",
                                                     "board.routing.json: /adc/* names no net of the board",
                                                     "board.routing.json: A is given the layer In7.Cu, which the board has not",
                                                     "board.routing.json: the pair A = B_NN names B_NN, which is no net of the board"])
        self.assertEqual(k.unheeded(picture, hints(nets={"D_?": dict(vias="few")})), [])
        self.assertEqual(k.unheeded(picture, None), [])

    def test_what_was_taken_is_told_by_source(self):
        picture = board(["CLK", "D_P", "D_N", "GND", "+24V", "OUT"])
        said = hints(nets={"+24V": dict(amps=3), "OUT": dict(ohms=50)})
        self.assertEqual(k.told(k.intents_of(picture, said)),
                         ["from board.routing.json: +24V amps 3; OUT ohms 50 (not acted on)",
                                                                "from the net names and parts: clock CLK; pair D_N = D_P"])
        self.assertEqual(k.told(k.intents_of(picture, said), only={"GND", "OUT"}),
                         ["from board.routing.json: OUT ohms 50 (not acted on)"])


class WhatFollows(unittest.TestCase):
    def rules(self, names, **said):
        picture = board(names)
        return picture, k.Rules.of(picture, k.Settings(hints=hints(nets=said) if said else None))

    def test_a_width_from_the_hints_or_from_a_current_and_a_clearance_from_the_hints_or_a_voltage(self):
        _, rules = self.rules(["A", "B", "C", "D", "E"], A=dict(width=0.6), B=dict(amps=3), C=dict(amps=0.01),
                              D=dict(clearance=0.5), E=dict(volts=230))
        self.assertEqual(rules.track_width("A"), 0.6)
        self.assertTrue(1.2 < rules.track_width("B") < 1.6, rules.track_width("B"))  # 3 A on an outer layer
        self.assertEqual(rules.track_width("C"), 0.25)  # (never narrower than its class)
        self.assertEqual((rules.clearance("D"), rules.clearance("E")), (0.502, 1.252))
        self.assertEqual(rules.stricter("A"), {"D": 0.502, "E": 1.252})  # the others keep that from its copper too

    def test_a_role_by_name_leans_and_what_the_hints_say_binds(self):
        picture, rules = self.rules(["CLK", "VSENSE", "/afe/IN", "SW", "X"],
                                    **{"/afe/IN": dict(role="sensitive", layers=["F.Cu", "In9.Cu"], max_mm=30),
                                                                               "X": dict(vias="none", priority="high")})
        # a clock by its name: slower to take a via, no more
        self.assertEqual(rules.leaning("CLK"), {"via_penalty": 2.0})
        self.assertEqual((rules.leaning("VSENSE"), rules.leaning("SW")), ({}, {}))  # by their names alone: nothing
        self.assertEqual(rules.leaning("/afe/IN"),
                         {"yield": 0.5, "weight": 2.0, "quiet": True, "max_mm": 30, "layers": ["F.Cu", "In9.Cu"]})
        self.assertEqual(rules.leaning("X"), {"yield": 0.3, "weight": 3.0, "no_vias": True})
        problem = k.build_problem(picture, rules,
                                  {n: [[(p.at, p.layers)] for p in picture.pads if p.net == n] for n in picture.groups})
        nets = {n["key"]: n for n in problem["nets"]}
        self.assertEqual((nets["CLK"].get("yield"), nets["CLK"]["via_penalty_mm"], nets["SW"].get("weight")),
                         (None, 20.0, None))
        self.assertEqual((nets["/afe/IN"]["quiet"], nets["/afe/IN"]["layers"], nets["/afe/IN"]["max_mm"],
                          nets["X"]["no_vias"]), (True, ["F.Cu"], 30, True))
        self.assertEqual(problem["rules"]["quiet_clearance_mm"], 2 * problem["rules"]["new_clearance_mm"])
        self.assertNotIn("quiet_clearance_mm",
                         k.build_problem(board(["CLK", "VSENSE"]), k.Rules.of(board(["CLK", "VSENSE"])),
                                                               {n: [[([2, 2], ["F.Cu"])], [([18, 2], ["F.Cu"])]]
                                                                for n in ("CLK", "VSENSE")})["rules"])

    def test_layers_from_the_hints_do_not_take_a_net_off_its_own_pads(self):
        picture, rules = self.rules(["A"], A=dict(layers=["B.Cu"]))  # (its pads are on F.Cu)
        reached = {"A": [[(p.at, p.layers)] for p in picture.pads]}
        (net,) = k.build_problem(picture, rules, reached)["nets"]
        self.assertEqual(net["layers"], ["F.Cu", "B.Cu"])
        picture.pads[0].layers = picture.pads[1].layers = ["F.Cu", "B.Cu"]
        reached = {"A": [[(p.at, p.layers)] for p in picture.pads]}
        self.assertEqual(k.build_problem(picture, rules, reached)["nets"][0]["layers"], ["B.Cu"])

    def test_a_net_laid_narrower_than_its_current_takes_is_noted(self):
        picture, rules = self.rules(["A"], A=dict(amps=3))
        reached, notes = {"A": [[(p.at, p.layers)] for p in picture.pads]}, []
        (net,) = k.build_problem(picture, rules, reached, narrow=True, notes=notes)["nets"]
        self.assertEqual(net["width"], 0.9)  # the pads are 1 mm wide
        self.assertEqual(notes,
                         ["A is laid 0.90 mm wide to enter its pads, narrower than the 1.37 mm its current takes"])
        self.assertEqual(k.build_problem(picture, rules, reached, notes=notes)["nets"][0]["width"], k.amps_width(3))

    def test_a_role_the_hints_give_leans_all_the_way(self):
        _, rules = self.rules(["A", "B"], A=dict(role="clock"), B=dict(role="noisy"))
        self.assertEqual(rules.leaning("A"), {"yield": 0.5, "weight": 2.0, "via_penalty": 2.0})
        self.assertEqual(rules.leaning("B"), {"yield": 0.3, "weight": 3.0})

    def test_the_hints_name_a_bus_or_take_a_net_out_of_one(self):
        names = [f"D{i}" for i in range(4)]
        picture = board(names)
        for pad in picture.pads:
            pad.part = "U1" if pad.at[0] < 10 else "U2"
        entries = lambda **said: {n["key"]: n.get("bus")
                                  for n in k.problem_of(picture,
                                                        k.Rules.of(picture, k.Settings(hints=hints(nets=said) if said else None)), support.ask)["nets"]}
        self.assertTrue(all(entries().values()))
        self.assertEqual(entries(D0=dict(bus=False), D3=dict(bus="mine"))["D0"], None)
        self.assertEqual(entries(D0=dict(bus=False), D3=dict(bus="mine"))["D3"], "mine")


class Pairs(WithEngine):
    """Two nets laid side by side."""

    def picture(self, names=("D_P", "D_N", "OTHER"), far=18.0):
        pads = []
        for i, name in enumerate(names):
            pads += [support.land(k, name, 2.0, 4.0 + i, half=0.3), support.land(k, name, far, 4.0 + i, half=0.3)]
        picture = support.picture(k, pads)
        picture.box = [0.0, 0.0, 20.0, 10.0]
        return picture

    def problem(self, picture, **settings):
        notes = []
        return k.problem_of(picture, k.Rules.of(picture, k.Settings(**settings)), support.ask, notes=notes), notes

    def test_a_pair_is_a_trunk_of_its_own_net_and_four_stubs(self):
        problem, notes = self.problem(self.picture())
        (pair,) = problem["pairs"]
        self.assertEqual((pair["key"], pair["legs"], pair["leg_width"], pair["pitch"]),
                         ("D_N = D_P", ["D_N", "D_P"], 0.25, 0.452))
        nets = {n["key"]: n for n in problem["nets"]}
        self.assertEqual(sorted(nets),
                         ["D_N = D_P D_N 1", "D_N = D_P D_N 2", "D_N = D_P D_P 1", "D_N = D_P D_P 2", "D_N = D_P trunk",
                          "OTHER"])
        trunk = nets[pair["trunk"]]
        # (room for a corner of 45 degrees)
        self.assertEqual((trunk["net"], trunk["width"]), ("~D_N = D_P", 0.744241))
        # it may change layer: through a via as wide as two of the nets' vias (0.6) side by side, which stand
        # as far apart as their copper may be (the clearance of 0.202 between them), at the price of two
        self.assertEqual((pair["via_pitch"], trunk["via"], trunk["via_penalty_mm"], "no_vias" in trunk),
                         (0.802, [1.402, 1.102], 20.0, False))
        # (and the one distance the engine keeps between holes is still that of the nets' own vias)
        self.assertEqual(problem["rules"]["hole_centre_mm"], 0.552)
        self.assertEqual({t["piece"] for t in trunk["terminals"]}, {0, 1})
        near = [t for t in trunk["terminals"] if t["piece"] == 0]
        # square to the pads first, then the places beside it, and the nearer to the pads the earlier
        self.assertEqual(sorted({t.get("turn", 0) for t in near}), list(range(10)))
        self.assertEqual([t["xy"][1] for t in near if "turn" not in t], [4.5, 4.5])  # (the pads are at y 4 and 5)
        off = lambda t: abs(t["xy"][0] - 2.0)  # (how far off the pads, which are at x 2)
        self.assertLess(max(off(t) for t in near if "turn" not in t), min(off(t) for t in near if t.get("turn") == 2))
        # a little off the pads, to either side of them
        self.assertTrue(all(0 < off(t) < 6 for t in near))
        self.assertEqual(pair["stubs"],
                         [["D_N = D_P D_N 1", "D_N = D_P D_P 1"], ["D_N = D_P D_N 2", "D_N = D_P D_P 2"]])
        stub = nets["D_N = D_P D_P 1"]
        self.assertEqual((stub["net"], stub["last"], stub["width"], stub["terminals"]),
                         ("D_P", True, 0.25, [dict(xy=[2.0, 4.0], layers=["F.Cu"])]))
        self.assertEqual(notes, [])

    def test_the_engine_lays_it_side_by_side(self):
        problem, _ = self.problem(self.picture())
        with redirect_stdout(io.StringIO()):
            solution = k.run_engine(problem, support.PROGRAM)
        self.assertEqual(solution["status"], "routed", solution.get("message"))
        legs = {n["net"]: n for n in solution["nets"] if n["key"].startswith("D_N = D_P/")}
        self.assertEqual(sorted(legs), ["D_N", "D_P"])
        self.assertLess(abs(legs["D_N"]["length_mm"] - legs["D_P"]["length_mm"]), 0.5)

    def test_no_pairs_when_told_so_or_in_the_narrower_try_or_for_a_net_given_up_as_a_pair(self):
        picture = self.picture()
        self.assertNotIn("pairs", self.problem(picture, pairs=False)[0])
        self.assertNotIn("pairs", self.problem(picture, guesses=False)[0])
        rules = k.Rules.of(picture)
        self.assertNotIn("pairs", k.problem_of(picture, rules, support.ask, narrow=True))
        self.assertNotIn("pairs", k.problem_of(picture, rules, support.ask, unpaired={"D_N = D_P": 3}))
        further = k.problem_of(picture, rules, support.ask, unpaired={"D_N = D_P": 1})
        trunk = {n["key"]: n for n in further["nets"]}["D_N = D_P trunk"]
        # the nearest lot of places is gone
        self.assertEqual(sorted({t.get("turn", 0) for t in trunk["terminals"]}), list(range(8)))
        # (the nearest lot reached out to 2.95)
        self.assertGreater(min(abs(t["xy"][0] - 2.0) for t in trunk["terminals"] if t["piece"] == 0), 0.95)

    def test_why_a_pair_by_name_is_laid_as_two_nets_is_noted(self):
        problem, notes = self.problem(self.picture(far=4.0))  # 2 mm from end to end
        self.assertNotIn("pairs", problem)
        self.assertEqual(notes,
                         ["D_N = D_P is laid as two nets: its ends are too near each other, or share no routing layer"])
        apart = self.picture()
        apart.pads[1].at = [18.0, 12.0]  # the far pad of the first net stands off on its own
        self.assertIn("no two places", self.problem(apart)[1][0])
        more = self.picture()
        for x in (6.0, 8.0, 10.0):  # three pads more on one of them: five
            more.pads.append(support.land(k, "D_P", x, 8.0, half=0.3))
            more.groups["D_P"].append([len(more.pads) - 1])
        problem, notes = self.problem(more)
        self.assertNotIn("pairs", problem)
        self.assertEqual(notes, ["D_N = D_P is laid as two nets: one of them has more than 4 pads"])
        poured = self.picture()
        poured.fills = [k.Pour("D_P", "F.Cu", [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]])]
        poured.pours = {"D_P": {0: [0]}}
        self.assertEqual(self.problem(poured)[1], ["D_N = D_P is laid as two nets: one of them has a zone"])
        one_quiet = hints(nets={"D_P": dict(role="sensitive")})
        self.assertIn("sensitive by the hints and the other is not",
                      self.problem(self.picture(), hints=one_quiet)[1][0])

    def test_a_pad_of_its_own_on_one_leg_does_not_prevent_the_pair(self):
        # (a pull-up on one leg)
        picture = self.picture()
        picture.pads.append(support.land(k, "D_P", 10.0, 8.0, half=0.3))
        picture.groups["D_P"].append([len(picture.pads) - 1])
        problem, notes = self.problem(picture)
        self.assertEqual((len(problem["pairs"]), notes), (1, []))
        own = {n["key"]: n for n in problem["nets"]}["D_P"]
        self.assertEqual([(t["xy"], t.get("piece")) for t in own["terminals"]],
                         [([10.0, 8.0], None), ([2.0, 4.0], 0), ([18.0, 4.0], 0)])

    def test_a_net_named_as_a_pairs_key_does_not_collide(self):
        picture = self.picture(names=("D_P", "D_N", "D_N = D_P"))
        problem, _ = self.problem(picture)
        keys = [n["key"] for n in problem["nets"]]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertEqual(problem["pairs"][0]["key"], "D_N = D_P'")

    def test_a_short_hop_past_a_part_is_laid_as_tracks_and_the_long_stretch_as_a_pair(self):
        # both nets pass a part 2 mm from their first pads
        picture = self.picture(names=("D_P", "D_N"))
        for i, name in enumerate(("D_P", "D_N")):
            picture.pads.append(support.land(k, name, 4.0, 4.0 + i, half=0.3))
            picture.groups[name].append([len(picture.pads) - 1])
        problem, notes = self.problem(picture)
        (pair,) = problem["pairs"]
        nets = {n["key"]: n for n in problem["nets"]}
        self.assertEqual(sorted(nets),
                         ["D_N", "D_N = D_P D_N 1", "D_N = D_P D_N 2", "D_N = D_P D_P 1", "D_N = D_P D_P 2",
                          "D_N = D_P trunk", "D_P"])
        # the entry of each net: its pad at the near end, and the two the trunk joins as one piece
        self.assertEqual([(t["xy"][0], t["piece"]) for t in nets["D_P"]["terminals"]], [(2.0, 0), (18.0, 1), (4.0, 1)])
        with redirect_stdout(io.StringIO()):
            solution = k.run_engine(problem, support.PROGRAM)
        self.assertEqual(solution["status"], "routed", solution.get("message"))

    def with_a_part(self, apart, between=None):
        """The pair's two nets with a part 4 mm from their first pads, its two pads `apart`; `between`: a pad
        of another net half way between the two."""
        picture = self.picture(names=("D_P", "D_N"))
        for name, y in (("D_P", 4.5 - apart / 2), ("D_N", 4.5 + apart / 2)):
            picture.pads.append(support.land(k, name, 6.0, y, half=0.3))
            picture.groups[name].append([len(picture.pads) - 1])
        if between:
            picture.pads.append(support.land(k, between, 6.0, 4.5, half=0.1))
        return picture

    def test_a_pair_runs_on_between_the_pads_of_a_part_in_its_middle_where_there_is_room(self):
        problem, notes = self.problem(self.with_a_part(3.0))
        (pair,) = problem["pairs"]
        nets = {n["key"]: n for n in problem["nets"]}
        key = "D_N = D_P (through)"
        # one trunk from end to end, the stubs of both ends and one more for either pad of the part; nothing
        # is left for an entry of the nets themselves
        self.assertEqual((pair["key"], pair["passes"]), (key, [[f"{key} D_N 3", f"{key} D_P 3"]]))
        self.assertEqual(sorted(nets), sorted([f"{key} {net} {n}" for net in ("D_N", "D_P") for n in (1, 2, 3)]
                                              + [f"{key} trunk"]))
        self.assertEqual(nets[f"{key} D_N 3"]["terminals"][0]["xy"], [6.0, 6.0])
        # the trunk passes half way between the two pads, one way on either hand, between its two ends
        passed = [t for t in nets[f"{key} trunk"]["terminals"] if t.get("pass")]
        self.assertEqual([(t["xy"], t["piece"], t["hand"], t["heading"]) for t in passed],
                         [([6.0, 4.5], 2, 1, [1.0, 0.0]), ([6.0, 4.5], 2, 2, [-1.0, 0.0])])
        pieces = [t["piece"] for t in nets[f"{key} trunk"]["terminals"] if t["hand"] == 1]
        self.assertEqual(sorted(set(pieces), key=pieces.index), [0, 2, 1])
        with redirect_stdout(io.StringIO()):
            solution = k.run_engine(problem, support.PROGRAM)
        self.assertEqual(solution["status"], "routed", solution.get("message"))
        legs = [n for n in solution["nets"] if n["key"].startswith(key + "/")]
        self.assertEqual([len(n["tracks"]) for n in legs], [4, 4])  # the leg and three stubs
        for leg in legs:
            self.assertTrue(leg["tracks"][0]["points"][0][0] < 4.0 and leg["tracks"][0]["points"][-1][0] > 16.0)

    def test_a_part_without_room_between_its_pads_is_not_passed_nor_one_tried_and_not_laid(self):
        through = lambda problem: [pair["key"] for pair in problem["pairs"] if "passes" in pair]
        # pads 1.4 mm apart leave 0.8 mm: the trunk and its clearance take more
        self.assertEqual(through(self.problem(self.with_a_part(1.4))[0]), [])
        # a pad of another net stands where the pair would pass
        self.assertEqual(through(self.problem(self.with_a_part(3.0, between="OTHER"))[0]), [])
        # not all laid so, it is laid stretch by stretch (here the one from the part on: the other is too short)
        picture = self.with_a_part(3.0)
        rules = k.Rules.of(picture, k.Settings())
        again = k.problem_of(picture, rules, support.ask, unpaired={"D_N = D_P (through)": 1})
        self.assertEqual([pair["key"] for pair in again["pairs"]], ["D_N = D_P"])

    def test_the_hints_set_the_gap_and_the_width(self):
        picture = self.picture()
        said = hints(pairs=[dict(nets=["D_P", "D_N"], gap=0.4, width=0.3)], nets={"OTHER": dict(pair=False)})
        (pair,) = self.problem(picture, hints=said)[0]["pairs"]
        self.assertEqual((pair["leg_width"], pair["pitch"]), (0.3, 0.7))

    def test_a_pair_that_is_laid_is_written_leg_by_leg_at_its_legs_width(self):
        board = PairBoard()
        outcome = self.route(board)
        (problem,) = self.runs()
        self.assertEqual([sorted(written) for written in board.writes], [["D_N = D_P/D_N", "D_N = D_P/D_P", "OTHER"]])
        self.assertEqual(board.sizes["D_N = D_P/D_P"], (0.25, [0.6, 0.3]))
        self.assertIn("laid as a pair: D_N = D_P", self.said)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 3, 0))

    def test_a_pair_that_is_not_all_laid_is_begun_further_from_its_pads_and_then_done_as_two_nets(self):
        board = PairBoard()
        self.stubborn(["D_N = D_P trunk"])
        outcome = self.route(board)
        first, second, third, fourth = self.runs()
        nearest = lambda problem: min(abs(t["xy"][0] - 2.0) for n in problem["nets"] if n["key"].endswith("trunk")
                                      for t in n["terminals"] if t["piece"] == 0)
        self.assertLess(nearest(first), nearest(second))  # begun further from its pads
        headed = lambda problem: {"heading" in t for n in problem["nets"] if n["key"].endswith("trunk")
                                  for t in n["terminals"]}
        # the third time free: any place, from any side
        self.assertEqual((headed(first), headed(second), headed(third)), ({True}, {True}, {False}))
        self.assertNotIn("pairs", fourth)
        self.assertEqual(sorted(n["key"] for n in fourth["nets"]), ["D_N", "D_P", "OTHER"])
        self.assertEqual(board.writes, [["D_N", "D_P", "OTHER"]])  # nothing of the runs before is kept
        self.assertEqual(self.said.count("not all laid as pairs: again, begun further from their pads or as single nets, for D_N = D_P"), 3)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 3, 0))

    def test_a_net_left_unjoined_with_pairs_laid_is_tried_once_without_them_and_the_pairs_are_kept_if_that_is_no_better(self):
        board = PairBoard()
        self.stubborn(["OTHER"], crowded=True)  # (joined by itself: something is in its way, perhaps the pair)
        outcome = self.route(board)
        first, alone, second, third = self.runs()
        self.assertEqual([n["key"] for n in alone["nets"]], ["OTHER"])
        self.assertEqual(("pairs" in first, "pairs" in second, len(second["nets"])), (True, False, 3))
        kept = [line for line in self.said if "pair" in line]
        self.assertEqual(kept,
                         ["from the net names and parts: pair D_N = D_P", "not all joined: once more without pairs",
                                "laid as a pair: D_N = D_P"])
        self.assertIn("no more is joined without: they are kept", self.said)
        self.assertEqual(board.writes[0], ["D_N = D_P/D_N", "D_N = D_P/D_P"])  # the first run's pair is what is written
        self.assertEqual([n["key"] for n in third["nets"]], ["OTHER"])  # then the narrower try for what is left
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 3, 0))  # (by itself, as said)


    def test_a_net_that_is_not_joined_by_itself_either_costs_no_run_without_the_pairs(self):
        board = PairBoard()
        self.stubborn(["OTHER"])
        outcome = self.route(board)
        first, alone, narrower = self.runs()  # no run of all the nets without the pair
        self.assertEqual(("pairs" in first, [n["key"] for n in alone["nets"]]), (True, ["OTHER"]))
        self.assertEqual([line for line in self.said if "pair" in line],
                         ["from the net names and parts: pair D_N = D_P", "laid as a pair: D_N = D_P"])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("unroutable", 3, 1))


class Guessed(WithEngine):
    """What is only guessed of a net is a wish, as a pair is."""

    def board(self):
        board = FakeBoard(0)
        board.names = ["CLK", "OTHER"]
        return board

    def test_a_net_left_unjoined_is_tried_once_without_what_the_names_were_taken_to_ask(self):
        board = self.board()
        self.stubborn(["OTHER"], crowded=True)
        outcome = self.route(board)
        first, _, second, third = self.runs()
        penalty = lambda problem: {n["key"]: n["via_penalty_mm"] for n in problem["nets"]}["CLK"]
        # a clock by its name, then a net like any other
        self.assertEqual((penalty(first), penalty(second)), (20.0, 10.0))
        self.assertEqual(self.said[:3],
                         ["from the net names and parts: clock CLK",
                          "not all joined: once more without what the nets' names were taken to ask",
                                         "no more is joined without: they are kept"])
        self.assertEqual([n["key"] for n in third["nets"]], ["OTHER"])
        self.assertEqual((outcome.status, outcome.left), ("routed", 0))  # (by itself it is joined)

    def test_all_joined_at_once_nothing_is_weighed(self):
        board = self.board()
        outcome = self.route(board)
        self.assertEqual((len(self.runs()), outcome.status), (1, "routed"))

    def test_without_a_guess_that_leans_nothing_is_weighed(self):
        board = FakeBoard(2)
        self.stubborn(["N000"])
        self.route(board)
        self.assertEqual(len(self.runs()), 2)  # the try and the narrower one, as ever


class PairBoard(FakeBoard):
    """D_P and D_N side by side and one net more, as `route` uses a board."""

    def __init__(self):
        super().__init__(0)
        self.names = ["D_P", "D_N", "OTHER"]

    def read(self, nets=None):
        picture = Pairs.picture(None)
        for i, name in enumerate(self.names):
            picture.groups[name] = [[2 * i, 2 * i + 1]] if name in self.joined else [[2 * i], [2 * i + 1]]
        return picture


if __name__ == "__main__":
    unittest.main()
