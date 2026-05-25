# Plan 004 — Route learning (per-hex TD(λ) value navigation)

## Context

Until now every noot navigated with a *heading bandit* (`Brain`): pick one of six
directions, walk `TRIP_LEN` hexes, return home, reinforce the heading by the trip's
welfare. It learned a coarse compass bearing, not *where* anything is, and the
forced out-and-back trip wasted half the motion returning home empty.

This increment replaces that with **per-hex value learning**: each noot keeps a
TD(λ) value estimate over the whole map and climbs the gradient toward wherever it
has earned reward. It's the first of three coupled changes (route learning →
surplus-discount pricing → merchant transporters); this pass lands the learning
core. Transporters stay on the existing `HaulContract` system for now.

## What shipped

### `RouteMemory` (replaces `Brain`)
- New component (`noot.rs`): a `value: Vec<f32>` over all tiles (indexed
  `row*cols + col`), an eligibility trace (`elig` + a small `active` index list of
  live entries), `pending_reward`, an owner-only `homing` flag, and `move_cooldown`.
- `RouteMemory::learn(from, to, reward)` is one online **TD(λ)** step:
  `δ = reward + γ·V[to] − V[from]`; the left tile's trace is set to 1 (replacing
  trace); then `V[t] += α·δ·e[t]` and `e[t] *= γλ` across the live trace, dropping
  tiles below `TRACE_CUTOFF`. Constants: `α=0.1, γ=0.9, λ=0.8`. The trace spreads a
  destination's reward back along the path that reached it, so whole *routes* gain
  value, not just the tile where reward landed.
- `Brain` (heading/weights/trip_step/outbound/trip_reward) and the unused `Home`
  component are gone, along with `choose_outbound`, `reinforce_and_reset`, `argmax`,
  `ready_to_depart`, and the `TRIP_LEN`/`HEADING_BIAS`/`WEIGHT_DECAY`/`REWARD_CAP`
  constants.

### Navigation (`movement.rs`)
- Each step a free-roamer takes a `value_step`: **ε-greedy** (ε=0.12) over in-bounds
  neighbours by learned value, ties broken at random so an all-zero (unlearned)
  field gives an unbiased walk that sharpens into gradient ascent as value
  accumulates. On every step it banks `pending_reward` (reset to 0) and folds it in
  via `learn`.
- **Owners** alternate two modes by inventory, with hysteresis: while `homing` they
  `step_toward` their deposit and sit on it (the `extract` system fills the load),
  flipping to touring once raw stock ≥ `LOAD_THRESHOLD` (6); while touring they
  value-walk to sell, flipping back to `homing` once stock ≤ `SELL_DONE` (1). They
  still learn on every step, so selling routes get reinforced.
- **Consumers / refiners** are pure value-walkers — they learn *where to buy* (food,
  intermediates) and where to sell, exactly the gap called out in the ledger.

### Reward = welfare + selling income
- `consume` (`economy.rs`) banks staple-eating welfare into `pending_reward`
  (was `Brain::trip_reward`).
- `meet_and_trade` now credits a non-transporter **seller**'s `pending_reward` with
  `price · SELL_REWARD_SCALE` (0.15, scaling bucks to roughly staple-welfare
  magnitude so a sale and a meal pull the field comparably). Buyers are rewarded
  indirectly: the welfare lands when they eat what they bought.

### Transporters (unchanged role, adapted to the new component)
- Still driven by `HaulContract` state. `haul_movement` now reads `move_cooldown`
  from `RouteMemory` and uses a plain random `wander` for the selling leg (the old
  heading bias is gone). Their value field is never trained (skipped in
  `meet_and_trade`) — merchant arbitrage that *will* use it is pass 2.

## Files
`noot.rs` (drop `Brain`/`Home`, add `RouteMemory` + TD(λ)), `movement.rs` (value-step
navigation, owner homing, transporter wander), `economy.rs` (reward plumbing in
`consume`/`meet_and_trade`), `main.rs` (spawn/respawn with `RouteMemory`, drop
`Home`, `n_tiles` wiring), `plans/INTENDED_FEATURES.md`.

## Verification
- `cargo check --target wasm32-unknown-unknown` — clean; `cargo clippy` adds no new
  warnings (native build needs Linux desktop libs absent here; wasm is the gate).
- On-device after deploy (not runnable in this env): owners cycle deposit↔buyers
  instead of out-and-back; consumers/refiners converge on food/trade hotspots rather
  than wandering on a fixed bearing; following a noot shows it returning to where it
  last earned. Tunables if learning looks too sticky or too noisy: `EPSILON`,
  `TD_ALPHA`, `TD_LAMBDA`, `SELL_REWARD_SCALE`.

## Next (pass 2)
Surplus-discount pricing (an overstocked seller's ask falls with holdings) to open a
buy/sell spread, then replace `HaulContract` with free-roaming **merchant**
transporters that buy surplus on their own account and resell for the spread, with a
per-merchant learned discount on anticipated profit.
