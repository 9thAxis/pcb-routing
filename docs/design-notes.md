# Design notes: what was measured and what was tried

A record for people working on the router. Times are from the board the router was first written for (a
static bus of 16 nets on 964,371 cells by 4 layers), one attempt on one core unless said otherwise, and from
the build of the day: they show what each change bought, not what the present build does.

## Speed

Speed, one attempt on the bus, one core. The attempts are chaotic (a different order of equal-cost states gives
a different, equally valid, negotiation), so the number of rounds varies between rows; times are for what that
build did.

| Change | Time |
|---|---|
| No corner cost (about 1,000 corners on the bus) | 16 s |
| Corner cost with direction of arrival in the search state (about 190 corners) | 219 s |
| Look-ahead = true cost to the goal past the fixed copper (was straight-line distance) | 107 s |
| One cost table per net and round instead of four tables read per step; the nine directions of a cell side by side in memory | 76 s |
| Radix heap instead of the standard binary heap (the profile showed 57 % of the time in its `pop`) | 40 s |
| Step rules past the fixed copper as one byte per (layer, cell), corner costs as a table; both shared by all attempts | 40.9 s, same answer |
| Bucket queue (each entry was moved about 9 times inside the radix heap) | 30 % less processor time over 18 attempts |
| Two passes: a direction-free first pass at current costs as the look-ahead (each cell was being expanded in about 7 of its 9 directions) | 28 s |
| The same for later connections of a net with several terminals (look-ahead towards its copper: out to a radius for short hops, kept and reused otherwise) | 25 s |
| Search record 16 to 8 bytes, first-pass record with the step cost in it; margin from the corner-free path | 21 s |

Later: masks and costs drawn up page by page only where a search goes (10 to 32 % fewer instructions on boards
of 27 to 79 nets); no exact look-ahead tables by default (the largest of those boards 695 to 439 thousand
million instructions and 3.4 to 1.0 GB, the same result); the first pass's look-ahead kept per state (13 to
18 % fewer).

Many attempts side by side on 18 cores (wall time): 18 attempts 41 s (was 199 s), 54 attempts 89 s (was about
430 s). Side by side each attempt takes about 1.8 times the processor time it takes alone: memory traffic, and
some of the cores being slower ones. 

## Route quality

Measured against the designers' own routing of the PCBWorld boards (111 boards that every router compared
connected completely; 1,105 nets the designer routed with tracks; 2,723 connections).

* Ending tracks on pads off the grid by sliding the last corner (`src/engine/ends.rs`) instead of with a stub
  from the last cell: turns of 90 degrees or more per 100 connections 75 to 11 (the designers: 25), segments
  shorter than twice their width 158 to 36 (90), corners per connection 2.5 to 1.6 (2.5). Track length and vias
  as before; all 119 boards still pass KiCad's rule check.
* A review with 43,000 random small problems found that the first version laid a stub untested into room
  another net had just moved into (22 cases, up to 0.022 mm short, only with pads narrower than their track),
  sent a track to a pad of another layer and off its via, and dropped the piece of track in a pad a net
  changes layer in. With these mended, cases more than 0.0025 mm under the rule between new nets went from
  243 (before any of this, from stubs) to none.
