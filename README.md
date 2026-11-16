# pcb-routing

An autorouter for printed circuit boards: a Rust library, a command-line program, and a bridge that routes a
KiCad board in place. All nets are laid together and settle between themselves who goes where (negotiated
congestion), so the result does not depend on the order of the nets.

**Check every result with your CAD program's design rule check before using it.**

Status: 0.1. The interfaces may still change.

## Route a KiCad board

    git clone https://github.com/9thAxis/pcb-routing && cd pcb-routing
    cargo build --release
    PCB_ROUTING=target/release/pcb-routing \
      /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 \
      tools/kicad_route.py BOARD.kicad_pcb -o ROUTED.kicad_pcb

Run it with the Python that ships with KiCad 9 or 10 (the path above is the macOS one). It routes every net
whose pads are not yet joined, keeps what is already routed, follows the board's filled zones and refills
them. The bridge is in this repository, not in the crate.

| Option | |
|---|---|
| `--nets A,B` | route only these nets |
| `--layers F.Cu,B.Cu` | route only on these layers |
| `--hints FILE` | what the nets are for ([docs/hints.md](https://github.com/9thAxis/pcb-routing/blob/main/docs/hints.md)); `BOARD.routing.json` beside the board is read without asking |
| `--coarse-first` | about three times faster, a little more track and vias |
| `--help` | the rest |

## What it does

* Exact clearances per net class and per pair of nets, keep-outs, board edge, hole spacing.
* Tracks at 45 degrees, through vias, nets with many pads as trees.
* **Copper zones**: pads a zone joins are left to it, zones that new tracks cut are mended, thermal relief
  is kept.
* **Differential pairs**, found by name (`_P`/`_N`, `+`/`-`, `DP`/`DM`) or named in the hints, laid side by
  side, through two vias side by side where they change layer.
* **Buses** kept together, **decoupling capacitors** joined to their pin, **long pads** left through an end.
* **Hints**: an optional file with roles, a width for a current, a clearance for a voltage, layers, pairs.
* Partial results: what cannot be joined is named, and everything else is laid.
* Deterministic: the same problem gives the same answer, whatever the thread count.

## What it does not do

Push and shove or interactive routing; length tuning; impedance; blind, buried or micro vias; BGA fan-out as its own step; arcs and teardrops; KiCad's custom rules. At most seven routing
layers. See [how it works](https://github.com/9thAxis/pcb-routing/blob/main/docs/how-it-works.md) for the
full list and the limits.

## Results

On the 119 boards of the PCBWorld benchmark (real open-source KiCad boards with their tracks removed), with
KiCad 10's design rule check as the judge:

| | Boards clean and complete | Time per board (small / medium / large) |
|---|---|---|
| pcb-routing | 119 of 119 | 0.6 s / 2.3 s / 19 s |
| KiCadRoutingTools | 108 | 0.7 s / 2.0 s / 8.2 s |
| FreeRouting 2.5 | 50 (107 with its narrowed tracks forgiven) | 2.8 s / 3.7 s / 12 s |

Against the boards' own designers, on the 111 boards all three routers completed:

| | Track length | Vias per 100 connections | Corners per connection |
|---|---|---|---|
| The designer | 1.00 | 18.7 | 2.5 |
| pcb-routing | 0.90 | 10.0 | 1.8 |
| KiCadRoutingTools | 0.90 | 36.9 | 1.8 |
| FreeRouting | 0.95 | 15.8 | 2.7 |

With the designers' zones put back, all 119 boards are again clean and complete. How these were measured,
the caveats, and the figures for buses, decoupling and pad exits are in
[docs/benchmarks.md](https://github.com/9thAxis/pcb-routing/blob/main/docs/benchmarks.md).

## As a library

```rust
use pcb_routing::{route, Problem, Status};

let problem: Problem = serde_json::from_str(&std::fs::read_to_string("problem.json")?)?;
let solution = route(&problem)?;
if solution.status != Status::Routed {
    eprintln!("not joined: {:?}", solution.unrouted);
}
for net in &solution.nets {
    println!("{}: {:.2} mm, {} vias", net.key, net.length_mm, net.vias.len());
}
```

`cargo add pcb-routing`. A problem that was run and could not be completed comes back as a solution, with
what could not be joined named; `route` returns an error only for a problem that cannot be run at all. The
problem format is documented on the types (`cargo doc --open`) and summarised in
[how it works](https://github.com/9thAxis/pcb-routing/blob/main/docs/how-it-works.md). `pcb_routing::pours`
and `pcb_routing::pairs` do the geometry of filled zones and differential pairs for whoever writes a front
end.

## As a program

    cargo install pcb-routing
    pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N] [--trace] [--verify]
    pcb-routing ask [QUESTION.json] [-o ANSWER.json]

`route` exits with 0 if every net is laid, 1 if not, 2 if the problem could not be run. `ask` answers a
front end's questions about zones and pairs as JSON. Try `examples/two_nets.json`.

## More

* [How it works](https://github.com/9thAxis/pcb-routing/blob/main/docs/how-it-works.md): the problem, the method, the limits, sources.
* [Benchmarks](https://github.com/9thAxis/pcb-routing/blob/main/docs/benchmarks.md): every table and how it was measured.
* [Hints](https://github.com/9thAxis/pcb-routing/blob/main/docs/hints.md): telling the router what the nets are for.
* [Design notes](https://github.com/9thAxis/pcb-routing/blob/main/docs/design-notes.md): what each step bought, and what was dropped.
* [Measuring route quality](https://github.com/9thAxis/pcb-routing/blob/main/tools/quality/README.md).

## Licence

Licensed under either of the Apache License, Version 2.0 (`LICENSE-APACHE`) or the MIT licence
(`LICENSE-MIT`), at your option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as
above, without any additional terms or conditions.
