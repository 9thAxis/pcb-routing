# Changelog

## Unreleased

- Where the parts of a board belong (`pcb_routing::floorplan`, the `floorplan` question, and
  `tools/kicad_floorplan.py`; read only). First the power: each ground is a domain of the parts that stand
  on it; a supply is a net that two or more capacitors bypass to a ground, whatever it is called; a part on
  two grounds stands between two domains; two supplies with a part on both are one fed from the other.
  Within a domain, parts are tied by the signal nets they share: the parts of a sheet tied to each other are
  a group, and groups are gathered into zones level by level (Louvain), whatever sheet they are drawn on.
  Then each zone is given ground: the board is cut across its longer side, the zones in the order they are
  pulled along it (by their ties, and by what already has its ground), each lot with the share its parts
  need, down to the smallest zones. Nothing is fixed where it stands; a connector or a switch moves with its
  zone and pulls it to an edge. Along every cut a road is kept free of parts for what crosses it, as wide as
  its tracks take side by side on the layers that carry routes (a supply as four): the cut of the whole
  board has the widest, the cuts within the zones the narrowest. A file that holds several boards (two of a
  stack, drawn side by side: each closed outline is one) has its domains parted between them so that as
  little as may be is tied from board to board and none is more than six tenths full; a rule area shut to
  tracks and vias on every layer is no room. The layers have their parts in it: a plane carries nothing, an
  inner layer whose tracks run one way carries the long runs that way, the others run any way; a road
  holds what fits on the layers that run its way. The ground between two domains is a gap, not a road:
  nothing runs along it, and what goes from domain to domain goes through the parts that stand on it. Every
  net between zones of one domain is given a way along the roads (the shortest, a turn from a road into
  one that runs the other way counted as a layer change, a full road as four times its length), which
  says what the plan comes to: track, layer changes, and the roads asked for more than they hold. The plan is
  then bettered by that measure: a road asked for too much is widened and the board laid out again, and each
  cut is tried with its two sides changed over and kept so if the whole is the better for it. A gap names
  the parts that stand on both its domains: they are placed on it. Roads are whole tracks wide, one at the
  least along every cut, and a way may run under a zone that is not its end, at twice its length. The hints
  say how the boards of a file stack (`boards`: what is added to a place on each to say where it is in the
  stack, and how much a board clears above what lies under it), how high parts stand (`heights`, by the
  beginning of a footprint's name) and what a part is to stand near (`parts`, `near`): what is tied across
  two boards is pulled to the same place of the stack, a zone with a part too tall for the ground under
  another board is laid out elsewhere where that can be done (and named where it cannot). The bridge does
  not hold the plan to the height between boards (the standoffs are chosen for it): it says what the ground
  under a board needs, beside what the stack has. What is weighed now counts how far the nets between
  domains reach as the boards stack and how far a part is from the point it is to be near; and each of the
  larger cuts is also tried across the shorter side of its ground. For each zone: the ground it is given and
  how its parts stand to that today. A part that stands on two grounds is given its place (`bridges`): on the
  gap between its two domains, across its middle, and along it as near what the part is tied to on both
  sides as the other parts of that gap allow (none over another); a domain is given ground for the parts
  that stand half on it; two domains that share such a part and are no neighbours cost the plan dearly, and
  so do parts longer side by side than their gap. Two parts that mate (`parts`, `mate`: the two halves of a
  link from board to board) are pulled to the same place of the stack, and the plan says how far apart they
  are left. Ground where no part may stand (the board's rule areas, and `keepouts` in the hints: an opening
  to be cut, what must be reached from above) is no room when a ground is cut: each lot has its share of
  what is left. Supplies are poured on the plane layers (`pours`; every plane after the first, which is the
  ground's): under each of the smallest zones, the supply most of its parts are on, of those of its ground
  that reach several zones. A supply so poured takes no room in a road but where parts of it stand under
  another's pour and have none of their own on their side of the cut; the plan names those parts (they are
  reached by a track) and says in how many pieces each pour is. The supply a group of parts would have of
  the plane has its say in where zones go: groups fed alike are tied when zones are gathered into larger
  ones, and the zones under one pour pull each other when ground is given, that a pour be a few large
  pieces and not a patchwork (on a board with two supplies wanted almost everywhere: 8 pieces too many to
  3, for 6 % more track between zones). Where ways turn from one road into another, the plan says how many
  layer changes are made and keeps a square free for their vias where the roads have less (`junctions`).
  And every part is given a place (`places`): within the ground of its zone the parts are pulled and cut
  like the zones themselves (a capacitor from a supply to the ground by a chip of its zone on that supply),
  then the bodies of each domain are settled, none over another and none on a road or on ground that is
  shut, the largest first, each as near where it was pulled as the others allow, turned a quarter where
  that is nearer. The plan says how long the signal nets are with the parts as they stand today and with
  each at its place. The bridge now tells the engine each part by the middle of its body (its courtyard;
  the box of its pads where it draws none), with where each pad and its own origin are from there, what
  the schematic says each pin is, its angle, value and footprint, and whether it has holes. A part with no
  pad on a net (a mounting hole) stays where it stands and no part is put on it. Parts with holes and
  large parts are settled first, over the ground of their domain; a part without holes may stand over a
  road when its zone is full (the road's tracks run beneath), a part with holes on none of the main ones.
  Each part is then turned so that its pads face what they are joined to (`turn` is now degrees: of the
  ways its body may lie, the one that leaves its pads nearest the nearest other pad of their nets), and a
  capacitor from a supply to a ground is of the group of a chip of its sheet on that supply, stands beside
  the pad of the chip that is on that supply (the chip's pads taking them in turn, the smallest capacitors
  first), lies away from the chip with its own supply pad toward the chip's, and takes that place though a
  small part stood there, which is settled again beside. The plan says how far the pads are from what they
  join before and after, and how many capacitors have their pad within 3 mm of the one they are for.
  A capacitor takes a place by its pad only if all it moves is the nearer what it is joined to for it, and
  may stand for a chip of another sheet; a connector is no chip. Domains that share a part on two grounds
  are kept on one board. The parts on a gap are stood along it once more by where the parts they are tied
  to came to stand; then the small parts are gone over a few times, each tried at the middle of what its
  nets join it to and kept there if its nets are the shorter; a part of two pads with a net of its own to
  one pad of another part (a resistor at its driver, a protector at its connector) goes to that pad.
  A chip with a pad on a net that goes to an inductor and to little else is a switching regulator
  (`loops`): its inductor and what else of two pads is on that net (a diode) stand by the chip's pad on
  it, the capacitors that close the loop by theirs, before any other capacitor is placed; the plan says
  how large the box of each loop's pads came out, beside what the bodies of its parts take, and whether
  that is tight. What of two pads is on another small net of such a chip (its divider, what sets its
  frequency, its bootstrap capacitor) stands by the chip's pad there. A large capacitor is by its pad
  within its own length and a millimetre, a small one within 3 mm.
  The work is the engine's; the bridge reads the board and tells the answer.
- A board's work as steps (`tools/kicad_plan.py`, read only). The parts by the sheet of the schematic they
  are drawn on, a sheet whose parts stand in separate places as a block for each; the parts not placed yet;
  and the nets in a designer's order: within one block, the pairs between blocks, clocks and what is
  sensitive, buses, the other nets between blocks, the supplies that tracks must join. Each step with its
  nets, its open connections, their least length and the ground its pads span; and the nets that wait for a
  part to be placed. It lays nothing.
- One step at a time (`tools/kicad_step.py`). A step of that plan is tried on its pads' own layers, on those
  with each other signal layer and on every signal layer (an inner layer that zones cover is a plane and
  carries no routes), each try measured (nets left, track against the least it could be, vias, how much of a
  pair runs side by side), the best kept, saved as a new board and checked with KiCad's design rule check
  against the board it began from. Then it stops.
- A few nets of a large board (bridge). A run looks at the ground round the pads it is to join (half of
  what they span on every side, 10 mm at the least), not at the whole board: twelve nets of a board of
  235 mm by 109 mm on six layers, which was refused as too large a grid, take 6 s a run.
- A pair whose nets are partly laid. Two nets with pads that copper already joins (the parts of a cell
  joined among themselves) are laid as a pair all the same: the pair comes to the pad of each group that
  stands nearest the other net, and its stub joins whichever pad of the group comes cheapest (a stub entry
  may have the terminals of one piece).
- Advice on where the parts stand (bridge). Where the two nets of a pair are in one order at one part and
  in the other at the next, so that the pair goes round an end or crosses over, the bridge says which part
  would have to be turned by 180 degrees, have two pins swapped, or change places with its neighbour. No
  part is moved. A connector or a switch, guessed from its reference, is never offered; `parts` in the hints
  file (`{"U3": {"turn": false}}`) keeps others as they stand, or lets a connector be offered. A part
  in the middle of a pair that stands the wrong way round is no longer tried to be passed.
- A pair runs on past a part in its middle. Two nets that pass a part on their way (a protection device,
  termination resistors) were laid as a pair up to it and as a pair again, or as two tracks, beyond it. Now
  the trunk is one line from the first end to the last, led between the part's two pads, and either pad
  gets a branch to its leg. New in a problem: `Terminal::pass` (a terminal the entry runs on through, along
  its heading; such an entry is one line, joined piece by piece in the order of its terminals) and
  `Pair::passes` (the stub entries of the pads passed). `pairs::plan` offers such trunks as `through`, the
  bridge takes them where the pads leave the trunk room and no other pad stands between, and falls back to
  a trunk for each stretch. A trunk that may change layer now ends on the layer of its pads where a place
  there is open. An entry laid once for each hand is not followed on a later hand beyond what the best
  before it cost. On 20 pairs of 13 public boards: coupled for 53 % of their length (50 % before); a USB
  pair 63 % to 96 % (its designer 89 %), another 77 % to 90 % (96 %), an Ethernet pair 72 % to 96 %, and
  the other pair of that board, which is not laid past its part there, 87 % to 75 % (its designer 58 %). All
  119 boards complete and clean with and without zones.
- Faster where pairs or guesses are in play. A net with a heading, and the trunk of a pair, are searched
  in two passes like any other: the first pass, which knows neither headings nor a trunk's layer changes,
  is their look-ahead, only no path is cut off by what it found. And the bridge no longer runs a whole
  board once more without pairs and guesses for a net that is not joined with the board to itself either
  (a ground net whose zone is gone): it lays that net alone first, which takes a fraction of a second. On
  12 public boards with pairs: 652 s to 516 s in all, the same pairs laid.
- Pairs change layer. `Pair::via_pitch`: the distance between the two vias of a pair, side by side across
  its way; the trunk entry then has a via as wide as both and need not have `no_vias`. In the search a
  trunk's layer change is one move, the via in the middle of a straight run, and the split draws each leg
  aside to a via of its own. The bridge offers it to every pair whose nets may have vias, at the price of
  two. A USB pair that went 71 mm round its board on one layer is 32 mm through two vias a leg, as its
  designer has it (34 mm). All 119 boards complete and clean with and without zones. A problem is refused
  where the two vias would be nearer than two holes may be, or the trunk's via too narrow to keep other
  holes off either of them.
- A last step of a cell or two onto a via is taken out of the track, as it is before a pad; and runs of a
  net that meet end to end and go straight on are one track. Segments under 0.1 mm away from pads on the
  119 public boards: 45 to 31.
- Pairs are laid the right way round. A pair cannot be turned over on one layer, and a trunk that reached
  its place from the side ended with one leg behind the other. Now: a terminal may have a `heading` (a track
  sets out that way and comes in against it) and a `hand` (an entry is joined at terminals of one hand
  only; it is laid once for each and the better kept); `pairs::plan` gives the places of a trunk as the two
  ways round its ends that bring each leg to its own pad (`gather`, `round`), out to 4.5 times as far off
  the pads as before. The bridge heads each place straight away from its pads; where the two ends share no
  layer, the trunk is dearer on the layer of the end with the wider pads and ends further off on a layer
  the pads are not on, where two vias have room; and the third try of a stretch is left free. On 18 pairs
  of 12 public boards: coupled for 50 % of their length (49 % before, the designers 44 %), with a USB pair
  at a through-hole connector from 58 to 77 % and no pair down by more than 3 points. All 119 boards
  complete and clean with and without zones.
- Pairs leave their pads square to them. The places a trunk may begin at were offered towards the far end
  only; where two pads stand in line with it (a part turned by 45 degrees, a row that faces away), one leg
  had to pass the other's pad and the pair was laid as two nets. The places square to the line of the two
  pads are now offered first, to either side, and the others as a second choice (`pairs::plan`). On 18 pairs
  of 12 public boards: coupled for 49 % of their length, from 35 % (the designers 44 %); a USB pair that
  was two nets is coupled for 95 %. All boards complete and clean.
- Fewer slivers at pads. A track ends on the exact place of its pad; where its last corner could not slide
  there, a stub from the cell centre was left, shorter than the track is wide and at any angle. Now the end
  of the last stretch turns onto the point (by seven degrees at the most), and a last step of a cell or two
  onto the pad's cell is left out. On the 119 public boards, segments under 0.1 mm at pads go from 374 to 87
  (the designers have 278), with every board still complete and clean. What is left stands where a via or a
  branch shares the cell.
- Long pads are left through an end. Ground can be dear to one net alone (`Dear::only`), and the KiCad bridge
  makes the ground beside the long sides of every pad at least 1.5 times as long as wide dear to the pad's
  own net (`--exit-cost MM`, 0.5 by default, 0 for none): a track runs out to the end of its pad before it
  turns, where before it left at 45 degrees from the middle, through a long side. On the 111 comparable
  public boards the share of long pads left through a long side goes from 53 % to 31 % (the designers:
  33 %), for 0.3 % more track and no more vias; all 119 boards stay complete and clean.

## 0.1.0 (2026-11-15)

First version as a library.

- `route(&Problem) -> Result<Solution, Error>` and `Router` (events per round, verification of the two-pass
  search against the one-pass search, one attempt alone). The command line is unchanged in its two commands,
  `route` and `masks`, and gains `--trace`, `--verify` and `--only`, and a third command, `ask` (below).
- A problem is checked before it is run (`Problem::validate`): sizes and costs that make sense, names that
  exist, a grid that fits `options.max_states`. Misspelt fields in a problem file are an error.
- `Solution::status` is an enum (written as `routed`, `conflicts`, `unroutable`, as before); an incomplete
  solution has no score (`null`).
- What can be laid is laid: a net with a terminal out of reach keeps its other connections, also without
  negotiation and for the nets kept for last; a pair that cannot be split or whose stubs do not all fit no
  longer ends the attempt; a net left out for want of room is tried again after the closing passes have moved
  the others. Incomplete attempts are ranked by how much is missing. A failure names both sides of the
  connection that could not be made.
- The fixed masks of the nets and the costs of the ground are drawn up page by page, only where a search
  goes, and nets with the same sizes and clearances hold one page wherever none of their own copper is near.
  No mask is drawn up for the whole board before the first net is laid.
- `options.exact_after` is `null` by default: no exact look-ahead tables. They cost more than they saved on
  every board measured (the largest of the hard set: 695 to 439 thousand million instructions and 3.4 to
  1.0 GB without them, the same result).
- A track ends at the exact place of a pad or hole only if the piece that leads there from the cell keeps
  its distances; else it ends on the cell. (A track wider than its pad could come too near the next pad.)
- A track that ends on a pad or hole off the grid is brought there by sliding its last corner along the
  stretch before it, so that it keeps its directions, instead of by a crooked stub shorter than the track
  is wide. What moves is tested off the grid against the fixed copper and the other nets; a stub is laid
  where the track may not move, and left out where it would come too near another net. On 111 public
  boards: turns of 90 degrees or more per 100 connections 75 to 11, segments shorter than twice their
  width 158 to 36.
- An entry that changes layer on a via of another entry of the same net holds that via too: the other entry
  could be laid again with its via elsewhere, and the result then had a layer change with no via.
- Buses: nets given the same `bus` name are drawn to run beside each other. A net of a bus pays
  `options.bus_pull` (0.2) more per unit length wherever it is not beside a laid net of its bus, in the lane
  its `bus_lane` gives it, so that the nets between two others find room; and `options.bus_cross` (6) more
  where it runs over or under one on another layer. The KiCad bridge names as a bus three or more nets with
  neighbouring pads on the same two parts, with lanes in the order of the pads (`--bus-pull`, 0 for none).
  On 199 pairs of bus neighbours on public boards: 63 to 72 % of a net's length beside its neighbour (the
  designers: 72 %), and 22 to 13 % of the pairs crossing (19 %).
- KiCad bridge: a capacitor with a pin of an IC of its net within 5 mm is joined to that pin by an entry of
  its own with five times the via penalty, and the rest of the net is joined without it (`--no-decoupling`
  for none). A capacitor counts if both its nets have three pads or more (not one in series with a signal).
  If a net with such links is not all laid, nothing of the run is kept and it is done again with that net
  whole. On 51 such capacitors on public boards: a via between capacitor and pin for 14 to 6 %
  (the designers: 6 %), for 1.4 % more track.
- KiCad bridge: zones are followed (`--no-pours` for the old way, where a net with a zone is routed as
  tracks). The zones are filled first, and pads that a filled zone joins count as joined; a net with a zone
  gets tracks only from the pads its zone does not reach, each to a pad or to a point of the zone, whichever
  comes cheapest. A track of another net pays double for its length across a zone and four times across
  the spokes of a pad (`--pour-cost`, `--spoke-cost`). After each run the zones are filled again: copper that
  ended on a piece of zone the other tracks cut away is taken back, pads parted from their zone are joined
  again, and where that cannot be done past the tracks that did it, the run is taken back and done again
  with those pads joined by a track. At the end the spokes of every pad are counted as KiCad's check
  counts them, and a pad with too few gets a straight track into its zone, which counts as one; a pad
  that no such track fits by is named. On the 119 public boards with their designers' zones put back: all
  complete with no new error in KiCad's check, 26,000 mm of track and 462 vias where ignoring the zones
  takes 33,000 mm and 388 (the vias: pads taken down to a zone on the other side).
- Zones, in the crate (`pcb_routing::pours`, and the command `pcb-routing ask`, which answers the same questions
  as JSON): from the pieces of the filled zones as polygons and the pads, which groups of pads the zones join
  (`survey`), the spokes of every pad the zones tie, counted as KiCad counts them, with the ground where a track
  would cut them; the places a net may be joined to its zone (`reach`); the copper of a run that hangs on a
  piece of zone (`parts`), and whether the zone is still there after a fill (`on_zone`); and the straight track
  that ties a pad left with too few spokes (`tie_targets`, `ties`). A pad is given the fewer of KiCad's count of
  its spokes and of that count without the tracks on other layers, which tie nothing to the zone. The bridge did
  all of this in Python; it now reads the board, fills the zones and asks, and keeps what it has laid if a
  question gets no answer. On the largest of the public boards that takes the time spent outside the router's
  own runs from 14 s to under 4 s.
- KiCad bridge, zones: a pad left with too few spokes and no room for a track of its own to its zone,
  because nets of this run pass close by it, is seen to by laying those nets again with the ground round the
  pad closed to them (the pad, its gap, the clearance the zone keeps, and 0.3 mm); they are put back as they
  were if they cannot all be joined so. A last resort: it is tested on made-up boards, and has not yet mended a pad
  on any of the 119 public boards.
- Differential pairs through the KiCad bridge. Two nets named as the halves of a pair (`_P`/`_N`, `-P`/`-N`,
  `P`/`N`, `+`/`-`, `DP`/`DM`; not supplies), or said to be one in the hints, with the same number of pads (four
  at most), are laid side by side between the places where a pad of each stand side by side: a trunk for each
  stretch between two such ends far enough apart, tracks as for any net between two that are not and to a pad
  that stands alone. `pcb_routing::pairs::plan` (and the question `pairs`) finds the ends and offers each trunk
  several places to begin and end, by how far off the pads they are; a trunk entry has them as the terminals of
  two pieces, the nearest first (`Terminal::turn`: of the open terminals of a piece only the earliest are
  offered). A stretch that is not all laid as a pair is done again begun further off, and in the end as two
  nets; where something else is left unjoined with pairs laid, the run is done once without them and the one
  that joins more is kept (`--no-pairs` for none). On 20 pairs of 13 public boards: coupled for 40 % of their
  length (18 % without, the designers 39 %), all boards complete and clean.
- Engine, pairs: a leg lies within the trunk it is split from. Where the trunk ended in a stretch shorter
  than the inside of its corner has room for, the inner leg ran backwards there, towards the other leg (0.008
  mm short of the clearance on one board): such a stretch is left out of that leg. And the outside of a
  corner too sharp for what the trunk is wider than its legs is cut straight across.
- KiCad bridge: what the nets are for (docs/hints.md). Guessed from names and parts: clocks and crystals are
  slow to change layer (`--no-guesses` for none). Said in `BOARD.routing.json` beside the board, or
  `--hints FILE`, by net name with wildcards: `role`, `width`, `amps`, `clearance`, `volts`, `max_mm`, `vias`,
  `layers`, `priority`, `pair` and `gap`, `bus`; `ohms` and `match` are taken down and not acted on. A
  guess only leans a net, and is dropped where it would cost a connection; a width, a clearance, layers,
  "no vias" and the distance a sensitive net keeps come from the file alone. The bridge says what it took
  from the names and what from the file.
- Engine: terminals of an entry may name the piece of copper they stand on (`piece`). Those of one piece are
  joined already, and the entry is joined to the piece at whichever of them comes cheapest. The bridge
  gives every group of pads that copper already joins as one piece, with its nearest pads to choose from,
  where it gave the group's first pad.
- Engine: ground that costs more without being closed (`dear`): polygons with a factor on the length of
  track in them, and on a via as a track of its diameter; the net a piece belongs to pays nothing more
  there, and where pieces overlap a net pays the dearest that is not its own.
- KiCad bridge: round pads and arcs are given as polygons that hold the true shape (they were drawn inside
  it, up to 0.005 mm short), and the margin added to every clearance is 0.002 mm instead of the 0.02 mm that
  had been covering for that. A board laid out at its minimum clearance has room again.
- The result types (`Solution`, `NetResult`, `Track`, `Conflict`, `ConflictKind`, `Attempt`, `Stats`) and
  `Shape` are `#[non_exhaustive]`: fields and kinds can be added without breaking callers. The tuning fields
  of `Options` are documented as not stable.
- Matched pairs (`Problem::pairs`): a trunk split into two legs, joined to their pads by stubs.
