"""A floorplan's places as moves of a board's parts: where each part's own origin is to stand and how it is to
be turned, from where its body is to stand. Nothing here writes a board (see kicad.move_parts)."""

from .floorplan import bodies


def _quartered(quarters, at):
    """A place turned so many quarters counter-clockwise as the board is looked at (its y runs down): as the
    engine turns (see `quartered` in src/floorplan.rs), and as KiCad turns a footprint."""
    x, y = at
    return [[x, y], [y, -x], [-x, -y], [-y, x]][quarters % 4]


def moves(picture, found):
    """What a floorplan asks of each part: {reference: (where its origin is to stand, its angle in degrees)},
    for the parts it gives a place other than where they stand. A part is placed by the middle of its body
    and turned about it; its origin (what its file places it by) goes round with it.

    Returns (moves, kept): `kept` names the parts left as they stand because the plan found them no room."""
    middle = {reference: [(box[0] + box[2]) / 2, (box[1] + box[3]) / 2] for reference, box in bodies(picture).items()}
    found_moves, kept = {}, []
    for place in found.get("places", []):
        reference = place["part"]
        part = picture.parts.get(reference)
        if part is None:
            continue
        if not place["fits"]:
            kept.append(reference)
            continue
        quarters = place["turn"] // 90
        from_middle = [part.at[k] - middle[reference][k] for k in (0, 1)]
        turned = _quartered(quarters, from_middle)
        origin = [round(place["at"][k] + turned[k], 4) for k in (0, 1)]
        angle = (part.angle + place["turn"]) % 360
        if any(abs(origin[k] - part.at[k]) > 0.01 for k in (0, 1)) or quarters % 4:
            found_moves[reference] = (origin, angle)
    return found_moves, sorted(kept)
