# Plan 001 — Noot economy: extraction, trade, and consumption

> On execution (after approval), first copy this file to `plans/001-noot-economy.md`
> in the repo, since plan mode only allows editing this scratch plan file.

## Context

The repo currently has a runnable hex-world generator (`src/world.rs`,
`src/hex.rs`, `src/elements.rs`, `src/rng.rs`, `src/main.rs`): a random
pointy-top hex map with easy/difficult terrain, four elements drawn from a
25-element pool (two replenishable, two finite), and a labor-free resource sim
that auto-accumulates a stockpile. It renders on a Bevy 0.18 canvas (mobile-first:
touch pan/pinch, on-screen invest buttons, HUD) and deploys to GitHub Pages.

This plan adds the **economic agent layer**: autonomous agents (**noots**) that
do the work of extracting goods, walk the map, meet, and **trade for a currency
(bucks, symbol `₦`)**. Goods are the *first-level refined products* of the four
world elements; each is a consumable. The goal of this increment is a visible,
self-running micro-economy: noots extracting, refining, walking, meeting,
trading, and consuming — with prices and wealth emerging on screen.

## Decisions (from clarifying questions + reconciliation)

- **Scope: runnable core first.** Build extraction-by-labor, owners, the
  refiner role, random-walk + simple RL + return-home, meet-and-trade for bucks,
  hunger/positional consumption, and an economy HUD. **Defer the transporter
  principal-agent layer** (hire / keep-a-cut / carry-bucks-back-to-owner) to a
  follow-up (Plan 002).
- **Refining: dedicated refiner role.** A *refiner* noot buys raw element units,
  converts them into the element's hardcoded refined good (fixed recipe + short
  work delay), and resells. This is required even in the core because two of the
  four goods are refined and noots must be able to consume them — so it forms a
  2-step supply chain (owner → refiner → consumer). *(This is the reconciliation:
  the "runnable core" defers transporters, but the refiner stays in because
  refined goods can't exist without it.)*
- **Good roles: independent random per world.** The four roles
  (staple-unrefined, staple-refined, positional-unrefined, positional-refined)
  are assigned randomly to the four chosen elements, independent of the
  replenishable/finite split.

## Naming

- Agents → **noots**. Currency → **bucks**, symbol **`₦`** (U+20A6, "N" with two
  strokes). Use `₦` in all HUD/price text.

## Data model

### Goods (`src/goods.rs`, new)
- Hardcode a **refined product name for every one of the 25 elements** (so any
  drawn element has a refined form on hand; LLM tech tree slots in here later):
  Lightning→Battery, Fire→Ember, Slime→Gel, Acid→Reagent, Water→Tonic,
  Sugar→Candy, Wood→Plank, Ice→Coolant, Stone→Brick, Sand→Glass, Oil→Fuel,
  Gold→Ingot, Iron→Steel, Salt→Cure, Steam→Turbine, Ash→Lye, Crystal→Lens,
  Mud→Clay, Smoke→Incense, Wind→Sail, Light→Beacon, Shadow→Veil, Copper→Wire,
  Sulfur→Match, Honey→Mead. (Add `refined: &'static str` to `ElementDef` in
  `elements.rs`.)
- `enum GoodForm { Raw, Refined }`, `enum GoodCategory { Staple, Positional }`.
- A **good** is identified by `(element_slot: usize, GoodForm)`. The four world
  goods are: each chosen element's consumable, where the form (Raw=unrefined /
  Refined) and category (Staple/Positional) come from the per-world role
  assignment (exactly one of each of the 4 combinations).
- `WorldGoods { roles: [(element_slot, GoodForm, GoodCategory); 4] }` built in
  worldgen via `rng.shuffle` over the four role tuples — store on the `Sim`/`World`.

### Resource sim refactor (`src/world.rs`)
Extraction becomes **labor-gated** (today it auto-accumulates):
- Add `stock: f64` (currently-available units) to every `Deposit`; replenishables
  also get `capacity: f64`.
- `World::tick(dt)` now only **regrows replenishable** `stock` logistically toward
  `capacity` at `rate * efficiency * terrain_factor`. Finite deposits do not regrow.
- Remove the auto-accumulating `ChosenElement.stockpile`; keep `efficiency`
  (now multiplies extraction) and add `extracted_total: f64` as a HUD stat.
- Extraction itself lives in the agent layer (below), pulling from `deposit.stock`
  (finite extraction scaled by `remaining/initial` for diminishing returns).

### Noots (`src/noot.rs`, new — Bevy components)
Each noot is an entity with:
- `Transform` (pixel pos) + `TilePos { col, row }` + `Home { col, row }`.
- `enum Role { Owner { deposit: usize }, Refiner, Consumer }`.
- `Inventory` — `HashMap<GoodKey, f64>` (small) + `bucks: f64` (start ~100 `₦`).
- `Hunger { staple: [f32; 2] }` — rises over time; consuming the matching staple
  good refills toward a **satiation cap** (satisficing: no utility past the cap).
- `Positional { stock: [f32; 2] }` — consuming a positional good raises stock;
  marginal utility `= k / (1 + stock)` (logarithmic total). Only consumed when
  **both staples are satisfied**.
- `RlMemory { heading: u8 (0..6), success_weight: [f32; 6], trip_step: u32,
  outbound: bool, sold_this_trip: bool }` for the walk/learning loop.

