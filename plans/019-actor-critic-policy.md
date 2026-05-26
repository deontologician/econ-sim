# Plan 019 — Off-policy actor-critic policy (mine/refine/trade/move) with a Maslow utility reward

> On execution (after approval), first copy this file to
> `plans/019-actor-critic-policy.md` in the repo (plan mode only edits this scratch file).

## Context

Noot decision-making is a hand-written heuristic (`economy::choose_action` sets a
per-tick `Action`), and navigation is a separate per-hex TD(λ) value field
(`RouteMemory` + `movement::value_step`). The user wants to port both into a single
**learned, off-policy actor-critic** over the explicit MDP:

- **State (MDP):** absolute hex position, bucks, hunger (2 staples), positional
  wealth, and whether the noot owns a deposit.
- **Actions:** `Mine`, `Refine`, `Trade`, `Move`.
- **Learning:** one **shared** actor-critic (all noots train one brain, pooled
  trajectories, survives deaths), trained **off-policy** from a replay buffer with an
  **A2C + target-critic** update.
- **Reward = Δ(utility)** where utility is a **Maslow-hierarchy** function over the
  game's concepts (physiological → safety → esteem).

Per the user's "unified critic" choice, the **critic's value over absolute position
replaces `RouteMemory`**: `Move` steps toward the highest-value neighbor hex. The
heuristic `choose_action`, the TD(λ) machinery, and the `pending_reward` plumbing are
removed; reward is derived purely from the utility delta.

Outcome: emergent, learned behavior (when to mine vs refine vs trade vs move, and
where to go) instead of hand-coded rules — the seam the user is porting toward.

## Decisions (from clarifying questions)
- **Unified critic** drives movement (drop `RouteMemory`); position enters the net as a
  per-tile embedding so the critic represents a spatial value surface.
- **A2C + replay + slow target critic** (not full discrete-SAC) — tractable, hand-rolled.
- **Shared brain**: a single `ActorCritic` resource; per-noot diversity via exploration.
- **Reward = ΔU**, U a Maslow-tiered utility (physiological/safety/esteem).
- Hand-rolled tiny MLP, **no ML crates** (consistent with the dependency guidance).

---

## Design

### 1. State featurization (`policy.rs::features`)
Per noot, built at each decision:
- **Position** → an index `pos = row*cols + col` into a learned per-tile embedding
  table `embed: [N_TILES][H]` (treat the 660-wide one-hot as an embedding lookup —
  no dense 660-vector matmul).
- **Other features** (`[f32; 6]`): `tanh(bucks/100)`, `hunger[0]/SAT`, `hunger[1]/SAT`,
  `positional_utility(inv)/ESTEEM_NORM`, `owns ? 1:0`, bias `1.0`.

### 2. Shared model (`policy.rs::ActorCritic`, a `Resource`)
1-hidden-layer MLP, `H=32`, tanh trunk; weights flat `Vec<f32>` (serde-friendly):
- trunk pre-activation `z = embed[pos] + W_other·other + b1` → `h = tanh(z)`.
- **actor head** `Wa:[4][H], ba:[4]` → 4 logits.
- **critic head** `Wv:[H], bv` → scalar `V`.
≈ 21.5k params (~86 KB f32; the embedding dominates). Forward/backprop hand-coded.
Cache `proj = W_other·other + b1` once per noot so neighbor-V evals are just embedding
lookups + head dots.

### 3. Action masking
Build `[bool;4]` from the exact gates already in `choose_action`/`extract`/`meet_and_trade`:
- **Mine**: claim present, its deposit tile == noot tile, raw stock < `CARRY_CAP`.
- **Refine**: claimless and holds an `Intermediate` (matches current refine gate).
- **Trade**: a counterparty within `hex_size*TRADE_RADIUS_FACTOR` (cheap proxy; the
  trade still only clears if economically feasible in `meet_and_trade`).
- **Move**: always valid (fallback). Masked logits → `-inf` before softmax.

