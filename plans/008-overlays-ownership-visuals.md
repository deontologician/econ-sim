# Plan 008 — Ownership colour, value/terrain overlays, deposit claims & owner pick

## Context

Visualization pass on top of the unified-noot model (Plan 007): make ownership and
the learned value field legible, and let a tap on a deposit jump to its owner.

## What shipped

### Noots recolour on ownership
- Each noot now owns a **unique** `ColorMaterial` (was a shared handle), so it can be
  tinted individually. `update_noot_color` (gated on `Changed<Claim>`) paints it
  amber (`NOOT_OWNER`) while it holds a claim, green (`NOOT_UNCLAIMED`) otherwise.

### Toggleable overlays (V / T)
- `Overlays { value, terrain }` resource; `overlay_controls` flips them on the V and
  T keys and shows/hides the corresponding hidden hex cells.
- Each tile spawns two hidden overlay cells reusing the hex mesh:
  - **Terrain** (z 0.4, static): green→red by `1 - terrain_factor` (easy→hard).
  - **Value** (z 1.6, under the noot layer at z 2.0): `update_value_overlay` sums
    `RouteMemory::value` across all noots per hex, normalizes to the busiest hex, and
    paints red with alpha ∝ the normalized value (transparent → solid red). Throttled
    to 5 Hz since the field drifts slowly.
- Overlay materials are born with alpha < 1 so `ColorMaterial`'s `From<Color>` picks
  `AlphaMode2d::Blend` automatically and the map shows through.

### Claim outline + click-to-owner
- Each deposit gets a hidden white ring (`Annulus`, z 1.1); `update_deposit_outlines`
  shows it iff some noot currently claims that deposit.
- `pick_selection` now falls back to deposits: a tap that hits no noot but lands on a
  deposit selects (and thus follows + panels + rings) that deposit's owner, found by
  scanning `Claim`s. An empty/unclaimed hit still clears the selection.

## Files
`main.rs` only: `Overlays` resource + `ValueOverlay`/`TerrainOverlay`/`DepositOutline`
components, noot colour consts, overlay-cell + outline spawning in `setup`,
per-noot materials, and the four systems (`overlay_controls`, `update_noot_color`,
`update_value_overlay`, `update_deposit_outlines`) plus the `pick_selection`
deposit branch.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Not run** (no GUI here). The two
  `&mut Visibility` queries in `overlay_controls` are made provably disjoint via
  `With/Without` so Bevy's access checker shouldn't panic, but that's a runtime check
  worth confirming on-device, along with: overlay legibility, that the value heat
  tracks where noots cluster, and that tapping a deposit jumps to its owner.
