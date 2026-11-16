"""The code that talks to KiCad, against duck-typed fakes and a stand-in `pcbnew` module (see support.py)."""
import unittest

import support

pcbnew = support.stub_pcbnew()
k = support.bridge()


class Uuid:
    def __init__(self, text):
        self.text = text

    def AsString(self):
        return self.text


class Item:
    """A pad, track or via of a fake board."""

    def __init__(self, name, kind, net="", layers=(pcbnew.F_Cu,)):
        self.m_Uuid, self.kind, self.net, self.layers = Uuid(name), kind, net, set(layers)

    def Type(self): return self.kind
    def GetNetname(self): return self.net
    def GetNetCode(self): return 1 if self.net else 0
    def IsOnLayer(self, layer): return layer in self.layers


def pad(name, net, layers=(pcbnew.F_Cu,)):
    return Item(name, pcbnew.PCB_PAD_T, net, layers)


def track(name):
    return Item(name, pcbnew.PCB_TRACE_T)


class Connectivity:
    """What touches what: pairs of items."""

    def __init__(self, *touching):
        self.near = {}
        for a, b in touching:
            self.near.setdefault(a.m_Uuid.text, []).append(b)
            self.near.setdefault(b.m_Uuid.text, []).append(a)

    def GetConnectedTracks(self, item):
        return [o for o in self.near.get(item.m_Uuid.text, []) if o.kind != pcbnew.PCB_PAD_T]

    def GetConnectedPads(self, item):
        return [o for o in self.near.get(item.m_Uuid.text, []) if o.kind == pcbnew.PCB_PAD_T]


class Net:
    def __init__(self, name):
        self.name = name

    def GetNetname(self):
        return self.name


class Board:
    def __init__(self, pads=(), connectivity=None, nets=()):
        self.pads, self.connectivity, self.added = list(pads), connectivity or Connectivity(), []
        self.nets = {name: Net(name) for name in nets}

    def GetPads(self): return self.pads
    def GetConnectivity(self): return self.connectivity
    def Zones(self): return []
    def GetTracks(self): return []
    def GetEnabledLayers(self): return self
    def CuStack(self): return [pcbnew.F_Cu, pcbnew.B_Cu]
    def GetLayerName(self, layer): return {pcbnew.F_Cu: "F.Cu", pcbnew.B_Cu: "B.Cu"}[layer]
    def GetNetsByName(self): return self.nets
    def Add(self, item): self.added.append(item)


def names(items):
    return [i.m_Uuid.text for i in items]


class Joined(unittest.TestCase):
    def setUp(self):
        # A: a1 -t1- t2 -a2 joined through two tracks, a3 apart.  B: b1 touches b2.  C: one pad.  D: two pads apart.
        self.a1, self.a2, self.a3 = pad("a1", "A"), pad("a2", "A"), pad("a3", "A")
        self.b1, self.b2 = pad("b1", "B"), pad("b2", "B", (pcbnew.F_Cu, pcbnew.B_Cu))
        self.c1, self.d1, self.d2 = pad("c1", "C"), pad("d1", "D"), pad("d2", "D")
        self.free, self.masked = pad("free", ""), pad("masked", "D", (pcbnew.F_Mask,))
        t1, t2, via, stub = track("t1"), track("t2"), Item("v1", pcbnew.PCB_VIA_T), track("stub")
        self.connectivity = Connectivity((self.a1, t1), (t1, via), (via, t2), (t2, self.a2), (self.a3, stub),
                                         (self.b1, self.b2))
        self.board = Board([self.a1, self.a2, self.a3, self.b1, self.b2, self.c1, self.d1, self.d2, self.free,
                            self.masked], self.connectivity)

    def test_joined_pads_follows_tracks_and_vias(self):
        self.assertEqual(names(k.joined_pads(self.connectivity, self.a1)[0]), ["a1", "a2"])
        self.assertEqual(names(k.joined_pads(self.connectivity, self.a2)[0]), ["a2", "a1"])

    def test_a_pad_alone_or_with_a_loose_track(self):
        self.assertEqual(names(k.joined_pads(self.connectivity, self.a3)[0]), ["a3"])
        self.assertEqual(names(k.joined_pads(self.connectivity, self.c1)[0]), ["c1"])

    def test_pads_that_touch(self):
        self.assertEqual(names(k.joined_pads(self.connectivity, self.b1)[0]), ["b1", "b2"])

    def test_a_loop_of_copper_ends(self):
        t1, t2 = track("t1"), track("t2")
        ring = Connectivity((self.a1, t1), (t1, t2), (t2, self.a1), (t1, self.a2), (t2, self.a2))
        self.assertEqual(sorted(names(k.joined_pads(ring, self.a1)[0])), ["a1", "a2"])

    def test_pad_groups(self):
        groups = {net: [names(g) for g in gs] for net, gs in k.pad_groups(self.board).items()}
        # no group for the pad without a net, nor for the pad that is on no copper layer
        self.assertEqual(groups,
                         {"A": [["a1", "a2"], ["a3"]], "B": [["b1", "b2"]], "C": [["c1"]], "D": [["d1"], ["d2"]]})

    def test_pad_groups_of_some_nets(self):
        self.assertEqual(set(k.pad_groups(self.board, {"B", "D"})), {"B", "D"})

    def test_unjoined_nets(self):
        self.assertEqual(k.unjoined_nets(self.board), ["A", "D"])
        self.assertEqual(k.unjoined_nets(self.board, {"A", "B", "C"}), ["A"])
        self.assertEqual(k.unjoined_nets(self.board, set()), [])

    def test_unjoined_nets_after_copper_is_added(self):
        t = track("new")
        self.board.connectivity = Connectivity((self.a1, self.a2), (self.a2, t), (t, self.a3), (self.b1, self.b2))
        self.assertEqual(k.unjoined_nets(self.board), ["D"])


