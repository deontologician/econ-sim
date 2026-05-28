//! The economic loop: noots mine deposits they've claimed, refine, meet and trade
//! for bucks, and consume what they value. There are no fixed roles — every noot
//! can mine (a claimed deposit), refine, consume, and arbitrage surplus.
//!
//! A trade clears at the **seller's ask** when the buyer's willingness-to-pay
//! meets it, the seller values the item less than the ask (so it's surplus to
//! them), and the buyer is solvent. One unit per meeting. The ask discounts
//! surplus the seller is glutted on (but never below cost), opening the spread a
//! noot arbitrages.

use bevy::prelude::*;

use crate::goods::{self, form_of, GoodForm, ItemRole, N_ITEMS};
use crate::hex::{hex_center, neighbors, torus_distance};
use crate::noot::*;
use crate::world::{terrain_factor, DepositKind, World};
use crate::policy::{self, ActorCritic, PolicyMemory, Trainer, Transition, N_ACT, N_OTHER};
use crate::{Sim, SimRng};
use serde::{Deserialize, Serialize};

/// The simulation advances in fixed **ticks** of `TICK_DT` simulated seconds each,
/// decoupled from real time. Game speed = how many ticks the GUI runs per rendered
/// frame; the headless harness just runs ticks back-to-back. All continuous accrual
/// (hunger, income, regrowth, extraction, age, cooldowns) multiplies a per-second
/// rate by `TICK_DT`; all windows/periods are counted in ticks; all reported rates
/// are **per tick**, so numbers are identical at any speed.
pub const TICK_DT: f32 = 1.0 / 60.0;

/// Decision/step cadence (simulated seconds): a noot picks a new action (and, if
/// Move, steps one hex) this often, scaled faster on easy terrain.
pub const BASE_STEP_TIME: f32 = 0.35;
/// Maslow utility weights (physiological ≫ safety ≫ esteem) and tier normalizers.
const W_PHYS: f32 = 1.0;
const W_SAFE: f32 = 0.6;
const W_ESTEEM: f32 = 0.4;
const SAFETY_BUCKS: f32 = 60.0;
const ESTEEM_NORM: f32 = 4.0;
/// Reward penalty applied to the transition that ends in starvation death.
const DEATH_PENALTY: f32 = 2.0;
/// Minibatch updates per frame for the shared policy.
const TRAIN_ITERS_PER_FRAME: usize = 4;

// Production rates.
pub const WORK_RATE: f32 = 3.0;
pub const REFINE_RATE: f32 = 2.0;

// --- Committed options (semi-MDP macro-actions) -----------------------------
/// Carried units at which a Mine option is "loaded enough" and terminates so the policy
/// can decide to haul and sell. Below `CARRY_CAP`, so a noot tops up over several option
/// rounds if it keeps choosing Mine, but doesn't have to fill completely before selling.
const LOAD_THRESHOLD: f32 = 10.0;
/// Hard cap on how many executor steps a single committed option runs before it is
/// force-terminated and the policy re-decides — bounds a plan that can't reach its goal
/// (an unminable claimed-away deposit, a market with no buyers) so noots never get stuck.
const OPTION_MAX_TICKS: u32 = 48;
/// "GPS optimism": the artificial value bonus added, each step, to a neighbour that lies
/// on the greedy route to the committed target (×1 if it shortens the path, ×−1 if it
/// lengthens it). Large versus the critic's natural value scale, so a noot follows the
/// suggested route by default but can still deviate when an adjacent tile's *learned*
/// value beats the route by more than this — the GPS suggests, it doesn't command.
const ROUTE_OPTIMISM: f32 = 8.0;
/// Bucks to build a structure (shop or refinery). Both cost the same.
const SHOP_COST: f32 = 100.0;
const REFINERY_COST: f32 = 100.0;
/// Extra bucks a noot keeps in reserve before a Build option is offered, so building
/// can't bankrupt it out of food money.
const SHOP_BUILD_BUFFER: f32 = 50.0;

/// The deposit a noot owns (its claimed hex carries one), if any.
fn owned_deposit(world: &crate::world::World, claim: &Claim) -> Option<usize> {
    claim.hex.and_then(|t| world.tiles[t].deposit)
}

/// Tile index of the nearest structure of `kind` anywhere (shops/refineries are shared —
/// any noot may sell at a shop or refine at a refinery, regardless of who owns it).
fn nearest_structure(
    world: &crate::world::World,
    pos: &TilePos,
    kind: crate::world::StructureKind,
) -> Option<(i32, i32)> {
    world
        .structures
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| {
            let t = &world.tiles[s.tile];
            (t.col, t.row)
        })
        .min_by_key(|&(c, r)| torus_distance(pos.col, pos.row, c, r, world.cols, world.rows))
}

/// Experience gained per unit produced (mining + refining); experience is per
/// individual and resets on death.
const SKILL_PER_UNIT: f32 = 0.001;
/// Cap on the speed bonus: at full mastery a noot works `1 + this`× as fast.
const SKILL_BONUS_CAP: f32 = 1.0;

/// Slow learning-by-doing multiplier on mining/refining throughput: 1.0 for a
/// novice, saturating at `1 + SKILL_BONUS_CAP` after long experience.
pub fn skill_factor(experience: f32) -> f32 {
    1.0 + (experience * SKILL_PER_UNIT).min(SKILL_BONUS_CAP)
}

/// Units of each staple a noot keeps as a food reserve: it won't sell within this
/// buffer, and it stocks up toward it when food is cheap — so a lean spell doesn't
/// immediately starve it (escaping the Malthusian knife-edge).
const FOOD_RESERVE: f32 = 4.0;
/// What a noot will pay (as a fraction of `STAPLE_VALUE`) to top up its reserve
/// while it has spare appetite — enough to claim glutted surplus, below full price.
const FOOD_BUFFER_WTP_FRAC: f32 = 0.25;

// STUB: universal income so consumers don't go broke. See INTENDED_FEATURES.md.
pub const BUCKS_INCOME: f32 = 0.6;

// Consumption.
const EAT_VALUE: f32 = 4.0; // appetite removed per staple unit eaten

/// Smoothing for the per-item sale-price EWMA (higher = tracks recent trades faster).
const PRICE_EWMA_ALPHA: f32 = 0.12;

// Asking prices (bucks) by item kind.
const ASK_RAW_CONSUMABLE: f32 = 6.0;
const ASK_INTERMEDIATE: f32 = 5.0;
const ASK_REFINED_CONSUMABLE: f32 = 12.0;

// Valuations (bucks).
const REFINER_WTP_INTERMEDIATE: f32 = 8.0;
const STAPLE_VALUE: f32 = 20.0; // WTP when starving
const POSITIONAL_VALUE: f32 = 40.0; // first-unit WTP, then /(1+held)
/// How steeply hunger discounts a noot's reservation price for keeping a durable
/// positional good: at full starvation it parts with one for `1−this` of its
/// marginal worth, so the hungry liquidate wealth for food money.
const POSITIONAL_SELL_URGENCY: f32 = 0.9;

const TRADE_RADIUS_FACTOR: f32 = 1.7; // × hex_size

// --- Surplus discounting (the merchant arbitrage spread) --------------------
// These three are the main levers on whether merchants can profit: the discount
// at a producer's typical stock must drop below a merchant's `DISCOUNT_INIT`
// willingness-to-pay for any surplus to change hands. Tuned so a freshly-loaded
// owner (~5–6 units, since owners leave their deposit at `LOAD_THRESHOLD`) already
// offers below 0.5× base.
/// Holdings a seller can carry before its ask starts discounting.
const SURPLUS_FREE: f32 = 2.0;
/// Steepness of the discount per unit held beyond `SURPLUS_FREE`.
const SURPLUS_K: f32 = 0.35;
/// A glutted seller's ask never falls below this fraction of its base ask.
const SURPLUS_FLOOR: f32 = 0.35;
/// A noot only buys surplus to resell when fed and holding more bucks than this,
/// so speculation never starves it of food money.
const ARBITRAGE_RESERVE: f32 = 30.0;

// --- Spatial price gradient (the merchant's reason to travel) ---------------
// A good's *local* market ask varies across the map: cheap on top of a rich source
// (a glut), dear in a region with no nearby source (scarce). The spread is the markup
// a merchant captures by hauling goods from source to deficit — and the gradient the
// planner reads when valuing where to go sell. (`meet_and_trade` prices the seller's
// ask at the *seller's tile*, so a noot earns more selling the same good far from
// where it was mined.)
/// Local ask on top of a full source, as a fraction of the good's base ask (glut).
const PRICE_FLOOR: f32 = 0.5;
/// Local ask in a region with no nearby source, as a multiple of the base ask. Kept
/// modest so even refined staples stay under a starving buyer's WTP and still clear.
const PRICE_CEIL: f32 = 1.6;
/// Hex distance over which a deposit suppresses local prices; past a few of these the
/// good is effectively scarce. A deposit's pull is also weighted by how full it is, so
/// a depleted finite source stops glutting its surroundings and the region dears up.
const SUPPLY_RANGE: f32 = 6.0;
/// Rebuild the supply-driven price field this often (deposit tiles are fixed, but their
/// fullness drifts as they deplete/regrow, so a periodic rebuild tracks that cheaply).
const PRICE_FIELD_REBUILD_TICKS: u32 = 600;
/// Per-hex penalty (in buck-equivalent margin) when choosing which market to head for,
/// so a noot prefers a near market and only bothers hauling a load far when the expected
/// markup justifies the trek. This gates the market heading — a tiny load won't pull a
/// noot off its deposit; a full one will.
const MARKET_DIST_PENALTY: f32 = 3.0;

