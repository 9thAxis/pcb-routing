"""What a designer would judge in a routed board, measured against a reference routing of the same board.

Everything here is a pure function of dumps (the JSON pictures written by dump.py): no files, no printing, no
state. score.py reads the dumps, calls these and prints the tables. Needs shapely and numpy.

A dump is a dict (lengths in mm, points [x, y], layers by name):

    layers       the copper layers, top to bottom
    pads         [{net, at, shapes: {layer: [ring, ...]}, ref, num}]   a ring is a list of points
    tracks       [{net, a, b, w, layer, len}], and "mid" for an arc
    vias         [{net, at, d}]                 taken as through vias: on every layer
    zone_nets    names of the nets that have a copper zone
    unconnected  how many connections KiCad counts as still open (absent in dumps of older make)
    rules        {clearance, track_widths: {class: mm}, patterns: [[pattern, class]], assignments: {net: class}}
                 from the board's project file, absent without one

Two points are the same place when they agree to 0.001 mm (`key`).

The reference decides which nets are measured; the same nets are then measured in every routing, the reference
included. A net is "routed with tracks" when the reference has at least one track segment of it, and "poured"
when the reference has a zone of it. The selections, by table:

general (`compared_nets`, `net_shapes`, `general_figures`)
    Nets with pads at two or more places, routed with tracks and not poured in the reference. A net with pads
    at n places has n - 1 connections. Summed over those nets:
    length      sum of the track segment lengths (KiCad's length: arcs along the arc)
    detour      length over the length of the shortest tree through the net's pad centres (Euclidean minimum
                spanning tree, no added points), sum over sum
    vias        vias of the net per 100 connections
    corners     places where exactly two straight segments of the net end on the same layer and the direction
                changes by more than 1 degree, per connection. Three segments meeting (a T) is a branch, not a
                corner; a segment end against an arc is not counted
    sharp       corners where the direction changes by more than 89 degrees (a turn of 90 degrees or more), per
                100 connections
    short       segments shorter than twice their own width, per 100 connections
    tight       share of track length whose gap to other nets' copper (tracks, vias, pads on the same layer;
                zones are not in the dump) is under 1.1 times the clearance. Each segment is cut into pieces of
                at most 0.5 mm; a piece's gap is the distance from its centre line to the nearest other copper
                less half its width. Arcs are measured along their chord
    gap         the median of that gap over the track length, in clearances, capped at 5

buses (`bus_pairs`, `bus_measures`, `bus_figures`)
    Two pads of a part are neighbours when their distance is at most 1.3 times the distance from the first to
    its nearest pad. Two nets are a pair between parts F and G when they have neighbouring pads on F and
    neighbouring pads on G, and both are routed with tracks and not poured in the reference. Pairs between the
    same two parts that hang together form a group; only groups of three nets or more are buses. The pitch of
    a pair is the larger of its two pad distances. Per pair (the first net is the one whose name sorts first),
    averaged over the pairs both of whose nets have tracks in the routing:
    alongside   share of the first net's track that lies within 1.5 pitches of the second's on the same layer
    same vias   both nets have the same number of vias
    crossing    a segment of one crosses a segment of the other on a different layer
    mismatch    difference of the two lengths over the longer

circuit (`circuit_nets`, `circuit_measures`, `circuit_figures`): read from net names and part references
    power       nets with pads at two or more places whose name looks like a supply or ground (`is_supply`), not
                poured in the reference, and with tracks in the routing being measured (so the count can differ
                between routings): share of length narrower than the net class's track width, vias per 100
                connections, detour
    decoupling  a two-pad part C.. between a supply net and a ground net (`GROUND`), with a pad of the same
                supply on a part U.. or IC.. within 5 mm; the nearest such pad is taken. Measured: the length of
                the net's copper from the capacitor's pad to that pad (shortest way along its tracks; a via
                costs nothing) over the straight distance, median over the capacitors; whether that way
                passes a via; and the share with no such way (joined through other pads, a zone, or not at
                all). Supplies poured in the reference are left out for the reference only
    crystals    nets, not power, on parts Y.., X.. or XTAL.., with tracks in the routing: detour and vias
    pairs       two nets, not power, whose names differ only in an ending _P/_N, -P/-N, P/N, +/- or DP/DM, both
                with tracks in the routing and not poured in the reference. The first is the name that sorts
                first: share of its length within 4 of its (widest) track widths of the other on the same
                layer, length mismatch, same number of vias

pad exits (`exit_measures`, `exit_figures`)
    For every pad of a net routed with tracks and not poured in the reference, and every straight segment of
    that net that ends on the pad's centre on one of its layers: follow the track (up to six segments, as long
    as it does not branch) to the first segment that leaves the pad's outline.
    grazing     that segment meets the edge of the outline it crosses at under 40 degrees
    long side   for pads at least 1.5 times as long as wide (by the smallest rectangle around them): the edge
                crossed runs along the pad's length
"""
import collections, fnmatch, heapq, itertools, math, sys
from pathlib import Path

