"""Where the parts of a board belong: the question put to the engine (src/floorplan.rs works it out) and its
answer as lines to read. Read only: nothing here moves a part."""

from .names import CAPACITOR_PART, FACING_PART, is_supply


def _ground(picture):
    """What each part takes on the board, in mm2: the box of its pads, grown by a millimetre on every side."""
    boxes = {}
    for pad in picture.pads:
        if pad.part:
            x, y = pad.at
            box = boxes.setdefault(pad.part, [x, y, x, y])
            box[:] = [min(box[0], x), min(box[1], y), max(box[2], x), max(box[3], y)]
    return {part: (box[2] - box[0] + 2.0) * (box[3] - box[1] + 2.0) for part, box in boxes.items()}


def floorplan(picture, rules, ask):
    """The floorplan of a board, as the engine answers (see `Floorplan` in src/floorplan.rs): its power (the
    grounds as domains, their supplies, the parts between two domains), the zones within each domain with the
    ground each is given, what flows between them, and how far the parts stand from that today.

    Nothing is fixed where it stands. A connector or a switch belongs on an edge of the board: it moves with
    its zone and pulls the zone to the nearest edge (see names.FACING_PART; not without guesses). A net is a supply if the names or the hints say so,
    besides those the engine finds by their capacitors."""
    x0, y0, x1, y1 = picture.box
    nets = sorted(picture.groups)
    index = {net: n for n, net in enumerate(nets)}
    pads = {}
    for pad in picture.pads:
        if pad.part:
            pads.setdefault(pad.part, []).append(index.get(pad.net))
    area = _ground(picture)
    placed = lambda part: x0 <= part.at[0] <= x1 and y0 <= part.at[1] <= y1
    guesses = rules.settings.guesses
    parts = [dict(name=reference, sheet=part.sheet or "", at=part.at, placed=placed(part),
                  edge=bool(guesses and FACING_PART.match(reference)),
                  capacitor=bool(CAPACITOR_PART.match(reference)), area=round(area.get(reference, 1.0), 2),
                  pads=pads.get(reference, []))
             for reference, part in sorted(picture.parts.items())]
    asked = [dict(name=net, supply=rules.intent(net).role == "supply" or is_supply(net) and rules.intent(net).role != "ground",
                  pair=isinstance(rules.intent(net).pair, str)) for net in nets]
    return ask(dict(ask="floorplan", box=picture.box, parts=parts, nets=asked))


def power_told(found, every=False):
    """The power of a floorplan as lines to read. `every`: every supply, and all the parts between domains."""
    power, zones = found["power"], {zone["name"]: zone for zone in found["zones"]}
    def where(ground, parts):
        zone = zones.get(ground)
        if not zone or not zone["box"]:
            return f"none of {len(parts)} placed"
        x0, y0, x1, y1 = zone["box"]
        return f"{zone['placed']} of {len(parts)} placed, x {x0:.0f} to {x1:.0f}, y {y0:.0f} to {y1:.0f}"
    lines = [f"{len(power['domains'])} domains (grounds); {len(power['between'])} parts stand on two or more; "
             f"{len(power['adrift'])} parts that nothing places in one", ""]
    for domain in power["domains"]:
        lines.append(f"{domain['ground']}: {len(domain['parts'])} parts; {where(domain['ground'], domain['parts'])}")
        for rail in domain["rails"][:None if every else 8]:
            lines.append(f"    {rail['net']}: {len(rail['parts'])} parts, {rail['capacitors']} capacitors to {domain['ground']}")
        if not every and len(domain["rails"]) > 8:
            lines.append(f"    and {len(domain['rails']) - 8} more supplies")
    lines += ["", "what stands between two domains (it is placed on the gap between them):"]
    by = {}
    for part, own in power["between"]:
        by.setdefault(" | ".join(own), []).append(part)
    for own, parts in sorted(by.items(), key=lambda item: -len(item[1])):
        lines.append(f"  {own}: {len(parts)} parts: {', '.join(parts if every else parts[:12])}"
                     f"{'' if every or len(parts) <= 12 else ' ...'}")
    lines += ["", "supplies joined by a part (one feeds the other through it):"]
    for a, b, parts in power["links"][:None if every else 25]:
        lines.append(f"  {a}  --  {b}: {', '.join(parts[:6])}{' ...' if len(parts) > 6 else ''}")
    return lines


def floorplan_told(found, depth=2, every=False):
    """The zones of a floorplan as lines to read, down to `depth` levels: each with the ground it is given and
    how its parts stand to that today. `every`: all levels, every flow, and the parts of the smallest zones."""
    lines = []
    def tell(zone, level):
        x0, y0, x1, y1 = zone["rect"]
        given = f"given x {x0:.0f} to {x1:.0f}, y {y0:.0f} to {y1:.0f}"
        if zone["box"]:
            now = (f"{zone['placed']} of {len(zone['parts'])} placed, {100 * zone['at_home']:.0f} % of them there, "
                   f"their middle {zone['off']:.0f} mm off")
        else:
            now = f"none of {len(zone['parts'])} placed"
        lines.append(f"{'  ' * level}{zone['name']}: {len(zone['parts'])} parts, {zone['area']:.0f} mm2 at the least, "
                     f"{zone['within']} nets within; {given}; {now}")
        if every and not zone["inside"]:
            lines.append(f"{'  ' * level}    {', '.join(zone['parts'])}")
        if every or level + 1 < depth:
            for inner in zone["inside"]:
                tell(inner, level + 1)
    for zone in found["zones"]:
        tell(zone, 0)
    shorter = 100 * (1 - found["then_mm"] / found["now_mm"]) if found["now_mm"] else 0.0
    lines += ["", f"the signal nets between the smallest zones: {found['now_mm']:.0f} mm with each zone where its parts stand "
                  f"today, {found['then_mm']:.0f} mm with each on the ground it is given "
                  f"({abs(shorter):.0f} % {'shorter' if shorter >= 0 else 'longer'})"]
    lines += ["", "what flows between zones (the nets from one to the other; the heaviest are to be neighbours):"]
    for a, b, count, pairs in found["flows"][:None if every else 15]:
        lines.append(f"  {count:3} nets{f', {pairs} pairs' if pairs else ''}: {a}  <->  {b}")
    if found["apart"]:
        lines += ["", "sheets drawn as one whose parts are tied apart (each group is placed with what it is tied to):"]
        for sheet, groups in found["apart"][:None if every else 15]:
            lines.append(f"  {sheet}:")
            for parts, zone in groups:
                lines.append(f"    {', '.join(parts[:6])}{' ...' if len(parts) > 6 else ''}  with  {zone}")
    return lines