* Buses drawn together (`Net::bus`, `Options::bus_pull`): a net of a bus pays more per unit length wherever
  it is not beside a laid net of its bus, on the same layer. As a surcharge away from the bus and not a
  discount beside it, so that every look-ahead stays a lower bound. The figures, over 199 pairs of bus
  neighbours (each pair once: counted once per two parts it is side by side on, as at first, one board with
  two pairs on many parts made 241 of them and the crossings look half as bad again):

  | | beside its neighbour | pairs that cross | pairs on one layer |
  |---|---|---|---|
  | the designers | 72 % | 19 % | 117 |
  | no pull | 63 % | 22 % | 100 |
  | pull 0.2, beside = within two pitches | 67 % | 24 % | 103 |
  | and lanes (`Net::bus_lane`) | 71 % | 21 % | 109 |
  | and a price on passing over or under (`Options::bus_cross` 6) | 70 % | 14 % | 111 |
  | and the capacitors linked to their pins | 72 % | 14 % | |

  Where two neighbours are on one layer they run together for 93 % of their length with or without any
  of this; all that was missing was on pairs split over two layers. Without lanes the outer nets of a bus
  closed up on each other as they left their pads and the net between them went to the other layer. With
  lanes, beside a net k lanes away is from k - 1/2 to k + 1 pitches from it. The pull's strength hardly
  matters (0.1, 0.2, 0.4 with lanes: 67, 71, 70 % beside; 23, 21, 25 % crossing, which is what one run
  differs from the next); the price on passing over does (2 and 6: 19 and 14 % crossing), for half a
  percent more track.
  The price: a net away from its bus looks ahead with 1/(1 + pull) of its real cost, so its first pass
  covers more ground (two bus nets far apart on an open board: 38,000 to 81,000 states at 0.2), and the
  negotiation takes a round more now and then (18 of 251 random problems at 0.2); the public boards take
  about a seventh longer. A review found that the closing passes compared a new path, priced against the
  bus as it stood, with the old path's price from when it was laid: the net of a bus laid first, with
  nothing to be beside yet, was never drawn in. Both are now priced against the bus as it stands.
  A second review: marking grew with the square of the lane distance (a bus of 32 nets with lanes 82 s,
  7 s without), so nets more than three lanes apart are not drawn to each other; a state over one net and
  too near another was beside or over by the order of the nets, so over is now drawn after beside for all;
  the closing passes of a bus could chase each other until they ran out (41 of 3,400 random runs), so a
  move must gain a hundredth. A wrong order of lanes is worse than none (six nets, lanes three apart as a
  grid of pads would give: 6 % more track), so the bridge gives lanes only where a bus is in one row or
  two on one part.
* Pad exits (bridge). What was measured before anything was built: corners, short segments and vias were
  already under the designers' figures, and the one figure of how a route reads that was worse was how tracks
  leave pads. From the middle of a long pad the shortest way to anything off to the side is at 45 degrees
  through a long side, and that is what the search took: 53 % of long pads, where the designers have 33 %.
  The ground beside the long sides is now dear to the pad's own net alone (0.2 mm deep; what a crossing
  costs is set in mm of length), so a track runs out to the end first: the same length on the grid, and
  one corner more. On the 111 comparable boards, by what a crossing costs:

  | Cost | Through a long side | Track, designer = 1 | Vias per 100 | Corners per connection | Short segments per 100 |
  |---|---|---|---|---|---|
  | none | 53.1 % | 0.901 | 10.7 | 1.70 | 40 |
  | 0.5 mm | 30.8 % | 0.903 | 10.0 | 1.78 | 44 |
  | 1 mm | 23.5 % | 0.904 | 9.8 | 1.81 | 48 |
  | 2 mm | 16.4 % | 0.905 | 10.1 | 1.88 | 52 |

  0.5 mm is the default: level with the designers, who leave a third of their long pads sideways because
  that is often the right way out. All 119 boards stay complete and clean at every cost tried.
* Decoupling (bridge): a capacitor by a pin of an IC as an entry of its own, slow to change layer. A via
  between capacitor and pin 14 to 6 % of 51 (the designers 6 %), the copper between them as short as the
  straight line; 1.4 % more track, since the capacitor no longer taps the nearest copper of its net. On
  one crowded board the link found no way and, tried again by itself afterwards, the net found none
  either: a run that leaves anything unjoined with links is now thrown away and done again without.
  A third review: one net left unjoined anywhere threw the whole run away and with it every link (now
  only the nets whose own links failed lose them); links were handed out by name until a budget far under
  the engine's limit ran out; any capacitor by a pin counted, one in series with a signal too (now both
  its nets must have three pads); a bus name changed results with the pull at 0 (the hundredth applied);
  nets of a bus too far apart to be drawn together no longer paid for crossing (they do again).