### Spawning (in `setup`)
- Seed a sim RNG resource (`Rng` from `world.seed`).
- Spawn ~40 noots. **Force owners onto deposits**: one Owner noot per deposit
  (home = deposit tile). Spawn a handful of Refiners and the rest Consumers at
  random tiles. (Guarantees the supply chain can run.)

## Systems (Bevy `Update`)

1. **`simulate`** (existing, refactored) — `world.tick(dt)` regrows replenishables.
2. **`hunger_tick`** — raise staple hunger over time.
3. **`movement`** — tile-to-tile random walk. Outbound: step is biased toward
   `RlMemory.heading` (~70% toward heading's neighbor, 30% random neighbor) for
   `L` steps; difficult terrain costs more time per step (ties terrain to the
   economy). Then `outbound=false` and the noot greedily steps back toward
   `Home`. On arrival home: run RL update (reinforce `heading` if
   `sold_this_trip`), then with prob `ε` pick a random new heading (explore),
   else keep best; reset trip. Pixel pos lerps toward target tile center.
4. **`extract`** — an Owner standing on its deposit tile pulls
   `min(work_rate * eff * terrain, stock)` (finite: × `remaining/initial`) from
   `deposit.stock` into inventory as the **raw** good; updates `extracted_total`.
5. **`refine`** — a Refiner holding raw units of a refinable good converts them
   (fixed ratio, short work delay) into the refined good.
6. **`meet_and_trade`** — bin noots by tile (spatial hash `HashMap<(i32,i32),
   Vec<Entity>>`); for co-located pairs, run a trade:
   - Buyer's **willingness-to-pay (WTP)** = marginal utility in bucks (staple:
     high while hungry, ~0 when satiated; positional: `k/(1+stock)`).
   - Seller's **ask** = reservation (owner: base extraction cost; refiner: input
     cost + margin).
   - Trade 1 unit if `WTP ≥ ask` and buyer has bucks; settle at `ask` (simple),
     move bucks buyer→seller, move the good seller→buyer; mark seller
     `sold_this_trip=true`. Record price for HUD.
7. **`consume`** — noots consume held goods they value: staples first (refill
   hunger up to cap), then positional goods (raise positional stock) only if
   staples satisfied. Accumulate a per-noot utility stat.
8. **`render_noots`** — sync `Transform`; color by role (Owner=gold,
   Refiner=blue, Consumer=green). Spawn small circle meshes once.
9. **`update_hud`** (extend existing) — economy panel: bucks in circulation,
   per-good last/avg trade price in `₦`, trades/sec, avg staple hunger, # by role,
   each deposit's standing `stock` / `remaining`. Keep invest buttons (now boost
   extraction efficiency).

## File changes (summary)
- New: `src/goods.rs`, `src/noot.rs`, `src/economy.rs` (trade/consume/extract/refine),
  `src/movement.rs`.
- Modified: `src/world.rs` (labor-gated stock refactor, store `WorldGoods`),
  `src/elements.rs` (`refined` name per element), `src/main.rs` (spawn noots,
  register systems, economy HUD, `₦`), maybe `assets/fonts/` (see risks).

## Determinism
Reuse the seeded `Rng` (`src/rng.rs`) for good-role assignment, noot spawns,
walks, and ε-exploration. A single fixed `SEED` keeps a build reproducible.

## Out of scope (→ Plan 002)
- **Transporters**: hire a non-owner to carry an owner's goods, sell elsewhere,
  keep a cut, carry the owner's share back home.
- Refinery *buildings*/sites (core uses the refiner *role* only), richer markets,
  per-noot inspection UI, LLM-generated tech tree.

## Risks / watch-items
- **`₦` glyph**: Bevy's default font (FiraMono) may not include U+20A6 → renders
  as tofu. Mitigation: bundle a font with the glyph (e.g. DejaVu Sans Mono) under
  `assets/fonts/` and load via `AssetServer` for HUD text; fall back to `N` if we
  want zero new assets. Decide during implementation; FiraMono is monospace, which
  the column-aligned HUD already relies on.
- **Performance**: ~40 noots + ~660 tiles is fine; the per-frame spatial hash is
  cheap. Keep trades to 1 unit/meeting to avoid loops.
- **Liquidity**: noots need a starting bucks endowment or no one can buy — give
  each ~100 `₦`.
- **No browser here**: I can verify compilation/codegen (wasm) but cannot click
  through the running app in this environment; final behavior needs a look on
  device after deploy.

## Verification
1. `cargo check --target wasm32-unknown-unknown` then
   `cargo build --target wasm32-unknown-unknown` — must be clean.
2. Add lightweight `#[cfg(test)]` unit tests for pure logic where it doesn't drag
   in Bevy (good-role assignment shuffle, WTP/ask pricing decision, satisficing
   vs log marginal utility, finite diminishing extraction). Run `cargo test` if
   the native Bevy build works in-env; otherwise rely on the wasm check.
3. Manual/on-device after merge to `main` (Pages deploy): noots move; deposit
   `stock`/`remaining` change; trade prices and bucks balances move; staple hunger
   stabilizes while positional stocks keep climbing slowly.
4. Per the side-project workflow: commit on
   `claude/tech-tree-elements-supply-kq4N1`, fast-forward `main`, push (no PR).