/// Ticks each production/consumption rate sample covers.
const RATE_WINDOW_TICKS: u32 = 30;

#[derive(Resource, Default, Clone, Serialize, Deserialize)]
pub struct EconStats {
    pub trades_total: u64,
    /// Total ticks elapsed (the sim clock).
    pub ticks: u64,
    /// Exponentially weighted moving average of actual sale prices, per item.
    pub ewma_price: [f32; N_ITEMS],
    /// The most recent actual clearing price, per item — held until the next sale of that
    /// item (so a no-trade spell shows a flat line at the last price, never a drop to 0).
    /// Drives the per-resource price graphs. 0 only before an item has ever traded.
    #[serde(default)]
    pub last_sale_price: [f32; N_ITEMS],
    /// Cumulative raw units extracted from deposits (the economy's supply).
    pub produced_total: f64,
    /// Cumulative units consumed (staples eaten + positional goods used up).
    pub consumed_total: f64,
    /// Cumulative bucks of margin realized reselling surplus.
    pub merchant_profit_total: f64,
    /// Cumulative welfare (utility) realized through consumption.
    pub utility_total: f64,
    /// Cumulative bucks of trade value cleared — the economy's nominal output (GDP).
    #[serde(default)]
    pub gdp_total: f64,
    /// Most recent windowed rates, **per tick**.
    pub production_rate: f32,
    pub consumption_rate: f32,
    pub merchant_profit_rate: f32,
    pub utility_rate: f32,
    /// Trade value cleared per tick (nominal GDP rate).
    #[serde(default)]
    pub gdp_rate: f32,
    /// Mean hex distance from source to point of sale over the last window — how far
    /// goods are being hauled (a trekking-merchant readout). 0 when nothing sold.
    #[serde(default)]
    pub mean_haul_dist: f32,
    /// Cumulative trades that cleared on each tile (`row * cols + col`) — a spatial
    /// heatmap of where commerce happens, for the Trades map overlay. Sized lazily to
    /// the map on first trade; persists across saves so the picture survives a reload.
    #[serde(default)]
    pub trade_hexes: Vec<u32>,
    // Accumulators for the in-progress rate window.
    produced_window: f32,
    consumed_window: f32,
    merchant_profit_window: f32,
    utility_window: f32,
    #[serde(default)]
    gdp_window: f32,
    #[serde(default)]
    haul_window: f64,
    #[serde(default)]
    haul_count_window: u32,
    window_ticks: u32,
}

/// Fold the running tallies into per-tick rates once per `RATE_WINDOW_TICKS`.
pub fn update_rates(mut stats: ResMut<EconStats>) {
    stats.ticks += 1;
    stats.window_ticks += 1;
    if stats.window_ticks >= RATE_WINDOW_TICKS {
        let inv = 1.0 / stats.window_ticks as f32;
        stats.production_rate = stats.produced_window * inv;
        stats.consumption_rate = stats.consumed_window * inv;
        stats.merchant_profit_rate = stats.merchant_profit_window * inv;
        stats.utility_rate = stats.utility_window * inv;
        stats.gdp_rate = stats.gdp_window * inv;
        stats.mean_haul_dist = if stats.haul_count_window > 0 {
            (stats.haul_window / stats.haul_count_window as f64) as f32
        } else {
            0.0
        };
        stats.produced_window = 0.0;
        stats.consumed_window = 0.0;
        stats.merchant_profit_window = 0.0;
        stats.utility_window = 0.0;
        stats.gdp_window = 0.0;
        stats.haul_window = 0.0;
        stats.haul_count_window = 0;
        stats.window_ticks = 0;
    }
}

pub fn income(ctrl: Res<IncomeControl>, mut wallets: Query<&mut Wallet>) {
    let d = ctrl.rate * TICK_DT;
    for mut w in &mut wallets {
        w.bucks += d;
    }
}

// --- Universal-income controller (targets a tiny sales "inflation") ---------
/// Target growth in total sale value, window over window.
pub const TARGET_INFLATION: f32 = 0.001;
/// Measurement/control window, in ticks.
const INCOME_WINDOW_TICKS: u32 = 1800;
/// Integral gain: `rate += INCOME_K * (target − measured)` each window.
const INCOME_K: f32 = 0.4;
/// EMA smoothing on the measured inflation (window-to-window sales are noisy).
const INCOME_MEAS_ALPHA: f32 = 0.5;
const INCOME_RATE_MIN: f32 = 0.0;
const INCOME_RATE_MAX: f32 = 3.0;

/// Trims the universal income so total trade value grows at roughly
/// `TARGET_INFLATION` per window. Inflation = this window's summed sale value vs.
/// the previous window's. `meet_and_trade` accumulates `this_window`.
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct IncomeControl {
    /// Current universal income (bucks/sec/noot) — `income` pays out `rate·TICK_DT`.
    pub rate: f32,
    /// Smoothed measured inflation (fractional sales growth per window).
    pub measured_inflation: f32,
    /// Sale value (bucks) summed over the current window.
    pub this_window: f64,
    last_window: f64,
    have_prev: bool,
    elapsed_ticks: u32,
}

impl Default for IncomeControl {
    fn default() -> Self {
        Self {
            rate: BUCKS_INCOME,
            measured_inflation: TARGET_INFLATION,
            this_window: 0.0,
            last_window: 0.0,
            have_prev: false,
            elapsed_ticks: 0,
        }
    }
}

pub fn income_controller(mut ctrl: ResMut<IncomeControl>) {
    ctrl.elapsed_ticks += 1;
    if ctrl.elapsed_ticks < INCOME_WINDOW_TICKS {
        return;
    }
    let this = ctrl.this_window;
    // Need a previous (non-empty) window to define growth; otherwise just rotate.
    if ctrl.have_prev && ctrl.last_window > 0.0 {
        let inflation = ((this - ctrl.last_window) / ctrl.last_window) as f32;
        ctrl.measured_inflation += INCOME_MEAS_ALPHA * (inflation - ctrl.measured_inflation);
        // Integral control: more income → more spending → faster sales growth, so
        // raise income when inflation is below target and cut it when above.
        let error = TARGET_INFLATION - ctrl.measured_inflation;
        ctrl.rate = (ctrl.rate + INCOME_K * error).clamp(INCOME_RATE_MIN, INCOME_RATE_MAX);
    }
    ctrl.last_window = this;
    ctrl.have_prev = true;
    ctrl.this_window = 0.0;
    ctrl.elapsed_ticks = 0;
}

// --- Hunger-rate controller (targets a steady death rate) -------------------
/// Starting appetite gained per second per staple, before the controller adjusts it.
pub const HUNGER_RATE_INIT: f32 = 0.5;
/// Target deaths **per tick**, as a fraction of the population (≈ 4%/min at 60 t/s).
pub const TARGET_DEATH_FRAC_PER_TICK: f32 = 0.04 / 3600.0;
/// Integral gain: `rate += GAIN·(target − measured)` each control window. Kept gentle:
/// a high gain winds the integrator up during death-free spells, then dumps it as a
/// hunger spike that starves the whole population at once (synchronized death waves).
const HUNGER_PID_GAIN: f32 = 30.0;
/// Control update interval, in ticks (deaths are rare, so we measure over a long
/// window to keep the quantized death count from jittering the controller).
const PID_PERIOD_TICKS: u32 = 600;
/// EMA smoothing on the measured death rate — discrete deaths are noisy.
const PID_MEAS_ALPHA: f32 = 0.25;
const HUNGER_RATE_MIN: f32 = 0.05;
const HUNGER_RATE_MAX: f32 = 3.0;

/// Integral controller that trims the global hunger rate so the realized
/// deaths-per-tick track `target_per_tick`. Deaths feed back via
/// `deaths_since_update`, bumped by `death_and_respawn`.
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct HungerControl {
    /// Current hunger rate (appetite/sec/staple) — `hunger_tick` applies `rate·TICK_DT`.
    pub rate: f32,
    pub target_per_tick: f32,
    /// Smoothed measured deaths per tick (readout + control error).
    pub measured_per_tick: f32,
    /// Deaths counted since the last control update.
    pub deaths_since_update: u32,
    elapsed_ticks: u32,
}

