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
  spoke. Where no such track fits by because new tracks of other nets pass close by the pad, those nets are
  laid again clear of it (and put back as they were if they cannot be). A pad still left with too few is named.

What the nets are for: taken from their names and parts, and from a hints file beside the board
(BOARD.routing.json, or --hints FILE; see docs/hints.md). Two nets named as the halves of a differential pair
are laid side by side where their pads stand side by side at both ends, and as two nets where that cannot be
done; clocks and crystals are slow to change layer; widths for a current, clearances for a voltage, layers,
"no vias" and the like come from the hints. What was taken from where is said at the start.

Run KiCad's design rule check on the result before using it.

What it does not do:

- It routes on a grid, with tracks at multiples of 45 degrees and through vias only (no blind, buried or micro
  vias, no arcs).
- No length tuning, and no impedance: a pair is laid at the width and gap it is told, or at its class's width
  and the clearance.

Exit status: 0 every net joined, 1 some nets left unjoined, 2 the engine failed (what it had laid before that
is saved all the same).
"""
import argparse, collections, fnmatch, itertools, json, math, os, re, subprocess, sys, tempfile
from dataclasses import dataclass, field, replace
from pathlib import Path

from net_names import CAPACITOR_PART, CHIP_PART, CRYSTAL_PART, GROUND, NAMED, is_supply, pair_names

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

def _r(v):
    return round(v, 6)


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
    gap: float = 0.0  # the gap the zone leaves round the pad
    clearance: float = 0.0  # the clearance the zone keeps from copper of other nets


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
# What the nets are for: guessed from their names and parts, and said in a hints file beside the board
# (BOARD.routing.json). A guess only leans a net (what it costs, how readily it gives way); what binds it
# (a width, a clearance, layers, no vias, a distance kept from other nets) comes from the hints alone.
# Nothing here touches KiCad.

ROLES = ("supply", "ground", "clock", "crystal", "sensitive", "noisy")
# How a role leans a net: (how readily it gives way, its weight among results, its via penalty as a factor).
# A role the hints give leans it all the way. One that is only guessed makes it slower to take a via and no
# more: on 20 public boards, guessed nets that held their way cost the other nets 13 % more vias.
LEANING = dict(clock=(0.5, 2.0, 2.0), crystal=(0.2, 3.0, 5.0), noisy=(0.3, 3.0, 1.0), sensitive=(0.5, 2.0, 1.0))
GUESSED = dict(clock=(1.0, 1.0, 2.0), crystal=(1.0, 1.0, 5.0))
HINTS = dict(role=str, width=float, clearance=float, amps=float, volts=float, max_mm=float, vias=str, layers=list, pair=(str, bool),
             gap=float, bus=(str, bool), priority=str, ohms=float, match=str)
# The most a hint may ask: beyond it, it is a slip of a unit (and the router would draw up a board of it).
MOST = dict(width=20.0, clearance=20.0, gap=20.0, amps=30.0, volts=5000.0)
NOT_ACTED_ON = ("ohms", "match")  # taken down and said back, but nothing in the router follows them yet


@dataclass
class Intent:
    """What is asked of a net beyond its class, and where each thing was taken from."""
    role: object = None  # one of ROLES
    width: object = None  # its track width, mm
    clearance: object = None  # what it keeps from all other copper, mm
    amps: object = None  # the current it carries: its tracks are at least as wide as that takes
    volts: object = None  # the voltage on it: it keeps at least the distance that takes
    max_mm: object = None  # its wanted greatest length
    vias: object = None  # "none", or "few"
    layers: object = None  # the layers it may use
    pair: object = None  # the net it is laid side by side with; False: with none, whatever its name
    gap: object = None  # between the legs of its pair, edge to edge
    bus: object = None  # the name of the bus it runs with; False: with none
    priority: object = None  # "high": it holds its shortest way and the others go round
    ohms: object = None
    match: object = None
    source: dict = field(default_factory=dict)  # what was set -> "names" or the hints file's name


@dataclass
class Hints:
    """A hints file as read: what it says of the nets a pattern names, in the order given, and its pairs."""
    name: str = ""
    nets: list = field(default_factory=list)  # [(pattern, {what: value})]
    pairs: list = field(default_factory=list)  # [{nets: [a, b], width, gap, ohms}]


def read_hints(path):
    """A hints file (JSON) as Hints; ValueError says what is wrong with it.

        {"nets": {"/adc/*": {"role": "sensitive"}, "+24V": {"amps": 3}, "CLK8": {"max_mm": 40, "vias": "none"}},
         "pairs": [{"nets": ["TX_P", "TX_N"], "width": 0.15, "gap": 0.18}]}

    A pattern is a net's name, with * and ? for any run of characters and any one (a name is tried with and
    without its leading /). Later patterns override earlier ones.
    """
    name = Path(path).name
    try:
        data = json.loads(Path(path).read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"{name}: {error}")
    wrong = lambda where, why: ValueError(f"{name}: {where}: {why}")
    number = lambda value: isinstance(value, (int, float)) and not isinstance(value, bool) and value > 0
    if not isinstance(data, dict) or set(data) - {"nets", "pairs"}:
        raise wrong("the file", "an object with `nets` and `pairs`, nothing else")
    if not isinstance(data.get("nets", {}), dict) or not isinstance(data.get("pairs", []), list):
        raise wrong("the file", "`nets` is an object (a pattern, and what is said of the nets it names), `pairs` a list")
    hints = Hints(name=name)
    for pattern, said in data.get("nets", {}).items():
        if not isinstance(said, dict):
            raise wrong(f"nets[{pattern}]", "an object")
        for what, value in said.items():
            kinds = HINTS.get(what)
            if kinds is None:
                raise wrong(f"nets[{pattern}].{what}", f"not known; one of {', '.join(sorted(HINTS))}")
            if kinds is float and not number(value):
                raise wrong(f"nets[{pattern}].{what}", "a number more than 0")
            if kinds is not float and not isinstance(value, kinds):
                raise wrong(f"nets[{pattern}].{what}", "of the wrong kind")
            if what in MOST and value > MOST[what]:
                raise wrong(f"nets[{pattern}].{what}", f"is {value}: {MOST[what]:g} at the most (mm, A, V)")
        for what, among in (("role", ROLES), ("vias", ("none", "few")), ("priority", ("high",))):
            if what in said and said[what] not in among:
                raise wrong(f"nets[{pattern}].{what}", f"one of {', '.join(among)}")
        if "layers" in said and not (said["layers"] and all(isinstance(layer, str) for layer in said["layers"])):
            raise wrong(f"nets[{pattern}].layers", "a list of layer names")
        for what in ("pair", "bus"):  # (a name, or false for none: true says nothing)
            if said.get(what) is True or said.get(what) == "":
                raise wrong(f"nets[{pattern}].{what}", "a name, or false")
        hints.nets.append((pattern, dict(said)))
    for k, pair in enumerate(data.get("pairs", [])):
        legs = pair.get("nets") if isinstance(pair, dict) else None
        if not (isinstance(legs, list) and len(legs) == 2 and all(isinstance(leg, str) for leg in legs) and legs[0] != legs[1]):
            raise wrong(f"pairs[{k}].nets", "the two nets of the pair")
        if set(pair) - {"nets", "width", "gap", "ohms"} or not all(number(pair[w]) and pair[w] <= MOST.get(w, math.inf) for w in pair if w != "nets"):
            raise wrong(f"pairs[{k}]", "`nets`, and `width`, `gap`, `ohms` as numbers more than 0 (20 mm at the most)")
        hints.pairs.append(dict(pair))
    return hints


def unheeded(picture, hints):
    """What a hints file says that nothing on the board answers to, as lines to say: a pattern that names no
    net, a pair with a net that is not there, a layer the board has not."""
    nets = sorted(picture.groups)
    named = lambda pattern: [net for net in nets if fnmatch.fnmatchcase(net, pattern) or fnmatch.fnmatchcase(net.lstrip("/"), pattern)]
    lines = []
    for pattern, said in (hints.nets if hints else []):
        if not named(pattern):
            lines.append(f"{hints.name}: {pattern} names no net of the board")
        other = said.get("pair")
        if isinstance(other, str) and other not in nets:
            lines.append(f"{hints.name}: {pattern} is to be a pair with {other}, which is no net of the board")
        if isinstance(other, str) and named(pattern) == [other]:
            lines.append(f"{hints.name}: {pattern} is to be a pair with itself")
        for layer in said.get("layers", []):
            if layer not in picture.layers:
                lines.append(f"{hints.name}: {pattern} is given the layer {layer}, which the board has not")
    for pair in (hints.pairs if hints else []):
        for leg in pair["nets"]:
            if leg not in nets:
                lines.append(f"{hints.name}: the pair {' = '.join(pair['nets'])} names {leg}, which is no net of the board")
    return lines


def intents_of(picture, hints=None, guesses=True):
    """What is asked of each net of a picture: {net: Intent}, from its name and parts (`guesses`) and then from
    the hints, which override."""
    nets = sorted(picture.groups)
    intents = {net: Intent() for net in nets}
    def take(net, source, **said):
        for what, value in said.items():
            setattr(intents[net], what, value)
            intents[net].source[what] = source
    if guesses:
        on_crystal = {pad.net for pad in picture.pads if pad.net and pad.part and CRYSTAL_PART.match(pad.part)}
        for net in nets:
            last = net.rsplit("/", 1)[-1]
            if GROUND.search(last) and is_supply(net):
                take(net, "names", role="ground")
            elif is_supply(net):
                take(net, "names", role="supply")
            elif net in on_crystal and len(picture.groups[net]) <= 3:
                take(net, "names", role="crystal")
            else:
                role = next((role for role, named in NAMED if named.search(last)), None)
                if role:
                    take(net, "names", role=role)
        for a, b in pair_names(nets):
            take(a, "names", pair=b)
            take(b, "names", pair=a)
    for pattern, said in (hints.nets if hints else []):
        for net in nets:
            if fnmatch.fnmatchcase(net, pattern) or fnmatch.fnmatchcase(net.lstrip("/"), pattern):
                take(net, hints.name, **said)
                other = said.get("pair")
                if isinstance(other, str) and other in intents:
                    take(other, hints.name, pair=net, **{w: said[w] for w in ("width", "gap", "ohms") if w in said})
    for pair in (hints.pairs if hints else []):
        a, b = pair["nets"]
        if a in intents and b in intents:
            sizes = {w: pair[w] for w in ("width", "gap", "ohms") if w in pair}
            take(a, hints.name, pair=b, **sizes)
            take(b, hints.name, pair=a, **sizes)
    # (a pair is two nets that name each other)
    for net in nets:
        other = intents[net].pair
        if other and (other not in intents or intents[other].pair != net):
            intents[net].pair = None
    return intents


def told(intents, only=None):
    """What was taken for the nets (those in `only`, if given) and from where, as lines to say."""
    lines = {}
    seen = set()
    for net in sorted(intents):
        intent = intents[net]
        if only is not None and net not in only:
            continue
        for what, source in sorted(intent.source.items()):
            value = getattr(intent, what)
            if value is None or (what == "pair" and (value is False or (value, net) in seen)):
                continue
            if what == "role" and source == "names" and value not in GUESSED:
                continue  # (a supply, a ground, a switch node by its name: nothing follows from that alone)
            seen.add((net, value) if what == "pair" else None)
            if what == "pair":
                text = f"pair {net} = {value}"
            elif what == "role":
                text = f"{value} {net}"
            else:
                text = f"{net} {what} {value if not isinstance(value, list) else ' '.join(value)}" + (" (not acted on)" if what in NOT_ACTED_ON else "")
            lines.setdefault(source, []).append(text)
    return [f"from {'the net names and parts' if source == 'names' else source}: {'; '.join(found)}" for source, found in sorted(lines.items())]


def amps_width(amps):
    """The width of an outer track of 35 um copper that carries a current with a rise of 10 degrees, in mm
    (IPC-2221: I = 0.048 dT^0.44 A^0.725, A in square mils)."""
    area = (amps / (0.048 * 10 ** 0.44)) ** (1 / 0.725)
    return _r(area / 1.378 * 0.0254)


def volts_clearance(volts):
    """The distance between bare outer conductors for a voltage, in mm (IPC-2221B table 6-1, column B2)."""
    for most, distance in ((30, 0.1), (150, 0.6), (300, 1.25), (500, 2.5)):
        if volts <= most:
            return distance
    return _r(2.5 + (volts - 500) * 0.005)


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
    pairs: bool = True  # two nets named as the halves of a differential pair are laid side by side
    guesses: bool = True  # what a net is for is guessed from its name and parts (see intents_of)
    lean: bool = True  # a role that is only guessed leans its net (see Rules.leaning); off for a run that joined less with it
    hints: object = None  # the hints file beside the board, as read (Hints)


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

    def __init__(self, minima, classes, ignored=frozenset(), settings=None, intents=None):
        self.minima, self.classes, self.ignored = minima, classes, frozenset(ignored)
        self.settings = settings or Settings()
        self.intents = intents or {}  # what is asked of each net beyond its class (see intents_of)
        # the smallest clearance any net keeps from other copper: what every track keeps from a pad or hole anyway
        self.least_clearance = min((self.clearance(net) for net in classes), default=self.kept(minima.clearance))

    @classmethod
    def of(cls, picture, settings=None):
        settings = settings or Settings()
        known = picture.__dict__.setdefault("_intents", {})  # (worked out once for a picture, hints and guesses)
        key = (id(settings.hints), settings.guesses)
        if key not in known:
            known[key] = intents_of(picture, settings.hints, settings.guesses)
        return cls(picture.minima, picture.classes, picture.ignored, settings, known[key])

    def intent(self, net):
        return self.intents.get(net) or Intent()

    def kept(self, distance):
        """A distance as the router is to keep it: with the slack for the grid. The one place it is added."""
        return distance + self.settings.slack

    def ignores(self, check):
        return check in self.ignored

    # -- between nets

    def clearance(self, net):
        """What a net keeps from copper of other nets: its class's clearance, at least the board's, and what
        the hints ask for it or its voltage takes."""
        intent = self.intent(net)
        asked = max(intent.clearance or 0.0, volts_clearance(intent.volts) if intent.volts else 0.0)
        return self.kept(max(self.classes[net].clearance, self.minima.clearance, asked))

    def stricter(self, net):
        """The nets whose class asks more than this net's: it keeps their clearance from their copper."""
        own = self.clearance(net)
        return {other: self.clearance(other) for other in self.classes if other != net and self.clearance(other) > own}

    def pair(self, a, b, nets):
        """How two nets are laid as a differential pair among `nets`, those of the run: (the width of each leg,
        the distance between the legs, centre to centre). The legs are as wide as the wider net's tracks, and
        as near each other as two nets laid in one run may come, or as far apart as the hints say if that is more."""
        width = max(self.track_width(a), self.track_width(b))
        gap = max(self.new_clearance(nets), self.intent(a).gap or 0.0, self.intent(b).gap or 0.0)
        return _r(width), _r(width + gap)

    def quiet_clearance(self, nets):
        """What a net the hints call sensitive keeps from the other nets laid with it, and they from it: twice
        what two ordinary nets keep. None if none of these nets is sensitive by the hints."""
        quiet = any(self.leaning(net).get("quiet") for net in nets)
        return _r(2 * self.new_clearance(nets)) if quiet else None

    def trunk_width(self, width, pitch):
        """How wide the trunk of a pair is laid: its two legs side by side, and what a leg takes on the
        outside of a turn of 45 degrees (so that such a corner is a corner), 0.04 mm at the least."""
        return _r(max(pitch + width + 0.04, pitch / math.cos(math.pi / 8) + width + 0.005))

    def leans(self, nets):
        """True if any of these nets is leaned by a role that is only guessed."""
        guessed = lambda intent: intent.role in GUESSED and intent.source.get("role") == "names"
        return self.settings.lean and any(guessed(self.intent(net)) for net in nets)

    def leaning(self, net):
        """What a net's entry is given beyond its sizes, for what the net is for: fields of the engine's entry,
        and `via_penalty` as a factor on the usual. A role leans it (how readily it gives way, its weight, its
        vias: less so where it is only guessed); the hints bind it (no vias, the layers it may use, a greatest
        length, a distance kept as a quiet net)."""
        intent, out = self.intent(net), {}
        hinted = lambda what: intent.source.get(what, "names") != "names"
        guessed = GUESSED if self.settings.lean else {}
        give, weight, vias = (LEANING if hinted("role") else guessed).get(intent.role, (1.0, 1.0, 1.0))
        if intent.priority == "high":
            give, weight = min(give, 0.3), max(weight, 3.0)
        if intent.vias == "few":
            vias = max(vias, 3.0)
        out.update({"yield": give} if give != 1.0 else {}, **(dict(weight=weight) if weight != 1.0 else {}), **(dict(via_penalty=vias) if vias != 1.0 else {}))
        if intent.vias == "none":
            out["no_vias"] = True
        if intent.max_mm:
            out["max_mm"] = intent.max_mm
        if intent.role == "sensitive" and hinted("role"):
            out["quiet"] = True
        if intent.layers:
            out["layers"] = list(intent.layers)
        return out

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
        """A net's track width: its class's, or what the hints say instead; at least what its current takes,
        and the board's narrowest."""
        intent = self.intent(net)
        return max(intent.width or self.classes[net].track_width, amps_width(intent.amps) if intent.amps else 0.0, self.minima.track_width)

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
        return bool(pad.part) and CAPACITOR_PART.match(pad.part) and len(both) == 2 and all(size[p.net] >= 3 for p in both)
    is_pin = lambda pad: bool(pad.part) and CHIP_PART.match(pad.part)
    links = []
    for g, group in enumerate(groups):
        if len(group) != 1 or not is_cap(pads[group[0]]):
            continue
        cap = pads[group[0]]
        pins = [(math.dist(cap.at, pads[i].at), i) for h, other in enumerate(groups) if h != g for i in other if is_pin(pads[i])]
        if pins and min(pins)[0] <= DECOUPLING_REACH:
            links.append((g, min(pins)[1]))
    return links


