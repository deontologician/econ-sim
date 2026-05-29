# 030 — Roads (decaying desire-paths) + multi-centre trade

## Context

Two linked asks:
1. "The cost of travel isn't high enough — I want ~4-5 trade centres, right now they all
   bunch on one hex."
2. "Roads should form where noots travel, decay exponentially, dramatically cut travel cost
   on them, and form natural basins that attract other noots rather than the straight route."

Diagnosis of the bunching: there were already ~4 shops, but one captured ~82% of trades —
because the Sell option treks to the *nearest shop unconditionally*, so once one hub forms
everyone funnels to it. Travel was nearly free, so distance didn't gate it.

## What shipped

### Roads (`World::road` = decaying *wear*, fed through a quadratic strength)
- A per-tile raw **wear** ∈ `[0, ROAD_MAX]`, serialized with the world (`#[serde(default)]`,
  lazily resized). `accumulate_traffic` (right after `policy_step`) decays the whole field
  each tick by a flat **linear** amount (`ROAD_DECAY = 0.0002` subtracted, not a fraction —
  so full roads erode gently and persist for thousands of ticks, with a clean survive-or-fade
  threshold) and deposits `ROAD_DEPOSIT = 0.05`
  on a tile **only when > 2 distinct noots have crossed it within `ROAD_DISTINCT_WINDOW`
  (600) ticks** — tracked by a small per-tile recency cache (`EconStats::road_seen`,
  transient). A lone noot shuttling its own route never qualifies, so only genuinely shared
  corridors wear in; that gate (not a tiny deposit) is what makes roads selective.
- **Effective strength is quadratic in wear** (`road_strength`): `((wear − LO)/(FULL − LO))²`
  with `LO = 0.04`, `FULL = 0.22`. Slow at first then shooting up, so a qualifying corridor
  tips from nothing to a full-strength road, while any residual low-wear wash is squashed
  toward zero. Travel-cost relief, the movement pull, and the overlay all read this.
- `hunger_tick`: strength cuts the movement surcharge (terrain + carry) by up to
  `ROAD_RELIEF = 0.85` — the "dramatically cheaper on roads" payoff.
- `value_guided_step`: adds `ROAD_PULL · strength − TERRAIN_PUSH · difficulty`. `ROAD_PULL =
  12` < `ROUTE_OPTIMISM = 20`, so a noot never trades forward progress for a road but picks
  the road among equally-progressing hexes — funnelling parallel paths onto a shared lane.
- Result (80-150k, 3 seeds): roads form on only **1-5% of tiles** but those reach **full
  strength** — a few bright corridors, not a smear. Trade-off: bright roads are
  winner-take-all, so they pull trade back toward one hub (top trade tile 43-95%), partly
  undoing the `SHOP_RANGE` dispersal — the two goals (one-hub-vs-many-centres and strong
  roads) genuinely tension.

### Higher travel cost + multi-centre dispersal
- `CARRY_HUNGER_K` 0.6 → **1.6**: hauling a full load hurts much more. The hunger PID
  re-centres the absolute death rate, so this lands as a *relative* penalty on roaming
  loaded — and makes roads (which relieve it) worth forming.
- `MARKET_DIST_PENALTY` 3 → **8**: the market planner prefers nearer markets.
- New `SHOP_RANGE = 6`: a noot won't trek further than this to an existing shop; beyond it
  it sells at the best *local* market (clustering nearby, seeding a new regional centre and
  making a local shop worth building). This bounds each shop's catchment so several centres
  coexist instead of one hub.

### Roads overlay (GUI)
- New `Roads` map-overlay mode (tan), cycled none → terrain → trades → routes → roads (and
  `V`). Reads the live decaying `World::road` (distinct from `Routes`, the all-time
  cumulative `traffic_hexes`).

## Verification (headless, 80k, 3 seeds vs no-road baseline)

| seed | cons (base → now) | top trade tile (base → now) |
|------|-------------------|------------------------------|
| 0x0EC05EED | 4001 → 3939 | 82% → **57%** |
| 0xBEEF | ~1001 → **1437** | — → 67% |
| 0xC0FFEE | 2879 → **4308** | 82% → **44%** |

Trade de-concentrates (top tile 82% → 44-57%, 2-3 real centres), consumption holds or
improves, roads form (corridors to ~0.4, ~27% of tiles roaded). `cargo check`/`clippy`
clean on wasm + native; 10 tests pass.

**Unverified**: the on-device look (Bevy can't render here) — the roads overlay and the
basin shapes need eyeballing on the deployed build.

## Notes / knobs

- **The core tension**: roads are winner-take-all (everyone converges → one basin), which
  fights multiple centres. `SHOP_RANGE` is the dial: lower → more, smaller centres with
  softer/regional roads; higher → fewer centres with one strong highway. 6 is a middle.
- Got 2-3 strong centres, not a clean 4-5 — pushing higher means a smaller `SHOP_RANGE`,
  at the cost of road strength and some economic noise. Left at a robust middle to tune
  on-device.
- The economy is sensitive to the travel-cost knobs (low cost + roads collapsed shop
  formation in testing); `CARRY_HUNGER_K = 1.6` is what keeps it healthy *and* makes roads
  matter.
