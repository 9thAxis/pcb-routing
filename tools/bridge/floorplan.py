"""Where the parts of a board belong: the question put to the engine (src/floorplan.rs works it out) and its
answer as lines to read. Read only: nothing here moves a part."""

import fnmatch

from .names import CAPACITOR_PART, DIODE_PART, FACING_PART, INDUCTOR_PART, is_supply
from .steps import habits, planes


def _pad_boxes(picture):
    boxes = {}
    for pad in picture.pads:
        if pad.part:
            x, y = pad.at
            box = boxes.setdefault(pad.part, [x, y, x, y])
            box[:] = [min(box[0], x), min(box[1], y), max(box[2], x), max(box[3], y)]
    return boxes


def bodies(picture):
    """Each part's body as it stands, [x0, y0, x1, y1]: its courtyard; where its footprint draws none, the box of
    its pads and BODY round it; a part with neither is a point."""
    pads = _pad_boxes(picture)
    found = {}
    for reference, part in picture.parts.items():
        if part.box:
            found[reference] = list(part.box)
        elif reference in pads:
            x0, y0, x1, y1 = pads[reference]
            found[reference] = [x0 - BODY, y0 - BODY, x1 + BODY, y1 + BODY]
        else:
            found[reference] = [part.at[0], part.at[1], part.at[0], part.at[1]]
    return found


# What the schematic calls a pin, as the engine knows it (see `Pin` in src/floorplan.rs).
PINS = dict(input="input", output="output", bidirectional="both", tri_state="output", passive="passive",
            power_in="power_in", power_out="power_out", open_collector="output", open_emitter="output")


BODY = 0.5  # what a part's body is taken to reach beyond the box of its pads, in mm
RUNS = 0.7  # a layer runs one way if this much of the track on it does
ENOUGH = 100.0  # and there is this much track on it to tell by, in mm


