"""What `route` does for a board with pours, against a stand-in for the engine and scripted boards. No KiCad."""
import unittest

import support
from test_route import WithEngine

k = support.bridge()

SQUARE = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]


class PouredBoard:
    """GND with two pads on a pour and, if asked, one beside it; a signal net under the pour.

    What the script says happens when copper is written:
    `cuts`: the signal's track parts the two pads on the pour, unless a track of GND is laid with it;
    `erodes`: the point of the pour that the pad beside it is taken to is gone once the signal is laid.
    """

    def __init__(self, beside=False, cuts=False, erodes=False):
        self.beside, self.cuts, self.erodes = beside, cuts, erodes
        self.written, self.reads, self.taken, self.segments = [], 0, [], {}

    def laid(self, net):
        return [key for keys in self.written for key in keys if key.split(" ")[0] == net]

    def parted(self):
        """True if the pour is cut in two between its pads."""
        return self.cuts and bool(self.laid("SIG")) and not self.laid("GND")

    def read(self, nets=None):
        self.reads += 1
        pads = [support.land(k, "GND", 2, 2, part="U1"), support.land(k, "GND", 8, 8, part="U2"),
                support.land(k, "SIG", 2, 12, part="U1"), support.land(k, "SIG", 8, 12, part="U2")]
        pads += [support.land(k, "GND", 14, 5, part="J1")] if self.beside else []
        halves = [[[0.0, 0.0], [10.0, 0.0], [10.0, 4.5], [0.0, 4.5]], [[0.0, 5.5], [10.0, 5.5], [10.0, 10.0], [0.0, 10.0]]]
        pours = [k.Pour("GND", "F.Cu", [ring]) for ring in (halves if self.parted() else [SQUARE])]
        on_pour = [[0], [1]] if self.parted() else [[0, 1]]
        beside = [] if not self.beside or any(key == "GND" for key in self.laid("GND")) else [[4]]
        if self.beside and not beside:
            on_pour[-1].append(4)
        groups = {"GND": on_pour + beside, "SIG": [[2, 3]] if self.laid("SIG") else [[2], [3]]}
        picture = support.picture(k, pads, groups)
        picture.pours = {"GND": {g: [pour] for g, pour in enumerate(pours)}}
        picture.fills = pours
        picture.box = [-1.0, -1.0, 16.0, 14.0]
        return picture

    def write(self, solution, sizes):
        laid = [n for n in solution["nets"] if n["tracks"]]
        self.written.append([n["key"] for n in laid])
        self.segments.update({n["key"]: sum(len(t["points"]) - 1 for t in n["tracks"]) for n in laid})
        return sum(self.segments[n["key"]] for n in laid), 0

    def unjoined(self, nets=None):
        groups = self.read().groups
        self.reads -= 1
        return sorted(n for n, g in groups.items() if len(g) > 1 and (nets is None or n in nets))

    def on_pour(self, net, layer, at):
        return not (self.erodes and len(self.laid("SIG")) == 1 and not self.taken)

    def take_back(self, keys):
        self.taken.append(list(keys))
        self.written[-1] = [key for key in self.written[-1] if key not in keys]
        return sum(self.segments[key] for key in keys), 0

    def mark(self):
        return len(self.written)

    def rewind(self, mark):
        gone = sum(self.segments[key] for keys in self.written[mark:] for key in keys)
        del self.written[mark:]
        return gone, 0


