# Plan 012 — Per-noot explore/exploit temperament; clear money on rebirth

## Context

Two follow-ups: noots should differ intrinsically in how much they explore vs.
exploit their learned value field, and a dead noot's money should not persist (it
was being refilled to `STARTING_BUCKS` on rebirth).

## What shipped

### Intrinsic explore/exploit ratio
- `RouteMemory` gains an `explore` field (the ε in the ε-greedy walk), replacing the
  global `EPSILON`. `value_step` uses the noot's own `explore`.
- Drawn per noot from `[EXPLORE_MIN, EXPLORE_MAX]` (0.03–0.30) at spawn and re-drawn
  on rebirth, so the population spans beelining exploiters → restless wanderers.
- Surfaced in the selection panel (`explore {:.2}`).

### Money cleared on rebirth (money sink)
- `death_and_respawn` now sets `wallet.bucks = 0.0` (was `STARTING_BUCKS`), so a
  noot's hoard leaves circulation when it dies rather than being topped back up.
  Initial spawns still get `STARTING_BUCKS` to bootstrap trade.

## Files
`noot.rs` (`explore` field + range consts), `movement.rs` (`value_step` takes ε),
`main.rs` (spawn/respawn draw ε; clear bucks; panel readout),
`plans/INTENDED_FEATURES.md`.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Unverified at runtime.** Watch that
  clearing bucks on rebirth doesn't trigger a death spiral (reborn noots start broke;
  the hunger PID should absorb it, since a rising death rate lowers the hunger rate) —
  if it does, seed rebirths with a small stipend instead of 0. Also eyeball that the
  explore spread produces visibly different roaming styles.
