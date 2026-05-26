# 020 — Committed options (semi-MDP) + trade-density overlay

## Context

After the spatial price field (cheap at source, dear in deficit) and the best-market
heading feature landed, headless rollouts showed the binding constraint wasn't pricing
or perception — it was that the learned policy **never learned to mine**. The
produce→haul→sell→buy-food→eat chain is too long for greedy per-step ΔU reward to
discover, so noots wandered: ~1k cumulative units produced over 60k ticks (≈0.36 noots'
worth of continuous mining) and mass starvation. A reward-shaping stop-gap (intrinsic
bonus per unit produced + exploration biased toward producing) bootstrapped mining as a
stand-in, but the principled fix flagged in the ledger was **committed multi-step plans**.

## What shipped

### Committed options (the principled fix)
The policy now chooses among four **committed options** (semi-MDP macro-actions) instead
of primitive moves:
- **Mine** — navigate to the claimed/nearest deposit, extract until loaded
  (`LOAD_THRESHOLD`).
- **Sell** — haul to `best_market_tile` for the carried cargo, linger (new
  `Action::Idle`, no haul hunger) so passing buyers trade.
- **Refine** — convert held intermediates in place.
- **Explore** — random-walk scouting (find deposits/markets/partners).

A deterministic executor in `policy_step` drives the chosen option to completion (greedy
hex `step_toward` — no pathfinding needed on the impassable-free torus); the policy only
re-decides at option boundaries (`option_done`), banking **one** transition there with
reward = the ΔU the option accrued. The policy never has to crack navigation as a
sparse-reward subtask, so directed behaviour replaces per-step dithering, and the Mine
option's value bootstraps from the later Sell/eat reward.

This let the **reward-shaping crutch be retired entirely** — rollouts are slightly
*better* without it (production ~6.4k→~6.7k, consumption ~3.1k→~3.4k).

`N_ACT` 8→4; old policy saves no longer `fit` and reinitialise (already handled).
`PolicyMemory` carries the committed plan (`committed`/`plan_target`/`plan_ticks`),
reset on death. `features()` decoupled from the action mask (`on_minable` computed
directly via `can_mine_here`).

### Trade-density overlay
`EconStats::trade_hexes` accumulates a per-hex tally of every cleared trade (at the
seller's tile), persisted in saves. A **Trades** toggle (button / `X` key) renders it as
a gold hex heatmap (sqrt ramp, mirroring the Crowd overlay). Headless reports
`trade_top5_share` and `trade_active_hexes`.

## Results (headless, seed 0x0EC05EED, 60k ticks)

vs. the flat-action reward-shaped version:
- Action mix gains real structure: navigate/mine/sell-linger/refine, instead of ~all
  "move".
- Consumption 2382 → 3120 (→ 3376 with shaping removed); starving 40–54 → 16–28; wealth
  167 → 233; experience 77 → 147; trades 190k → 249k; deaths still regulated near target.
- **Agglomeration / "cities"**: ~86–94% of all trades pool into the busiest 5% of hexes
  (`trade_top5_share`).
- **Trekking**: source→sale haul distance 4–6 hexes.

## Files

- `noot.rs` — `Action::Idle`.
- `policy.rs` — option action layout (`A_MINE`/`A_SELL`/`A_REFINE`/`A_EXPLORE`, `N_ACT`=4);
  `PolicyMemory` committed-plan fields; removed `shaping`.
- `economy.rs` — option helpers (`can_mine_here`, `sellable_units`, `has_intermediate`,
  `carried_units`, `option_mask`, `option_target`, `option_done`, `step_toward`);
  rewritten `policy_step`; `EconStats::trade_hexes` + increment in `meet_and_trade`;
  removed shaping from `extract`/`refine` and the reward; reset plan on respawn.
- `main.rs` — `TradeOverlay`/`TradesButton`, `update_trade_overlay`, Trades toggle wired
  into `overlay_controls`, button + per-hex cell spawn.
- `bin/headless.rs` — `Action::Idle` arm + `act_idle`; `trade_top5_share`/
  `trade_active_hexes` metrics.

## Verification

- `cargo clippy` clean on `wasm32-unknown-unknown` and the native headless build; 7 unit
  tests pass.
- Headless rollouts (two seeds) + save/load roundtrip (trade_hexes persists).
- **Unverified**: the GUI can't run in the sandbox, so the Trades overlay render and the
  on-screen behaviour are confirmed only by a clean wasm compile, not visually.
