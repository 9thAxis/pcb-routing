"""How a track leaves a pad. A pad that is longer than it is wide is left through an end, along its length:
the ground beside its long sides costs its own net more, so that its track runs out to an end before it turns.
(From the middle of such a pad a track at 45 degrees crosses a long side, and that is how the search would
leave it, for it is no longer.) Plain geometry of pads; nothing here touches KiCad."""

import math

from .lengths import r6

LONG = 1.5  # a pad at least this many times as long as wide has ends
MOST_DEAR = 100.0  # the greatest factor the router takes for ground that costs more


def long_pad(outline):
    """The smallest rectangle round a pad's outline (a ring of points), if it is LONG: (its middle, the
    direction of its length as a unit vector, half its length, half its width). None for any other pad."""
    best = None
    for k, (x0, y0) in enumerate(outline):
        x1, y1 = outline[(k + 1) % len(outline)]
        side = math.hypot(x1 - x0, y1 - y0)
        if side < 1e-9:
            continue
        ux, uy = (x1 - x0) / side, (y1 - y0) / side
        along = [x * ux + y * uy for x, y in outline]
        across = [y * ux - x * uy for x, y in outline]
        spans = (max(along) - min(along), max(across) - min(across))
        if best is None or spans[0] * spans[1] < best[0] - 1e-12:
            best = (spans[0] * spans[1], (ux, uy), along, across, spans)
    if best is None:
        return None
    _, (ux, uy), along, across, (length, width) = best
    a, c = (max(along) + min(along)) / 2, (max(across) + min(across)) / 2
    middle = [a * ux - c * uy, a * uy + c * ux]
    if width > length:
        (ux, uy), length, width = (-uy, ux), width, length
    if width < 1e-9 or length < LONG * width:
        return None
    return middle, (ux, uy), length / 2, width / 2


def beside(outline, depth):
    """The ground beside the long sides of a pad, `depth` deep, as two rings; none for a pad that is not long."""
    found = long_pad(outline)
    if found is None:
        return []
    (mx, my), (ux, uy), half, narrow = found
    at = lambda u, v: [r6(mx + u * ux - v * uy), r6(my + u * uy + v * ux)]
    return [[at(-half, side * narrow), at(half, side * narrow), at(half, side * (narrow + depth)),
             at(-half, side * (narrow + depth))] for side in (1, -1)]


def exits(pads, nets, layers, cost, step):
    """The ground that costs the pads' own nets more, as the router's `dear`: beside the long sides of every
    long pad of the nets named, on the routing layers it is on. A track that leaves through a long side pays
    for `cost` mm of length more, about. Nothing with a cost of 0."""
    if cost <= 0:
        return []
    depth = max(0.2, 3 * step)
    factor = r6(min(1 + cost / depth, MOST_DEAR))
    dear = []
    for pad in pads:
        for layer in (l for l in pad.layers if l in layers and pad.net in nets):
            for rings in pad.shapes.get(layer, []):
                strips = beside(rings[0], depth)
                dear += [dict(rings=[ring], layers=[layer], factor=factor, only=pad.net) for ring in strips]
    return dear
