"""The two tries and the batches, against a stand-in for the engine (fake_engine.py). No KiCad."""
import io, json, os, sys, tempfile, unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

import support

k = support.bridge()
ENGINE = [sys.executable, str(support.FAKE_ENGINE)]


class FakeBoard:
    """A board as `route` uses it: nets of two surface-mount pads each, joined once a solution lays a track for them."""

    def __init__(self, count, narrow_pads=()):
        self.names = [f"N{i:03}" for i in range(count)]
        self.narrow_pads = set(narrow_pads)
        self.joined, self.reads, self.writes, self.sizes = set(), [], [], {}

    def read(self, nets=None):
        self.reads.append(None if nets is None else set(nets))
        pads, groups = [], {}
        for i, name in enumerate(self.names):
            half = 0.15 if name in self.narrow_pads else 0.5
            pads += [support.land(k, name, 2.0, 2.0 * i, half=half), support.land(k, name, 8.0, 2.0 * i, half=half)]
            groups[name] = [[2 * i, 2 * i + 1]] if name in self.joined else [[2 * i], [2 * i + 1]]
        picture = support.picture(k, pads, groups)
        picture.box = [0.0, 0.0, 10.0, 2.0 * len(self.names)]
        return picture

    def write(self, solution, sizes):
        laid = [n for n in solution["nets"] if n["tracks"]]
        self.joined.update(n["net"] for n in laid)
        self.sizes.update({n["key"]: sizes[n["key"]] for n in laid})
        self.writes.append([n["key"] for n in laid])
        return sum(len(t["points"]) - 1 for n in laid for t in n["tracks"]), sum(len(n["vias"]) for n in laid)

    def unjoined(self, nets=None):
        return [n for n in self.names if n not in self.joined and (nets is None or n in nets)]


class WithEngine(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)
        patch = mock.patch.dict(os.environ, FAKE_ENGINE_DIR=str(self.dir))
        patch.start()
        self.addCleanup(patch.stop)
        self.said = []

    def stubborn(self, nets, below=0.0):
        (self.dir / "stubborn.json").write_text(json.dumps(dict(nets=nets, below=below)))

    def fail_on(self, run):
        (self.dir / "fail_on").write_text(str(run))

    def runs(self):
        """The problems the engine was given, in order."""
        files = sorted((self.dir / "runs").glob("*.json"), key=lambda f: int(f.stem)) if (self.dir / "runs").exists() else []
        return [json.loads(f.read_text()) for f in files]

    def route(self, board, **more):
        run = lambda problem, first: k.run_engine(problem, ENGINE)
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return k.route(board, k.Settings(), run, say=self.said.append, **more)


class TwoTries(WithEngine):
    def test_all_joined_at_the_first_try(self):
        board = FakeBoard(3)
        outcome = self.route(board)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left, outcome.tracks, outcome.vias, outcome.failure), ("routed", 3, 0, 3, 0, None))
        self.assertEqual(len(self.runs()), 1)
        self.assertEqual(self.said, [])
        self.assertEqual(board.sizes["N000"], (0.25, [0.6, 0.3]))

    def test_second_try_is_narrower_and_only_for_the_nets_left(self):
        board = FakeBoard(3, narrow_pads={"N001"})
        self.stubborn(["N001"], below=0.25)
        outcome = self.route(board)
        first, second = self.runs()
        self.assertEqual([n["key"] for n in first["nets"]], ["N000", "N001", "N002"])
        self.assertEqual([n["key"] for n in second["nets"]], ["N001"])
        self.assertEqual((first["nets"][1]["width"], second["nets"][0]["width"]), (0.25, 0.2))
        self.assertNotIn("via_in_pad", first["rules"])
        self.assertIs(second["rules"]["via_in_pad"], True)
        self.assertEqual(self.said, ["second try, narrower: N001"])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left, outcome.tracks), ("routed", 3, 0, 3))
        self.assertEqual(board.sizes["N001"], (0.2, [0.6, 0.3]))  # written at the width it was laid at
        # the second problem is built from the board as the first try left it
        self.assertEqual(len(board.reads), 2)

    def test_still_unjoined_after_the_second_try(self):
        board = FakeBoard(3)
        self.stubborn(["N001"])
        outcome = self.route(board)
        self.assertEqual(len(self.runs()), 2)  # and no third
        self.assertEqual((outcome.status, outcome.wanted, outcome.left, outcome.tracks, outcome.failure), ("unroutable", 3, 1, 2, None))

    def test_only_the_nets_asked_for(self):
        board = FakeBoard(4)
        outcome = self.route(board, only={"N001", "N003"})
        self.assertEqual([n["key"] for n in self.runs()[0]["nets"]], ["N001", "N003"])
        self.assertEqual((outcome.wanted, outcome.left), (2, 0))
        self.assertEqual(board.unjoined(), ["N000", "N002"])

    def test_nothing_to_route(self):
        board = FakeBoard(2)
        board.joined = set(board.names)
        outcome = self.route(board)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left, outcome.laid), ("routed", 0, 0, False))
        self.assertEqual(self.runs(), [])

    def test_options_reach_the_engine(self):
        self.route(FakeBoard(1), options=dict(variants=3, max_states=5))
        self.assertEqual(self.runs()[0]["options"], dict(variants=3, max_states=5, bus_pull=0.2))


