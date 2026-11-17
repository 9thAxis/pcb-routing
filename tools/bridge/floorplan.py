"""Where the parts of a board belong together: zones of sheets, the parts of a sheet that are tied elsewhere,
and what flows between zones. Read only: nothing here moves a part.

The schematic says which parts are drawn together (its sheets). It does not say which sheets belong side by
side on the board, nor that a part drawn on one sheet works with the parts of another. Both follow from the
nets: parts are tied by the signal nets they share (not by supplies and grounds, which tie everything to
everything), and groups that are tied more within than without are found by the method of Blondel et al.
(Louvain), level by level: sheets into zones, zones into larger zones.
"""

import math
from dataclasses import dataclass, field

from .power import power


def gathered(groups, ties):
    """Groups of nodes after one round of local moves: each group, as one, goes to the neighbouring group that
    raises the modularity most, until none moves. `ties`: {(a, b): weight} between nodes, a < b. The groups
    that end together are one, in the order of the first."""
    index = {node: g for g, members in enumerate(groups) for node in members}
    between = {}
    for (a, b), weight in ties.items():
        if a in index and b in index:
            key = (min(index[a], index[b]), max(index[a], index[b]))
            between[key] = between.get(key, 0.0) + weight
    merged = {}
    for g, to in enumerate(_moved(len(groups), between)):
        merged.setdefault(to, []).extend(groups[g])
    return [merged[to] for to in sorted(merged, key=lambda to: min(index[node] for node in merged[to]))]


def communities(nodes, ties):
    """Nodes in groups that are tied more within than without: [[node]], the largest first. Rounds of local
    moves (see gathered), each on the groups of the round before, until a round changes nothing. The order
    of the nodes given decides between equals, so the same question has the same answer."""
    groups = [[node] for node in nodes]
    while True:
        found = gathered(groups, ties)
        if len(found) == len(groups):
            return sorted(groups, key=lambda members: (-len(members), nodes.index(members[0])))
        groups = found


def _moved(count, between):
    """Which group each of `count` nodes ends in after local moves; `between`: {(i, j): weight}, i <= j (i == j:
    what a node, itself a group of an earlier pass, has within)."""
    beside = [{} for _ in range(count)]
    degree = [0.0] * count
    for (i, j), weight in between.items():
        if i == j:
            degree[i] += 2 * weight
        else:
            beside[i][j] = beside[i].get(j, 0.0) + weight
            beside[j][i] = beside[j].get(i, 0.0) + weight
            degree[i] += weight
            degree[j] += weight
    total = sum(degree)
    if total <= 0:
        return list(range(count))
    home, weight_of = list(range(count)), list(degree)
    for _ in range(100):
        changed = False
        for node in range(count):
            here = home[node]
            to = {}
            for other, weight in beside[node].items():
                to[home[other]] = to.get(home[other], 0.0) + weight
            weight_of[here] -= degree[node]
            gain = lambda group: to.get(group, 0.0) - weight_of[group] * degree[node] / total
            best = max(sorted(to), key=gain, default=here)
            if gain(best) <= gain(here) + 1e-12:
                best = here
            weight_of[best] += degree[node]
            if best != here:
                home[node], changed = best, True
        if not changed:
            break
    return home


@dataclass
class Zone:
    """Parts that belong together on the board: a group of a sheet's parts, or zones that are tied together."""
    name: str
    sheets: list  # the names of the sheets its parts are drawn on
    parts: list
    placed: int  # of them, those on the board
    box: list  # [x0, y0, x1, y1] of those; None with none
    area: float  # mm2 its parts take, pads and the ground between them: what it needs at the least
    inside: list = field(default_factory=list)  # the zones it is made of, where it is made of several
    within: int = 0  # signal nets that stay within it


@dataclass
class Floorplan:
    zones: list  # [Zone]: the domains (the parts on one ground), each with the zones within it
    flows: list  # [(zone, zone, nets between them, of which pairs)], the heaviest first
    apart: list  # [(sheet, [(the parts of one group of it, the top zone they are in)])]: sheets drawn as one, tied apart
    strays: list  # [(part, its zone, mm from the middle of the zone's placed parts, how far the zone reaches)]
    power: object = None  # the domains and supplies these zones stand on (see power.py)


def signal_nets(picture, rules, found):
    """The nets that say which parts work together: not the grounds and supplies of `found` (see power.py),
    nor what the names or a zone make one; each as the parts it has pads on."""
    rails = {domain.ground for domain in found.domains} | {rail.net for domain in found.domains for rail in domain.rails}
    nets = {}
    for net, groups in picture.groups.items():
        if net in rails or rules.intent(net).role in ("supply", "ground") or net in picture.pours:
            continue
        parts = sorted({picture.pads[i].part for group in groups for i in group if picture.pads[i].part})
        if len(parts) > 1:
            nets[net] = parts
    return nets


