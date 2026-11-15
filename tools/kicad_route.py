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

Zones: the zones are filled first, and pads that a filled zone joins count as joined (--no-pours: only tracks,
vias and pads that touch do, and a net meant to be joined by a zone is routed as tracks).

- A net with a zone gets tracks only from the pads its zone does not reach, each to a pad of the net or to a
  point of the zone, whichever comes cheapest.
- A track of another net pays for its length across a zone (--pour-cost), and more where it would cut the
  spokes that tie a pad to its zone (--spoke-cost): it goes round where that is not far.
- After each run the zones are filled again. A track that ended on a piece of zone the other new tracks cut
  away is taken back, and pads that the new tracks parted from their zone are joined again. Where that cannot
  be done past the tracks that did it, the run is taken back and done again with those pads joined by a track.
- At the end every pad that is tied to its zone by fewer spokes than the board asks for (counted as KiCad's
  design rule check counts them) gets a straight track from its middle into the zone, which counts as a
  spoke; one that no such track fits by is named.

Run KiCad's design rule check on the result before using it.

What it does not do:

- It routes on a grid, with tracks at multiples of 45 degrees and through vias only (no blind, buried or micro
  vias, no arcs).
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
    zone: int = 0  # the zone it is a piece of: a number, the same for all pieces of one zone


@dataclass
class Thermal:
    """A pad tied to a filled zone of its net by thermal spokes, on one layer."""
    net: object
    pad: int  # its index among the pads
    layer: str
    least: int  # the spokes the board asks of it
    spokes: int  # those it has, counted as KiCad's design rule check counts them
    ground: list = field(default_factory=list)  # the ground where a track would cut its spokes: polygons
    outline: list = field(default_factory=list)  # for a pad with too few: the ring a spoke crosses, half the gap from the pad
    fill: list = field(default_factory=list)  # and the pieces of the zone it is tied to, as indices into the picture's fills


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
    pours: dict = field(default_factory=dict)  # net -> {index of a group: the pours that join it, as indices into fills}
    fills: list = field(default_factory=list)  # every piece of every filled zone, whatever it joins: [Pour]
    thermals: list = field(default_factory=list)  # every pad tied to a filled zone by spokes: [Thermal]

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
    pours: bool = True  # pads that a filled zone joins count as joined, and the rest of the net is taken to the zone
    pour_cost: float = 2.0  # what a track pays for its length across another net's pour, as a factor
    spoke_cost: float = 4.0  # and across the ground where it would cut a spoke that ties a pad to its pour
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

    def inset(self, net):
        """How far within a zone a track of a net is to end: half its width and a little."""
        return _r(self.track_width(net) / 2 + 0.1)

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


# --------------------------------------------------------------------------------------------------------------
# Zones: the geometry is the engine's (src/pours); these are the questions put to it. `ask` takes a question
# and returns the answer (see `Engine.ask`). Nothing here touches KiCad.

def _spots(pads):
    """Pads as a question names them: net, place and layers."""
    return [dict(net=pad.net, at=pad.at, layers=pad.layers) for pad in pads]


class _Named:
    """The pours of a picture that a question names, as the question takes them: `fills`, and for each pour
    named its index among them."""

    def __init__(self, picture):
        self.picture, self.index = picture, {}

    def __call__(self, pours):
        return [self.index.setdefault(f, len(self.index)) for f in pours]

    @property
    def fills(self):
        return [dict(net=pour.net, layer=pour.layer, rings=pour.rings) for pour in (self.picture.fills[f] for f in self.index)]


def reach_of(picture, rules, names, ask):
    """Where each group of joined pads of these nets may be reached: {net: per group, [(point, its layers)]}.
    The engine joins a group at whichever of its points comes cheapest.

    A lone pad is reached at the pad; a group of several at its pads nearest the other groups; one that a
    pour joins also at points of the pour near the others, a track's half width and a little within it.
    """
    if not names:
        return {}
    named, nets = _Named(picture), []
    for name in names:
        pours = picture.pours.get(name, {}) if rules.settings.pours else {}
        nets.append(dict(groups=picture.groups[name], inset=rules.inset(name),
                         pours=[named(pours.get(g, [])) for g in range(len(picture.groups[name]))]))
    found = ask(dict(ask="reach", pads=_spots(picture.pads), fills=named.fills, layers=rules.routing_layers(picture.layers),
                     step=rules.settings.step, nets=nets))
    return {name: [[(place["xy"], place["layers"]) for place in group] for group in groups] for name, groups in zip(names, found)}


def parts_of(picture, problem, solution, ask):
    """What a run laid, as what can be taken back piece by piece: the copper of an entry that has points of a
    pour among its terminals as the pieces that hang together (keyed `KEY # n`, with `of` the entry's key),
    each with the points of a pour it came to end on (`ends`: net, layer, xy); any other entry whole."""
    return ask(dict(ask="parts", pads=_spots(picture.pads), entries=problem["nets"], laid=solution["nets"]))