import numpy as np
import shapely
from shapely.geometry import LineString, MultiLineString, Point, Polygon

DEFAULT_CLEARANCE = 0.2  # mm, when the dumps carry no rules and the caller names none
PIECE = 0.5              # mm: tracks are cut into pieces this long at most for the gap to other copper
GAP_CAP = 5.0            # clearances: gaps are not told apart beyond this
TIGHT = 1.1              # clearances: a gap under this is tight
BUS_REACH = 1.5          # pad pitches within which two bus nets run alongside
PAIR_REACH = 4.0         # track widths within which the halves of a differential pair are coupled

# What names say (supplies, grounds, pairs, capacitors, chips, crystals) is said once, in tools/bridge/names.py:
# the router goes by the same.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from bridge.beside import DECOUPLING_REACH as DECAP_REACH, NEIGHBOUR
from bridge.names import CAPACITOR_PART as CAPACITOR, CHIP_PART as CHIP, CRYSTAL_PART as CRYSTAL, GROUND, is_supply, pair_names as pair_halves


class Tally(collections.Counter):
    """Sums by name, and a list of samples for the figures that are medians."""

    def __init__(self):
        super().__init__()
        self.samples = []

    def add(self, other):
        self.update(other)
        self.samples.extend(other.samples)
        return self


# --------------------------------------------------------------------------------------------------------------
# Geometry

def key(p):
    """A point as a place: to 0.001 mm."""
    return (round(p[0], 3), round(p[1], 3))


def places(points):
    """The distinct places among these points."""
    return list({key(p) for p in points})


def tree_length(points):
    """Length of the shortest tree joining the distinct places among these points (Euclidean minimum spanning
    tree, no added points), and how many places there are."""
    pts = places(points)
    if len(pts) < 2:
        return 0.0, len(pts)
    P = np.array(pts)
    n = len(P)
    best, done, total = np.full(n, np.inf), np.zeros(n, bool), 0.0
    best[0] = 0
    for _ in range(n):
        i = int(np.argmin(np.where(done, np.inf, best)))
        total += best[i]
        done[i] = True
        best = np.minimum(best, np.hypot(*(P - P[i]).T))
    return float(total), n


def turn(u, v):
    """Change of direction in degrees where two segments meet, given the unit directions in which each leaves
    the meeting point: 0 straight on, 180 back on itself."""
    return 180 - math.degrees(math.acos(max(-1, min(1, u[0] * v[0] + u[1] * v[1]))))


def median_by_length(samples):
    """The value at half the total length of (value, length) samples; 0 without any."""
    ordered = sorted(samples)
    half, run = (sum(length for _, length in ordered) or 1) / 2, 0
    for value, length in ordered:
        run += length
        if run >= half:
            return value
    return 0


