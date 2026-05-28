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
  loop trims it so total sale value grows at a small per-window target ("inflation" =
  this window's summed sales vs. the previous window's, windows counted in **ticks**).
  A reborn noot restarts with the starting wallet (a separate, uncontrolled money
  injection). *(stub)*
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
- **NOW**: refining converts a raw intermediate into the element's hardcoded refined
  good at a fixed 1:1 ratio, and now happens **only inside a refinery** (a built site;
  see *Generic hex ownership*) — a noot must stand on one to refine (any refinery,
  shared). *(partial)*
- **INTENDED**: recipes with real input ratios and byproducts, multi-step chains, and
  an LLM-generated tech tree feeding the refined-product definitions (currently
  hardcoded in `elements.rs`).
- **STATUS**: partial

### Ownership
- **NOW**: emergent claims, now unified into **one improved hex per noot** (see
  *Generic hex ownership*): a claimless noot adopts the deposit it works or refinery it
  uses, building claims a hex, and the claim frees on death. Fresh worlds spawn everyone
  claimless (mining/roles are emergent). *(partial)*
- **INTENDED**: buying/selling deposits, contested ownership, inheritance of claims,
  voluntary role-switching, and richer claim lifecycles (upkeep, defense).
- **STATUS**: partial

## Agents (noots)

### Generic hex ownership: mines, shops, refineries
- **NOW**: a single unified claim — `Claim::hex` is the **one** improved tile a noot owns
  (at most one). Improvements are a **deposit** (a mine, claimed by working it), a
  **shop**, or a **refinery** (`World::structures`, built for 100 bucks via the
  `BuildShop`/`BuildRefinery` options on non-deposit ground; building over an *unclaimed*
  structure replaces its kind, so you can take an abandoned refinery and make it a shop).
  This forces specialization: a noot is a miner, shopkeeper, or refiner. Buildings are
  shared infrastructure — **refining only happens while standing in a refinery** (any),
  and Sell heads to the nearest shop (else the price-field market). A claimless noot
  adopts the deposit/refinery it works (`claim_improvements`); on death the hex abandons
  (deposit reopens, structure stands for another to take/rebuild). Persisted; shops draw
  as cyan diamonds, refineries orange. Headless (100k ticks): ~30 miners + a shared
  refinery + a shop emerge, the refined-goods chain routes through the refinery, economy
  stays healthy (production/consumption climb, deaths regulated), save/load round-trips.
- **INTENDED**: a real economic edge to owning a building (rent/fees/stocking); smarter
  build siting toward demand; upkeep/decay so abandoned structures don't accumulate;
  voluntary re-claim so a noot can switch roles without dying; contested ownership.
- **STATUS**: partial

