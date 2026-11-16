"""Pads tied to their zone by too few spokes: tracks of their own, and nets laid again clear of them."""

import collections, math
from dataclasses import replace

from .rules import Rules
from .questions import tie_targets, which_tie
from .lengths import disc
from .problem import build_problem, problem_of
from .engine import ENGINE_NETS, EngineFailed

CLEARINGS = 2  # how often nets are laid again clear of pads their tracks have starved
KEEP_OFF = 0.3  # mm beyond the clearance a zone keeps: what is left of a zone nearer a track than this is no spoke
TIE_ROUNDS = 3  # how often pads with too few spokes are given a track of their own to their zone


def tie_starved(board, settings, engine, outcome, only=None, before=frozenset(), options=None, say=print,
                laid_before=None):
    """See to every pad that is tied to its zone by fewer spokes than the board asks for. Tracks of other nets
    across its spokes are what starve a pad: one that this run starved is seen to whatever its net; one that
    the board came with (`before`: those, as (pad, layer)) as well, if its net is among those named in `only`
    or none are named.

    A pad gets a straight track from its middle into the zone, where one fits: that is a tie as good as a
    spoke, and counted as one (see `tied_by_tracks`). Where none fits, the nets whose new tracks pass by it
    are laid again clear of it (see `cleared`; `laid_before`: the tracks and vias the board came with, see
    `Picture.copper_keys`), and the ties are tried again. What was done and what could not be is added to
    `outcome` (an `Outcome`).
    """
    name = lambda picture, thermal: f"{picture.pads[thermal.pad].part or '?'} ({thermal.net}, {thermal.layer})"
    ours = lambda t: (t.pad, t.layer) not in before or only is None or t.net in only
    tried = collections.defaultdict(list)  # (pad, layer) -> the points of its zone already tried
    cleared_for = set()  # the pads that nets have been laid again for
    for again in range(CLEARINGS + 1):
        picture, starved = tied_by_tracks(board, settings, engine, outcome, ours, tried, options, say)
        # (a pad the board came with starved is not worth other nets' tracks: only what this run did is undone)
        new = [t for t in starved if (t.pad, t.layer) not in before and (t.pad, t.layer) not in cleared_for]
        if not new or again == CLEARINGS or laid_before is None:
            break
        cleared_for.update((t.pad, t.layer) for t in new)
        try:
            if not cleared(board, settings, engine, outcome, picture, new, laid_before, options, say):
                break
        except EngineFailed as failure:
            say(f"nothing laid again for the pads with too few spokes: {failure}")
            break
        tried.clear()
    outcome.starved = sorted(name(picture, thermal) for thermal in starved)
    if outcome.tied:
        say(f"{outcome.tied} pads with too few spokes to their zone tied to it by a track of their own")
    if outcome.starved:
        say(f"too few spokes to their zone, and no room for a track to it: {', '.join(outcome.starved)}")


def tied_by_tracks(board, settings, engine, outcome, ours, tried, options, say):
    """Give the pads with too few spokes (those `ours` holds for) tracks to their zones, in rounds: returns
    the board's picture after them and the pads still starved. `tried`: (pad, layer) -> the points of its
    zone already tried, which is added to."""
    routed_on = lambda picture: Rules.of(picture, settings).routing_layers(picture.layers)
    starved_of = lambda picture: [t for t in picture.thermals
                                  if t.spokes < t.least and ours(t) and t.layer in routed_on(picture)]
    picture, starved = None, []
    for round_ in range(TIE_ROUNDS + 1):
        picture = board.read()
        starved = starved_of(picture)
        if not starved or round_ == TIE_ROUNDS:
            break
        rules = Rules.of(picture, settings)
        rules = Rules.of(picture, replace(settings, step=rules.step({t.net for t in starved})))
        asked = [(thermal, picture.pads[thermal.pad].at, rules.inset(thermal.net), tried[thermal.pad, thermal.layer])
                 for thermal in starved]
        stubs = []
        try:
            for (thermal, *_), targets in zip(asked, tie_targets(picture, asked, engine.ask)):
                if targets and len(stubs) < ENGINE_NETS - 1:
                    stubs.append((f"{thermal.net} + {len(stubs)}", thermal, targets))
            if not stubs:
                break  # (nothing left to try: `starved` is what stays so)
            # (the second round on: narrower, where the net's own width found no room)
            problem = build_problem(picture, rules, {}, narrow=round_ > 0, options=options, stubs=stubs)
            solution = engine.route(problem)
            # only a track that counts as a spoke is kept: straight from the pad onto the zone
            laid = {net["key"]: net for net in solution["nets"]}
            alone = [(thermal, laid[key]["tracks"][0]["points"]) for key, thermal, _ in stubs
                     if key in laid and len(laid[key]["tracks"]) == 1 and not laid[key].get("vias")]
            tying = {id(thermal) for (thermal, _), tied in zip(alone, which_tie(picture, alone, engine.ask)) if tied}
        except EngineFailed as failure:
            say(f"no tracks to the zones of pads with too few spokes: {failure}")
            break
        kept = []
        for key, thermal, targets in stubs:
            tracks = laid.get(key, {}).get("tracks", [])
            if id(thermal) in tying:
                kept.append(laid[key])
            # A point is not given again once a track has been laid to it that does not tie, nor any once a
            # narrower track has found no way to any of them. (A track's ends, whichever way round it is.)
            ends = [p for track in tracks for p in (track["points"][0], track["points"][-1])]
            reached = [t for t in targets if any(math.dist(t, p) < 2 * rules.settings.step for p in ends)]
            tried[thermal.pad, thermal.layer] += reached if tracks else targets if round_ > 0 else []
        if kept:
            added = board.write(dict(solution, nets=kept), {n["key"]: (n["width"], n["via"]) for n in problem["nets"]})
            outcome.tracks, outcome.tied = outcome.tracks + added[0], outcome.tied + len(kept)
    return picture, starved


