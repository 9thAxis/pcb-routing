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
  it is not within twice the least track pitch of a laid net of its bus, on the same layer. As a surcharge
  away from the bus and not a discount beside it, so that every look-ahead stays a lower bound. Beside its
  neighbour: 58 to 64 % of a net's length (the designers 68 %). The strength hardly matters: pulls of 0.1,
  0.2, 0.3, 0.4, 0.6 gave 64.2, 64.4, 65.0, 64.4, 64.7 %, with corners per connection rising from 1.58 to
  1.70. Pairs of neighbours that cross: 30 % without, 34, 37, 32, 30, 32 % with, in no order: a few more,
  within what one run differs from the next. Nothing orders the nets within the bundle.
  The price: a net away from its bus looks ahead with 1/(1 + pull) of its real cost, so its first pass
  covers more ground (two bus nets far apart on an open board: 38,000 to 81,000 states at 0.2), and the
  negotiation takes a round more now and then (18 of 251 random problems at 0.2). A review found that the
  closing passes compared a new path, priced against the bus as it stood, with the old path's price from
  when it was laid: the net of a bus laid first, with nothing to be beside yet, was never drawn in. Both
  are now priced against the bus as it stands (65 % beside, 37 % crossing at 0.2: the crossings are a real
  cost of the pull, not chance).
* Open: a via between a decoupling capacitor and its pin twice as often (14 % against 6 %); short
  last stretches before a pad.

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