def stack_of(picture, rules):
    """What each copper layer is for, top to bottom, as the engine's `stack`: a plane (an inner layer that zones
    cover) carries nothing; an inner layer whose tracks run one way carries the long runs that way; the others
    run any way (the outer layers, and an inner layer that has little on it yet)."""
    flat, runs, found = planes(picture), habits(picture), []
    for layer in rules.routing_layers(picture.layers):
        mm, across, up = runs.get(layer, (0.0, 0.0, 0.0))
        inner = layer not in (picture.layers[0], picture.layers[-1])
        way = ("plane" if layer in flat else
               "across" if inner and mm >= ENOUGH and across >= RUNS else
               "up" if inner and mm >= ENOUGH and up >= RUNS else "any")
        found.append(dict(name=layer, way=way))
    return found


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
    pads, offsets, pins, drilled = {}, {}, {}, set()
    body = bodies(picture)
    mid = {reference: [(box[0] + box[2]) / 2, (box[1] + box[3]) / 2] for reference, box in body.items()}
    for pad in picture.pads:
        if pad.part:
            pads.setdefault(pad.part, []).append(index.get(pad.net))
            at = mid.get(pad.part, pad.at)
            offsets.setdefault(pad.part, []).append([round(pad.at[0] - at[0], 4), round(pad.at[1] - at[1], 4)])
            pins.setdefault(pad.part, []).append(PINS.get(pad.kind.split("+")[0], "unknown"))  # (less "+no_connect")
            if pad.drill is not None:
                drilled.add(pad.part)
    # (what a part takes of the board: its body; the room for tracks is what the zones are given beyond that)
    area = {part: max((box[2] - box[0]) * (box[3] - box[1]), 0.01) for part, box in bodies(picture).items()}
    placed = lambda part: x0 <= part.at[0] <= x1 and y0 <= part.at[1] <= y1
    guesses = rules.settings.guesses
    # (a part is said by the middle of its body, which is what is placed and turned; its own origin, that the
    # file places it by, is told as far from that: `anchor`; and each pad likewise: `offsets`)
    parts = [dict(name=reference, sheet=part.sheet or "", at=[round(v, 4) for v in mid[reference]], placed=placed(part),
                  edge=bool(guesses and FACING_PART.match(reference)),
                  capacitor=bool(CAPACITOR_PART.match(reference)), inductor=bool(INDUCTOR_PART.match(reference)),
                  diode=bool(DIODE_PART.match(reference)), area=round(area.get(reference, 1.0), 2),
                  pads=pads.get(reference, []), offsets=offsets.get(reference, []), pins=pins.get(reference, []),
                  anchor=[round(part.at[k] - mid[reference][k], 4) for k in (0, 1)], through=reference in drilled,
                  value=part.value or "", footprint=(part.footprint or "").split(":")[-1], angle=part.angle)
             for reference, part in sorted(picture.parts.items())]
    asked = [dict(name=net, supply=rules.intent(net).role == "supply" or is_supply(net) and rules.intent(net).role != "ground",
                  pair=isinstance(rules.intent(net).pair, str), sensitive=rules.intent(net).role == "sensitive")
             for net in nets]
    # (what a track takes across its way: the usual net's width and clearance; and the layers that carry routes)
    signal = [net for net in nets if rules.intent(net).role not in ("supply", "ground")] or nets
    takes = sorted(rules.track_width(net) + rules.clearance(net) for net in signal)
    layers = [layer for layer in rules.routing_layers(picture.layers) if layer not in planes(picture)]
    # (each closed outline is a board of its own; a rule area shut to tracks and vias on every layer is no room)
    box_of = lambda ring: [min(p[0] for p in ring), min(p[1] for p in ring), max(p[0] for p in ring), max(p[1] for p in ring)]
    boards = [box_of(polygon[0]) for polygon in picture.outline or []]
    shut = [box_of(area.rings[0]) for area in picture.areas
            if area.tracks and area.vias and set(picture.layers) <= set(area.layers) and area.rings]
    boards.sort()
    shut += [list(k) for k in (rules.settings.hints.keepouts if rules.settings.hints else [])]
    # How the boards stack, from the hints (`boards`, left to right): what is added to a place on each to say
    # where it is in the stack; and under a board that says how much it clears, the ground of every other
    # board that lies beneath it has that much height.
    hints = rules.settings.hints
    said = (hints.boards if hints else [])[:len(boards)]
    shifts = [b.get("shift", [0.0, 0.0]) for b in said] + [[0.0, 0.0]] * (len(boards) - len(said))
    limits = []
    for upper, how in enumerate(said):
        for lower, under in enumerate(boards):
            if "clear" not in how or lower == upper:
                continue
            dx, dy = (shifts[upper][k] - shifts[lower][k] for k in (0, 1))
            x0, y0, x1, y1 = boards[upper]
            over = [max(x0 + dx, under[0]), max(y0 + dy, under[1]), min(x1 + dx, under[2]), min(y1 + dy, under[3])]
            if over[0] < over[2] and over[1] < over[3]:
                limits.append(dict(rect=[round(v, 4) for v in over], height=how["clear"]))
    # (how high a part stands: by the longest beginning of its footprint's name that the hints give a height)
    def height(part):
        name = part.footprint.split(":")[-1]
        known = [prefix for prefix in (hints.heights if hints else {}) if name.startswith(prefix)]
        return hints.heights[max(known, key=len)] if known else None
    for part in parts:
        of = picture.parts[part["name"]]
        tall = height(of)
        near = [what["near"] for pattern, what in (hints.parts if hints else [])
                if "near" in what and fnmatch.fnmatchcase(part["name"], pattern)]
        mate = [what["mate"] for pattern, what in (hints.parts if hints else [])
                if "mate" in what and fnmatch.fnmatchcase(part["name"], pattern)]
        # (its body, to be settled: see `bodies`; what it takes of the board, above, is by its pads and more:
        # the tracks to it want room too)
        box = body[part["name"]]
        if box[2] > box[0] and box[3] > box[1]:
            part["size"] = [round(box[2] - box[0], 3), round(box[3] - box[1], 3)]
        part.update(**(dict(height=tall) if tall is not None else {}), **(dict(near=near[-1]) if near else {}),
                    **(dict(mate=mate[-1]) if mate else {}))
    # The height between two boards of a stack is not held to: it is what the standoffs are chosen for. The
    # plan is made as if there were room, and says how much each such ground then needs (`clear` in the hints
    # is what the stack has now, to compare).
    found = ask(dict(ask="floorplan", box=picture.box, parts=parts, nets=asked, stack=stack_of(picture, rules),
                    regions=boards if len(boards) > 1 else [], keepouts=shut,
                    shifts=shifts if len(boards) > 1 else [],
                    pitch=round(takes[len(takes) // 2], 4) if takes else 0.4, layers=max(len(layers), 1),
                    rows=(hints.rows if hints else None) or "cheap",
                    via=round(sorted(rules.via(net)[0] + rules.clearance(net) for net in signal)[len(signal) // 2], 4)))
    if not isinstance(found, dict) or "zones" not in found:
        return found
    stands = {part["name"]: part.get("height") for part in parts}
    def smallest(zone):
        return [zone] if not zone["inside"] else [leaf for inner in zone["inside"] for leaf in smallest(inner)]
    leaves = [leaf for zone in found["zones"] for leaf in smallest(zone)]
    meet = lambda a, b: min(a[2], b[2]) - max(a[0], b[0]) > 1e-6 and min(a[3], b[3]) - max(a[1], b[1]) > 1e-6
    found["clear"] = []
    for limit in limits:
        under = [(stands[name], name, leaf["name"]) for leaf in leaves if meet(leaf["rect"], limit["rect"])
                 for name in leaf["parts"] if stands.get(name) is not None]
        tallest = max(under, default=None)
        found["clear"].append(dict(rect=limit["rect"], has=limit["height"], needs=tallest[0] if tallest else 0.0,
                                   part=tallest[1] if tallest else None, zone=tallest[2] if tallest else None))
    return found


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
    if len(found.get("regions", [])) > 1:
        lines += ["", "the boards, and what each is given to hold:"]
        for region in found["regions"]:
            x0, y0, x1, y1 = region["rect"]
            lines.append(f"  x {x0:.0f} to {x1:.0f}, y {y0:.0f} to {y1:.0f}: {', '.join(region['zones'])}; its parts take "
                         f"{region['needed']:.0f} mm2 and its roads {region['roads']:.0f} of {region['room']:.0f} mm2 of room")
    roads = found.get("roads", [])
    gaps, real = [r for r in roads if r["gap"]], [r for r in roads if not r["gap"]]
    size = lambda r: (r["rect"][2] - r["rect"][0]) * (r["rect"][3] - r["rect"][1])
    if roads:
        lines += ["", f"ground kept free of parts: {len(gaps)} gaps between domains ({sum(map(size, gaps)):.0f} mm2: nothing "
                      f"crosses but through the parts that stand on them) and {len(real)} roads within them "
                      f"({sum(map(size, real)):.0f} mm2), each as wide as what crosses it takes on the layers that run its way:"]
        for road in real[:None if every else 12]:
            x0, y0, x1, y1 = road["rect"]
            one, other = (", ".join(side[:2]) + (f" and {len(side) - 2} more" if len(side) > 2 else "") for side in road["between"])
            lines.append(f"  level {road['level']}, {'up and down' if road['up'] else 'across'}: {road['width']:.1f} mm wide, "
                         f"{max(x1 - x0, y1 - y0):.0f} mm long, holds {road['lanes']} tracks, {road['used']} planned along it"
                         f"{' (TOO MANY)' if road['used'] > road['lanes'] else ''}; between [{one}] and [{other}]")
    for gap in gaps:
        one, other = (", ".join(side) for side in gap["between"])
        on_it = ", ".join(gap["parts"]) if gap["parts"] else "no part"
        lines.append(f"  gap {gap['width']:.0f} mm between [{one}] and [{other}]: {on_it} on it")
    if found.get("bridges"):
        lines += ["", "the parts on two grounds, each given its place (on the gap between its two domains, as near what it "
                      "is tied to on both sides as the others there allow):"]
        how = dict(gap="on the gap between", link="the link from board to board,", apart="NOT NEIGHBOURS:")
        for bridge in found["bridges"]:
            reach = {"gap": "", "link": f", {bridge['reach']:.0f} mm sideways from the other as the boards stack",
                     "apart": f", {bridge['reach']:.0f} mm from the other"}[bridge["on"]]
            now = "not placed yet" if bridge["off"] is None else f"{bridge['off']:.0f} mm from where it stands"
            lines.append(f"  {bridge['part']}: {how[bridge['on']]} {bridge['grounds'][0]} and {bridge['grounds'][1]}, at "
                         f"({bridge['at'][0]:.1f}, {bridge['at'][1]:.1f}){reach}; {now}")
        if found["travel"].get("cramped"):
            lines.append(f"  TOO MANY on a gap: side by side they are {found['travel']['cramped']:.0f} mm longer than it")
    if found.get("pours"):
        pours = found["pours"]
        lines += ["", f"supplies poured on {', '.join(pours[0]['layers'])} (under each of the smallest zones, the supply most of "
                      f"its parts are on): {len(pours)} supplies; {sum(len(p['by_track']) for p in pours)} parts stand under "
                      f"another's pour and are reached by a track; {sum(p['pieces'] - 1 for p in pours)} pieces more than one each"]
        for pour in pours[:None if every else 14]:
            left = (f"; by track: {', '.join(pour['by_track'][:6])}{' ...' if len(pour['by_track']) > 6 else ''}"
                    if pour["by_track"] else "")
            lines.append(f"  {pour['net']} ({pour['ground']}): under {len(pour['rects'])} zones, {pour['area']:.0f} mm2, "
                         f"{pour['parts']} parts on it, in {pour['pieces']} piece{'s' if pour['pieces'] > 1 else ''}{left}")
        if not every and len(pours) > 14:
            lines.append(f"  and {len(pours) - 14} more")
    if found.get("junctions"):
        meets = found["junctions"]
        short = [j for j in meets if j["side"] > 0]
        lines += ["", f"where ways turn from one road into another: {len(meets)} places, {sum(j['changes'] for j in meets)} layer "
                      f"changes; at {len(short)} the vias need more ground than the roads have there, and a square is kept "
                      f"free of parts ({sum(j['side'] ** 2 for j in short):.0f} mm2 in all):"]
        for j in short[:None if every else 8]:
            lines.append(f"  at ({j['at'][0]:.1f}, {j['at'][1]:.1f}): {j['changes']} layer changes, {j['needs']:.1f} mm2 for "
                         f"their vias, the roads have {j['has']:.2f}: {j['side']:.1f} mm square")
    if found.get("places"):
        places = found["places"]
        tight = [p for p in places if not p["fits"]]
        longer = 100 * (found["parts_then_mm"] / found["parts_now_mm"] - 1) if found["parts_now_mm"] else 0.0
        lines += ["", f"a place for every part within its zone: {len(places)} parts, {sum(1 for p in places if p['turn'])} "
                      f"turned; {len(tight)} found no room on the ground of their domain"
                      + (f" ({', '.join(p['part'] for p in tight[:12])}{' ...' if len(tight) > 12 else ''})" if tight else ""),
                  f"  the signal nets all of whose parts stand on the board today: {found['parts_now_mm']:.0f} mm as they "
                  f"stand, {found['parts_then_mm']:.0f} mm with each part at its place ({abs(longer):.0f} % "
                  f"{'longer' if longer > 0 else 'shorter'})",
                  f"  each then turned so that its pads face what they are joined to: from pad to the nearest other pad of "
                  f"its net, {found['pads_first_mm']:.0f} mm in all before and {found['pads_mm']:.0f} mm after; "
                  f"{found['bypass'][1]} of {found['bypass'][0]} bypass capacitors have their supply pad by the pad they are for (within 3 mm; a large one within its length)"]
    if found.get("blocks", [0])[0]:
        alike = found.get("alike", [])
        lines.append(f"  blocks (a chip or a connector with the parts that stand by its pads, laid out by themselves and placed "
                     f"as one): {found['blocks'][0]} blocks of {found['blocks'][1]} parts; {sum(a[0] for a in alike)} of them "
                     f"in {len(alike)} make-ups that repeat")
        for count, same, size, chips in alike[:None if every else 10]:
            lines.append(f"    {count} blocks of {size} parts ({', '.join(chips[:8])}{' ...' if len(chips) > 8 else ''}): "
                         + ("all laid out alike" if same == count else f"{same} laid out alike, {count - same} NOT"))
    for repeat in found.get("repeats", [])[:None if every else 10]:
        names = ", ".join(repeat["sheets"][:4]) + (f" and {len(repeat['sheets']) - 4} more" if len(repeat["sheets"]) > 4 else "")
        how = (f"{repeat['alike']} of them placed whole, laid out alike"
               + (f", {repeat['together']} side by side" if repeat.get("together") else ", each by itself")
               if repeat["alike"] else "placed part by part (not laid out alike)")
        lines.append(f"  {len(repeat['sheets'])} sheets of one make-up, {repeat['parts']} parts each ({names}): {how}")
        if repeat.get("matched"):
            lines.append(f"      their own nets, copy against copy: {repeat['matched']} nets, the longest copy of a net "
                         f"{repeat['spread_mm']:.1f} mm longer than the shortest on the average, {repeat['worst_mm']:.1f} mm at "
                         f"the worst ({repeat['worst']}); the longest such net {repeat['longest_mm']:.1f} mm; "
                         f"{repeat['shared']} nets shared by all of them, {repeat['shared_mm']:.0f} mm in all")
    if found.get("sensitive", [0])[0]:
        count, total, worst, name = found["sensitive"]
        lines.append(f"  sensitive nets: {count}, {total:.0f} mm in all, the longest {worst:.1f} mm ({name})")
    if found.get("served"):
        lines.append(f"  filter and protection parts that stand for one pad (a capacitor from a chip's input to ground or across "
                     f"two of its inputs, a diode or a part in series at a connector): {found['served'][1]} of "
                     f"{found['served'][0]} by that pad")
    if "chains" in found:
        back = found.get("back", [])
        lines.append(f"  signal chains from one chip or connector through parts in series to another: {found['chains']}; "
                     f"{len(back)} double back (from pad to pad through their parts is more than half as long again as "
                     f"straight, and 3 mm for each hop)")
        for a, b, through, direct, along in back[:None if every else 8]:
            lines.append(f"    {a} to {b} through {', '.join(through)}: {direct:.0f} mm straight, {along:.0f} mm through them")
    for loop in found.get("loops", []):
        lines.append(f"  switching regulator {loop['chip']} (switch node {loop['node']}): {', '.join(loop['parts'])} stand by it; "
                     f"the pads of its loop lie within {loop['box'][0]:.1f} x {loop['box'][1]:.1f} mm ({loop['mm2']:.0f} mm2, "
                     f"its parts take {loop['bodies']:.0f}: {'tight' if loop['tight'] else 'LOOSE'})"
                     + (f"; at its other pads: {', '.join(loop['beside'])}" if loop["beside"] else ""))
    travel, first = found.get("travel"), found.get("first")
    if travel and first and (first["mm"], first["vias"], first["over"]) != (travel["mm"], travel["vias"], travel["over"]):
        lines += ["", f"as first laid out: {first['mm']:.0f} mm, {first['vias']} layer changes, {first['over']} roads asked for "
                      "more than they hold; then those roads were widened, and each cut tried with its two sides changed over"]
    if travel:
        each = travel["vias"] / travel["nets"] if travel["nets"] else 0.0
        lines += ["", f"between domains (through the parts on the gaps, and from board to board): {travel['through_mm']:.0f} mm "
                      f"of reach; the parts that are to stand near a point are {travel['near_mm']:.0f} mm from it in all"
                      + (f"; the parts that mate are {travel['mate_mm']:.0f} mm apart as the boards stack" if travel.get("mate_mm") else "")]
        lines += [f"every net between zones of one domain given a way along the roads: {travel['nets']} nets, "
                      f"{travel['mm']:.0f} mm, {travel['vias']} layer changes ({each:.1f} a net); {travel['over']} roads asked "
                      f"for more than they hold; {travel['through']} nets go from domain to domain through the parts on "
                      f"the gaps; {travel['stranded']} could be given no way"]
    for zone, under, parts, highest in found.get("tall", []):
        lines.append(f"  TOO TALL: {zone} has {100 * under:.0f} % of its ground where there is little height, with "
                     f"{', '.join(parts[:8])}{' ...' if len(parts) > 8 else ''} ({highest:g} mm)")
    if found.get("tall"):
        lines.append(f"  (with {max(t[3] for t in found['tall']):g} mm clear above that ground, none of them would be too tall)")
    for under in found.get("clear", []):
        x0, y0, x1, y1 = under["rect"]
        lines.append(f"  under another board (x {x0:.0f} to {x1:.0f}, y {y0:.0f} to {y1:.0f}): the tallest part given ground "
                     f"there is {under['part']} at {under['needs']:g} mm ({under['zone']}); the stack has {under['has']:g} mm "
                     f"clear there now" + ("" if under["needs"] <= under["has"] else ": it needs more"))
    if found.get("stranded"):
        lines.append(f"  given no way: {', '.join(found['stranded'])}")
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
