"""The picture: a board as plain data. Lengths in mm, points [x, y], layers by name. A polygon is a list of rings (the
outline first, then its holes); a ring is a list of points."""

from dataclasses import dataclass, field


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
    # outer copper layer -> how far its mask opening reaches beyond its outline
    openings: dict = field(default_factory=dict)
    hole_at: object = None  # the centre of its drilled hole
    drill: object = None  # the hole's diameter (the longer side of a slot); None: no hole
    clearance: object = None  # its own clearance, where it or its footprint overrides the class
    smallest: object = None  # its smallest dimension on any copper layer
    part: object = None  # the reference of the part it belongs to


@dataclass
class Part:
    """A part of the board, as far as a plan of the work needs it."""
    at: list  # where it stands
    sheet: str = ""  # the name of the schematic sheet it is drawn on
    file: str = ""  # that sheet's file
    value: str = ""
    footprint: str = ""


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
    # for a pad with too few: the ring a spoke crosses, half the gap from the pad
    outline: list = field(default_factory=list)
    # and the pieces of the zone it is tied to, as indices into the picture's fills
    fill: list = field(default_factory=list)
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
    parts: dict = field(default_factory=dict)  # reference -> Part
    groups: dict = field(default_factory=dict)  # net -> its groups of pads already joined, as indices into pads
    # net -> {index of a group: the pours that join it, as indices into fills}
    pours: dict = field(default_factory=dict)
    fills: list = field(default_factory=list)  # every piece of every filled zone, whatever it joins: [Pour]
    thermals: list = field(default_factory=list)  # every pad tied to a filled zone by spokes: [Thermal]
    # What its nets are for, once worked out (rules.py keeps it here, by the hints and whether guesses are made).
    intents: dict = field(default_factory=dict, repr=False, compare=False)

    @property
    def holes(self):
        """Every drilled hole, whatever it belongs to."""
        return ([Hole(p.hole_at, p.drill, p.net, p.clearance) for p in self.pads if p.drill] +
                [Hole(v.at, v.drill, v.net) for v in self.vias if v.drill])

    def unjoined(self, only=None):
        """The nets whose pads are not all joined, by name."""
        return sorted(n for n, groups in self.groups.items() if len(groups) > 1 and (only is None or n in only))

    def copper_keys(self):
        """Its tracks and vias as things to tell them apart by: what is on the board later and not among these
        has been laid since."""
        tracks = {(t.net, t.layer, tuple(t.a), tuple(t.b)) for t in self.tracks}
        return tracks | {(v.net, None, tuple(v.at), tuple(v.at)) for v in self.vias}
