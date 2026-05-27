# 022 — Player-built shops + noot stack indicator

## Context

Two requests: (1) noots should be able to spend 100 bucks to build a shop on an empty
hex — a waypoint they can seek — that becomes claimable when the owner dies; (2) a stack
indicator showing how many noots occupy a hex (their sprites overlap on a shared tile).

For (1) the chosen role (clarified with the user) is a **personal sell post**: a noot's
own shop becomes its Sell destination, a stable home base versus the shifting price-field
market.

## What shipped

### Shops (`world.rs`, `noot.rs`, `economy.rs`, `policy.rs`)
- `World::shops: Vec<Shop { tile }>` and `Tile::shop: Option<usize>` (both
  `#[serde(default)]` so old saves load). `World::build_shop` / `tile_empty` helpers.
- Ownership mirrors deposit claims: `Claim::shop: Option<usize>` (derived, not stored on
  the shop). On death both claims reset (`death_and_respawn`), freeing the deposit and
  leaving the shop standing for another noot to adopt.
- New **Build** committed option (`A_BUILD`, `N_ACT` 4→5): available when a noot owns no
  shop and holds ≥ `SHOP_COST` (100) + `SHOP_BUILD_BUFFER` (50). Its executor sets
  `Action::Build` on open ground (else hops to a neighbour). `build_shops` raises the
  shop, claims it for the builder, and deducts 100 bucks.
- `claim_shops` lets a shopless noot adopt an unowned shop it stands on (so abandoned
  posts get reused rather than piling up).
- **Sell retarget**: `option_target` for `A_SELL` heads to the noot's own shop tile if it
  owns one, else the best-market tile as before.
- GUI: `sync_shop_markers` spawns a cyan diamond per shop (incrementally, since shops are
  created during play); persists across save/load.

### Noot stack indicator (`main.rs`)
- A pool of `N_NOOTS` world-space `Text2d` labels (`StackLabel`), repositioned each
  refresh by `update_stack_labels` to every hex holding ≥2 noots, captioned with the
  count; unused labels parked hidden.

## Verification

- `cargo clippy` clean on `wasm32-unknown-unknown` and native headless; 7 tests pass.
- Headless (seed 0x0EC05EED): ~22→25 shops emerge and stabilise over 80k ticks (the cash
  gate caps the build rate), all owned, economy stays healthy (production/consumption
  climbing, deaths regulated). New headless fields: `shops`, `shops_owned`, `act_build`.
- Save/load round-trips with shops (23 → 23 on reload, ownership intact).
- **Unverified**: the GUI can't run in the sandbox, so the shop markers and the stack
  labels (including `Text2d` size/placement) are confirmed only by a clean wasm compile.

## Notes

- `act_build` reads ~0 in samples because building is a single-tick action rarely caught
  in an instantaneous snapshot; the rising `shops` count is the real signal.
- Old policy saves no longer `fit` (N_ACT changed) and reset to fresh, as before.
