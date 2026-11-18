"""What the nets are for: guessed from their names and parts, and said in a hints file beside the board
(BOARD.routing.json). A guess only leans a net (what it costs, how readily it gives way); what binds it (a width, a
clearance, layers, no vias, a distance kept from other nets) comes from the hints alone. Nothing here touches KiCad."""

import fnmatch, json, math
from dataclasses import dataclass, field
from pathlib import Path

from .names import CRYSTAL_PART, GROUND, NAMED, is_supply, pair_names

ROLES = ("supply", "ground", "clock", "crystal", "sensitive", "noisy")
# How a role leans a net: (how readily it gives way, its weight among results, its via penalty as a factor).
# A role the hints give leans it all the way. One that is only guessed makes it slower to take a via and no
# more: on 20 public boards, guessed nets that held their way cost the other nets 13 % more vias.
LEANING = dict(clock=(0.5, 2.0, 2.0), crystal=(0.2, 3.0, 5.0), noisy=(0.3, 3.0, 1.0), sensitive=(0.5, 2.0, 1.0))
GUESSED = dict(clock=(1.0, 1.0, 2.0), crystal=(1.0, 1.0, 5.0))
HINTS = dict(role=str, width=float, clearance=float, amps=float, volts=float, max_mm=float, vias=str, layers=list,
             pair=(str, bool), gap=float, bus=(str, bool), priority=str, ohms=float, match=str)
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
    parts: list = field(default_factory=list)  # [(pattern, {turn: bool, near: [x, y], mate: reference})]
    boards: list = field(default_factory=list)  # the boards of the file, left to right: [{shift: [x, y], clear: mm}]
    heights: dict = field(default_factory=dict)  # the beginning of a footprint's name -> how high it stands, mm
    keepouts: list = field(default_factory=list)  # ground where no part may stand: [[x0, y0, x1, y1]]


def read_hints(path):
    """A hints file (JSON) as Hints; ValueError says what is wrong with it.

        {"nets": {"/adc/*": {"role": "sensitive"}, "+24V": {"amps": 3}, "CLK8": {"max_mm": 40, "vias": "none"}},
         "pairs": [{"nets": ["TX_P", "TX_N"], "width": 0.15, "gap": 0.18}],
         "parts": {"J*": {"turn": false}}}

    A pattern is a net's name, with * and ? for any run of characters and any one (a name is tried with and
    without its leading /). Later patterns override earlier ones. `parts`: a pattern for the references of
    parts, with whether such a part may be turned where it stands (for advice only: see advice.py) and a
    point it is to stand near, or the part it mates with (the other half of a link from board to board). `boards` and `heights` are for the floorplan (see floorplan.py): how the boards
    of the file stack, how high parts stand, and ground where no part may stand (an opening to be cut, what must
    be reached from above) besides the board's own rule areas.
    """
    name = Path(path).name
    try:
        data = json.loads(Path(path).read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"{name}: {error}")
    wrong = lambda where, why: ValueError(f"{name}: {where}: {why}")
    number = lambda value: isinstance(value, (int, float)) and not isinstance(value, bool) and value > 0
    if not isinstance(data, dict) or set(data) - {"nets", "pairs", "parts", "boards", "heights", "keepouts"}:
        raise wrong("the file", "an object with `nets`, `pairs`, `parts`, `boards`, `heights` and `keepouts`, nothing else")
    if not (isinstance(data.get("nets", {}), dict) and isinstance(data.get("pairs", []), list)
            and isinstance(data.get("parts", {}), dict)):
        raise wrong("the file", "`nets` is an object (a pattern, and what is said of the nets it names), "
                                "`pairs` a list, `parts` an object (a pattern, and what is said of those parts)")
    hints = Hints(name=name)
    point = lambda value: (isinstance(value, list) and len(value) == 2
                           and all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in value))
    for pattern, said in data.get("parts", {}).items():
        if (not isinstance(said, dict) or set(said) - {"turn", "near", "mate"} or not isinstance(said.get("turn", True), bool)
                or not ("near" not in said or point(said["near"])) or not isinstance(said.get("mate", ""), str)):
            raise wrong(f"parts[{pattern}]", "`turn`: true or false; `near`: a point [x, y]; `mate`: the reference "
                                             "of the part it mates with; nothing else")
        hints.parts.append((pattern, dict(said)))
    boards = data.get("boards", [])
    if not isinstance(boards, list) or not all(
            isinstance(b, dict) and not set(b) - {"shift", "clear"} and ("shift" not in b or point(b["shift"]))
            and ("clear" not in b or number(b["clear"])) for b in boards):
        raise wrong("boards", "a list, one for each board from left to right, of `shift`: [x, y] and `clear`: mm")
    hints.boards = [dict(b) for b in boards]
    heights = data.get("heights", {})
    if not isinstance(heights, dict) or not all(number(v) for v in heights.values()):
        raise wrong("heights", "the beginning of a footprint's name, and how high such a part stands in mm")
    hints.heights = dict(heights)
    keepouts = data.get("keepouts", [])
    if not isinstance(keepouts, list) or not all(
            isinstance(k, list) and len(k) == 4 and all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in k)
            and k[0] < k[2] and k[1] < k[3] for k in keepouts):
        raise wrong("keepouts", "a list of grounds where no part may stand, each [x0, y0, x1, y1]")
    hints.keepouts = [list(k) for k in keepouts]
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
        if not (isinstance(legs, list) and len(legs) == 2 and all(isinstance(leg, str) for leg in legs)
                and legs[0] != legs[1]):
            raise wrong(f"pairs[{k}].nets", "the two nets of the pair")
        if set(pair) - {"nets", "width", "gap",
                        "ohms"} or not all(number(pair[w]) and pair[w] <= MOST.get(w, math.inf) for w in pair
                                           if w != "nets"):
            raise wrong(f"pairs[{k}]", "`nets`, and `width`, `gap`, `ohms` as numbers more than 0 (20 mm at the most)")
        hints.pairs.append(dict(pair))
    return hints


def unheeded(picture, hints):
    """What a hints file says that nothing on the board answers to, as lines to say: a pattern that names no
    net, a pair with a net that is not there, a layer the board has not."""
    nets = sorted(picture.groups)
    named = lambda pattern: [net for net in nets
                             if fnmatch.fnmatchcase(net, pattern) or fnmatch.fnmatchcase(net.lstrip("/"), pattern)]
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
    parts = {pad.part for pad in picture.pads if pad.part}
    for pattern, _ in (hints.parts if hints else []):
        if not any(fnmatch.fnmatchcase(part, pattern) for part in parts):
            lines.append(f"{hints.name}: {pattern} names no part of the board")
    for pair in (hints.pairs if hints else []):
        for leg in pair["nets"]:
            if leg not in nets:
                lines.append(f"{hints.name}: the pair {' = '.join(pair['nets'])} names {leg}, "
                             "which is no net of the board")
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
                said = " ".join(value) if isinstance(value, list) else value
                text = f"{net} {what} {said}" + (" (not acted on)" if what in NOT_ACTED_ON else "")
            lines.setdefault(source, []).append(text)
    return [f"from {'the net names and parts' if source == 'names' else source}: {'; '.join(found)}"
            for source, found in sorted(lines.items())]
