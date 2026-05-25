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

## Economy

### Consumer income / wages
- **NOW**: every noot gets a flat universal bucks trickle (`BUCKS_INCOME`/sec) so
  consumers don't go broke and demand keeps circulating. Death is a partial **money
  sink** — a noot's bucks are cleared (not refilled) on rebirth, so its hoard leaves
  circulation. Still net-inflationary while the trickle is on. *(stub)*
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
- **NOW**: per-noot **per-hex value learning** (`RouteMemory`, Plan 004). Each noot
  trains a TD(λ) value estimate over the whole map (α=0.1, γ=0.9, λ=0.8) and moves
  ε-greedily up the gradient toward where it has earned reward, where ε is an
  **intrinsic per-noot explore/exploit ratio** drawn at birth (re-drawn on rebirth).
  Reward = staple
  welfare from eating **+** scaled selling income, banked per-tile and folded in on
  each step; the eligibility trace credits whole routes, not just the reward tile.
  Every noot trains the same field (Plan 007): a claim-holder homes to its deposit
  to refill then value-walks to sell; a claimless noot value-walks to find food,
  buyers, and surplus to flip. *(partial)*
- **INTENDED**: richer *state* than position alone (local memory of observed
  prices/inventories, who's nearby), function approximation instead of a full
  per-hex table, and merchant transporters that learn arbitrage routes (pass 2).
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