### 4. Unified-critic navigation (replaces `RouteMemory`)
When the sampled action is `Move`: evaluate `V(s')` for each in-bounds neighbor
(same `proj`, swap `embed[pos']`), step to the argmax (ε-greedy via the per-noot
`explore` for spatial exploration). No homing state machine — the critic learns that
deposits/buyers are valuable and the policy picks Mine/Trade once there. The value
overlay (HUD) is repurposed to render the critic's `V` per tile.

### 5. Maslow utility & reward (`policy.rs::utility`)
Tiers from game concepts, each ~[0,1], higher tiers gated by lower satisfaction:
- `phys  = hunger.utility() / N_STAPLES`                       (being fed)
- `safety= 0.5·clamp(min_staple_held / FOOD_RESERVE,0,1)
         + 0.5·clamp(bucks / SAFETY_BUCKS,0,1)`                (food buffer + savings)
- `esteem= positional_utility(inv)` (Σ ln(1+held))             (status/wealth)
- gate `g(x)=clamp((x−0.5)/0.4,0,1)`
- **`U = W_PHYS·phys + W_SAFE·g(phys)·safety + W_ESTEEM·g(phys)·g(safety)·esteem`**
  (e.g. 1.0 / 0.6 / 0.4) — esteem only rewards once fed **and** safe = the hierarchy.

**Reward** for the transition ending this step: `r = U(s') − U(s) − DEATH·died`.
ΔU already penalizes rising hunger and rewards eating/earning/accumulating, so no
separate living cost. `died` = the noot starved & respawned this step (`U` resets).
This **replaces** the `pending_reward` banking in `consume`/`meet_and_trade`.

### 6. Off-policy A2C (`policy.rs`)
- **Replay buffer** (`Resource`, ring, cap ~16k): `Transition{ s:(pos,[f32;6]), a:u8,
  r:f32, s2:(pos,[f32;6]), mask2:[bool;4], done:bool }`. Transient (not saved).
- **Per-noot `PolicyMemory` component**: caches last `(s,a)` and last `U` (for ΔU).
  Transient (reinit on spawn/load).
- **Decision step** (one per noot per move-cooldown tick = the MDP timestep): compute
  `U_now`; if a cached `(s,a)` exists push the transition; featurize; sample action
  from masked softmax (`SimRng`); set `Action`; cache `(s_now,a_now,U_now)`.
- **Training system**: once buffer ≥ WARMUP, each frame draw a minibatch (B=32):
  target `y = r + γ·V_target(s2)·(1−done)` (slow **target** copy, Polyak τ=0.01);
  advantage `A=y−V(s)`; critic grad on `(V−y)`; actor grad `−A·∇logπ(a|s) −
  β·∇entropy`; Adam-ish or SGD+momentum, lr 1e-3, γ=0.95, β≈0.01. Backprop is
  hand-coded through head→trunk (+ the single active embedding row).

### 7. Decision cadence
The decision tick = the existing per-noot move cooldown (`BASE_STEP_TIME`/terrain).
A chosen action persists over the interval: `extract`/`refine` apply each frame gated
on `Action`; `meet_and_trade` requires `Action::Trade`; `Move` applies one hex step at
the tick. Reward accrues as ΔU between ticks.

---

## Integration

**New `src/policy.rs`**: `ActorCritic` (resource, Serialize/Deserialize), `ReplayBuffer`
(resource, transient), `PolicyMemory` (component), `features`, `utility`, forward,
backprop, `sample_action`, `best_move_neighbor`. Plus `#[cfg(test)]` unit tests.

**`src/noot.rs`**: add `Action::Trade`; **remove `RouteMemory`** (value/elig/TD `learn`/
homing/pending_reward); keep `explore` (move it to `PolicyMemory`); keep `NootMeta`
(skill), `Trader`, `Hunger`, etc.

**`src/economy.rs`**: delete `choose_action`; new `policy_step` system (decision +
transition recording + sets `Action`); gate `meet_and_trade` on `Action::Trade`
(a trading noot may buy/sell with any nearby counterparty); **remove** reward banking
from `consume`/`meet_and_trade` (keep their economic effects); new `train_policy`
system.

