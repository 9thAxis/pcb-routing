"""How a track leaves a long pad: the ground beside its long sides, and a route that goes by it. No KiCad."""
import math, unittest

import support

k = support.bridge()


def oblong(x, y, long=1.6, wide=0.6, turned=0.0):
    """The outline of a pad `long` by `wide` round (x, y), its length turned by `turned` degrees."""
    c, s = math.cos(math.radians(turned)), math.sin(math.radians(turned))
    corners = [(-long / 2, -wide / 2), (long / 2, -wide / 2), (long / 2, wide / 2), (-long / 2, wide / 2)]
    return [[x + u * c - v * s, y + u * s + v * c] for u, v in corners]


def pad(net, x, y, **shape):
    return k.Pad(net=net, at=[x, y], layers=["F.Cu"], shapes={"F.Cu": [[oblong(x, y, **shape)]]})


class LongPads(unittest.TestCase):
    def test_a_long_pad_has_a_middle_a_direction_and_two_halves(self):
        middle, along, half, narrow = k.long_pad(oblong(5, 3))
        self.assertEqual(([round(v, 6) for v in middle], round(half, 6), round(narrow, 6)), ([5, 3], 0.8, 0.3))
        self.assertAlmostEqual(abs(along[0]), 1.0)

    def test_its_direction_is_found_however_it_is_turned(self):
        _, along, half, narrow = k.long_pad(oblong(5, 3, turned=30))
        self.assertAlmostEqual(abs(along[0] * math.cos(math.radians(30)) + along[1] * math.sin(math.radians(30))), 1.0)
        self.assertEqual((round(half, 6), round(narrow, 6)), (0.8, 0.3))

    def test_a_pad_that_is_nearly_square_or_round_has_no_ends(self):
        self.assertIsNone(k.long_pad(oblong(0, 0, long=1.0, wide=0.8)))
        disc = [[math.cos(2 * math.pi * i / 24), math.sin(2 * math.pi * i / 24)] for i in range(24)]
        self.assertIsNone(k.long_pad(disc))

    def test_the_ground_beside_it_lies_along_its_long_sides_only(self):
        one, other = k.beside(oblong(5, 3), 0.2)
        ys = lambda ring: sorted({round(y, 6) for _, y in ring})
        xs = lambda ring: sorted({round(x, 6) for x, _ in ring})
        self.assertEqual((xs(one), xs(other)), ([4.2, 5.8], [4.2, 5.8]))
        self.assertEqual(sorted([ys(one), ys(other)]), [[2.5, 2.7], [3.3, 3.5]])
        self.assertEqual(k.beside(oblong(0, 0, long=1.0, wide=0.8), 0.2), [])


class Exits(unittest.TestCase):
    def test_only_the_pads_own_net_pays_and_only_on_a_routing_layer(self):
        pads = [pad("A", 5, 3), pad("B", 9, 3), pad("A", 1, 1, long=0.8, wide=0.8)]
        dear = k.exits(pads, {"A"}, ["F.Cu", "B.Cu"], 0.5, 0.05)
        self.assertEqual([(d["only"], d["layers"], d["factor"]) for d in dear], [("A", ["F.Cu"], 3.5)] * 2)
        self.assertEqual(k.exits(pads, {"A"}, ["B.Cu"], 0.5, 0.05), [])
        self.assertEqual(k.exits(pads, {"A"}, ["F.Cu"], 0.0, 0.05), [])

    def test_the_factor_stays_within_what_the_router_takes(self):
        self.assertEqual(k.exits([pad("A", 5, 3)], {"A"}, ["F.Cu"], 10.0, 0.01)[0]["factor"], 51.0)
        self.assertLessEqual(k.exits([pad("A", 5, 3)], {"A"}, ["F.Cu"], 1000.0, 0.01)[0]["factor"], 100.0)

    def test_a_track_to_a_pad_off_to_the_side_leaves_through_the_end(self):
        # a pad lying along x, and a pad up and to the right of it: the short way is at 45 degrees from its
        # middle, through its long side
        pads = [pad("A", 5, 5), pad("A", 9, 2, long=0.8, wide=0.8)]
        picture = support.picture(k, pads)
        def leaves(cost):
            rules = k.Rules.of(picture, k.Settings(step=0.05, exit_cost=cost))
            solution = k.Engine(support.PROGRAM).route(k.problem_of(picture, rules, support.ask))
            self.assertEqual(solution["status"], "routed")
            points = [p for track in solution["nets"][0]["tracks"] for p in track["points"]]
            # where the track is as it passes the end of the pad (x = 5.8)
            beyond = min((p for p in points if p[0] >= 5.8 - 1e-6), key=lambda p: p[0])
            before = max((p for p in points if p[0] <= 5.8 + 1e-6), key=lambda p: p[0])
            t = (5.8 - before[0]) / (beyond[0] - before[0]) if beyond[0] > before[0] else 0.0
            return before[1] + t * (beyond[1] - before[1])
        self.assertLess(leaves(0.0), 4.7)  # out through the long side (y = 4.7), well before the end
        self.assertAlmostEqual(leaves(0.5), 5.0, delta=0.3)  # along the pad and out through its end


if __name__ == "__main__":
    unittest.main()