def cleared(board, settings, engine, outcome, picture, starved, laid_before, options, say):
    """Lay again, clear of pads that have too few spokes and no room for a track to their zone, the nets whose
    new tracks pass by them: True if that was done.

    A track of another net close by a pad takes the zone away from the pad's spokes, and one that goes round
    the pad leaves no way out for a track of the pad's own. The nets that this run laid within reach of such
    a pad (the pad, the gap, the clearance the zone keeps and a track's width) are taken out and laid again
    with that ground closed to them. If they cannot all be joined so, or the way they then take parts another
    net (from its zone), they are put back as they were.
    `board.take_out(nets)` removes what has been written for those nets and `board.put_back()` restores it
    (both return the numbers of track segments and vias).
    """
    part = lambda thermal: picture.pads[thermal.pad].part or "?"
    reach = {}  # (pad, layer) -> (its middle, how far from it a track of another net is too near)
    for thermal in starved:
        at = picture.pads[thermal.pad].at
        # (the outline is half the gap from the pad)
        beyond = max((math.dist(at, p) for p in thermal.outline), default=0.0)
        reach[thermal.pad, thermal.layer] = (at, beyond + thermal.gap / 2 + thermal.clearance + KEEP_OFF)
    def near(net, layer, a, b, half):
        for thermal in starved:
            at, far = reach[thermal.pad, thermal.layer]
            vx, vy = b[0] - a[0], b[1] - a[1]
            l2 = vx * vx + vy * vy
            t = max(0.0, min(1.0, ((at[0] - a[0]) * vx + (at[1] - a[1]) * vy) / l2)) if l2 else 0.0
            nearest = (a[0] + t * vx, a[1] + t * vy)
            if net != thermal.net and layer in (None, thermal.layer) and math.dist(at, nearest) < far + half:
                return True
        return False
    new = lambda key: key not in laid_before
    laid_now = (t for t in picture.tracks if t.net and new((t.net, t.layer, tuple(t.a), tuple(t.b))))
    culprits = {t.net for t in laid_now if near(t.net, t.layer, t.a, t.b, t.width / 2)}
    culprits |= {v.net for v in picture.vias
                 if v.net and new((v.net, None, tuple(v.at), tuple(v.at)))
                 and near(v.net, None, v.at, v.at, v.width / 2)}
    culprits = sorted(culprits)
    if not culprits or len(culprits) >= ENGINE_NETS:
        return False
    open_before = set(board.unjoined())
    mark = board.mark()
    taken = board.take_out(culprits)
    after = board.read()
    # (as single nets, without the links of capacitors: this is a repair)
    rules = Rules.of(after, replace(settings, pairs=False, decoupling=False))
    rules = Rules.of(after, replace(rules.settings, step=rules.step(set(culprits))))
    closed = [dict(rings=[disc(at, far)], layers=[layer], tracks=True, vias=True)
              for (_, layer), (at, far) in reach.items()]
    problem = problem_of(after, rules, engine.ask, only=set(culprits), options=options, closed=closed)
    solution = engine.route(problem) if problem["nets"] else dict(status="routed", nets=[])
    done = solution["status"] == "routed"
    if done:
        added = board.write(solution, {n["key"]: (n["width"], n["via"]) for n in problem["nets"]})
        # (all joined, and no other net parted by the new way they take: it may cut a zone that the old did not)
        done = set(board.unjoined()) <= open_before
    if not done:
        board.rewind(mark)
        board.put_back()
        say(f"no way clear of {', '.join(sorted({part(t) for t in starved}))} for {', '.join(culprits)}: "
            "left as they were")
        return False
    outcome.tracks, outcome.vias = outcome.tracks - taken[0] + added[0], outcome.vias - taken[1] + added[1]
    say(f"laid again clear of {', '.join(sorted({part(t) for t in starved}))}, whose spokes they cut: "
        f"{', '.join(culprits)}")
    return True
