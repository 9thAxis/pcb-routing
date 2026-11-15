"""Scoring dumps against a reference: which boards count, the sums, the tables, the files and the command lines.
Needs shapely and numpy; skipped without them."""
import contextlib, io, json, math, shutil, sys, tempfile, unittest
from pathlib import Path

import quality_support as q
from quality_support import board, pad, track, via

score = q.scoring("score")
quality = q.load("quality")

PADS = [pad("A", 0, 0, ref="J1"), pad("A", 3, 4, ref="J2"), pad("GND", 0, 9, ref="J1", num="2"), pad("GND", 5, 9, ref="J2", num="2")]


def reference(**more):
    """The designer's board: A straight, GND poured."""
    return board(PADS, [track("A", (0, 0), (3, 4))], zone_nets=["GND"], **more)


def bent(**more):
    """The same board with A laid as an L through a via, and GND as a track."""
    more.setdefault("unconnected", 0)
    return board(PADS, [track("A", (0, 0), (3, 0)), track("A", (3, 0), (3, 4)), track("GND", (0, 9), (5, 9))], [via("A", 3, 0)], **more)


def one(routings, **options):
    names = ["designer"] + list(routings)
    return score.compare([("b", reference(), routings)], names, **options)


class Compare(unittest.TestCase):
    def test_rows_for_the_reference_and_each_routing(self):
        result = one({"x": bent()})
        general = result["general"]
        self.assertEqual((result["boards"], result["skipped"], result["names"]), (["b"], {}, ["designer", "x"]))
        self.assertEqual((general["boards"], general["nets"], general["connections"]), (1, 1, 1))  # GND is poured: not compared
        self.assertAlmostEqual(general["rows"]["designer"]["length_vs_reference"], 1.0)
        self.assertAlmostEqual(general["rows"]["designer"]["detour"], 1.0)
        self.assertAlmostEqual(general["rows"]["x"]["length_vs_reference"], 1.4)
        self.assertAlmostEqual(general["rows"]["x"]["detour"], 1.4)
        self.assertAlmostEqual(general["rows"]["x"]["vias_per_100"], 100.0)
        self.assertAlmostEqual(general["rows"]["x"]["corners_per_connection"], 1.0)
        self.assertEqual((result["exits"]["rows"]["designer"]["exits"], result["exits"]["rows"]["x"]["exits"]), (2, 2))

    def test_an_incomplete_routing_takes_the_board_out_for_everyone(self):
        result = one({"x": bent(), "y": bent(unconnected=1)})
        self.assertEqual(result["boards"], [])
        self.assertIn("y has 1 open", result["skipped"]["b"])
        self.assertEqual(result["general"]["rows"]["x"]["mm"], 0)

    def test_the_caller_can_state_that_routings_are_complete(self):
        old = bent()
        del old["unconnected"]
        self.assertIn("does not say", one({"x": old})["skipped"]["b"])
        self.assertEqual(one({"x": old}, assume_complete=True)["boards"], ["b"])

    def test_a_missing_dump_takes_the_board_out(self):
        self.assertEqual(one({"x": bent(), "y": None})["skipped"], {"b": "no dump of y"})

    def test_a_routing_of_another_board_takes_it_out(self):
        other = board(PADS[:2], unconnected=0)
        self.assertIn("pads differ", one({"x": other})["skipped"]["b"])

    def test_boards_add_up(self):
        boards = [("b1", reference(), {"x": bent()}), ("b2", reference(), {"x": bent()}), ("b3", reference(), {"x": bent(unconnected=2)})]
        result = score.compare(boards, ["designer", "x"])
        self.assertEqual((result["boards"], list(result["skipped"])), (["b1", "b2"], ["b3"]))
        self.assertEqual((result["general"]["boards"], result["general"]["connections"]), (2, 2))
        self.assertAlmostEqual(result["general"]["rows"]["x"]["mm"], 14.0)
        self.assertAlmostEqual(result["general"]["rows"]["x"]["detour"], 1.4)

    def test_the_clearance_comes_from_the_rules_unless_given(self):
        def tight(clearance=None, rules=None):
            near = bent()
            near["tracks"].append(track("B", (0, 0.6), (2, 0.6)))  # 0.4 mm of air beside A's first leg
            ref = reference(**({"rules": rules} if rules else {}))
            return score.compare([("b", ref, {"x": near})], ["designer", "x"], clearance=clearance)["general"]["rows"]["x"]["tight_percent"]
        self.assertEqual(tight(), 0.0)  # 0.2 mm by default: two clearances away
        self.assertGreater(tight(rules=dict(clearance=0.4)), 25.0)
        self.assertGreater(tight(clearance=0.4), 25.0)
        self.assertEqual(tight(clearance=0.2, rules=dict(clearance=0.4)), 0.0)

    def test_per_net_rows_on_request(self):
        self.assertNotIn("nets", one({"x": bent()}))
        rows = one({"x": bent()}, per_net=True)["nets"]
        self.assertEqual([(r["board"], r["net"], r["routing"], r["connections"]) for r in rows], [("b", "A", "designer", 1), ("b", "A", "x", 1)])
        self.assertAlmostEqual(rows[1]["mm"], 7.0)

    def test_a_board_without_comparable_nets_still_counts_for_the_other_tables(self):
        poured = board(PADS, [track("GND", (0, 9), (5, 9))], zone_nets=["GND"])
        result = score.compare([("b", poured, {"x": bent()})], ["designer", "x"])
        self.assertEqual((result["boards"], result["general"]["boards"]), (["b"], 0))