impl HungerControl {
    pub fn new(target_per_tick: f32) -> Self {
        Self {
            rate: HUNGER_RATE_INIT,
            target_per_tick,
            measured_per_tick: target_per_tick,
            deaths_since_update: 0,
            elapsed_ticks: 0,
        }
    }
}

/// Once per `PID_PERIOD_TICKS`, fold the window's death count into the smoothed rate
/// and nudge the hunger rate up when too few are dying, down when too many are.
pub fn hunger_pid(mut ctrl: ResMut<HungerControl>) {
    ctrl.elapsed_ticks += 1;
    if ctrl.elapsed_ticks < PID_PERIOD_TICKS {
        return;
    }
    let raw = ctrl.deaths_since_update as f32 / PID_PERIOD_TICKS as f32;
    ctrl.measured_per_tick += PID_MEAS_ALPHA * (raw - ctrl.measured_per_tick);
    let error = ctrl.target_per_tick - ctrl.measured_per_tick;
    ctrl.rate = (ctrl.rate + HUNGER_PID_GAIN * error).clamp(HUNGER_RATE_MIN, HUNGER_RATE_MAX);
    ctrl.deaths_since_update = 0;
    ctrl.elapsed_ticks = 0;
}

/// Age every noot by one tick.
pub fn age_noots(mut q: Query<&mut NootMeta>) {
    for mut m in &mut q {
        m.age += TICK_DT;
    }
}

/// How much the hardest terrain accelerates hunger: a noot on a cliff (difficulty 1)
/// burns `1 + this`× the appetite per tick of one on the easiest ground (difficulty 0).
/// Rough ground costs energy, giving the terrain-difficulty feature something to act on.
const HUNGER_TERRAIN_K: f32 = 1.0;

/// Transport cost: a noot moving with a full load burns `1 + this`× the hunger of an
/// empty one. Carried goods have weight, so hauling them across the map costs energy —
/// which gives distance a real price, rewards efficient routes over wandering, and makes
/// it cheaper to settle near where you trade (agglomeration) than to roam loaded.
const CARRY_HUNGER_K: f32 = 0.6;

pub fn hunger_tick(
    ctrl: Res<HungerControl>,
    sim: Res<Sim>,
    mut q: Query<(&TilePos, &Action, &Inventory, &mut Hunger)>,
) {
    let base = ctrl.rate * TICK_DT;
    let world = &sim.0;
    for (pos, action, inv, mut h) in &mut q {
        let idx = (pos.row * world.cols + pos.col) as usize;
        let difficulty = world.tiles[idx].difficulty.clamp(0.0, 1.0);
        // Carrying weighs on a noot only while it's actually hauling (a Move tick).
        let carry = if *action == Action::Move {
            let load: f32 = inv.items.iter().sum();
            CARRY_HUNGER_K * (load / CARRY_CAP).min(1.0)
        } else {
            0.0
        };
        let d = base * (1.0 + HUNGER_TERRAIN_K * difficulty + carry);
        for a in &mut h.staple {
            *a = (*a + d).min(STAPLE_SATIATION);
        }
    }
}

/// Gini coefficient of a set of values (negatives floored to 0): 0 = perfect equality,
/// approaching 1 = one holder has everything. Uses the sorted-rank formula
/// `G = (2·Σ i·x_i)/(n·Σx) − (n+1)/n` over `x` sorted ascending (i 1-indexed). For the
/// wealth distribution this is the single number behind the "how exponential" curve.
pub fn gini(values: &[f32]) -> f32 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let mut v: Vec<f32> = values.iter().map(|&x| x.max(0.0)).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f32 = v.iter().sum();
    if sum <= 0.0 {
        return 0.0;
    }
    let weighted: f32 = v
        .iter()
        .enumerate()
        .map(|(i, &x)| (i as f32 + 1.0) * x)
        .sum();
    ((2.0 * weighted) / (n as f32 * sum) - (n as f32 + 1.0) / n as f32).clamp(0.0, 1.0)
}

/// Maslow-tiered utility over the game's concepts: physiological (fed) ≫ safety
/// (food buffer + savings) ≫ esteem (positional wealth). Higher tiers only count
/// once lower ones are satisfied, so the policy learns the hierarchy. Reward = ΔU.
pub fn maslow_utility(hunger: &Hunger, inv: &Inventory, wallet: &Wallet, goods: &goods::WorldGoods) -> f32 {
    let phys = hunger.utility() / N_STAPLES as f32; // ∈[0,1]
    let min_food = (0..N_ITEMS)
        .filter_map(|i| match goods.role_of(i) {
            ItemRole::Staple(_) => Some(inv.items[i]),
            _ => None,
        })
        .fold(f32::MAX, f32::min);
    let min_food = if min_food.is_finite() { min_food } else { 0.0 };
    let safety = 0.5 * (min_food / FOOD_RESERVE).clamp(0.0, 1.0)
        + 0.5 * (wallet.bucks / SAFETY_BUCKS).clamp(0.0, 1.0);
    let esteem = positional_utility(goods, inv); // Σ ln(1+held)
    // Tier gate: higher needs start to matter once a lower need is partly met (≈30%)
    // and count fully past ≈70%. A softer gate than a hard 0.5 step so a chronically
    // half-hungry noot still gets a learning gradient for stocking food and earning.
    let gate = |x: f32| ((x - 0.3) / 0.4).clamp(0.0, 1.0);
    W_PHYS * phys + W_SAFE * gate(phys) * safety + W_ESTEEM * gate(phys) * gate(safety) * esteem
}

/// The tile a noot should head to mine: its own deposit if it owns one, else the nearest
/// *unclaimed* deposit (which it can claim on arrival). `None` if there's nowhere to mine.
fn mine_target(
    world: &crate::world::World,
    claim: &Claim,
    pos: &TilePos,
    claimed: &[bool],
) -> Option<(i32, i32)> {
    if let Some(d) = owned_deposit(world, claim) {
        let t = &world.tiles[world.deposits[d].tile];
        return Some((t.col, t.row));
    }
    world
        .deposits
        .iter()
        .filter(|dep| !claimed[dep.tile])
        .map(|dep| {
            let t = &world.tiles[dep.tile];
            (t.col, t.row)
        })
        .min_by_key(|&(c, r)| torus_distance(pos.col, pos.row, c, r, world.cols, world.rows))
}

/// The tile where the goods this noot is carrying would fetch the most, net of the haul
/// to get there: argmax over tiles of `Σ_held (local_ask − cost_basis)·held` minus a
/// per-hex transport penalty. `None` when it carries nothing worth selling — so the
/// gate naturally keeps a lightly-loaded noot mining and only sends a full one trekking.
fn best_market_tile(
    field: &PriceField,
    world: &crate::world::World,
    pos: &TilePos,
    inv: &Inventory,
    trader: &Trader,
) -> Option<(i32, i32)> {
    let carried: Vec<(usize, f32)> = (0..N_ITEMS)
        .filter(|&i| inv.items[i] >= 1.0 && !matches!(world.goods.role_of(i), ItemRole::Junk))
        .map(|i| (i, inv.items[i]))
        .collect();
    if carried.is_empty() {
        return None;
    }
    let (cols, rows) = (world.cols, world.rows);
    let mut best: Option<((i32, i32), f32)> = None;
    for r in 0..rows {
        for c in 0..cols {
            let tile = (r * cols + c) as usize;
            let mut val = 0.0f32;
            for &(item, held) in &carried {
                let margin = field.local_ask(&world.goods, tile, item) - trader.cost_basis[item];
                if margin > 0.0 {
                    val += margin * held;
                }
            }
            if val <= 0.0 {
                continue;
            }
            let dist = torus_distance(pos.col, pos.row, c, r, cols, rows) as f32;
            let score = val - MARKET_DIST_PENALTY * dist;
            if best.is_none_or(|(_, s)| score > s) {
                best = Some(((c, r), score));
            }
        }
    }
    best.map(|(t, _)| t)
}

/// Per-direction signed change in toroidal hex distance to `target` if the noot steps
/// that way: +1 if the step gets closer, −1 farther, 0 otherwise (and all-zero when
/// there is no target). Indexed identically to `hex::neighbors`, so it lines up with the
/// six directional move actions — a ready-made "head toward X" gradient for the actor.
fn heading_gradient(world: &crate::world::World, pos: &TilePos, target: Option<(i32, i32)>) -> [f32; policy::N_DIRS] {
    let mut g = [0.0f32; policy::N_DIRS];
    if let Some((tc, tr)) = target {
        let here = torus_distance(pos.col, pos.row, tc, tr, world.cols, world.rows);
        for (d, &(nc, nr)) in neighbors(pos.col, pos.row, world.cols, world.rows)
            .iter()
            .enumerate()
        {
            g[d] = (here - torus_distance(nc, nr, tc, tr, world.cols, world.rows)) as f32;
        }
    }
    g
}

