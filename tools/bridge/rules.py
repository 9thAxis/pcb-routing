"""The rules: what the board asks of the router, each stated once. Nothing here touches KiCad."""

import math
from dataclasses import dataclass

from .lengths import r6
from .intent import GUESSED, Intent, LEANING, intents_of


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


def amps_width(amps):
    """The width of an outer track of 35 um copper that carries a current with a rise of 10 degrees, in mm
    (IPC-2221: I = 0.048 dT^0.44 A^0.725, A in square mils)."""
    area = (amps / (0.048 * 10 ** 0.44)) ** (1 / 0.725)
    return r6(area / 1.378 * 0.0254)


def volts_clearance(volts):
    """The distance between bare outer conductors for a voltage, in mm (IPC-2221B table 6-1, column B2)."""
    for most, distance in ((30, 0.1), (150, 0.6), (300, 1.25), (500, 2.5)):
        if volts <= most:
            return distance
    return r6(2.5 + (volts - 500) * 0.005)


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
        key = (id(settings.hints), settings.guesses)  # (worked out once for a picture, hints and guesses)
        if key not in picture.intents:
            picture.intents[key] = intents_of(picture, settings.hints, settings.guesses)
        return cls(picture.minima, picture.classes, picture.ignored, settings, picture.intents[key])

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
        return r6(width), r6(width + gap)

    def quiet_clearance(self, nets):
        """What a net the hints call sensitive keeps from the other nets laid with it, and they from it: twice
        what two ordinary nets keep. None if none of these nets is sensitive by the hints."""
        quiet = any(self.leaning(net).get("quiet") for net in nets)
        return r6(2 * self.new_clearance(nets)) if quiet else None

    def trunk_width(self, width, pitch):
        """How wide the trunk of a pair is laid: its two legs side by side, and what a leg takes on the
        outside of a turn of 45 degrees (so that such a corner is a corner), 0.04 mm at the least."""
        return r6(max(pitch + width + 0.04, pitch / math.cos(math.pi / 8) + width + 0.005))

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
        leans = {"yield": give, "weight": weight, "via_penalty": vias}
        out.update({field: factor for field, factor in leans.items() if factor != 1.0})
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
        return r6(self.track_width(net) / 2 + 0.1)

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
