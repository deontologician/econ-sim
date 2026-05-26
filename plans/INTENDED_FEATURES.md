# Intended Features — master ledger

Living document. Whenever something is **faked / stubbed / simplified** to keep
moving, add an entry here describing the real, principled version we intend to
build. Each entry: what we shipped now, why it's a stand-in, and the intended
design. Newest first within each section.

## Legend
- **NOW**: what currently exists in the code.
- **INTENDED**: the principled version we want.
- **STATUS**: `stub` (placeholder), `partial` (works but simplified), `deferred`
  (not built yet), `done` (intended version shipped — keep for history).

---

## Persistence

### Save / load / migrations
- **NOW**: the **full ECS state** is serialized to `localStorage` (serde → JSON,
  `save.rs`): the world (tiles/deposits/stocks/chosen/goods), the hunger & income
  controllers, `EconStats`, and every noot (position, inventory, wallet, hunger,
  claim, trader, meta, and its learned value field). **S**/Save snapshots; boot
  resumes it; **G**/New clears it. Each save carries a `version`; load parses to a
  `serde_json::Value` and **replays every migration step** (`migrate_step`) from the
  file's version up to `SAVE_VERSION` before deserializing, so old saves upgrade in
  place. A parse failure / newer-than-this-build version falls back to fresh. *(partial)*
- **INTENDED**: autosave + multiple named slots; export/import. (The migration
  framework exists; it just has no steps yet — v1 is the first schema.)
- **STATUS**: partial

## Economy

### Food storage & productivity growth
- **NOW**: noots keep a fixed `FOOD_RESERVE` (units they won't sell and top up toward
  when cheap) so a surplus buffer can build, and a slow `skill_factor` raises mining/
  refining throughput with per-individual `experience` (capped 2×). Both are flat
  constants / a simple capped-linear curve. *(partial)*
- **INTENDED**: a reserve sized to *expected* hunger volatility and price (not a magic
  constant); spoilage/storage cost so hoarding has limits; per-task skills with proper
  diminishing-returns learning curves, maybe transferable or teachable.
- **STATUS**: partial

### Consumer income / wages
- **NOW**: every noot gets a universal bucks trickle so consumers don't go broke and
  demand keeps circulating. The rate is **controlled** (`IncomeControl`): an integral
  loop trims it so total sale value grows at a target ~0.1%/min ("inflation" = this
  minute's summed sales vs. last minute's). A reborn noot restarts with the starting
  wallet (a separate, uncontrolled money injection). *(stub)*
- **INTENDED**: no free money. Noots earn bucks only by producing/selling, doing
  paid work (transport, refining, hauling), or a real labor market. Bucks should
  be conserved except where minted by a defined mechanism. Retiring the trickle
  needs a consumer-side way to earn (e.g. labor) so demand doesn't collapse.
- **STATUS**: stub

### Extraction efficiency
- **NOW**: each element carries a single `efficiency` multiplier (fixed at 1.0)
  that scales replenishable regrowth and finite extraction. The old player
  "invest" buttons that bumped it have been removed as a dead end. *(stub)*
- **INTENDED**: efficiency is a property of the **extractor**, not a global or
  per-resource setting — owners improve their own extraction over time (learning,
  capital, tooling), so productivity varies between extractors and emerges from
  play rather than a slider.
- **STATUS**: stub

### Pricing / market clearing
- **NOW**: a trade settles at the **seller's ask** when the buyer's WTP ≥ ask and
  they're solvent; 1 unit per meeting. The ask is now **inventory-aware**: a
  non-merchant seller discounts a base price by `surplus_discount(held)` (Plan 005),
  so the more glutted it is the cheaper it dumps — this is the spatial spread
  merchants arbitrage. Merchants ask full base price. *(partial)*
- **INTENDED**: bilateral bargaining or a local market that clears on
  supply/demand; prices emerge from scarcity, transport cost, and competition;
  variable quantities. Base prices are still per-good constants under the discount.
- **STATUS**: partial

### Trade / arbitrage (no merchant role)
- **NOW**: there is no transporter/merchant role — arbitrage is a universal noot
  behavior (`Trader`, Plans 005→007). A fed noot with spare cash buys surplus at
  `discount × base_ask` and resells at `base_ask` for the spread (resale floored at
  cost basis, so margins are non-negative). The `discount` is learned: up on a
  profitable sale, down on a buy, so noots self-sort into active arbitrageurs vs
  cautious subsisters. The old principal–agent `HaulContract` is gone. *(partial)*
- **INTENDED**: richer price-setting than a fixed base + learned discount (real
  bid/ask, negotiation), inventory/working-capital limits, competition and
  congestion effects, and arbitrage across genuinely spatial prices rather than the
  surplus-discount proxy.
- **STATUS**: partial

### Refining
- **NOW**: a *refiner role* converts a raw intermediate into the element's
  hardcoded refined good at a fixed 1:1 ratio after a short work delay. *(partial)*
- **INTENDED**: refineries as capital/sites, recipes with real input ratios and
  byproducts, multi-step chains, and an LLM-generated tech tree feeding the
  refined-product definitions (currently hardcoded in `elements.rs`).
- **STATUS**: partial

### Ownership
- **NOW**: emergent claims (Plan 007). Deposits start unowned; a claimless noot
  standing on an unclaimed deposit claims it (sticky — keeps its first), and only
  the claimant may mine it. A claim frees when its holder dies. One noot is seeded
  per deposit at spawn so mining starts immediately. *(partial)*
