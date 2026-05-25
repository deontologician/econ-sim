# Plan 003 — Durable positional goods, starvation death, pause

## Context

Plan 002 left positional goods being auto-consumed into a hidden `stock`, noots
immortal, and the sim impossible to freeze. This increment makes positional goods
behave like real durable wealth, gives noots mortality, and adds a pause control.

## What shipped

### Durable positional goods (held wealth, sold by choice)
- Positional goods are no longer eaten. `consume` (`economy.rs`) handles only
  staples; positional goods stay in `Inventory` and their welfare is computed from
  what's *held*: `positional_utility(goods, inv) = Σ ln(1 + held)` over positional
  items (used by the selection panel). The `Positional`/`stock` component and
  `N_POSITIONAL` are gone.
- Trading now distinguishes the two sides:
  - **`wtp` (buyer):** positional WTP `= POSITIONAL_VALUE/(1+held)`, and only when
    the buyer's staples are satisfied (you buy luxuries once fed; marginal worth
    falls as your holding grows).
  - **`reservation` (seller):** the lowest price to part with a unit. For
    positional goods `= POSITIONAL_VALUE/(1+held) · (1 − POSITIONAL_SELL_URGENCY ·
    hunger_frac)`. Hunger discounts the keep-value, so a starving, goods-rich noot
    liquidates for food money while a fed one only sheds surplus (where marginal
    worth already dropped below the ask). Staples/intermediates keep their old
    valuations.
  - A trade clears at the fixed `ask` when `buyer_wtp ≥ ask > seller_reservation`
    and the buyer is solvent (unchanged structure).
- Self-balancing: hungry → sell positional → buy staples → eat; fed + spare bucks
  → buy/hold positional. No hard switch, just the marginal-utility comparison.

### Starvation death + respawn
- `Hunger` gains `starving_secs`; `fully_starving()` = all staples pinned at max.
- `death_and_respawn` (`main.rs`) accumulates `starving_secs` while fully starving,
  resets it otherwise, and at `DEATH_GRACE_SECS` (20s) **reincarnates** the noot:
  fresh wallet (`STARTING_BUCKS`), empty inventory, `Hunger::fresh()` (half), a new
  `Brain`, idle `HaulContract`, and a new location — owners back on their deposit,
  everyone else at a random tile. Population is conserved ("respawn a replacement"
  without entity churn).
- Spawns (initial + respawn) now use `Hunger::fresh()` (half appetite) instead of
  fully starving, so nobody dies at `t=0` and the cold-start economy has a window.

### Pause
- `Paused(bool)` resource + `sim_running` run condition gating every simulation
  group (`simulate`, income/hunger, haul/movement, extract/refine/trade/settle,
  consume/death/rates). Input, camera, selection and HUD stay live while paused.
- `pause_controls` toggles on **Spacebar** or the on-screen **Pause/Play** button
  (`PauseButton`, an absolutely-positioned top-right UI `Button` with a
  `PauseLabel` caption); the HUD shows `[PAUSED]`.

## Files
`noot.rs` (Hunger rework, drop `Positional`/`N_POSITIONAL`), `economy.rs`
(durable consume, `wtp`/`reservation` split, `positional_utility`), `main.rs`
(pause resource/systems/button, `death_and_respawn`, spawn + panel + HUD updates),
`plans/INTENDED_FEATURES.md`.

## Verification
- `cargo check --target wasm32-unknown-unknown` — clean. (Native build needs Linux
  desktop libs absent here; wasm is the gate.)
- On-device after deploy (not runnable in this env): Space / the button freezes
  and resumes the sim; noots seen liquidating positional goods when hungry and
  accumulating them when fed; a noot kept from food dies after ~20s at max hunger
  and reappears fresh (owners on their deposit).
