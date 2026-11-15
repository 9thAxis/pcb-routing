#!/usr/bin/env python3
"""Route the unconnected nets of a KiCad board with pcb-routing.

    kicad_route.py BOARD.kicad_pcb -o ROUTED.kicad_pcb [--nets A,B] [--step auto|MM] [--variants N]
                   [--layers F.Cu,B.Cu] [--problem FILE] [--solution FILE] [--engine PATH]

Run it with the Python that ships with KiCad (it needs the `pcbnew` module), for example on macOS:

    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 kicad_route.py ...

What it does:

1. Reads the board: pads, tracks, vias, drilled holes, drawings on copper, the outline and the rule areas become
   the fixed picture; every net with pads that are not yet joined becomes one entry, with one terminal per group
   of joined pads. Widths, via sizes and clearances come from each net's class and the board's minima.
2. Runs the `pcb-routing` program on that problem. Nets it could not join are tried once more against the board
   as it then stands, with a track narrower than the net's narrowest pad. The engine takes fewer than 250 nets
   in one run: a board with more is routed in batches of at most 200, each against the board as the batch
   before left it.
3. Adds the tracks and vias found to the board, fills the zones again, and saves it under the new name.

Run KiCad's design rule check on the result before using it.

What it does not do:

- It routes on a grid, with tracks at multiples of 45 degrees and through vias only (no blind, buried or micro
  vias, no arcs).
- By default it does not follow zones or planes when deciding which pads are already joined: only tracks, vias
  and pads that touch. A net meant to be joined by a plane is then routed as tracks, unless it is left out by
  naming the others with --nets. With --pours (not yet the default: it leaves some boards incomplete that are
  completed without) the zones are filled first and pads that a filled zone joins count as joined: a net with
  a zone gets tracks only from the pads its zone does not reach, each to the nearest point of the zone; pads
  that the new tracks part from their zone are joined again.
- No differential pairs and no length tuning through this bridge yet (the engine can lay pairs; the bridge does
  not ask for them).

Exit status: 0 every net joined, 1 some nets left unjoined, 2 the engine failed (what it had laid before that
is saved all the same).
"""
import argparse, collections, itertools, json, math, os, re, subprocess, sys, tempfile
from dataclasses import dataclass, field, replace
from pathlib import Path

try:
    import pcbnew
except ImportError:  # the rules and the problem are worked out without KiCad; only reading and writing a board need it
    pcbnew = None

# The engine refuses a problem of this many entries or more; a board with more is routed in batches.
ENGINE_NETS = 250
ARC_ERROR = 0.002  # mm: how far the corners of a polygon drawn for a round shape lie outside it
BATCH_NETS = 200


# --------------------------------------------------------------------------------------------------------------
# The picture: a board as plain data. Lengths in mm, points [x, y], layers by name.
# A polygon is a list of rings (the outline first, then its holes); a ring is a list of points.

@dataclass
class NetClass:
    """What a net's class asks for."""
    clearance: float
    track_width: float
    via_size: float
    via_drill: float


@dataclass
class Minima:
    """The board's own limits (Board Setup > Constraints)."""
    clearance: float = 0.0
    hole_clearance: float = 0.0
    edge_clearance: float = 0.0
    track_width: float = 0.0
    via_size: float = 0.0
    through_drill: float = 0.0
    annular_width: float = 0.0
    hole_to_hole: float = 0.0


@dataclass
class Pad:
    net: object  # its net's name, None without one
    at: list  # where a track meets it
    layers: list  # the copper layers it is on
    shapes: dict  # layer -> its polygons there: on its copper layers, and where it has only a mask opening
    openings: dict = field(default_factory=dict)  # outer copper layer -> how far its mask opening reaches beyond its outline
    hole_at: object = None  # the centre of its drilled hole
    drill: object = None  # the hole's diameter (the longer side of a slot); None: no hole
    clearance: object = None  # its own clearance, where it or its footprint overrides the class
    smallest: object = None  # its smallest dimension on any copper layer
    part: object = None  # the reference of the part it belongs to


@dataclass
class Track:
    a: list
    b: list
    width: float
    net: object
    layer: str


@dataclass
class Via:
    at: list
    width: float
    drill: float
    net: object


@dataclass
class Hole:
    """A drilled hole, of a pad or of a via."""
    at: list
    drill: float
    net: object
    clearance: object = None  # the clearance override of the pad it belongs to


@dataclass
class Polygon:
    """Copper that is neither pad, straight track nor via: an arc of track (with its net), a drawing or text."""
    rings: list
    layer: str
    net: object = None


@dataclass
class Area:
    """A rule area that keeps tracks or vias out."""
    rings: list
    layers: list
    tracks: bool
    vias: bool


@dataclass
class Pour:
    """One piece of a filled zone: copper of a net that joins what touches it."""
    net: object
    layer: str
    rings: list  # its outline, then its holes
    lone: bool = False  # joined to the rest of its net by one pad alone (its spokes then lead nowhere, for KiCad)


@dataclass
class Picture:
    """Everything read from a board."""
    layers: list  # every copper layer, top to bottom
    box: list  # [x0, y0, x1, y1] of the board
    pads: list = field(default_factory=list)
    tracks: list = field(default_factory=list)
    vias: list = field(default_factory=list)
    polygons: list = field(default_factory=list)
    outline: object = None  # the polygons of the board's outline; None if it has none
    areas: list = field(default_factory=list)
    classes: dict = field(default_factory=dict)  # net -> NetClass
    minima: Minima = field(default_factory=Minima)
    ignored: frozenset = frozenset()  # the design rule checks the project sets to 'ignore'
    groups: dict = field(default_factory=dict)  # net -> its groups of pads already joined, as indices into pads
    pours: dict = field(default_factory=dict)  # net -> {index of a group: the pours that join it}

    @property
    def holes(self):
        """Every drilled hole, whatever it belongs to."""
        return ([Hole(p.hole_at, p.drill, p.net, p.clearance) for p in self.pads if p.drill] +
                [Hole(v.at, v.drill, v.net) for v in self.vias if v.drill])

    def unjoined(self, only=None):
        """The nets whose pads are not all joined, by name."""
        return sorted(n for n, groups in self.groups.items() if len(groups) > 1 and (only is None or n in only))


# --------------------------------------------------------------------------------------------------------------
# The rules: what the board asks of the router, each stated once. Nothing here touches KiCad.

@dataclass
class Settings:
    """What the user chose for a run."""
    step: object = 0.05  # grid step in mm; None: chosen from the board's rules
    coarse_first: bool = False  # a first pass at twice the step, for speed
    slack: float = 0.002
    via_penalty: float = 10.0
    layers: object = None  # names of the layers that may carry routes; None: every copper layer
    pours: bool = False  # pads that a filled zone joins count as joined, and the rest of the net is taken to the zone
    decoupling: bool = True  # a capacitor beside a pin of an IC on its net is joined to that pin itself
    bus_pull: float = 0.2  # how strongly nets that leave and arrive side by side are drawn to run together; 0: not


def ignored_checks(board_path):
    """The design rule checks the board's project sets to 'ignore' (KiCad keeps them in the .kicad_pro)."""
    try:
        project = json.loads(Path(board_path).with_suffix(".kicad_pro").read_text())
        severities = project["board"]["design_settings"]["rule_severities"]
        return frozenset(name for name, level in severities.items() if level == "ignore")
    except Exception:
        return frozenset()


