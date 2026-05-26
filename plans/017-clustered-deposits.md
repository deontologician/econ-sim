# Plan 017 — Clustered resource fields (many more deposits)

## Context

Only `DEPOSITS_PER_ELEMENT = 3` (12 total) and scattered one-by-one. Wanted: many
more spawns, grouped into related clusters.

## What shipped (`world.rs`, `main.rs`)

- `place_deposits` now spawns **clusters**: per element, `CLUSTERS_PER_ELEMENT` (3)
  fields, each a centre picked on the preferred terrain (`pick_empty_tile`) plus
  `DEPOSITS_PER_CLUSTER - 1` more packed within `CLUSTER_RADIUS` (2) tiles via the new
  `pick_near_empty`. Total = 4 × 3 × 4 = **48 deposits** (was 12), in same-element
  fields rather than lone tiles. Deposit creation factored into `push_deposit`.
- Population stays stable: a fresh start seeds a miner on at most `MAX_SEEDED_MINERS`
  (12) deposits — the rest are claimed emergently as roamers cross them — so the
  noot count no longer scales with deposit count (still ~56).

## Files
`world.rs` (cluster placement, `push_deposit`, `pick_near_empty`, cluster consts),
`main.rs` (`MAX_SEEDED_MINERS`, capped seeding in the fresh-start population + target).

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Unverified at runtime.** Watch
  cluster density/legibility on the map and whether 48 deposits over-supply or
  over-fragment the economy. Tunables: `CLUSTERS_PER_ELEMENT`, `DEPOSITS_PER_CLUSTER`,
  `CLUSTER_RADIUS`, `MAX_SEEDED_MINERS`.
