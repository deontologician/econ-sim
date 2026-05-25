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
  consumers don't go broke and demand keeps circulating. *(stub)*
- **INTENDED**: no free money. Noots earn bucks only by producing/selling, doing
  paid work (transport, refining, hauling), or a real labor market. Bucks should
  be conserved except where minted by a defined mechanism.
- **STATUS**: stub

### Pricing / market clearing
- **NOW**: a trade settles at the **seller's fixed ask** (per-good constant) when
  the buyer's willingness-to-pay ≥ ask and they're solvent; 1 unit per meeting. *(partial)*
- **INTENDED**: bilateral bargaining or a local market that clears on
  supply/demand; prices emerge from scarcity, transport cost, and competition;
  variable quantities; inventory-aware asks.
- **STATUS**: partial

### Transporters (principal–agent hauling)
- **NOW**: not present. *(deferred)*
- **INTENDED**: an owner can hire a non-owner to carry goods elsewhere and sell;
  the transporter keeps a cut and returns the owner's share. Contracts, trust,
  default risk, route choice. (Plan 002.)
- **STATUS**: deferred

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

### Movement learning (RL)
- **NOW**: a simple per-noot heading bandit — reinforce the outbound heading if a
  sale happened, ε-greedy exploration, hardcoded straight-ish return home. *(partial)*
- **INTENDED**: richer policy (state = local memory of where buyers/sellers/prices
  were), value estimates per region, proper exploration/exploitation, maybe
  learned routes.
- **STATUS**: partial

## World

### Seeding / variety
- **NOW**: fixed `SEED` constant ⇒ every load is the same world. *(stub)*
- **INTENDED**: seed from entropy (or a UI seed field) for per-playthrough variety;
  shareable seeds.
- **STATUS**: stub

## Rendering / UI

### Currency glyph `₦`
- **NOW**: render the `₦` (U+20A6) glyph; if the bundled/default font lacks it we
  fall back to `N`. *(partial — confirm on device)*
- **INTENDED**: guarantee the glyph via a bundled font subset; a proper styled
  currency mark.
- **STATUS**: partial
