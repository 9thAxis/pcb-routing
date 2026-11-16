"""What the tests of tools/quality share: loading its modules, and hand-made dumps."""
import importlib, math, sys, unittest
from pathlib import Path

QUALITY = Path(__file__).resolve().parent.parent / "quality"


def load(name):
    """A module of tools/quality."""
    if str(QUALITY) not in sys.path:
        sys.path.insert(0, str(QUALITY))
    return importlib.import_module(name)


def scoring(name="measures"):
    """A module that needs shapely and numpy; the tests of the calling module are skipped without them."""
    try:
        import numpy, shapely  # noqa: F401
    except ImportError:
        raise unittest.SkipTest("shapely and numpy are not installed")
    return load(name)


def rectangle(x, y, w=1.0, h=1.0):
    return [[x - w / 2, y - h / 2], [x + w / 2, y - h / 2], [x + w / 2, y + h / 2], [x - w / 2, y + h / 2]]


def pad(net, x, y, ref="", num="1", layers=("F.Cu",), w=1.0, h=1.0):
    """A pad of w by h mm; on both layers it is a through-hole pad."""
    return dict(net=net, at=[x, y], shapes={l: [rectangle(x, y, w, h)] for l in layers}, ref=ref, num=num)


def track(net, a, b, w=0.2, layer="F.Cu", **more):
    return dict(net=net, a=list(a), b=list(b), w=w, layer=layer, len=math.dist(a, b), **more)


def via(net, x, y, d=0.6):
    return dict(net=net, at=[x, y], d=d)


def board(pads=(), tracks=(), vias=(), zone_nets=(), **more):
    """A two-layer dump."""
    return dict(layers=["F.Cu", "B.Cu"], pads=list(pads), tracks=list(tracks), vias=list(vias),
                zone_nets=list(zone_nets), **more)