* Zones (bridge; on by default). The benchmark's boards come without their zones; 86 of the 119 originals
  have them, and they can be put back on the stripped boards (the pads agree to 0.0002 mm). The first
  version counted what a filled zone joins as joined, took each remaining pad to the nearest point of the
  zone, and kept other nets off the spokes of every pad by treating them as copper: a fifth less track,
  but 7 of 119 boards incomplete and 20 new thermal relief errors. What it took to get to none of either:
  * The engine joins a piece of copper at any of several points (`piece`), so a pad is taken to whichever
    pad or point of the zone comes cheapest, and a point that turns out closed costs nothing.
  * Zones and spokes are dear ground, not closed ground. Closing the spokes shut the way out of a USB
    connector on one board; a price keeps tracks off them where there is another way. A factor of 2 for a
    zone and 4 for spokes. On the ten large boards a zone factor of 2 leaves the fills in 159 pieces
    where 1 leaves 183, for 333 vias where 1 takes 298; all complete at 1, 2 and 3. At 4 signals are
    crowded off the zones onto each other and two boards are not clean.
  * A zone is only what the last fill left of it. A track that ends on a point of a zone is joined until
    another new track passes near and the fill draws back: such copper is found after each fill and taken
    back, piece by piece, and its net joined again.
  * Tracks laid in one run can ring a piece of zone in on both layers, with no way left to stitch it to
    the rest. Then the run is taken back and done again with the pads of the two pieces as an ordinary
    connection, pad to pad, which the other nets must leave room for. (From a point of one piece to a point
    of the other does not do: on the zone whole again those are a step apart.)
  * Spokes are counted as KiCad's check counts them: two crossings of the fill's edge with the line half
    the gap from the pad are one spoke; those of a piece of fill that joins nothing else count only if the
    pad has no others; a straight track from the pad onto the fill counts as one. A pad left with too few
    gets that track. The count agrees with KiCad's on all 238 boards tried (stripped and routed), but
    for a track on another layer, which KiCad counts and which ties nothing to this layer's fill. A pad
    is given the fewer of the two counts: with such a track counted KiCad no longer counts the spokes of
    a piece of fill that joins the pad alone, so the count without it can be the higher one (a through
    pad with a track on the front and, on the back, two spokes to an island of the zone that the new
    tracks had cut off: one board of the 119, found when this was moved into the crate).
  With the zones: 26,000 mm of track where ignoring them takes 33,000, 462 vias for 388 (a land on one
  side is taken down to a zone on the other), 326 s for 213 s on the 119 boards. On the nets the designers
  routed as tracks, the tracks keep a little less distance from other copper on the large boards (a median
  gap of 3.9 clearances for 4.8), and turn sharply less often (7 per 100 connections for 12).
  Where it is done: all of the above was first written in the bridge, in Python, where the zones are
  read. None of it needs KiCad once the fills are polygons, a board from another program has the same
  questions, and on the largest board choosing the points of the zones alone took 10 s of Python (all
  that is done outside the router's own runs now takes under 4 s there, KiCad's fills included): it is
  now a module of the crate (`src/pours`), which the bridge asks through `pcb-routing ask`. The port
  gives the same groups, pieces, spoke counts and points on all 119 boards. One thing is made differently:
  the ground of a spoke was cut out with KiCad's polygon operations (the zone within a band round the pad,
  grown by the gap); it is now everything within that distance of where the half-gap line runs on the
  zone, one polygon without a hollow per spoke, which needs no polygon clipping and covers the old
  ground and about a tenth more. And places
  as good as equally far from what they were found for now come in the order of where they are, not of
  the last bit of a distance (on 5 of the 119 boards that changes which of them are offered). Filling a
  zone stays KiCad's: its fill is what the board is checked against. Copper that only touches a piece
  of zone, edge to edge, is joined to it, as it is for KiCad: taking that for apart parted pads from
  their zones on 4 boards. Which loose copper hangs together (vias and tracks that reach no pad but
  join two pieces of zone) and whether a point is still on a zone after a fill are the crate's as well:
  the bridge asks KiCad only for what KiCad alone knows.
  A pad no track fits by: on a board outside the 119, a ground track of the run went round two sides of a
  connector's shield pad, cut three of its spokes and left no way out for a track of the pad's own. The
  price on spokes had not kept it off; nothing after the fact could. Now the nets whose new tracks pass
  within reach of such a pad are taken out and laid again with that ground closed, and put back if that
  cannot be done, or if the way they then take parts another net from its zone: the first run of the 119
  boards with it left one net of one board unjoined that way. (The board it was written for has since
  come out clean without it, as the ways taken moved; the step is tested on boards made for it, in KiCad
  itself, and has yet to help on a board of the sets.)
  Still open: a custom rule for the thermal gap or the number of spokes is not read; a pad no straight
  track fits by is only named; the time a cut zone costs when a run is done again.
