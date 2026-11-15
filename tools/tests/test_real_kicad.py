"""The code that reads zones, against KiCad itself: boards made by kicad_scenes.py with KiCad's Python, and
checked by KiCad's design rule check. Skipped where KiCad is not installed.

KiCad's Python is taken from $KICAD_PYTHON, its command line program from $KICAD_CLI, the engine from
$PCB_ROUTING or the repository's release build; on macOS the first two are looked for in the application.
"""
import json, os, shutil, subprocess, tempfile, unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
APP = Path("/Applications/KiCad/KiCad.app/Contents")


def program(variable, *candidates):
    named = os.environ.get(variable)
    found = [c for c in ([named] if named else list(candidates)) if c and (Path(c).is_file() or shutil.which(str(c)))]
    return str(found[0]) if found else None


PYTHON = program("KICAD_PYTHON", APP / "Frameworks/Python.framework/Versions/Current/bin/python3")
CLI = program("KICAD_CLI", APP / "MacOS/kicad-cli", "kicad-cli")
ENGINE = program("PCB_ROUTING", HERE.parent.parent / "target/release/pcb-routing")


@unittest.skipUnless(PYTHON and CLI, "KiCad is not installed")
class RealKicad(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)

    def scene(self, name):
        run = subprocess.run([PYTHON, str(HERE / "kicad_scenes.py"), name, str(self.dir)], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        return json.loads(run.stdout.strip().splitlines()[-1])

    def check(self, board):
        """KiCad's design rule check of a board, its zones filled again: (kinds of error, unconnected items)."""
        report = self.dir / "drc.json"
        subprocess.run([CLI, "pcb", "drc", "--severity-error", "--format", "json", "--refill-zones", "-o", str(report), str(board)],
                       capture_output=True)
        found = json.loads(report.read_text())
        return sorted(v["type"] for v in found["violations"]), len(found["unconnected_items"])

    def test_spokes_are_counted_as_kicad_counts_them_and_a_track_to_the_zone_counts_as_one(self):
        said = self.scene("spokes")
        self.assertEqual(said["whole"], {"J1": 3, "J2": 4})  # in the corner of the zone one spoke has nowhere to go
        self.assertEqual((said["cut"], said["starved"]), ({"J1": 1, "J2": 4}, ["J1"]))  # two more cut by the track
        self.assertTrue(said["targets"])
        self.assertEqual(said["tied"], {"J1": 2, "J2": 4})
        self.assertEqual(said["least"], [2, 2])
        self.assertEqual(self.check(said["saved"]), ([], 0))  # KiCad agrees: with the track, no pad is starved

    def test_a_pad_cut_off_from_two_spokes_is_an_error_for_kicad_until_it_is_tied(self):
        # (the same scene without the tie: what the count of one stands for)
        said = self.scene("starved")
        self.assertEqual(said["cut"], {"J1": 1, "J2": 4})
        self.assertEqual(self.check(said["saved"]), (["starved_thermal"], 0))

    def test_a_zone_joins_the_pads_it_touches_and_a_track_across_it_parts_them(self):
        said = self.scene("groups")
        self.assertEqual(said["whole"], [["J1", "J2", "J3"], ["J4"]])
        self.assertEqual(said["cut"], [["J1", "J2"], ["J3"], ["J4"]])
        self.assertEqual((said["fills"], said["pours"]), ([1, 2], [[1], [1, 1]]))
        self.assertEqual(said["unjoined"], ["GND"])
        self.assertEqual(said["without"], [["J1"], ["J2"], ["J3"], ["J4"]])  # a zone joins nothing unless asked
        self.assertEqual(self.check(said["saved"])[1], 2)  # KiCad: J3 and J4 each to be joined to the rest

    @unittest.skipUnless(ENGINE, "the engine is not built")
    def test_a_board_with_a_zone_is_routed_clean_with_and_without_pours(self):
        said = self.scene("to_route")
        lengths = {}
        for pours in (True, False):
            out = self.dir / f"routed-{pours}.kicad_pcb"
            run = subprocess.run([PYTHON, str(HERE.parent / "kicad_route.py"), said["saved"], "-o", str(out), "--engine", ENGINE]
                                 + (["--pours"] if pours else []), capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
            self.assertEqual(self.check(out), ([], 0), run.stdout)
            lengths[pours] = int(run.stdout.strip().splitlines()[-1].split(" track segments")[0].split()[-1])
            if pours:
                self.assertIn("routed: 3 of 3 nets joined", run.stdout)
        self.assertLess(lengths[True], lengths[False])  # the zone carries GND: only the pad beside it gets a track


if __name__ == "__main__":
    unittest.main()
