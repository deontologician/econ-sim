# 025 — Noot names & reincarnation ordinals

## Context

Noots were anonymous ("[selected] noot — ..."). The ask: give each a randomly generated
first + last name (like "Tim Dorphindel") at sim start, and when it dies and is reborn,
suffix the name with its incarnation ("the 3rd").

## What shipped

- `NootName { first, last, incarnation }` component (`noot.rs`), with:
  - `random(rng)` — first from a curated short-name list; last assembled from
    start/optional-mid/end syllables (e.g. "Dor"+"phin"+"del"), incarnation 1.
  - `reincarnate()` — `incarnation += 1` (the name itself is kept).
  - `display()` — "First Last" on the first life, "First Last the Nth" after, via an
    English `ordinal()` helper (1st/2nd/3rd, with the 11–13 → "th" exception).
  - `is_unnamed()` — the `Default` placeholder, used to backfill old saves.
- Added to every noot at spawn (GUI fresh + restore, headless fresh + restore). The name
  **survives death**: `economy::death_and_respawn` (which resets the rest of a noot's
  state) now calls `name.reincarnate()` instead of clearing it.
- Persistence: `NootSave.name` (`#[serde(default)]`). A pre-names save loads with the
  unnamed placeholder, which the spawner replaces with a fresh random name (no
  SAVE_VERSION bump needed — purely additive + self-healing).
- GUI: the followed-noot panel shows `name.display()`.

## Verification

- `cargo clippy` clean on wasm + native; 7 tests pass.
- Headless (seed 0x0EC05EED, 60k ticks): names generate well ("Hal Dravanthorn", "Lux
  Korgard", "Yara Norvale"); incarnations climb to 6 as noots die/respawn; deaths still
  occur (so reincarnation runs). Save→load→save round-trips names + incarnations exactly;
  a pre-names save loads and gets fresh names at incarnation 1.
- **Unverified**: the on-device panel render — clean wasm compile only.

## Notes

- Names live on all noots (headless too) so the shared `death_and_respawn` query can
  require `&mut NootName` without skipping any noot.
- Future: show names on map labels / in a trade or obituary log; lineage names on
  inheritance.
