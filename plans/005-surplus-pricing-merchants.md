# Plan 005 — Surplus pricing + free-roaming merchants

## Context

Pass 1 (Plan 004) gave every noot a learned value field but left transporters on
the old `HaulContract` system and prices fixed. This pass — the second of the
route-learning arc — opens a price spread and turns transporters into independent
arbitrageurs that learn to exploit it. The `HaulContract` machinery is deleted
entirely.

## What shipped

### Surplus-discount pricing (`economy.rs`)
- `ask` is split into `base_ask` (the good's market price, unchanged constants) and
  `seller_ask(item, role, held)`. A non-merchant seller's offer is
  `base_ask × surplus_discount(held)`: full price up to `SURPLUS_FREE` units held,
  then falling hyperbolically toward `SURPLUS_FLOOR` as the overstock grows. So a
  glutted producer dumps cheap, while a hungry owner (high staple reservation) still
  refuses to part with food. This is the spread merchants live off, and advances the
  "inventory-aware asks" ledger item.
- Trades clear at `seller_ask` (was the flat `ask`); `last_price` now varies.
- **Key tunables** (commented in-code): `SURPLUS_FREE`/`SURPLUS_K`/`SURPLUS_FLOOR`
  must make a typical producer stock discount below a merchant's `DISCOUNT_INIT`, or
  no surplus changes hands. Defaults assume owners leave their deposit at
  `LOAD_THRESHOLD ≈ 6`, so ~5–6 units already prices below 0.5× base.

### Merchants replace haulers (`noot.rs`, `economy.rs`, `movement.rs`)
- New `Merchant` component: a learned `discount` and a per-item `cost_basis`
  (running average price paid). Replaces `HaulContract`/`HaulState`. The
  `haul_assign` / `haul_loading` / `haul_settle` / `haul_movement` systems and the
  `PRINCIPAL_SHARE` / `HAUL_CAPACITY` / `HAUL_SELL_STEPS` / `MIN_HIRE` constants are
  gone.
- Merchants free-roam through the normal `movement` system now (no `Without<...>`
  split), so their `RouteMemory` value field finally trains.
- In `meet_and_trade`:
  - **Buy:** a merchant's `wtp` for any tradeable good is `discount × base_ask` (its
    discounted anticipation of resale value). It buys surplus when a producer's
    discounted ask falls at/under that. On buying it banks the *discounted
    anticipated profit* `discount × (base − price)` into its value field (learns
    *where surplus is*), averages the price into `cost_basis`, and nudges `discount`
    **down** toward `DISCOUNT_MIN` (exposure breeds caution).
  - **Sell:** a merchant's `reservation` is its `cost_basis`, and it asks full
    `base_ask` (merchants don't surplus-discount — the margin came from buying
    cheap), so every resale clears at a non-negative margin. On selling it banks the
    realized margin (learns *where buyers are*) and nudges `discount` **up** toward
    `DISCOUNT_MAX` (success breeds optimism).
- Net: successful merchants bid more aggressively for surplus (competing the margin
  away); merchants stuck holding cargo bid less until they clear it — a
  self-regulating population, exactly the "successful anticipate more, unsuccessful
  less" behaviour requested.

### Merchants don't starve
- `hunger_tick` and `death_and_respawn` now skip transporters (they don't eat), so a
  merchant's learned discount, cost basis, and inventory persist instead of being
  wiped on a starvation respawn. They still carry `Hunger` (the trade query needs
  it) — it just stays put.

### Readouts (`main.rs`)
- HUD: the haulers line becomes `merchant discount {avg}  margin ₦{}/s`; the roster
  says "merchants". `EconStats.hauled_*` renamed to `merchant_profit_*` (realized
  resale margin per second).
- Selection panel: a followed transporter shows `merchant — discount X.XX` and its
  cargo.

## Files
`noot.rs` (Merchant, drop HaulContract/HaulState + haul constants), `economy.rs`
(surplus pricing, merchant trade economics, drop haul systems, hunger skip, stats
rename), `movement.rs` (drop haul_movement/wander + contract filter), `main.rs`
(spawn Merchant, schedule, death skip, HUD + panel), `plans/INTENDED_FEATURES.md`.

## Verification
- `cargo check` + `cargo clippy` (wasm target): both clean. Native/runtime can't run
  here, so the **economic dynamics are unverified** — needs on-device observation.
- What to watch on-device: do merchants actually trade (HUD margin > 0, discount
  drifting off 0.50)? If margin stays 0, the spread isn't opening — lower
  `SURPLUS_FREE`/`SURPLUS_FLOOR` or raise `SURPLUS_K`/`DISCOUNT_INIT`. Watch for
  pathologies: consumers near deposits starving because merchants buy out all
  surplus (raise merchant caution / lower `DISCOUNT_MAX`), or prices collapsing.
