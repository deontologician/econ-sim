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

### Roads (`World::road`, a decaying field)
- A per-tile `road ∈ [0,1]`, serialized with the world (`#[serde(default)]`, lazily resized).
- `accumulate_traffic` (right after `policy_step`): decays the whole field each tick
  (`ROAD_DECAY = 0.999`, ~690-tick half-life) and deposits `ROAD_DEPOSIT = 0.025` on every
  tile a noot steps into (capped at 1). The small deposit + slow decay mean a road only
  forms from sustained, repeated travel (and then lingers) — not from one noot passing
  through. Trade-off: since noot traffic is thinly spread, hard-earned roads are also faint
  (busiest lanes peak ~0.2); making them few-but-bright needs a stronger `ROAD_PULL` to
  concentrate traffic, which risks detours/instability — deferred for on-device tuning.
- `hunger_tick`: a tile's road cuts the movement surcharge (terrain + carry) by up to
  `ROAD_RELIEF = 0.85` — the "dramatically cheaper on roads" payoff.
- `value_guided_step`: each step adds `ROAD_PULL · road − TERRAIN_PUSH · difficulty` to the
  neighbour score. `ROAD_PULL = 12` is below `ROUTE_OPTIMISM = 20`, so a noot never trades
  forward progress for a road but *does* pick the road among equally-progressing hexes —
  the tie-break that funnels parallel paths onto a shared lane (positive feedback → basins).

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
