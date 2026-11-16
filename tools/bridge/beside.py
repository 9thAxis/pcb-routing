"""What stands beside what on a board: nets that leave one part and arrive at another side by side (buses), and
capacitors that stand by the pin of a chip they decouple. Plain geometry of pads and their names; the measures
of tools/quality go by the same distances."""

import collections, itertools, math

from .names import CAPACITOR_PART, CHIP_PART

NEIGHBOUR = 1.3  # two pads of a part are neighbours within this many times the distance to the nearest pad


def buses(pads, nets=None):
    """Which nets run as buses: net name -> (name of its bus, its lane across it), among `nets` if given.

    Two nets are neighbours on a part if they have pads there no further apart than NEIGHBOUR times the distance
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
                if distance > NEIGHBOUR * others[0][0]:
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


DECOUPLING_REACH = 5.0  # mm from a capacitor's pad to the pin it is taken to stand by


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