def on_zone(picture, points, ask):
    """Whether each of these points (net, layer, xy) is on a pour of its net, as the picture has the zones."""
    wanted = {(point["net"], point["layer"]) for point in points}
    fills = [dict(net=pour.net, layer=pour.layer, rings=pour.rings) for pour in picture.fills if (pour.net, pour.layer) in wanted]
    return ask(dict(ask="on_zone", fills=fills, points=points))


def tie_targets(picture, starved, ask):
    """For pads with too few spokes, as [(Thermal, the pad's middle, how far within the zone to end, points
    not to give again)]: for each, the points of its zone that a straight track from its middle may end on."""
    named = _Named(picture)
    pads = [dict(outline=thermal.outline, fill=named(thermal.fill), at=at, inset=inset, tried=tried) for thermal, at, inset, tried in starved]
    return ask(dict(ask="tie_targets", fills=named.fills, pads=pads)) if pads else []


def ties(picture, tracks, ask):
    """For tracks laid to tie a pad to its zone, as [(Thermal, the track's points)]: whether each counts as a
    spoke of its pad for KiCad."""
    named = _Named(picture)
    tracks = [dict(outline=thermal.outline, fill=named(thermal.fill), points=points) for thermal, points in tracks]
    return ask(dict(ask="ties", fills=named.fills, tracks=tracks)) if tracks else []


# --------------------------------------------------------------------------------------------------------------
# The problem itself.

TIE_ROUNDS = 3  # how often pads with too few spokes are given a track of their own to their zone
POUR_REPEATS = 8  # how often nets that new tracks have parted from their pour are joined again, for one lot of nets
POUR_AGAINS = 3  # and how often the lot is taken back and laid again to hold a cut pour together
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


def problem_of(picture, rules, ask, only=None, **more):
    """build_problem for the unjoined nets of a picture, each reached where the engine says (`ask` puts the
    question to it: see reach_of)."""
    return build_problem(picture, rules, reach_of(picture, rules, picture.unjoined(only), ask), only=only, **more)


