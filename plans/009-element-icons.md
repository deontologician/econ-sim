# Plan 009 — Thematic vector icons per element

## Context

Resources (the 25 candidate elements, 4 drawn per world) were identified only by a
colour and a name. They needed a recognisable icon shown both on the map and in the
stats overlay. No external crates are allowed and the two surfaces differ (world
mesh vs UI), so the icon is a **procedurally generated texture** usable as a `Sprite`
(map) and an `ImageNode` (HUD).

## What shipped

### `icon.rs` — a tiny SDF vector-icon engine
- A handful of signed-distance primitives (circle, ring, capsule segment, rotated
  box, triangle, hexagon) with `Over`/`Cut` compositing.
- `render_icon(id) -> Image`: rasterizes a per-element **recipe** (a short list of
  primitives + colours) into a 64×64 `Rgba8UnormSrgb` texture, antialiased against
  the pixel size. Each element's recipe is thematic (lightning bolt, water/oil/acid
  droplets, flame, gem, gold ingot, iron/honey hexagons, sun, shadow crescent, wind
  gusts, cloud, dunes, …) and uses the element's own colour as the base hue.
- Generation is a one-off at setup for the 4 chosen elements; bundle size unaffected
  (textures are built at runtime, not shipped).

### Map
- Deposit markers are now a dark backing disc (z 1.0) carrying the element's icon
  sprite (z 1.05), still ringed by the hidden claim outline. Replaces the old flat
  colour-tinted hexagon.

### Stats overlay
- The top HUD panel is now a column: the global-summary text, then one row per
  resource — the element icon (`ImageNode`, 18px) beside its stats line
  (`ResourceLine`). `update_hud` writes the summary and each row text separately
  (disjoint `Text` queries via `With`/`Without`).

## Files
`icon.rs` (new), `main.rs` (generate icon handles in `setup`; deposit disc+sprite;
HUD icon rows + split `update_hud` with `resource_line`), `elements.rs` (doc),
`CLAUDE.md` (refreshed `Where things live`).

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Visually unverified** — no GUI here,
  so the 25 icons are drawn blind. The SDF engine is the load-bearing part and is
  straightforward; individual recipes may need nudging once seen on-device (a few are
  rough). Worth confirming: icon legibility at ~18px, contrast on the dark disc and
  the dark HUD panel, and that sRGB colours look right through the texture path.