class Pours(WithEngine):
    def route(self, board):
        return super().route(board, settings=k.Settings(pours=True))

    def test_pads_on_a_pour_are_left_alone_and_the_pad_beside_it_is_taken_to_it(self):
        board = PouredBoard(beside=True)
        outcome = self.route(board)
        gnd = {n["key"]: n for n in self.runs()[0]["nets"]}["GND"]
        self.assertEqual([t["xy"] for t in gnd["terminals"] if "piece" not in t], [[14, 5]])
        self.assertTrue(all(t["piece"] == 0 for t in gnd["terminals"] if "piece" in t))
        self.assertEqual(len(self.runs()), 1)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 2, 0))

    def test_a_track_that_ends_on_a_pour_cut_away_is_taken_back_and_laid_again(self):
        board = PouredBoard(beside=True, erodes=True)
        outcome = self.route(board)
        self.assertEqual(board.taken, [["GND"]])  # (all of a piece: under the entry's own key)
        self.assertEqual(board.written, [["SIG"], ["GND"]])  # the signal stays; the net is joined again against it
        self.assertEqual([n["key"] for n in self.runs()[1]["nets"]], ["GND"])
        self.assertIn("ended on a pour that the other new tracks cut away, taken back: GND", self.said)
        self.assertEqual((outcome.status, outcome.left), ("routed", 0))
        self.assertEqual(outcome.tracks, sum(board.segments[key] for keys in board.written for key in keys))

    def test_a_pour_cut_in_two_is_joined_again(self):
        board = PouredBoard(cuts=True)
        outcome = self.route(board)
        first, second = self.runs()
        self.assertEqual([n["key"] for n in first["nets"]], ["SIG"])  # the pour joins GND: nothing to do for it
        self.assertEqual([n["key"] for n in second["nets"]], ["GND"])
        self.assertEqual({t["piece"] for t in second["nets"][0]["terminals"]}, {0, 1})  # the two pieces of the pour
        self.assertIn("parted by the new tracks, joined again: GND", self.said)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 2, 0))

    def test_a_cut_pour_that_cannot_be_joined_again_is_held_together_in_a_new_run_of_the_lot(self):
        board = PouredBoard(cuts=True)
        self.stubborn(["GND"])  # no way past the signal's track
        outcome = self.route(board)
        first, second, third = self.runs()
        self.assertEqual([n["key"] for n in third["nets"]], ["SIG", "GND ~ 1"])
        self.assertEqual(third["nets"][1]["terminals"], second["nets"][0]["terminals"])  # the pieces as they were cut
        self.assertEqual(third["copper"], first["copper"])  # against the board as it was before the signal
        self.assertEqual(board.written, [["SIG", "GND ~ 1"]])  # the first run's copper is gone
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 2, 0))
        self.assertEqual(outcome.tracks, board.segments["SIG"] + board.segments["GND ~ 1"])


class Parts(unittest.TestCase):
    """The copper of one entry, as the pieces that hang together."""

    def laid(self, tracks, vias=()):
        return dict(key="GND", net="GND", tracks=[dict(layer=layer, points=points) for layer, points in tracks], vias=list(vias))

    def test_copper_all_of_a_piece_is_one_part_under_the_entrys_own_key(self):
        laid = self.laid([("F.Cu", [[0, 0], [5, 0]]), ("F.Cu", [[2, 0], [2, 3]])])  # a branch from the middle of the first
        self.assertEqual(k.parts_of(laid), [dict(laid, of="GND")])

    def test_two_stubs_that_do_not_touch_are_two_parts(self):
        laid = self.laid([("F.Cu", [[0, 0], [5, 0]]), ("F.Cu", [[0, 4], [5, 4]]), ("B.Cu", [[5, 4], [5, 9]])], vias=[[5, 4]])
        first, second = k.parts_of(laid)
        self.assertEqual((first["key"], first["of"], len(first["tracks"]), first["vias"]), ("GND # 1", "GND", 1, []))
        self.assertEqual((second["key"], len(second["tracks"]), second["vias"]), ("GND # 2", 2, [[5, 4]]))  # joined by the via

    def test_tracks_that_cross_on_different_layers_are_apart(self):
        laid = self.laid([("F.Cu", [[0, 0], [5, 0]]), ("B.Cu", [[5, 0], [5, 5]])])
        self.assertEqual(len(k.parts_of(laid)), 2)

    def test_the_ends_on_a_pour_are_found_per_part(self):
        pads = [support.land(k, "GND", 0, 0), support.land(k, "GND", 0, 4)]
        picture = support.picture(k, pads)
        terminals = [dict(xy=[0, 0], layers=["F.Cu"]), dict(xy=[0, 4], layers=["F.Cu"]),
                     dict(xy=[5, 0], layers=["F.Cu"], piece=0), dict(xy=[5, 9], layers=["B.Cu"], piece=0)]
        problem = dict(nets=[dict(key="GND", net="GND", terminals=terminals)])
        laid = self.laid([("F.Cu", [[0, 0], [5, 0]]), ("F.Cu", [[0, 4], [5, 4]]), ("B.Cu", [[5, 4], [5, 9]])], vias=[[5, 4]])
        ends = k.pour_ends(picture, problem, k.parts_of(laid))
        self.assertEqual(ends, {"GND # 1": [("GND", "F.Cu", [5, 0])], "GND # 2": [("GND", "B.Cu", [5, 9])]})


