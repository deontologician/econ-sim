# Plan 016 — Save migration system (versioned replay) + RON → JSON

## Context

Plan 015 shipped full-state save/load but only *version-gated* (mismatch → discard).
The ask: a real migration system — track a save version, keep a step per
version→next, and **replay all missing steps** on load so old saves upgrade in place.

## Why JSON instead of RON

A value-level replay needs to parse an old save into a generic tree, mutate it, then
deserialize into the current types. Verified in a throwaway crate: `ron::Value` does
**not** round-trip our enums (`DepositKind`, `ItemRole`, …) — `into_rust` errors with
"expected enum … found a map". `serde_json::Value` round-trips them faithfully and
`from_value` works. Both crates are already in Bevy's tree, so JSON is the right tool;
swapped the save blob to `serde_json` and dropped the `ron` dep.

## What shipped (`save.rs`, `Cargo.toml`)

- Save is JSON with a `version` field; `SAVE_VERSION` is the current schema (now 1 —
  the JSON lineage starts fresh; old RON blobs simply fail to parse → fresh start).
- `load`: parse → `serde_json::Value`, read `version`; bail if newer than this build;
  then `while version < SAVE_VERSION { migrate_step(version, &mut value); version += 1 }`
  and finally `from_value::<Snapshot>`. So every missing migration replays in order.
- `migrate_step(from_version, &mut Value)`: the extension point — one branch per
  version bump, mutating the JSON tree (add field w/ default, rename, restructure).
  Empty today (v1 is first); documented with the pattern to add the next step.
- `store`: `serde_json::to_string`. A parse failure / version-too-new → no save.

## Files
`Cargo.toml` (ron → serde_json), `save.rs` (JSON + replay loop + `migrate_step`),
`CLAUDE.md`, `plans/INTENDED_FEATURES.md`.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. Round-trip-through-`Value` (incl. enums)
  and the header/version read were verified in a standalone crate. **The localStorage
  path and a real cross-version migration are unverified at runtime** (no second
  version exists yet to exercise the replay). When the first schema change lands, add
  the `migrate_step` arm and test resuming a pre-change save.
