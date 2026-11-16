"""What `route` does for a board with pours, against a stand-in for the engine and scripted boards. No KiCad."""
import io, sys, unittest
from contextlib import redirect_stdout

import support
from test_route import FakeBoard, WithEngine

k = support.bridge()

SQUARE = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]


class PouredBoard:
    """GND with two pads on a pour and, if asked, one beside it; a signal net under the pour.

    What the script says happens when copper is written:
    `cuts`: the signal's track parts the two pads on the pour, unless a track of GND is laid with it;
    `erodes`: the point of the pour that the pad beside it is taken to is gone once the signal is laid.
    """

    def __init__(self, beside=False, cuts=False, erodes=False, more=()):
        self.beside, self.cuts, self.erodes, self.more = beside, cuts, erodes, list(more)
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
        first_more = len(pads)
        for n, net in enumerate(self.more):  # (other nets of two pads each, off to the side)
            pads += [support.land(k, net, 2, 20 + 2 * n), support.land(k, net, 8, 20 + 2 * n)]
        halves = [[[0.0, 0.0], [10.0, 0.0], [10.0, 4.5], [0.0, 4.5]], [[0.0, 5.5], [10.0, 5.5], [10.0, 10.0], [0.0, 10.0]]]
        eroded = self.erodes and len(self.laid("SIG")) == 1 and not self.taken
        whole = [[0.0, 0.0], [9.0, 0.0], [9.0, 10.0], [0.0, 10.0]] if eroded else SQUARE  # (drawn back from its right edge)
        pours = [k.Pour("GND", "F.Cu", [ring]) for ring in (halves if self.parted() else [whole])]
        on_pour = [[0], [1]] if self.parted() else [[0, 1]]
        beside = [] if not self.beside or any(key == "GND" for key in self.laid("GND")) else [[4]]
        if self.beside and not beside:
            on_pour[-1].append(4)
        groups = {"GND": on_pour + beside, "SIG": [[2, 3]] if self.laid("SIG") else [[2], [3]]}
        for n, net in enumerate(self.more):
            a = first_more + 2 * n
            groups[net] = [[a, a + 1]] if self.laid(net) else [[a], [a + 1]]
        picture = support.picture(k, pads, groups)
        picture.pours = {"GND": {g: [g] for g in range(len(pours))}}
        picture.fills = pours
        picture.box = [-1.0, -1.0, 16.0, 30.0]
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
        # the pads of the pieces as they were cut, not points of the pour
        self.assertEqual(third["nets"][1]["terminals"], [dict(xy=[2, 2], layers=["F.Cu"]), dict(xy=[8, 8], layers=["F.Cu"])])
        self.assertEqual(third["copper"], first["copper"])  # against the board as it was before the signal
        self.assertEqual(board.written, [["SIG", "GND ~ 1"]])  # the first run's copper is gone
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 2, 0))
        self.assertEqual(outcome.tracks, board.segments["SIG"] + board.segments["GND ~ 1"])


    def test_a_pour_cut_in_a_run_that_left_another_net_unjoined_counts_as_left(self):
        board = PouredBoard(cuts=True, more=["BAD"])
        self.stubborn(["BAD", "GND"])
        outcome = self.route(board)
        self.assertEqual(sorted(board.unjoined()), ["BAD", "GND"])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("unroutable", 3, 2))  # GND was joined, and is not

    def test_without_pours_nothing_is_rewound(self):
        board = PouredBoard(cuts=True)
        board.mark = board.rewind = None  # (not to be asked for)
        self.stubborn(["GND"])
        outcome = WithEngine.route(self, board)  # as --no-pours: GND is four pads to join, and cannot be
        self.assertEqual((outcome.status, outcome.left), ("unroutable", 1))


