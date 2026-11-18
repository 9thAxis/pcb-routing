#!/usr/bin/env python3
"""A floorplan as pictures: the ground each zone is given with the roads and gaps between; the supplies poured on
the plane layers; and the place of every part. From the JSON that kicad_floorplan.py --json writes.

    draw_floorplan.py PLAN.json OUT.png

Any Python with matplotlib will do: this opens no board.
"""
import json, sys


def leaves(zone):
    return [zone] if not zone["inside"] else [leaf for inner in zone["inside"] for leaf in leaves(inner)]


def draw(found, out):
    """Write the pictures of a floorplan (see `Floorplan` in src/floorplan.rs) to `out`."""
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.patches import Rectangle

    zones = found["zones"]
    x0, y0 = min(z["rect"][0] for z in zones), min(z["rect"][1] for z in zones)
    x1, y1 = max(z["rect"][2] for z in zones), max(z["rect"][3] for z in zones)
    wide, high = max(x1 - x0, 1.0), max(y1 - y0, 1.0)
    panels = ["the ground each zone is given; grey: roads kept for tracks, dark: gaps between domains",
              "supplies poured on the plane layers (white: none)",
              "a place for every part (green: front, blue: back, red: found no room; squares: parts on gaps)"]
    across = 16.0
    fig, axes = plt.subplots(len(panels), 1, figsize=(across, len(panels) * (across * high / wide + 0.6)))
    box = lambda r, **how: Rectangle((r[0], r[1]), r[2] - r[0], r[3] - r[1], **how)
    for ax, title in zip(axes, panels):
        ax.set_xlim(x0 - 2, x1 + 2)
        ax.set_ylim(y1 + 2, y0 - 2)
        ax.set_aspect("equal")
        ax.set_title(title, fontsize=10)
    colours = plt.get_cmap("tab10")
    for k, zone in enumerate(zones):
        colour = colours(k % 10)
        axes[0].add_patch(box(zone["rect"], color=colour, alpha=0.15))
        axes[0].text(zone["rect"][0] + 1, zone["rect"][1] + 3, zone["name"], color=colour, fontsize=10, weight="bold")
        for inner in zone["inside"]:
            axes[0].add_patch(box(inner["rect"], fill=False, color=colour, lw=1.0))
            if len(inner["parts"]) >= 10:
                r = inner["rect"]
                label = inner["name"].split(" (")[0].split(" and ")[0][:32] + f" ({len(inner['parts'])})"
                axes[0].text((r[0] + r[2]) / 2, (r[1] + r[3]) / 2, label, color=colour, fontsize=6, ha="center", va="center")
        for ax in axes[1:]:
            ax.add_patch(box(zone["rect"], fill=False, color="black", lw=1.2))
        for leaf in leaves(zone):
            axes[2].add_patch(box(leaf["rect"], fill=False, color="grey", lw=0.4))
    for road in found.get("roads", []):
        dark = road["gap"]
        for ax in axes if dark else axes[::2]:
            ax.add_patch(box(road["rect"], color="black", alpha=0.5 if dark else 0.2, lw=0))
    for bridge in found.get("bridges", []):
        axes[0].plot(*bridge["at"], "s", color="red", ms=5)
        axes[0].text(bridge["at"][0] + 1, bridge["at"][1] - 1, bridge["part"], color="red", fontsize=6)
    shades = plt.get_cmap("tab20")
    for k, pour in enumerate(found.get("pours", [])):
        for rect in pour["rects"]:
            axes[1].add_patch(box(rect, color=shades(k % 20), alpha=0.75, lw=0.3, ec="white"))
        if pour["rects"]:
            r = max(pour["rects"], key=lambda r: (r[2] - r[0]) * (r[3] - r[1]))
            axes[1].text((r[0] + r[2]) / 2, (r[1] + r[3]) / 2, pour["net"], fontsize=6, ha="center", va="center")
    for place in found.get("places", []):
        if place["rect"] is None:
            axes[2].plot(*place["at"], "s", color="black", ms=3)
            continue
        colour = "red" if not place["fits"] else "#4c78a8" if place.get("back") else "#54a24b"
        for rect in place.get("pieces") or [place["rect"]]:
            axes[2].add_patch(box(rect, color=colour, alpha=0.4, lw=0.3, ec="black"))
        r = place["rect"]
        if (r[2] - r[0]) * (r[3] - r[1]) > 12:
            axes[2].text(*place["at"], place["part"], fontsize=4.5, ha="center", va="center")
    fig.tight_layout()
    fig.savefig(out, dpi=80)
    plt.close(fig)


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    with open(args[0]) as plan:
        draw(json.load(plan), args[1])
    return 0


if __name__ == "__main__":
    sys.exit(main())