class Steps(WithEngine):
    def coarse_first(self, board, **more):
        run = lambda problem, first: k.run_engine(problem, ENGINE)
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return k.route(board, k.Settings(step=None, coarse_first=True), run, say=self.said.append, **more)

    def test_the_step_follows_from_the_rules(self):
        picture = FakeBoard(1).read()
        rules = lambda settings, **sizes: k.Rules(picture.minima, {"N000": k.NetClass(via_size=0.6, via_drill=0.3, **sizes)}, settings=settings)
        step = lambda **sizes: rules(k.Settings(step=None), **sizes).step(["N000"])
        # 0.25 mm tracks 0.22 mm apart (with the slack): 0.47 mm from one to the next, nine cells of 0.05 mm
        self.assertEqual(step(track_width=0.25, clearance=0.2), 0.05)
        self.assertEqual(step(track_width=0.5, clearance=0.3), 0.1)
        # a step the user chose is kept
        self.assertEqual(rules(k.Settings(step=0.08), track_width=0.25, clearance=0.2).tries(["N000"]), [(0.08, False), (0.08, True)])
        self.assertEqual(rules(k.Settings(step=0.08, coarse_first=True), track_width=0.25, clearance=0.2).tries(["N000"]),
                         [(0.16, False), (0.08, False), (0.08, True)])

    def test_all_joined_on_the_coarse_grid(self):
        outcome = self.coarse_first(FakeBoard(3))
        self.assertEqual([p["step"] for p in self.runs()], [0.1])
        self.assertEqual((outcome.status, outcome.left), ("routed", 0))

    def test_what_is_left_goes_to_the_fine_grid_and_then_narrower(self):
        board = FakeBoard(3, narrow_pads={"N001"})
        self.stubborn(["N001"], below=0.25)
        outcome = self.coarse_first(board)
        runs = self.runs()
        self.assertEqual([(p["step"], [n["key"] for n in p["nets"]], p["nets"][-1]["width"]) for p in runs],
                         [(0.1, ["N000", "N001", "N002"], 0.25), (0.05, ["N001"], 0.25), (0.05, ["N001"], 0.2)])
        self.assertEqual(self.said, ["next try, at a 0.05 mm step: N001", "second try, narrower: N001"])
        self.assertEqual((outcome.status, outcome.left, len(board.reads)), ("routed", 0, 3))

    def test_no_fine_try_on_a_grid_too_large(self):
        board = FakeBoard(3)
        self.stubborn(["N001"])
        # the board is 12 by 8 mm with its margin: 121 * 81 cells at 0.1 mm, four times that at 0.05
        outcome = self.coarse_first(board, options=dict(max_states=20_000))
        self.assertEqual([p["step"] for p in self.runs()], [0.1])
        self.assertEqual(len([line for line in self.said if "larger than --max-states" in line]), 2)
        self.assertEqual((outcome.status, outcome.left, outcome.failure), ("unroutable", 1, None))


