"""What the tests share: a stand-in for KiCad's `pcbnew` module, so that the bridge loads without KiCad, and a
few hand-made pictures."""
import sys, types
from pathlib import Path

TOOLS = Path(__file__).resolve().parent.parent
FAKE_ENGINE = Path(__file__).resolve().parent / "fake_engine.py"


class VECTOR2I:
    def __init__(self, x, y):
        self.x, self.y = x, y


class _Item:
    """A track or via as the bridge fills it in."""

    def __init__(self, board):
        self.board, self.width = board, None

    def SetStart(self, p): self.start = (p.x, p.y)
    def SetEnd(self, p): self.end = (p.x, p.y)
    def SetPosition(self, p): self.at = (p.x, p.y)
    def SetLayer(self, layer): self.layer = layer
    def SetNet(self, net): self.net = net
    def SetViaType(self, kind): self.kind = kind
    def SetDrill(self, drill): self.drill = drill
    def SetLayerPair(self, top, bottom): self.pair = (top, bottom)


class PCB_TRACK(_Item):
    def SetWidth(self, width): self.width = width


class PCB_VIA(_Item):
    def SetWidth(self, layer, width): self.width = width  # as KiCad 9 and later: a width per layer


def stub_pcbnew():
    """Put a stand-in `pcbnew` in sys.modules (once) and return it."""
    stub = sys.modules.get("pcbnew")
    if getattr(stub, "is_stub", False):
        return stub
    stub = types.ModuleType("pcbnew")
    stub.is_stub = True
    stub.PCB_PAD_T, stub.PCB_TRACE_T, stub.PCB_VIA_T, stub.PCB_ARC_T = 1, 2, 3, 4
    stub.F_Cu, stub.B_Cu, stub.F_Mask, stub.B_Mask = 0, 2, 1, 3
    stub.VIATYPE_THROUGH = "through"
    stub.ToMM = lambda v: v / 1e6
    stub.FromMM = lambda v: int(round(v * 1e6))
    stub.VECTOR2I, stub.PCB_TRACK, stub.PCB_VIA = VECTOR2I, PCB_TRACK, PCB_VIA
    sys.modules["pcbnew"] = stub
    return stub


def bridge():
    """The bridge module, loaded against the stand-in."""
    stub_pcbnew()
    if str(TOOLS) not in sys.path:
        sys.path.insert(0, str(TOOLS))
    import kicad_route
    return kicad_route


def square(k, x, y, half=0.5):
    return [[[k._r(x - half), k._r(y - half)], [k._r(x + half), k._r(y - half)], [k._r(x + half), k._r(y + half)], [k._r(x - half), k._r(y + half)]]]


def land(k, net, x, y, layer="F.Cu", half=0.5, **more):
    """A surface-mount pad: a square on one layer."""
    return k.Pad(net=net, at=[x, y], layers=[layer], shapes={layer: [square(k, x, y, half)]}, smallest=2 * half, **more)


def pin(k, net, x, y, drill=1.0, half=0.9, **more):
    """A through-hole pad on both layers."""
    shapes = {l: [square(k, x, y, half)] for l in ("F.Cu", "B.Cu")}
    return k.Pad(net=net, at=[x, y], layers=["F.Cu", "B.Cu"], shapes=shapes, hole_at=[x, y], drill=drill, smallest=2 * half, **more)


def picture(k, pads, groups=None, **more):
    """A two-layer board of 20 mm by 10 mm with these pads; every pad a group of its own unless told otherwise."""
    if groups is None:
        groups = {}
        for i, pad in enumerate(pads):
            if pad.net:
                groups.setdefault(pad.net, []).append([i])
    more.setdefault("classes", {p.net: k.NetClass(0.2, 0.25, 0.6, 0.3) for p in pads if p.net})
    more.setdefault("minima", k.Minima(clearance=0.15, hole_clearance=0.25, edge_clearance=0.3, track_width=0.15, via_size=0.5,
                                       through_drill=0.3, annular_width=0.1, hole_to_hole=0.25))
    return k.Picture(layers=["F.Cu", "B.Cu"], box=[0.0, 0.0, 20.0, 10.0], pads=pads, groups=groups, **more)