/// Build the policy's non-positional feature vector and the tile index. Two engineered
/// headings — a per-direction gradient toward the target deposit and toward the nearest
/// other noot — let the actor learn to walk to a resource (then mine) and to seek out
/// trade partners (clumping); `on_minable`/`noot_near` flag that those actions are live.
// A featurizer naturally pulls in every slice of agent state; splitting it would only
// scatter the feature layout that's deliberately kept in one place.
#[allow(clippy::too_many_arguments)]
fn features(
    world: &crate::world::World,
    field: &PriceField,
    pos: &TilePos,
    claim: &Claim,
    hunger: &Hunger,
    inv: &Inventory,
    wallet: &Wallet,
    trader: &Trader,
    claimed: &[bool],
    nearest_noot: Option<(i32, i32)>,
) -> (usize, [f32; N_OTHER]) {
    let pos_idx = (pos.row * world.cols + pos.col) as usize;
    let mut o = [0.0f32; N_OTHER];
    o[0] = (wallet.bucks / 100.0).tanh();
    o[1] = hunger.staple[0] / STAPLE_SATIATION;
    o[2] = hunger.staple[1] / STAPLE_SATIATION;
    o[3] = positional_utility(&world.goods, inv) / ESTEEM_NORM;
    o[4] = if owned_deposit(world, claim).is_some() { 1.0 } else { 0.0 };
    o[5] = 1.0; // bias

    let dep = heading_gradient(world, pos, mine_target(world, claim, pos, claimed));
    o[policy::O_DEPOSIT_DIR..policy::O_DEPOSIT_DIR + policy::N_DIRS].copy_from_slice(&dep);
    o[policy::O_ON_MINABLE] = if can_mine_here(world, claim, pos, inv) { 1.0 } else { 0.0 };

    let noot = heading_gradient(world, pos, nearest_noot);
    o[policy::O_NOOT_DIR..policy::O_NOOT_DIR + policy::N_DIRS].copy_from_slice(&noot);
    if let Some((tc, tr)) = nearest_noot {
        // "Within trade range" ≈ adjacent or co-located (the trade radius is ~1 hex).
        let d = torus_distance(pos.col, pos.row, tc, tr, world.cols, world.rows);
        o[policy::O_NOOT_NEAR] = if d <= 1 { 1.0 } else { 0.0 };
    }
    o[policy::O_TERRAIN] = world.tiles[pos_idx].difficulty.clamp(0.0, 1.0);

    let market = best_market_tile(field, world, pos, inv, trader);
    let market_dir = heading_gradient(world, pos, market);
    o[policy::O_MARKET_DIR..policy::O_MARKET_DIR + policy::N_DIRS].copy_from_slice(&market_dir);
    o[policy::O_HAS_CARGO] = if market.is_some() { 1.0 } else { 0.0 };
    (pos_idx, o)
}

/// The tile of the nearest noot other than `me` (for the trade-partner heading).
fn nearest_other_noot(
    snapshot: &[(Entity, i32, i32)],
    me: Entity,
    pos: &TilePos,
    cols: i32,
    rows: i32,
) -> Option<(i32, i32)> {
    snapshot
        .iter()
        .filter(|(e, _, _)| *e != me)
        .min_by_key(|(_, c, r)| torus_distance(pos.col, pos.row, *c, *r, cols, rows))
        .map(|&(_, c, r)| (c, r))
}

/// True when the noot is standing on its own deposit with room to extract more — the
/// precondition for `extract`, and the `on_minable` state feature. (A claimless noot also
/// mines the tick *after* it lands on an unclaimed deposit, once `claim_improvements` has
/// assigned it; the Mine executor stays put on the target tile so that handoff happens.)
fn can_mine_here(world: &crate::world::World, claim: &Claim, pos: &TilePos, inv: &Inventory) -> bool {
    let tile = (pos.row * world.cols + pos.col) as usize;
    claim.hex == Some(tile)
        && world.tiles[tile].deposit.is_some_and(|d| {
            let slot = world.deposits[d].element_slot;
            inv.items[goods::item_index(slot, GoodForm::Raw)] < CARRY_CAP
        })
}

/// Units of held goods worth taking to market: everything non-junk, minus the food
/// reserve a noot keeps for itself (it won't sell within `FOOD_RESERVE` of a staple).
fn sellable_units(world: &crate::world::World, inv: &Inventory) -> f32 {
    (0..N_ITEMS)
        .map(|i| {
            let reserve = if matches!(world.goods.role_of(i), ItemRole::Staple(_)) {
                FOOD_RESERVE
            } else {
                0.0
            };
            let junk = matches!(world.goods.role_of(i), ItemRole::Junk);
            (inv.items[i] - reserve).max(0.0) * if junk { 0.0 } else { 1.0 }
        })
        .sum()
}

/// Whether the noot holds any unrefined intermediate (the precondition for `refine`).
fn has_intermediate(world: &crate::world::World, inv: &Inventory) -> bool {
    (0..N_ITEMS)
        .any(|i| matches!(world.goods.role_of(i), ItemRole::Intermediate) && inv.items[i] > 0.0)
}

/// Total non-junk units a noot is carrying (its haul load).
fn carried_units(world: &crate::world::World, inv: &Inventory) -> f32 {
    (0..N_ITEMS)
        .filter(|&i| !matches!(world.goods.role_of(i), ItemRole::Junk))
        .map(|i| inv.items[i])
        .sum()
}

/// Which committed options the policy may choose from this state, given one-hex ownership.
/// Mine: owns a deposit, or is unowned and an unclaimed deposit exists. Sell: has surplus.
/// Refine: holds an intermediate and a refinery exists to use. Build shop/refinery: owns
/// nothing yet and can afford it. Explore: always (the fallback).
#[allow(clippy::too_many_arguments)]
fn option_mask(
    world: &crate::world::World,
    claim: &Claim,
    inv: &Inventory,
    wallet: &Wallet,
    free_deposit: bool,
    any_refinery: bool,
) -> [bool; N_ACT] {
    let owns_nothing = claim.hex.is_none();
    let mut mask = [false; N_ACT];
    mask[policy::A_MINE] = owned_deposit(world, claim).is_some() || (owns_nothing && free_deposit);
    mask[policy::A_SELL] = sellable_units(world, inv) >= 1.0;
    mask[policy::A_REFINE] = any_refinery && has_intermediate(world, inv);
    mask[policy::A_BUILD_SHOP] = owns_nothing && wallet.bucks >= SHOP_COST + SHOP_BUILD_BUFFER;
    mask[policy::A_BUILD_REFINERY] =
        owns_nothing && wallet.bucks >= REFINERY_COST + SHOP_BUILD_BUFFER;
    mask[policy::A_EXPLORE] = true;
    mask
}

/// The destination a committed option navigates toward: its (or the nearest unclaimed)
/// deposit for Mine; the nearest shop, else the best market, for Sell; the nearest
/// refinery for Refine. In-place options (Explore, Build — done on the current tile)
/// have no travel target.
#[allow(clippy::too_many_arguments)]
fn option_target(
    act: usize,
    world: &crate::world::World,
    field: &PriceField,
    claim: &Claim,
    pos: &TilePos,
    inv: &Inventory,
    trader: &Trader,
    claimed: &[bool],
) -> Option<(i32, i32)> {
    use crate::world::StructureKind;
    match act {
        policy::A_MINE => mine_target(world, claim, pos, claimed),
        policy::A_SELL => nearest_structure(world, pos, StructureKind::Shop)
            .or_else(|| best_market_tile(field, world, pos, inv, trader)),
        policy::A_REFINE => nearest_structure(world, pos, StructureKind::Refinery),
        _ => None,
    }
}

/// Whether the committed option has run its course and the policy should decide afresh:
/// mined a worthwhile load, sold off the surplus, refined everything, claimed a hex by
/// building, or hit the step cap.
fn option_done(
    act: usize,
    world: &crate::world::World,
    claim: &Claim,
    inv: &Inventory,
    plan_ticks: u32,
) -> bool {
    if plan_ticks == 0 {
        return true;
    }
    match act {
        policy::A_MINE => carried_units(world, inv) >= LOAD_THRESHOLD,
        policy::A_SELL => sellable_units(world, inv) < 1.0,
        policy::A_REFINE => !has_intermediate(world, inv),
        policy::A_BUILD_SHOP | policy::A_BUILD_REFINERY => claim.hex.is_some(),
        policy::A_EXPLORE => false,
        _ => true,
    }
}

