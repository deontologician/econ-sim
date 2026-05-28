# 026 — Whole-run graph history, rolled up & persisted

## Context

Reloading a save dropped all the HUD graph data — the stat strip and price graphs started
empty, because their history lived in GUI-only `VecDeque` rings (`StatHistory` /
`PriceHistory`) that were never part of the save. The ask: persist historicals, with
rollups for older windows to save space. Chosen scope (confirmed with the user): keep the
**whole run**, recent at full resolution, older downsampled.

## What shipped

- `src/history.rs` (new, gui-free core module): `RollupHistory<const N>` — a time series
  that retains the entire run at a bounded bucket count (`HISTORY_CAP`). On overflow it
  merges the oldest adjacent pair of equal span (weighted by how many raw samples each
  covers), so the newest buckets stay 1 sample wide and resolution coarsens toward the
  past (≈ power-of-two tiers). Stored as `Vec<Vec<f32>>` + parallel `spans` (serde has no
  blanket const-generic-array impl). `N_STAT_SERIES` (=16) also moved here; `graph::SERIES`
  now re-exports it as `N_SERIES`, so the two can't drift.
- `Snapshot` gained `stat_history` + `price_history` (`#[serde(default)]`), so new saves
  carry them and pre-history saves load with empty series.
- GUI: `StatHistory`/`PriceHistory` are now newtypes over `RollupHistory`; `sample_stats`
  just `push`es (the rollup self-bounds); `render_graphs`/`render_prices` read `.iter()` /
  `.back()`; `save_game` persists both; `setup` restores them on resume.
- Headless `snapshot()` writes empty history (it doesn't sample graphs); a GUI resume of a
  headless save simply starts its charts fresh.

## Verification

- `cargo clippy` clean on wasm + native; 10 tests pass (3 new in `history`: stays within
  cap & keeps newest full-res; rollup preserves the running mean; JSON round-trip).
- Headless: new saves include `stat_history`/`price_history`; an old pre-history save
  (no fields) still loads via serde default.
- **Unverified**: the on-device panel render of restored history — clean wasm compile
  only (the GUI can't run in the sandbox, and headless doesn't sample graphs, so the
  populated round-trip is covered by the serde test rather than an end-to-end GUI run).

## Notes

- Sparklines plot buckets index-spaced, so the rolled-up old end is time-compressed — an
  overview, not a uniform-time axis. Future: show per-bucket span on the axis, a
  recent-vs-whole-run zoom toggle, and min/max envelopes when rolling up.
- No `SAVE_VERSION` bump: the new fields are additive and default-filled.