def build_problem(picture, rules, reached, only=None, narrow=False, options=None, unlinked=frozenset(), stubs=None):
    """The router's problem for the unjoined nets of a picture (those named in `only`, if given).

    `reached`: where each of those nets may be reached (see reach_of). `narrow`: the second try, with
    narrower tracks and vias allowed in a net's own lands. `unlinked`: nets whose capacitors get no link of
    their own to the pin they stand by. `stubs`: instead of the unjoined nets, a track for each of these pads
    from its middle to its zone: [(key, Thermal, points of the zone to end on)].
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
    # A pour is cut by a track of another net, and a pad is cut off from its pour by one across its spokes: both
    # are open ground that costs more, to every net but the pour's own.
    dear = [dict(rings=pour.rings, layers=[pour.layer], factor=rules.settings.pour_cost, net=pour.net)
            for pour in picture.fills if pour.layer in layers and rules.settings.pour_cost > 1]
    dear += [dict(rings=rings, layers=[thermal.layer], factor=rules.settings.spoke_cost, net=thermal.net)
             for thermal in picture.thermals if thermal.layer in layers and rules.settings.spoke_cost > 1 for rings in thermal.ground]

    smallest = {}
    for pad in picture.pads:
        if pad.net and pad.smallest is not None:
            smallest[pad.net] = min(pad.smallest, smallest.get(pad.net, pad.smallest))
    unjoined = picture.unjoined(only) if stubs is None else []
    bus = buses(picture.pads, set(unjoined)) if rules.settings.bus_pull > 0 else {}
    nets = []
    def entry(key, name, terminals, via_penalty, **more):
        width = rules.narrow_width(name, smallest.get(name)) if narrow else rules.track_width(name)
        return dict(key=key, net=name, width=_r(width), via=[_r(v) for v in rules.via(name)], layers=layers,
                    clearance=dict(default=_r(rules.clearance(name)), nets={n: _r(c) for n, c in rules.stricter(name).items()}),
                    terminals=terminals, via_penalty_mm=via_penalty, **more)
    for key, thermal, targets in stubs or []:
        start = dict(xy=picture.pads[thermal.pad].at, layers=[thermal.layer])
        ends = [dict(xy=at, layers=[thermal.layer], piece=0) for at in targets]
        nets.append(entry(key, thermal.net, [start] + ends, rules.settings.via_penalty, no_vias=True))
    for place, name in enumerate(unjoined):
        # One terminal per group of pads that existing copper already joins, or several that stand for the
        # one piece of copper: a group of several pads, or one that a pour joins, is reached where it comes
        # cheapest (see `reach_of`), so that nothing is laid where a pour already joins.
        on = lambda pad: [l for l in layers if l in pad.layers]
        groups = picture.groups[name]
        pours = picture.pours.get(name, {}) if rules.settings.pours else {}
        reach = reached[name]
        if not all(reach):
            print(f"skipped {name}: a pad of it is on no routing layer", file=sys.stderr)
            continue
        def terminals(g):
            piece = dict(piece=g) if len(reach[g]) > 1 else {}
            return [dict(xy=at, layers=at_layers, **piece) for at, at_layers in reach[g]]
        terminal = lambda pad: dict(xy=pad.at, layers=on(pad))
        # A capacitor beside a pin of an IC is joined to that pin, in an entry of its own that is slow to
        # change layer; the rest of the net is joined as it would be without the capacitor. (Not on a net
        # with a pour: there the pin and the capacitor both stand on it, or are each taken to it.)
        links = decoupling(picture.pads, groups) if rules.settings.decoupling and name not in unlinked and not pours else []
        # (as many as the engine has room for beside this net's own entry and one for each net still to come)
        room = ENGINE_NETS - 1 - len(nets) - 1 - (len(unjoined) - place - 1)
        links = [(g, pin) for g, pin in links if on(picture.pads[pin])][:max(0, room)]
        taken = {g for g, _ in links}
        rest = [g for g in range(len(groups)) if g not in taken]
        for g, pin in links:
            cap = picture.pads[groups[g][0]]
            key = f"{name} | {cap.part}"
            while key in picture.groups or any(n["key"] == key for n in nets):
                key += "'"  # (a net may be named just so)
            nets.append(entry(key, name, [terminal(cap), terminal(picture.pads[pin])],
                              rules.settings.via_penalty * DECOUPLING_VIA_PENALTY))
        if len(rest) < 2:
            continue
        nets.append(entry(name, name, [t for g in rest for t in terminals(g)], rules.settings.via_penalty))
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
                **(dict(dear=dear) if dear else {}), rules=problem_rules, nets=nets, options=dict(dict(bus_pull=rules.settings.bus_pull), **(options or {})))


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


def ask_engine(question, engine, threads=0):
    """Put a question about a board's zones to the pcb-routing program (its `pours` command) and return the
    answer. `engine` is the program (or a command as a list)."""
    command = ([engine] if isinstance(engine, (str, os.PathLike)) else list(engine)) + ["pours"] + (["--threads", str(threads)] if threads else [])
    try:
        run = subprocess.run(command, input=json.dumps(question), capture_output=True, text=True)
    except OSError as error:
        raise EngineFailed(f"cannot run {command[0]}: {error}")
    if run.returncode:
        raise EngineFailed(run.stderr.strip() or f"{command[0]} pours ended with status {run.returncode}")
    try:
        return json.loads(run.stdout)
    except ValueError:
        raise EngineFailed(f"{command[0]} pours gave no answer that can be read")


@dataclass
class Engine:
    """The pcb-routing program as `route` uses it."""
    program: object  # the program, or a command as a list
    threads: int = 0  # the threads it may use (0: as many as the machine has)
    problem_path: object = None  # where the problem of the first run is kept, if anywhere
    solution_path: object = None  # and its solution

    def route(self, problem, first=False):
        """Run a problem and return its solution (`first`: the first run of all); EngineFailed if it gives none."""
        return run_engine(problem, self.program, self.threads, self.problem_path if first else None, self.solution_path if first else None)

    def ask(self, question):
        """Put a question about the board's zones (see src/pours) and return the answer; EngineFailed if it gives none."""
        return ask_engine(question, self.program, self.threads)


@dataclass
class Outcome:
    status: str = "routed"  # routed, or how the engine's last incomplete run ended (unroutable, conflicts)
    wanted: int = 0  # nets that were to be joined
    left: int = 0  # of them, those still not joined
    tracks: int = 0
    vias: int = 0
    failure: object = None  # what the engine said, if a run of it failed
    tied: int = 0  # pads with too few spokes to their zone that were given a track to it
    starved: list = field(default_factory=list)  # and those left with too few, by name

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


