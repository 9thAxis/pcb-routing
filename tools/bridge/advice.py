"""Advice on where the parts stand: what turning one of them would make easier to lay. Said, never done: the
bridge moves no part."""

import fnmatch
import math

from .names import FACING_PART
from .questions import facing

IN_LINE = 0.5  # the pads of an end stand across the pair's way if the sine of the angle between is more


def stays(picture, hints, part, guesses=True):
    """Whether a part is to stay as it stands: as the hints say of it (`parts`, `turn`: the last pattern that
    names it); else, if guesses are made, a part that by its reference is a connector or a switch: it faces
    whoever uses the board."""
    said = [what["turn"] for pattern, what in (hints.parts if hints else [])
            if "turn" in what and fnmatch.fnmatchcase(part or "", pattern)]
    return not said[-1] if said else bool(guesses and FACING_PART.match(part or ""))


def changed_over(picture, pair):
    """The ends of a pair (see pairs_of) whose two pads stand changed over against the others of their line:
    {end: the ends it is changed over against}. Along a line of ends (a trunk's two, and those it passes), the
    first net's pad is on the left at some and on the right at others: the fewer are the ones changed over,
    and with as many of each, all are. An end whose pads stand in line with the pair's way is on neither side."""
    a, b, _, _, plan = pair
    faced = facing(picture, a, b)
    pads = lambda end: [faced[net][i] for net, i in zip((a, b), plan["ends"][end])]
    middle = lambda end: [(p + q) / 2 for p, q in zip(*(pad.at for pad in pads(end)))]
    found = {}
    for trunk in plan.get("through") or plan["trunks"]:
        line = [trunk["ends"][0]] + [passed["end"] for passed in trunk.get("passes", [])] + [trunk["ends"][1]]
        sides = {}
        for k, end in enumerate(line):
            before, after = middle(line[max(k - 1, 0)]), middle(line[min(k + 1, len(line) - 1)])
            first, second = (pad.at for pad in pads(end))
            way, across = math.dist(before, after), math.dist(first, second)
            if way < 1e-9 or across < 1e-9:
                continue
            side = ((after[0] - before[0]) * (first[1] - second[1])
                    - (after[1] - before[1]) * (first[0] - second[0])) / (way * across)
            if abs(side) > IN_LINE:
                sides[end] = side > 0
        left, right = ([end for end in sides if sides[end] == on_left] for on_left in (True, False))
        if left and right:
            for few, many in ((left, right), (right, left)):
                if len(few) <= len(many):
                    found.update({end: many for end in few})
    return found


def done(picture, two):
    """What is to be done to the part, or the two parts, of the two pads of an end to change them over."""
    first, second = two
    parts = sorted({first.part, second.part} - {None})
    if len(parts) == 2:
        return f"{parts[0]} and {parts[1]} in each other's place"
    # (turned by 180 degrees a part's two pads change places where they stand only if they stand either
    # side of its middle; else they come to lie on its other side)
    mine = [pad.at for pad in picture.pads if pad.part == parts[0]]
    centre = [sum(at[k] for at in mine) / len(mine) for k in (0, 1)]
    between = [(first.at[k] + second.at[k]) / 2 for k in (0, 1)]
    if math.dist(centre, between) <= 0.25 * math.dist(first.at, second.at) + 0.1:
        return f"{parts[0]} turned by 180 degrees"
    return (f"those two pins of {parts[0]} swapped in the schematic, if the part allows (turned by 180 degrees, "
            "the part has them on its other side)")


def pair_advice(picture, hints, pair, guesses=True):
    """What to say of one pair: for each line of its ends where some stand changed over against the others
    (the pair then goes round an end, or crosses over on another layer), the part that would have to be turned.
    Of several that would do, those that may be turned (see stays); nothing where none may."""
    a, b, _, _, plan = pair
    faced = facing(picture, a, b)
    pads = lambda end: [faced[net][i] for net, i in zip((a, b), plan["ends"][end])]
    over = changed_over(picture, pair)
    lines, said = [], set()
    for end in sorted(over):
        first, second = pads(end)
        parts = sorted({first.part, second.part} - {None})
        against = sorted({pad.part for other in over[end] for pad in pads(other)} - {None})
        if not parts or any(stays(picture, hints, part, guesses) for part in parts) or tuple(parts) in said:
            continue
        # (said once of two ends that are each changed over against the other: either would do)
        either = [other for other in over[end] if end in over.get(other, [])]
        others = sorted({pad.part for other in either for pad in pads(other)} - {None})
        free = [part for part in others if not stays(picture, hints, part, guesses)]
        said.update({tuple(parts), tuple(free)})
        where = f"at {' and '.join(parts)} than at {', '.join(against)}" if against else f"at {' and '.join(parts)}"
        what = f"{a} = {b}: its two nets are the other way round {where}, so the pair goes round an end or crosses over. "
        cures = [done(picture, pads(end))] + [done(picture, pads(other)) for other in either
                                              if not any(stays(picture, hints, pad.part, guesses) for pad in pads(other))]
        lines.append(f"{what}With {'; or with '.join(dict.fromkeys(cures))}, it would run straight on.")
    return lines