class Rules:
    """How a board's rules become the router's distances."""

    WINDOW_MARGIN = 1.0  # the window reaches this far beyond the board
    MASK_EDGE = 0.01  # copper of another net stays this far outside a solder mask opening
    NARROW_BY = 0.1  # the second try's track is this much narrower than the net's narrowest pad
    NARROWEST = 0.1  # and never narrower than this
    DEFAULT_VIA_DRILL = 0.3  # for the hole spacing when no net is to be routed
    STEPS = (0.1, 0.05, 0.025)  # the steps chosen from
    CELLS_PER_PITCH = 6  # at least this many cells from one track to the next

    def __init__(self, minima, classes, ignored=frozenset(), settings=None):
        self.minima, self.classes, self.ignored = minima, classes, frozenset(ignored)
        self.settings = settings or Settings()
        # the smallest clearance any net keeps from other copper: what every track keeps from a pad or hole anyway
        self.least_clearance = min((self.clearance(net) for net in classes), default=self.kept(minima.clearance))

    @classmethod
    def of(cls, picture, settings=None):
        return cls(picture.minima, picture.classes, picture.ignored, settings)

    def kept(self, distance):
        """A distance as the router is to keep it: with the slack for the grid. The one place it is added."""
        return distance + self.settings.slack

    def ignores(self, check):
        return check in self.ignored

    # -- between nets

    def clearance(self, net):
        """What a net keeps from copper of other nets: its class's clearance, at least the board's."""
        return self.kept(max(self.classes[net].clearance, self.minima.clearance))

    def stricter(self, net):
        """The nets whose class asks more than this net's: it keeps their clearance from their copper."""
        own = self.clearance(net)
        return {other: self.clearance(other) for other in self.classes if other != net and self.clearance(other) > own}

    def new_clearance(self, nets):
        """Between copper of two nets laid in the same run: the most any of them asks."""
        return max((self.clearance(net) for net in nets), default=self.kept(self.minima.clearance))

    # -- what stands in the way

    def pad_growth(self, pad, layer):
        """How much a pad is grown on a layer, beyond the clearance every net keeps from it anyway; None where
        it puts nothing in the way there.

        A pad asks more than that of other nets where it has a clearance of its own, and where its solder mask
        opening reaches further (copper of another net there would lie bare), unless the project ignores that check.
        """
        has_copper = layer in pad.layers
        opening = None if self.ignores("solder_mask_bridge") else pad.openings.get(layer)
        if not has_copper and opening is None:
            return None
        grow = 0.0
        if has_copper and pad.clearance is not None:
            grow = max(grow, self.kept(pad.clearance) - self.least_clearance)
        if opening is not None:
            grow = max(grow, opening + self.MASK_EDGE - self.least_clearance)
        return grow

    def hole_growth(self, hole):
        """How much a drilled hole is grown, likewise: copper keeps the board's hole clearance (or the clearance
        of the hole's own pad, if that is more) from its edge, unless the project ignores that check."""
        asked = 0.0 if self.ignores("hole_clearance") else self.minima.hole_clearance
        return max(self.kept(max(asked, hole.clearance or 0.0)) - self.least_clearance, 0.0)

    def hole_spacing(self, drill, other):
        """Centre to centre distance of two holes of these diameters."""
        return self.kept((drill + other) / 2 + self.minima.hole_to_hole)

    def edge_margin(self):
        """What copper keeps from the board's edge and from rule areas (0 if the project ignores the check)."""
        asked = 0.0 if self.ignores("copper_edge_clearance") else self.minima.edge_clearance
        return self.kept(asked) if asked > 0 else 0.0

    # -- what is laid

    def track_width(self, net):
        return max(self.classes[net].track_width, self.minima.track_width)

    def narrow_width(self, net, smallest_pad):
        """The second try's width for a net that could not be laid at its class width: narrower than its
        narrowest pad (a track wider than a fine-pitch pad cannot enter it; a little under the pad's width, as
        pads on an inch pitch do not sit on the grid and the nearest cell to a pad's middle is then a little
        nearer one neighbour), never under the board's minimum."""
        width = self.track_width(net)
        if smallest_pad is not None:
            width = min(width, smallest_pad - self.NARROW_BY)
        return max(width, self.minima.track_width, self.NARROWEST)

    def via(self, net):
        """A net's via: [pad diameter, drill], with at least the board's smallest annular ring."""
        drill = max(self.classes[net].via_drill, self.minima.through_drill)
        size = max(self.classes[net].via_size, self.minima.via_size, drill + 2 * self.minima.annular_width)
        return [size, drill]

    # -- how fine

    def step(self, nets):
        """The grid step for these nets: the user's, else the coarsest that still has several cells from one
        track to the next (tracks can only be a whole number of cells apart, so a coarse grid wastes room)."""
        if self.settings.step is not None:
            return self.settings.step
        pitch = min((self.track_width(net) + self.clearance(net) for net in nets), default=0.0)
        return next((s for s in self.STEPS if s * self.CELLS_PER_PITCH <= pitch + 1e-9), self.STEPS[-1])

    def tries(self, nets):
        """The tries the nets get, in turn, each for what the one before left unjoined: (grid step, narrower or not).

        Once as the rules say, once narrower. If the user asked for it, a pass on a grid twice as coarse comes
        first: most nets of most boards are joined there at a quarter of the work, at the price of a little
        more track and a few more vias, and of room on a crowded board.
        """
        step = self.step(nets)
        return ([(2 * step, False)] if self.settings.coarse_first else []) + [(step, False), (step, True)]

    def states(self, box, layers):
        """The size of the router's grid at the present step: cells times routing layers."""
        window, step = self.window(box), self.settings.step
        across = lambda a, b: round((b - a) / step) + 1
        return across(window[0], window[2]) * across(window[1], window[3]) * len(self.routing_layers(layers))

    # -- where

    def routing_layers(self, layers):
        wanted = self.settings.layers
        return [l for l in layers if wanted is None or l in wanted]

    def window(self, box):
        """The area routed in: a little beyond the board, with the cells on multiples of the step so that
        parts placed on a grid are met."""
        step, margin = self.settings.step, self.WINDOW_MARGIN
        low = lambda v: round(math.floor((v - margin) / step) * step, 6)
        high = lambda v: round(math.ceil((v + margin) / step) * step, 6)
        return [low(box[0]), low(box[1]), high(box[2]), high(box[3])]


# --------------------------------------------------------------------------------------------------------------
# The problem: a picture and the rules as the router's JSON (src/model.rs). Nothing here touches KiCad.

def _r(v):
    return round(v, 6)


def _disc(centre, radius, sides=16):
    """A polygon that covers a disc."""
    out = radius / math.cos(math.pi / sides)
    return [[_r(centre[0] + out * math.cos(2 * math.pi * k / sides)), _r(centre[1] + out * math.sin(2 * math.pi * k / sides))]
            for k in range(sides)]


def _grown(rings, grow, net, layers):
    """A polygon and what growing it by a distance adds: a round-ended stroke of that half-width along each edge."""
    out = [dict(t="poly", rings=rings, net=net, layers=layers)]
    if grow > 0:
        for ring in rings:
            for a, b in zip(ring, ring[1:] + ring[:1]):
                out.append(dict(t="seg", a=a, b=b, w=_r(2 * grow), net=net, layers=layers))
    return out


