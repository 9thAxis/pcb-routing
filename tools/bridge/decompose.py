"""A board's work as steps: what is to be joined, in the order a designer would take it. Read only: nothing
here lays a track.

A board is not handed to the router whole. Its parts stand in blocks (the sheets of its schematic), and its
nets are of different kinds: those within one block, short and many; the few that must be laid with care
between blocks (pairs, clocks, what is sensitive); buses; the rest; the supplies. `decompose` sorts them so,
and gives each step the nets it joins, the ground it works in and what it has left to do.
"""

import math
from dataclasses import dataclass, field

from .beside import buses

# The kinds of step, in the order they are taken.
WITHIN, SIDE_BY_SIDE, CAREFUL, TOGETHER, BETWEEN, RAIL = "local", "pair", "careful", "bus", "link", "supply"
STEP_ORDER = [WITHIN, SIDE_BY_SIDE, CAREFUL, TOGETHER, BETWEEN, RAIL]
STEP_WHY = {WITHIN: "the nets within one block: short, and they fix where the block is entered and left",
       SIDE_BY_SIDE: "two nets laid side by side from one block to another: laid before what would cross their way",
       CAREFUL: "a clock, a crystal or a sensitive net between blocks: it takes its way first",
       TOGETHER: "nets that run together from one part to another: laid together, in the order of their pads",
       BETWEEN: "the other nets from one block to another",
       RAIL: "a supply or a ground that no zone joins: tracks, last, round what is laid"}
CAREFUL_ROLES = ("clock", "crystal", "sensitive")


@dataclass
class Block:
    """The parts of one sheet of the schematic that stand on the board."""
    name: str  # the sheet's name
    file: str = ""
    parts: list = field(default_factory=list)  # references, those on the board
    parked: list = field(default_factory=list)  # those not yet placed: off the board
    box: list = None  # [x0, y0, x1, y1] of the parts on the board; None with none


@dataclass
class Step:
    """One piece of the work."""
    kind: str
    name: str  # what it is, for a reader
    nets: list
    blocks: list  # the names of the blocks it touches
    open: int  # connections it has still to make (a net of n pieces has n - 1)
    span: float  # how long its open connections are at the least, in mm: from piece to nearest piece
    box: list  # the ground its pads span
    layers: list  # the copper layers its pads are on


@dataclass
class Plan:
    blocks: list  # [Block], those with a part on the board first
    steps: list  # [Step], in order; a step with nothing open is done
    waiting: dict  # net -> the parts it waits for: not placed yet
    joined: int  # nets all joined already
    by_zone: list  # supplies and grounds that a zone joins: nothing to lay but what the zone leaves


def on_board(picture, at):
    x0, y0, x1, y1 = picture.box
    return x0 <= at[0] <= x1 and y0 <= at[1] <= y1


SHEET_APART = 3.0  # parts of one sheet are two blocks where they stand this many times further apart than its parts do