**`src/movement.rs`**: rewrite `movement` — on `Action::Move` at the cooldown tick,
step to the best-`V` neighbor (ε-greedy); drop `value_step`/`learn`/homing.

**`src/main.rs`**: `.init_resource::<ActorCritic>()` + `ReplayBuffer`; schedule —
replace `choose_action` with `policy_step`, add `train_policy` (after
trade/consume so rewards are current); spawn bundles swap `RouteMemory` →
`PolicyMemory` (+ `Action::Trade` available); `death_and_respawn` resets
`PolicyMemory` (not the shared net); value-overlay system reads critic `V`.

**`src/save.rs`**: add `pub policy: ActorCritic` to `Snapshot`; bump `SAVE_VERSION`
1→2; `migrate_step` arm inserts a fresh-init net for v1 saves; `save_game`/`setup`
gather/insert it. `NootSave` drops `value` (RouteMemory gone), keeps `explore`.
Replay buffer / target net / Adam moments / `PolicyMemory` are transient.

**Reused**: `positional_utility`, `Hunger::utility`, the `Action` gating pattern in
`extract`/`refine`, `meet_and_trade` pricing (`Trader`), `SimRng`, `tile_idx`/
`neighbors`/`in_bounds`, the value-overlay rendering.

---

## Verification
- **Gate**: `cargo check` + `cargo clippy --target wasm32-unknown-unknown` clean
  (zero warnings).
- **Native unit tests** (`#[cfg(test)]`, no GUI/wasm): (1) masked softmax sums to 1 &
  masked actions are 0; (2) finite-difference gradient check on one weight vs analytic
  backprop; (3) replay ring wraparound/sampling bounds; (4) `utility` monotonicity &
  Maslow gating (esteem contributes ~0 when starving); (5) `features` range/determinism.
- **On-device after deploy** (no browser here): extend HUD/selection panel with policy
  diagnostics — per-action selection mix, mean `V`, mean reward/step, policy entropy.
  Learning signals: entropy falls from ~ln4≈1.39; reward/step trends up;
  `production_rate`/`utility_rate` (existing HUD) match or beat the old heuristic;
  noots visibly seek deposits, mine, then trade. Save → reload resumes the trained net.
- Land per usual: branch `claude/tech-tree-elements-supply-kq4N1`, commit, fast-forward
  `main` (no PR) → Pages deploy. Update `plans/INTENDED_FEATURES.md` (Action selection
  stub → learned rollout; Movement-learning RL entry → unified critic).

## Critical files
- `src/policy.rs` (new) — net, replay, `PolicyMemory`, features, utility, train.
- `src/economy.rs` — remove `choose_action`/reward-banking; `policy_step`,
  `train_policy`; gate `meet_and_trade` on `Action::Trade`.
- `src/movement.rs` — critic-driven `Move`; drop `value_step`/homing.
- `src/noot.rs` — `Action::Trade`; remove `RouteMemory`; `explore`→`PolicyMemory`.
- `src/main.rs` — resources, schedule, spawn bundles, overlay-from-critic, respawn reset.
- `src/save.rs` — `Snapshot.policy`, `SAVE_VERSION` 1→2, migration, gather/restore.

## Risks / notes
- Biggest behavioral risk: a single shared net learning good spatial navigation from
  the position embedding — if the value surface is too coarse, navigation degrades vs
  the old per-hex TD. Mitigation: the embedding is per-tile (full resolution); watch
  the value overlay on-device and tune `H`/lr/γ.
- Net adds ~86 KB to the wasm-resident state and ~150–250 KB to each JSON save
  (acceptable; localStorage ~5 MB). Persist weights only (optimizer state transient).
- This is a large, single-PR-sized rewrite touching every agent system; it cannot be
  runtime-verified in this sandbox, so the native unit tests + on-device HUD signals
  are the safety net.