def buses(pads, nets=None):
    """Which nets run as buses: net name -> (name of its bus, its lane across it), among `nets` if given.

    Two nets are neighbours on a part if they have pads there no further apart than 1.3 times the distance
    from one of them to its nearest pad, whatever that pad is of. A bus is three or more nets joined by being
    neighbours on the same two parts; a net that is in two such groups joins them into one bus.
    """
    parts = {}
    for pad in pads:
        if pad.part:
            parts.setdefault(pad.part, []).append(pad)
    wanted = lambda net: net is not None and (nets is None or net in nets)
    beside = {}  # (net, net) -> the parts they are neighbours on
    for part, own in sorted(parts.items()):
        for a in own:
            others = sorted(((math.dist(a.at, b.at), b.net or "") for b in own if b is not a and math.dist(a.at, b.at) > 1e-6))
            for distance, net in others:
                if distance > 1.3 * others[0][0]:
                    break
                if wanted(a.net) and wanted(net or None) and net != a.net:
                    beside.setdefault(tuple(sorted((a.net, net))), set()).add(part)
    links = {}  # (part, part) -> the pairs of nets that are neighbours on both
    for pair, where in sorted(beside.items()):
        for two in itertools.combinations(sorted(where), 2):
            links.setdefault(two, []).append(pair)

    def find(leader, net):
        while leader.setdefault(net, net) != net:
            net = leader[net]
        return net

    bus = {}
    for two, pairs in sorted(links.items()):
        group = {}  # nets joined within these two parts
        for a, b in pairs:
            group[find(group, a)] = find(group, b)
        sizes = collections.Counter(find(group, net) for net in group)
        for a, b in pairs:
            if sizes[find(group, a)] >= 3:
                bus[find(bus, a)] = find(bus, b)
    names = {net: "bus " + find(bus, net) for net in bus}
    return {net: (name, lane) for name in sorted(set(names.values()))
            for net, lane in _lanes({net for net in names if names[net] == name}, parts).items()}


def _lanes(nets, parts):
    """The place of each net across its bus: its rank along the row of pads on the part that has most of the
    bus's nets (along the longer side of what those pads cover, then across). All 0, which is no order, unless
    that part has all of the bus's nets, each once, in one row or two: an array of pads has no order across it.
    """
    def on(part):
        return [pad for pad in parts[part] if pad.net in nets]
    part = max(sorted(parts), key=lambda part: len({pad.net for pad in on(part)}))
    pads = on(part)
    wide = max(p.at[0] for p in pads) - min(p.at[0] for p in pads) >= max(p.at[1] for p in pads) - min(p.at[1] for p in pads)
    along, across = (0, 1) if wide else (1, 0)
    rows = {round(p.at[across], 3) for p in pads}
    if len(pads) != len(nets) or {p.net for p in pads} != set(nets) or len(rows) > 2:
        return {net: 0 for net in nets}
    order = [pad.net for pad in sorted(pads, key=lambda p: (p.at[along], p.at[across]))]
    return {net: lane for lane, net in enumerate(order)}


def _area(rings):
    """The copper of a polygon given as its outline and holes, in mm2."""
    ring = lambda r: abs(sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(r, r[1:] + r[:1]))) / 2
    return ring(rings[0]) - sum(ring(r) for r in rings[1:]) if rings else 0.0


def _within(rings, p):
    """True if a point is on a polygon given as its outline and holes (even-odd)."""
    inside = False
    for ring in rings:
        for a, b in zip(ring, ring[1:] + ring[:1]):
            if (a[1] > p[1]) != (b[1] > p[1]) and p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]):
                inside = not inside
    return inside