/// One **GPS-guided** hex step from `(col,row)` toward `target`. Each neighbour is scored
/// by the critic's learned value of standing there plus a large [`ROUTE_OPTIMISM`] bonus
/// for staying on the greedy route (the step that shortens the toroidal distance). The
/// noot takes the best-scoring neighbour, so it follows the suggested route by default
/// but will deviate toward an adjacent tile whose learned value beats the route — like a
/// GPS that proposes a path yet lets you turn off for something better. The map has no
/// impassable tiles, so the route always exists; the bonus keeps progress reliable.
fn value_guided_step(
    world: &crate::world::World,
    ac: &ActorCritic,
    o: &[f32; policy::N_OTHER],
    col: i32,
    row: i32,
    target: (i32, i32),
) -> (i32, i32) {
    let (cols, rows) = (world.cols, world.rows);
    let here = torus_distance(col, row, target.0, target.1, cols, rows);
    let mut best = (col, row);
    let mut best_score = f32::MIN;
    for (nc, nr) in neighbors(col, row, cols, rows) {
        let d = torus_distance(nc, nr, target.0, target.1, cols, rows);
        let on_route = (here - d) as f32; // +1 closer, 0 sideways, −1 farther
        let tile = (nr * cols + nc) as usize;
        let score = ROUTE_OPTIMISM * on_route + ac.value(tile, o);
        if score > best_score {
            best_score = score;
            best = (nc, nr);
        }
    }
    best
}

/// The learned decision step, over **committed options** rather than primitive steps.
/// Once per noot per cadence: if a committed option is still in progress, the executor
/// drives its next step (navigate toward the goal, mine, refine, or scout) and the policy
/// stays out of it — that commitment is what turns per-step dithering into directed
/// produce→haul→sell behaviour. When the option terminates (or the noot died), close out
/// the option's transition (reward = the ΔU it accrued plus production shaping, or a death
/// penalty), pick a fresh option from the shared policy, and lock in its target. Each
/// executor step persists an `Action` the extract/refine/trade systems apply every frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn policy_step(
    sim: Res<Sim>,
    field: Res<PriceField>,
    ac: Res<ActorCritic>,
    mut trainer: ResMut<Trainer>,
    mut rng: ResMut<SimRng>,
    mut q: Query<(
        Entity,
        &mut TilePos,
        &Claim,
        &Inventory,
        &Hunger,
        &Wallet,
        &Trader,
        &mut Action,
        &mut PolicyMemory,
    )>,
) {
    let world = &sim.0;
    // Snapshot every noot's tile up front so each can read the others' positions while
    // we hold the query mutably (positions only shift one hex/tick, so this is fresh).
    let snapshot: Vec<(Entity, i32, i32)> =
        q.iter().map(|(e, p, ..)| (e, p.col, p.row)).collect();
    // Which hexes are owned right now, so a noot heads only for *unclaimed* deposits and
    // the masks know whether a free deposit / any refinery exists (one-hex ownership).
    let n_tiles = (world.cols * world.rows) as usize;
    let mut claimed = vec![false; n_tiles];
    for (_, _, claim, ..) in q.iter() {
        if let Some(h) = claim.hex {
            claimed[h] = true;
        }
    }
    let free_deposit = world.deposits.iter().any(|d| !claimed[d.tile]);
    let any_refinery = world
        .structures
        .iter()
        .any(|s| s.kind == crate::world::StructureKind::Refinery);
    for (e, mut pos, claim, inv, hunger, wallet, trader, mut action, mut mem) in &mut q {
        mem.cooldown -= TICK_DT;
        if mem.cooldown > 0.0 && !mem.died {
            continue;
        }

        // Features at the current tile, recomputed each acting step: they feed both the
        // option decision (when re-deciding below) and the value-guided "GPS" movement.
        let nearest_noot = nearest_other_noot(&snapshot, e, &pos, world.cols, world.rows);
        let (s_pos, s_o) = features(
            world, &field, &pos, claim, hunger, inv, wallet, trader, &claimed, nearest_noot,
        );

        // Re-decide only at option boundaries: when the committed plan is finished, was
        // never set, or the noot just died (banking one terminal transition).
        let finished = !mem.committed
            || mem.died
            || option_done(mem.last_act, world, claim, inv, mem.plan_ticks);
        if finished {
            let u_now = maslow_utility(hunger, inv, wallet, &world.goods);

            // Close out the previous option's transition (reward = the ΔU the option
            // accrued, or a death penalty). Committed options make the long
            // produce→haul→sell chain learnable from this bare ΔU alone — the Mine
            // option's value bootstraps from the later Sell/eat reward — so no
            // production shaping bonus is needed.
            if mem.has_prev {
                let (r, done) = if mem.died {
                    (-DEATH_PENALTY, true)
                } else {
                    (u_now - mem.last_u, false)
                };
                trainer.record(Transition {
                    pos: mem.last_pos,
                    o: mem.last_o,
                    mask: mem.last_mask,
                    act: mem.last_act,
                    r,
                    pos2: s_pos,
                    o2: s_o,
                    done,
                });
            }
            mem.died = false;

            // Pick the next option: masked softmax, or an ε-chance uniform random option
            // (the per-noot exploration temperament). Uniform ε samples each valid option
            // often, so no extra bias is needed to discover producing or building.
            let mask = option_mask(world, claim, inv, wallet, free_deposit, any_refinery);
            let act = if rng.0.chance(mem.explore) {
                let valid: Vec<usize> = (0..N_ACT).filter(|&a| mask[a]).collect();
                valid[rng.0.below(valid.len())]
            } else {
                let logits = ac.logits(s_pos, &s_o);
                let probs = policy::masked_softmax(&logits, &mask);
                policy::sample(&probs, &mut rng.0)
            };

            mem.last_pos = s_pos;
            mem.last_o = s_o;
            mem.last_mask = mask;
            mem.last_act = act;
            mem.last_u = u_now;
            mem.has_prev = true;
            mem.committed = true;
            mem.plan_target = option_target(act, world, &field, claim, &pos, inv, trader, &claimed);
            mem.plan_ticks = OPTION_MAX_TICKS;
        }

        // Execute one step of the committed option, setting this tick's Action.
        match mem.last_act {
            policy::A_MINE => match mem.plan_target {
                // On the deposit tile: mine (or hold here so claim_improvements can claim
                // it before mining starts next tick). Otherwise step toward it.
                Some((tc, tr)) if pos.col == tc && pos.row == tr => *action = Action::Mine,
                Some((tc, tr)) => {
                    let (nc, nr) = value_guided_step(world, &ac, &s_o, pos.col, pos.row, (tc, tr));
                    pos.col = nc;
                    pos.row = nr;
                    *action = Action::Move;
                }
                None => *action = Action::Idle,
            },
            policy::A_SELL => match mem.plan_target {
                // At the market: linger (Idle, no haul cost) so a passing buyer can trade.
                Some((tc, tr)) if pos.col == tc && pos.row == tr => *action = Action::Idle,
                Some((tc, tr)) => {
                    let (nc, nr) = value_guided_step(world, &ac, &s_o, pos.col, pos.row, (tc, tr));
                    pos.col = nc;
                    pos.row = nr;
                    *action = Action::Move;
                }
                None => *action = Action::Idle,
            },
            policy::A_REFINE => match mem.plan_target {
                // Refining happens only inside a refinery — go there, then refine on it.
                Some((tc, tr)) if pos.col == tc && pos.row == tr => *action = Action::Refine,
                Some((tc, tr)) => {
                    let (nc, nr) = value_guided_step(world, &ac, &s_o, pos.col, pos.row, (tc, tr));
                    pos.col = nc;
                    pos.row = nr;
                    *action = Action::Move;
                }
                None => *action = Action::Idle,
            },
            policy::A_BUILD_SHOP | policy::A_BUILD_REFINERY => {
                let here = (pos.row * world.cols + pos.col) as usize;
                // Buildable iff not a deposit hex (open ground, or build over a structure).
                if world.tiles[here].deposit.is_none() {
                    *action = if mem.last_act == policy::A_BUILD_SHOP {
                        Action::BuildShop
                    } else {
                        Action::BuildRefinery
                    };
                } else {
                    // Standing on a deposit — hop to a neighbour to find open ground.
                    let (nc, nr) = neighbors(pos.col, pos.row, world.cols, world.rows)
                        [rng.0.below(policy::N_DIRS)];
                    pos.col = nc;
                    pos.row = nr;
                    *action = Action::Move;
                }
            }
            _ => {
                // Explore: a random hex step (scouting for deposits/markets/partners).
                let (nc, nr) = neighbors(pos.col, pos.row, world.cols, world.rows)
                    [rng.0.below(policy::N_DIRS)];
                pos.col = nc;
                pos.row = nr;
                *action = Action::Move;
            }
        }
        mem.plan_ticks = mem.plan_ticks.saturating_sub(1);
        let tf = terrain_factor(world.tiles[(pos.row * world.cols + pos.col) as usize].difficulty);
        mem.cooldown = BASE_STEP_TIME / tf;
    }
}

