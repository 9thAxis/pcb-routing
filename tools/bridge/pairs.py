"""Differential pairs: which nets are laid as one, and the entries of the problem that lay them so. Where the pads
of two nets face each other is the engine's to say (src/pairs.rs)."""

from .lengths import r6
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
        if not plan["trunks"]:
            few = len(plan["ends"]) < 2
            note(a, b, "no two places where a pad of each stands by the other" if few
                 else "its ends are too near each other, or share no routing layer")
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
    # (the places come as two lots for each distance from the pads: square to the pads, then the others)
    further = lambda key, trunk: [lots[min(2 * unpaired.get(key, 0), len(lots)):] for lots in trunk["gather"]]
    trunks = [(key_of(number), trunk) for number, trunk in enumerate(plan["trunks"])]
    trunks = [(key, dict(trunk, gather=further(key, trunk))) for key, trunk in trunks]
    trunks = [(key, trunk) for key, trunk in trunks if all(trunk["gather"])]
    entries, said = [], []
    held = list(range(len(plan["ends"])))  # (for each end, the first of the ends that trunks join it to)
    for key, trunk in trunks:
        # (the places of an end by how far off its pads they are: the nearer take their turn first)
        places = [dict(xy=place["xy"], layers=place["layers"], piece=end, **(dict(turn=turn) if turn else {}))
                  for end in (0, 1) for turn, lot in enumerate(trunk["gather"][end]) for place in lot]
        clearance = dict(default=r6(max(rules.clearance(a), rules.clearance(b))),
                         nets={n: r6(c) for net in (a, b) for n, c in rules.stricter(net).items()})
        wide = entry(f"{key} trunk", a, places, no_vias=True)
        entries.append(dict(wide, net=f"~{key}", width=rules.trunk_width(width, pitch), clearance=clearance))
        stubs = [[], []]
        for end in (0, 1):
            for net, i in zip((a, b), plan["ends"][trunk["ends"][end]]):
                entries.append(dict(entry(f"{key} {net} {end + 1}", net, [terminal(pads[net][i])], last=True),
                                    width=width))
                stubs[end].append(entries[-1]["key"])
        said.append(dict(key=key, trunk=f"{key} trunk", legs=[a, b], leg_width=width, pitch=pitch, stubs=stubs))
        first, second = (held[end] for end in trunk["ends"])
        held = [min(first, second) if h in (first, second) else h for h in held]
    for side, net in enumerate((a, b)):
        at_ends = [dict(terminal(pads[net][end[side]]), piece=held[k]) for k, end in enumerate(plan["ends"])]
        if trunks and len(set(held)) + len(plan["loose"][side]) > 1:
            entries.append(entry(net, net, [terminal(pads[net][i]) for i in plan["loose"][side]] + at_ends))
    return entries, said