def _tied(nets, of):
    """Ties between things (`of`: part -> thing) from nets: a net of k things ties each two of them by
    1 / (k - 1), so that a net counts once for each thing it reaches whatever its size."""
    ties = {}
    for parts in nets.values():
        things = sorted({of[part] for part in parts if part in of})
        for i, a in enumerate(things):
            for b in things[i + 1:]:
                ties[(a, b)] = ties.get((a, b), 0.0) + 1.0 / (len(things) - 1)
    return ties


def _ground(picture, parts):
    """What parts take on the board, in mm2: the box of each part's pads, grown by a millimetre on every side."""
    boxes = {}
    for pad in picture.pads:
        if pad.part in parts:
            x, y = pad.at
            box = boxes.setdefault(pad.part, [x, y, x, y])
            box[:] = [min(box[0], x), min(box[1], y), max(box[2], x), max(box[3], y)]
    return sum((box[2] - box[0] + 2.0) * (box[3] - box[1] + 2.0) for box in boxes.values())


def floorplan(picture, rules):
    """The zones of a board, what flows between them, the sheets whose parts are tied apart, and the parts that
    stand far from their zone (see the module).

    First the domains: the parts on one ground (see power.py). A part on two grounds is counted with the one
    most of its pads are on, and named among what stands between domains. Within a domain, the smallest zones
    are the groups within each sheet (its parts that are tied to each other more than to its other parts; a
    part that no signal net ties to anything goes with the largest). Those are gathered into zones, and zones
    into larger ones, level by level, whatever sheet they are drawn on: the filter of an input, drawn with the
    input, comes out with the converter it feeds."""
    x0, y0, x1, y1 = picture.box
    on_board = lambda part: x0 <= picture.parts[part].at[0] <= x1 and y0 <= picture.parts[part].at[1] <= y1
    sheet_of = {part: picture.parts[part].sheet or "(on no sheet)" for part in picture.parts}
    found = power(picture)
    nets = signal_nets(picture, rules, found)
    between_parts = _tied(nets, {part: part for part in picture.parts})

    def zone(name, parts, inside=()):
        among = set(parts)
        at = [picture.parts[part].at for part in parts if on_board(part)]
        box = [min(p[0] for p in at), min(p[1] for p in at), max(p[0] for p in at), max(p[1] for p in at)] if at else None
        return Zone(name=name, sheets=sorted({sheet_of[part] for part in parts}), parts=sorted(parts), placed=len(at),
                    box=box, area=round(_ground(picture, among)), inside=list(inside),
                    within=sum(1 for of_net in nets.values() if set(of_net) <= among))

    apart, named_first = [], {}

    def within(domain_parts):
        """The zones of the parts of one domain: the top level, each with those it is made of."""
        mine = set(domain_parts)
        atoms, atom_of = [], {}  # the groups within the sheets: (sheet, its parts)
        for sheet in sorted({sheet_of[part] for part in mine}):
            parts = sorted(part for part in mine if sheet_of[part] == sheet)
            own = {pair: weight for pair, weight in between_parts.items()
                   if pair[0] in mine and pair[1] in mine and sheet_of[pair[0]] == sheet == sheet_of[pair[1]]}
            tied = {part for pair in own for part in pair}
            groups = communities([part for part in parts if part in tied], own) or [[]]
            groups[0] = sorted(groups[0] + [part for part in parts if part not in tied])
            for group in groups:
                atom_of.update({part: len(atoms) for part in group})
                atoms.append((sheet, sorted(group)))

        def named(members):
            """A zone by the sheet most of its parts are drawn on; and by its first parts, if it is not all of it."""
            count = {}
            for atom in members:
                count[atoms[atom][0]] = count.get(atoms[atom][0], 0) + len(atoms[atom][1])
            sheet = max(sorted(count), key=lambda name: count[name])
            whole = count[sheet] == sum(1 for part in sheet_of if sheet_of[part] == sheet)
            of_sheet = sorted(part for atom in members for part in atoms[atom][1] if atoms[atom][0] == sheet)
            some = "" if whole else f" ({', '.join(of_sheet[:3])}{' ...' if len(of_sheet) > 3 else ''})"
            more = len(count) - 1
            return f"{sheet}{some}" + (f" and {more} more" if more else "")

        ties = _tied({net: [part for part in parts if part in mine] for net, parts in nets.items()}, atom_of)
        levels = [[[atom] for atom in range(len(atoms)) if atoms[atom][1]]]
        while True:
            gathered_now = gathered(levels[-1], ties)
            if len(gathered_now) == len(levels[-1]) or len(gathered_now) < 2:
                break
            levels.append(gathered_now)
        parts_of = lambda members: [part for atom in members for part in atoms[atom][1]]
        kept = {tuple(members): zone(named(members), parts_of(members)) for members in levels[0]}
        for level in levels[1:]:
            made = {}
            for members in level:
                inside = [z for under, z in kept.items() if set(under) <= set(members)]
                made[tuple(members)] = inside[0] if len(inside) == 1 else zone(named(members), parts_of(members), inside)
            kept = made
        # A sheet whose groups are not gathered together at the first level: drawn as one, tied apart.
        first = levels[1] if len(levels) > 1 else levels[0]
        first_of = {atom: g for g, members in enumerate(first) for atom in members}
        for sheet in sorted({sheet_of[part] for part in mine}):
            of_sheet = [atom for atom in range(len(atoms)) if atoms[atom][0] == sheet and atoms[atom][1]]
            if len({first_of[atom] for atom in of_sheet}) > 1:
                apart.append((sheet, [(atoms[atom][1], named(first[first_of[atom]])) for atom in of_sheet]))
        return sorted(kept.values(), key=lambda z: -len(z.parts))

    zones = [zone(domain.ground, domain.parts, within(domain.parts)) for domain in found.domains]
    if found.adrift:
        zones.append(zone("(on no ground)", found.adrift, within(found.adrift)))

    # What flows: between the zones just under the domains, and between the domains themselves.
    home = {}
    for top in zones:
        for inner in top.inside or [top]:
            home.update({part: f"{top.name}: {inner.name}" if top.inside else top.name for part in inner.parts})
    flows = {}
    for net, parts in nets.items():
        reached = sorted({home[part] for part in parts if part in home})
        paired = isinstance(rules.intent(net).pair, str)
        for i, a in enumerate(reached):
            for b in reached[i + 1:]:
                count, pairs = flows.get((a, b), (0, 0))
                flows[(a, b)] = (count + 1, pairs + (1 if paired else 0))
    flows = sorted(((a, b, count, pairs // 2) for (a, b), (count, pairs) in flows.items()), key=lambda f: (-f[2], f[0], f[1]))

    # A part that stands far from the rest of its zone: further from their middle than the zone reaches.
    strays = []
    for top in zones:
        for z in top.inside or [top]:
            at = {part: picture.parts[part].at for part in z.parts if on_board(part)}
            if len(at) < 4:
                continue
            middle = [sorted(p[k] for p in at.values())[len(at) // 2] for k in (0, 1)]
            far = sorted(math.dist(p, middle) for p in at.values())
            reach = far[(3 * len(far)) // 4] * 2  # (twice what three quarters of its parts are within)
            strays += [(part, z.name, round(math.dist(p, middle), 1), round(reach, 1)) for part, p in sorted(at.items())
                       if math.dist(p, middle) > reach]
    return Floorplan(zones=zones, flows=flows, apart=apart, strays=strays, power=found)


def floorplan_told(plan, depth=2, every=False):
    """A floorplan as lines to read: the zones down to `depth` levels. `every`: all levels, every flow, and the
    parts of the smallest zones."""
    lines = [f"{len(plan.zones)} zones; {len(plan.apart)} sheets whose parts are tied apart; "
             f"{len(plan.strays)} parts placed far from their zone", ""]
    def tell(z, level):
        where = (f"{z.placed} of {len(z.parts)} placed, x {z.box[0]:.0f} to {z.box[2]:.0f}, y {z.box[1]:.0f} to {z.box[3]:.0f}"
                 if z.box else f"none of {len(z.parts)} placed")
        lines.append(f"{'  ' * level}{z.name}: {len(z.parts)} parts, {z.area} mm2 at the least, {z.within} nets within; {where}")
        if every and not z.inside:
            lines.append(f"{'  ' * level}    {', '.join(z.parts)}")
        if every or level + 1 < depth:
            for inner in sorted(z.inside, key=lambda inner: -len(inner.parts)):
                tell(inner, level + 1)
    for z in plan.zones:
        tell(z, 0)
    lines += ["", "what flows between zones (the nets from one to the other; the heaviest are to be neighbours):"]
    for a, b, count, pairs in plan.flows[:None if every else 15]:
        lines.append(f"  {count:3} nets{f', {pairs} pairs' if pairs else ''}: {a}  <->  {b}")
    if plan.apart:
        lines += ["", "sheets drawn as one whose parts are tied apart (each group is placed with what it is tied to):"]
        for sheet, groups in plan.apart[:None if every else 15]:
            lines.append(f"  {sheet}:")
            for parts, zone in groups:
                lines.append(f"    {', '.join(parts[:6])}{' ...' if len(parts) > 6 else ''}  with  {zone}")
    if plan.strays:
        lines += ["", "parts placed far from the rest of their zone:"]
        for part, zone, far, reach in plan.strays[:None if every else 15]:
            lines.append(f"  {part} ({zone}): {far} mm from its middle, the zone reaches {reach} mm")
    return lines