class Unanswered(WithEngine):
    """A question about the zones that the engine does not answer."""

    def no_answer_from(self, number):
        (self.dir / "fail_pours").write_text(str(number))

    def test_a_question_without_an_answer_in_mid_run_keeps_what_was_laid(self):
        board = PouredBoard(cuts=True)
        self.no_answer_from(3)  # where the signal may be reached, what was laid for it; then where GND may be
        outcome = super().route(board, settings=k.Settings(pours=True))
        self.assertEqual(board.written, [["SIG"]])
        self.assertIn("not to answer", outcome.failure)
        self.assertEqual((outcome.tracks, outcome.wanted, outcome.left), (board.segments["SIG"], 2, 1))  # the zone is cut, and stays so

    def test_without_an_answer_to_the_first_question_nothing_is_laid(self):
        board = PouredBoard()
        self.no_answer_from(1)
        outcome = super().route(board, settings=k.Settings(pours=True))
        self.assertEqual((board.written, outcome.laid), ([], False))
        self.assertIn("not to answer", outcome.failure)

    def test_an_answer_that_cannot_be_read_is_no_answer(self):
        with self.assertRaises(k.EngineFailed):
            k.ask_engine(dict(ask="ties", fills=[], tracks=[]), [sys.executable, "-c", "print('{')"])
        with self.assertRaises(k.EngineFailed):
            k.ask_engine(dict(ask="ties", fills=[], tracks=[]), str(self.dir / "no-such-engine"))


class ManyNets(FakeBoard):
    """Nets enough for two batches, on a board without zones, with what a board with pours is asked."""

    def mark(self):
        return len(self.writes)

    def rewind(self, mark):
        raise AssertionError("nothing is to be taken back")

    take_back = rewind


class Batches(WithEngine):
    def test_a_net_an_earlier_batch_left_is_not_taken_for_one_parted_by_a_later(self):
        board = ManyNets(300)
        self.stubborn(["N000"])
        outcome = super().route(board, settings=k.Settings(pours=True))
        self.assertEqual([len(p["nets"]) for p in self.runs()], [150, 1, 150])  # the batch, its second try, the next batch
        self.assertFalse([line for line in self.said if "parted" in line])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("unroutable", 300, 1))


class StarvedBoard:
    """One pad of GND tied to its zone by one spoke, until a track is written for it."""

    ZONE = [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], [[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]]]
    LINE = [[4.25, 4.25], [5.75, 4.25], [5.75, 5.75], [4.25, 5.75]]

    def __init__(self, half=0.5):
        self.written, self.half = [], half

    def read(self, nets=None):
        picture = support.picture(k, [support.land(k, "GND", 5, 5, part="U1", half=self.half), support.land(k, "GND", 8, 8, part="U2")], {"GND": [[0, 1]]})
        tied = any(self.written)
        picture.thermals = [k.Thermal("GND", 0, "F.Cu", 2, 2 if tied else 1, outline=[] if tied else self.LINE, fill=[] if tied else [0]),
                            k.Thermal("GND", 1, "F.Cu", 2, 4)]
        picture.fills = [k.Pour("GND", "F.Cu", self.ZONE)]
        return picture

    def write(self, solution, sizes):
        self.written.append([n["key"] for n in solution["nets"] if n["tracks"]])
        return sum(len(t["points"]) - 1 for n in solution["nets"] for t in n["tracks"]), 0

    def unjoined(self, nets=None):
        return []


class ShutInBoard:
    """A pad of GND with one spoke and no way for a track of its own to its zone, because the signal that this
    run laid goes round it; until the signal is laid again."""

    LINE = StarvedBoard.LINE

    def __init__(self):
        self.signal = "round the pad"  # or "taken out", "clear of it"
        self.calls = []

    def read(self, nets=None):
        pads = [support.land(k, "GND", 5, 5, part="U1"), support.land(k, "GND", 8, 8, part="U2"),
                support.land(k, "SIG", 1, 6.3, part="R1"), support.land(k, "SIG", 9.5, 6.3, part="R2")]
        picture = support.picture(k, pads, {"GND": [[0, 1]], "SIG": [[2], [3]] if self.signal == "taken out" else [[2, 3]]})
        shut_in = self.signal == "round the pad"
        picture.thermals = [k.Thermal("GND", 0, "F.Cu", 2, 1 if shut_in else 2, outline=self.LINE if shut_in else [], gap=0.5, clearance=0.2)]
        if self.signal != "taken out":
            y = 6.3 if shut_in else 9.0
            picture.tracks = [k.Track(a=[1, y], b=[9.5, y], width=0.25, net="SIG", layer="F.Cu")]
        return picture

    def write(self, solution, sizes):
        self.calls.append(("write", [n["key"] for n in solution["nets"] if n["tracks"]]))
        self.signal = "clear of it"
        return 3, 0

    def unjoined(self, nets=None):
        return ["SIG"] if self.signal == "taken out" else []

    def take_out(self, nets):
        self.calls.append(("take out", sorted(nets)))
        self.signal = "taken out"
        return 1, 0

    def put_back(self):
        self.calls.append(("put back",))
        self.signal = "round the pad"
        return 1, 0

    def mark(self):
        return 0

    def rewind(self, mark):
        self.calls.append(("rewind",))
        return 0, 0


