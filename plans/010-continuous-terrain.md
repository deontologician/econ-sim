# Plan 010 — Continuous terrain difficulty, overlay-only

## Context

Terrain was binary (`Terrain::Easy`/`Difficult`, `terrain_factor` 1.0/0.55) and the
base tiles were tinted by it — which fought the red value-field overlay. Two asks:
make difficulty continuous (anchored to neighbours, with occasional cliff-like
jumps), and stop tinting tiles by it so the value overlay reads cleanly.

## What shipped

### Continuous difficulty field (`world.rs`)
- `Terrain` enum removed. `Tile` now carries `difficulty: f32` in `[0, 1]`
  (0 = easy, 1 = cliff). `terrain_factor(difficulty)` is continuous:
  `1 - DIFFICULTY_SLOWDOWN · difficulty` (1.0 → 0.45), used by growth, extraction,
  and movement just as before.
- `generate_terrain`: white noise → `SMOOTHING_PASSES` relaxations toward the
  neighbourhood mean (each hex anchored to its surroundings; out-of-bounds reads as
  max difficulty for rugged edges) → range stretched back to `[0, 1]` → **cliffs**:
  each hex has a `CLIFF_CHANCE` of seeding a sharp jump to near-max difficulty
  (spreading to some neighbours so it reads as a short ridge, not a dot).
- Deposit placement now biases on a difficulty threshold (`prefer_hard`):
  replenishables favour low difficulty, finite stocks favour high.

### Difficulty is overlay-only (`main.rs`)
- All base tiles share one neutral material (`srgb 0.18,0.20,0.22`); the old
  easy/difficult tint is gone. Difficulty is shown solely by the toggleable terrain
  overlay (T), now a smooth green→red gradient driven by `tile.difficulty` (alpha
  raised to 0.5). With nothing intrinsic on the tiles, the red value heat overlay is
  legible.

## Files
`world.rs` (terrain model + generation + deposit bias), `movement.rs` and the
in-`world.rs` growth/extraction calls (pass `difficulty`), `main.rs` (neutral tiles,
overlay from `difficulty`, drop `Terrain` import), `plans/INTENDED_FEATURES.md`.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Visually unverified** (no GUI). Worth
  confirming on-device: that the field looks continuous with believable cliffs (tune
  `SMOOTH_SELF_WEIGHT`, `CLIFF_CHANCE`, `SMOOTHING_PASSES`), and that the neutral
  tiles read well under both overlays and none.
