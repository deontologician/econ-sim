# 035 — Server-grown tech tree (research → discover → construct)

Status: **Phases 1–2 shipped**, Phase 3 designed. Three-phase feature; this doc is the
contract for all three.

- **Phase 1 (shipped)**: client `Research` action + per-resource demand reporting.
- **Phase 2 (shipped)**: server aggregates demand, grows + names the global tech tree
  (`src/tech.rs` pure logic; `src/bin/server.rs` growth task, OpenRouter naming with
  procedural fallback, `GET /tech`, tree in the `/submit` response, HTML section,
  `tech_state.json` persistence). Verified live end-to-end (procedural naming; LLM kicks in
  once `OPENROUTER_API_KEY` is set on the Sprite).
- **Phase 3 (next)**: client fetches the tree, discovery-on-research, workshop construction,
  tech-item effects.

## Goal (user's framing)

Noots accumulate items, take a **research** action that records demand on the server. The
server slowly grows a **global, live** tech tree: it looks at which resources are researched
most across *all* agents in *all* worlds and, with some probability, grows a new tech whose
inputs are the in-demand resources. The next time a noot researches while holding the new
tech's prerequisite items, it **discovers** the tech. Discovered, a world's noots can **build
a constructor building** that consumes the prerequisites and outputs the tech as a new good.
The server randomizes each new tech's attributes (game effect; consumable/durable;
positional/staple; misc) and **names** it with a cheap LLM (OpenRouter, e.g. a Qwen model;
`durable stick + iron → shovel`), with a procedural fallback.

## Decisions (locked)

- **Naming**: OpenRouter (cheap model, Qwen-class), key as a Sprite secret
  `OPENROUTER_API_KEY` + `OPENROUTER_MODEL` (default a cheap Qwen). Procedural word-blend
  fallback when the key is absent or the call fails — so the feature never hard-depends on
  the LLM.
- **Scope**: **global & live**. One shared tree on the Sprite; a grown tech becomes
  discoverable in every world, including running ones. Consequence: a world's late-game
  evolution depends on server state, not its seed alone — an accepted break from strict
  seed-determinism (worldgen itself stays seed-reproducible).
- **Sequencing**: plan all (this doc), build **Phase 1** now.

## Global identity bridge (the crux)

Goods are **per-world**: inventory is a fixed `[f32; N_ITEMS]` (`N_ITEMS=8` = 4 element slots
× {Raw,Refined}) indexed locally; each world draws 4 of the 25 global `ELEMENTS`
(`world.chosen[slot].id : ElementId`). A *global* tree can't key on local indices. So:

- **Global item identity** = `(ElementId, GoodForm)` — 25 × 2 = 50 stable keys across all
  worlds. A local item `i` maps out via `world.chosen[i/2].id` + `GoodForm::from(i%2)`.
- **Research demand** is reported per global item identity (the client sends per-element/form
  demand; the server aggregates into a 50-wide histogram).
- **A tech's recipe** is a set of `(ElementId, GoodForm, qty)` inputs. A world can discover /
  construct a tech only if its `chosen` elements cover the recipe's `ElementId`s — so tech
  availability naturally varies with a world's resource endowment.

## Phase 1 — client research action + demand reporting (this session)

Self-contained, deterministic, headless-verifiable. No server or tech-item gameplay yet; it
lays the action, the data, and the wire format.

1. **`Action::Research`** (`noot.rs`) — new transient action variant.
2. **Policy** (`policy.rs`): `A_RESEARCH = 6`, `N_ACT = 6 → 7`. All of `logits` /
   `masked_softmax` / `backward` are generic over `N_ACT`; the actor head `wa` is
   `[a*H + j]` row-major, so the new action is the trailing block. `N_OTHER` stays **28**
   (no new feature column) to keep the save migration to just the actor head.
3. **Decision wiring** (`economy.rs`):
   - `option_mask`: `mask[A_RESEARCH] = carried_units() > ε` (holds any non-junk item).
   - `option_target`: `A_RESEARCH → None`/stay (research in place); executor sets
     `Action::Research` with no move.
   - new system **`research`** (chained right after `refine` in `add_sim_systems`): when
     `Action::Research`, accrue `stats.research_demand[i] += inv.items[i] * TICK_DT` for each
     held non-junk item, and bump a `research_total` / per-tick `research_rate`.
   - **Reward stand-in**: in `policy_step`, when the just-finished option was `A_RESEARCH`
     while legal, add a small `RESEARCH_BONUS` to the transition reward so the action stays
     in the learned repertoire (ε-exploration alone is too sparse). Tuned in headless to keep
     noots surviving/trading. *Marked a stand-in in INTENDED_FEATURES — the real payoff is the
     Phase-3 discovery/tech utility.*
