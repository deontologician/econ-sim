# Plan 013 — Inflation-targeting universal income; save/load + migrations

## Context

Staples and the hunger PID are stable; now control the universal income against
measured inflation, and add localStorage persistence so a world survives reloads
while iterating on the rules.

## What shipped

### Income controller (`economy.rs`)
- `IncomeControl` resource holds the live universal-income `rate` (init
  `BUCKS_INCOME`), the smoothed `measured_inflation`, and the rolling sales window.
- "Inflation" = summed sale value this minute vs. the previous minute
  (`(this − last) / last`). `meet_and_trade` accumulates `this_window`; `income`
  pays out `rate`; `income_controller` runs once per `INCOME_WINDOW` (60 s).
- Integral control toward `TARGET_INFLATION_PER_MIN` (0.1%/min): since more income →
  more spending → faster sales growth, `rate += INCOME_K·(target − inflation)`,
  clamped to `[0, 3]`. Rate itself is the integrator, so it converges with zero
  steady-state error. HUD shows `income ₦/s` and `sales infl %/min → target`.

### Save / load / migrations (`save.rs`, localStorage)
- Versioned `key=value;` blob (`v=1;seed=…`) in `localStorage`; `load` parses and
  runs `migrate(version, …)` up to `SAVE_VERSION` (unknown/corrupt → discarded, so a
  bad blob never wedges boot). Native builds get no-op stubs.
- `setup` reuses the saved seed (replaying the same map across reloads) or rolls and
  persists a new one. A "New" button / **G** clears the save and reloads for a fresh
  world. Added web-sys `Storage` + `Location` features.
- Only the seed is persisted today — the sim restarts on the fixed map. Full
  live-state snapshots are deferred (see INTENDED_FEATURES).

## Files
`economy.rs`, `save.rs` (new), `main.rs` (module, resource, schedule, seed load/store,
New button + handler, HUD line), `Cargo.toml` (web-sys features),
`plans/INTENDED_FEATURES.md`.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Unverified at runtime / on a real
  browser.** Watch: localStorage actually persists the seed across reloads and "New"
  rerolls; the income loop settles near 0.1%/min without oscillating or pinning at a
  clamp (tune `INCOME_K`, `INCOME_MEAS_ALPHA`). Note rebirth-to-`STARTING_BUCKS` is a
  separate, uncontrolled money injection, so the income lever only partly governs the
  money supply.
