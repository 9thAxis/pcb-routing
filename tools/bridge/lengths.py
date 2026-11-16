"""Lengths as they are written: millimetres, to a millionth."""

import math


def r6(v):
    """A length as it is written: to a millionth of a millimetre."""
    return round(v, 6)


def disc(centre, radius, sides=16):
    """A polygon that covers a disc."""
    out = radius / math.cos(math.pi / sides)
    return [[r6(centre[0] + out * math.cos(2 * math.pi * k / sides)),
             r6(centre[1] + out * math.sin(2 * math.pi * k / sides))]
            for k in range(sides)]