class WriteBack(unittest.TestCase):
    def setUp(self):
        self.board = Board(nets=["A", "B"])
        self.solution = dict(status="routed", nets=[
            dict(key="A", net="A", length_mm=3.0, vias=[[2.0, 1.0]],
                 tracks=[dict(layer="F.Cu", points=[[0.0, 0.0], [1.0, 1.0], [2.0, 1.0]]),
                         dict(layer="B.Cu", points=[[2.0, 1.0], [5.0, 1.0]])]),
            dict(key="B", net="B", length_mm=1.0, vias=[],
                 tracks=[dict(layer="B.Cu", points=[[0.0, 4.0], [1.5, 4.0]])]),
            dict(key="C", net="B", length_mm=0.0, vias=[], tracks=[]),
        ])
        self.sizes = {"A": (0.25, [0.6, 0.3]), "B": (0.15, [0.5, 0.3]), "C": (0.15, [0.5, 0.3])}

    def test_counts(self):
        self.assertEqual(k.write_back(self.board, self.solution, self.sizes), (4, 1))
        self.assertEqual(len(self.board.added), 5)

    def test_tracks_are_one_segment_per_pair_of_points(self):
        k.write_back(self.board, self.solution, self.sizes)
        tracks = [i for i in self.board.added if isinstance(i, support.PCB_TRACK)]
        self.assertEqual([(t.start, t.end, t.layer, t.width, t.net.name) for t in tracks], [
            ((0, 0), (1000000, 1000000), pcbnew.F_Cu, 250000, "A"),
            ((1000000, 1000000), (2000000, 1000000), pcbnew.F_Cu, 250000, "A"),
            ((2000000, 1000000), (5000000, 1000000), pcbnew.B_Cu, 250000, "A"),
            ((0, 4000000), (1500000, 4000000), pcbnew.B_Cu, 150000, "B"),
        ])

    def test_vias_are_through_vias_of_the_nets_size(self):
        k.write_back(self.board, self.solution, self.sizes)
        via, = [i for i in self.board.added if isinstance(i, support.PCB_VIA)]
        self.assertEqual((via.at, via.width, via.drill, via.kind, via.pair, via.net.name),
                         ((2000000, 1000000), 600000, 300000, "through", (pcbnew.F_Cu, pcbnew.B_Cu), "A"))

    def test_older_kicad_via_width_without_a_layer(self):
        class OldVia(support.PCB_VIA):
            def SetWidth(self, width): self.width = width

        pcbnew.PCB_VIA = OldVia
        self.addCleanup(setattr, pcbnew, "PCB_VIA", support.PCB_VIA)
        k.write_back(self.board, self.solution, self.sizes)
        self.assertEqual([i.width for i in self.board.added if isinstance(i, OldVia)], [600000])

    def test_nothing_laid(self):
        self.assertEqual(k.write_back(self.board, dict(status="unroutable", nets=[]), {}), (0, 0))
        self.assertEqual(self.board.added, [])


class BoardAdapter(unittest.TestCase):
    """KicadBoard rebuilds the connectivity after writing, before the board is looked at again."""

    def test_connectivity_is_rebuilt_once_after_a_write(self):
        board = Board([pad("a1", "A"), pad("a2", "A")], nets=["A"])
        board.built = 0
        board.BuildConnectivity = lambda: setattr(board, "built", board.built + 1)
        adapter = k.KicadBoard(board, frozenset(), support.ask, pours=False)
        self.assertEqual(adapter.unjoined(), ["A"])
        self.assertEqual(board.built, 0)  # as loaded, it is up to date
        adapter.write(dict(nets=[dict(key="A", net="A", vias=[],
                                      tracks=[dict(layer="F.Cu", points=[[0, 0], [1, 0]])])]), {"A": (0.2, [0.6, 0.3])})
        self.assertEqual(board.built, 0)
        adapter.unjoined({"A"})
        adapter.unjoined({"A"})
        self.assertEqual(board.built, 1)


if __name__ == "__main__":
    unittest.main()
