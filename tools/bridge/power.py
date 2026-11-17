"""The power of a board: its grounds, the supplies that stand on each, and the parts between two grounds.
Read only.

A board is first of all its grounds. Parts on one ground are one domain: they may stand together and share
planes. Parts on two grounds are what joins two domains (an isolator, a converter, a link), and stand on the
gap between them. Within a domain the supplies say what feeds what. All of it is in the nets, whatever they
are called:

- a ground is a net named as one (GND, 0V, VSS, as a word of the name: VIB_GND, FIELD_0V);
- a supply is a net that two or more capacitors join straight to a ground (they bypass it), or one named as
  one (see names.py); a net with one such capacitor is a filtered signal, not a supply.
"""

import re
from dataclasses import dataclass, field

from .names import CAPACITOR_PART, is_supply

GROUND_WORD = re.compile(r"(^|[^A-Za-z0-9])(A|D|P)?(GND|VSS|0V|GROUND)\d*($|[^A-Za-z0-9])", re.IGNORECASE)
BYPASSED = 2  # capacitors straight to a ground that make a net a supply


@dataclass
class Rail:
    """A supply: a net that feeds parts."""
    net: str
    ground: object  # the ground its capacitors go to; None if none says
    parts: list
    capacitors: int  # those from it straight to its ground


@dataclass
class Domain:
    """The parts on one ground."""
    ground: str
    parts: list  # those on this ground alone, or mostly (see Power.between)
    rails: list  # [Rail], the one with the most parts first


@dataclass
class Power:
    domains: list  # [Domain], the largest first
    between: list  # [(part, the grounds it stands on)]: what joins two domains or more
    links: list  # [(rail, rail, the parts that stand on both)]: a regulator, a filter, a switch between two supplies
    adrift: list  # parts that no ground and no supply places in a domain
    home: dict = field(default_factory=dict)  # part -> the ground of its domain


def grounds_of(picture):
    """The nets that are grounds, by name."""
    return sorted(net for net in picture.groups if GROUND_WORD.search(net))


def power(picture):
    """The power of a board (see the module)."""
    pads_of = {}
    for pad in picture.pads:
        if pad.part and pad.net:
            pads_of.setdefault(pad.part, []).append(pad.net)
    grounds = grounds_of(picture)
    is_ground = set(grounds)
    parts_on = {}
    for part, nets in pads_of.items():
        for net in set(nets):
            parts_on.setdefault(net, []).append(part)

    # A capacitor from a net straight to a ground bypasses the net.
    bypass = {}  # net -> {ground: capacitors}
    for part, nets in pads_of.items():
        if CAPACITOR_PART.match(part) and len(nets) == 2 and len(set(nets)) == 2:
            for net, other in (nets, nets[::-1]):
                if other in is_ground and net not in is_ground:
                    bypass.setdefault(net, {})[other] = bypass.get(net, {}).get(other, 0) + 1
    rails = {}
    for net in sorted(picture.groups):
        if net in is_ground:
            continue
        to = bypass.get(net, {})
        most = max(sorted(to), key=lambda ground: to[ground], default=None)
        if (most and to[most] >= BYPASSED) or is_supply(net):
            rails[net] = Rail(net=net, ground=most, parts=sorted(parts_on.get(net, [])), capacitors=to.get(most, 0))

    # A part is on the grounds it has a pad on; without one, on those of the supplies it has a pad on.
    stands = {}
    for part, nets in pads_of.items():
        own = {net for net in nets if net in is_ground}
        own = own or {rails[net].ground for net in nets if net in rails and rails[net].ground}
        stands[part] = own
    # (and what neither says, on the ground of most of the parts it shares a net with, as far as that goes)
    for _ in range(10):
        found = {}
        for part in sorted(stands):
            if stands[part]:
                continue
            count = {}
            for net in set(pads_of[part]):
                for other in parts_on.get(net, []):
                    for ground in stands.get(other, ()):
                        count[ground] = count.get(ground, 0) + 1
            if count:
                found[part] = {max(sorted(count), key=lambda ground: count[ground])}
        if not found:
            break
        stands.update(found)

    def first(part):
        """The ground a part with several is counted with: the one most of its pads are on, or lead to."""
        count = {}
        for net in pads_of[part]:
            ground = net if net in is_ground else rails[net].ground if net in rails else None
            if ground in stands[part]:
                count[ground] = count.get(ground, 0) + 1
        return max(sorted(stands[part]), key=lambda ground: count.get(ground, 0))
    home = {part: first(part) for part in stands if stands[part]}
    between = sorted((part, sorted(own)) for part, own in stands.items() if len(own) > 1)
    domains = [Domain(ground=ground, parts=sorted(part for part in home if home[part] == ground),
                      rails=sorted((rail for rail in rails.values() if rail.ground == ground), key=lambda rail: -len(rail.parts)))
               for ground in grounds]
    domains = sorted((domain for domain in domains if domain.parts), key=lambda domain: -len(domain.parts))

    # Two supplies with a part on both that is no capacitor: one feeds the other through it.
    links = {}
    for part, nets in sorted(pads_of.items()):
        on = sorted({net for net in nets if net in rails})
        if len(on) > 1 and not CAPACITOR_PART.match(part):
            for i, a in enumerate(on):
                for b in on[i + 1:]:
                    links.setdefault((a, b), []).append(part)
    return Power(domains=domains, between=between, links=sorted((a, b, parts) for (a, b), parts in links.items()),
                 adrift=sorted(part for part in picture.parts if part not in home), home=home)


def power_told(found, picture, every=False):
    """The power of a board as lines to read. `every`: every supply, and the parts between domains by name."""
    x0, y0, x1, y1 = picture.box
    placed = lambda parts: [p for p in parts if x0 <= picture.parts[p].at[0] <= x1 and y0 <= picture.parts[p].at[1] <= y1]
    def where(parts):
        at = [picture.parts[p].at for p in placed(parts)]
        return (f"{len(at)} of {len(parts)} placed, x {min(p[0] for p in at):.0f} to {max(p[0] for p in at):.0f}, "
                f"y {min(p[1] for p in at):.0f} to {max(p[1] for p in at):.0f}") if at else f"none of {len(parts)} placed"
    lines = [f"{len(found.domains)} domains (grounds); {len(found.between)} parts stand on two or more; "
             f"{len(found.adrift)} parts that nothing places in one", ""]
    for domain in found.domains:
        lines.append(f"{domain.ground}: {len(domain.parts)} parts; {where(domain.parts)}")
        for rail in domain.rails[:None if every else 8]:
            lines.append(f"    {rail.net}: {len(rail.parts)} parts, {rail.capacitors} capacitors to {domain.ground}")
        if not every and len(domain.rails) > 8:
            lines.append(f"    and {len(domain.rails) - 8} more supplies")
    lines += ["", "what stands between two domains (it is placed on the gap between them):"]
    by = {}
    for part, own in found.between:
        by.setdefault(" | ".join(own), []).append(part)
    for own, parts in sorted(by.items(), key=lambda item: -len(item[1])):
        lines.append(f"  {own}: {len(parts)} parts" + (f": {', '.join(parts)}" if every or len(parts) <= 12 else
                                                       f": {', '.join(parts[:12])} ..."))
    lines += ["", "supplies joined by a part (one feeds the other through it):"]
    for a, b, parts in found.links[:None if every else 25]:
        lines.append(f"  {a}  --  {b}: {', '.join(parts[:6])}{' ...' if len(parts) > 6 else ''}")
    return lines