class Output(unittest.TestCase):
    def test_tables_have_a_row_for_every_name_in_every_table(self):
        text = score.tables(one({"x": bent(), "a-long-name-for-a-router": bent()}))
        self.assertIn("1 boards compared, 0 skipped", text)
        self.assertIn("general: 1 boards, 1 nets, 1 connections", text)
        for name in ("designer", "x", "a-long-name-for-a-router"):
            self.assertEqual(sum(line.startswith(name + " ") for line in text.splitlines()), 4, name)
        widths = {len(line) for line in text.splitlines() if line.startswith(("designer ", "x ", "a-long")) and "|" not in line and "." in line}
        self.assertLessEqual(len(widths), 3)  # one width per table

    def test_json_holds_no_nan(self):
        result = one({"x": bent()})
        self.assertTrue(math.isnan(result["circuit"]["rows"]["x"]["decap_path_over_straight_median"]))
        text = json.dumps(score.plain(result), allow_nan=False)
        self.assertIsNone(json.loads(text)["circuit"]["rows"]["x"]["decap_path_over_straight_median"])


class Files(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)

    def write(self, name, dump):
        (self.dir / name).write_text(json.dumps(dump))
        return str(self.dir / name)

    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                status = score.main(list(argv))
            except SystemExit as stop:
                status = stop.code
        return status, out.getvalue(), err.getvalue()

    def many(self):
        for name in ("b1", "b2", "b.with.dots"):
            self.write(f"{name}.human.json", reference())
            self.write(f"{name}.x.json", bent())
        self.write("b2.y.json", bent())
        self.write("stray.x.json", bent())

    def test_boards_of_a_directory(self):
        self.many()
        found = list(score.boards_in(self.dir, "human", ["x", "y"]))
        self.assertEqual([name for name, _, _ in found], ["b.with.dots", "b1", "b2"])
        self.assertEqual([sorted(n for n, d in routings.items() if d) for _, _, routings in found], [["x"], ["x"], ["x", "y"]])
        self.assertEqual([name for name, _, _ in score.boards_in(self.dir, "human", ["x"], only={"b2"})], ["b2"])

    def test_one_board(self):
        status, out, err = self.run_main(self.write("ref.json", reference()), "mine=" + self.write("mine.json", bent()))
        self.assertEqual((status, err), (0, ""))
        self.assertRegex(out, r"(?m)^reference +5 +1\.000 +1\.000 ")
        self.assertRegex(out, r"(?m)^mine +7 +1\.400 +1\.400 +100\.0 +1\.00 +100\.0 ")

    def test_the_reference_s_row_can_be_named(self):
        _, out, _ = self.run_main(self.write("ref.json", reference()), "mine=" + self.write("mine.json", bent()), "--reference", "designer")
        self.assertRegex(out, r"(?m)^designer +5 ")

    def test_an_incomplete_routing_is_named_and_the_status_is_1(self):
        status, out, err = self.run_main(self.write("ref.json", reference()), "mine=" + self.write("mine.json", bent(unconnected=2)))
        self.assertEqual((status, out), (1, ""))
        self.assertIn("skipped ref: mine has 2 open connections", err)
        self.assertEqual(self.run_main(str(self.dir / "ref.json"), "mine=" + str(self.dir / "mine.json"), "--assume-complete")[0], 0)

    def test_a_directory_of_boards(self):
        self.many()
        status, out, err = self.run_main("--dir", str(self.dir), "--reference", "human", "x", "y", "--json", str(self.dir / "out.json"))
        self.assertEqual(status, 0)
        self.assertIn("1 boards compared, 2 skipped", out)
        self.assertIn("skipped b1: no dump of y", err)
        written = json.loads((self.dir / "out.json").read_text())
        self.assertEqual((written["boards"], sorted(written["skipped"]), written["names"]), (["b2"], ["b.with.dots", "b1"], ["human", "x", "y"]))
        self.assertAlmostEqual(written["general"]["rows"]["y"]["detour"], 1.4)

    def test_boards_can_be_chosen(self):
        self.many()
        (self.dir / "boards.txt").write_text("b1\n")
        status, out, _ = self.run_main("--dir", str(self.dir), "--reference", "human", "x", "--boards", str(self.dir / "boards.txt"))
        self.assertIn("1 boards compared, 0 skipped", out)

    def test_json_alone_on_standard_output(self):
        status, out, _ = self.run_main(self.write("ref.json", reference()), "mine=" + self.write("mine.json", bent()), "--json", "-", "--per-net")
        self.assertEqual(json.loads(out)["nets"][1]["routing"], "mine")

    def test_what_is_refused(self):
        ref, mine = self.write("ref.json", reference()), self.write("mine.json", bent())
        for argv in ([ref], [ref, mine], [f"a={ref}", f"b={mine}"], [ref, f"reference={mine}"], [ref, f"a={mine}", f"a={mine}"],
                     ["--dir", str(self.dir), "x"], ["--dir", str(self.dir), "--reference", "human", f"x={mine}"], [ref, f"a={mine}", "--boards", ref]):
            with self.subTest(argv):
                self.assertEqual(self.run_main(*argv)[0], 2)