### Action selection (learned actor-critic over committed options)
- **NOW**: a **shared off-policy actor-critic** (`policy.rs`, `economy::policy_step`)
  picks each noot's **committed option** (a semi-MDP macro-action) from a masked softmax
  over its state — not a primitive step. The six options are **Mine** (go to its own, or
  the nearest unclaimed, deposit and extract a load), **Sell** (haul to the nearest shop,
  else the best market, and linger to trade), **Refine** (go to a refinery and convert
  intermediates there), **Explore** (scout), and **BuildShop**/**BuildRefinery** (claim a
  hex, above). A
  deterministic executor drives the chosen option to completion (greedy hex navigation —
  no pathfinding needed on the impassable-free torus) and the policy only re-decides at
  option boundaries, where **one transition** is recorded (reward = the ΔU the option
  accrued). This is what makes the long produce→haul→sell chain learnable: the policy
  never has to crack navigation as a sparse-reward subtask, so directed behaviour
  replaces per-step dithering, and the Mine option's value bootstraps from the later
  Sell/eat reward — **no production-shaping bonus needed** (see below). The state mixes an
  absolute position embedding with position-invariant cues: bucks/hunger/positional/
  ownership, an *on-minable* flag, the local **terrain difficulty**, and six-way
  *direction gradients* toward the target deposit, the nearest other noot, and the best
  market for the carried cargo (these still inform option *value*; the executor navigates
  by them). **Trading is automatic** — any nearby pair clears a mutually-beneficial trade,
  gated by each noot's learned `discount`/reservation thresholds (no explicit Trade
  action). Reward = Δ of a Maslow-tiered utility (physiological ≫ safety ≫ esteem, with a
  softened tier gate). Trained A2C-style (several minibatches/frame) from a shared replay
  buffer with a slow target critic; one brain persists across deaths and saves. Headless
  (60k ticks): structured mine/haul/sell/refine behaviour, consumption ~3.1k, ~86–94% of
  trades pooling into the busiest 5% of hexes (emergent marketplaces), deaths regulated.
  *(partial)*
- **INTENDED**: the option set is approximate (one-step TD on variable-length options,
  ignoring option length in the discount); richer relative/local state (observed
  prices/inventories, who's nearby); reward/entropy/lr tuning; on-device validation.
- **STATUS**: partial

### Production reward-shaping & guided exploration *(retired)*
- **NOW**: an earlier stop-gap — a small intrinsic reward per unit mined/refined plus an
  exploration bias toward producing — bootstrapped mining under the old flat per-step
  action space, where bare ΔU left the policy wandering (≈0 production, mass starvation).
  **Both have been removed**: the committed-options action space above makes the chain
  learnable from bare ΔU, and rollouts are if anything slightly *better* without the
  bonus (production ~6.4k→~6.7k, consumption ~3.1k→~3.4k), so the crutch was dead weight.
- **INTENDED**: nothing — the principled fix (committed options) replaced it.
- **STATUS**: done

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
  `DEATH_GRACE_SECS` (simulated seconds) dies and is immediately reborn fresh at a
  **random tile** (claimless, full wallet, empty inventory, half hunger, new explore
  temperament), so the population stays fixed. The global hunger rate is no longer
  fixed: an **integral controller** (`HungerControl`, Plan 006) trims it so the
  realized death rate tracks a target (~4%/min of the population). The loop is now
  denominated **per tick** and kept gentle — a high gain wound the integrator up during
  death-free spells and dumped it as a hunger spike that starved the whole population at
  once (synchronized death waves); a long measurement window + low gain removed them.
  *(partial)*
- **INTENDED**: real demographics — births/deaths move the population, claims and
  wealth pass on or revert (inheritance, abandoned deposits), causes of death
  beyond starvation, and an economy that adapts to a changing headcount instead of
  a fixed roster.
- **STATUS**: partial

### Noot names & reincarnation
- **NOW**: every noot carries a persistent `NootName` — a randomly generated first +
  last name (short first like "Tim"/"Lux"; fantasy surname assembled from syllables like
  "Dorphindel"/"Korgard") assigned at spawn. The name **survives death**: on rebirth only
  the `incarnation` counter advances, so the followed-noot panel reads "Lux Korgard the
  5th". Persisted in saves (`#[serde(default)]` so pre-names saves load and get fresh
  names). Verified headless: incarnations climb (seen 1→6 over 60k ticks), save/load
  round-trips names + counts. *(partial — panel render unconfirmed on device)*
- **INTENDED**: surface names elsewhere (map labels, trade/death log); family/lineage
  names on inheritance; richer/seeded name pools.
- **STATUS**: partial

### Movement learning (RL)
- **NOW**: folded into the shared actor-critic (see *Action selection*). The old
  per-hex TD(λ) field (`RouteMemory`) is gone; the actor **directly emits one of the six
  relative hex directions** as its move action (ε-greedy via a per-noot exploration
  ratio), informed by the position embedding plus the deposit-direction gradient cue.
  One shared net learns both *what to do* and *where to go*. *(partial)*
- **INTENDED**: richer *state* (local memory of observed prices/inventories, who's
  nearby); confirm navigation matches or beats the old per-hex table on-device.
- **STATUS**: partial

## World

### Seeding / variety
- **NOW**: the seed is drawn from entropy each load (`js_sys` time+random on web,
  `SystemTime` natively), so every visit is a different world. The run stays
  reproducible from the drawn seed, which the HUD prints. *(partial)*
- **INTENDED**: a UI seed field to enter/replay a specific seed; shareable seed
  links.
- **STATUS**: partial

### Deposit abundance by consumption role
- **NOW**: deposits are no longer spread evenly across the four elements. Each slot's
  number of clusters is set by what its consumable good is (`consumption_rank` →
  `CLUSTERS_BY_CONSUMPTION_RANK = [4,3,2,1]`): the raw staple eaten directly is most
  common, then the staple that must be refined to eat, then the raw positional luxury,
  then the refined positional luxury (rarest). With `DEPOSITS_PER_CLUSTER = 3` that's
  `12/9/6/3 = 30` deposits, down from a flat `48` — leaner starting resources, with food
  abundant and luxuries scarce. Verified headless: 30 deposits, economy still regulates.
- **INTENDED**: tune the spread against population/economy balance; perhaps scale with
  map size; possibly vary per-deposit stock by role too.
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

## Simulation engine

### Fixed-tick clock & game speed
- **NOW**: the sim is **decoupled from real time**. Every system advances the world by
  exactly one `economy::TICK_DT` (fixed); a `SimSchedule` holds the whole sim and
  `run_sim_ticks` drives it many times per rendered frame, banking `ticks/sec · frame_dt`
  ticks (capped to avoid a death-spiral). A `SimSpeed` resource + an on-screen
  **transport bar** (`[<<] [Play/Pause] [>>]` with a `x.x ticks/s` readout; `,`/`.`
  keys too) step the tick rate, so the sim can run far faster than the wall clock while
  the render loop stays at the browser's frame rate. All controllers/stats are
  denominated **per tick** (rates, and the PID/income windows counted in ticks) so the
  numbers are identical at any speed. The headless harness runs the same schedule
  back-to-back and streams full econ stats as **JSONL** to stdout for offline tuning.
  *(partial)*
- **INTENDED**: a true fixed-timestep with render interpolation; a numeric speed/seed
  entry field; pause/step controls; maybe deterministic replay from the JSONL stream.
- **STATUS**: partial

## Rendering / UI

### Tap-to-inspect (noots and hexes)
- **NOW**: a tap picks the nearest noot under it (follows it, bottom panel shows its
  details), or — on a miss — **inspects the tapped hex** (`SelectedHex` + `describe_hex`):
  the panel shows terrain + work-speed, and whatever is on the tile — a deposit (element,
  stock, what it produces raw→refined and each form's role, and whether it's being
  mined), a shop/refinery (and owned vs abandoned), or open ground. A white ring
  (`HexHighlight`) marks the inspected tile. Selection and hex-inspection are mutually
  exclusive. *(partial — render unconfirmed on device)*
- **INTENDED**: tap a structure to also see/jump to its owner; show a deposit's current
  extraction rate / regrowth; richer per-tile economic readouts.
- **STATUS**: partial

### Noot stack indicator
- **NOW**: noots can share a hex and their sprites overlap, so a pooled set of
  world-space labels (`update_stack_labels`) shows the **count** on any hex holding ≥2
  noots — repositioned each refresh, unused ones parked hidden. *(partial — render
  unconfirmed on device; font size/placement may want tuning)*
- **INTENDED**: tune size/legibility; maybe fan out or spiral the stacked sprites too.
- **STATUS**: partial

### Wealth-distribution graph
- **NOW**: a **Wealth** toggle (button / `I`) opens a panel charting every noot's bucks
  sorted richest → poorest as a zero-based bar chart (`graph::render_bars`), with the
  **Gini coefficient** (`economy::gini`) in the caption. A steep convex drop / high Gini
  = lots of inequality. Headless emits `wealth_gini` per sample (verified: it climbs
  ~0 → ~0.72 over 60k ticks as the economy concentrates wealth).
- **INTENDED**: maybe a Lorenz-curve variant; track Gini over time as a strip series;
  break wealth down by claim status or region.
- **STATUS**: partial

### Per-resource price graphs
- **NOW**: `EconStats::last_sale_price[item]` holds each resource's most recent clearing
  price (set in `meet_and_trade`, persisted in saves). A **Prices** toggle (button / `P`)
  opens a panel with one auto-scaled sparkline per resource (labelled `<name> ₦<price>`),
  fed from a `PriceHistory` rollup sampled alongside the stats (whole-run, persisted — see
  *Whole-run graph history*). The series **holds the last
  sale price through no-trade spells** — a flat line, never a drop to zero — so a thinly
  traded good still reads its true price. Traces are **colour-coded by role** (green
  staple/food, tan intermediate, gold luxury) and staple labels carry a `*`, so the food
  prices stand out. *(partial — data verified headless; panel render unconfirmed on device)*
- **INTENDED**: optionally overlay bid/ask or the `ewma_price` band; mark the actual
  trade ticks; a shared y-scale toggle so levels (not just shapes) compare across goods.
- **STATUS**: partial

### Whole-run graph history (rolled up, persisted)
- **NOW**: the stat-strip and price graphs are backed by `history::RollupHistory<N>` (a
  core, gui-free type) instead of a fixed rolling ring. It retains the **entire run** at a
  bounded bucket count (`HISTORY_CAP`): recent samples stay full-resolution while older
  ones are progressively averaged into wider buckets (the oldest equal-resolution pair is
  merged on overflow, weighted by span — roughly power-of-two tiers). Persisted in the
  save (`Snapshot.stat_history`/`price_history`, `#[serde(default)]`), so reloading brings
  the graphs back populated with the whole history. Sparklines plot buckets index-spaced,
  so the rolled-up old end is time-compressed (overview). Verified: serde round-trip +
  cap/mean rollup tests; old pre-history saves load (empty series); headless writes empty
  history (it doesn't sample graphs). *(partial — panel render unconfirmed on device)*
- **INTENDED**: show per-bucket span/time on the axis; a "recent vs whole-run" zoom toggle;
  keep min/max envelopes (not just the mean) when rolling up.
- **STATUS**: partial

### Map overlays (cycled): terrain difficulty + trade density
- **NOW**: a single **Overlay** button (or `V`) cycles the map heatmap none → terrain →
  trades, mirroring the Noots colour cycler (the old separate Crowd/Terrain/Trades
  buttons and the crowd-density overlay were dropped — crowd wasn't useful). *Terrain*
  tints green→red by difficulty; *Trades* shows `EconStats::trade_hexes` (a per-hex tally
  of every cleared trade, counted at the seller's tile, persisted in saves) as a gold
  sqrt-ramped heatmap so emergent marketplaces stand out. Headless reports
  `trade_top5_share` (~0.86–0.94: commerce pools hard into the busiest 5% of hexes) and
  `trade_active_hexes`. *(partial — data verified headless; overlay render unconfirmed on device)*
- **INTENDED**: optionally a decaying/windowed trade variant to show *current* hot
  markets rather than all-time; per-good breakdown; a legend/scale on the overlay.
- **STATUS**: partial

### Currency glyph `₦`
- **NOW**: render the `₦` (U+20A6) glyph; if the bundled/default font lacks it we
  fall back to `N`. *(partial — confirm on device)*
- **INTENDED**: guarantee the glyph via a bundled font subset; a proper styled
  currency mark.
- **STATUS**: partial
