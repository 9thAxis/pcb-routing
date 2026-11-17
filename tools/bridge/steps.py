"""One step of a board's work (see decompose.py): which layers it may be tried on, and what a try of it came
to. The trying itself is the command line's (tools/kicad_step.py): it needs the board."""

import math

PLANE = 0.5  # an inner layer is a plane if zones cover this much of the board on it
VIA_MM = 10.0  # what a via counts for, in mm of track, when tries are weighed
COUPLED = 4.0  # track widths within which the two nets of a pair run side by side (as tools/quality has it)
SAMPLE = 0.1  # mm between the points a track is measured at


def _area(ring):
    return abs(sum(p[0] * q[1] - q[0] * p[1] for p, q in zip(ring, ring[1:] + ring[:1]))) / 2


def planes(picture):
    """The inner layers that are planes: zones cover more than PLANE of the board there. They carry no routes
    (an outer layer with a zone still does: on two layers there is nowhere else)."""
    x0, y0, x1, y1 = picture.box
    board = max((x1 - x0) * (y1 - y0), 1e-9)
    covered = {}
    for pour in picture.fills:
        outline, holes = pour.rings[0], pour.rings[1:]
        covered[pour.layer] = covered.get(pour.layer, 0.0) + _area(outline) - sum(_area(hole) for hole in holes)
    return [layer for layer in picture.layers[1:-1] if covered.get(layer, 0.0) > PLANE * board]


def habits(picture):
    """How the tracks already on each layer run: {layer: (mm of track, the share of it that runs across the
    board, the share that runs up and down)}; what is neither runs at an angle. Where a board has an inner
    layer for each direction, a designer has said so with the tracks."""
    found = {}
    for track in picture.tracks:
        dx, dy = abs(track.b[0] - track.a[0]), abs(track.b[1] - track.a[1])
        length = math.hypot(dx, dy)
        total, across, up = found.get(track.layer, (0.0, 0.0, 0.0))
        found[track.layer] = (total + length, across + (length if dy < 0.2 * dx else 0.0),
                              up + (length if dx < 0.2 * dy else 0.0))
    return {layer: (round(total, 1), round(across / total, 2), round(up / total, 2))
            for layer, (total, across, up) in found.items() if total > 0}


def layer_options(picture, step):
    """The sets of layers a step is tried on, each with a name: the layers its pads are on alone; those with
    one of the other signal layers; and every signal layer. The planes are in none."""
    signal = [layer for layer in picture.layers if layer not in planes(picture)]
    own = [layer for layer in signal if layer in step.layers]
    found = [("its pads' own", own)] if own else []
    found += [(f"with {layer}", [l for l in signal if l in own or l == layer]) for layer in signal if layer not in own]
    found.append(("every signal layer", signal))
    seen, options = [], []
    for name, layers in found:
        if layers and layers not in seen:
            seen.append(layers)
            options.append((name, layers))
    return options


def _points(track):
    length = math.dist(track.a, track.b)
    count = max(1, round(length / SAMPLE))
    return [[track.a[k] + (track.b[k] - track.a[k]) * (i + 0.5) / count for k in (0, 1)] for i in range(count)], length / count


def _far(p, track):
    a, b = track.a, track.b
    along = [b[0] - a[0], b[1] - a[1]]
    length = along[0] ** 2 + along[1] ** 2
    t = 0.0 if length < 1e-18 else min(1.0, max(0.0, ((p[0] - a[0]) * along[0] + (p[1] - a[1]) * along[1]) / length))
    return math.dist(p, [a[0] + t * along[0], a[1] + t * along[1]])


def figures(before, after, step, pairs):
    """What a try of a step laid, from the board's picture before and after: per net its new track in mm and
    its new vias; in all, the track, the vias, the nets still not joined, how many times the least length the
    track is (see Step.span), and for each of `pairs` (two nets of the step) the share of the first net's new
    track that runs within COUPLED widths of the second's on the same layer."""
    old = before.copper_keys()
    tracks = [t for t in after.tracks if t.net in step.nets and (t.net, t.layer, tuple(t.a), tuple(t.b)) not in old]
    vias = [v for v in after.vias if v.net in step.nets and (v.net, None, tuple(v.at), tuple(v.at)) not in old]
    length = lambda net: sum(math.dist(t.a, t.b) for t in tracks if t.net == net)
    nets = {net: dict(mm=round(length(net), 2), vias=sum(1 for v in vias if v.net == net)) for net in step.nets}
    coupled = {}
    for a, b in pairs:
        mine, theirs = [t for t in tracks if t.net == a], [t for t in tracks if t.net == b]
        near = 0.0
        for track in mine:
            beside = [other for other in theirs if other.layer == track.layer]
            points, each = _points(track)
            near += each * sum(1 for p in points if any(_far(p, other) <= COUPLED * track.width for other in beside))
        coupled[f"{a} = {b}"] = round(near / max(length(a), 1e-9), 3)
    total = sum(n["mm"] for n in nets.values())
    return dict(nets=nets, mm=round(total, 1), vias=len(vias), left=sorted(after.unjoined(set(step.nets))),
                detour=round(total / step.span, 2) if step.span > 0 else None, coupled=coupled,
                layers=[layer for layer in after.layers if any(t.layer == layer for t in tracks)])


def weight(found):
    """What a try is weighed by, least first: the nets it left unjoined; then its track in mm, with VIA_MM
    for each via and, for a pair, its track that does not run beside the other net's counted once more."""
    apart = sum((1 - share) * found["nets"][pair.split(" = ")[0]]["mm"] * 2 for pair, share in found["coupled"].items())
    return (len(found["left"]), round(found["mm"] + VIA_MM * found["vias"] + apart, 1))
