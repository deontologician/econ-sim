# Plan 007 — Emergent ownership, unified noots (roles removed)

## Context

Until now noots were born into fixed castes (`Role`: Owner/Refiner/Consumer/
Transporter), and deposits were owned by the owner pre-seeded onto them. This
collapses all of that into **one noot type** with **emergent land claims**: there
are no roles; every noot can mine a deposit it has claimed, refine, consume, and
arbitrage surplus. The hunger-rate PID (Plan 006) is retargeted to the now-uniform
population.

## What shipped

### No roles — one noot
- `Role` is gone. Added a `Noot` marker (so picking/follow/ring queries can still
  find noot entities), a `Claim { deposit: Option<usize> }` component, and renamed
  `Merchant` → `Trader` (the learned `discount` + per-item `cost_basis`), now on
  **every** noot.
- Every system that branched on role is now universal or claim-driven:
  - `hunger_tick`, `consume` — all noots eat/starve (no transporter exemption).
  - `refine` — every noot refines any intermediate it holds.
  - `extract` — a noot mines the deposit named by its `Claim` when standing on it.

### Emergent claims (`claim_deposits`)
- Deposits start unowned. A noot with no claim that's standing on an unclaimed
  deposit claims it (sticky — it keeps its first claim and ignores others). Claims
  live solely in the `Claim` components, so a claim frees automatically when its
  holder dies (claim reset to `None`) and another noot can take the deposit.
- `movement` keys its homing loop off `Claim`: a claim-holder mines until loaded
  (`LOAD_THRESHOLD`), value-walks to sell, and homes to refill when sold out
  (`SELL_DONE`); a claimless noot just free-roams (and claims what it crosses).
- Spawn seeds one noot per deposit (pre-claimed, so mining starts at t=0); the rest
  (`N_ROAMERS`) spawn unclaimed at random tiles. On death a noot respawns unclaimed
  at a random tile.

### Unified valuation (`economy.rs`)
- `wtp` = max(consumption value, arbitrage value). Arbitrage = `discount × base_ask`
  but only when **fed and holding > `ARBITRAGE_RESERVE` bucks**, so a noot never
  spends food money speculating. Intermediates are worth their refined output to
  anyone (all can refine).
- `seller_ask` = `base_ask × surplus_discount(held)` **floored at cost_basis** — so
  freshly mined goods (cost ≈ 0) dump cheap when glutted, while goods bought to flip
  are never resold at a loss (no deadlock, margins ≥ 0). The per-role ask exemption
  is gone.
- On every **buy**: bank a discounted "good deal" reward, average price into cost
  basis, nudge `discount` down. On every **sell**: bank realized margin
  (≈ income for a producer, the spread for a flipper), nudge `discount` up. The
  population thus self-sorts — thriving sellers turn into aggressive arbitrageurs;
  subsistence buyers stay cautious — with no role labels.

### PID retarget + readouts
- `HungerControl` target is now `0.02 × total noots` (everyone is mortal).
- HUD: roster line → `{claimed}/{deposits} claimed · avg appetite · avg discount`;
  kept trade-margin/utility and the deaths→target/hunger-rate line. Selection panel:
  `noot — mining <element>|unclaimed · discount · ₦ · hunger · utility`.

## Files
`noot.rs` (drop `Role`; add `Noot`/`Claim`/`Trader`), `economy.rs` (`claim_deposits`,
universal extract/refine/consume/hunger, unified wtp/reservation/ask + trade,
`WORK_RATE` rename), `movement.rs` (claim-driven homing), `main.rs` (unified spawn,
schedule, death/respawn, PID retarget, HUD/panel, `Noot` marker queries),
`plans/INTENDED_FEATURES.md`.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Dynamics unverified** (no runtime
  here). Watch on-device: do roamers claim freed deposits; does production hold up
  with emergent (rather than seeded) ownership; does the arbitrage/discount split
  emerge without degenerating (everyone hoarding, or prices collapsing)? Levers:
  `ARBITRAGE_RESERVE`, the `SURPLUS_*`/`DISCOUNT_*` constants, `N_ROAMERS`.
