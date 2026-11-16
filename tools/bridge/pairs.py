"""Differential pairs: which nets are laid as one, and the entries of the problem that lay them so. Where the pads
of two nets face each other is the engine's to say (src/pairs.rs)."""

import math

from .lengths import disc, r6
from .questions import pair_plans

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
    sizes = [rules.pair(a, b, names) for a, b in named]
    plans = pair_plans(picture, rules, [(a, b, pitch) for (a, b), (_, pitch) in zip(named, sizes)], ask)
    for (a, b), plan in zip(named, plans):
        if not plan["trunks"] and not plan.get("through"):
            few = len(plan["ends"]) < 2
            note(a, b, "no two places where a pad of each stands by the other" if few
                 else "its ends are too near each other, or share no routing layer")
    return [(a, b, width, pitch, plan) for (a, b), (width, pitch), plan in zip(named, sizes, plans)
            if plan["trunks"] or plan.get("through")]


def room_between(first, second, layers):
    """How much ground the copper of two pads leaves between them on these layers, along the line from the
    one to the other."""
    apart = math.dist(first.at, second.at)
    if apart < 1e-9:
        return 0.0
    way = [(second.at[k] - first.at[k]) / apart for k in (0, 1)]
    def reach(pad, towards):
        corners = [point for layer in layers for polygon in pad.shapes.get(layer, []) for point in polygon[0]]
        return max((towards * ((x - pad.at[0]) * way[0] + (y - pad.at[1]) * way[1]) for x, y in corners), default=0.0)
    return apart - reach(first, 1) - reach(second, -1)


def pad_near(picture, at, layers, within, but):
    """Whether the copper of a pad other than those of `but` comes nearer a point than `within` on these layers."""
    def near(a, b):
        along = [b[0] - a[0], b[1] - a[1]]
        length = along[0] ** 2 + along[1] ** 2
        t = 0.0 if length < 1e-18 else min(1.0, max(0.0, ((at[0] - a[0]) * along[0] + (at[1] - a[1]) * along[1]) / length))
        return math.dist(at, [a[0] + t * along[0], a[1] + t * along[1]]) < within
    for pad in picture.pads:
        if any(pad is other for other in but) or not set(layers) & set(pad.layers):
            continue
        outlines = [polygon[0] for layer in layers for polygon in pad.shapes.get(layer, [])]
        if math.dist(pad.at, at) < within or any(near(a, b) for ring in outlines for a, b in zip(ring, ring[1:] + ring[:1])):
            return True
    return False


def stretches_of(picture, rules, pair, unpaired, name):
    """The trunks one pair is laid with, as [(key, trunk)], the trunks as the engine plans them (src/pairs.rs).

    A pair with a part in its middle runs on between the two pads of that part where they leave its trunk
    room and no other pad stands in it (the plan's `through`), keyed `NAME (through)`: one trunk from its
    first end to its last. That has
    one try: not all laid so, the pair is laid stretch by stretch (the plan's `trunks`), each from an end to
    the next and only where the two are far enough apart.
    """
    a, b, width, pitch, plan = pair
    plain = [(name + (f" ({number + 1})" if len(plan["trunks"]) > 1 else ""), trunk)
             for number, trunk in enumerate(plan["trunks"])]
    passing = [trunk for trunk in plan.get("through", []) if trunk.get("passes")]
    numbered = lambda trunk: f" {passing.index(trunk) + 1}" if len(passing) > 1 else ""
    key_of = lambda trunk: (f"{name} (through{numbered(trunk)})" if trunk.get("passes")
                            else next(key for key, other in plain if other["ends"] == trunk["ends"]))
    pads = lambda end: [picture.pads[picture.groups[net][i][0]] for net, i in zip((a, b), plan["ends"][end])]
    # (the trunk with its clearance on either side, and a step for the grid it is laid on)
    wide = (rules.trunk_width(width, pitch) + 2 * max(rules.clearance(a), rules.clearance(b))
            + 2 * rules.settings.step)
    room = lambda passed: (room_between(*pads(passed["end"]), passed["place"]["layers"]) >= wide and not
                           pad_near(picture, passed["place"]["xy"], passed["place"]["layers"], wide / 2, pads(passed["end"])))
    fits = all(room(passed) for trunk in passing for passed in trunk["passes"])
    if passing and fits and not any(unpaired.get(key_of(trunk)) for trunk in passing):
        return [(key_of(trunk), trunk) for trunk in plan["through"]]
    return plain


