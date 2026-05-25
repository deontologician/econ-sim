# Plan 002 — Transporters + map UX (follow, cleanup, random seed, utility RL)

## Context

The economy (Plan 001) runs, but an owner has to **leave its deposit to sell**
(it lingers extracting only until it carries `LOAD_THRESHOLD` raw units, then
tours), so it can't both produce and distribute. This plan adds **transporters**
(principal–agent hauling) so owners can stay put producing while hired haulers
carry goods to market. It also clears dead UI, adds inspection UX, randomizes the
world, and makes agent learning welfare-driven.

## What shipped

### Transporters (principal–agent hauling)
- `Role::Transporter` + a `HaulContract { state, employer, deposit, cargo_item,
  proceeds, sell_steps }` component (`HaulState`: Idle→ToPickup→Loading→Selling→
  Returning). Constants in `noot.rs`: `PRINCIPAL_SHARE=0.6`, `HAUL_CAPACITY=12`,
  `HAUL_SELL_STEPS=12`, `MIN_HIRE=4`.
- **Payment = revenue split on return**: the hauler sells the owner's cargo at
  market (banking the price into its own wallet), tallies `proceeds`, and on
  returning to the owner pays `proceeds * PRINCIPAL_SHARE`, keeping the rest;
  unsold cargo is handed back. Bucks are conserved.
- Systems (`economy.rs` + `movement.rs`): `haul_assign` (idle hauler → most-loaded
  unclaimed owner; two-pass over one query to avoid aliasing; **stub** for a real
  labor market), `haul_movement` (`With`-disjoint from `movement`; navigates by
  deposit tile, wanders while Selling), `haul_loading` (`get_many_mut` transfer
  owner→hauler), proceeds hook in `meet_and_trade` (`Query<&mut HaulContract>`),
  `haul_settle` (`get_many_mut` payout + cargo return).
- Emergent: a hauler draining the owner keeps it below the depart threshold, so
  the owner keeps extracting; with few/slow haulers it tours itself (fallback).
- 6 transporters spawned (magenta) for ~12 owners.

### Welfare / utility + buyer rewards (added after approval)
- Implemented utility: `Hunger::utility()` = Σ `(satiation−appetite)/satiation`
  ("not starving"); `Positional::utility()` = Σ `ln(1+stock)` (diminishing).
- `Brain.sold_this_trip` (bool) → `Brain.trip_reward` (f32). `consume` now accrues
  the **welfare realized** (hunger relieved + positional log-gain) into the
  consuming noot's `trip_reward`; `reinforce_and_reset` reinforces the heading by
  that reward (capped). This **rewards buyers for buying** (buy → consume →
  reward) on the same footing sellers had. Transporters skip `consume` (they
  don't eat cargo).
- `EconStats` gains `utility_total`/`utility_rate` (and `hauled_total`/
  `hauled_rate`), surfaced in the HUD.

### Removed investment vestiges
- Deleted invest buttons, 1–4 digit investing, `invest()`, `ElementButton`/
  `ButtonLabel`, button colors, and the efficiency/invest text in the HUD. The
  `efficiency` field stays (fixed 1.0); see the "Extraction efficiency" ledger
  entry for the intended per-extractor design.

### Select a noot & follow + info panel
- `Selection(Option<Entity>)` resource. `pick_selection` (tap/left-click,
  distinguishing a tap from a pan via `TAP_SLOP`; `viewport_to_world_2d` +
  nearest-noot hit-test) selects/clears. `follow_selected` lerps the camera onto
  the selected noot; a real pan (`touch_camera`/`keyboard_mouse_camera`) clears
  the selection. `update_selection_ring` shows an `Annulus` highlight.
  `update_selection_panel` fills the bottom strip (vacated by the invest buttons)
  with role/bucks/hunger/utility/cargo, plus haul state for transporters.

### Random seed each load
- `random_seed()` draws entropy at startup (`js_sys` Date+random on web,
  `SystemTime` natively); the HUD prints the seed (reproducible per run).

## Files
`noot.rs`, `economy.rs`, `movement.rs`, `main.rs`, `Cargo.toml` (js-sys wasm dep),
`index.html` (unchanged this plan), `plans/INTENDED_FEATURES.md` (ledger updates).

## Verification
- `cargo check --target wasm32-unknown-unknown` — clean, zero warnings. (Native
  build needs Linux desktop libs absent in CI env; wasm is the gate.)
- On-device after deploy: seed differs each load; magenta haulers cycle
  deposit→buyers→deposit while owners stay producing; bucks conserved on settle;
  tap a noot to follow + see its panel; pan/empty-tap deselects; invest UI gone.
