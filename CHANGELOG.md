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
