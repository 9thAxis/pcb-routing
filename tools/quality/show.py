#!/usr/bin/env python3
"""Draw routings of a board side by side, from the dumps written by dump.py.

    show.py OUT.png NAME=DUMP.json [NAME=DUMP.json ...] [--box X0 Y0 X1 Y1] [--columns N]

One panel per dump, titled NAME: pads grey, vias black, tracks coloured by layer (the same colour for the same
layer in every panel). --box shows only that part of the board, in mm as KiCad gives positions (y down).
Needs matplotlib; the scoring does not.
"""
import argparse, json, math, sys
from pathlib import Path

LAYER_COLOURS = ["#c0392b", "#2471a3", "#27ae60", "#b7950b"]  # by position in the stack, repeating
PANEL = 10  # inches
BOARD_WIDTH = 60  # mm taken for the track widths when no box is given


def grid(count, columns=None):
    """(rows, columns) of panels for this many dumps: as square as it goes unless the columns are given."""
    columns = columns or math.ceil(math.sqrt(count))
    return math.ceil(count / columns), columns


def line_width(track_width, box=None):
    """A track's width in points on a panel showing `box` (or a board of the usual width)."""
    return max(0.6, track_width * (72 * 9 / ((box[2] - box[0]) if box else BOARD_WIDTH)))


def draw(axis, dump, title, box=None):
    """One dump on one matplotlib axis."""
    from matplotlib.patches import Circle, Polygon

    layers = dump["layers"]
    for pad in dump["pads"]:
        for rings in pad["shapes"].values():
            for ring in rings:
                if len(ring) >= 3:
                    axis.add_patch(Polygon(ring, closed=True, fc="#bbbbbb", ec="none", alpha=0.6))
    for t in dump["tracks"]:
        k = layers.index(t["layer"]) if t["layer"] in layers else 0
        axis.plot([t["a"][0], t["b"][0]], [t["a"][1], t["b"][1]], color=LAYER_COLOURS[k % len(LAYER_COLOURS)],
                  lw=line_width(t["w"], box), solid_capstyle="round", alpha=0.8)
    for v in dump["vias"]:
        axis.add_patch(Circle(v["at"], v["d"] / 2, fc="#222222", ec="none"))
    axis.set_aspect("equal")
    axis.invert_yaxis()
    axis.set_title(title, fontsize=18)
    axis.axis("off")
    if box:
        axis.set_xlim(box[0], box[2])
        axis.set_ylim(box[3], box[1])
    else:
        axis.autoscale()


def picture(named, output, box=None, columns=None):
    """Draw [(name, dump)] side by side into the image file `output`."""
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    rows, columns = grid(len(named), columns)
    figure, axes = plt.subplots(rows, columns, figsize=(PANEL * columns, PANEL * rows), squeeze=False)
    for axis in axes.flat[len(named):]:
        axis.axis("off")
    for axis, (name, dump) in zip(axes.flat, named):
        draw(axis, dump, name, box)
    plt.tight_layout()
    plt.savefig(output, dpi=70)
    plt.close(figure)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", help="the image to write (.png, or whatever matplotlib takes)")
    parser.add_argument("dumps", nargs="+", metavar="NAME=DUMP.json")
    parser.add_argument("--box", nargs=4, type=float, metavar=("X0", "Y0", "X1", "Y1"),
                        help="show only this part, in mm")
    parser.add_argument("--columns", type=int, help="panels in a row (default: as square as it goes)")
    args = parser.parse_args(argv)
    if not all("=" in d for d in args.dumps):
        parser.error("give each dump as NAME=DUMP.json")
    named = [(name, json.loads(Path(path).read_text())) for name, path in (d.split("=", 1) for d in args.dumps)]
    picture(named, args.output, args.box, args.columns)
    return 0


if __name__ == "__main__":
    sys.exit(main())