def route(board, settings, engine, only=None, options=None, say=print):
    """Join the unjoined nets of a board (those named in `only`, if given).

    `board` reads and writes it: `read(nets)` gives its picture as it now stands (with the groups of joined pads
    of those nets at least), `write(solution, sizes)` adds a solution's copper and returns the numbers of track
    segments and vias added, `unjoined(nets)` names those of the nets whose pads are still not all joined; for
    a board with pours, `take_back(keys)` removes what the last solution written laid for those entries, `mark()` gives the place
    in the record of what has been written and `rewind(mark)` removes all written since (both return the
    numbers of track segments and vias removed).
    `engine` runs problems and answers questions about zones (see `Engine`).
    """
    outcome = Outcome()
    picture = board.read(only)
    names = picture.unjoined(only)
    open_before = set(board.unjoined())  # nets that are not this run's to join, or were not joined to begin with
    starved_before = {(t.pad, t.layer) for t in picture.thermals if t.spokes < t.least}
    given_up = set()  # nets that an earlier batch left unjoined
    lots = batches(names)
    if len(lots) > 1:
        say(f"{len(names)} nets to join, more than the engine takes in one run: routed in {len(lots)} batches of at most "
            f"{max(len(l) for l in lots)}, each against the board as the batch before left it")
    wanted, last, first = [], "routed", True
    # (a question about the zones that gets no answer ends the run as a failed run of the engine does: what
    # has been laid is kept)
    try:
        for number, lot in enumerate(lots):
            if number:
                picture = board.read(lot)
                say(f"batch {number + 1} of {len(lots)}")
            tries = Rules.of(picture, settings).tries(lot)
            number_of_try, reread, unlinked, repeats = 0, False, set(), 0
            # For pours that new tracks cut: the last run of the lot's own nets, as (its problem, where the board's
            # record stood before it, its picture); whether the present run is one that joins a cut pour again;
            # the entries that hold cut pours together; and whether the lot's run is to be done again with those.
            base, rejoining, bridges, again, agains = None, False, [], False, 0
            while number_of_try < len(tries):
                step, narrow = tries[number_of_try]
                at_step = replace(settings, step=step)
                if again:
                    picture = base[2]
                    problem = dict(base[0], nets=[n for n in base[0]["nets"] if " ~ " not in n["key"]] + bridges)
                else:
                    if reread:
                        # what could not be joined is tried once more against the board as it now stands
                        picture = board.read(lot)
                    rules = Rules.of(picture, at_step)
                    most = (options or {}).get("max_states")
                    if number_of_try and most and rules.states(picture.box, picture.layers) > most:
                        say(f"no try at a {step:g} mm step: the grid would be larger than --max-states")
                        number_of_try, reread = number_of_try + 1, True
                        continue
                    problem = problem_of(picture, rules, engine.ask, only=set(lot), narrow=narrow, options=options, unlinked=unlinked)
                keys = list(dict.fromkeys(n["net"] for n in problem["nets"]))  # (a net may have several entries)
                if not number_of_try and reread is False and not unlinked:
                    wanted += [n for n in keys if n not in wanted]
                if not keys:
                    break
                if number_of_try and reread:
                    finer = step != tries[number_of_try - 1][0]
                    say(f"{'next try, at a %g mm step' % step if finer else 'second try, narrower'}: {', '.join(keys)}")
                try:
                    solution = engine.route(problem, first)
                except EngineFailed as failure:
                    outcome.failure = str(failure)
                    break
                # A link from a capacitor to its pin is a wish, not a need. Where a net with such links is not all
                # laid, nothing of the run is kept: it is done again with that net whole, as it is from then on.
                failed = {n["net"] for n in problem["nets"] if any(m.startswith(n["key"] + ":") for m in solution.get("unrouted", []))}
                failed |= {n["net"] for n in problem["nets"] for c in solution.get("conflicts", []) if c.get("key") == n["key"]}
                linked = {n["net"] for n in problem["nets"] if n["key"].startswith(n["net"] + " | ")} - unlinked
                if failed & linked and not again:
                    say(f"not all joined with the capacitors linked to their pins: again without those links for {', '.join(sorted(failed & linked))}")
                    unlinked |= failed & linked
                    reread = False
                    continue
                first = False
                mark = board.mark() if settings.pours else None
                # (an entry that has points of a pour among its terminals is written piece by piece: see below)
                parts = parts_of(picture, problem, solution, engine.ask) if settings.pours else [dict(laid, of=laid["key"]) for laid in solution["nets"]]
                sizes = {n["key"]: (n["width"], n["via"]) for n in problem["nets"]}
                added = board.write(dict(solution, nets=parts), {part["key"]: sizes[part["of"]] for part in parts})
                # A track that ends on a pour is joined only while the pour is there: where the other new tracks
                # have cut it away, the copper that hangs on that end is taken back, and its net is among those
                # still to join.
                ends = [(part["key"], end) for part in parts for end in part.get("ends", [])]
                standing = on_zone(board.read(), [end for _, end in ends], engine.ask) if ends else []
                loose = sorted({key for (key, _), stands in zip(ends, standing) if not stands})
                if loose:
                    taken = board.take_back(loose)
                    added = (added[0] - taken[0], added[1] - taken[1])
                    nets_of = {part["key"]: part["net"] for part in parts}
                    say(f"ended on a pour that the other new tracks cut away, taken back: {', '.join(sorted({nets_of[key] for key in loose}))}")
                outcome.tracks, outcome.vias = outcome.tracks + added[0], outcome.vias + added[1]
                if solution["status"] == "routed":
                    # The new tracks may have cut a pour and parted pads it joined: those nets are joined again,
                    # against the board as it now stands.
                    # (any net of this lot so far, or one that was joined before the run began)
                    cut = [n for n in board.unjoined() if (n in wanted or n not in open_before) and n not in given_up] if repeats < POUR_REPEATS else []
                    if not cut:
                        break
                    if not rejoining:
                        base = (problem, mark, picture)
                    say(f"parted by the new tracks, joined again: {', '.join(cut)}")
                    wanted += [n for n in cut if n not in wanted]
                    lot, reread, repeats, rejoining, again = cut, True, repeats + 1, True, False
                    continue
                if rejoining and base is not None and settings.pours and agains < POUR_AGAINS and len(base[0]["nets"]) + len(bridges) + len(problem["nets"]) < ENGINE_NETS:
                    # A cut pour could not be joined again past the tracks that cut it (they may have shut a piece
                    # of it in). Everything laid since is taken back, and the lot is laid again with the pads of
                    # the pieces as they were cut to be joined by a track, pad to pad: the others then leave the
                    # way for it. (Not from a point of the pour to another: on the pour whole again those are a
                    # step apart, and a track between them holds nothing together.)
                    taken = board.rewind(base[1])
                    outcome.tracks, outcome.vias = outcome.tracks - taken[0], outcome.vias - taken[1]
                    by_pads = problem_of(picture, Rules.of(picture, replace(at_step, pours=False)), engine.ask, only=set(keys),
                                            narrow=narrow, options=options, unlinked=set(keys))
                    bridges += [dict(entry, key=f"{entry['net']} ~ {len(bridges) + k + 1}") for k, entry in enumerate(by_pads["nets"])]
                    say(f"no way to join them past the new tracks: those are laid again, with the pieces of the pour held together: {', '.join(keys)}")
                    lot, reread, rejoining, again, agains = list(dict.fromkeys(n["net"] for n in base[0]["nets"])), False, False, True, agains + 1
                    continue
                # (the next try is for what is left, against the board as it stands: nothing of this one is laid again)
                base, rejoining, bridges, again = None, False, [], False
                last = solution["status"]
                lot = keys
                number_of_try, reread = number_of_try + 1, True
            if outcome.failure:
                break
            given_up = set(board.unjoined(set(wanted))) if wanted else set()
        if settings.pours and not outcome.failure:
            tie_starved(board, settings, engine, outcome, only, starved_before, options, say)
    except EngineFailed as failure:
        outcome.failure = str(failure)
    # (a net that was joined and that the new tracks parted for good is one left unjoined, as much as any)
    try:
        wanted += [n for n in board.unjoined() if n not in open_before and n not in wanted] if wanted else []
        # the board has the last word: a net is joined when its pads are
        outcome.left = len(board.unjoined(set(wanted))) if wanted else 0
    except EngineFailed as failure:  # (what the zones now join is not known: every net counts as left)
        outcome.failure, outcome.left = outcome.failure or str(failure), len(wanted)
    outcome.wanted = len(wanted)
    outcome.status = "routed" if outcome.left == 0 else last if last != "routed" else "unroutable"
    return outcome