def _clusters(at):
    """Points as the clusters they stand in: two are in one if a chain of points joins them with no step
    longer than SHEET_APART times what is usual from a point to its nearest (the median). Indices, per cluster."""
    if len(at) < 3:
        return [list(range(len(at)))]
    nearest = sorted(min(math.dist(p, q) for k, q in enumerate(at) if k != i) for i, p in enumerate(at))
    reach = SHEET_APART * max(nearest[len(nearest) // 2], 1e-6)
    left, found = set(range(len(at))), []
    while left:
        cluster, edge = [], [min(left)]
        left.discard(edge[0])
        while edge:
            i = edge.pop()
            cluster.append(i)
            near = [j for j in left if math.dist(at[i], at[j]) <= reach]
            left.difference_update(near)
            edge += near
        found.append(sorted(cluster))
    return sorted(found, key=lambda cluster: -len(cluster))


def blocks_of(picture):
    """The blocks of a board: its parts by the sheet they are drawn on (parts on no sheet are one block). A
    sheet whose parts stand in separate places is a block for each place (the filter of an input drawn on the
    input's sheet and placed by the converter): what joins them is then a net between blocks, not within one."""
    sheets = {}
    for reference, part in sorted(picture.parts.items()):
        sheets.setdefault((part.file or "", part.sheet or "(on no sheet)"), []).append(reference)
    found = []
    for (file, name), references in sheets.items():
        placed = [r for r in references if on_board(picture, picture.parts[r].at)]
        parked = [r for r in references if r not in placed]
        clusters = _clusters([picture.parts[r].at for r in placed]) if placed else [[]]
        for number, cluster in enumerate(clusters):
            parts = [placed[i] for i in cluster]
            at = [picture.parts[r].at for r in parts]
            box = [min(p[0] for p in at), min(p[1] for p in at), max(p[0] for p in at), max(p[1] for p in at)] if at else None
            where = f" (at {(box[0] + box[2]) / 2:.0f}, {(box[1] + box[3]) / 2:.0f})" if len(clusters) > 1 else ""
            found.append(Block(name=name + where, file=file, parts=parts, parked=parked if number == 0 else [], box=box))
    return sorted(found, key=lambda block: (not block.parts, block.name))


def _span(picture, net):
    """What a net has still to join, at the least: each piece to the nearest of those joined before it."""
    pieces = [[picture.pads[i].at for i in group] for group in picture.groups[net]]
    apart = lambda one, other: min(math.dist(p, q) for p in one for q in other)
    joined, left, total = pieces[:1], pieces[1:], 0.0
    while left:
        far, nearest = min(((min(apart(piece, other) for other in joined), k) for k, piece in enumerate(left)))
        total += far
        joined.append(left.pop(nearest))
    return total


def decompose(picture, rules):
    """The work of a board as a Plan (see the module). `rules` says what the nets are for (see Rules.intent)."""
    blocks = blocks_of(picture)
    block_of = {reference: block.name for block in blocks for reference in block.parts + block.parked}
    parked = {reference for block in blocks for reference in block.parked}
    pads_of = lambda net: [picture.pads[i] for group in picture.groups[net] for i in group]
    waiting, joined, by_zone, open_nets = {}, 0, [], []
    for net in sorted(picture.groups):
        missing = sorted({pad.part for pad in pads_of(net) if pad.part in parked})
        role = rules.intent(net).role
        if missing:
            waiting[net] = missing
        elif len(picture.groups[net]) < 2:
            joined += 1
            if net in picture.pours:
                by_zone.append(net)
        else:
            open_nets.append(net)
            if net in picture.pours:
                by_zone.append(net)
    touched = lambda net: sorted({block_of.get(pad.part, "(no part)") for pad in pads_of(net)})
    runs_with = buses(picture.pads, set(open_nets)) if open_nets else {}

    def step(kind, name, nets):
        nets = sorted(nets)
        at = [pad.at for net in nets for pad in pads_of(net)]
        on = {layer for net in nets for pad in pads_of(net) for layer in pad.layers}
        return Step(kind=kind, name=name, nets=nets, blocks=sorted({b for net in nets for b in touched(net)}),
                    open=sum(len(picture.groups[net]) - 1 for net in nets),
                    span=round(sum(_span(picture, net) for net in nets), 1),
                    box=[round(v, 2) for v in (min(p[0] for p in at), min(p[1] for p in at),
                                               max(p[0] for p in at), max(p[1] for p in at))],
                    layers=[layer for layer in picture.layers if layer in on])

    lots = {kind: {} for kind in STEP_ORDER}  # kind -> {name of the step: its nets}
    for net in open_nets:
        intent, where = rules.intent(net), touched(net)
        other = intent.pair if isinstance(intent.pair, str) and intent.pair in open_nets else None
        if intent.role in ("supply", "ground") or net in picture.pours:
            lots[RAIL].setdefault(net, []).append(net)
        elif len(where) == 1:
            lots[WITHIN].setdefault(where[0], []).append(net)
        elif other:
            lots[SIDE_BY_SIDE].setdefault(" = ".join(sorted((net, other))), []).append(net)
        elif intent.role in CAREFUL_ROLES:
            lots[CAREFUL].setdefault(f"{intent.role} {net}", []).append(net)
        elif net in runs_with or isinstance(intent.bus, str):
            lots[TOGETHER].setdefault(f"bus {intent.bus if isinstance(intent.bus, str) else runs_with[net][0]}", []).append(net)
        else:
            lots[BETWEEN].setdefault(" to ".join(where), []).append(net)
    steps = []
    for kind in STEP_ORDER:
        found = [step(kind, name, nets) for name, nets in lots[kind].items()]
        # Within a kind: the blocks with the most to join first (they are the crowded ones); what goes furthest
        # between blocks first (it has the fewest ways).
        key = (lambda s: (-s.open, s.name)) if kind in (WITHIN, RAIL) else (lambda s: (-s.span, s.name))
        steps += sorted(found, key=key)
    return Plan(blocks=blocks, steps=steps, waiting=waiting, joined=joined, by_zone=by_zone)


def plan_told(plan, every=False):
    """A plan as lines to read. `every`: with the nets of each step."""
    placed = [block for block in plan.blocks if block.parts]
    lines = [f"{sum(len(b.parts) for b in plan.blocks)} parts on the board in {len(placed)} blocks; "
             f"{sum(len(b.parked) for b in plan.blocks)} parts not placed yet",
             f"{plan.joined} nets joined already; {sum(len(s.nets) for s in plan.steps)} nets to join in "
             f"{len(plan.steps)} steps ({sum(s.open for s in plan.steps)} connections, "
             f"{sum(s.span for s in plan.steps):.0f} mm at the least); {len(plan.waiting)} nets wait for parts to be placed",
             ""]
    number = 0
    for kind in STEP_ORDER:
        of_kind = [s for s in plan.steps if s.kind == kind]
        if not of_kind:
            continue
        lines.append(f"{kind}: {STEP_WHY[kind]}")
        for s in of_kind:
            number += 1
            x0, y0, x1, y1 = s.box
            where = "" if kind == WITHIN else f" [{'; '.join(s.blocks)}]"
            lines.append(f"  {number:3}. {s.name}{where}: {len(s.nets)} nets, {s.open} connections, {s.span:.0f} mm, "
                         f"x {x0:.0f} to {x1:.0f}, y {y0:.0f} to {y1:.0f}, pads on {', '.join(s.layers)}")
            if every:
                lines.append(f"       {', '.join(s.nets)}")
        lines.append("")
    if plan.waiting:
        lines.append("waiting for parts to be placed:")
        parts = sorted({part for missing in plan.waiting.values() for part in missing})
        sheets = [f"{block.name} ({len(block.parked)} of {len(block.parts) + len(block.parked)} parts)"
                  for block in plan.blocks if block.parked]
        lines += [f"  {len(plan.waiting)} nets for {len(parts)} parts of: " + "; ".join(sheets)]
        if every:
            lines += [f"  {net}: {', '.join(missing)}" for net, missing in sorted(plan.waiting.items())]
    return lines
