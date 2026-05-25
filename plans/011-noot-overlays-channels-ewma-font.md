# Plan 011 — Noot-colouring overlays, terrain channels, EWMA prices, embedded font

## Context

A batch of asks: richer terrain (channels through rough ground, more clumping, less
extreme roughness), a noot-colouring overlay that ranks the population by a property,
EWMA of actual sale prices, and a fix for Unicode glyphs rendering as tofu.

## What shipped

### Terrain shaping (`world.rs`)
- More clumping (`SMOOTH_SELF_WEIGHT` 0.35→0.25, passes 4→5) for larger regions.
- `ROUGHNESS_GAMMA` (1.7) skews the normalized field toward easy ground, so really
  rough terrain is rarer; cliffs are sparser (`CLIFF_CHANCE` 0.03→0.018).
- `carve_channels`: `N_CHANNELS` meandering low-difficulty corridors carved edge to
  edge (last, so they stay passable even through cliffs).

### Noot-colouring overlays (`main.rs`, `noot.rs`, `economy.rs`)
- `NootColorMode` { Ownership (default), Age, Bucks, Positional, Transactions } in a
  `NootColoring` resource; cycled by the **N** key or a new "Noots: …" button whose
  caption shows the mode. Non-ownership modes min-max scale the property across the
  live population and ramp white (low) → blue (high), rebuilt each frame.
- New `NootMeta { age, transactions }` per noot: `age_noots` advances age (sim-time);
  `meet_and_trade` bumps `transactions` for both sides of each trade; reset on
  respawn. Positional wealth = sum of held positional goods (computed on the fly).

### EWMA sale prices (`economy.rs`, `main.rs`)
- `EconStats.last_price` → `ewma_price`: each trade folds the realized price in with
  `PRICE_EWMA_ALPHA` (lazy-init to the first sample). The HUD resource rows show it.

### Embedded UI font (`main.rs`, `assets/fonts/`)
- Bevy's default font is an ASCII subset, so `₦ → · —` were tofu. Now
  `DejaVuSansMono.ttf` is embedded via `include_bytes!`, added to `Assets<Font>`, and
  used by every `TextFont`. Embedding (not asset-server loading) avoids Pages
  subpath/base-href issues. Unicode glyphs restored in all HUD/panel strings.

## Files
`world.rs`, `noot.rs`, `economy.rs`, `main.rs`, `assets/fonts/DejaVuSansMono.ttf`
(+ license), `CLAUDE.md`.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Visually unverified** (no GUI). Worth
  confirming on-device: channels look passable and terrain reads well; the four noot
  colour ramps are legible and discriminating; EWMA prices settle sensibly; and the
  embedded font renders ₦/→/·/— (the whole point). Font adds ~340 KB to the wasm.
