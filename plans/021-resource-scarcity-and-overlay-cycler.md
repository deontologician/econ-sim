# 021 — Leaner, role-weighted resources + a single cycling map overlay

## Context

Two requests: (1) fewer starting resources, with deposit abundance ordered by what the
good *is* — raw consumable most common, then the staple that must be refined to eat, then
raw positional, then refined positional (rarest); (2) the crowd-density overlay wasn't
useful — drop it, and fold the remaining map overlays into one button that cycles, like
the noot-colour cycler.

## What shipped

### Deposit abundance by consumption role (`world.rs`)
Deposits were spread evenly: every element got `3 clusters × 4 = 12` deposits (48 total).
Now each slot's cluster count comes from what its consumable good is:

- `consumption_rank(good)`: Staple·Raw → 0, Staple·Refined → 1, Positional·Raw → 2,
  Positional·Refined → 3.
- `CLUSTERS_BY_CONSUMPTION_RANK = [4, 3, 2, 1]`, `DEPOSITS_PER_CLUSTER = 3` (was 4).
- Per slot: `12 / 9 / 6 / 3 = 30` deposits total (was 48) — leaner, food abundant,
  luxuries scarce.

`place_deposits` reads `world.goods.goods[slot]` (populated before placement) to pick the
cluster count; resource-role (replenishable/finite) terrain preference is unchanged.

### Single cycling map overlay (`main.rs`)
- Removed the crowd-density overlay entirely: `ValueOverlay` cells, `update_crowd_overlay`,
  the `Crowd` button, and `Overlays::value`.
- Replaced the separate `Terrain`/`Trades` toggle buttons with one **Overlay** cycle
  button (`MapOverlayButton` + `MapOverlayLabel`) mirroring the noot-colour cycler. A new
  `MapOverlayMode { None, Terrain, Trades }` (with `next`/`label`) lives in
  `Overlays::map`; `V` or the button cycles it, syncing the terrain/trade hex cells'
  visibility and the button caption/tint.
- `update_trade_overlay` now gates on `overlays.map == Trades`.
- Net: the top-right button column drops from 8 buttons to 6.

## Verification

- `cargo clippy` clean on `wasm32-unknown-unknown` and native headless; 7 tests pass.
- Headless (seed 0x0EC05EED, 40k ticks): 30 deposits (was 48), economy still regulates
  (deaths near target, real production/consumption; `claimed` ≈ deposit count, reflecting
  the intended tighter scarcity).
- **Unverified**: the GUI can't run in the sandbox, so the overlay cycler's on-screen
  render/behaviour is confirmed only by a clean wasm compile.

## Notes

- Pre-existing saves keep their `value`/`terrain`/`trades` overlay bools only in old
  blobs; `Overlays` isn't serialized, so there's no save-compat concern. The deposit
  layout only affects newly generated worlds (existing saved worlds keep their deposits).