def tie_starved(board, settings, engine, outcome, only=None, before=frozenset(), options=None, say=print):
    """Give every pad that is tied to its zone by fewer spokes than the board asks for a straight track from
    its middle into the zone, where one fits: that is a tie as good as a spoke, and counted as one. Tracks of
    other nets across its spokes are what starve a pad: one that this run starved is tied whatever its net;
    one that the board came with (`before`: those, as (pad, layer)) is tied as well, if its net is among
    those named in `only` or none are named. What was done and what could not be is noted in `outcome`."""
    name = lambda picture, thermal: f"{picture.pads[thermal.pad].part or '?'} ({thermal.net}, {thermal.layer})"
    ours = lambda t: (t.pad, t.layer) not in before or only is None or t.net in only
    routed_on = lambda picture: Rules.of(picture, settings).routing_layers(picture.layers)
    starved_of = lambda picture: [t for t in picture.thermals if t.spokes < t.least and ours(t) and t.layer in routed_on(picture)]
    tried = collections.defaultdict(list)  # (pad, layer) -> the points of its zone already tried
    starved = []
    for round_ in range(TIE_ROUNDS + 1):
        picture = board.read()
        starved = starved_of(picture)
        if not starved or round_ == TIE_ROUNDS:
            break
        rules = Rules.of(picture, settings)
        rules = Rules.of(picture, replace(settings, step=rules.step({t.net for t in starved})))
        asked = [(thermal, picture.pads[thermal.pad].at, rules.inset(thermal.net), tried[thermal.pad, thermal.layer])
                 for thermal in starved]
        stubs = []
        try:
            for (thermal, *_), targets in zip(asked, tie_targets(picture, asked, engine.ask)):
                if targets and len(stubs) < ENGINE_NETS - 1:
                    stubs.append((f"{thermal.net} + {len(stubs)}", thermal, targets))
            if not stubs:
                break  # (nothing left to try: `starved` is what stays so)
            # (the second round on: narrower, where the net's own width found no room)
            problem = build_problem(picture, rules, {}, narrow=round_ > 0, options=options, stubs=stubs)
            solution = engine.route(problem)
            # only a track that counts as a spoke is kept: straight from the pad onto the zone
            laid = {net["key"]: net for net in solution["nets"]}
            alone = [(thermal, laid[key]["tracks"][0]["points"]) for key, thermal, _ in stubs
                     if key in laid and len(laid[key]["tracks"]) == 1 and not laid[key].get("vias")]
            tying = {id(thermal) for (thermal, _), tied in zip(alone, ties(picture, alone, engine.ask)) if tied}
        except EngineFailed as failure:
            say(f"no tracks to the zones of pads with too few spokes: {failure}")
            break
        kept = []
        for key, thermal, targets in stubs:
            tracks = laid.get(key, {}).get("tracks", [])
            if id(thermal) in tying:
                kept.append(laid[key])
            # A point is not given again once a track has been laid to it that does not tie, nor any once a
            # narrower track has found no way to any of them. (A track's ends, whichever way round it is.)
            ends = [p for track in tracks for p in (track["points"][0], track["points"][-1])]
            reached = [t for t in targets if any(math.dist(t, p) < 2 * rules.settings.step for p in ends)]
            tried[thermal.pad, thermal.layer] += reached if tracks else targets if round_ > 0 else []
        if kept:
            added = board.write(dict(solution, nets=kept), {n["key"]: (n["width"], n["via"]) for n in problem["nets"]})
            outcome.tracks, outcome.tied = outcome.tracks + added[0], outcome.tied + len(kept)
    outcome.starved = sorted(name(picture, thermal) for thermal in starved)
    if outcome.tied:
        say(f"{outcome.tied} pads with too few spokes to their zone tied to it by a track of their own")
    if outcome.starved:
        say(f"too few spokes to their zone, and no room for a track to it: {', '.join(outcome.starved)}")


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