- **INTENDED**: buying/selling deposits, contested ownership, inheritance of claims,
  and richer claim lifecycles (abandonment, multiple holdings, defense).
- **STATUS**: partial

## Agents (noots)

### Action selection (learned actor-critic)
- **NOW**: a **shared off-policy actor-critic** (`policy.rs`, `economy::policy_step`)
  picks each noot's action from a masked softmax over its state (absolute position
  embedding + bucks/hunger/positional/ownership). Actions are **relative**: 6 hex move
  directions (the actor learns a directional policy directly, rather than navigating
  by the critic's absolute value surface) plus Mine and Refine. **Trading is
  automatic** — any nearby pair clears a mutually-beneficial trade, gated by each
  noot's learned `discount`/reservation thresholds (no explicit Trade action). Reward
  = Δ of a Maslow-tiered utility (physiological ≫ safety ≫ esteem). Trained A2C-style
  (several minibatches/frame) from a shared replay buffer with a slow target critic;
  one brain persists across deaths and saves. *(partial)*
- **INTENDED**: faster learning — likely **relative/local state** too (position-
  invariant cues like direction-to-nearest-deposit) so one rule generalizes across
  the map instead of a per-tile flow field; reward/entropy/lr tuning; validation that
  it beats the old heuristic on-device.
- **STATUS**: partial

### Welfare / utility
- **NOW**: a noot's utility = "not starving" (per staple, `(satiation−appetite)/
  satiation`) + diminishing positional welfare (`Σ ln(1+held)`) from the durable
  positional goods it *holds* (no longer consumed). Staple welfare is realized at
  consumption and reported economy-wide as a utility/sec rate; positional welfare
  is a stock shown per-noot in the selection panel. *(partial)*
- **INTENDED**: utility should drive more decisions — willingness-to-pay,
  labor-supply and role choices, savings — not just the movement reward and a
  readout. Tune the staple/positional weighting and add more want categories.
- **STATUS**: partial

### Durable goods & voluntary selling
- **NOW**: positional (luxury) goods are **durable** — held in inventory as wealth,
  never auto-consumed. A noot buys them only once its staples are satisfied, and
  sells from its holdings when its reservation price — the marginal worth of
  keeping one (`POSITIONAL_VALUE/(1+held)`), discounted by hunger via
  `POSITIONAL_SELL_URGENCY` — falls below the buyer-facing ask. So the
  hungry-and-goods-rich liquidate wealth for food money while the fed shed only
  surplus: a self-balancing loop. Staples stay perishable food. *(partial)*
- **INTENDED**: a real asset / secondhand market — durable goods depreciate, span
  more types (tools, housing, capital), price inventory-aware, and feed production
  (capital) rather than only welfare.
- **STATUS**: partial

### Mortality (starvation death)
- **NOW**: a noot that sits fully starving (all staples maxed) for
  `DEATH_GRACE_SECS` dies and is immediately reborn as a fresh agent of the same
  role — owners back on their deposit, everyone else at a random tile — with a
  full wallet, empty inventory and half hunger, so the population stays fixed.
  The global hunger rate is no longer fixed: a **PID loop** (`HungerControl`, Plan
  006) trims it so the realized death rate tracks 2%/min of the mortal population.
  *(partial)*
- **INTENDED**: real demographics — births/deaths move the population, claims and
  wealth pass on or revert (inheritance, abandoned deposits), causes of death
  beyond starvation, and an economy that adapts to a changing headcount instead of
  a fixed roster.
- **STATUS**: partial

### Movement learning (RL)
- **NOW**: folded into the shared actor-critic (see *Action selection*). The old
  per-hex TD(λ) field (`RouteMemory`) is gone; movement is the **critic's value over
  absolute position** — when the policy picks Move it steps to the best-valued
  neighbour hex (ε-greedy via a per-noot exploration ratio). One shared net learns
  both *what to do* and *where to go*. *(partial)*
- **INTENDED**: richer *state* than position alone (local memory of observed
  prices/inventories, who's nearby); confirm the embedding learns sharp enough
  spatial gradients to navigate as well as the old per-hex table did.
- **STATUS**: partial

## World

### Seeding / variety
- **NOW**: the seed is drawn from entropy each load (`js_sys` time+random on web,
  `SystemTime` natively), so every visit is a different world. The run stays
  reproducible from the drawn seed, which the HUD prints. *(partial)*
- **INTENDED**: a UI seed field to enter/replay a specific seed; shareable seed
  links.
- **STATUS**: partial

### Renewable vs finite resources
- **NOW**: each world draws two **replenishable** elements (deposits regrow
  logistically toward a capacity at a per-deposit rate) and two **finite** ones
  (a large fixed stock extracted with diminishing returns as it depletes, never
  regrowing). Roles are assigned per playthrough; replenishables favour
  low-difficulty ground, finite stocks hide in the rugged, cliff-strewn terrain. *(partial)*
- **INTENDED**: richer resource dynamics — discovery/prospecting of new deposits,
  depletion driving migration and price shocks, regrowth affected by use and
  environment, and finite stocks creating long-run scarcity the economy must
  adapt to.
- **STATUS**: partial

## Rendering / UI

### Currency glyph `₦`
- **NOW**: render the `₦` (U+20A6) glyph; if the bundled/default font lacks it we
  fall back to `N`. *(partial — confirm on device)*
- **INTENDED**: guarantee the glyph via a bundled font subset; a proper styled
  currency mark.
- **STATUS**: partial
