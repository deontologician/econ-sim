# Plan 014 — Food storage (escape the Malthusian trap) + learning by doing

## Context

Noots sat on a starvation knife-edge: a fed noot's staple reservation fell with its
hunger, so it dumped *all* its food the moment it was sated and had to re-buy the
instant hunger returned — no buffer, no resilience to supply gaps. And per-noot
productivity was fixed, so the food supply never grew. Two fixes, aimed at letting a
surplus build up over time.

## What shipped (`economy.rs`, `noot.rs`, `main.rs`)

### Food reserve / storage
- Each noot keeps a `FOOD_RESERVE` (4 units) buffer of each staple:
  - **Reservation**: within the reserve, a held staple is priced at full
    `STAPLE_VALUE` — above any ask, so the buffer never sells. Only the true surplus
    beyond the reserve is sold, cheaply when fed (as before).
  - **WTP**: below the reserve a noot also *stocks up* when food is cheap (pays up to
    `FOOD_BUFFER_WTP_FRAC × STAPLE_VALUE`), even when not hungry.
- `consume` still eats the buffer down when hungry; the noot rebuilds it when food is
  available — so the reserve smooths consumption across lean spells instead of
  starving the moment supply dips.

### Learning by doing
- `NootMeta` gains `experience`, accrued per unit mined + refined (reset on death).
- `skill_factor(exp) = 1 + min(exp·SKILL_PER_UNIT, SKILL_BONUS_CAP)` (up to 2×, ~slow:
  ~1000 units to saturate). `extract`/`refine` scale throughput by it, so a long-lived
  worker produces meaningfully more — growing the food/goods supply over time.
- Shown in the selection panel (`skill x.xx×`).

## Deferred
- **Full-ECS-state save/load** (resume mid-run, not just replay the seed) — the large
  serde-less snapshot job — is *not* in this change; still seed-only persistence.

## Files
`economy.rs` (reserve consts + wtp/reservation buffer; skill consts + `skill_factor`;
extract/refine apply skill & accrue experience), `noot.rs` (`NootMeta.experience`),
`main.rs` (selection-panel skill readout), plans.

## Verification & caveats
- `cargo check` + `cargo clippy` (wasm): clean. **Unverified at runtime.** Watch: do
  buffers actually fill and smooth hunger (vs. locking up so much food that the
  un-reserved hungry can't buy)? does skill growth visibly lift production without
  runaway? Tunables: `FOOD_RESERVE`, `FOOD_BUFFER_WTP_FRAC`, `SKILL_PER_UNIT`,
  `SKILL_BONUS_CAP`.