class StarvedBoard:
    """One pad of GND tied to its zone by one spoke, until a track is written for it."""

    ZONE = [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], [[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]]]
    LINE = [[4.25, 4.25], [5.75, 4.25], [5.75, 5.75], [4.25, 5.75]]

    def __init__(self):
        self.written = []

    def read(self, nets=None):
        picture = support.picture(k, [support.land(k, "GND", 5, 5, part="U1"), support.land(k, "GND", 8, 8, part="U2")], {"GND": [[0, 1]]})
        tied = any(self.written)
        picture.thermals = [k.Thermal("GND", 0, "F.Cu", 2, 2 if tied else 1, outline=[] if tied else self.LINE, fill=[] if tied else [self.ZONE]),
                            k.Thermal("GND", 1, "F.Cu", 2, 4)]
        picture.fills = [k.Pour("GND", "F.Cu", self.ZONE)]
        return picture

    def write(self, solution, sizes):
        self.written.append([n["key"] for n in solution["nets"] if n["tracks"]])
        return sum(len(t["points"]) - 1 for n in solution["nets"] for t in n["tracks"]), 0

    def unjoined(self, nets=None):
        return []


class Spokes(WithEngine):
    def route(self, board):
        return super().route(board, settings=k.Settings(pours=True))

    def test_a_pad_with_too_few_spokes_gets_a_track_to_its_zone(self):
        board = StarvedBoard()
        outcome = self.route(board)
        (problem,) = self.runs()
        (entry,) = problem["nets"]
        self.assertEqual((entry["key"], entry["net"], entry["no_vias"]), ("GND + 0", "GND", True))
        self.assertEqual(entry["terminals"][0], dict(xy=[5, 5], layers=["F.Cu"]))
        self.assertEqual(len(entry["terminals"]), 9)  # the pad, and a point of the zone in each of eight directions
        self.assertTrue(all(t["piece"] == 0 for t in entry["terminals"][1:]))
        self.assertEqual(board.written, [["GND + 0"]])
        self.assertEqual((outcome.tied, outcome.starved, outcome.tracks), (1, [], 8))
        self.assertEqual(self.said, ["1 pads with too few spokes to their zone tied to it by a track of their own"])

    def test_a_pad_that_no_track_fits_by_is_named(self):
        board = StarvedBoard()
        self.stubborn(["GND + 0"])
        outcome = self.route(board)
        self.assertEqual(board.written, [])
        self.assertEqual((outcome.tied, outcome.starved), (0, ["U1 (GND, F.Cu)"]))
        self.assertEqual(self.said, ["too few spokes to their zone, and no room for a track to it: U1 (GND, F.Cu)"])

    def test_a_board_without_pours_is_not_looked_at(self):
        board = StarvedBoard()
        outcome = super().route(board)
        self.assertEqual((self.runs(), outcome.tied), ([], 0))


if __name__ == "__main__":
    unittest.main()