def filled_zones(board):
    """The zones that are filled with copper of a net, each with its number among them."""
    return list(enumerate(zone for zone in board.Zones() if not zone.GetIsRuleArea() and zone.GetNetname()))


def zone_fills(board):
    """Every piece of every filled zone on copper: [Pour]. The zones must be filled."""
    name = board.GetLayerName
    out = []
    for number, zone in filled_zones(board):
        for l in copper_layers(board):
            if zone.IsOnLayer(l):
                fill = zone.GetFilledPolysList(l)
                out.extend(Pour(zone.GetNetname(), name(l), rings_of(pcbnew.SHAPE_POLY_SET(fill.Polygon(i)))[0], number)
                           for i in range(fill.OutlineCount()))
    return out


def zone_wires(board, of):
    """The vias and tracks of the nets that have zones, as the engine's survey takes them: net, layer (none
    for a via), ends, and the group of pads each is joined to (`of`: the group of each track and via, see
    pad_groups), none for copper that reaches no pad."""
    name = board.GetLayerName
    zoned = {zone.GetNetname() for _, zone in filled_zones(board)}
    out = []
    for item in board.GetTracks():
        net = item.GetNetname()
        if net in zoned:
            via = item.Type() == pcbnew.PCB_VIA_T
            out.append(dict(net=net, layer=None if via else name(item.GetLayer()), group=of.get(net, {}).get(item.m_Uuid.AsString()),
                            ends=[xy(item.GetPosition())] if via else [xy(item.GetStart()), xy(item.GetEnd())]))
    return out


def zone_connection(zone, pad):
    """How a zone is to meet a pad of its net: the pad's own setting, else its footprint's, else the zone's."""
    how = pad.GetLocalZoneConnection()
    footprint = pad.GetParentFootprint()
    if how == pcbnew.ZONE_CONNECTION_INHERITED and footprint is not None:
        how = footprint.GetLocalZoneConnection()
    if how == pcbnew.ZONE_CONNECTION_INHERITED:
        how = zone.GetPadConnection()
    if how == pcbnew.ZONE_CONNECTION_THT_THERMAL:
        through = pad.GetAttribute() == pcbnew.PAD_ATTRIB_PTH
        how = pcbnew.ZONE_CONNECTION_THERMAL if through else pcbnew.ZONE_CONNECTION_FULL
    return how


