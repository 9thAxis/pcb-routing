"""The problem: a picture and the rules as the router's JSON (src/model.rs). Nothing here touches KiCad."""

import math, sys

from .lengths import r6
from .rules import amps_width
from .questions import reach_of
from .beside import buses, decoupling
from .pairs import pair_entries, pairs_of
from .engine import ENGINE_NETS


def disc(centre, radius, sides=16):
    """A polygon that covers a disc."""
    out = radius / math.cos(math.pi / sides)
    return [[r6(centre[0] + out * math.cos(2 * math.pi * k / sides)),
             r6(centre[1] + out * math.sin(2 * math.pi * k / sides))]
            for k in range(sides)]


def _grown(rings, grow, net, layers):
    """A polygon and what growing it by a distance adds: a round-ended stroke of that half-width along each edge."""
    out = [dict(t="poly", rings=rings, net=net, layers=layers)]
    if grow > 0:
        for ring in rings:
            for a, b in zip(ring, ring[1:] + ring[:1]):
                out.append(dict(t="seg", a=a, b=b, w=r6(2 * grow), net=net, layers=layers))
    return out


DECOUPLING_VIA_PENALTY = 5.0  # times the usual: the detour accepted before the link to the pin changes layer


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
        copper.append(dict(t="circle", c=hole.at, r=r6(hole.drill / 2 + rules.hole_growth(hole)), net=hole.net,
                           layers=every))

    if picture.outline is not None:
        frame = [[window[0] - 1, window[1] - 1], [window[2] + 1, window[1] - 1], [window[2] + 1, window[3] + 1],
                 [window[0] - 1, window[3] + 1]]
        rings = [frame]
        for polygon in picture.outline:
            rings.extend(polygon)  # even-odd: closed outside the board and inside its cut-outs
        regions.append(dict(rings=rings))
    for area in picture.areas:
        regions.append(dict(rings=area.rings, layers=[l for l in area.layers if l in layers], tracks=area.tracks,
                            vias=area.vias))
    regions.extend(closed)
    # A pour is cut by a track of another net, and a pad is cut off from its pour by one across its spokes: both
    # are open ground that costs more, to every net but the pour's own.
    dear = [dict(rings=pour.rings, layers=[pour.layer], factor=rules.settings.pour_cost, net=pour.net)
            for pour in picture.fills if pour.layer in layers and rules.settings.pour_cost > 1]
    dear += [dict(rings=rings, layers=[thermal.layer], factor=rules.settings.spoke_cost, net=thermal.net)
             for thermal in picture.thermals if thermal.layer in layers and rules.settings.spoke_cost > 1
             for rings in thermal.ground]

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
        within = [l for l in layers
                  if asked is None or l in asked
                  or any(l in t["layers"] and not set(t["layers"]) & set(asked) for t in terminals)]
        return dict(key=key, net=name, width=r6(width), via=[r6(v) for v in rules.via(name)], layers=within or layers,
                    clearance=dict(default=r6(rules.clearance(name)),
                                   nets={n: r6(c) for n, c in rules.stricter(name).items()}),
                    terminals=terminals, via_penalty_mm=via_penalty * lean.pop("via_penalty", 1.0),
                    **dict(lean, **more))
    for key, thermal, targets in stubs or []:
        start = dict(xy=picture.pads[thermal.pad].at, layers=[thermal.layer])
        ends = [dict(xy=at, layers=[thermal.layer], piece=0) for at in targets]
        nets.append(entry(key, thermal.net, [start] + ends, rules.settings.via_penalty, no_vias=True))
    on = lambda pad: [l for l in layers if l in pad.layers]
    terminal = lambda pad: dict(xy=pad.at, layers=on(pad))
    paired, problem_pairs = set(), []
    for pair in pairs:
        a, b = pair[:2]
        entries, said = pair_entries(picture, rules, pair, entry, unpaired or {},
                                     set(picture.groups) | {n["key"] for n in nets})
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
        links = decoupling(picture.pads,
                           groups) if rules.settings.decoupling and name not in unlinked and not pours else []
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
    problem.update(rules=problem_rules, nets=nets,
                   options=dict(dict(bus_pull=rules.settings.bus_pull), **(options or {})))
    return problem