PAIR_PADS = 4  # the most pads a net of a pair has: more is a rail or a bus by another name


def pairs_of(picture, rules, names, ask, notes=None):
    """Which of these unjoined nets are to be laid as differential pairs, and how: [(first net, second net,
    leg width, pitch, plan)], the plan as the engine gives it (src/pairs.rs: the ends of the pair, the trunks
    between them and where each may begin and end, the pads at no end).

    Two nets are a pair if their names or the hints say so (see intents_of), neither has more than four pads,
    a zone, or a pad joined to another yet, both or neither are held apart as sensitive, and the engine finds
    them pads side by side at two ends or more, far enough apart to run together between. Why a pair by
    name is laid as two nets is added to `notes`, if given.
    """
    note = lambda a, b, why: notes.append(f"{a} = {b} is laid as two nets: {why}") if notes is not None else None
    other = lambda net: rules.intent(net).pair
    quiet = lambda net: bool(rules.leaning(net).get("quiet"))
    def why_not(a, b):
        if max(len(picture.groups[a]), len(picture.groups[b])) > PAIR_PADS:
            return f"one of them has more than {PAIR_PADS} pads"
        if a in picture.pours or b in picture.pours:
            return "one of them has a zone"
        if not all(len(group) == 1 for net in (a, b) for group in picture.groups[net]):
            return "some of their pads are joined already"
        if quiet(a) != quiet(b):
            return "one of them is sensitive by the hints and the other is not"
    named = []
    for a, b in [(a, other(a)) for a in names if other(a) in names and a < other(a)]:
        why = why_not(a, b)
        if why:
            note(a, b, why)
        else:
            named.append((a, b))
    if not named:
        return []
    spots = lambda net: _spots(picture.pads[group[0]] for group in picture.groups[net])
    sizes = [rules.pair(a, b, names) for a, b in named]
    plans = ask(dict(ask="pairs", layers=rules.routing_layers(picture.layers), step=rules.settings.step,
                     pairs=[dict(a=spots(a), b=spots(b), pitch=pitch) for (a, b), (_, pitch) in zip(named, sizes)]))
    for (a, b), plan in zip(named, plans):
        if not plan["trunks"]:
            note(a, b, "no two places where a pad of each stands by the other" if len(plan["ends"]) < 2 else "its ends are too near each other, or share no routing layer")
    return [(a, b, width, pitch, plan) for (a, b), (width, pitch), plan in zip(named, sizes, plans) if plan["trunks"]]