4. **Stats** (`EconStats`): `research_demand: [f64; N_ITEMS]`, `research_total: f64`,
   `research_rate: f32`, all `#[serde(default)]`; surfaced in the headless JSONL.
5. **Save migration** (`save.rs`): `SAVE_VERSION 2 → 3`. `migrate_step(2,..)` grows a present,
   old-shaped policy: append `H` zeros to `policy.wa`, one `0.0` to `policy.ba` (new action
   starts neutral, trained brain preserved). New `EconStats` fields ride `#[serde(default)]`.
6. **Wire format** (`leaderboard.rs`): `Summary` gains `research: Vec<ResearchDemand>` where
   `ResearchDemand { element, refined, form, demand }` — self-describing per global
   (element, form), populated from `stats.research_demand` × `world.chosen`. Consumed in
   Phase 2; populated now so data starts flowing on the next submit.

**Phase 1 verification**: `cargo check --target wasm32-unknown-unknown` + `cargo clippy`
clean; headless run shows `research_rate > 0`, `research_demand` accruing on the in-world
elements, and the economy otherwise healthy (pop stable, trades flowing); old save migrates
(policy grows, no reset).

## Phase 2 — server grows the tree + names it (next)

- **Aggregate**: `/submit` already receives the snapshot; fold `Summary.research` into a
  persisted global `demand: [f64; 50]` (decayed over time so recent demand dominates).
- **Grow**: on a timer (real wall-clock, e.g. every N minutes) with probability rising in
  total demand, mint a new `Tech`: pick 1–3 inputs sampled ∝ demand; **randomize**
  attributes — effect (extraction/refine efficiency mult, hunger value, positional weight,
  carry-cap, …), consumable|durable, positional|staple, tier, magnitude — via a server RNG.
- **Name**: build a prompt from the inputs + attributes; call OpenRouter
  (`OPENROUTER_API_KEY`, `OPENROUTER_MODEL`) with a tight JSON/one-word response; fall back to
  a procedural blend (`durable Oak + Iron → "Oaken Brace"`). Needs a minimal blocking HTTP
  client — add `ureq` (tiny, rustls) under the `server` feature only; never in the wasm build.
- **Persist & serve**: `tech_tree.json` next to `leaderboard.json`; expose `GET /tech` →
  `{ techs: [...] }` and embed the same in the `/submit` 200 response body so the client gets
  it on every report without a second request.
- **Tests**: growth/decay/sampling are pure functions — unit-test them; the namer has a
  no-network deterministic path.

## Phase 3 — client discovery, construction, tech effects (last)

- **Fetch**: read the `/submit` response (add web-sys `Response` + `wasm-bindgen-futures`;
  the POST stops being fire-and-forget) and cache the tech tree in a resource; optional
  `GET /tech` on boot.
- **Tech-item model**: extend inventory with a dynamic `tech: Vec<f32>` (or
  `SmallVec`)/`HashMap<TechId,f32>` *alongside* the fixed `[f32; N_ITEMS]` core, so trade /
  pricing / WTP for the 8 base goods stay untouched; tech items get their own
  utility/trade rules from their attributes.
- **Discover**: `Action::Research` while holding a tech's full prerequisite set flips a
  per-world `discovered: HashSet<TechId>` (in `World`, serialized).
- **Construct**: new `StructureKind::Workshop` + `A_BUILD_WORKSHOP` / a construct step that
  consumes the recipe inputs and outputs +1 tech item; tech attributes feed
  `maslow_utility` (staple→hunger, positional→esteem, durable→persists) and any
  efficiency effects apply to the owner/world.
- **Surface**: HUD chip + leaderboard column for discovered/constructed techs.
- Save migration for the new `World.discovered`, the tech inventory, the new action/structure
  (grow actor head again; `#[serde(default)]` the rest).

## Risks / notes

- Action-count migration is the recurring cost (Phases 1 and 3 each add actions → grow `wa`).
  The trailing-block layout makes it a safe append.
- Global-live tech means replay isn't bit-identical across server epochs; acceptable per the
  decision above. Worldgen stays seed-pure.
- LLM is strictly best-effort and off the hot path (server-side, on tech birth only) — a key
  outage degrades to procedural names, never blocks growth.
- Keep `ureq`/LLM strictly under the `server` feature; the phone bundle must not grow.