* Differential pairs through the bridge. The 119 boards have three (the "seven pairs" of an earlier note
  were mostly `LOAD+`/`LOAD-` and the like: rails); the whole dataset has 42 that the designers routed, on 19
  boards, and 20 of them on the 13 boards small enough to run again and again are what this was measured on.
  * A pair is two nets by name or by the hints, with the same few pads, whose pads stand side by side at
    two places or more. Names alone are not enough: `VIN+`/`VIN-` are rails, and a net of ten pads against
    one of five is no pair whatever it is called.
  * The engine lays a pair as one trunk between two points, on one layer. A real pair is often a chain
    (connector, protection device, controller): a trunk for each stretch long enough, tracks for a hop too
    short to gather and part in. Before that, the two USB pairs the designers had coupled best (96 and
    99 %) were not laid as pairs at all.
  * A trunk is offered nine places at each end (three distances, towards the other end and 45 degrees to
    either side), as one piece: the first version had one place, and one place is often under a pad. Laid
    as the cheapest connection it took the furthest of them, which are nearest each other, and gave away
    that much of the pair at either end. The places now take their turn, the nearest first
    (`Terminal::turn`). So near its pads a pair's stubs sometimes find no way: a stretch that is not all
    laid is done again without the nearest places before it is given up. (Turns alone, without that,
    lost a USB pair that had been laid, and gained nothing on the whole.)
  * A pair is a wish. A stretch whose stubs find no way (the legs arrive in the wrong order and cannot cross
    on one layer) is done again as two nets, and if anything at all is left unjoined with pairs laid, once
    more without any: one board of the 13 was left incomplete by its pairs before that.
  * Two faults of the engine's split came out, both at corners (see the changelog): a leg that ran
    backwards beside a stretch too short for it, and the outside of a sharp corner beyond the trunk.
  * Which way round (17 November). Three things found by drawing the pairs that failed, not by reading
    their messages: a trunk offered places only towards the far end had to pass one pad to reach the other
    where the two stand in line with it; a trunk that came to its place from the side ended with one leg
    behind the other; and a pair cannot be turned over on one layer, so the ways out of its two ends go
    together. Pricing the ground beside the way in did not hold a trunk to it; a heading in the search does
    (`Terminal::heading`), with the two ways round as hands (`Terminal::hand`), and where the ends share no
    layer the trunk ends further off on the layer the pads are not on, for two vias. 35 % to 51 % on the
    same 20 pairs. Tried and dropped: shutting a trunk's place inside the engine when its stubs fail and
    laying it again with all else fixed (nothing changed: what makes a pair fit is the board laid again).
  Before that: coupled for 40 % of their length, where the designers have 39 % and the router without pairs 18 %
  (36 % before the places took their turn and a pair was begun again further off); the lengths of the two
  halves differ by 15 % (designers 19 %, without pairs 16 %). The price is time on the boards where a pair
  is not laid at once: each new beginning is a run of the whole lot. Still open: untwisting
  at an end; coupling up to the pads; impedance.
  * Changing layer (17 November). A USB pair that the designer took under the board through two vias at
    either end (34 mm) went 71 mm round it on the top. The trunk may now change layer: through one via as
    wide as two of the legs' side by side (`Pair::via_pitch`), at the price of two, which the split makes
    into the two with each leg drawn aside to its own at 45 degrees. First the trunk was only held to its
    direction through the via, and turned a step before or after it: a leg then passes the other's via too
    near, whichever way the two are turned (the two vias are as near each other as the rules allow, so
    there is nothing to give). So a layer change is one move of the search: a straight run, the via in the
    middle of it, as long a run on the other layer. That pair: 32 mm, two vias a leg, as the designer's.
    Nothing else among the 20 pairs changed, at the price of two vias or of one: where the others fall
    short of their designers it is at ends with three parts in a row, not for want of a layer.
