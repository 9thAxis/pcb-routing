"""What the names of nets and parts say they are: supplies, grounds, the halves of differential pairs, clocks,
crystals and the like. Said once, for the bridge that routes by it and the tools that measure by it."""
import re

SUPPLY = re.compile(
    r"^/?(.*/)?(\+?\d+(\.\d+)?V\d*|\+?\dV\d|V(CC|DD|SS|EE|BAT|BUS|IN|OUT|REF|PP|MOT|SYS)\w*|[AD]?GND\w*|GND|PWR\w*"
    r"|\+BATT?|VBAT\w*|AVCC|AVDD|DVDD|3V3|5V|12V|24V|V?[+-])$", re.I)
GROUND = re.compile(r"GND|VSS|VEE", re.I)
PAIR_HALF = re.compile(r"^(.*?)([_-]?[PN]|[+-]|D[PM])$")
_WORD = lambda words: re.compile(r"(^|[^A-Za-z])(" + words + r")\d*([^A-Za-z]|$)", re.I)
# (a role, and what in the last part of a net's name says so; the first that fits is taken)
NAMED = [("crystal", _WORD("XTAL|XTALIN|XTALOUT|OSC|OSCIN|OSCOUT|XIN|XOUT|OSC32|XTAL32")),
         ("clock", _WORD("CLK|CLOCK|SCK|SCLK|SCL|MCLK|BCLK|LRCLK|XCLK|PCLK|REFCLK|TCK")),
         ("noisy", _WORD("SW|LX|PHASE|BOOT|BST|GATE|HDRV|LDRV|PWM")),
         ("sensitive",
          _WORD("VREF|AREF|REFP|REFN|AIN|AINP|AINN|ADC|SENSE|VSENSE|ISENSE|SENSEP|SENSEN|MIC|ANT|RF|NTC|RTD|THERM"))]
CRYSTAL_PART = re.compile(r"^(Y|XTAL)\d")  # (not X: that is a connector as often)
CAPACITOR_PART = re.compile(r"^C\d")
INDUCTOR_PART = re.compile(r"^L\d")
DIODE_PART = re.compile(r"^(D|TVS|ZD)\d")
NOISY_PART = re.compile(r"^(Y|XTAL|OSC)\d")  # oscillators: what disturbs its neighbours all the time
# a part that someone plugs into or presses: a connector, a switch (not JP, a jumper)
FACING_PART = re.compile(r"^(J|P|CN|CON|X|USB|RJ|SW|S|BTN|PB)\d")
CHIP_PART = re.compile(r"^(U|IC)\d")


def is_supply(name):
    """True for the name of a supply or of a ground: `SUPPLY`, and the like with a sign after it (VIN+ and
    VIN- are rails)."""
    return bool(SUPPLY.match(name) or (name[-1:] in ("+", "-") and SUPPLY.match(name[:-1])))


def pair_names(nets):
    """The nets that are the two halves of a differential pair by their names, as [(first, second)]: names
    that differ only in an ending _P/_N, -P/-N, P/N, +/- or DP/DM, and are not those of supplies."""
    halves = {}
    for name in sorted(nets):
        match = PAIR_HALF.match(name)
        if match and not is_supply(name):
            stem = name[:-1] if name[-2:] in ("DP", "DM") else match.group(1)
            halves.setdefault(stem, []).append(name)
    return [tuple(pair) for pair in halves.values() if len(pair) == 2]
