# Plan 018 — Per-tick action selection; drop explicit miner seeding

## Context

Anyone can become a miner (emergent claims), so pre-seeding miners is unneeded. And
refining was universal + automatic — every noot refined its own intermediates each
frame for free. Wanted: mining and refining trade off (pick one per tick), as the
first step of a larger port to learned **action rollouts** (mine / refine / trade /
move).

## What shipped (`noot.rs`, `economy.rs`, `main.rs`)

- New `Action` component (`Move`/`Mine`/`Refine`) — one per noot per tick — and a
  heuristic `choose_action` system that sets it: **Mine** if a claimed deposit is
  underfoot with carry room; else **Refine** if the noot is *claimless* and holds an
  intermediate; else **Move**. This is the seam a learned policy will replace.
- `extract` now acts only on `Action::Mine`, `refine` only on `Action::Refine` — so
  the two are mutually exclusive per tick. Consequence: claim-holders mine and sell
  raw rather than refining their own output; the claimless specialize as refiners
  (buy raw intermediates, convert them).
- No more pre-seeded miners: a fresh start spawns all `N_NOOTS` (56) claimless and
  free-roaming; mining is fully emergent (claim the first unowned deposit crossed).
  Dropped `N_ROAMERS`/`MAX_SEEDED_MINERS`.

## Files
`noot.rs` (`Action`), `economy.rs` (`choose_action`; gate `extract`/`refine`),
`main.rs` (schedule, all-roamer spawn, `N_NOOTS`, `Action` in both spawn bundles),
plans.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Unverified at runtime.** Things to
  watch: with sticky claims, refiner supply = the claimless count (≥ noots − deposits),
  which could bottleneck refined goods if most noots claim; and cold-start has no
  miners until roamers reach a deposit (a few seconds; the hunger PID + spawn jitter
  should absorb it). `choose_action` is a heuristic stub — the real policy (full
  rollout incl. move/trade gating) is the larger port still to come.