class EngineFailure(WithEngine):
    def test_failure_at_the_first_try_lays_nothing(self):
        board = FakeBoard(3)
        self.fail_on(1)
        outcome = self.route(board)
        self.assertIn("told to fail", outcome.failure)
        self.assertFalse(outcome.laid)
        self.assertEqual((outcome.wanted, outcome.left, board.writes), (3, 3, []))

    def test_failure_at_the_second_try_keeps_the_first(self):
        """Defect 2: the first try's tracks used to be lost."""
        board = FakeBoard(3)
        self.stubborn(["N001"])
        self.fail_on(2)
        outcome = self.route(board)
        self.assertIn("told to fail", outcome.failure)
        self.assertTrue(outcome.laid)
        self.assertEqual((outcome.wanted, outcome.left, outcome.tracks), (3, 1, 2))
        self.assertEqual(board.writes, [["N000", "N002"]])
        self.assertNotEqual(outcome.status, "routed")

    def test_failure_in_a_later_batch_keeps_the_earlier_ones(self):
        board = FakeBoard(300)
        self.fail_on(2)
        outcome = self.route(board)
        self.assertTrue(outcome.failure and outcome.laid)
        self.assertEqual((outcome.left, outcome.tracks), (150, 150))
        self.assertEqual(len(self.runs()), 2)  # it stops there

    def test_main_saves_what_was_laid_before_the_failure(self):
        board = FakeBoard(3)
        self.stubborn(["N001"])
        self.fail_on(2)
        status, saved, out, err = self.main(board)
        self.assertEqual((status, saved), (2, ["out.kicad_pcb"]))
        self.assertIn("2 of 3 nets joined", out.splitlines()[-1])
        self.assertIn("told to fail", err)

    def test_main_saves_nothing_if_nothing_was_laid(self):
        self.fail_on(1)
        status, saved, out, err = self.main(FakeBoard(3))
        self.assertEqual((status, saved), (2, []))
        self.assertIn("told to fail", err)

    def test_main_reports_and_saves_a_complete_run(self):
        status, saved, out, err = self.main(FakeBoard(2), "--problem", str(self.dir / "p.json"), "--solution", str(self.dir / "s.json"))
        self.assertEqual((status, saved), (0, ["out.kicad_pcb"]))
        self.assertEqual(out.splitlines()[-1], "routed: 2 of 2 nets joined, 2 track segments and 0 vias added; saved out.kicad_pcb")
        self.assertEqual(len(json.loads((self.dir / "p.json").read_text())["nets"]), 2)
        self.assertEqual(json.loads((self.dir / "s.json").read_text())["status"], "routed")

    def test_main_with_nets_left_unjoined(self):
        self.stubborn(["N001"])
        status, saved, out, err = self.main(FakeBoard(2))
        self.assertEqual((status, saved), (1, ["out.kicad_pcb"]))
        self.assertEqual(out.splitlines()[-1], "unroutable: 1 of 2 nets joined, 1 track segments and 0 vias added; saved out.kicad_pcb")

    def test_main_with_nothing_to_route(self):
        board = FakeBoard(2)
        board.joined = set(board.names)
        status, saved, out, err = self.main(board)
        self.assertEqual((status, saved), (0, ["out.kicad_pcb"]))
        self.assertIn("nothing to route", out)

    def main(self, board, *more):
        """Run the bridge's main() on a fake board, with the fake engine as the program; KiCad's part is stood in for."""
        engine = self.dir / "engine"
        engine.write_text(f"#!/bin/sh\nexec '{sys.executable}' '{support.FAKE_ENGINE}' \"$@\"\n")
        engine.chmod(0o755)
        saved = []
        kicad = mock.patch.multiple(support.stub_pcbnew(), create=True, LoadBoard=lambda path: mock.Mock(),
                                    SaveBoard=lambda path, b: saved.append(path), ZONE_FILLER=lambda b: mock.Mock())
        out, err = io.StringIO(), io.StringIO()
        with kicad, mock.patch.object(k, "KicadBoard", lambda b, ignored: board), redirect_stdout(out), redirect_stderr(err):
            status = k.main(["in.kicad_pcb", "-o", "out.kicad_pcb", "--engine", str(engine), *more])
        return status, saved, out.getvalue(), err.getvalue()