def pair_entries(picture, rules, pair, entry, unpaired, taken):
    """The entries for one differential pair (see pairs_of) and what the problem's `pairs` says of it:
    (entries, pairs); nothing if every stretch of it has been given up as a pair. `unpaired`: for the key of
    a stretch that was not all laid as a pair, how many of the nearest lots of places its trunk is no longer
    offered (it is begun further from its pads; with none left it is given up).

    For each stretch between two of its ends, a trunk: laid as one wide track of a net of its own between
    places a little off the pads (those of one end are one piece: the engine takes the nearest that are open), and four
    stubs that join the legs it is split into to the pads. What the trunks do not join is joined as the pads
    of any net are: the pads at the ends that trunks join stand for one piece, a pad at no end stands alone.
    `entry` makes an entry (see build_problem); `taken`: the keys in use.
    """
    a, b, width, pitch, plan = pair
    layers = rules.routing_layers(picture.layers)
    terminal = lambda pad: dict(xy=pad.at, layers=[l for l in layers if l in pad.layers])
    pads = {net: [picture.pads[group[0]] for group in picture.groups[net]] for net in (a, b)}
    name = f"{a} = {b}"
    while any(key == name or key.startswith(name + " ") for key in taken):
        name += "'"  # (a net may be named just so)
    key_of = lambda number: name + (f" ({number + 1})" if len(plan["trunks"]) > 1 else "")
    further = lambda key, trunk: [lots[min(unpaired.get(key, 0), len(lots)):] for lots in trunk["gather"]]
    trunks = [(key_of(number), dict(trunk, gather=further(key_of(number), trunk))) for number, trunk in enumerate(plan["trunks"])]
    trunks = [(key, trunk) for key, trunk in trunks if all(trunk["gather"])]
    entries, said = [], []
    held = list(range(len(plan["ends"])))  # (for each end, the first of the ends that trunks join it to)
    for key, trunk in trunks:
        # (the places of an end by how far off its pads they are: the nearer take their turn first)
        places = [dict(xy=place["xy"], layers=place["layers"], piece=end, **(dict(turn=turn) if turn else {}))
                  for end in (0, 1) for turn, lot in enumerate(trunk["gather"][end]) for place in lot]
        clearance = dict(default=_r(max(rules.clearance(a), rules.clearance(b))), nets={n: _r(c) for net in (a, b) for n, c in rules.stricter(net).items()})
        entries.append(dict(entry(f"{key} trunk", a, places, no_vias=True), net=f"~{key}", width=rules.trunk_width(width, pitch), clearance=clearance))
        stubs = [[], []]
        for end in (0, 1):
            for net, i in zip((a, b), plan["ends"][trunk["ends"][end]]):
                entries.append(dict(entry(f"{key} {net} {end + 1}", net, [terminal(pads[net][i])], last=True), width=width))
                stubs[end].append(entries[-1]["key"])
        said.append(dict(key=key, trunk=f"{key} trunk", legs=[a, b], leg_width=width, pitch=pitch, stubs=stubs))
        first, second = (held[end] for end in trunk["ends"])
        held = [min(first, second) if h in (first, second) else h for h in held]
    for side, net in enumerate((a, b)):
        at_ends = [dict(terminal(pads[net][end[side]]), piece=held[k]) for k, end in enumerate(plan["ends"])]
        if trunks and len(set(held)) + len(plan["loose"][side]) > 1:
            entries.append(entry(net, net, [terminal(pads[net][i]) for i in plan["loose"][side]] + at_ends))
    return entries, said


