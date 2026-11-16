"""The problem: a picture and the rules as the router's JSON (src/model.rs). Nothing here touches KiCad."""

import collections, itertools, math, sys

from .names import CAPACITOR_PART, CHIP_PART
from .picture import r6
from .intent import amps_width
from .questions import spots, reach_of
from .engine import ENGINE_NETS


def disc(centre, radius, sides=16):
    """A polygon that covers a disc."""
    out = radius / math.cos(math.pi / sides)
    return [[r6(centre[0] + out * math.cos(2 * math.pi * k / sides)), r6(centre[1] + out * math.sin(2 * math.pi * k / sides))]
            for k in range(sides)]


def _grown(rings, grow, net, layers):
    """A polygon and what growing it by a distance adds: a round-ended stroke of that half-width along each edge."""
    out = [dict(t="poly", rings=rings, net=net, layers=layers)]
    if grow > 0:
        for ring in rings:
            for a, b in zip(ring, ring[1:] + ring[:1]):
                out.append(dict(t="seg", a=a, b=b, w=r6(2 * grow), net=net, layers=layers))
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
    pads_of = lambda net: spots(picture.pads[group[0]] for group in picture.groups[net])
    sizes = [rules.pair(a, b, names) for a, b in named]
    plans = ask(dict(ask="pairs", layers=rules.routing_layers(picture.layers), step=rules.settings.step,
                     pairs=[dict(a=pads_of(a), b=pads_of(b), pitch=pitch) for (a, b), (_, pitch) in zip(named, sizes)]))
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
    further = lambda key, trunk: [lots[min(unpaired.get(key, 0), len(lots)):] for lots in trunk["gather"]]
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
    reach = reach_of(picture, rules, names, ask)
    return build_problem(picture, rules, reach, only=only, narrow=narrow, pairs=pairs, unpaired=unpaired, notes=notes,
                         **more)


def build_problem(picture, rules, reached, only=None, narrow=False, options=None, unlinked=frozenset(), stubs=None,
                  pairs=(), unpaired=None, notes=None,
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
        copper.append(dict(t="seg", a=track.a, b=track.b, w=r6(track.width), net=track.net, layers=[track.layer]))
    for via in picture.vias:
        copper.append(dict(t="circle", c=via.at, r=r6(via.width / 2), net=via.net, layers=every))
    for polygon in picture.polygons:
        copper.append(dict(t="poly", rings=polygon.rings, net=polygon.net, layers=[polygon.layer]))
    # every drilled hole, of a pad or of a via alike: a hole, and a disc that copper of other nets keeps off
    drilled = picture.holes
    for hole in drilled:
        holes.append(dict(xy=hole.at, net=hole.net))
        copper.append(dict(t="circle", c=hole.at, r=r6(hole.drill / 2 + rules.hole_growth(hole)), net=hole.net, layers=every))

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
            asked = amps_width(rules.intent(name).amps)
            note(f"{name} is laid {width:.2f} mm wide to enter its pads, narrower than the {asked:.2f} mm "
                 "its current takes")
        lean = rules.leaning(name)  # (what the net is for: see Rules.leaning)
        # the layers the hints give it, and those of any terminal that is on none of them: a pad is where it is
        asked = lean.pop("layers", None)
        within = [l for l in layers if asked is None or l in asked or any(l in t["layers"] and not set(t["layers"]) & set(asked) for t in terminals)]
        return dict(key=key, net=name, width=r6(width), via=[r6(v) for v in rules.via(name)], layers=within or layers,
                    clearance=dict(default=r6(rules.clearance(name)), nets={n: r6(c) for n, c in rules.stricter(name).items()}),
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
            regions.append(dict(rings=[disc(hole.at, max(radius, 0.001))], layers=[], tracks=False, vias=True))

    real = [n["net"] for n in nets if n["net"] in picture.classes]
    quiet = rules.quiet_clearance(real)
    problem_rules = dict(hole_centre_mm=r6(rules.hole_spacing(via_drill, via_drill)), pad_drill_margin_mm=0.0)
    if quiet:
        problem_rules["quiet_clearance_mm"] = quiet
    problem_rules.update(region_margin_mm=r6(margin), new_clearance_mm=r6(rules.new_clearance(real)))
    if narrow:
        problem_rules["via_in_pad"] = True  # a land enclosed by another pad can only be left through a via in it
    problem = dict(window=window, step=rules.settings.step, layers=layers, copper=copper, holes=holes, regions=regions)
    if dear:
        problem["dear"] = dear
    if problem_pairs:
        problem["pairs"] = problem_pairs
    problem.update(rules=problem_rules, nets=nets, options=dict(dict(bus_pull=rules.settings.bus_pull), **(options or {})))
    return problem
