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
  consumers don't go broke and demand keeps circulating. Transporters are the
  first *earned* income (a share of haul revenue), but the universal trickle is
  still on. *(stub)*
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
- **NOW**: a trade settles at the **seller's fixed ask** (per-good constant) when
  the buyer's willingness-to-pay ≥ ask and they're solvent; 1 unit per meeting. *(partial)*
- **INTENDED**: bilateral bargaining or a local market that clears on
  supply/demand; prices emerge from scarcity, transport cost, and competition;
  variable quantities; inventory-aware asks.
- **STATUS**: partial

### Transporters (principal–agent hauling)
- **NOW**: a *transporter role* (`HaulContract` state machine). An idle hauler is
  auto-matched to the most-loaded owner (`haul_assign` — a stub for a real labor
  market), walks to the deposit, loads `HAUL_CAPACITY` raw units, wanders selling
  for a bounded number of steps, then returns and pays the owner `PRINCIPAL_SHARE`
  of the take (keeping the rest); unsold cargo is returned. While a hauler keeps
  draining an owner, the owner stays put extracting instead of touring. *(partial)*
- **INTENDED**: negotiated contracts (not auto-assignment), trust and default
  risk (a hauler could abscond with the goods/proceeds), competition for haulers,
  route/price choice, and pay flowing from a real labor market rather than a
  fixed share constant. (Plan 002.)
- **STATUS**: partial

### Refining
- **NOW**: a *refiner role* converts a raw intermediate into the element's
  hardcoded refined good at a fixed 1:1 ratio after a short work delay. *(partial)*
- **INTENDED**: refineries as capital/sites, recipes with real input ratios and
  byproducts, multi-step chains, and an LLM-generated tech tree feeding the
  refined-product definitions (currently hardcoded in `elements.rs`).
- **STATUS**: partial

### Ownership
- **NOW**: "start on a deposit ⇒ own it"; we deliberately seed one owner noot onto
  each deposit so extraction can happen. *(partial)*
- **INTENDED**: emergent land claims, buying/selling deposits, contested
  ownership, inheritance of claims.
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
  *(partial)*
- **INTENDED**: real demographics — births/deaths move the population, claims and
  wealth pass on or revert (inheritance, abandoned deposits), causes of death
  beyond starvation, and an economy that adapts to a changing headcount instead of
  a fixed roster.
- **STATUS**: partial

### Movement learning (RL)
- **NOW**: a per-noot heading bandit reinforced by the **welfare gained that
  trip** (consuming what was bought/sold-for), so buyers are rewarded for buying
  just as sellers were for selling; ε-greedy exploration, greedy return home.
  *(partial)*
- **INTENDED**: richer policy (state = local memory of where buyers/sellers/prices
  were), value estimates per region, proper exploration/exploitation, maybe
  learned routes. Consumers in particular should learn *where to buy*, not just a
  heading.
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
  regrowing). Roles are assigned per playthrough; replenishables favour easy
  terrain, finite stocks hide in difficult terrain. *(partial)*
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
