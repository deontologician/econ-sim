# 023 — Generic hex ownership + refineries

## Context

Refining used to happen anywhere (a `Refine` action with no place requirement), and
ownership was two ad-hoc claims (`deposit` + `shop`). The user wanted: refining to require
a **refinery** building you must stand in; a unified **generic ownership** where improving
a hex (mine / shop / refinery) claims it, **at most one per noot** (confirmed), abandoned
on death; and the ability to build over an unclaimed structure (e.g. take an abandoned
refinery and make it a shop).

## What shipped

### Unified one-hex claim
- `Claim { hex: Option<usize> }` replaces `{ deposit, shop }` — the single improved tile a
  noot owns. Helpers `owned_deposit`, `nearest_structure` derive role from the hex.
- A noot is therefore a **miner**, **shopkeeper**, or **refiner** (forced specialization);
  most are claimless merchants/explorers until they claim or build.

### Structures (shops + refineries)
- `World::structures: Vec<Structure { tile, kind: StructureKind }>` + `Tile::structure`
  (both `#[serde(default)]`), replacing the shop-specific storage. `build_structure`
  appends, or **rebuilds an unclaimed structure's kind in place** (build-over).
- `Action::BuildShop` / `Action::BuildRefinery`; `build_structures` builds on non-deposit,
  non-owned ground for 100 bucks and claims the hex (one-hex gate: only if claimless).
- `claim_improvements` lets a claimless noot adopt the deposit it mines or the refinery it
  refines at; on death `Claim::hex` resets, freeing the hex (structure stands).

### Refining requires a refinery
- `refine` now only runs when the noot stands on a refinery tile (any — shared); the
  `Refine` option navigates to the nearest refinery first.

### Policy options (N_ACT 5→6)
- `A_MINE` (own/nearest-unclaimed deposit), `A_SELL` (nearest shop, else market),
  `A_REFINE` (nearest refinery), `A_EXPLORE`, `A_BUILD_SHOP`, `A_BUILD_REFINERY`. Masks
  gate on ownership + a precomputed "free deposit exists" / "any refinery exists".

### GUI + staple price cue (also requested)
- `sync_structure_markers` draws shops cyan / refineries orange (recolours on build-over);
  deposit-outline and selection-panel ownership now derived from `Claim::hex`.
- Price graphs colour traces by role (green staple, tan intermediate, gold luxury) and
  tag staple labels with `*`, so the food goods stand out.

## Verification

- `cargo clippy` clean on `wasm32-unknown-unknown` and native headless; 7 tests pass.
- Headless (seed 0x0EC05EED, 100k ticks): ~30 miners + a shared refinery + a shop emerge;
  the refined-goods chain routes through the refinery; production/consumption climb
  (7.1k/4.3k), deaths regulated. Save/load round-trips (structures + claims persist).
- New headless fields: `refineries`, `miners`, `refiners`, `shopkeepers` (replacing
  `shops_owned`). Bumped `headless` `recursion_limit` for the larger JSONL record.
- **Unverified**: GUI render (markers, selection text, price colours) — clean wasm compile
  only.

## Notes

- Owning a building has no direct economic edge yet (shared use); building is driven by
  need (a refinery to refine; a shop waypoint). Few structures emerge (one shared refinery
  suffices). Logged `partial` — owner incentives/upkeep/role-switching are future work.
- Old saves load with empty structures and reset claims (Claim/world schema changed);
  the policy resets too (N_ACT changed), as before.