/// A few A2C minibatch updates on the shared policy each frame (warms up first) —
/// several per frame so the shared buffer's experience is reused and learning keeps
/// pace with the ~160 decisions/sec the population generates.
pub fn train_policy(mut ac: ResMut<ActorCritic>, mut trainer: ResMut<Trainer>, mut rng: ResMut<SimRng>) {
    for _ in 0..TRAIN_ITERS_PER_FRAME {
        trainer.train(&mut ac, &mut rng.0);
    }
}

/// Register the fixed-tick simulation pipeline on `schedule`, in canonical order and
/// chained so each system sees the previous one's writes. The single source of truth
/// for the sim order, shared by the GUI app's `SimSchedule` and the headless harness —
/// neither defines the list itself. (Movement is a GUI-only sprite glide, not here.)
pub fn add_sim_systems(schedule: &mut Schedule) {
    schedule.add_systems(
        (
            simulate,
            income,
            income_controller,
            hunger_tick,
            hunger_pid,
            age_noots,
            update_price_field,
            policy_step,
            build_structures,
            claim_improvements,
            extract,
            refine,
            meet_and_trade,
            consume,
            death_and_respawn,
            update_rates,
            train_policy,
        )
            .chain(),
    );
}

/// Advance the resource simulation (deposit regrowth) by one tick.
pub fn simulate(mut sim: ResMut<Sim>) {
    sim.0.tick(TICK_DT);
}

/// Seconds a noot can sit fully starving before it dies and is reborn fresh.
pub const DEATH_GRACE_SECS: f32 = 20.0;

/// A noot that has sat fully starving for `DEATH_GRACE_SECS` dies and is reborn fresh
/// at a random tile (full wallet, empty inventory, half hunger, no claim, new
/// exploration temperament). Position is set on the tile only — the GUI sprite glide
/// (when present) follows it — so this system has no rendering dependency. The policy
/// keeps its cached pre-death (state, action) plus a `died` flag so `policy_step`
/// banks one terminal transition before the new episode.
#[allow(clippy::type_complexity)]
pub fn death_and_respawn(
    mut rng: ResMut<SimRng>,
    sim: Res<Sim>,
    mut ctrl: ResMut<HungerControl>,
    mut q: Query<(
        &mut Hunger,
        &mut Inventory,
        &mut Wallet,
        &mut PolicyMemory,
        &mut Trader,
        &mut NootMeta,
        &mut NootName,
        &mut Claim,
        &mut TilePos,
    )>,
) {
    let world = &sim.0;
    for (mut hunger, mut inv, mut wallet, mut mem, mut trader, mut meta, mut name, mut claim, mut pos) in
        &mut q
    {
        if hunger.fully_starving() {
            hunger.starving_secs += TICK_DT;
        } else {
            hunger.starving_secs = 0.0;
        }
        if hunger.starving_secs < DEATH_GRACE_SECS {
            continue;
        }
        ctrl.deaths_since_update += 1;
        *inv = Inventory::new();
        wallet.bucks = STARTING_BUCKS;
        *hunger = Hunger::fresh(&mut rng.0);
        mem.died = true;
        mem.explore = rng.0.range(EXPLORE_MIN, EXPLORE_MAX);
        mem.cooldown = 0.0;
        // Drop any committed plan so the reborn noot decides fresh next tick.
        mem.committed = false;
        mem.plan_target = None;
        mem.plan_ticks = 0;
        *trader = Trader::new();
        *meta = NootMeta::new();
        // The name persists across death; only the incarnation advances ("the 2nd", ...).
        name.reincarnate();
        // Abandon the owned hex: a deposit reopens; a structure stands for another to take.
        claim.hex = None;
        pos.col = rng.0.below(world.cols as usize) as i32;
        pos.row = rng.0.below(world.rows as usize) as i32;
    }
}

pub fn extract(
    mut sim: ResMut<Sim>,
    mut stats: ResMut<EconStats>,
    mut q: Query<(&Action, &Claim, &TilePos, &mut Inventory, &mut NootMeta)>,
) {
    for (action, claim, pos, mut inv, mut meta) in &mut q {
        if *action != Action::Mine {
            continue;
        }
        // Must be standing on the deposit it owns (its claimed hex).
        let tile = (pos.row * sim.0.cols + pos.col) as usize;
        if claim.hex != Some(tile) {
            continue;
        }
        let Some(deposit) = sim.0.tiles[tile].deposit else {
            continue;
        };
        let slot = sim.0.deposits[deposit].element_slot;
        let raw = goods::item_index(slot, GoodForm::Raw);
        if inv.items[raw] >= CARRY_CAP {
            continue;
        }
        // Learning by doing: a seasoned miner pulls more per second.
        let rate = WORK_RATE * skill_factor(meta.experience);
        let got = sim.0.extract_from(deposit, rate, TICK_DT) as f32;
        inv.items[raw] += got;
        meta.experience += got;
        stats.produced_window += got;
        stats.produced_total += got as f64;
    }
}

/// Build (or rebuild over) a structure for any noot whose action is `BuildShop`/
/// `BuildRefinery`: it must own nothing yet, afford the cost, and stand on non-deposit
/// ground that isn't an owned structure. The builder claims the hex (its one improvement).
pub fn build_structures(
    mut sim: ResMut<Sim>,
    mut q: Query<(&Action, &TilePos, &mut Wallet, &mut Claim)>,
) {
    let n_tiles = (sim.0.cols * sim.0.rows) as usize;
    let mut claimed = vec![false; n_tiles];
    for (_, _, _, claim) in &q {
        if let Some(h) = claim.hex {
            claimed[h] = true;
        }
    }
    for (action, pos, mut wallet, mut claim) in &mut q {
        let (kind, cost) = match action {
            Action::BuildShop => (crate::world::StructureKind::Shop, SHOP_COST),
            Action::BuildRefinery => (crate::world::StructureKind::Refinery, REFINERY_COST),
            _ => continue,
        };
        if claim.hex.is_some() || wallet.bucks < cost {
            continue;
        }
        let tile = (pos.row * sim.0.cols + pos.col) as usize;
        // Can't build on a deposit, nor over a structure someone still owns.
        if sim.0.tiles[tile].deposit.is_some() || claimed[tile] {
            continue;
        }
        sim.0.build_structure(tile, kind);
        claim.hex = Some(tile);
        claimed[tile] = true;
        wallet.bucks -= cost;
    }
}

/// Assign ownership of the hex a noot is actively working but no one owns: a claimless
/// noot mining (`Mine`) an unclaimed deposit, or refining (`Refine`) at an unclaimed
/// refinery, adopts that hex. Claims live only in `Claim`, so a hex frees on its holder's
/// death and the deposit/structure can be taken up again.
pub fn claim_improvements(sim: Res<Sim>, mut q: Query<(&Action, &TilePos, &mut Claim)>) {
    let n_tiles = (sim.0.cols * sim.0.rows) as usize;
    let mut claimed = vec![false; n_tiles];
    for (_, _, claim) in &q {
        if let Some(h) = claim.hex {
            claimed[h] = true;
        }
    }
    for (action, pos, mut claim) in &mut q {
        if claim.hex.is_some() {
            continue;
        }
        let tile = (pos.row * sim.0.cols + pos.col) as usize;
        if claimed[tile] {
            continue;
        }
        let adopt = match action {
            Action::Mine => sim.0.tiles[tile].deposit.is_some(),
            Action::Refine => {
                sim.0.structure_kind(tile) == Some(crate::world::StructureKind::Refinery)
            }
            _ => false,
        };
        if adopt {
            claim.hex = Some(tile);
            claimed[tile] = true;
        }
    }
}

/// Refine held intermediates — only for a noot whose action is `Refine` *and* that is
/// standing inside a refinery (any refinery; refining is shared). Faster with experience.
pub fn refine(sim: Res<Sim>, mut q: Query<(&Action, &TilePos, &mut Inventory, &mut NootMeta)>) {
    for (action, pos, mut inv, mut meta) in &mut q {
        if *action != Action::Refine {
            continue;
        }
        let tile = (pos.row * sim.0.cols + pos.col) as usize;
        if sim.0.structure_kind(tile) != Some(crate::world::StructureKind::Refinery) {
            continue;
        }
        let rate = REFINE_RATE * skill_factor(meta.experience);
        for slot in 0..4 {
            let raw = goods::item_index(slot, GoodForm::Raw);
            if sim.0.goods.role_of(raw) != ItemRole::Intermediate || inv.items[raw] <= 0.0 {
                continue;
            }
            let refined = goods::item_index(slot, GoodForm::Refined);
            let amount = (rate * TICK_DT).min(inv.items[raw]);
            inv.items[raw] -= amount;
            inv.items[refined] += amount;
            meta.experience += amount;
        }
    }
}