def thermal_gap(zone, pad):
    """The gap a zone leaves round a pad it meets by spokes: the pad's own, else the zone's."""
    own = pad.GetLocalThermalGapOverride()
    return own if isinstance(own, int) and own > 0 else zone.GetThermalReliefGap()


def spoked_pads(board, index):
    """The pads that filled zones of their net are to meet by thermal spokes, as the engine's survey takes
    them: per pad and layer, the zone (its number, see filled_zones), the line a spoke crosses (the pad's
    outline grown by half the gap), the gap, the clearance the zone keeps, the spokes the board asks for, and
    the tracks that begin on the pad, on that layer and elsewhere. `index`: a pad's id -> its index in the picture. The
    zones must be filled and the connectivity up to date."""
    enabled = copper_layers(board)
    name = board.GetLayerName
    connectivity = board.GetConnectivity()
    least = getattr(board.GetDesignSettings(), "m_MinResolvedSpokes", 2)
    pads = {}
    for pad in board.GetPads():
        pads.setdefault(pad.GetNetname(), []).append(pad)
    out = []
    for number, zone in filled_zones(board) if least > 0 else []:
        clearance = zone.GetLocalClearance()
        clearance = max(clearance if isinstance(clearance, int) else 0, zone.GetNet().GetNetClassSlow().GetClearance())
        for l in enabled:
            fill = zone.GetFilledPolysList(l) if zone.IsOnLayer(l) else None
            if fill is None or fill.OutlineCount() == 0:
                continue
            whole = fill.BBox()
            for pad in pads.get(zone.GetNetname(), []):
                if not (pad.IsOnLayer(l) and pad.FlashLayer(l)) or zone_connection(zone, pad) != pcbnew.ZONE_CONNECTION_THERMAL:
                    continue
                gap = thermal_gap(zone, pad)
                if gap <= 0 or pad.m_Uuid.AsString() not in index or not whole.Intersects(pad.GetBoundingBox()):
                    continue
                tracks = [track for track in connectivity.GetConnectedTracks(pad) if track.Type() != pcbnew.PCB_VIA_T]
                ends = lambda here: [[xy(track.GetStart()), xy(track.GetEnd())] for track in tracks if (track.GetLayer() == l) == here]
                out.append(dict(pad=index[pad.m_Uuid.AsString()], layer=name(l), zone=number, gap=MM(gap), clearance=MM(clearance),
                                least=least, line=rings_of(_transformed(pad, l, gap // 2))[0][0], tracks=ends(True), elsewhere=ends(False)))
    return out


def _transformed(pad, layer, by):
    """A pad's copper on a layer grown by `by`, as a polygon set."""
    out = pcbnew.SHAPE_POLY_SET()
    pad.TransformShapeToPolygon(out, layer, by, pcbnew.FromMM(ARC_ERROR), pcbnew.ERROR_OUTSIDE)
    return out


def unjoined_nets(board, nets=None):
    """The nets (of those named, if given) whose pads the board's tracks and vias do not all join, whatever
    its zones join. The board's connectivity must be up to date (BuildConnectivity after adding copper)."""
    return sorted(name for name, groups in pad_groups(board, nets).items() if len(groups) > 1)


def read_board(board, ignored, ask, pours=True):
    """A board as a Picture. With `pours`, what a filled zone joins counts as joined, and the picture has the
    zones and the pads they tie (the zones must be filled; `ask` puts the question to the engine); without,
    only tracks, vias and pads that touch join."""
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

    of = {}
    for net, own in pad_groups(board, copper=of).items():
        picture.groups[net] = [[index[p.m_Uuid.AsString()] for p in group if p.m_Uuid.AsString() in index] for group in own]
    picture.fills = zone_fills(board) if pours else []
    if picture.fills:
        # what the zones join, and how they tie each pad: the engine's to work out from the plain geometry
        zoned = {pour.net for pour in picture.fills}
        copper = lambda pad: {layer: pad.shapes[layer] for layer in pad.layers} if pad.net in zoned else {}
        pads = [dict(net=pad.net, at=pad.at, layers=pad.layers, shapes=copper(pad)) for pad in picture.pads]
        found = ask(dict(ask="survey", pads=pads, wires=zone_wires(board, of), groups=picture.groups,
                         fills=[dict(net=pour.net, layer=pour.layer, rings=pour.rings, zone=pour.zone) for pour in picture.fills],
                         spoked=spoked_pads(board, index)))
        picture.groups = found["groups"]
        picture.pours = {net: {g: pieces for g, pieces in enumerate(own) if pieces} for net, own in found["pours"].items()}
        picture.thermals = [Thermal(**thermal) for thermal in found["thermals"]]
    return picture


def write_back(board, solution, sizes, added=None):
    """Add a solution's tracks and vias to the board; `sizes` gives (track width, [via diameter, drill]) per
    entry key. Returns the numbers of track segments and vias added; `added`, if given, is filled with the
    items of each entry: {key: [item]}."""
    keep = lambda key, item: added.setdefault(key, []).append(item) if added is not None else None
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
                board.Add(item); tracks += 1; keep(laid["key"], item)
        for v in laid["vias"]:
            item = pcbnew.PCB_VIA(board)
            item.SetPosition(at(v)); item.SetViaType(pcbnew.VIATYPE_THROUGH)
            try:
                item.SetWidth(pcbnew.F_Cu, pcbnew.FromMM(via[0]))
            except TypeError:
                item.SetWidth(pcbnew.FromMM(via[0]))
            item.SetDrill(pcbnew.FromMM(via[1])); item.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); item.SetNet(net)
            board.Add(item); vias += 1; keep(laid["key"], item)
    return tracks, vias


class KicadBoard:
    """A pcbnew board as `route` uses it; it keeps the zones filled and the connectivity up to date with what
    has been written. `ask` puts a question to the engine; `pours`: what a filled zone joins counts as joined."""

    def __init__(self, board, ignored, ask, pours=True):
        self.board, self.ignored, self.ask, self.pours = board, ignored, ask, pours
        self.zoned = pours and any(not zone.GetIsRuleArea() for zone in board.Zones())
        self.stale = self.zoned  # (a board may come with its zones not filled)
        self.picture = None  # the board as it was last read, as long as nothing has been written since
        self.last = {}  # what the last solution written laid, by entry
        self.written = []  # and each one before it, in order

    def _fresh(self):
        if self.stale:
            if self.zoned:
                pcbnew.ZONE_FILLER(self.board).Fill(self.board.Zones())  # new tracks cut the pours
            self.board.BuildConnectivity()
            self.stale, self.picture = False, None

    def read(self, nets=None):
        self._fresh()
        if self.picture is None:
            self.picture = read_board(self.board, self.ignored, self.ask, self.pours)
        return self.picture

    def write(self, solution, sizes):
        self.stale, self.last = True, {}
        self.written.append(self.last)
        return write_back(self.board, solution, sizes, self.last)

    def mark(self):
        return len(self.written)

    def rewind(self, mark):
        tracks = vias = 0
        while len(self.written) > mark:
            self.last = self.written.pop()
            taken = self.take_back(list(self.last))
            tracks, vias = tracks + taken[0], vias + taken[1]
        self.last = {}
        return tracks, vias

    def take_back(self, keys):
        tracks = vias = 0
        for key in keys:
            for item in self.last.pop(key, []):
                via = item.Type() == pcbnew.PCB_VIA_T
                self.board.Remove(item)
                tracks, vias = tracks + (not via), vias + via
        self.stale = True
        return tracks, vias

    def unjoined(self, nets=None):
        if self.zoned:
            return self.read().unjoined(nets)
        self._fresh()
        return unjoined_nets(self.board, nets)


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
    parser.add_argument("--no-pours", action="store_true", help="do not count pads that a filled zone joins as joined: a net with a zone is routed as tracks, as if the zone were not there")
    parser.add_argument("--pour-cost", type=float, default=2.0, help="what a track pays for its length across another net's pour, as a factor (1: nothing more)")
    parser.add_argument("--spoke-cost", type=float, default=4.0, help="and across the ground where it would cut a spoke that ties a pad to its pour")
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
    if not (1 <= args.pour_cost <= 100 and 1 <= args.spoke_cost <= 100):
        parser.error("--pour-cost and --spoke-cost take a number from 1 to 100")
    settings = Settings(step=step, coarse_first=args.coarse_first, slack=args.slack, via_penalty=args.via_penalty, layers=names(args.layers), bus_pull=args.bus_pull, decoupling=not args.no_decoupling, pours=not args.no_pours,
                        pour_cost=args.pour_cost, spoke_cost=args.spoke_cost)
    options = dict(variants=args.variants, max_states=args.max_states)
    engine = Engine(args.engine, args.threads, args.problem, args.solution)

    if not Path(args.board).is_file():
        parser.error(f"no such board: {args.board}")  # (pcbnew crashes on a file that is not there)
    board = pcbnew.LoadBoard(args.board)
    try:
        outcome = route(KicadBoard(board, ignored_checks(args.board), engine.ask, settings.pours), settings, engine,
                        only=names(args.nets), options=options)
    except EngineFailed as failure:  # (a question about the zones that got no answer: nothing is saved)
        print(failure, file=sys.stderr)
        return 2

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
