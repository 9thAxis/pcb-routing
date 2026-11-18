"""A floorplan judged: what the engine's answer (see `Floorplan` in src/floorplan.rs) comes to on one line, and
several boards' lines summed. Read only; nothing here asks the engine or opens a board."""

TOUCH = 0.05  # two bodies are over one another if they share more than this in both directions, in mm


def bodies_over(found):
    """The pairs of parts whose bodies are over one another on one side of the board: [(part, part)]. A part told
    in pieces is over another only where a piece is; a part that found no room is not counted (it is told so)."""
    rects = [(place["part"], rect, place.get("back", False)) for place in found.get("places", [])
             if place["rect"] and place["fits"] for rect in (place.get("pieces") or [place["rect"]])]
    pairs = set()
    for i, (a, ra, side_a) in enumerate(rects):
        for b, rb, side_b in rects[i + 1:]:
            if (a != b and side_a == side_b and min(ra[2], rb[2]) - max(ra[0], rb[0]) > TOUCH
                    and min(ra[3], rb[3]) - max(ra[1], rb[1]) > TOUCH):
                pairs.add((a, b))
    return sorted(pairs)


def judged(found):
    """One board's floorplan as figures: its parts, those without room, bodies over one another, its nets
    against the board as it stands, and what was found in it."""
    places = found.get("places", [])
    return dict(parts=len(places), back=sum(1 for p in places if p.get("back")),
                no_room=sorted(p["part"] for p in places if not p["fits"]), over=len(bodies_over(found)),
                now_mm=found.get("parts_now_mm", 0.0), then_mm=found.get("parts_then_mm", 0.0),
                domains=len(found.get("power", {}).get("domains", [])), blocks=found.get("blocks", [0, 0])[0],
                repeats=len(found.get("repeats", [])), regulators=len(found.get("loops", [])),
                close=len(found.get("close", [])), chains=found.get("chains", 0), back_chains=len(found.get("back", [])),
                bypass=found.get("bypass", [0, 0]), served=found.get("served", [0, 0]))


def judged_told(rows):
    """Several boards' figures ({name: judged(...)}; None for a board that failed) as lines to read."""
    done = {name: row for name, row in rows.items() if row}
    failed = sorted(name for name, row in rows.items() if not row)
    parts = sum(row["parts"] for row in done.values())
    homeless = {name: row["no_room"] for name, row in done.items() if row["no_room"]}
    over = {name: row["over"] for name, row in done.items() if row["over"]}
    lines = [f"{len(rows)} boards, {parts} parts; {len(failed)} failed" + (f": {', '.join(failed)}" if failed else ""),
             f"parts without room: {sum(map(len, homeless.values()))} on {len(homeless)} boards; bodies over one another "
             f"on {len(over)} boards"]
    compared = sorted(row["then_mm"] / row["now_mm"] for row in done.values() if row["now_mm"] > 0)
    if compared:
        middle = compared[len(compared) // 2]
        now, then = (sum(row[k] for row in done.values() if row["now_mm"] > 0) for k in ("now_mm", "then_mm"))
        lines.append(f"against the boards as they stand ({len(compared)} boards with nets all of whose parts are placed): "
                     f"{then:.0f} mm for {now:.0f}; the middle board {middle:.2f} of its length; shorter on "
                     f"{sum(1 for r in compared if r < 1)}, more than half as long again on {sum(1 for r in compared if r > 1.5)}")
    count = lambda what: sum(1 for row in done.values() if row[what])
    lines.append(f"with more than one ground: {sum(1 for row in done.values() if row['domains'] > 1)}; with blocks: "
                 f"{count('blocks')}; with sheets that repeat: {count('repeats')}; with a switching regulator: "
                 f"{count('regulators')}; with parts on the back: {count('back')}")
    lines += [f"  no room on {name}: {', '.join(which)}" for name, which in sorted(homeless.items())]
    lines += [f"  {pairs} pairs of bodies over one another on {name}" for name, pairs in sorted(over.items())]
    return lines