pub fn consume(
    sim: Res<Sim>,
    mut stats: ResMut<EconStats>,
    mut q: Query<(&mut Inventory, &mut Hunger)>,
) {
    let dt_goods = &sim.0.goods;
    let mut eaten = 0.0f32;
    let mut utility_gained = 0.0f32;
    for (mut inv, mut hunger) in &mut q {
        // Staples first (satisficing: eat only to satiation, surplus unused).
        // Positional goods are *durable* — they're held as wealth (welfare from
        // the holding, see `positional_utility`) and sold by choice, never eaten.
        for item in 0..N_ITEMS {
            if let ItemRole::Staple(sub) = dt_goods.role_of(item) {
                if inv.items[item] > 0.0 && hunger.staple[sub] > 0.0 {
                    let needed = hunger.staple[sub] / EAT_VALUE;
                    let eat = inv.items[item].min(needed);
                    inv.items[item] -= eat;
                    hunger.staple[sub] = (hunger.staple[sub] - eat * EAT_VALUE).max(0.0);
                    eaten += eat;
                    // Welfare (also feeds the policy reward via the utility delta).
                    utility_gained += (eat * EAT_VALUE) / STAPLE_SATIATION;
                }
            }
        }
    }
    stats.consumed_window += eaten;
    stats.consumed_total += eaten as f64;
    stats.utility_window += utility_gained;
    stats.utility_total += utility_gained as f64;
}

/// Diminishing (logarithmic) welfare from the durable positional goods a noot
/// currently holds in inventory: `Σ ln(1 + held)` over positional items.
pub fn positional_utility(goods: &goods::WorldGoods, inv: &Inventory) -> f32 {
    (0..N_ITEMS)
        .filter(|&i| matches!(goods.role_of(i), ItemRole::Positional(_)))
        .map(|i| (1.0 + inv.items[i]).ln())
        .sum()
}

struct Snap {
    e: Entity,
    pos: Vec2,
    tile: usize,
    inv: [f32; N_ITEMS],
    bucks: f32,
    hunger: [f32; N_STAPLES],
    satisfied: bool,
    /// Learned discount on anticipated resale value, and average price paid per
    /// held item (the floor a noot will resell at).
    discount: f32,
    cost_basis: [f32; N_ITEMS],
}

/// What a noot is willing to *pay* to acquire one unit (buyer side): the greater
/// of its consumption value and — if fed and flush — its arbitrage value (a
/// learned discount on the good's market ask, what it bets it can resell for).
fn wtp(goods: &goods::WorldGoods, item: usize, s: &Snap) -> f32 {
    let consumption = match goods.role_of(item) {
        ItemRole::Staple(sub) => {
            let hunger_val = STAPLE_VALUE * (s.hunger[sub] / STAPLE_SATIATION);
            // Below the reserve, also stock up (even when not hungry) so a buffer of
            // food can accumulate when it's cheap.
            if s.inv[item] < FOOD_RESERVE {
                hunger_val.max(STAPLE_VALUE * FOOD_BUFFER_WTP_FRAC)
            } else {
                hunger_val
            }
        }
        // Durable luxuries are only bought once fed; marginal worth falls as the
        // noot's *held* stock of that good grows.
        ItemRole::Positional(_) => {
            if s.satisfied {
                POSITIONAL_VALUE / (1.0 + s.inv[item])
            } else {
                0.0
            }
        }
        // Anyone can refine, so an intermediate is worth its refined output.
        ItemRole::Intermediate => REFINER_WTP_INTERMEDIATE,
        ItemRole::Junk => 0.0,
    };
    // Arbitrage demand: only with staples satisfied and cash above a reserve, so a
    // noot never spends its food money speculating.
    let arbitrage = if s.satisfied
        && s.bucks > ARBITRAGE_RESERVE
        && !matches!(goods.role_of(item), ItemRole::Junk)
    {
        s.discount * base_ask(goods, item)
    } else {
        0.0
    };
    consumption.max(arbitrage)
}

/// The lowest price at which a noot will *part with* one unit it holds (seller
/// side): the worth of keeping it. For staples that's its hunger value; for
/// durable positional goods, the marginal keep-value discounted by hunger (so the
/// starving liquidate wealth for food). Cost basis is enforced separately in the
/// ask, so resales never clear at a loss.
fn reservation(goods: &goods::WorldGoods, item: usize, s: &Snap) -> f32 {
    match goods.role_of(item) {
        // Won't part with food up to its reserve (held at full staple value, so the
        // buffer never clears); only true surplus beyond it sells, cheaply when fed.
        ItemRole::Staple(sub) => {
            if s.inv[item] <= FOOD_RESERVE {
                STAPLE_VALUE
            } else {
                STAPLE_VALUE * (s.hunger[sub] / STAPLE_SATIATION)
            }
        }
        ItemRole::Positional(_) => {
            let marginal = POSITIONAL_VALUE / (1.0 + s.inv[item]);
            let hunger_frac = s.hunger.iter().copied().fold(0.0f32, f32::max) / STAPLE_SATIATION;
            marginal * (1.0 - POSITIONAL_SELL_URGENCY * hunger_frac).max(0.0)
        }
        // Raw intermediates are refined rather than kept, so nothing to hold onto.
        ItemRole::Intermediate => 0.0,
        ItemRole::Junk => 0.0,
    }
}

/// The good's market price (what a fed consumer pays at full price).
fn base_ask(goods: &goods::WorldGoods, item: usize) -> f32 {
    match goods.role_of(item) {
        ItemRole::Intermediate => ASK_INTERMEDIATE,
        ItemRole::Staple(_) | ItemRole::Positional(_) => match form_of(item) {
            GoodForm::Raw => ASK_RAW_CONSUMABLE,
            GoodForm::Refined => ASK_REFINED_CONSUMABLE,
        },
        ItemRole::Junk => f32::MAX,
    }
}

/// Multiplier on a glutted seller's ask: full price up to `SURPLUS_FREE` held,
/// then falling hyperbolically toward `SURPLUS_FLOOR` as the overstock grows — the
/// more a noot is drowning in a good, the cheaper it dumps the surplus.
fn surplus_discount(held: f32) -> f32 {
    if held <= SURPLUS_FREE {
        1.0
    } else {
        (1.0 / (1.0 + SURPLUS_K * (held - SURPLUS_FREE))).max(SURPLUS_FLOOR)
    }
}

/// The price a noot offers one unit at: the *local* market ask (cheap near a source,
/// dear in a deficit region) discounted for its own surplus, but never below its cost
/// basis — so freshly mined goods (cost ≈ 0) dump cheap when glutted, while goods
/// bought to flip won't be resold at a loss.
fn seller_ask(local_ask: f32, held: f32, cost_basis: f32) -> f32 {
    (local_ask * surplus_discount(held)).max(cost_basis)
}

/// Spatial good prices: the local market ask for each item at each tile. The same good
/// is cheap on top of a rich source (glut) and dear in regions with no nearby source
/// (scarce), so hauling it across the map earns a real markup. Derived from a
/// distance-decayed, fullness-weighted supply potential per element slot (both the raw
/// and refined item of a slot share its supply). Transient: rebuilt from the world,
/// never serialized.
#[derive(Resource, Default)]
pub struct PriceField {
    cols: i32,
    rows: i32,
    n_slots: usize,
    /// `price[tile][item]` = local market ask (bucks) for `item` at `tile`.
    price: Vec<[f32; N_ITEMS]>,
    /// `src_dist[tile*n_slots + slot]` = hex distance from `tile` to the nearest deposit
    /// of `slot` — how far a unit of that good has been hauled when sold there.
    src_dist: Vec<f32>,
    elapsed: u32,
}

impl PriceField {
    /// Local market ask for `item` at tile index `tile`, falling back to the global
    /// base ask before the field is first built.
    pub fn local_ask(&self, goods: &goods::WorldGoods, tile: usize, item: usize) -> f32 {
        self.price
            .get(tile)
            .map(|row| row[item])
            .unwrap_or_else(|| base_ask(goods, item))
    }

    /// Hex distance from `tile` to the nearest deposit of the element `slot` (0 before
    /// the field is built) — the haul distance of a good of that slot sold here.
    pub fn source_dist(&self, tile: usize, slot: usize) -> f32 {
        if self.n_slots == 0 {
            return 0.0;
        }
        self.src_dist
            .get(tile * self.n_slots + slot)
            .copied()
            .unwrap_or(0.0)
    }
}

/// A deposit's pull on local prices: how full it is (0 when depleted, 1 when brimming),
/// so a worked-out finite source stops glutting its surroundings.
fn supply_weight(dep: &crate::world::Deposit) -> f32 {
    match &dep.kind {
        DepositKind::Replenishable {
            stock, capacity, ..
        } => (*stock / *capacity).clamp(0.0, 1.0) as f32,
        DepositKind::Finite {
            remaining, initial,
        } => (*remaining / *initial).clamp(0.0, 1.0) as f32,
    }
}