def problem_of(picture, rules, ask, only=None, unpaired=None, narrow=False, notes=None, **more):
    """build_problem for the unjoined nets of a picture, with what the engine works out for it (`ask` puts the
    questions to it): where each net may be reached (see reach_of), and which nets are laid as pairs (see
    pairs_of; `unpaired`: see pair_entries; none in the narrower second try). `notes`: a list that what is
    worth telling the user is added to."""
    names = picture.unjoined(only)
    pairs = pairs_of(picture, rules, names, ask, notes) if rules.settings.pairs and not narrow else []
    return build_problem(picture, rules, reach_of(picture, rules, names, ask), only=only, narrow=narrow, pairs=pairs, unpaired=unpaired, notes=notes, **more)


def build_problem(picture, rules, reached, only=None, narrow=False, options=None, unlinked=frozenset(), stubs=None, pairs=(), unpaired=None, notes=None,
                  closed=()):
    """The router's problem for the unjoined nets of a picture (those named in `only`, if given).

    `reached`: where each of those nets may be reached (see reach_of). `narrow`: the second try, with
    narrower tracks and vias allowed in a net's own lands. `unlinked`: nets whose capacitors get no link of
    their own to the pin they stand by. `stubs`: instead of the unjoined nets, a track for each of these pads
    from its middle to its zone: [(key, Thermal, points of the zone to end on)]. `pairs`: the nets to lay as
    differential pairs (see pairs_of), `unpaired`: those not all laid as such before (see pair_entries). `notes`: a list
    that what is worth telling the user is added to. `closed`: ground closed to the nets of this problem beyond
    what the board closes, as regions of the engine's.
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
    regions.extend(closed)
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
    note = lambda text: notes.append(text) if notes is not None else None
    narrowed = set()  # nets laid narrower than the current the hints give them takes
    def entry(key, name, terminals, via_penalty=rules.settings.via_penalty, **more):
        width = rules.narrow_width(name, smallest.get(name)) if narrow else rules.track_width(name)
        if rules.intent(name).amps and width < amps_width(rules.intent(name).amps) and name not in narrowed:
            narrowed.add(name)
            note(f"{name} is laid {width:.2f} mm wide to enter its pads, narrower than the {amps_width(rules.intent(name).amps):.2f} mm its current takes")
        lean = rules.leaning(name)  # (what the net is for: see Rules.leaning)
        # the layers the hints give it, and those of any terminal that is on none of them: a pad is where it is
        asked = lean.pop("layers", None)
        within = [l for l in layers if asked is None or l in asked or any(l in t["layers"] and not set(t["layers"]) & set(asked) for t in terminals)]
        return dict(key=key, net=name, width=_r(width), via=[_r(v) for v in rules.via(name)], layers=within or layers,
                    clearance=dict(default=_r(rules.clearance(name)), nets={n: _r(c) for n, c in rules.stricter(name).items()}),
                    terminals=terminals, via_penalty_mm=via_penalty * lean.pop("via_penalty", 1.0), **dict(lean, **more))
    for key, thermal, targets in stubs or []:
        start = dict(xy=picture.pads[thermal.pad].at, layers=[thermal.layer])
        ends = [dict(xy=at, layers=[thermal.layer], piece=0) for at in targets]
        nets.append(entry(key, thermal.net, [start] + ends, rules.settings.via_penalty, no_vias=True))
    on = lambda pad: [l for l in layers if l in pad.layers]
    terminal = lambda pad: dict(xy=pad.at, layers=on(pad))
    paired, problem_pairs = set(), []
    for pair in pairs:
        a, b = pair[:2]
        entries, said = pair_entries(picture, rules, pair, entry, unpaired or {}, set(picture.groups) | {n["key"] for n in nets})
        if said and len(nets) + len(entries) + len(unjoined) >= ENGINE_NETS:
            note(f"{a} = {b} is laid as two nets: there is no room for it among the {ENGINE_NETS - 1} entries of a run")
        elif said and not all(on(picture.pads[group[0]]) for net in (a, b) for group in picture.groups[net]):
            note(f"{a} = {b} is laid as two nets: a pad of it is on no routing layer")
        elif said:
            paired |= {a, b}
            nets += entries
            problem_pairs += said
    for place, name in enumerate(unjoined):
        if name in paired:
            continue
        # One terminal per group of pads that existing copper already joins, or several that stand for the
        # one piece of copper: a group of several pads, or one that a pour joins, is reached where it comes
        # cheapest (see `reach_of`), so that nothing is laid where a pour already joins.
        groups = picture.groups[name]
        pours = picture.pours.get(name, {}) if rules.settings.pours else {}
        reach = reached[name]
        if not all(reach):
            print(f"skipped {name}: a pad of it is on no routing layer", file=sys.stderr)
            continue
        def terminals(g):
            piece = dict(piece=g) if len(reach[g]) > 1 else {}
            return [dict(xy=at, layers=at_layers, **piece) for at, at_layers in reach[g]]
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
        said = rules.intent(name).bus  # (the hints may name its bus, or say it runs with none)
        if said:
            nets[-1]["bus"] = said
        elif name in bus and said is None:
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

    real = [n["net"] for n in nets if n["net"] in picture.classes]
    quiet = rules.quiet_clearance(real)
    problem_rules = dict(hole_centre_mm=_r(rules.hole_spacing(via_drill, via_drill)), pad_drill_margin_mm=0.0, **(dict(quiet_clearance_mm=quiet) if quiet else {}),
                         region_margin_mm=_r(margin), new_clearance_mm=_r(rules.new_clearance(real)))
    if narrow:
        problem_rules["via_in_pad"] = True  # a land enclosed by another pad can only be left through a via in it
    return dict(window=window, step=rules.settings.step, layers=layers, copper=copper, holes=holes, regions=regions,
                **(dict(dear=dear) if dear else {}), **(dict(pairs=problem_pairs) if problem_pairs else {}), rules=problem_rules, nets=nets, options=dict(dict(bus_pull=rules.settings.bus_pull), **(options or {})))


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
    """Put a question to the pcb-routing program (its `ask` command: see src/ask.rs) and return the answer. `engine` is the program (or a command as a list)."""
    command = ([engine] if isinstance(engine, (str, os.PathLike)) else list(engine)) + ["ask"] + (["--threads", str(threads)] if threads else [])
    try:
        run = subprocess.run(command, input=json.dumps(question), capture_output=True, text=True)
    except OSError as error:
        raise EngineFailed(f"cannot run {command[0]}: {error}")
    if run.returncode:
        raise EngineFailed(run.stderr.strip() or f"{command[0]} ask ended with status {run.returncode}")
    try:
        return json.loads(run.stdout)
    except ValueError:
        raise EngineFailed(f"{command[0]} ask gave no answer that can be read")


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
        """Put a question (see src/ask.rs) and return the answer; EngineFailed if it gives none."""
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


def batches(names, most=ENGINE_NETS, size=BATCH_NETS, together=()):
    """The nets in lots the engine accepts: all at once if fewer than `most`, else evenly in lots of at most
    `size`. `together`: pairs of nets that go in one lot (the halves of a differential pair); the second
    joins the first where they would be parted."""
    if len(names) < most:
        return [list(names)] if names else []
    count = math.ceil(len(names) / size)
    each = math.ceil(len(names) / count)
    lots = [list(names[k:k + each]) for k in range(0, len(names), each)]
    lot_of = {name: lot for lot in lots for name in lot}
    for a, b in together:
        if a in lot_of and b in lot_of and lot_of[a] is not lot_of[b]:
            lot_of[b].remove(b)
            lot_of[a].append(b)
            lot_of[b] = lot_of[a]
    return [lot for lot in lots if lot]


def route(board, settings, engine, only=None, options=None, say=print):
    """Join the unjoined nets of a board (those named in `only`, if given).

    `board` reads and writes it: `read(nets)` gives its picture as it now stands (with the groups of joined pads
    of those nets at least), `write(solution, sizes)` adds a solution's copper and returns the numbers of track
    segments and vias added, `unjoined(nets)` names those of the nets whose pads are still not all joined; for
    a board with pours, `take_back(keys)` removes what the last solution written laid for those entries, `mark()` gives the place
    in the record of what has been written and `rewind(mark)` removes all written since (both return the
    numbers of track segments and vias removed).
    `engine` runs problems and answers the bridge's questions (see `Engine`).

    The nets go in lots the engine accepts, and each lot through its tries (see `_Lot`).
    """
    run = _Run(board, settings, engine, options, say)
    outcome, wanted = run.outcome, run.wanted
    picture = board.read(only)
    names = picture.unjoined(only)
    run.open_before = set(board.unjoined())
    starved_before = {(t.pad, t.layer) for t in picture.thermals if t.spokes < t.least}
    laid_before = copper_keys(picture)
    for line in told(Rules.of(picture, settings).intents, set(names)) + unheeded(picture, settings.hints):
        say(line)
    intents = Rules.of(picture, settings).intents
    lots = batches(names, together=[(a, intents[a].pair) for a in names if intents[a].pair and a < intents[a].pair])
    if len(lots) > 1:
        say(f"{len(names)} nets to join, more than the engine takes in one run: routed in {len(lots)} batches of at most "
            f"{max(len(l) for l in lots)}, each against the board as the batch before left it")
    # (a question that gets no answer ends the run as a failed run of the engine does: what has been laid is kept)
    try:
        for number, lot in enumerate(lots):
            if number:
                picture = board.read(lot)
                say(f"batch {number + 1} of {len(lots)}")
            _Lot(run, lot, picture).join()
            if outcome.failure:
                break
            run.given_up = set(board.unjoined(set(wanted))) if wanted else set()
        if settings.pours and not outcome.failure:
            tie_starved(board, settings, engine, outcome, only, starved_before, options, say, laid_before)
    except EngineFailed as failure:
        outcome.failure = str(failure)
    # (a net that was joined and that the new tracks parted for good is one left unjoined, as much as any)
    try:
        wanted += [n for n in board.unjoined() if n not in run.open_before and n not in wanted] if wanted else []
        # the board has the last word: a net is joined when its pads are
        outcome.left = len(board.unjoined(set(wanted))) if wanted else 0
    except EngineFailed as failure:  # (what the zones now join is not known: every net counts as left)
        outcome.failure, outcome.left = outcome.failure or str(failure), len(wanted)
    outcome.wanted = len(wanted)
    outcome.status = "routed" if outcome.left == 0 else run.last if run.last != "routed" else "unroutable"
    return outcome


class _Run:
    """What `route` carries from one lot of nets to the next."""

    def __init__(self, board, settings, engine, options, say):
        self.board, self.settings, self.engine, self.options, self.say = board, settings, engine, options, say
        self.outcome = Outcome()
        self.wanted = []  # the nets that were to be joined, in the order met
        self.last = "routed"  # how the engine's last run of a lot's last try ended
        self.first = True  # no run of the engine has been kept yet
        self.open_before = set()  # nets that are not this run's to join, or were not joined to begin with
        self.given_up = set()  # nets that an earlier lot left unjoined
        self.noted = set()  # what has been said once and is not said again


class _Lot:
    """One lot of nets on its way through its tries (see Rules.tries): each try is one run of the engine for
    what the try before left unjoined, against the board as it then stands. A run can be done over before
    anything of it is written:

    - without the links of capacitors to their pins, for a net that was not all laid with them;
    - with a pair begun further from its pads, and in the end as two nets, where the pair was not all laid;
      and, where something else is left unjoined, once without any pair and without what was only guessed
      of the nets, of which the one that joins more is kept.

    And after it is written, for the zones:

    - nets that the new tracks parted from their zone are joined again, in runs of their own;
    - where that cannot be done past the tracks that did it, everything written for the lot is taken back
      and the lot is laid again with those pads joined by tracks.
    """

    def __init__(self, run, lot, picture):
        self.run, self.lot, self.picture = run, lot, picture
        self.tries = Rules.of(picture, run.settings).tries(lot)
        self.number, self.reread = 0, False  # the try at hand; whether the board is to be read again for it
        # the nets given up as linked; the pairs not all laid as pairs, with how far off their pads they are begun next (see pair_entries)
        self.unlinked, self.unpaired = set(), {}
        # a run with wishes granted (pairs, what the names were taken to ask) kept to be weighed against one
        # without; whether that was done; and whether the wishes are off for the rest of the lot
        self.weighing, self.weighed, self.plain = None, False, False
        # For pours that new tracks cut: how often their nets have been joined again; the last run of the lot's
        # own nets, as (its problem, where the board's record stood before it, its picture); whether the present
        # run is one that joins a cut pour again; the entries that hold cut pours together; whether the lot's
        # run is being done again with those, and how often it has been.
        self.repeats, self.base, self.rejoining, self.bridges, self.again, self.agains = 0, None, False, [], False, 0

    def join(self):
        run = self.run
        while self.number < len(self.tries):
            problem = self.drawn_up()
            if problem is None:
                continue  # (no try at this step)
            # (a net may have several entries; the trunk of a pair is of no net of the board)
            keys = list(dict.fromkeys(n["net"] for n in problem["nets"] if n["net"] in self.picture.groups))
            if not self.number and self.reread is False and not self.unlinked and not self.unpaired:
                run.wanted += [n for n in keys if n not in run.wanted]
            if not keys:
                break
            if self.number and self.reread:
                step = self.tries[self.number][0]
                finer = step != self.tries[self.number - 1][0]
                run.say(f"{'next try, at a %g mm step' % step if finer else 'second try, narrower'}: {', '.join(keys)}")
            try:
                solution = run.engine.route(problem, run.first)
            except EngineFailed as failure:
                run.outcome.failure = str(failure)
                break
            kept = self.without_wishes(problem, solution)
            if kept is None:
                continue  # (done over)
            problem, solution = kept
            run.first = False
            for pair in problem.get("pairs", []):
                run.say(f"laid as a pair: {pair['key']}")
            mark = self.written(problem, solution)
            if solution["status"] == "routed":
                if not self.joined_again(problem, mark):
                    break
            elif not self.held_together(problem, keys):
                # (the next try is for what is left, against the board as it stands: nothing of this one is laid again)
                self.base, self.rejoining, self.bridges, self.again = None, False, [], False
                run.last = solution["status"]
                self.lot = keys
                self.number, self.reread = self.number + 1, True
                self.unpaired, self.weighed = {}, False  # (on a finer grid a pair has its chance again)

    def at_step(self):
        """The settings of the try at hand. (No pairs among nets that are only being joined again after a pour was cut.)"""
        settings = self.run.settings
        return replace(settings, step=self.tries[self.number][0], pairs=settings.pairs and not self.rejoining and not self.plain,
                       lean=settings.lean and not self.plain)

    def drawn_up(self):
        """The problem of the next run; None if the try at hand is passed over (its grid would be too large)."""
        run, (step, narrow) = self.run, self.tries[self.number]
        if self.again:
            self.picture = self.base[2]
            return dict(self.base[0], nets=[n for n in self.base[0]["nets"] if " ~ " not in n["key"]] + self.bridges)
        if self.reread:
            # what could not be joined is tried once more against the board as it now stands
            self.picture = run.board.read(self.lot)
        rules = Rules.of(self.picture, self.at_step())
        most = (run.options or {}).get("max_states")
        if self.number and most and rules.states(self.picture.box, self.picture.layers) > most:
            run.say(f"no try at a {step:g} mm step: the grid would be larger than --max-states")
            self.number, self.reread = self.number + 1, True
            return None
        notes = []
        problem = problem_of(self.picture, rules, run.engine.ask, only=set(self.lot), narrow=narrow, options=run.options,
                             unlinked=self.unlinked, unpaired=self.unpaired, notes=notes)
        for note in notes:
            if note not in run.noted:
                run.say(note)
        run.noted.update(notes)
        return problem

    def without_wishes(self, problem, solution):
        """What of a run is kept, as (problem, solution); None if it is to be done over without a wish that
        was not granted: nothing of it is written then."""
        run, narrow = self.run, self.tries[self.number][1]
        # A link from a capacitor to its pin is a wish, not a need. Where a net with such links is not all
        # laid, the run is done again with that net whole, as it is from then on.
        failed = {n["net"] for n in problem["nets"] if any(m.startswith(n["key"] + ":") for m in solution.get("unrouted", []))}
        failed |= {n["net"] for n in problem["nets"] for c in solution.get("conflicts", []) if c.get("key") == n["key"]}
        linked = {n["net"] for n in problem["nets"] if n["key"].startswith(n["net"] + " | ")} - self.unlinked
        # So is laying two nets as a pair. First the links are given up, and the pairs stay.
        if failed & linked and not self.again:
            run.say(f"not all joined with the capacitors linked to their pins: again without those links for {', '.join(sorted(failed & linked))}")
            self.unlinked |= failed & linked
            self.reread = False
            return None
        # Then a pair that is not all laid is done again begun further from its pads (so near them its stubs
        # may have no room), and as two nets when it has been begun as far off as it is offered; and where
        # something else is left unjoined, the run is done once more without any pair (they take room) and
        # without what was only guessed of the nets (a clock kept off vias takes room too): that run stands
        # if it joins more, for the rest of the lot; else the first.
        said = solution.get("unrouted", []) + [c.get("key", "") + ":" for c in solution.get("conflicts", [])]
        leaned = Rules.of(self.picture, self.at_step()).leans(n["net"] for n in problem["nets"])
        of_pair = lambda pair: [pair["key"], pair["trunk"]] + [stub for end in pair["stubs"] for stub in end]
        broken = {pair["key"] for pair in problem.get("pairs", []) if any(m.startswith(key + ":") for m in said for key in of_pair(pair))}
        if self.weighing is not None:
            if len(said) >= self.weighing[2]:
                run.say("no more is joined without: they are kept")
                (problem, solution, _), self.plain = self.weighing, False
            self.weighing, broken = None, set()
        elif broken:
            run.say(f"not all laid as pairs: again, begun further from their pads or as single nets, for {', '.join(sorted(broken))}")
        elif said and not self.weighed and not self.again and (problem.get("pairs") or leaned):
            wishes = (["pairs"] if problem.get("pairs") else []) + (["what the nets' names were taken to ask"] if leaned else [])
            run.say(f"not all joined: once more without {' and '.join(wishes)}")
            self.weighing, self.weighed, self.plain = (problem, solution, len(said)), True, True
            self.reread = False
            return None
        if not broken:
            return problem, solution
        self.unpaired.update({key: self.unpaired.get(key, 0) + 1 for key in broken})
        if self.again:
            # (a lot that is being laid again to hold a cut pour together: its problem is drawn up anew)
            base, mark, picture = self.base
            of_lot = {n["net"] for n in base["nets"] if n["net"] in picture.groups}
            anew = problem_of(picture, Rules.of(picture, self.at_step()), run.engine.ask, only=of_lot, narrow=narrow, options=run.options,
                              unlinked=self.unlinked, unpaired=self.unpaired)
            self.base = (anew, mark, picture)
        self.reread = False
        return None

    def written(self, problem, solution):
        """Write what a run laid; returns where the board's record stood before it. A track that ends on a
        pour is joined only while the pour is there: where the other new tracks have cut it away, the copper
        that hangs on that end is taken back, and its net is among those still to join."""
        run, board = self.run, self.run.board
        mark = board.mark() if run.settings.pours else None
        # (an entry that has points of a pour among its terminals is written piece by piece)
        parts = parts_of(self.picture, problem, solution, run.engine.ask) if run.settings.pours else [dict(laid, of=laid["key"]) for laid in solution["nets"]]
        sizes = {n["key"]: (n["width"], n["via"]) for n in problem["nets"]}
        # (what is laid for a pair comes back per leg, under the pair's key and the leg's net: the leg and its stubs)
        sizes.update({f"{pair['key']}/{leg}": sizes[stub] for pair in problem.get("pairs", []) for leg, stub in zip(pair["legs"], pair["stubs"][0])})
        added = board.write(dict(solution, nets=parts), {part["key"]: sizes[part["of"]] for part in parts})
        ends = [(part["key"], end) for part in parts for end in part.get("ends", [])]
        standing = on_zone(board.read(), [end for _, end in ends], run.engine.ask) if ends else []
        loose = sorted({key for (key, _), stands in zip(ends, standing) if not stands})
        if loose:
            taken = board.take_back(loose)
            added = (added[0] - taken[0], added[1] - taken[1])
            nets_of = {part["key"]: part["net"] for part in parts}
            run.say(f"ended on a pour that the other new tracks cut away, taken back: {', '.join(sorted({nets_of[key] for key in loose}))}")
        run.outcome.tracks, run.outcome.vias = run.outcome.tracks + added[0], run.outcome.vias + added[1]
        return mark

    def joined_again(self, problem, mark):
        """After a run that laid all it was given: True if another is to follow, for nets the new tracks have
        parted (they may have cut a pour and parted pads it joined: those nets are joined again, against the
        board as it now stands); False if the lot is done."""
        run = self.run
        # (any net of this lot so far, or one that was joined before the run began)
        mine = lambda n: (n in run.wanted or n not in run.open_before) and n not in run.given_up
        cut = [n for n in run.board.unjoined() if mine(n)] if self.repeats < POUR_REPEATS else []
        if not cut:
            return False
        if not self.rejoining:
            self.base = (problem, mark, self.picture)
        run.say(f"parted by the new tracks, joined again: {', '.join(cut)}")
        run.wanted += [n for n in cut if n not in run.wanted]
        self.lot, self.reread, self.repeats, self.rejoining, self.again = cut, True, self.repeats + 1, True, False
        return True

    def held_together(self, problem, keys):
        """After a run that left something unjoined: True if the lot is to be laid again from the start to hold
        a cut pour together.

        A cut pour could not be joined again past the tracks that cut it (they may have shut a piece of it
        in). Everything laid since is taken back, and the lot is laid again with the pads of the pieces as
        they were cut to be joined by a track, pad to pad: the others then leave the way for it. (Not from a
        point of the pour to another: on the pour whole again those are a step apart, and a track between
        them holds nothing together.)"""
        run, narrow = self.run, self.tries[self.number][1]
        room = self.base is not None and len(self.base[0]["nets"]) + len(self.bridges) + len(problem["nets"]) < ENGINE_NETS
        if not (self.rejoining and room and run.settings.pours and self.agains < POUR_AGAINS):
            return False
        taken = run.board.rewind(self.base[1])
        run.outcome.tracks, run.outcome.vias = run.outcome.tracks - taken[0], run.outcome.vias - taken[1]
        by_pads = problem_of(self.picture, Rules.of(self.picture, replace(self.at_step(), pours=False, pairs=False)), run.engine.ask,
                             only=set(keys), narrow=narrow, options=run.options, unlinked=set(keys))
        self.bridges += [dict(entry, key=f"{entry['net']} ~ {len(self.bridges) + k + 1}") for k, entry in enumerate(by_pads["nets"])]
        run.say(f"no way to join them past the new tracks: those are laid again, with the pieces of the pour held together: {', '.join(keys)}")
        self.lot = list(dict.fromkeys(n["net"] for n in self.base[0]["nets"]))
        self.reread, self.rejoining, self.again, self.agains = False, False, True, self.agains + 1
        return True


def tie_starved(board, settings, engine, outcome, only=None, before=frozenset(), options=None, say=print, laid_before=None):
    """See to every pad that is tied to its zone by fewer spokes than the board asks for. Tracks of other nets
    across its spokes are what starve a pad: one that this run starved is seen to whatever its net; one that
    the board came with (`before`: those, as (pad, layer)) as well, if its net is among those named in `only`
    or none are named.

    A pad gets a straight track from its middle into the zone, where one fits: that is a tie as good as a
    spoke, and counted as one (see `tied_by_tracks`). Where none fits, the nets whose new tracks pass by it
    are laid again clear of it (see `cleared`; `laid_before`: the tracks and vias the board came with, see
    `copper_keys`), and the ties are tried again. What was done and what could not be is noted in `outcome`.
    """
    name = lambda picture, thermal: f"{picture.pads[thermal.pad].part or '?'} ({thermal.net}, {thermal.layer})"
    ours = lambda t: (t.pad, t.layer) not in before or only is None or t.net in only
    tried = collections.defaultdict(list)  # (pad, layer) -> the points of its zone already tried
    cleared_for = set()  # the pads that nets have been laid again for
    for again in range(CLEARINGS + 1):
        picture, starved = tied_by_tracks(board, settings, engine, outcome, ours, tried, options, say)
        # (a pad the board came with starved is not worth other nets' tracks: only what this run did is undone)
        new = [t for t in starved if (t.pad, t.layer) not in before and (t.pad, t.layer) not in cleared_for]
        if not new or again == CLEARINGS or laid_before is None:
            break
        cleared_for.update((t.pad, t.layer) for t in new)
        try:
            if not cleared(board, settings, engine, outcome, picture, new, laid_before, options, say):
                break
        except EngineFailed as failure:
            say(f"nothing laid again for the pads with too few spokes: {failure}")
            break
        tried.clear()
    outcome.starved = sorted(name(picture, thermal) for thermal in starved)
    if outcome.tied:
        say(f"{outcome.tied} pads with too few spokes to their zone tied to it by a track of their own")
    if outcome.starved:
        say(f"too few spokes to their zone, and no room for a track to it: {', '.join(outcome.starved)}")


CLEARINGS = 2  # how often nets are laid again clear of pads their tracks have starved


def tied_by_tracks(board, settings, engine, outcome, ours, tried, options, say):
    """Give the pads with too few spokes (those `ours` holds for) tracks to their zones, in rounds: returns
    the board's picture after them and the pads still starved. `tried`: (pad, layer) -> the points of its
    zone already tried, which is added to."""
    routed_on = lambda picture: Rules.of(picture, settings).routing_layers(picture.layers)
    starved_of = lambda picture: [t for t in picture.thermals if t.spokes < t.least and ours(t) and t.layer in routed_on(picture)]
    picture, starved = None, []
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
    return picture, starved


def copper_keys(picture):
    """The tracks and vias of a picture as things to tell them apart by: what is on the board later and not
    among these has been laid since."""
    return {(t.net, t.layer, tuple(t.a), tuple(t.b)) for t in picture.tracks} | {(v.net, None, tuple(v.at), tuple(v.at)) for v in picture.vias}


def cleared(board, settings, engine, outcome, picture, starved, laid_before, options, say):
    """Lay again, clear of pads that have too few spokes and no room for a track to their zone, the nets whose
    new tracks pass by them: True if that was done.

    A track of another net close by a pad takes the zone away from the pad's spokes, and one that goes round
    the pad leaves no way out for a track of the pad's own. The nets that this run laid within reach of such
    a pad (the pad, the gap, the clearance the zone keeps and a track's width) are taken out and laid again
    with that ground closed to them. If they cannot all be joined so, or the way they then take parts another
    net (from its zone), they are put back as they were.
    `board.take_out(nets)` removes what has been written for those nets and `board.put_back()` restores it
    (both return the numbers of track segments and vias).
    """
    part = lambda thermal: picture.pads[thermal.pad].part or "?"
    reach = {}  # (pad, layer) -> (its middle, how far from it a track of another net is too near)
    for thermal in starved:
        at = picture.pads[thermal.pad].at
        beyond = max((math.dist(at, p) for p in thermal.outline), default=0.0)  # (the outline is half the gap from the pad)
        reach[thermal.pad, thermal.layer] = (at, beyond + thermal.gap / 2 + thermal.clearance + KEEP_OFF)
    def near(net, layer, a, b, half):
        for thermal in starved:
            at, far = reach[thermal.pad, thermal.layer]
            vx, vy = b[0] - a[0], b[1] - a[1]
            l2 = vx * vx + vy * vy
            t = max(0.0, min(1.0, ((at[0] - a[0]) * vx + (at[1] - a[1]) * vy) / l2)) if l2 else 0.0
            if net != thermal.net and layer in (None, thermal.layer) and math.dist(at, (a[0] + t * vx, a[1] + t * vy)) < far + half:
                return True
        return False
    new = lambda key: key not in laid_before
    culprits = {t.net for t in picture.tracks if t.net and new((t.net, t.layer, tuple(t.a), tuple(t.b))) and near(t.net, t.layer, t.a, t.b, t.width / 2)}
    culprits |= {v.net for v in picture.vias if v.net and new((v.net, None, tuple(v.at), tuple(v.at))) and near(v.net, None, v.at, v.at, v.width / 2)}
    culprits = sorted(culprits)
    if not culprits or len(culprits) >= ENGINE_NETS:
        return False
    open_before = set(board.unjoined())
    mark = board.mark()
    taken = board.take_out(culprits)
    after = board.read()
    # (as single nets, without the links of capacitors: this is a repair)
    rules = Rules.of(after, replace(settings, pairs=False, decoupling=False))
    rules = Rules.of(after, replace(rules.settings, step=rules.step(set(culprits))))
    closed = [dict(rings=[_disc(at, far)], layers=[layer], tracks=True, vias=True) for (_, layer), (at, far) in reach.items()]
    problem = problem_of(after, rules, engine.ask, only=set(culprits), options=options, closed=closed)
    solution = engine.route(problem) if problem["nets"] else dict(status="routed", nets=[])
    done = solution["status"] == "routed"
    if done:
        added = board.write(solution, {n["key"]: (n["width"], n["via"]) for n in problem["nets"]})
        # (all joined, and no other net parted by the new way they take: it may cut a zone that the old did not)
        done = set(board.unjoined()) <= open_before
    if not done:
        board.rewind(mark)
        board.put_back()
        say(f"no way clear of {', '.join(sorted({part(t) for t in starved}))} for {', '.join(culprits)}: left as they were")
        return False
    outcome.tracks, outcome.vias = outcome.tracks - taken[0] + added[0], outcome.vias - taken[1] + added[1]
    say(f"laid again clear of {', '.join(sorted({part(t) for t in starved}))}, whose spokes they cut: {', '.join(culprits)}")
    return True


KEEP_OFF = 0.3  # mm beyond the clearance a zone keeps: what is left of a zone nearer a track than this is no spoke


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
        self.out = []  # what was taken out of the board to be laid again, till it is known that it need not be put back

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

    def take_out(self, nets):
        tracks = vias = 0
        self.out = []
        for record in self.written:
            for key, items in record.items():
                for item in [item for item in items if item.GetNetname() in nets]:
                    via = item.Type() == pcbnew.PCB_VIA_T
                    self.board.Remove(item)
                    items.remove(item)
                    self.out.append(item)
                    tracks, vias = tracks + (not via), vias + via
        self.stale = True
        return tracks, vias

    def put_back(self):
        tracks = vias = 0
        for item in self.out:
            via = item.Type() == pcbnew.PCB_VIA_T
            self.board.Add(item)
            tracks, vias = tracks + (not via), vias + via
        self.written.append({"put back": self.out})
        self.out, self.stale, self.last = [], True, {}
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
    parser.add_argument("--no-pairs", action="store_true", help="do not lay two nets named as the halves of a differential pair (_P/_N, +/-, DP/DM) side by side")
    parser.add_argument("--hints", help="what the nets are for, as a JSON file (default: BOARD.routing.json beside the board, if it is there; see read_hints)")
    parser.add_argument("--no-guesses", action="store_true", help="do not guess what a net is for from its name and parts (clocks, crystals, switch nodes, pairs): only the hints say")
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
    beside = Path(args.board).with_suffix(".routing.json")
    try:
        hints = read_hints(args.hints) if args.hints else read_hints(beside) if beside.is_file() else None
    except ValueError as error:
        parser.error(str(error))
    settings = Settings(hints=hints, guesses=not args.no_guesses, step=step, coarse_first=args.coarse_first, slack=args.slack, via_penalty=args.via_penalty, layers=names(args.layers), bus_pull=args.bus_pull, decoupling=not args.no_decoupling, pours=not args.no_pours, pairs=not args.no_pairs,
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