def upper_median(values):
    """The middle value (the upper of the two middle ones); nan without any."""
    return sorted(values)[len(values) // 2] if values else float("nan")


def chords(tracks):
    """The centre lines of these tracks by layer, each from end to end (an arc as its chord); tracks of no
    length left out."""
    lines = collections.defaultdict(list)
    for t in tracks:
        if t["a"] != t["b"]:
            lines[t["layer"]].append(LineString([t["a"], t["b"]]))
    return lines


def share_near(lines, others, reach):
    """Length of these lines (by layer) that lies within `reach` of the others on the same layer."""
    near = 0.0
    for layer, segments in lines.items():
        if layer in others:
            zone = MultiLineString(others[layer]).buffer(reach)
            near += sum(s.intersection(zone).length for s in segments)
    return near


def tracks_by_net(dump):
    by = collections.defaultdict(list)
    for t in dump["tracks"]:
        by[t["net"]].append(t)
    return by


def vias_by_net(dump):
    return collections.Counter(v["net"] for v in dump["vias"])


# --------------------------------------------------------------------------------------------------------------
# Which boards and nets

def tracked_nets(reference):
    """Nets the reference routed with tracks and not with a zone."""
    return {t["net"] for t in reference["tracks"]} - set(reference["zone_nets"])


def compared_nets(reference):
    """The nets of the general table: {net: (shortest tree in mm, connections)}, in the order of the pads."""
    at = collections.defaultdict(list)
    for p in reference["pads"]:
        if p["net"]:
            at[p["net"]].append(p["at"])
    tracked, nets = tracked_nets(reference), {}
    for net, points in at.items():
        tree, count = tree_length(points)
        if count >= 2 and net in tracked:
            nets[net] = (tree, count - 1)
    return nets


def same_pads(a, b):
    """Whether two dumps are of the same board: pads at the same places on the same nets."""
    pads = lambda d: sorted((p["net"] or "",) + key(p["at"]) for p in d["pads"])
    return pads(a) == pads(b)


def open_connections(dump):
    """How many connections the dump says are open; None when it does not say."""
    return dump.get("unconnected")


def unusable(reference, routings, assume_complete=False):
    """Why a board cannot be compared, or None when it can. `routings` is {name: dump}. Every routing must be of
    the reference's board and, unless the caller vouches for them with `assume_complete`, say that it has no
    open connection. The reference need not be complete."""
    for name, dump in routings.items():
        if not same_pads(reference, dump):
            return f"{name} is not the reference's board (pads differ)"
    if assume_complete:
        return None
    for name, dump in routings.items():
        left = open_connections(dump)
        if left is None:
            return f"{name} does not say whether it is complete (dump it again, or state it with --assume-complete)"
        if left:
            return f"{name} has {left} open connections"
    return None


def board_rules(reference, routings):
    """The rules to measure a board by: the reference's, else those of the first routing that has any."""
    return next((d["rules"] for d in [reference, *routings.values()] if d.get("rules")), {})


def clearance_of(rules):
    """The Default class's clearance, 0.2 mm without one."""
    return rules.get("clearance") or DEFAULT_CLEARANCE


def class_width(rules, net):
    """The track width of a net's class: the first pattern that matches, else its assignment, else Default; 0
    when the rules name none."""
    widths = rules.get("track_widths") or {}
    for pattern, netclass in rules.get("patterns") or []:
        if fnmatch.fnmatchcase(net, pattern or "") and widths.get(netclass):
            return widths[netclass]
    for name, classes in (rules.get("assignments") or {}).items():
        first = classes[0] if isinstance(classes, list) and classes else classes
        if name == net and classes and widths.get(first):
            return widths[first]
    return widths.get("Default") or 0.0


# --------------------------------------------------------------------------------------------------------------
# General: length, detour, vias, corners, short segments, spacing

def copper_by_layer(dump):
    """Per layer: the copper shapes (tracks, vias, pads), the net of each, and a search tree over them."""
    shapes = {l: [] for l in dump["layers"]}
    owner = {l: [] for l in dump["layers"]}
    for t in dump["tracks"]:
        if t["layer"] in shapes and t["a"] != t["b"]:
            line = LineString([t["a"]] + ([t["mid"]] if "mid" in t else []) + [t["b"]])
            shapes[t["layer"]].append(line.buffer(t["w"] / 2))
            owner[t["layer"]].append(t["net"] or None)
    for v in dump["vias"]:
        for l in dump["layers"]:
            shapes[l].append(Point(v["at"]).buffer(v["d"] / 2))
            owner[l].append(v["net"] or None)
    for p in dump["pads"]:
        for l, rings in p["shapes"].items():
            for ring in rings:
                if len(ring) >= 3 and l in shapes:
                    shapes[l].append(Polygon(ring).buffer(0))
                    owner[l].append(p["net"] or None)
    return shapes, owner, {l: shapely.STRtree(s) for l, s in shapes.items()}


def net_shapes(dump, clearance, nets):
    """For each of `nets` in one routing: a Counter of mm (track length), short (segments shorter than twice
    their width), corners, sharp, vias; and the gaps to other nets' copper along its tracks as (gap in
    clearances, mm of track) pieces."""
    counts = {n: collections.Counter() for n in nets}
    gaps = {n: [] for n in nets}
    shapes, owner, trees = copper_by_layer(dump)
    ends = collections.defaultdict(list)  # (net, layer, place) -> directions leaving it, None for an arc
    for t in dump["tracks"]:
        n = t["net"]
        if n not in counts:
            continue
        counts[n]["mm"] += t["len"]
        if t["len"] < 2 * t["w"]:
            counts[n]["short"] += 1
        a, b = t["a"], t["b"]
        L = math.hypot(b[0] - a[0], b[1] - a[1])
        if L < 1e-6 or t["layer"] not in trees:
            continue
        u = None if "mid" in t else ((b[0] - a[0]) / L, (b[1] - a[1]) / L)
        ends[(n, t["layer"]) + key(a)].append(u)
        ends[(n, t["layer"]) + key(b)].append(None if u is None else (-u[0], -u[1]))
        k = max(1, math.ceil(L / PIECE))
        tree, around, own = trees[t["layer"]], shapes[t["layer"]], owner[t["layer"]]
        for i in range(k):
            piece = LineString([(a[0] + (b[0] - a[0]) * i / k, a[1] + (b[1] - a[1]) * i / k),
                                (a[0] + (b[0] - a[0]) * (i + 1) / k, a[1] + (b[1] - a[1]) * (i + 1) / k)])
            near = tree.query(piece.buffer(t["w"] / 2 + GAP_CAP * clearance))
            dist = min((piece.distance(around[j]) for j in near if own[j] != n), default=math.inf)
            gaps[n].append((min(GAP_CAP, max(0.0, dist - t["w"] / 2) / clearance), L / k))
    for (n, _, _, _), leaving in ends.items():
        if len(leaving) == 2 and None not in leaving:
            change = turn(*leaving)
            if change > 1:
                counts[n]["corners"] += 1
            if change > 89:
                counts[n]["sharp"] += 1
    for v in dump["vias"]:
        if v["net"] in counts:
            counts[v["net"]]["vias"] += 1
    return counts, gaps


def general_measures(dump, clearance, nets):
    """One routing of one board over `nets` (from compared_nets): a Tally of mm, short, corners, sharp, vias,
    tree (mm) and connections, with the gap pieces as samples; and the per-net Counters it was summed from."""
    counts, gaps = net_shapes(dump, clearance, nets)
    tally = Tally()
    for net, (tree, connections) in nets.items():
        tally.update(counts[net])
        tally["tree"] += tree
        tally["connections"] += connections
        tally.samples.extend(gaps[net])
    return tally, counts


def general_figures(tally, reference_mm):
    """The row of the general table from a routing's Tally and the reference's track length."""
    connections = max(tally["connections"], 1)
    length = sum(mm for _, mm in tally.samples) or 1
    return dict(
        mm=tally["mm"],
        detour=tally["mm"] / max(tally["tree"], 1e-9),
        length_vs_reference=tally["mm"] / max(reference_mm, 1e-9),
        vias_per_100=100 * tally["vias"] / connections,
        corners_per_connection=tally["corners"] / connections,
        sharp_per_100=100 * tally["sharp"] / connections,
        short_per_100=100 * tally["short"] / connections,
        tight_percent=100 * sum(mm for gap, mm in tally.samples if gap < TIGHT) / length,
        median_gap=median_by_length(tally.samples),
    )


# --------------------------------------------------------------------------------------------------------------
# Buses

def neighbour_nets(reference):
    """{(net A, net B): {part: pad distance}} for nets with neighbouring pads on a part, A sorting before B."""
    parts = collections.defaultdict(list)
    for p in reference["pads"]:
        if p.get("ref"):
            parts[p["ref"]].append(p)
    near = collections.defaultdict(dict)
    for ref, pads in parts.items():
        for a in pads:
            others = sorted((math.dist(a["at"], b["at"]), i) for i, b in enumerate(pads) if b is not a and math.dist(a["at"], b["at"]) > 1e-6)
            if not others:
                continue
            for dist, i in others:
                if dist > NEIGHBOUR * others[0][0]:
                    break
                b = pads[i]
                if a["net"] and b["net"] and a["net"] != b["net"]:
                    near[tuple(sorted((a["net"], b["net"])))][ref] = dist
    return near


def bus_pairs(reference):
    """The bus pairs of a board as [(net A, net B, pitch)], each pair once with the widest pitch it has between
    two parts, and how many buses they form (groups of nets joined by those pairs)."""
    tracked = tracked_nets(reference)
    between = collections.defaultdict(list)  # (part F, part G) -> [(A, B, pitch)]
    for (a, b), where in neighbour_nets(reference).items():
        if a in tracked and b in tracked:
            for f, g in itertools.combinations(sorted(where), 2):
                between[(f, g)].append((a, b, max(where[f], where[g])))
    pitch = {}  # (A, B) -> pitch, for the pairs of a group of three or more between the same two parts
    for pairs in between.values():
        for group in _groups((a, b) for a, b, _ in pairs):
            if len(group) >= 3:
                for a, b, d in pairs:
                    if a in group:
                        pitch[(a, b)] = max(d, pitch.get((a, b), d))
    return [(a, b, d) for (a, b), d in pitch.items()], len(_groups(pitch))


def _groups(pairs):
    """The sets of things joined by the pairs, in the order first met."""
    linked = collections.defaultdict(set)
    for a, b in pairs:
        linked[a].add(b)
        linked[b].add(a)
    seen, groups = set(), []
    for net in linked:
        if net in seen:
            continue
        group, stack = set(), [net]
        while stack:
            x = stack.pop()
            if x not in group:
                group.add(x)
                stack.extend(linked[x])
        seen |= group
        groups.append(group)
    return groups


def bus_measures(dump, pairs):
    """One routing over the bus pairs of its board: a Tally of pairs, missing (a net without tracks), and the
    sums over pairs of alongside, same vias, crossing, mismatch."""
    tally = Tally()
    by, vias = tracks_by_net(dump), vias_by_net(dump)
    lines = {n: chords(ts) for n, ts in by.items()}
    length = {n: sum(t["len"] for t in ts if t["a"] != t["b"]) for n, ts in by.items()}
    for a, b, pitch in pairs:
        la, lb = lines.get(a), lines.get(b)
        if not la or not lb:
            tally["missing"] += 1
            continue
        tally["pairs"] += 1
        tally["same vias"] += vias[a] == vias[b]
        tally["crossing"] += any(sa.crosses(sb) for l, ss in la.items() for sa in ss for l2, s2 in lb.items() if l2 != l for sb in s2)
        whole = MultiLineString([s for ss in la.values() for s in ss]).length
        tally["alongside"] += share_near(la, lb, BUS_REACH * pitch) / max(whole, 1e-9)
        tally["mismatch"] += abs(length[a] - length[b]) / max(length[a], length[b], 1e-9)
    return tally


def bus_figures(tally):
    pairs = max(tally["pairs"], 1)
    return dict(
        pairs=tally["pairs"],
        alongside_percent=100 * tally["alongside"] / pairs,
        same_vias_percent=100 * tally["same vias"] / pairs,
        crossing_percent=100 * tally["crossing"] / pairs,
        length_mismatch_percent=100 * tally["mismatch"] / pairs,
    )


# --------------------------------------------------------------------------------------------------------------
# Circuit: power, decoupling, crystals, differential pairs

def circuit_nets(reference):
    """What the reference's net names and part references say the nets are for:
    pads      {net: its pads}
    power     nets named as supplies or ground, sorted
    decaps    [(supply net, the capacitor's pad on it, the chip pad it decouples)]
    crystals  nets on crystals, sorted
    pairs     [(first, second)] halves of differential pairs"""
    pads = collections.defaultdict(list)
    for p in reference["pads"]:
        if p["net"]:
            pads[p["net"]].append(p)
    nets = {n for n, ps in pads.items() if len(places(p["at"] for p in ps)) >= 2}
    power = {n for n in nets if is_supply(n)}
    parts = collections.defaultdict(list)
    for p in reference["pads"]:
        parts[p.get("ref", "")].append(p)
    decaps = []
    for ref, ps in parts.items():
        if CAPACITOR.match(ref) and len(ps) == 2:
            for supply, ground, pad in ((ps[0]["net"], ps[1]["net"], ps[0]), (ps[1]["net"], ps[0]["net"], ps[1])):
                if supply in power and not GROUND.search(supply) and GROUND.search(ground or ""):
                    pins = [q for q in pads[supply] if CHIP.match(q.get("ref", "")) and math.dist(q["at"], pad["at"]) <= DECAP_REACH]
                    if pins:
                        decaps.append((supply, pad, min(pins, key=lambda q: math.dist(q["at"], pad["at"]))))
    crystals = {p["net"] for ref, ps in parts.items() if CRYSTAL.match(ref) for p in ps if p["net"] in nets and p["net"] not in power}
    pairs = [(a, b) for a, b in pair_halves(nets) if a not in power and b not in power]
    return dict(pads=dict(pads), power=sorted(power), decaps=decaps, crystals=sorted(crystals), pairs=pairs)


def copper_path(dump, net, a, b):
    """(length of the net's copper from pad a to pad b along its tracks, whether that way passes a via), or
    None when no such way exists. Tracks are cut where another track of the net ends on them or a via stands on
    them; a track end inside a pad's outline (grown by 0.01 mm) on that layer is on the pad; a via joins all
    layers at no length."""
    graph = collections.defaultdict(list)

    def edge(u, v, length, via=False):
        graph[u].append((v, length, via))
        graph[v].append((u, length, via))

    mine = [t for t in dump["tracks"] if t["net"] == net]
    stands = [key(v["at"]) for v in dump["vias"] if v["net"] == net]
    points = collections.defaultdict(set)  # per layer: where tracks end and vias stand
    for t in mine:
        points[t["layer"]] |= {key(t["a"]), key(t["b"])}
    for at in stands:
        for l in dump["layers"]:
            points[l].add(at)
    ends = []
    for t in mine:
        ta, tb = key(t["a"]), key(t["b"])
        L = math.dist(ta, tb)
        if L < 1e-6:
            on = [ta, tb]
        else:
            on = sorted({q for q in points[t["layer"]] if abs(math.dist(ta, q) + math.dist(q, tb) - L) < 2e-3} | {ta, tb}, key=lambda q: math.dist(ta, q))
        for q, r in zip(on, on[1:]):
            edge((t["layer"],) + q, (t["layer"],) + r, math.dist(q, r) if L > 1e-6 and "mid" not in t else t["len"])
        ends += [(t["layer"],) + q for q in on]
    for at in stands:
        for l1 in dump["layers"]:
            for l2 in dump["layers"]:
                if l1 < l2:
                    edge((l1,) + at, (l2,) + at, 0.0, True)
    ends = set(ends)
    for pad in dump["pads"]:
        if pad["net"] != net:
            continue
        node = ("pad",) + key(pad["at"])
        for layer, rings in pad["shapes"].items():
            if not rings or len(rings[0]) < 3:
                continue
            outline = Polygon(rings[0]).buffer(0.01)
            for e in ends:
                if e[0] == layer and outline.contains(Point(e[1], e[2])):
                    edge(node, e, 0.0)
    source, target = ("pad",) + key(a["at"]), ("pad",) + key(b["at"])
    seen, heap = set(), [(0.0, False, source)]
    while heap:
        dist, via, u = heapq.heappop(heap)
        if u in seen:
            continue
        seen.add(u)
        if u == target:
            return dist, via
        for v, length, is_via in graph[u]:
            if v not in seen:
                heapq.heappush(heap, (dist + length, via or is_via, v))
    return None


def circuit_measures(dump, circuit, poured, rules, is_reference=False):
    """One routing over what circuit_nets found in its board's reference. `poured` are the reference's zone
    nets, `rules` as in a dump. A Tally of sums; its samples are the decoupling path over straight ratios."""
    tally = Tally()
    by, vias, pads = tracks_by_net(dump), vias_by_net(dump), circuit["pads"]
    length = lambda n: sum(t["len"] for t in by[n])
    for n in circuit["power"]:
        if n in poured or not by.get(n):
            continue
        width = class_width(rules, n)
        tally["power nets"] += 1
        tally["power mm"] += length(n)
        tally["power narrow mm"] += sum(t["len"] for t in by[n] if t["w"] < width - 1e-6)
        tally["power vias"] += vias[n]
        tree, count = tree_length(p["at"] for p in pads[n])
        tally["power connections"] += count - 1
        tally["power tree"] += tree
    for supply, pad, pin in circuit["decaps"]:
        if supply in poured and is_reference:
            continue
        tally["decaps"] += 1
        way = copper_path(dump, supply, pad, pin)
        if way is None:
            tally["decap not joined directly"] += 1
            continue
        tally.samples.append(way[0] / max(math.dist(pad["at"], pin["at"]), 1e-6))
        tally["decap via between"] += way[1]
    for n in circuit["crystals"]:
        if not by.get(n):
            continue
        tally["crystal nets"] += 1
        tally["crystal mm"] += length(n)
        tally["crystal tree"] += tree_length(p["at"] for p in pads[n])[0]
        tally["crystal vias"] += vias[n]
    for a, b in circuit["pairs"]:
        if not by.get(a) or not by.get(b) or a in poured or b in poured:
            continue
        reach = PAIR_REACH * max(t["w"] for t in by[a])
        tally["pairs"] += 1
        tally["pair coupled"] += share_near(chords(by[a]), chords(by[b]), reach) / max(length(a), 1e-9)
        tally["pair mismatch"] += abs(length(a) - length(b)) / max(length(a), length(b), 1e-9)
        tally["pair same vias"] += vias[a] == vias[b]
    return tally


def circuit_figures(tally):
    over = lambda a, b: tally[a] / max(tally[b], 1e-9)
    return dict(
        power_nets=tally["power nets"],
        power_narrow_percent=100 * over("power narrow mm", "power mm"),
        power_vias_per_100=100 * over("power vias", "power connections"),
        power_detour=over("power mm", "power tree"),
        decaps=tally["decaps"],
        decap_path_over_straight_median=upper_median(tally.samples),
        decap_via_between_percent=100 * tally["decap via between"] / max(len(tally.samples), 1),
        decap_not_joined_directly_percent=100 * over("decap not joined directly", "decaps"),
        crystal_nets=tally["crystal nets"],
        crystal_detour=over("crystal mm", "crystal tree"),
        crystal_vias=tally["crystal vias"],
        pairs=tally["pairs"],
        pair_coupled_percent=100 * over("pair coupled", "pairs"),
        pair_mismatch_percent=100 * over("pair mismatch", "pairs"),
        pair_same_vias_percent=100 * over("pair same vias", "pairs"),
    )


# --------------------------------------------------------------------------------------------------------------
# Pad exits

def long_direction(outline):
    """The unit direction of a pad's length when it is at least 1.5 times as long as wide, else None."""
    box = outline.minimum_rotated_rectangle.exterior.coords[:]
    e1, e2 = math.dist(box[0], box[1]), math.dist(box[1], box[2])
    if max(e1, e2) < 1.5 * min(e1, e2):
        return None
    a, b = (box[0], box[1]) if e1 > e2 else (box[1], box[2])
    return ((b[0] - a[0]) / max(e1, e2), (b[1] - a[1]) / max(e1, e2))


def leaving_segment(outline, start, first, onward):
    """The segment (from, to) with which a track that starts at a pad's centre towards `first` leaves the pad's
    outline, following it for up to six segments; None if it branches or ends before leaving. `onward(p)` gives
    the far ends of the segments that end at p."""
    prev, cur = start, first
    for _ in range(6):
        if not outline.contains(Point(cur)):
            break
        ways = [q for q in onward(cur) if key(q) != key(prev)]
        if len(ways) != 1:
            return None
        prev, cur = cur, ways[0]
    return None if outline.contains(Point(cur)) else (prev, cur)


def crossed_edge(outline, prev, cur):
    """The direction (not unit) of the edge of the outline that the segment prev-cur crosses; None if it
    crosses none."""
    hit = LineString([prev, cur]).intersection(outline.exterior)
    if hit.is_empty:
        return None
    x = hit if hit.geom_type == "Point" else list(hit.geoms)[0] if hasattr(hit, "geoms") else hit.representative_point()
    if x.geom_type != "Point":
        x = x.representative_point()
    ring = outline.exterior.coords[:]
    i = min(range(len(ring) - 1), key=lambda i: LineString([ring[i], ring[i + 1]]).distance(x))
    return ring[i + 1][0] - ring[i][0], ring[i + 1][1] - ring[i][1]


def exit_measures(dump, nets):
    """One routing: a Tally of exits, grazing, long pads (exits from long pads), through long side. `nets` are
    the nets whose pads count (tracked_nets of the reference)."""
    tally = Tally()
    far = collections.defaultdict(list)  # (net, layer, place) -> the other ends of the straight segments ending there
    for t in dump["tracks"]:
        if "mid" in t or t["a"] == t["b"]:
            continue
        far[(t["net"], t["layer"]) + key(t["a"])].append(t["b"])
        far[(t["net"], t["layer"]) + key(t["b"])].append(t["a"])
    for pad in dump["pads"]:
        n = pad["net"]
        if not n or n not in nets:
            continue
        for layer, rings in pad["shapes"].items():
            if not rings or len(rings[0]) < 3:
                continue
            outline = Polygon(rings[0]).buffer(0)
            if outline.is_empty:
                continue
            along = long_direction(outline)
            onward = lambda p: far.get((n, layer) + key(p), [])
            for first in onward(pad["at"]):
                leaving = leaving_segment(outline, pad["at"], first, onward)
                if leaving is None:
                    continue
                prev, cur = leaving
                edge = crossed_edge(outline, prev, cur)
                if edge is None:
                    continue
                ex, ey = edge
                tx, ty = cur[0] - prev[0], cur[1] - prev[1]
                el, tl = math.hypot(ex, ey), math.hypot(tx, ty)
                if el < 1e-9 or tl < 1e-9:
                    continue
                angle = math.degrees(math.acos(min(1, abs(ex * tx + ey * ty) / (el * tl))))  # 90: square to the edge
                tally["exits"] += 1
                tally["grazing"] += angle < 40
                if along:
                    tally["long pads"] += 1
                    tally["through long side"] += abs(ex * along[0] + ey * along[1]) / el > 0.7
    return tally


def exit_figures(tally):
    return dict(
        exits=tally["exits"],
        grazing_percent=100 * tally["grazing"] / max(tally["exits"], 1),
        long_pad_exits=tally["long pads"],
        through_long_side_percent=100 * tally["through long side"] / max(tally["long pads"], 1),
    )