/// Recompute the whole price field from the current deposit layout/fullness: for each
/// slot, accumulate a linear distance kernel from every deposit (weighted by fullness),
/// normalize to `[0,1]`, then map full→`PRICE_FLOOR` and empty→`PRICE_CEIL` of base.
fn rebuild_price_field(field: &mut PriceField, world: &World) {
    let (cols, rows) = (world.cols, world.rows);
    let n = (cols * rows) as usize;
    let n_slots = world.chosen.len();
    field.cols = cols;
    field.rows = rows;
    field.n_slots = n_slots;
    field.price = vec![[0.0f32; N_ITEMS]; n];
    field.src_dist = vec![f32::MAX; n * n_slots];

    let mut pot = vec![vec![0.0f32; n]; n_slots];
    for dep in &world.deposits {
        let slot = dep.element_slot;
        let dt = &world.tiles[dep.tile];
        let weight = supply_weight(dep);
        for r in 0..rows {
            for c in 0..cols {
                let tile = (r * cols + c) as usize;
                let dist = torus_distance(c, r, dt.col, dt.row, cols, rows) as f32;
                // Nearest source (physical, regardless of fullness) for the haul metric.
                let sd = &mut field.src_dist[tile * n_slots + slot];
                *sd = sd.min(dist);
                // Glut suppression scales with how full the deposit is.
                let k = (1.0 - dist / SUPPLY_RANGE).max(0.0);
                if weight > 0.0 && k > 0.0 {
                    pot[slot][tile] += weight * k;
                }
            }
        }
    }
    // A slot with no deposits keeps its sentinel; clamp those to 0 so the metric is sane.
    for d in &mut field.src_dist {
        if !d.is_finite() {
            *d = 0.0;
        }
    }

    for (slot, pot_slot) in pot.iter().enumerate() {
        let maxp = pot_slot.iter().copied().fold(0.0f32, f32::max).max(1e-6);
        for (tile, &p) in pot_slot.iter().enumerate() {
            let norm = (p / maxp).clamp(0.0, 1.0);
            // norm 1 (right on a full source) → FLOOR; norm 0 (no source near) → CEIL.
            let mult = PRICE_CEIL + (PRICE_FLOOR - PRICE_CEIL) * norm;
            for form in 0..2 {
                let item = slot * 2 + form;
                field.price[tile][item] = base_ask(&world.goods, item) * mult;
            }
        }
    }
}

/// Keep the spatial price field current: build it on first run (and if the map resizes),
/// then rebuild every `PRICE_FIELD_REBUILD_TICKS` to track deposit depletion/regrowth.
pub fn update_price_field(sim: Res<Sim>, mut field: ResMut<PriceField>) {
    let world = &sim.0;
    let resized = field.cols != world.cols || field.rows != world.rows;
    if field.price.is_empty() || resized {
        rebuild_price_field(&mut field, world);
        field.elapsed = 0;
        return;
    }
    field.elapsed += 1;
    if field.elapsed >= PRICE_FIELD_REBUILD_TICKS {
        rebuild_price_field(&mut field, world);
        field.elapsed = 0;
    }
}

struct Tx {
    buyer: Entity,
    seller: Entity,
    item: usize,
    price: f32,
}

#[allow(clippy::type_complexity)]
pub fn meet_and_trade(
    sim: Res<Sim>,
    field: Res<PriceField>,
    mut stats: ResMut<EconStats>,
    mut income: ResMut<IncomeControl>,
    mut q: Query<(
        Entity,
        &TilePos,
        &mut Inventory,
        &mut Wallet,
        &Hunger,
        &mut Trader,
        &mut NootMeta,
    )>,
) {
    let goods = &sim.0.goods;
    let cols = sim.0.cols;
    let hex_size = sim.0.hex_size;
    let radius2 = (hex_size * TRADE_RADIUS_FACTOR).powi(2);

    // Snapshot (immutable read) so we can reason about pairs without aliasing.
    // Positions come from the tile (pixel centre), so trade has no rendering dep.
    let mut snaps: Vec<Snap> = q
        .iter()
        .map(|(e, tp, inv, wal, hunger, trader, _meta)| {
            let (px, py) = hex_center(tp.col, tp.row, hex_size);
            Snap {
                e,
                pos: Vec2::new(px, py),
                tile: (tp.row * cols + tp.col) as usize,
                inv: inv.items,
                bucks: wal.bucks,
                hunger: hunger.staple,
                satisfied: hunger.satisfied(),
                discount: trader.discount,
                cost_basis: trader.cost_basis,
            }
        })
        .collect();

    let mut txs: Vec<Tx> = Vec::new();

    // Trading is automatic: any two nearby noots clear their best mutually-beneficial
    // trade. Each noot's learned `discount` (and hunger-driven reservation) are the
    // internal thresholds that decide what it will buy/sell and at what price.
    for i in 0..snaps.len() {
        for j in (i + 1)..snaps.len() {
            if snaps[i].pos.distance_squared(snaps[j].pos) > radius2 {
                continue;
            }
            // Pick the single most valuable feasible trade across both directions.
            let mut best: Option<(usize, usize, usize, f32, f32)> = None; // buyer_i, seller_i, item, price, surplus
            for &(bi, si) in &[(i, j), (j, i)] {
                for item in 0..N_ITEMS {
                    // One whole unit changes hands, so the seller must hold a full unit
                    // — selling out of a fractional holding would drive inventory
                    // negative and corrupt the running cost basis.
                    if snaps[si].inv[item] < 1.0 {
                        continue;
                    }
                    let local_ask = field.local_ask(goods, snaps[si].tile, item);
                    let price = seller_ask(local_ask, snaps[si].inv[item], snaps[si].cost_basis[item]);
                    let buyer_wtp = wtp(goods, item, &snaps[bi]);
                    let seller_res = reservation(goods, item, &snaps[si]);
                    if buyer_wtp >= price && seller_res < price && snaps[bi].bucks >= price {
                        let surplus = buyer_wtp - price;
                        if best.is_none_or(|(_, _, _, _, s)| surplus > s) {
                            best = Some((bi, si, item, price, surplus));
                        }
                    }
                }
            }

            if let Some((bi, si, item, price, _)) = best {
                txs.push(Tx {
                    buyer: snaps[bi].e,
                    seller: snaps[si].e,
                    item,
                    price,
                });
                // Reflect in the snapshot so balances stay consistent this frame.
                snaps[bi].bucks -= price;
                snaps[bi].inv[item] += 1.0;
                snaps[si].bucks += price;
                snaps[si].inv[item] -= 1.0;
                stats.trades_total += 1;
                stats.gdp_window += price;
                stats.gdp_total += price as f64;
                stats.haul_window += field.source_dist(snaps[si].tile, item / 2) as f64;
                stats.haul_count_window += 1;
                // Spatial trade heatmap: tally the sale at the seller's tile (where the
                // price clears). Sized lazily so fresh and loaded games both work.
                let n_tiles = (cols * sim.0.rows) as usize;
                if stats.trade_hexes.len() != n_tiles {
                    stats.trade_hexes = vec![0; n_tiles];
                }
                stats.trade_hexes[snaps[si].tile] += 1;
                // Last clearing price for this item — held until its next sale.
                stats.last_sale_price[item] = price;
                income.this_window += price as f64;
                // EWMA of realized sale prices (lazy-init to the first sample).
                stats.ewma_price[item] = if stats.ewma_price[item] <= 0.0 {
                    price
                } else {
                    stats.ewma_price[item] + PRICE_EWMA_ALPHA * (price - stats.ewma_price[item])
                };
            }
        }
    }

    // Apply to the ECS, one entity borrow at a time.
    for tx in txs {
        // Buyer side: average in the cost basis and grow more cautious.
        if let Ok((_, _, mut inv, mut wal, _, mut trader, mut meta)) = q.get_mut(tx.buyer) {
            let held_before = inv.items[tx.item];
            inv.items[tx.item] += 1.0;
            wal.bucks -= tx.price;
            let total = trader.cost_basis[tx.item] * held_before + tx.price;
            trader.cost_basis[tx.item] = total / (held_before + 1.0);
            trader.discount = (trader.discount - DISCOUNT_LR * (trader.discount - DISCOUNT_MIN))
                .max(DISCOUNT_MIN);
            meta.transactions += 1;
        }

        // Seller side: realized margin grows trade optimism; tally the profit stat.
        if let Ok((_, _, mut inv, mut wal, _, mut trader, mut meta)) = q.get_mut(tx.seller) {
            inv.items[tx.item] -= 1.0;
            wal.bucks += tx.price;
            let margin = tx.price - trader.cost_basis[tx.item];
            if margin > 0.0 {
                trader.discount = (trader.discount + DISCOUNT_LR * (DISCOUNT_MAX - trader.discount))
                    .min(DISCOUNT_MAX);
            }
            stats.merchant_profit_window += margin.max(0.0);
            stats.merchant_profit_total += margin.max(0.0) as f64;
            meta.transactions += 1;
        }
    }
}
