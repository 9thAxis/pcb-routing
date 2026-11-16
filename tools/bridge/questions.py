"""Zones: the geometry is the engine's (src/pours); these are the questions put to it. `ask` takes a question and
returns the answer (see `Engine.ask` in engine.py). Nothing here touches KiCad."""


def spots(pads):
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
        return [dict(net=pour.net, layer=pour.layer, rings=pour.rings)
                for pour in (self.picture.fills[f] for f in self.index)]


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
    found = ask(dict(ask="reach", pads=spots(picture.pads), fills=named.fills,
                     layers=rules.routing_layers(picture.layers), step=rules.settings.step, nets=nets))
    return {name: [[(place["xy"], place["layers"]) for place in group] for group in groups]
            for name, groups in zip(names, found)}


def parts_of(picture, problem, solution, ask):
    """What a run laid, as what can be taken back piece by piece: the copper of an entry that has points of a
    pour among its terminals as the pieces that hang together (keyed `KEY # n`, with `of` the entry's key),
    each with the points of a pour it came to end on (`ends`: net, layer, xy); any other entry whole."""
    return ask(dict(ask="parts", pads=spots(picture.pads), entries=problem["nets"], laid=solution["nets"]))


def on_zone(picture, points, ask):
    """Whether each of these points (net, layer, xy) is on a pour of its net, as the picture has the zones."""
    wanted = {(point["net"], point["layer"]) for point in points}
    fills = [dict(net=pour.net, layer=pour.layer, rings=pour.rings) for pour in picture.fills
             if (pour.net, pour.layer) in wanted]
    return ask(dict(ask="on_zone", fills=fills, points=points))


def tie_targets(picture, starved, ask):
    """For pads with too few spokes, as [(Thermal, the pad's middle, how far within the zone to end, points
    not to give again)]: for each, the points of its zone that a straight track from its middle may end on."""
    named = _Named(picture)
    pads = [dict(outline=thermal.outline, fill=named(thermal.fill), at=at, inset=inset, tried=tried)
            for thermal, at, inset, tried in starved]
    return ask(dict(ask="tie_targets", fills=named.fills, pads=pads)) if pads else []


def which_tie(picture, tracks, ask):
    """For tracks laid to tie a pad to its zone, as [(Thermal, the track's points)]: whether each counts as a
    spoke of its pad for KiCad."""
    named = _Named(picture)
    tracks = [dict(outline=thermal.outline, fill=named(thermal.fill), points=points) for thermal, points in tracks]
    return ask(dict(ask="ties", fills=named.fills, tracks=tracks)) if tracks else []


def pair_plans(picture, rules, pairs, ask):
    """For each of some pairs (net, net, pitch): where its pads face each other and where it may be laid as one
    (src/pairs.rs: `plan`). A group of pads that are joined already counts as its first pad."""
    pads_of = lambda net: spots(picture.pads[group[0]] for group in picture.groups[net])
    asked = [dict(a=pads_of(a), b=pads_of(b), pitch=pitch) for a, b, pitch in pairs]
    return ask(dict(ask="pairs", layers=rules.routing_layers(picture.layers), step=rules.settings.step, pairs=asked))


def survey(pads, wires, groups, fills, spoked, ask):
    """What the filled zones of a board join, and how they tie each pad (src/pours: `survey`)."""
    zoned = {pour.net for pour in fills}
    copper = lambda pad: {layer: pad.shapes[layer] for layer in pad.layers} if pad.net in zoned else {}
    return ask(dict(ask="survey",
                    pads=[dict(net=pad.net, at=pad.at, layers=pad.layers, shapes=copper(pad)) for pad in pads],
                    wires=wires, groups=groups, spoked=spoked,
                    fills=[dict(net=pour.net, layer=pour.layer, rings=pour.rings, zone=pour.zone) for pour in fills]))