class Batches(WithEngine):
    def test_split(self):
        names = [str(i) for i in range(1000)]
        self.assertEqual(k.batches([]), [])
        self.assertEqual(k.batches(names[:249]), [names[:249]])
        self.assertEqual([len(b) for b in k.batches(names[:250])], [125, 125])
        self.assertEqual([len(b) for b in k.batches(names[:401])], [134, 134, 133])
        self.assertEqual([len(b) for b in k.batches(names)], [200] * 5)
        self.assertEqual(sum(k.batches(names[:777]), []), names[:777])  # every net once, in order

    def test_fewer_than_the_engine_limit_go_in_one_run(self):
        outcome = self.route(FakeBoard(249))
        self.assertEqual([len(p["nets"]) for p in self.runs()], [249])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 249, 0))
        self.assertEqual(self.said, [])

    def test_more_are_routed_in_batches(self):
        """Defect 3: the engine refuses 250 entries or more."""
        board = FakeBoard(450)
        outcome = self.route(board)
        problems = self.runs()
        self.assertEqual([len(p["nets"]) for p in problems], [150, 150, 150])
        self.assertEqual(sorted(n["key"] for p in problems for n in p["nets"]), board.names)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left, outcome.tracks), ("routed", 450, 0, 450))
        # it says so
        self.assertIn("450 nets", self.said[0])
        self.assertIn("3 batches of at most 150", self.said[0])
        # each batch is built from the board as the one before left it: read again each time
        self.assertEqual(len(board.reads), 3)
        self.assertEqual(board.reads[1], set(board.names[150:300]))

    def test_each_batch_has_its_second_try(self):
        board = FakeBoard(260)
        self.stubborn(["N005", "N200", "N201"], below=0.25)
        board.narrow_pads = {"N005", "N200", "N201"}
        outcome = self.route(board)
        self.assertEqual([len(p["nets"]) for p in self.runs()], [130, 1, 130, 2])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 260, 0))

    def test_what_stays_unjoined_is_counted_over_all_batches(self):
        board = FakeBoard(260)
        self.stubborn(["N005", "N200"])
        outcome = self.route(board)
        self.assertEqual((outcome.status, outcome.wanted, outcome.left, outcome.tracks), ("unroutable", 260, 2, 258))


class RunEngine(WithEngine):
    problem = dict(window=[0, 0, 1, 1], step=0.05, layers=["F.Cu"], nets=[])

    def test_solution_and_kept_files(self):
        with redirect_stdout(io.StringIO()) as out:
            solution = k.run_engine(self.problem, ENGINE, threads=2, problem_path=self.dir / "p.json", solution_path=self.dir / "s.json")
        self.assertEqual(solution["status"], "routed")
        self.assertEqual(json.loads((self.dir / "p.json").read_text()), self.problem)
        self.assertEqual(json.loads((self.dir / "s.json").read_text()), solution)
        self.assertIn("fake run 1", out.getvalue())  # the engine's report is passed on

    def test_an_old_solution_file_is_not_taken_for_the_answer(self):
        (self.dir / "s.json").write_text(json.dumps(dict(status="routed", nets=[])))
        self.fail_on(1)
        with redirect_stdout(io.StringIO()), self.assertRaises(k.EngineFailed) as raised:
            k.run_engine(self.problem, ENGINE, solution_path=self.dir / "s.json")
        self.assertIn("told to fail", str(raised.exception))
        self.assertFalse((self.dir / "s.json").exists())

    def test_a_program_that_is_not_there(self):
        with self.assertRaises(k.EngineFailed) as raised:
            k.run_engine(self.problem, str(self.dir / "no-such-engine"))
        self.assertIn("no-such-engine", str(raised.exception))

    def test_a_program_that_ends_without_a_solution(self):
        with redirect_stdout(io.StringIO()), self.assertRaises(k.EngineFailed):
            k.run_engine(self.problem, [sys.executable, "-c", "import sys; sys.exit(0)"])


