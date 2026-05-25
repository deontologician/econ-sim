# econ-sim — working notes for Claude

A **mobile-first** Bevy (0.18) agent-based economy sim that deploys to the web as
WebAssembly — the primary target is a phone in portrait, played by touch.
Noots (the agents) extract, refine, trade, consume, learn where to go, and die of
starvation. No gameplay input beyond pan/zoom/pause and tapping a noot to follow it.

## Build & verify

- **Gate:** `cargo check --target wasm32-unknown-unknown`. This is the build that
  must pass — the project ships as wasm.
- **Native (`cargo run`/`check`) does not work in the web/CI sandbox**: Bevy's
  desktop backend needs Linux GUI/GPU system libraries that aren't installed here.
  The app therefore can't be launched or visually verified from this environment;
  say so plainly instead of claiming a feature was seen working. Behavioural
  verification happens on-device after deploy.

## Linting — zero-warning policy

**Always run clippy and fix *every* warning before committing — including
pre-existing ones, not just warnings your change introduced.** Leave the tree at
zero clippy warnings.

```
cargo clippy --target wasm32-unknown-unknown
```

- Prefer the real fix (e.g. `is_none_or`, `is_multiple_of`, collapsing identical
  branches) over silencing.
- Only suppress with a *targeted* `#[allow(...)]` plus a one-line reason when the
  lint fights an inherent idiom — e.g. `clippy::type_complexity` on a wide Bevy
  system `Query`. Never blanket-allow at crate level.

## Conventions

- **Mobile-first / touch-first.** Assume a phone in portrait as the primary device.
  Every control needs an on-screen, finger-sized tap target — keyboard shortcuts are
  extras, never the only way to reach a feature. The camera fits the map to the
  screen on launch; gameplay input stays limited to pan / pinch-zoom / pause / tap.
  Design and reason about touch ergonomics before desktop.
- **No external crates beyond Bevy** (plus `web-sys`/`js-sys` on wasm). The PRNG is
  hand-rolled (`rng.rs`, SplitMix64) to keep the bundle small and worldgen
  reproducible from a single seed. Don't reach for `rand` etc.
- **Comments explain *why*, not *what*** — hidden constraints, invariants,
  short-circuit-order assumptions. Skip narration of obvious code. No emojis.
- Currency is rendered with `₦` (falls back to `N` if the font lacks the glyph).

## Where things live

- `world.rs` — hex map, terrain, deposits, resource regrowth/depletion.
- `goods.rs` / `elements.rs` — per-world good identity, item roles, element data.
- `noot.rs` — agent ECS components (`Role`, `Inventory`, `Wallet`, `Hunger`,
  `RouteMemory`, `HaulContract`).
- `economy.rs` — extraction, refining, trade (`meet_and_trade`), consumption,
  reward plumbing, `EconStats`.
- `movement.rs` — locomotion + the per-hex **TD(λ) value-learning** navigation
  (`RouteMemory`); transporters move by haul-contract state.
- `main.rs` — Bevy app wiring, spawn/respawn, camera/input, HUD, pause.

## Plans & the feature ledger

- Each increment gets a short writeup in `plans/NNN-title.md` (context, what
  shipped, files, verification). Newest number wins.
- **`plans/INTENDED_FEATURES.md` is a living ledger.** Whenever something is
  faked/stubbed/simplified to keep moving, add or update an entry: what shipped now,
  why it's a stand-in, and the principled version intended. Keep entries honest —
  mark `stub`/`partial`/`deferred`/`done`.

## Git

- Develop on the assigned feature branch (`claude/...`). **Commit freely to the
  branch to checkpoint work — even if it doesn't compile.** WIP saves are expected;
  the branch is scratch space, not a clean-history release.
- **Merge the branch to `main` whenever the change compiles cleanly (`cargo check`
  + `cargo clippy` both green) and you don't believe it will break `main`.** No PR
  needed. This is the release gate, and merging to `main` triggers the Pages deploy.
- "Won't break `main`" is a judgment on the build plus the soundness of the change:
  the app can't be launched in this sandbox, so a clean compile is the strongest
  automated signal but is not runtime verification.