OFF_LAYER = 1.5  # what a trunk pays, as a factor, on a layer the tighter of its two ends is not on
FREE = 2  # the try at which a stretch is left free of hands and headings; after it, it is laid as two nets


def pair_entries(picture, rules, pair, entry, unpaired, taken, names):
    """The entries for one differential pair (see pairs_of) and what the problem's `pairs` says of it:
    (entries, pairs); nothing if every stretch of it has been given up as a pair. `unpaired`: for the key of
    a stretch that was not all laid as a pair, how many of the nearest lots of places its trunk is no longer
    offered (it is begun further from its pads; with none left it is given up).

    For each stretch between two of its ends, a trunk: laid as one wide track of a net of its own between
    places a little off the pads (those of one end are one piece: the engine takes the nearest that are open), and four
    stubs that join the legs it is split into to the pads. What the trunks do not join is joined as the pads
    of any net are: the pads at the ends that trunks join stand for one piece, a pad at no end stands alone.
    `entry` makes an entry (see build_problem); `taken`: the keys in use; `names`: the nets of the run.

    A trunk may change layer: through one via as wide as two of the legs' side by side, which the engine makes
    into those two (see Rules.pair_vias, and `via_pitch` in src/model.rs). And it may run on past a part in
    the middle of the pair (see stretches_of): it is led between the two pads of that part, each of which
    gets a stub to its leg.
    """
    a, b, width, pitch, plan = pair
    layers = rules.routing_layers(picture.layers)
    terminal = lambda pad: dict(xy=pad.at, layers=[l for l in layers if l in pad.layers])
    pads = {net: [picture.pads[group[0]] for group in picture.groups[net]] for net in (a, b)}
    name = f"{a} = {b}"
    while any(key == name or key.startswith(name + " ") for key in taken):
        name += "'"  # (a net may be named just so)
    # (the places come as two lots for each distance from the pads: square to the pads, then the others; and
    # one way round the two ends or the other, `gather` and `round`: see src/pairs.rs)
    # A stretch not all laid as a pair is begun further from its pads; and the third time it is left free:
    # any place with any other and from any side, which lays some pairs that have no room to be laid right.
    tried = lambda key: unpaired.get(key, 0)
    further = lambda key, lots: lots[2:] if tried(key) == 1 else lots if tried(key) in (0, FREE) else []
    ways = lambda key, trunk: {way: [further(key, lots) for lots in trunk[way]] for way in ("gather", "round")}
    trunks = [(key, dict(trunk, **ways(key, trunk))) for key, trunk in stretches_of(picture, rules, pair, unpaired, name)]
    trunks = [(key, trunk) for key, trunk in trunks if all(any(lots) for lots in trunk["gather"])]
    entries, said = [], []
    margin = rules.edge_margin()
    held = list(range(len(plan["ends"])))  # (for each end, the first of the ends that trunks join it to)
    for key, trunk in trunks:
        # (the places of an end by how far off its pads they are: the nearer take their turn first; and those
        # of one way round on one hand, so that the trunk never leaves one way and arrives the other)
        # (and each is left straight away from the middle of its two pads: a leg before each pad)
        middles = [[(first[0] + second[0]) / 2, (first[1] + second[1]) / 2] for first, second in
                   ([pads[net][i].at for net, i in zip((a, b), plan["ends"][trunk["ends"][end]])] for end in (0, 1))]
        away = lambda place, end: [r6(place["xy"][0] - middles[end][0]), r6(place["xy"][1] - middles[end][1])]
        held_to = {} if tried(key) == FREE else None
        # On a layer that the pads of an end are not on, its stubs change layer: the trunk ends further off
        # there, where two vias have room beside each other, and nearer only where that far off is shut (a place
        # for each layer takes its turn among those of that layer).
        of_pads = [{layer for layer in layers if all(layer in pads[net][i].layers
                                                      for net, i in zip((a, b), plan["ends"][trunk["ends"][end]]))}
                   for end in (0, 1)]
        # (the hints may keep either net of it on its layer)
        changes = not (rules.leaning(a).get("no_vias") or rules.leaning(b).get("no_vias"))
        vias_fit = 2.5 * max(rules.via(a)[0], rules.via(b)[0]) + pitch
        far = lambda place, end: math.dist(place["xy"], middles[end]) >= vias_fit
        places = []
        for hand, way in ((1, "gather"), (2, "round")):
            for end in (0, 1):
                if end == 1:
                    # (what it passes comes between its two ends: the engine joins them in this order)
                    places += [dict(xy=passed["place"]["xy"], layers=passed["place"]["layers"], piece=2 + number,
                                    hand=hand, heading=[r6(v) for v in passed[way]], **{"pass": True})
                               for number, passed in enumerate(trunk.get("passes", []))]
                for own in (True, False):
                    lots = [[(place, [l for l in place["layers"] if (l in of_pads[end]) == own]) for place in lot]
                            for lot in trunk[way][end]]
                    lots = [[(place, on) for place, on in lot if on] for lot in lots]
                    if not own:
                        # (where there is no room so far off, nearer after all: the stubs may yet find a way)
                        lots = ([[item for item in lot if far(item[0], end)] for lot in lots] +
                                [[item for item in lot if not far(item[0], end)] for lot in lots])
                    # (a trunk that may change layer comes to the layer of the pads itself, and ends on another
                    # only where every place on theirs is shut)
                    lots = [lot for lot in lots if lot or own]
                    for turn, lot in enumerate(lots, 0 if own or not changes else after):
                        places += [dict(xy=place["xy"], layers=on, piece=end, **(dict(turn=turn) if turn else {}),
                                        **(dict(hand=hand, heading=away(place, end)) if held_to is None else held_to))
                                   for place, on in lot]
                    after = len(lots)
        clearance = dict(default=r6(max(rules.clearance(a), rules.clearance(b))),
                         nets={n: r6(c) for net in (a, b) for n, c in rules.stricter(net).items()})
        wide = entry(f"{key} trunk", a, places, 2 * rules.settings.via_penalty)
        if changes:
            # Either via of the two keeps the hole spacing from every hole of the board. The engine sees to it
            # for holes no wider than a via's; the ground nearer a wider hole than its spacing and half the
            # vias' distance is shut to the trunk's via (a region is shut to a via's edge).
            apart, via = rules.pair_vias(a, b, width, pitch, names)
            shut = lambda hole: rules.hole_spacing(hole.drill, via[1] - apart) + apart / 2 - via[0] / 2 - margin
            holes = [dict(rings=[disc(hole.at, max(shut(hole), 0.001))], layers=[], tracks=False, vias=True)
                     for hole in picture.holes if hole.drill > via[1] - apart]
            wide = dict(wide, via=via, regions=wide.get("regions", []) + holes)
        else:
            wide = dict(wide, no_vias=True)
        # Where the two ends share no layer, the stubs of one change layer. The trunk is dearer on the layers
        # that the end with the closer pads is not on: beside those there is the least room for a via.
        ends = [[pads[net][i] for net, i in zip((a, b), plan["ends"][trunk["ends"][end]])] for end in (0, 1)]
        of_end = lambda end: {layer for layer in layers if all(layer in pad.layers for pad in end)}
        tighter = min(ends, key=lambda end: math.dist(end[0].at, end[1].at))
        dearer = {} if of_end(ends[0]) & of_end(ends[1]) else {l: OFF_LAYER for l in layers if l not in of_end(tighter)}
        wide = dict(wide, layer_cost=dict(wide.get("layer_cost", {}), **dearer)) if dearer else wide
        entries.append(dict(wide, net=f"~{key}", width=rules.trunk_width(width, pitch), clearance=clearance))
        stubs = [[], []]
        for end in (0, 1):
            for net, i in zip((a, b), plan["ends"][trunk["ends"][end]]):
                entries.append(dict(entry(f"{key} {net} {end + 1}", net, [terminal(pads[net][i])], last=True),
                                    width=width))
                stubs[end].append(entries[-1]["key"])
        passes = []
        for number, passed in enumerate(trunk.get("passes", [])):
            passes.append([])
            for net, i in zip((a, b), plan["ends"][passed["end"]]):
                entries.append(dict(entry(f"{key} {net} {number + 3}", net, [terminal(pads[net][i])], last=True),
                                    width=width))
                passes[-1].append(entries[-1]["key"])
        said.append(dict(key=key, trunk=f"{key} trunk", legs=[a, b], leg_width=width, pitch=pitch, stubs=stubs,
                         **(dict(via_pitch=apart) if changes else {}), **(dict(passes=passes) if passes else {})))
        together = {held[end] for end in list(trunk["ends"]) + [passed["end"] for passed in trunk.get("passes", [])]}
        held = [min(together) if h in together else h for h in held]
    for side, net in enumerate((a, b)):
        at_ends = [dict(terminal(pads[net][end[side]]), piece=held[k]) for k, end in enumerate(plan["ends"])]
        if trunks and len(set(held)) + len(plan["loose"][side]) > 1:
            entries.append(entry(net, net, [terminal(pads[net][i]) for i in plan["loose"][side]] + at_ends))
    return entries, said
