# Routing hints

What a net is for is not in a board file. The KiCad bridge takes it from three places, each overriding the one
before:

1. **The board**: net classes (width, clearance, via), zones, part references.
2. **Guesses** from the names of the nets and the parts they are on (`--no-guesses` for none).
3. **A hints file**: `BOARD.routing.json` beside `BOARD.kicad_pcb`, or the file named with `--hints`.

At the start of a run the bridge says what it took from the names and what from the file.

A guess only *leans* a net: what a detour or a via costs it, how readily it gives way to others. What *binds* a
net (a width, a clearance, the layers it may use, no vias, a distance kept from every other net) comes from the
hints file alone. So a wrong guess can cost a little quality and cannot make a board fail a rule or a net fail
to route for want of room it was never asked to keep. Laying two nets as a pair is the one guess that changes
the copper outright, and a pair that cannot be laid as one is laid as two nets.

## What is guessed

| From | Taken for | What follows from the guess |
|---|---|---|
| `GND`, `AGND`, `VSS`, `+3V3`, `VCC`, `VBUS`, `12V`, `VIN+` ... | ground, supply | never one half of a pair |
| a pad on a part `Y1`, `X2`, `XTAL1`; `XTAL`, `OSC`, `XIN`, `XOUT` in the name | crystal | five times slower to take a via |
| `CLK`, `SCK`, `SCL`, `MCLK`, `BCLK`, `REFCLK`, `TCK` ... as a word of the name | clock | twice as slow to take a via |
| `SW`, `LX`, `PHASE`, `BOOT`, `GATE`, `PWM` ... as a word of the name | noisy (a switch node, a gate drive) | nothing, unless the hints say so too |
| `VREF`, `AREF`, `AIN`, `ADC`, `SENSE`, `MIC`, `ANT`, `RF`, `NTC`, `RTD` ... as a word of the name | sensitive | nothing, unless the hints say so too |
| names that differ only in an ending `_P`/`_N`, `-P`/`-N`, `P`/`N`, `+`/`-`, `DP`/`DM` | a differential pair | laid side by side (below) |

A guessed role does less than the same role in the hints file. Measured on 20 public boards, guessed clocks,
switch nodes and sensitive nets that were made to hold their shortest way cost the other nets 13 % more vias;
made only slower to change layer, the clocks have half the vias they had (12 to 6 on 18 nets) for 3 % more
on the rest.

"As a word" means with no letter before or after it: `SPI_SCK` and `CLK2` are clocks, `CLKOUT_EN` and `SWDIO`
are not.

## The file

```json
{
  "nets": {
    "/adc/*":   {"role": "sensitive", "clearance": 0.4},
    "/adc/CLK": {"role": "clock", "vias": "none", "max_mm": 40},
    "+24V":     {"amps": 3, "volts": 24},
    "SPARE_P":  {"pair": false}
  },
  "pairs": [
    {"nets": ["TX_P", "TX_N"], "width": 0.15, "gap": 0.18}
  ]
}
```

A key of `nets` is a net's name, with `*` for any run of characters and `?` for any one; a name is tried with
and without its leading `/`. Where several fit a net, the later ones override the earlier, field by field. A
field the file does not know is an error, so that a slip of the pen is not taken for "nothing asked".

| Field | Value | What follows |
|---|---|---|
| `role` | `supply`, `ground`, `clock`, `crystal`, `noisy`, `sensitive` | a clock or a crystal is slow to take a via; a clock, a crystal, a noisy or a sensitive net holds its way and the others go round (a crystal and a switch node hardest); a `sensitive` net keeps twice the usual distance from every other net laid with it |
| `width` | mm | its track width, instead of its class's |
| `amps` | A | its tracks are at least as wide as that current takes on an outer layer of 35 µm copper with a rise of 10 °C (IPC-2221) |
| `clearance` | mm | what it keeps from all other copper, if more than its class's |
| `volts` | V | it keeps at least the distance IPC-2221B asks between bare outer conductors at that voltage |
| `max_mm` | mm | its wanted greatest length: results that exceed it are ranked behind those that do not |
| `vias` | `none`, `few` | it may not change layer; or it is three times slower to |
| `layers` | a list of layer names | the layers it may use, among those routed on |
| `priority` | `high` | it holds its shortest way and the others go round |
| `pair` | a net's name, or `false` | the net it is laid side by side with; `false`: with none, whatever its name |
| `gap` | mm | between the legs of its pair, edge to edge, if more than the clearance |
| `bus` | a name, or `false` | the bus it runs with (nets of one bus are drawn to run beside each other); `false`: with none |
| `ohms`, `match` | a number; a name | taken down and said back, **not acted on**: there is no impedance calculation and no length matching yet |

`pairs` is a list of pairs by their two nets, with `width`, `gap` and `ohms` as above.

## Pairs

Two nets are laid as a pair where each has the same number of pads (four at most), none of them joined to
another yet, and a pad of each stand side by side (no more than 5 mm apart, each the other's nearest) at two
places or more. Between two such ends far enough apart, the pair is laid as one wide track and split into two
legs at the pitch asked for; each leg is joined to its pads by a short track at either end. Between two ends
too close for that (a protection device next to its connector), and to a pad that stands alone (a pull-up on
one leg), the nets are joined as any net is.

The legs are as wide as the nets' tracks and as far apart as the clearance, or the `gap`. A stretch that cannot
be laid as a pair (no room for it, or the legs would have to cross to reach their pads) is laid as two nets,
and the bridge says so.