def into_pour(rings, p, inset):
    """The point of a pour to take a track from `p` to: `p` itself if it is on the pour, else the nearest point
    of its edge moved `inset` further in. None if that is not on the pour (a sliver too thin to end a track on)."""
    if _within(rings, p):
        return list(p)
    best = None
    for ring in rings:
        for a, b in zip(ring, ring[1:] + ring[:1]):
            vx, vy = b[0] - a[0], b[1] - a[1]
            l2 = vx * vx + vy * vy
            t = max(0.0, min(1.0, ((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / l2)) if l2 else 0.0
            q = (a[0] + t * vx, a[1] + t * vy)
            d = math.dist(p, q)
            if best is None or d < best[0]:
                best = (d, q)
    if best is None or best[0] == 0:
        return None
    d, q = best
    q = [_r(q[0] + (q[0] - p[0]) / d * inset), _r(q[1] + (q[1] - p[1]) / d * inset)]
    return q if _within(rings, q) else None


def to_pours(pads, groups, pours, layers, inset):
    """How a net with pours is joined: [(start point, its layers, target point, its layers)], one for each
    group of pads that is not on the net's largest pour. The target is the nearest of that pour's group's copper
    to the group: a point of one of its pours, or one of its pads. The start is the group's pad nearest to it;
    or, for a group with a pour of its own, the point of that pour nearest the target: a piece of pour joined
    only through a pad's spokes would leave them counted as leading nowhere.

    `groups`: the net's groups of joined pads; `pours`: {index of a group: its pours}; `layers`: the routing
    layers. Empty if the net has no pour on a routing layer.
    """
    usable = {g: [pour for pour in own if pour.layer in layers] for g, own in pours.items()}
    area = {g: sum(_area(pour.rings) for pour in own) for g, own in usable.items() if own}
    if not area:
        return []
    main = max(sorted(area), key=area.get)
    links = []
    # A piece of pour of the main group that one pad alone joins to it (a through-hole pad, by the other side):
    # joined to the rest of the group's copper from a point of its own, as a stitching via would.
    anchor = max(usable[main], key=lambda pour: _area(pour.rings))
    for lone in (pour for pour in usable[main] if pour.lone and pour is not anchor):
        best = None
        for pour in (pour for pour in usable[main] if not pour.lone or pour is anchor):
            at = into_pour(pour.rings, lone.rings[0][0], inset)
            start = at and into_pour(lone.rings, at, inset)
            at = start and into_pour(pour.rings, start, inset)
            if at and (best is None or math.dist(start, at) < best[0]):
                best = (math.dist(start, at), start, [lone.layer], at, [pour.layer])
        if best:
            links.append(best[1:])
    for g, group in enumerate(groups):
        if g == main:
            continue
        best = None
        for i in group:
            on = [l for l in layers if l in pads[i].layers]
            if not on:
                continue
            here = pads[i].at
            targets = [(pads[j].at, [l for l in layers if l in pads[j].layers]) for j in groups[main]]
            targets += [(into_pour(pour.rings, here, inset), [pour.layer]) for pour in usable[main]]
            for at, target_on in targets:
                if at is not None and target_on and (best is None or math.dist(here, at) < best[0]):
                    best = (math.dist(here, at), here, on, at, target_on)
        if best is None:
            continue
        _, start, on, at, target_on = best
        own = [(into_pour(pour.rings, at, inset), [pour.layer]) for pour in usable.get(g, [])]
        own = [(math.dist(point, at), point, layer) for point, layer in own if point is not None]
        if own:
            _, start, on = min(own)
        links.append((start, on, at, target_on))
    return links


KEPT_SPOKES = 4  # thermal spokes of a pad that other nets keep clear of, where it has them
POUR_REPEATS = 3  # how often nets that new tracks have parted from their pour are joined again
DECOUPLING_REACH = 5.0  # mm from a capacitor's pad to the pin it is taken to stand by
DECOUPLING_VIA_PENALTY = 5.0  # times the usual: the detour accepted before the link to the pin changes layer


def decoupling(pads, groups):
    """The capacitors of a net that stand by a pin of an IC on it: [(group of the capacitor's pad, pad index of
    the pin)], for `groups`, the net's groups of joined pads as indices into `pads`.

    A capacitor is a part C.. of two pads whose pad on the net is joined to nothing yet, and whose two nets
    have three pads or more each (a supply and its return; not a capacitor in series with a signal). The pin
    is the nearest pad of a part U.. or IC.. on the net, in another group, within DECOUPLING_REACH.
    """
    by_part, size = collections.defaultdict(list), collections.Counter(pad.net for pad in pads if pad.net)
    for pad in pads:
        if pad.part:
            by_part[pad.part].append(pad)
    def is_cap(pad):
        both = by_part.get(pad.part, [])
        return bool(pad.part) and re.match(r"C\d", pad.part) and len(both) == 2 and all(size[p.net] >= 3 for p in both)
    is_pin = lambda pad: bool(pad.part) and re.match(r"(U|IC)\d", pad.part)
    links = []
    for g, group in enumerate(groups):
        if len(group) != 1 or not is_cap(pads[group[0]]):
            continue
        cap = pads[group[0]]
        pins = [(math.dist(cap.at, pads[i].at), i) for h, other in enumerate(groups) if h != g for i in other if is_pin(pads[i])]
        if pins and min(pins)[0] <= DECOUPLING_REACH:
            links.append((g, min(pins)[1]))
    return links


def build_problem(picture, rules, only=None, narrow=False, options=None, unlinked=frozenset()):
    """The router's problem for the unjoined nets of a picture (those named in `only`, if given).

    `narrow`: the second try, with narrower tracks and vias allowed in a net's own lands. `unlinked`: nets whose
    capacitors get no link of their own to the pin they stand by.
    """
    every = picture.layers
    layers = rules.routing_layers(every)
    window = rules.window(picture.box)
    copper, holes, regions = [], [], []

    for pad in picture.pads:
        land = len(pad.layers) == 1  # a solder land: on one copper layer
        for layer, polygons in pad.shapes.items():
            grow = rules.pad_growth(pad, layer)
            if grow is None:
                continue
            for rings in polygons:
                shapes = _grown(rings, grow, pad.net, [layer])
                shapes[0]["pad"] = land and layer in pad.layers
                copper.extend(shapes)
    for track in picture.tracks:
        copper.append(dict(t="seg", a=track.a, b=track.b, w=_r(track.width), net=track.net, layers=[track.layer]))
    for via in picture.vias:
        copper.append(dict(t="circle", c=via.at, r=_r(via.width / 2), net=via.net, layers=every))
    for polygon in picture.polygons:
        copper.append(dict(t="poly", rings=polygon.rings, net=polygon.net, layers=[polygon.layer]))
    # every drilled hole, of a pad or of a via alike: a hole, and a disc that copper of other nets keeps off
    drilled = picture.holes
    for hole in drilled:
        holes.append(dict(xy=hole.at, net=hole.net))
        copper.append(dict(t="circle", c=hole.at, r=_r(hole.drill / 2 + rules.hole_growth(hole)), net=hole.net, layers=every))

    if picture.outline is not None:
        frame = [[window[0] - 1, window[1] - 1], [window[2] + 1, window[1] - 1], [window[2] + 1, window[3] + 1], [window[0] - 1, window[3] + 1]]
        rings = [frame]
        for polygon in picture.outline:
            rings.extend(polygon)  # even-odd: closed outside the board and inside its cut-outs
        regions.append(dict(rings=rings))
    for area in picture.areas:
        regions.append(dict(rings=area.rings, layers=[l for l in area.layers if l in layers], tracks=area.tracks, vias=area.vias))

    smallest = {}
    for pad in picture.pads:
        if pad.net and pad.smallest is not None:
            smallest[pad.net] = min(pad.smallest, smallest.get(pad.net, pad.smallest))
    unjoined = picture.unjoined(only)
    bus = buses(picture.pads, set(unjoined)) if rules.settings.bus_pull > 0 else {}
    nets = []
    for place, name in enumerate(unjoined):
        # one terminal per group of pads that existing copper already joins
        on = lambda pad: [l for l in layers if l in pad.layers]
        groups = picture.groups[name]
        if not all(on(picture.pads[group[0]]) for group in groups):
            print(f"skipped {name}: a pad of it is on no routing layer", file=sys.stderr)
            continue
        terminal = lambda pad: dict(xy=pad.at, layers=on(pad))
        width = rules.narrow_width(name, smallest.get(name)) if narrow else rules.track_width(name)
        entry = lambda key, terminals, via_penalty: dict(
            key=key, net=name, width=_r(width), via=[_r(v) for v in rules.via(name)], layers=layers,
            clearance=dict(default=_r(rules.clearance(name)), nets={n: _r(c) for n, c in rules.stricter(name).items()}),
            terminals=terminals, via_penalty_mm=via_penalty)
        # A net with a pour: each group of pads that is not on it is taken to its nearest point, by an entry of
        # its own, so that nothing is laid where the pour already joins.
        poured = to_pours(picture.pads, groups, picture.pours.get(name, {}), layers, _r(width / 2 + 0.1)) if rules.settings.pours else []
        for number, (start, start_layers, at, target_layers) in enumerate(poured):
            nets.append(entry(f"{name} > {number + 1}", [dict(xy=start, layers=start_layers), dict(xy=at, layers=target_layers)],
                              rules.settings.via_penalty))
        if poured:
            continue
        # A capacitor beside a pin of an IC is joined to that pin, in an entry of its own that is slow to
        # change layer; the rest of the net is joined as it would be without the capacitor.
        links = decoupling(picture.pads, groups) if rules.settings.decoupling and name not in unlinked else []
        # (as many as the engine has room for beside this net's own entry and one for each net still to come)
        room = ENGINE_NETS - 1 - len(nets) - 1 - (len(unjoined) - place - 1)
        links = [(g, pin) for g, pin in links if on(picture.pads[pin])][:max(0, room)]
        taken = {g for g, _ in links}
        rest = [terminal(picture.pads[group[0]]) for g, group in enumerate(groups) if g not in taken]
        for g, pin in links:
            cap = picture.pads[groups[g][0]]
            key = f"{name} | {cap.part}"
            while key in picture.groups or any(n["key"] == key for n in nets):
                key += "'"  # (a net may be named just so)
            nets.append(entry(key, [terminal(cap), terminal(picture.pads[pin])],
                              rules.settings.via_penalty * DECOUPLING_VIA_PENALTY))
        if len(rest) < 2:
            continue
        nets.append(entry(name, rest, rules.settings.via_penalty))
        if name in bus:
            nets[-1]["bus"], nets[-1]["bus_lane"] = bus[name]

    # The engine keeps one distance between hole centres: enough for two holes of the widest new via's drill. A
    # wider hole asks more of a new via, and its own net's vias are not held off by its copper: the ground
    # round such a hole is closed to vias (a net may still change layer exactly on a hole of its own).
    via_drill = max((n["via"][1] for n in nets), default=rules.DEFAULT_VIA_DRILL)
    margin = rules.edge_margin()
    for hole in drilled:
        if nets and hole.drill > via_drill:
            # a region is closed to a via's edge, so the disc is smaller by the narrowest via's radius
            radius = max(rules.hole_spacing(hole.drill, n["via"][1]) - n["via"][0] / 2 for n in nets) - margin
            regions.append(dict(rings=[_disc(hole.at, max(radius, 0.001))], layers=[], tracks=False, vias=True))

    problem_rules = dict(hole_centre_mm=_r(rules.hole_spacing(via_drill, via_drill)), pad_drill_margin_mm=0.0,
                         region_margin_mm=_r(margin), new_clearance_mm=_r(rules.new_clearance(n["net"] for n in nets)))
    if narrow:
        problem_rules["via_in_pad"] = True  # a land enclosed by another pad can only be left through a via in it
    return dict(window=window, step=rules.settings.step, layers=layers, copper=copper, holes=holes, regions=regions,
                rules=problem_rules, nets=nets, options=dict(dict(bus_pull=rules.settings.bus_pull), **(options or {})))


# --------------------------------------------------------------------------------------------------------------
# The engine.

class EngineFailed(Exception):
    """The engine gave no solution; the message is what it said."""


def run_engine(problem, engine, threads=0, problem_path=None, solution_path=None):
    """Run the pcb-routing program on a problem and return its solution; its report goes to standard output.

    `engine` is the program (or a command as a list). The problem and the solution are kept under the given
    names, if any.
    """
    with tempfile.TemporaryDirectory() as tmp:
        problem_path = Path(problem_path or Path(tmp) / "problem.json")
        solution_path = Path(solution_path or Path(tmp) / "solution.json")
        problem_path.write_text(json.dumps(problem))
        solution_path.unlink(missing_ok=True)  # an old file must not pass for this run's answer
        command = ([engine] if isinstance(engine, (str, os.PathLike)) else list(engine)) + ["route", str(problem_path), "-o", str(solution_path)]
        if threads:
            command += ["--threads", str(threads)]
        try:
            run = subprocess.run(command, capture_output=True, text=True)
        except OSError as error:
            raise EngineFailed(f"cannot run {command[0]}: {error}")
        print(run.stdout, end="")
        if run.returncode == 2 or not solution_path.exists():
            raise EngineFailed(run.stderr.strip() or f"{command[0]} ended with status {run.returncode} and no solution")
        return json.loads(solution_path.read_text())


@dataclass
class Outcome:
    status: str = "routed"  # routed, or how the engine's last incomplete run ended (unroutable, conflicts)
    wanted: int = 0  # nets that were to be joined
    left: int = 0  # of them, those still not joined
    tracks: int = 0
    vias: int = 0
    failure: object = None  # what the engine said, if a run of it failed

    @property
    def laid(self):
        return self.tracks + self.vias > 0


def batches(names, most=ENGINE_NETS, size=BATCH_NETS):
    """The nets in lots the engine accepts: all at once if fewer than `most`, else evenly in lots of at most `size`."""
    if len(names) < most:
        return [list(names)] if names else []
    count = math.ceil(len(names) / size)
    each = math.ceil(len(names) / count)
    return [list(names[k:k + each]) for k in range(0, len(names), each)]


def route(board, settings, run, only=None, options=None, say=print):
    """Join the unjoined nets of a board (those named in `only`, if given).

    `board` reads and writes it: `read(nets)` gives its picture as it now stands (with the groups of joined pads
    of those nets at least), `write(solution, sizes)` adds a solution's copper and returns the numbers of track
    segments and vias added, `unjoined(nets)` names those of the nets whose pads are still not all joined.
    `run(problem, first)` runs the engine (`first`: the first run of all) and raises EngineFailed if it gives
    no solution.
    """
    outcome = Outcome()
    picture = board.read(only)
    names = picture.unjoined(only)
    open_before = set(board.unjoined())  # nets that are not this run's to join, or were not joined to begin with
    lots = batches(names)
    if len(lots) > 1:
        say(f"{len(names)} nets to join, more than the engine takes in one run: routed in {len(lots)} batches of at most "
            f"{max(len(l) for l in lots)}, each against the board as the batch before left it")
    wanted, last, first = [], "routed", True
    for number, lot in enumerate(lots):
        if number:
            picture = board.read(lot)
            say(f"batch {number + 1} of {len(lots)}")
        tries = Rules.of(picture, settings).tries(lot)
        number_of_try, reread, unlinked, repeats = 0, False, set(), 0
        while number_of_try < len(tries):
            step, narrow = tries[number_of_try]
            at_step = replace(settings, step=step)
            if reread:
                # what could not be joined is tried once more against the board as it now stands
                picture = board.read(lot)
            rules = Rules.of(picture, at_step)
            most = (options or {}).get("max_states")
            if number_of_try and most and rules.states(picture.box, picture.layers) > most:
                say(f"no try at a {step:g} mm step: the grid would be larger than --max-states")
                number_of_try, reread = number_of_try + 1, True
                continue
            problem = build_problem(picture, rules, only=set(lot), narrow=narrow, options=options, unlinked=unlinked)
            keys = list(dict.fromkeys(n["net"] for n in problem["nets"]))  # (a net may have several entries)
            if not number_of_try and reread is False and not unlinked:
                wanted += keys
            if not keys:
                break
            if number_of_try and reread:
                finer = step != tries[number_of_try - 1][0]
                say(f"{'next try, at a %g mm step' % step if finer else 'second try, narrower'}: {', '.join(keys)}")
            try:
                solution = run(problem, first)
            except EngineFailed as failure:
                outcome.failure = str(failure)
                break
            # A link from a capacitor to its pin is a wish, not a need. Where a net with such links is not all
            # laid, nothing of the run is kept: it is done again with that net whole, as it is from then on.
            failed = {n["net"] for n in problem["nets"] if any(m.startswith(n["key"] + ":") for m in solution.get("unrouted", []))}
            failed |= {n["net"] for n in problem["nets"] for c in solution.get("conflicts", []) if c.get("key") == n["key"]}
            linked = {n["net"] for n in problem["nets"] if n["key"].startswith(n["net"] + " | ")} - unlinked
            if failed & linked:
                say(f"not all joined with the capacitors linked to their pins: again without those links for {', '.join(sorted(failed & linked))}")
                unlinked |= failed & linked
                reread = False
                continue
            first = False
            added = board.write(solution, {n["key"]: (n["width"], n["via"]) for n in problem["nets"]})
            outcome.tracks, outcome.vias = outcome.tracks + added[0], outcome.vias + added[1]
            if solution["status"] == "routed":
                # The new tracks may have cut a pour and parted pads it joined: those nets are joined again,
                # against the board as it now stands.
                cut = [n for n in board.unjoined() if n in keys or n not in open_before] if repeats < POUR_REPEATS else []
                if not cut:
                    break
                say(f"parted by the new tracks, joined again: {', '.join(cut)}")
                wanted += [n for n in cut if n not in wanted]
                lot, reread, repeats = cut, True, repeats + 1
                continue
            last = solution["status"]
            lot = keys
            number_of_try, reread = number_of_try + 1, True
        if outcome.failure:
            break
    outcome.wanted = len(wanted)
    # the board has the last word: a net is joined when its pads are
    outcome.left = len(board.unjoined(set(wanted))) if wanted else 0
    outcome.status = "routed" if outcome.left == 0 else last if last != "routed" else "unroutable"
    return outcome


# --------------------------------------------------------------------------------------------------------------
# KiCad: the only code that uses pcbnew.

def MM(value):
    return pcbnew.ToMM(value)


def xy(p):
    return [round(MM(p.x), 6), round(MM(p.y), 6)]


def rings_of(poly_set):
    """Every outline of a SHAPE_POLY_SET as its rings: the outline first, then its holes."""
    out = []
    for i in range(poly_set.OutlineCount()):
        chain = poly_set.Outline(i)
        rings = [[xy(chain.CPoint(k)) for k in range(chain.PointCount())]]
        for h in range(poly_set.HoleCount(i)):
            hole = poly_set.Hole(i, h)
            rings.append([xy(hole.CPoint(k)) for k in range(hole.PointCount())])
        out.append(rings)
    return out


def pad_polygon(pad, layer):
    """A pad's copper on one of its layers, as a polygon that holds all of it: a round edge becomes corners
    that lie outside the true shape, by at most ARC_ERROR, so that what keeps its distance from the polygon
    keeps it from the pad."""
    shape = pcbnew.SHAPE_POLY_SET()
    pad.TransformShapeToPolygon(shape, layer, 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
    return shape


def pad_outline(pad, layer):
    """A pad's outline as it would be on a layer, for where it has an opening but no copper."""
    return pad_polygon(pad, layer)


def mask_opening(pad, layer):
    """How far the solder mask opening of a pad reaches beyond its outline on an outer copper layer, in mm; None
    where the mask covers it there (or on an inner layer)."""
    mask = pcbnew.F_Mask if layer == pcbnew.F_Cu else pcbnew.B_Mask if layer == pcbnew.B_Cu else None
    if mask is None or not pad.IsOnLayer(mask):
        return None
    try:
        return max(MM(pad.GetSolderMaskExpansion(layer)), 0.0)
    except TypeError:
        return max(MM(pad.GetSolderMaskExpansion()), 0.0)


def local_clearance(pad):
    """A pad's own clearance in mm, where it or its footprint overrides the net class; else None."""
    for owner in (pad, pad.GetParentFootprint()):
        try:
            value = owner.GetLocalClearance()
        except Exception:
            value = None
        if isinstance(value, tuple):
            value = value[0] if value else None
        if value:
            return MM(value)
    return None


def pad_middle(pad, layer):
    """The middle of a pad's copper: where a track meets a pad that has no hole."""
    try:
        return pad.ShapePos(layer)
    except TypeError:
        return pad.ShapePos()
    except Exception:
        return pad.GetPosition()


def pad_smallest(pad, layers):
    """A pad's smallest dimension on the given copper layers, in mm."""
    def size(layer):
        try:
            s = pad.GetSize(layer)
        except TypeError:
            s = pad.GetSize()
        return min(MM(s.x), MM(s.y))
    return min((size(l) for l in layers), default=None)


def drawing_polygon(item, layer):
    """A drawing or text on a copper layer as polygons; its bounding box if KiCad cannot give its outline."""
    poly = pcbnew.SHAPE_POLY_SET()
    try:
        item.TransformShapeToPolygon(poly, layer, 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
    except Exception:
        poly = pcbnew.SHAPE_POLY_SET()
    if poly.OutlineCount() == 0:
        box = item.GetBoundingBox()
        poly.NewOutline()
        for x, y in ((box.GetLeft(), box.GetTop()), (box.GetRight(), box.GetTop()), (box.GetRight(), box.GetBottom()), (box.GetLeft(), box.GetBottom())):
            poly.Append(x, y)
    return poly


def copper_layers(board):
    return [l for l in board.GetEnabledLayers().CuStack()]


def joined_pads(connectivity, pad):
    """The pads that copper already joins to a pad, followed from item to item through tracks and vias; and
    those tracks and vias."""
    found, pads, others, waiting = {pad.m_Uuid.AsString()}, [pad], [], [pad]
    while waiting:
        item = waiting.pop()
        for other in list(connectivity.GetConnectedTracks(item)) + list(connectivity.GetConnectedPads(item)):
            key = other.m_Uuid.AsString()
            if key in found:
                continue
            found.add(key)
            waiting.append(other)
            (pads if other.Type() == pcbnew.PCB_PAD_T else others).append(other)
    return pads, others


def pad_groups(board, nets=None, copper=None):
    """For each net with pads on copper (those named, if given): its groups of pads already joined. `copper`,
    if given, is filled with the tracks and vias of each group: {net: {item's id: index of its group}}."""
    enabled = copper_layers(board)
    connectivity = board.GetConnectivity()
    by_net = {}
    for pad in board.GetPads():
        name = pad.GetNetname()
        if name and pad.GetNetCode() > 0 and (nets is None or name in nets) and any(pad.IsOnLayer(l) for l in enabled):
            by_net.setdefault(name, []).append(pad)
    out = {}
    for name, pads in by_net.items():
        seen, groups = set(), []
        for pad in pads:
            if pad.m_Uuid.AsString() in seen:
                continue
            group, others = joined_pads(connectivity, pad)
            seen.update(p.m_Uuid.AsString() for p in group)
            if copper is not None:
                copper.setdefault(name, {}).update((o.m_Uuid.AsString(), len(groups)) for o in others)
            groups.append(group)
        out[name] = groups
    return out


def pour_groups(board, groups, of):
    """`groups` (pad_groups) with the groups that a filled zone joins made one, and for each net with pours
    {index of a group: [Pour]}. `of`: the group of each track and via (see pad_groups). The zones must be filled.

    A piece of a zone's fill joins the pads of its net whose copper on its layer it touches (through thermal
    spokes too; a pad it keeps clear of is not joined), and the vias and track ends of its net that stand on
    it: those of a group of pads, and those that reach no pad but join two pieces of pour.
    """
    enabled = copper_layers(board)
    name = board.GetLayerName
    connectivity = board.GetConnectivity()
    zones, copper = {}, {}
    for zone in board.Zones():
        if not zone.GetIsRuleArea() and zone.GetNetname() in groups:
            zones.setdefault(zone.GetNetname(), []).append(zone)
    for track in board.GetTracks():
        if track.GetNetname() in zones:
            copper.setdefault(track.GetNetname(), []).append(track)
    out, pours = {}, {}
    for net, own in groups.items():
        if net not in zones:
            out[net] = own
            continue
        # what a piece of pour can join: the groups of pads, then the clusters of copper that reach no pad
        node = {pad.m_Uuid.AsString(): g for g, group in enumerate(own) for pad in group}
        node.update(of.get(net, {}))
        count = len(own)
        for item in copper.get(net, []):
            if item.m_Uuid.AsString() in node:
                continue
            waiting = [item]
            node[item.m_Uuid.AsString()] = count
            while waiting:
                for other in connectivity.GetConnectedTracks(waiting.pop()):
                    if other.m_Uuid.AsString() not in node:
                        node[other.m_Uuid.AsString()] = count
                        waiting.append(other)
            count += 1
        leader = list(range(count))

        def find(g):
            while leader[g] != g:
                g = leader[g]
            return g

        islands = []  # (something it joins, Pour)
        for zone in zones[net]:
            for l in enabled:
                if not zone.IsOnLayer(l):
                    continue
                fill = zone.GetFilledPolysList(l)
                for i in range(fill.OutlineCount()):
                    island = pcbnew.SHAPE_POLY_SET(fill.Polygon(i))
                    box = island.BBox()
                    touched, items = set(), 0
                    for group in own:
                        for pad in group:
                            if pad.IsOnLayer(l) and box.Intersects(pad.GetBoundingBox()):
                                both = pad_polygon(pad, l)
                                both.BooleanIntersection(island)
                                if both.OutlineCount() > 0:
                                    touched.add(node[pad.m_Uuid.AsString()])
                                    items += 1
                    for item in copper.get(net, []):
                        via = item.Type() == pcbnew.PCB_VIA_T
                        if not (via or item.GetLayer() == l):
                            continue
                        ends = [item.GetPosition()] if via else [item.GetStart(), item.GetEnd()]
                        if any(island.Contains(end) for end in ends):
                            touched.add(node[item.m_Uuid.AsString()])
                            items += 1
                    if touched:
                        first = find(min(touched))
                        for g in touched:
                            leader[find(g)] = first
                        islands.append((first, Pour(net, name(l), rings_of(island)[0], lone=items == 1)))
        # the groups of pads, as the pours and the loose copper have joined them
        heads = sorted({find(g) for g in range(len(own))})
        out[net] = [[pad for g, group in enumerate(own) if find(g) == head for pad in group] for head in heads]
        for g, pour in islands:
            if find(g) in heads:  # (a pour that joins no pad is nobody's)
                pours.setdefault(net, {}).setdefault(heads.index(find(g)), []).append(pour)
    return out, pours


def kept_spokes(board, picture):
    """The thermal spokes that must stay: for each pad a filled zone of its net reaches through spokes, as many
    of them as the board asks of a pad (two unless it says otherwise), as polygons of the pad's net that other
    nets keep their distance from. Those furthest from the pads of other nets, where a track is least wanted.
    The zones must be filled.

    A spoke is found as it is filled: the zone's copper within the gap round the pad. A pad that the zone
    covers without a gap has none.
    """
    enabled = copper_layers(board)
    name = board.GetLayerName
    least = getattr(board.GetDesignSettings(), "m_MinResolvedSpokes", 2)
    others = [(pad.net, pad.at) for pad in picture.pads]
    out = []
    for zone in board.Zones():
        net = zone.GetNetname()
        if zone.GetIsRuleArea() or not net:
            continue
        gap = zone.GetThermalReliefGap()
        if gap <= 0:
            continue
        clearance = zone.GetLocalClearance()
        clearance = max(clearance if isinstance(clearance, int) else 0, zone.GetNet().GetNetClassSlow().GetClearance())
        for l in enabled:
            if not zone.IsOnLayer(l):
                continue
            fill = zone.GetFilledPolysList(l)
            box = fill.BBox()
            for pad in board.GetPads():
                if pad.GetNetname() != net or not pad.IsOnLayer(l) or not box.Intersects(pad.GetBoundingBox()):
                    continue
                # the ring between a fifth and four fifths of the gap from the pad
                ring, inner = pcbnew.SHAPE_POLY_SET(), pcbnew.SHAPE_POLY_SET()
                pad.TransformShapeToPolygon(ring, l, gap * 4 // 5, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
                pad.TransformShapeToPolygon(inner, l, gap // 5, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
                ring.BooleanSubtract(inner)
                whole = ring.Area()
                ring.BooleanIntersection(fill)
                if ring.OutlineCount() == 0 or ring.Area() > 0.6 * whole:
                    continue  # not reached, or covered without a gap
                # Grown by the gap, or by the clearance the zone keeps from a track if that is more (often more
                # than a track keeps from copper): enough for most spokes to keep their pour. More would make
                # sure of it, and close the ways between the pads beside a ground pad.
                ring.Inflate(max(gap, clearance), pcbnew.CORNER_STRATEGY_CHAMFER_ALL_CORNERS, pcbnew.FromMM(ARC_ERROR))
                spokes = []
                for rings in rings_of(ring):
                    middle = [sum(p[0] for p in rings[0]) / len(rings[0]), sum(p[1] for p in rings[0]) / len(rings[0])]
                    room = min((math.dist(middle, at) for other, at in others if other != net), default=math.inf)
                    spokes.append((-room, middle, rings))
                out.extend(Polygon(rings, name(l), net) for _, _, rings in sorted(spokes)[:max(least, KEPT_SPOKES)])
    return out


def joined(board, nets=None, pours=True):
    """For each net with pads on copper (those named, if given): its groups of pads already joined, by tracks,
    vias and pads that touch and, with `pours`, by filled zones; and the pours of each net that has them."""
    of = {}
    groups = pad_groups(board, nets, of)
    return pour_groups(board, groups, of) if pours else (groups, {})


def unjoined_nets(board, nets=None, pours=True):
    """The nets (of those named, if given) whose pads the board's copper does not all join. The board's
    connectivity must be up to date (BuildConnectivity after adding copper), and its zones filled."""
    return sorted(name for name, groups in joined(board, nets, pours)[0].items() if len(groups) > 1)


def read_board(board, ignored=frozenset(), nets=None, pours=True):
    """A board as a Picture. `nets`: work out the groups of joined pads for these nets only. `pours`: what a
    filled zone joins counts as joined (the zones must be filled)."""
    settings = board.GetDesignSettings()
    enabled = copper_layers(board)
    name = board.GetLayerName
    box = board.GetBoardEdgesBoundingBox()
    if box.GetWidth() == 0 or box.GetHeight() == 0:
        box = board.ComputeBoundingBox(False)
    picture = Picture(layers=[name(l) for l in enabled], ignored=frozenset(ignored),
                      box=[MM(box.GetLeft()), MM(box.GetTop()), MM(box.GetRight()), MM(box.GetBottom())],
                      minima=Minima(clearance=MM(settings.m_MinClearance), hole_clearance=MM(settings.m_HoleClearance),
                                    edge_clearance=MM(settings.m_CopperEdgeClearance), track_width=MM(settings.m_TrackMinWidth),
                                    via_size=MM(settings.m_ViasMinSize), through_drill=MM(settings.m_MinThroughDrill),
                                    annular_width=MM(settings.m_ViasMinAnnularWidth), hole_to_hole=MM(settings.m_HoleToHoleMin)))

    index = {}
    for pad in board.GetPads():
        net = pad.GetNetname() or None
        if net and pad.GetNetCode() > 0 and net not in picture.classes:
            cls = pad.GetNet().GetNetClassSlow()
            picture.classes[net] = NetClass(MM(cls.GetClearance()), MM(cls.GetTrackWidth()), MM(cls.GetViaDiameter()), MM(cls.GetViaDrill()))
        on = [l for l in enabled if pad.IsOnLayer(l)]
        shapes, openings = {}, {}
        for l in enabled:
            opening = mask_opening(pad, l)
            if l in on:
                shapes[name(l)] = rings_of(pad_polygon(pad, l))
            elif opening is not None:
                # an opening without copper under it (a pad on the other side's mask, or on no copper layer at all)
                shapes[name(l)] = rings_of(pad_outline(pad, on[0] if on else l))
            if opening is not None:
                openings[name(l)] = opening
        drill = pad.GetDrillSize()
        drilled = drill.x > 0
        index[pad.m_Uuid.AsString()] = len(picture.pads)
        picture.pads.append(Pad(net=net, at=xy(pad.GetPosition() if drilled or not on else pad_middle(pad, on[0])),
                                layers=[name(l) for l in on], shapes=shapes, openings=openings,
                                hole_at=xy(pad.GetPosition()) if drilled else None,
                                drill=MM(max(drill.x, drill.y)) if drilled else None,
                                clearance=local_clearance(pad), smallest=pad_smallest(pad, on),
                                part=pad.GetParentFootprint().GetReference() if pad.GetParentFootprint() else None))

    for track in board.GetTracks():
        net = track.GetNetname() or None
        if track.Type() == pcbnew.PCB_VIA_T:
            try:
                width = MM(track.GetWidth(pcbnew.F_Cu))
            except TypeError:
                width = MM(track.GetWidth())
            try:
                drill = MM(track.GetDrillValue())
            except Exception:
                drill = MM(track.GetDrill())
            picture.vias.append(Via(at=xy(track.GetPosition()), width=width, drill=drill, net=net))
        elif track.Type() == pcbnew.PCB_ARC_T:
            # an arc as a few straight pieces
            poly = pcbnew.SHAPE_POLY_SET()
            track.TransformShapeToPolygon(poly, track.GetLayer(), 0, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
            picture.polygons.extend(Polygon(rings, name(track.GetLayer()), net) for rings in rings_of(poly))
        else:
            picture.tracks.append(Track(a=xy(track.GetStart()), b=xy(track.GetEnd()), width=MM(track.GetWidth()), net=net, layer=name(track.GetLayer())))

    # text and drawings on copper layers, on the board and in footprints: copper of no net
    drawn = list(board.GetDrawings())
    for footprint in board.GetFootprints():
        drawn.extend(footprint.GraphicalItems())
        drawn.extend(f for f in footprint.GetFields() if f.IsVisible())
    for item in drawn:
        for l in enabled:
            if item.IsOnLayer(l):
                picture.polygons.extend(Polygon(rings, name(l)) for rings in rings_of(drawing_polygon(item, l)))

    outline = pcbnew.SHAPE_POLY_SET()
    try:
        has_outline = board.GetBoardPolygonOutlines(outline, False)
    except TypeError:
        has_outline = board.GetBoardPolygonOutlines(outline)
    if has_outline:
        picture.outline = rings_of(outline)
    for zone in board.Zones():
        if not zone.GetIsRuleArea():
            continue  # filled copper is cut by new tracks when the zones are filled again
        if not (zone.GetDoNotAllowTracks() or zone.GetDoNotAllowVias()):
            continue
        on = [name(l) for l in enabled if zone.IsOnLayer(l)]
        for rings in rings_of(zone.Outline()):
            picture.areas.append(Area(rings=rings, layers=on, tracks=bool(zone.GetDoNotAllowTracks()), vias=bool(zone.GetDoNotAllowVias())))

    groups, picture.pours = joined(board, nets, pours)
    if pours:
        picture.polygons.extend(kept_spokes(board, picture))
    for net, own in groups.items():
        picture.groups[net] = [[index[p.m_Uuid.AsString()] for p in group if p.m_Uuid.AsString() in index] for group in own]
    return picture


def write_back(board, solution, sizes):
    """Add a solution's tracks and vias to the board; `sizes` gives (track width, [via diameter, drill]) per
    entry key. Returns the numbers of track segments and vias added."""
    codes = {net.GetNetname(): net for net in board.GetNetsByName().values()}
    layer_id = {board.GetLayerName(l): l for l in copper_layers(board)}
    at = lambda p: pcbnew.VECTOR2I(pcbnew.FromMM(p[0]), pcbnew.FromMM(p[1]))
    tracks = vias = 0
    for laid in solution["nets"]:
        net, (width, via) = codes[laid["net"]], sizes[laid["key"]]
        for track in laid["tracks"]:
            for a, b in zip(track["points"], track["points"][1:]):
                item = pcbnew.PCB_TRACK(board)
                item.SetStart(at(a)); item.SetEnd(at(b)); item.SetWidth(pcbnew.FromMM(width))
                item.SetLayer(layer_id[track["layer"]]); item.SetNet(net)
                board.Add(item); tracks += 1
        for v in laid["vias"]:
            item = pcbnew.PCB_VIA(board)
            item.SetPosition(at(v)); item.SetViaType(pcbnew.VIATYPE_THROUGH)
            try:
                item.SetWidth(pcbnew.F_Cu, pcbnew.FromMM(via[0]))
            except TypeError:
                item.SetWidth(pcbnew.FromMM(via[0]))
            item.SetDrill(pcbnew.FromMM(via[1])); item.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); item.SetNet(net)
            board.Add(item); vias += 1
    return tracks, vias


class KicadBoard:
    """A pcbnew board as `route` uses it; it keeps the zones filled and the connectivity up to date with what
    has been written. `pours`: what a filled zone joins counts as joined."""

    def __init__(self, board, ignored=frozenset(), pours=True):
        self.board, self.ignored, self.pours = board, ignored, pours
        self.zoned = pours and any(not zone.GetIsRuleArea() for zone in board.Zones())
        self.stale = self.zoned  # (a board may come with its zones not filled)

    def _fresh(self):
        if self.stale:
            if self.zoned:
                pcbnew.ZONE_FILLER(self.board).Fill(self.board.Zones())  # new tracks cut the pours
            self.board.BuildConnectivity()
            self.stale = False

    def read(self, nets=None):
        self._fresh()
        return read_board(self.board, self.ignored, nets, self.pours)

    def write(self, solution, sizes):
        self.stale = True
        return write_back(self.board, solution, sizes)

    def unjoined(self, nets=None):
        self._fresh()
        return unjoined_nets(self.board, nets, self.pours)


# --------------------------------------------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("board")
    parser.add_argument("-o", "--output", required=True)
    parser.add_argument("--nets", help="only these nets, comma separated (default: every net with pads not yet joined)")
    parser.add_argument("--layers", help="copper layers that may carry routes, comma separated (default: all)")
    parser.add_argument("--step", default="auto", help="grid step in mm, or 'auto' (the default): chosen from the track widths and clearances of the nets")
    parser.add_argument("--coarse-first", action="store_true", help="a first pass on a grid twice as coarse, then what is left on the fine one: about three times faster, a little more track and vias, and less room on a crowded board")
    parser.add_argument("--slack", type=float, default=0.002, help="added to every clearance for the grid, in mm")
    parser.add_argument("--variants", type=int, default=1, help="differently ordered attempts run side by side")
    parser.add_argument("--via-penalty", type=float, default=10.0, help="length of detour accepted before a via is preferred, in mm")
    parser.add_argument("--pours", action="store_true", help="not yet the default: fill the zones first and count pads that a filled zone joins as joined, so that a net with a zone gets tracks only from the pads its zone does not reach")
    parser.add_argument("--no-decoupling", action="store_true", help="do not join a capacitor that stands by a pin of an IC to that pin itself")
    parser.add_argument("--bus-pull", type=float, default=0.2, help="how much longer a net that leaves and arrives beside others may run to stay beside them, as a share of its length (0: buses are not kept together)")
    parser.add_argument("--max-states", type=int, default=40_000_000, help="largest grid accepted, in cells times layers (memory grows with it and with the number of nets)")
    parser.add_argument("--problem", help="keep the problem file here (of the first run of the engine)")
    parser.add_argument("--solution", help="keep the solution file here (of the first run of the engine)")
    parser.add_argument("--engine", default=os.environ.get("PCB_ROUTING", "pcb-routing"), help="the pcb-routing program (default: $PCB_ROUTING, else pcb-routing on the path)")
    parser.add_argument("--threads", type=int, default=0, help="threads the engine may use (default: as many as the machine has)")
    args = parser.parse_args(argv)
    if pcbnew is None:
        parser.error("the pcbnew module is not here: run this with the Python that ships with KiCad")

    names = lambda text: set(text.split(",")) if text else None
    try:
        step = None if args.step == "auto" else float(args.step)
    except ValueError:
        parser.error("--step takes a number of mm, or auto")
    if not 0 <= args.bus_pull <= 10:
        parser.error("--bus-pull takes a number from 0 to 10")
    settings = Settings(step=step, coarse_first=args.coarse_first, slack=args.slack, via_penalty=args.via_penalty, layers=names(args.layers), bus_pull=args.bus_pull, decoupling=not args.no_decoupling, pours=args.pours)
    options = dict(variants=args.variants, max_states=args.max_states)
    run = lambda problem, first: run_engine(problem, args.engine, args.threads, args.problem if first else None, args.solution if first else None)

    board = pcbnew.LoadBoard(args.board)
    outcome = route(KicadBoard(board, ignored_checks(args.board), settings.pours), settings, run, only=names(args.nets), options=options)

    if outcome.failure:
        print(outcome.failure, file=sys.stderr)
        if not outcome.laid:
            return 2  # nothing laid: nothing to save
    if outcome.wanted == 0:
        print("nothing to route: every net's pads are joined")
        pcbnew.SaveBoard(args.output, board)
        return 0
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    pcbnew.SaveBoard(args.output, board)
    joined = f"{outcome.wanted - outcome.left} of {outcome.wanted} nets joined, {outcome.tracks} track segments and {outcome.vias} vias added; saved {args.output}"
    if outcome.failure:
        print(f"the engine failed; kept what it had laid before: {joined}")
        return 2
    print(f"{outcome.status}: {joined}")
    return 0 if outcome.status == "routed" else 1


if __name__ == "__main__":
    sys.exit(main())
