# 028 — Routes (movement traffic) map overlay

## Context

Follow-on to plan 027 (GPS-taught navigation). With movement now a learned per-step
decision, the question is *what paths emerge*. There was already a **Trades** overlay
(`EconStats::trade_hexes`) showing where deals *close* — but that only marks the endpoints
(marketplaces), not the corridors noots travel to reach them. The ask: an overlay that
accumulates per-hex movement over all noots, to reveal the emergent trade routes / hauling
lanes.

## What shipped

A new **Routes** map overlay, cycled by the same Overlay button (now
none → terrain → trades → routes → none) and `V` key.

- **`EconStats::traffic_hexes: Vec<u32>`** — cumulative count of noot steps *into* each
  tile, mirroring `trade_hexes` (lazily sized, `serde(default)`, persists across saves).
- **`economy::accumulate_traffic`** — runs right after `policy_step` in the shared sim
  pipeline. Uses Bevy's `Changed<TilePos>` filter, so it counts exactly the noots that
  stepped to a new hex *this tick*. Lingering at a deposit/market (no `TilePos` write)
  doesn't accumulate, so the heatmap favours the travelled corridors over the endpoints.
- **GUI (`main.rs`)**: `RouteOverlay` heat cell per tile (cyan, z 1.5, just under the noot
  layer and the gold trade cell), recoloured by `update_route_overlay` (throttled, gated
  on the Routes mode, `sqrt` gamma so lightly-used lanes still read). New `Routes` enum
  variant + `ROUTES_BTN_ON` button tint; the three heat-cell visibility queries are made
  pairwise-disjoint with `Without` filters so Bevy can schedule them.

## Verification

- `cargo check`/`clippy` clean on wasm + native; headless builds; runs.
- Accumulation confirmed via `--save` (seed 0x0EC05EED): after 5k ticks `traffic_hexes`
  populated (653/660 tiles, total 6688 steps) while `trade_hexes` stayed concentrated
  (208/660 tiles, one hub at 4721) — the two heatmaps measure genuinely different things.
- **Unverified**: on-device GUI render (can't launch Bevy here) — only the data feeding it
  is checked.

## Notes

- Counting hex *entries* (not per-tick occupancy) is the key choice: occupancy would be
  swamped by noots parked at deposits/markets; entry-counting paints the lanes.
- u32 matches `trade_hexes`; at ~56 noots a step every few ticks, overflow is far off.
