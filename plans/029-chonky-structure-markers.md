# 029 — Chonky full-hex structure markers

## Context

Shops and refineries rendered as a single small diamond (`hex_size * 0.42`, 4-gon), which
was easy to confuse with the small round noots and the disc-and-icon deposits. The ask:
make structures fill the whole hex and read as distinct, chunkier buildings.

## What shipped

`sync_structure_markers` now draws each structure as a stacked, full-hex marker instead of
one diamond:

- **Frame** — dark hexagon `hex_size * 0.97` (≈ the tile size) at z 0.98, an opaque border
  so the structure reads as a solid block filling the hex.
- **Body** — kind-coloured hexagon `hex_size * 0.86` at z 1.0 (cyan shop / orange
  refinery), the `StructureMarker` entity recoloured on a build-over.
- **Emblem** — a white kind-*shaped* glyph at z 1.06: an upward triangle (shop) or a
  diamond (refinery), so the two kinds read apart by shape, not colour alone. New
  `StructureEmblem` component; the mesh is swapped if a build-over flips the kind.

All three sit below the noot layer (z 2.0) so a noot visiting a shop still draws on top,
and below the trade/route heat overlays (z 1.5/1.6) — consistent with how deposits layer.
Shared meshes/materials are built once into a `StructAssets` local (sizes depend on
`MapView::hex_size`, only known after setup); per-structure body materials stay individual
so a build-over can recolour one without touching the rest.

## Verification

- `cargo check`/`clippy` clean on wasm + native, zero warnings.
- **Unverified**: the actual on-screen look — Bevy can't render in the sandbox. This is a
  pure presentation change (no sim/state effect), so it can't break `main`; eyeball it on
  the deployed Pages build on a phone.

## Notes

- Kept structures *below* the heat overlays (like deposits) rather than above; if they
  should stay legible with an overlay on, raise their z above 1.6.
- Emblem shapes are deliberately simple (triangle vs diamond) so the per-kind mesh swap on
  build-over is a cheap handle assignment, no Transform rotation to track.