class OneCommand(unittest.TestCase):
    def test_kicad_s_python_is_the_one_named_or_the_first_that_has_pcbnew(self):
        self.assertEqual(quality.kicad_python({"KICAD_PYTHON": "/k/python"}, works=lambda p: True), "/k/python")
        self.assertIsNone(quality.kicad_python({"KICAD_PYTHON": "/k/python"}, works=lambda p: p != "/k/python"))
        self.assertEqual(quality.kicad_python({}, works=lambda p: True), quality.MACOS_PYTHON)
        self.assertEqual(quality.kicad_python({}, works=lambda p: p == sys.executable), sys.executable)
        self.assertIsNone(quality.kicad_python({}, works=lambda p: False))

    def test_a_python_that_is_not_there_has_no_pcbnew(self):
        self.assertFalse(quality.has_pcbnew("/no/such/python"))

    def test_the_dump_command(self):
        self.assertEqual(quality.dump_command("py", "b.kicad_pcb", "out.json"), ["py", str(q.QUALITY / "dump.py"), "b.kicad_pcb", "-o", "out.json"])
        self.assertEqual(quality.dump_command("py", "b.kicad_pcb", "out.json", "p.kicad_pro")[-2:], ["--project", "p.kicad_pro"])

    def run_main(self, tmp, *more, dumps):
        """quality.main with a stand-in for KiCad's Python that copies a prepared dump instead of reading a board."""
        for name, dump in dumps.items():
            (tmp / f"{name}.kicad_pcb").write_text(json.dumps(dump))
        copy = "import shutil, sys; shutil.copy(sys.argv[1], sys.argv[2])"
        saved = quality.kicad_python, quality.dump_command
        quality.kicad_python = lambda: sys.executable
        quality.dump_command = lambda python, board, output, project=None: [python, "-c", copy, str(board), str(output)]
        self.addCleanup(lambda: [setattr(quality, "kicad_python", saved[0]), setattr(quality, "dump_command", saved[1])])
        out, err = io.StringIO(), io.StringIO()
        argv = [str(tmp / "ref.kicad_pcb")] + [f"{name}={tmp / name}.kicad_pcb" for name in dumps if name != "ref"] + list(more)
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                status = quality.main(argv)
            except SystemExit as stop:
                status = stop.code
        return status, out.getvalue(), err.getvalue()

    def test_boards_are_dumped_then_scored(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            status, out, err = self.run_main(tmp, "--dumps", str(tmp / "kept"), dumps=dict(ref=reference(), mine=bent()))
            self.assertEqual((status, err), (0, ""))
            self.assertRegex(out, r"(?m)^mine +7 +1\.400 ")
            self.assertEqual(sorted(p.name for p in (tmp / "kept").iterdir()), ["mine.json", "reference.json"])

    def test_an_incomplete_routing_is_not_scored(self):
        with tempfile.TemporaryDirectory() as tmp:
            status, out, err = self.run_main(Path(tmp), dumps=dict(ref=reference(), mine=bent(unconnected=3)))
            self.assertEqual(status, 1)
            self.assertIn("mine has 3 open connections", err)

    def test_a_board_that_cannot_be_dumped(self):
        with tempfile.TemporaryDirectory() as tmp:
            status, out, err = self.run_main(Path(tmp), "missing=" + str(Path(tmp) / "missing.kicad_pcb"), dumps=dict(ref=reference()))
            self.assertEqual(status, 2)
            self.assertIn("could not dump", err)

    def test_what_is_refused(self):
        for argv in (["ref.kicad_pcb", "routed.kicad_pcb"], ["ref.kicad_pcb", "a=x.kicad_pcb", "a=y.kicad_pcb"], ["ref.kicad_pcb", "reference=x.kicad_pcb"],
                     ["ref.kicad_pcb", "a/b=x.kicad_pcb"]):
            with self.subTest(argv), self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                quality.main(argv)


class Picture(unittest.TestCase):
    show = q.load("show")

    def test_panels_as_square_as_it_goes(self):
        self.assertEqual([self.show.grid(n) for n in (1, 2, 3, 4, 5)], [(1, 1), (1, 2), (2, 2), (2, 2), (2, 3)])
        self.assertEqual(self.show.grid(5, columns=5), (1, 5))

    def test_track_widths_follow_the_part_shown(self):
        self.assertAlmostEqual(self.show.line_width(0.25, [0, 0, 10, 10]), 0.25 * 64.8)
        self.assertEqual(self.show.line_width(0.01), 0.6)

    def test_a_picture_is_written(self):
        try:
            import matplotlib  # noqa: F401
        except ImportError:
            self.skipTest("matplotlib is not installed")
        with tempfile.TemporaryDirectory() as tmp:
            ref, mine = Path(tmp) / "ref.json", Path(tmp) / "mine.json"
            ref.write_text(json.dumps(reference()))
            mine.write_text(json.dumps(bent()))
            self.assertEqual(self.show.main([str(Path(tmp) / "out.png"), f"designer={ref}", f"mine={mine}", "--box", "-1", "-1", "6", "10"]), 0)
            self.assertEqual((Path(tmp) / "out.png").read_bytes()[:4], b"\x89PNG")


if __name__ == "__main__":
    unittest.main()
