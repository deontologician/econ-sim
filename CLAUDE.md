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
- **Dependencies: weigh effort vs. reward.** This ships as wasm to a phone, so the
  bias is toward a small bundle and few deps — but a crate is fine when it clearly
  earns its place. Guidelines:
  - **Don't add a dep for something simple.** Hand-roll the small stuff: the PRNG is
    SplitMix64 in `rng.rs` (also keeps worldgen reproducible from one seed) — don't
    reach for `rand`; icons are drawn procedurally in `icon.rs`, not an image crate.
  - **A crate already in Bevy's tree is nearly free** (no real bundle/compile cost) —
    prefer those. `serde`+`serde_json` (save system) are already pulled in by Bevy.
  - **A genuinely new dep needs a reason**: significant effort saved or correctness
    gained, weighed against bundle size on mobile. When in doubt, ask.
- **Comments explain *why*, not *what*** — hidden constraints, invariants,
  short-circuit-order assumptions. Skip narration of obvious code. No emojis.
- Currency is rendered with `₦`. The UI text uses an **embedded** font
  (`assets/fonts/DejaVuSansMono.ttf`, pulled in via `include_bytes!` in `main.rs`)
  because Bevy's built-in default font is a tiny ASCII subset that renders `₦`, `→`,
  `·`, `—` as tofu. Embedding (vs. loading through the asset server) keeps it robust
  on the Pages subpath. Stick to glyphs that font covers, or extend the font.

## Where things live

- `world.rs` — hex map, terrain, deposits, resource regrowth/depletion.
- `goods.rs` / `elements.rs` — per-world good identity, item roles, element data.
- `icon.rs` — procedural thematic element icons (SDF shapes → RGBA texture), shared
  by the map sprites and the HUD images.
- `noot.rs` — agent ECS components. **No roles**: every noot is one unified type
  (`Noot` marker) carrying `Claim` (which deposit it owns, if any), `Trader`
  (learned arbitrage discount + cost basis), `Inventory`, `Wallet`, `Hunger`,
  `RouteMemory`.
- `economy.rs` — emergent claiming (`claim_deposits`), extraction, universal
  refining, trade (`meet_and_trade`, unified consumption+arbitrage valuation),
  consumption, the hunger-rate PID, reward plumbing, `EconStats`.
- `movement.rs` — locomotion + the per-hex **TD(λ) value-learning** navigation
  (`RouteMemory`); a claim-holder homes to its deposit to refill then value-walks to
  sell, a claimless noot just roams the value gradient.
- `main.rs` — Bevy app wiring, spawn/respawn, camera fit/input, HUD, pause, the
  value/terrain overlays, deposit-claim outlines, and per-noot ownership colour.

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
