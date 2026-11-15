# Changelog

## 0.1.0 (unreleased)

First version as a library.

- `route(&Problem) -> Result<Solution, Error>` and `Router` (events per round, verification of the two-pass
  search against the one-pass search, one attempt alone). The command line is unchanged in its two commands,
  `route` and `masks`, and gains `--trace`, `--verify` and `--only`.
- A problem is checked before it is run (`Problem::validate`): sizes and costs that make sense, names that
  exist, a grid that fits `options.max_states`. Misspelt fields in a problem file are an error.
- `Solution::status` is an enum (written as `routed`, `conflicts`, `unroutable`, as before); an incomplete
  solution has no score (`null`).
- Matched pairs (`Problem::pairs`): a trunk split into two legs, joined to their pads by stubs.

Fixed, from a review of the code before publication:

- A net could be reported unroutable when a path existed: the two-pass search derived its cut-off from a
  followed path's tabulated cost, whose per-step tolerance added up over a long line.
- New vias of different nets kept only the copper clearance from each other, not the hole spacing.
- The legs of a pair could be assigned to the wrong sides when both pads lay on one side of the trunk.
- A pair's stubs assumed a 0.05 mm grid.
- A negative corner or layer cost, or a zero step, sent the search round for ever and the machine out of memory.

Removed: the options `haste`, `stiffen` and `repairs`, which measurements had not justified.