* What the nets are for (bridge). Three sources: the board, guesses from names and parts, a hints file.
  The first version let a guessed clock, crystal, switch node or sensitive net hold its shortest way
  (`yield`). On the 20 larger boards the 36 nets so guessed lost 12 vias between them and the other 760
  gained 47 (13 % more). Made only slower to change layer, the 18 clocks go from 12 vias to 6 and the rest
  gain 11, which is within what these boards move by for any small change. So that is all a guess does,
  for clocks and crystals; the rest of what a role means is for a role that the hints file gives, where
  somebody has said so. What binds (widths, clearances, layers, no vias, the distance kept by a sensitive
  net) never comes from a guess. Even so little can cost a connection: on the tightest of the large boards
  without its zones, one net taken for a clock (`/SCL`) and kept off vias left another with no way. A
  guess is a wish, as a pair is: where something is left unjoined the lot is laid once more without either,
  and the run that joins more stands.
* Open: short last stretches before a pad.

## Tried and not kept

* Laying all nets of a round at once against the previous round's picture (order independent, and one net's
  time per round): does not settle on this bus; after 80 rounds five nets were still in conflict.
* Laying a batch of six side by side and re-laying those that collide with a batch-mate: settles, but nearly
  every net of the bus collides, so no time is gained.
* Accepting paths up to 20 % dearer for less search: 3 % faster, worse lines; with the two-pass search,
  slower (the second pass floods where the first pass's costs end).
* A net over its wanted length giving way less each round: with several tight wanted lengths the stiffened
  nets hold their ground and one line went round by 110 mm in all 18 attempts.
* Repairs (lay the net furthest over its wanted length on its shortest path, hold it, let the others settle
  again): three repairs tried on one attempt each made the sum of excesses worse, and the run took 400 s.
* On a stall, laying all nets again at a low price for claimed ground with the standing costs kept: the same
  two nets ended in the same gap both times (replaced by spreading the standing cost).
* A first-pass margin fixed at 1, 3, 8, 20, 50, 150 corners: 26, 26, 24, 22, 25, 33 s; the default is 20 as
  the cap.

## Open

In order of expected gain: the first pass still covers about a quarter of the board per connection (a
look-ahead that knows corners would remove the need for a margin); attempts whose `last` nets cannot be laid
are found out only at the end; the bucket queue rescans its far list each time its window empties, which could
matter late in a long negotiation; via positions as a resource of their own; length matching.

* Exact look-ahead tables (the cost to the goal past the fixed copper alone, by a search of its own, kept and
  shared by the attempts): once the first pass existed they cost more than they saved on every board measured.
  They are still there behind `options.exact_after`.
* Ending a track on its pad without the last one or two points of its path, where those make a tail no
  longer than the track is wide, and sliding the stretch before: short segments 36.2 to 35.9 per 100
  connections, nothing else changed. Not worth the code.
* A first pass on a grid twice as coarse, with the fine grid only for what is left: about three times faster,
  1.7 % more track, 5 % more vias, and four more connections left on a crowded board. Offered by the KiCad
  bridge as `--coarse-first`, not the default.
