# Plan 015 — Full-ECS-state save/load (serde + RON)

## Context

Seed-only persistence (Plan 013) replayed the same map but restarted the sim. The
ask was a true **resume**. After confirming serde/ron are already in Bevy's tree
(so no real bundle cost), we relaxed the "no external crates" rule for the save
module and snapshot the whole simulation.

## What shipped

### serde derives
- `Serialize`/`Deserialize` (+ `Clone`) on the persisted data: world types
  (`World`, `Tile`, `ChosenElement`, `Deposit`, `DepositKind`, `ResourceRole`),
  goods (`WorldGoods`, `ConsumableGood`, `GoodForm`, `GoodCategory`, `ItemRole`),
  `ElementId`, the noot components (`TilePos`, `Inventory`, `Wallet`, `Hunger`,
  `Claim`, `Trader`, `NootMeta`), and the resources (`HungerControl`,
  `IncomeControl`, `EconStats`).

### `save.rs` — full snapshot
- `Snapshot { version, world, hunger, income, stats, noots: Vec<NootSave> }`,
  serialized to RON in `localStorage`. `NootSave` keeps each noot's components plus
  its `RouteMemory.value`/`explore`/`homing` (the eligibility trace is transient and
  rebuilt empty via `RouteMemory::restored`).
- `load` parses + checks `SAVE_VERSION` (==2); any failure/mismatch → `None`, so a
  stale/corrupt blob falls back to a fresh world instead of wedging boot.

### Wiring (`main.rs`)
- `setup` branches: a valid save restores the world, controllers/stats, and respawns
  every noot from its `NootSave`; otherwise it generates a fresh world as before.
- **S** key / "Save" button snapshots the live ECS; **G**/"New" clears + reloads.

## Files
`Cargo.toml` (serde, ron), `CLAUDE.md` (rule note), `noot.rs`/`world.rs`/`goods.rs`/
`elements.rs`/`economy.rs` (derives; `RouteMemory::restored`), `save.rs` (rewrite),
`main.rs` (setup branch, `spawn_restored_noot`, `save_game`, Save button), plans.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Unverified at runtime / in a real
  browser** — serde derives are compiler-checked, but the localStorage round-trip,
  the resume path, and RON size (~hundreds of KB with the per-noot value fields) need
  an on-device check. Save is manual + single-slot; migrations currently discard on
  version mismatch rather than upgrading.
