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

### Roads (`World::road_edges` = per-**edge** decaying wear, fed through a quadratic strength)
- A road is the **link between two cells**, not a cell. Wear is stored per edge
  (`World::road_edges`, flat `tile*6 + dir`; the two half-edges share one **canonical** slot
  via `economy::edge_id`), serialized with the world (`#[serde(default)]`, lazily resized).
  `accumulate_traffic` (right after `policy_step`) compares each noot's tile to last tick's
  (`PolicyMemory::last_tile`); an adjacent change is a step across that edge. It decays every
  edge each tick by a flat **linear** amount (`ROAD_DECAY = 0.00005` subtracted — full road
  persists ~20k ticks of disuse, clean survive-or-fade threshold) and deposits
  `ROAD_DEPOSIT = 0.05` on the crossed edge **only when > 2 distinct noots have crossed *that
  edge* within `ROAD_DISTINCT_WINDOW` (600) ticks** (per-edge recency cache
  `EconStats::road_seen`, transient). A lone noot shuttling its own route never qualifies, so
  only genuinely shared links wear in.
- **Effective strength is quadratic in wear** (`road_strength`): `((wear − LO)/(FULL − LO))²`
  with `LO = 0.04`, `FULL = 0.7`. Quadratic so the low-wear wash is squashed toward zero,
  and the span is *wide* so a road brightens gradually over a lot of sustained travel rather
  than snapping to full — a slow build, not a switch. Travel-cost relief, the movement pull,
  and the road rendering all read this.
- `hunger_tick`: the strength of the **edge just crossed** (cached on
  `PolicyMemory::last_edge_road`, since `hunger_tick` runs before the move) cuts the movement
  surcharge (terrain + carry) by up to `ROAD_RELIEF = 0.85` — the "dramatically cheaper on
  roads" payoff for *riding* a road, not standing on a cell.
- `value_guided_step`: adds `ROAD_PULL · strength(edge) − TERRAIN_PUSH · difficulty`, where
  the road term is the **edge** to that neighbour. `ROAD_PULL = 26` sits **between**
  `ROUTE_OPTIMISM = 20` and `2×` it: above 20, so a noot steps *sideways onto* a full road,
  giving up a hex of straight progress because riding is cheaper (instead of trudging the
  bare ground beside it); below 40, so it won't step *backwards* for a road, which is what
  makes it leave once the road curves away — "merge on, ride, get off at the end." (Pull is
  local/adjacent only; attracting toward a *distant* road would need a cost-to-go field.)
- **Rendering**: roads are drawn **permanently** on the map (`update_roads`, no toggle) as
  thin line segments between hex centres (`RoadEdge`, one shared mesh, per-edge transform),
  opacity ∝ strength, hidden below a small threshold; seam-wrapping edges aren't drawn. The
  old per-cell Roads *overlay* was removed; Terrain/Trades/Routes overlays remain.
- Result (100-150k, 3 seeds): a selective network of **~10-20 road segments (5-8 full)**,
  economy healthy (cons ~4.9-6.7k; 0xBEEF low ~1.6k as always), roads persist across saves.
  Trade-off: strong roads are winner-take-all and can pull trade toward one hub (top trade
  tile 50-94%) — the one-hub-vs-many-centres and strong-roads goals genuinely tension.

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
