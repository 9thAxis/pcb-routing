"""One step of a board's work: the layers it is tried on, and what a try came to."""
import copy
import unittest

import support

k = support.bridge()


def board():
    """Four layers, the second a ground plane; a pair of nets from the left to the right on the top layer."""
    pads = [support.land(k, "D_P", 2.0, 4.0, half=0.3), support.land(k, "D_N", 2.0, 5.0, half=0.3),
            support.land(k, "D_P", 18.0, 4.0, half=0.3), support.land(k, "D_N", 18.0, 5.0, half=0.3)]
    picture = support.picture(k, pads)
    picture.layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
    picture.box = [0.0, 0.0, 20.0, 10.0]
    picture.fills = [k.Pour("GND", "In1.Cu", [[[0.5, 0.5], [19.5, 0.5], [19.5, 9.5], [0.5, 9.5]]]),
                     k.Pour("GND", "B.Cu", [[[0.5, 0.5], [19.5, 0.5], [19.5, 9.5], [0.5, 9.5]]])]
    picture.tracks = [k.Track([1.0, 1.0], [9.0, 1.0], 0.25, "X", "In2.Cu"), k.Track([9.0, 1.0], [9.5, 1.5], 0.25, "X", "In2.Cu")]
    return picture


class Steps(unittest.TestCase):
    def setUp(self):
        self.picture = board()
        plan = k.decompose(self.picture, k.Rules.of(self.picture, k.Settings()))
        (self.step,) = plan.steps

    def test_an_inner_layer_that_zones_cover_is_a_plane_and_an_outer_one_is_not(self):
        self.assertEqual(k.planes(self.picture), ["In1.Cu"])
        mm, across, up = k.habits(self.picture)["In2.Cu"]
        self.assertEqual((round(mm, 1), across, up), (8.7, 0.92, 0.0))

    def test_a_step_is_tried_on_its_own_layers_then_with_each_other_signal_layer_then_on_all(self):
        self.assertEqual(k.layer_options(self.picture, self.step),
                         [("its pads' own", ["F.Cu"]), ("with In2.Cu", ["F.Cu", "In2.Cu"]), ("with B.Cu", ["F.Cu", "B.Cu"]),
                          ("every signal layer", ["F.Cu", "In2.Cu", "B.Cu"])])

    def test_a_try_is_measured_by_what_it_laid_and_weighed_with_its_vias_and_what_of_a_pair_runs_alone(self):
        after = copy.deepcopy(self.picture)
        # D_P straight across; D_N beside it for the first 8 mm, then away and back on the other side
        after.tracks += [k.Track([2.0, 4.0], [18.0, 4.0], 0.25, "D_P", "F.Cu"),
                         k.Track([2.0, 5.0], [10.0, 5.0], 0.25, "D_N", "F.Cu"),
                         k.Track([10.0, 5.0], [10.0, 9.0], 0.25, "D_N", "B.Cu"),
                         k.Track([10.0, 9.0], [18.0, 9.0], 0.25, "D_N", "B.Cu"),
                         k.Track([18.0, 9.0], [18.0, 5.0], 0.25, "D_N", "B.Cu")]
        after.vias += [k.Via([10.0, 5.0], 0.6, 0.3, "D_N")]
        after.groups = {"D_P": [[0, 2]], "D_N": [[1, 3]]}
        found = k.figures(self.picture, after, self.step, [("D_N", "D_P")])
        self.assertEqual(found["nets"], {"D_N": dict(mm=24.0, vias=1), "D_P": dict(mm=16.0, vias=0)})
        self.assertEqual((found["mm"], found["vias"], found["left"], found["detour"], found["layers"]),
                         (40.0, 1, [], 1.25, ["F.Cu", "B.Cu"]))
        # a third of D_N runs beside D_P (8 of 24 mm; within a millimetre of it on the same layer)
        self.assertAlmostEqual(found["coupled"]["D_N = D_P"], 8 / 24, delta=0.02)
        self.assertEqual(k.weight(found)[0], 0)
        self.assertAlmostEqual(k.weight(found)[1], 40.0 + 10.0 + 2 * 16.0, delta=1.0)
        # a try that leaves a net unjoined is behind any that leaves none
        after.groups["D_N"] = [[1], [3]]
        self.assertGreater(k.weight(k.figures(self.picture, after, self.step, [("D_N", "D_P")])), k.weight(found))


if __name__ == "__main__":
    unittest.main()
