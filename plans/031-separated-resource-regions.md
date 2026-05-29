# 031 — Separated resource regions

## Context

Deposits clumped indiscriminately: cluster centres were seeded at random per element, so
*different* resources often landed next to each other and a noot could mine several
resource types without travelling. The ask: keep same-resource clumping (good), but push
*different* resources far apart so noots must trade across regions.

## What shipped (`world.rs`, `place_deposits`)

- **One region per element, well separated.** `pick_separated_center` farthest-point-samples
  a region centre for each element — maximising the minimum toroidal distance to the
  already-chosen centres (with a small nudge toward the element's preferred terrain). On the
  30×22 torus this spreads the four element territories ~11 hexes apart.
- **Each element packs into a tight blob.** All of an element's deposits (count =
  `CLUSTERS_BY_CONSUMPTION_RANK[rank] × DEPOSITS_PER_CLUSTER`, unchanged: 12/9/6/3) are placed
  within `ELEMENT_RADIUS = 2` of its centre, replacing the old two-level
  cluster-of-clusters spread. So a resource clumps into a compact territory and the gaps to
  other resources stay wide.
- Removed the now-unused `pick_empty_tile` and `CLUSTER_RADIUS`.

## Verification (headless)

- 5 seeds: all 30 deposits place; intra-element spread 2-6 hexes (tight clumps); **minimum
  inter-element distance 5-7 hexes** (was 3-4 before).
- Economy (3 seeds, 80k) is healthy *and more consistent* than before: consumption
  3292-3475 across seeds (0xBEEF, historically the starving outlier ~1.6k, is now 3.3k),
  deaths regulated (50-68/1e5), trade active (0.7-2.1M). Forcing specialisation balanced
  consumption rather than starving regions.
- `cargo clippy` clean on wasm + native.
- **Unverified**: the on-device look of the four distinct resource territories.

## Notes

- `ELEMENT_RADIUS` is the tightness knob (smaller = tighter clumps, wider gaps, but risks
  not fitting the 12-deposit element); 2 fits comfortably (48% of a 5×5 box).
- Separation is capped by the map: four regions on a 30×22 torus sit ~11 hexes apart, so
  with radius-2 blobs the nearest different-resource deposits are ~5-7 apart — "somewhat far
  apart", as intended.