class CapacitorBoard(FakeBoard):
    """One net of three pads: a pin of U1, a capacitor beside it, a connector far off."""

    def __init__(self):
        super().__init__(0)
        self.names = ["VCC"]

    def read(self, nets=None):
        self.reads.append(None if nets is None else set(nets))
        pads = [support.land(k, "VCC", 2.0, 2.0, part="U1"), support.land(k, "VCC", 4.0, 2.0, part="C1"),
                support.land(k, "GND", 4.0, 4.0, part="C1"), support.land(k, "VCC", 18.0, 2.0, part="J1"),
                support.land(k, "GND", 2.0, 4.0, part="U1"), support.land(k, "GND", 18.0, 4.0, part="J1")]
        groups = {"VCC": [[0, 1, 3]] if "VCC" in self.joined else [[0], [1], [3]], "GND": [[2, 4, 5]]}
        picture = support.picture(k, pads, groups)
        picture.box = [0.0, 0.0, 20.0, 6.0]
        return picture

    def write(self, solution, sizes):
        added = super().write(solution, sizes)
        self.joined.discard("VCC")
        if all(n["tracks"] for n in solution["nets"]):
            self.joined.add("VCC")
        return added


class Decoupling(WithEngine):
    def test_the_capacitor_is_linked_to_its_pin(self):
        board = CapacitorBoard()
        outcome = self.route(board)
        self.assertEqual([n["key"] for n in self.runs()[0]["nets"]], ["VCC | C1", "VCC"])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 1, 0))
        self.assertEqual(self.said, [])

    def test_a_link_that_cannot_be_laid_costs_nothing(self):
        board = CapacitorBoard()
        self.stubborn(["VCC | C1"])
        outcome = self.route(board)
        first, second = self.runs()
        self.assertEqual([n["key"] for n in first["nets"]], ["VCC | C1", "VCC"])
        self.assertEqual([n["key"] for n in second["nets"]], ["VCC"])  # the same net again, whole and unlinked
        self.assertEqual(len(second["nets"][0]["terminals"]), 3)
        self.assertEqual(board.writes, [["VCC"]])  # nothing of the first run was kept
        self.assertEqual(len(board.reads), 1)
        self.assertEqual(self.said, ["not all joined with the capacitors linked to their pins: again without those links for VCC"])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("routed", 1, 0))


class CapacitorAmongOthers(FakeBoard):
    """Three ordinary nets, and a supply with a capacitor by a pin."""

    def read(self, nets=None):
        plain = super().read(nets)
        base = len(plain.pads)
        pads = plain.pads + [support.land(k, "VCC", 2.0, 20.0, part="U1"), support.land(k, "VCC", 4.0, 20.0, part="C1"),
                             support.land(k, "GND", 4.0, 22.0, part="C1"), support.land(k, "VCC", 18.0, 20.0, part="J1"),
                             support.land(k, "GND", 2.0, 22.0, part="U1"), support.land(k, "GND", 18.0, 22.0, part="J1")]
        groups = dict(plain.groups, GND=[[base + 2, base + 4, base + 5]],
                      VCC=[[base, base + 1, base + 3]] if "VCC" in self.joined else [[base], [base + 1], [base + 3]])
        picture = support.picture(k, pads, groups)
        picture.box = [0.0, 0.0, 20.0, 24.0]
        return picture

    def write(self, solution, sizes):
        added = super().write(solution, sizes)
        if not all(n["tracks"] for n in solution["nets"] if n["net"] == "VCC"):
            self.joined.discard("VCC")
        return added

    def unjoined(self, nets=None):
        return [n for n in self.names + ["VCC"] if n not in self.joined and (nets is None or n in nets)]


class LinksAndOtherNets(WithEngine):
    def test_another_net_left_unjoined_does_not_cost_the_links(self):
        board = CapacitorAmongOthers(3)
        self.stubborn(["N001"])
        outcome = self.route(board)
        first, second = self.runs()
        self.assertIn("VCC | C1", [n["key"] for n in first["nets"]])
        self.assertEqual([n["key"] for n in second["nets"]], ["N001"])  # the narrower try, for the net left over
        self.assertIn("VCC | C1", board.writes[0])
        self.assertEqual((outcome.status, outcome.wanted, outcome.left), ("unroutable", 4, 1))


if __name__ == "__main__":
    unittest.main()