class ShutIn(WithEngine):
    """A pad starved by a net of this run, with no room for a track to its zone."""

    def see_to(self, board, laid_before=frozenset()):
        outcome = k.Outcome(tracks=1)
        with redirect_stdout(io.StringIO()):
            k.tie_starved(board, k.Settings(pours=True), k.Engine(support.ENGINE), outcome, say=self.said.append, laid_before=laid_before)
        return outcome

    def test_the_net_that_goes_round_it_is_laid_again_clear_of_it(self):
        board = ShutInBoard()
        outcome = self.see_to(board)
        (problem,) = self.runs()
        self.assertEqual([n["key"] for n in problem["nets"]], ["SIG"])
        (closed,) = [r for r in problem["regions"] if r.get("layers") == ["F.Cu"]]
        xs = [p[0] for p in closed["rings"][0]]
        self.assertTrue(3 < min(xs) < 3.5 and 6.5 < max(xs) < 7, (min(xs), max(xs)))  # the pad, its gap, the clearance, and 0.3 mm
        self.assertEqual(board.calls, [("take out", ["SIG"]), ("write", ["SIG"])])
        self.assertEqual(self.said, ["laid again clear of U1, whose spokes they cut: SIG"])
        self.assertEqual((outcome.starved, outcome.tracks), ([], 3))

    def test_where_it_cannot_be_it_is_put_back_and_the_pad_is_named(self):
        board = ShutInBoard()
        self.stubborn(["SIG"])
        outcome = self.see_to(board)
        self.assertEqual(board.calls, [("take out", ["SIG"]), ("rewind",), ("put back",)])
        self.assertEqual(self.said, ["no way clear of U1 for SIG: left as they were",
                                     "too few spokes to their zone, and no room for a track to it: U1 (GND, F.Cu)"])
        self.assertEqual((outcome.starved, outcome.tracks), (["U1 (GND, F.Cu)"], 1))

    def test_a_track_the_board_came_with_is_left_where_it_is(self):
        board = ShutInBoard()
        outcome = self.see_to(board, laid_before=k.copper_keys(board.read()))
        self.assertEqual((board.calls, self.runs()), ([], []))
        self.assertEqual(outcome.starved, ["U1 (GND, F.Cu)"])


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

    def test_a_narrower_track_is_tried_where_the_nets_own_finds_no_way(self):
        board = StarvedBoard(half=0.15)  # a pad 0.3 mm wide; the class's track is 0.25 mm
        self.stubborn(["GND + 0"], below=0.25)
        outcome = self.route(board)
        first, second = self.runs()
        self.assertEqual((first["nets"][0]["width"], second["nets"][0]["width"]), (0.25, 0.2))
        self.assertEqual(second["nets"][0]["terminals"], first["nets"][0]["terminals"])  # the same points again
        self.assertEqual((outcome.tied, outcome.starved), (1, []))

    def test_a_pad_the_board_came_with_starved_is_left_alone_when_other_nets_are_named(self):
        board = StarvedBoard()
        outcome = super().route(board, settings=k.Settings(pours=True), only={"SIG"})
        self.assertEqual((self.runs(), outcome.tied, outcome.starved), ([], 0, []))

    def test_a_board_without_pours_is_not_looked_at(self):
        board = StarvedBoard()
        outcome = super().route(board)
        self.assertEqual((self.runs(), outcome.tied), ([], 0))


if __name__ == "__main__":
    unittest.main()
