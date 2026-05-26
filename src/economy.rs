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
use crate::hex::{hex_center, neighbors};
use crate::noot::*;
use crate::world::terrain_factor;
use crate::policy::{self, ActorCritic, PolicyMemory, Trainer, Transition, N_ACT, N_OTHER};
use crate::{Sim, SimRng};
use serde::{Deserialize, Serialize};

/// Decision/step cadence: a noot picks a new action (and, if Move, steps one hex)
/// this often, scaled faster on easy terrain. This is the MDP timestep.
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

// --- Learning by doing ------------------------------------------------------
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

/// How long each production/consumption rate sample covers (seconds).
const RATE_WINDOW: f32 = 0.5;

#[derive(Resource, Default, Clone, Serialize, Deserialize)]
pub struct EconStats {
    pub trades_total: u64,
    /// Exponentially weighted moving average of actual sale prices, per item.
    pub ewma_price: [f32; N_ITEMS],
    /// Cumulative raw units extracted from deposits (the economy's supply).
    pub produced_total: f64,
    /// Cumulative units consumed (staples eaten + positional goods used up).
    pub consumed_total: f64,
    /// Cumulative bucks of margin realized by merchants reselling surplus.
    pub merchant_profit_total: f64,
    /// Cumulative welfare (utility) realized through consumption.
    pub utility_total: f64,
    /// Most recent windowed rates, in units (or utility/bucks) per second.
    pub production_rate: f32,
    pub consumption_rate: f32,
    pub merchant_profit_rate: f32,
    pub utility_rate: f32,
    // Accumulators for the in-progress rate window.
    produced_window: f32,
    consumed_window: f32,
    merchant_profit_window: f32,
    utility_window: f32,
    window_elapsed: f32,
}

/// Convert the running production/consumption/profit tallies into per-second
/// rates, once per `RATE_WINDOW` so the HUD numbers don't jitter every frame.
pub fn update_rates(time: Res<Time>, mut stats: ResMut<EconStats>) {
    stats.window_elapsed += time.delta_secs();
    if stats.window_elapsed >= RATE_WINDOW {
        let inv = 1.0 / stats.window_elapsed;
        stats.production_rate = stats.produced_window * inv;
        stats.consumption_rate = stats.consumed_window * inv;
        stats.merchant_profit_rate = stats.merchant_profit_window * inv;
        stats.utility_rate = stats.utility_window * inv;
        stats.produced_window = 0.0;
        stats.consumed_window = 0.0;
        stats.merchant_profit_window = 0.0;
        stats.utility_window = 0.0;
        stats.window_elapsed = 0.0;
    }
}

pub fn income(time: Res<Time>, ctrl: Res<IncomeControl>, mut wallets: Query<&mut Wallet>) {
    let d = ctrl.rate * time.delta_secs();
    for mut w in &mut wallets {
        w.bucks += d;
    }
}

// --- Universal-income controller (targets a tiny sales "inflation") ---------
/// Target growth in total sale value, minute over minute (0.1% / min).
pub const TARGET_INFLATION_PER_MIN: f32 = 0.001;
/// Measurement/control window — one minute, per the inflation definition.
const INCOME_WINDOW: f32 = 60.0;
/// Integral gain: `rate += INCOME_K * (target − measured)` each window.
const INCOME_K: f32 = 0.4;
/// EMA smoothing on the measured inflation (window-to-window sales are noisy).
const INCOME_MEAS_ALPHA: f32 = 0.5;
const INCOME_RATE_MIN: f32 = 0.0;
const INCOME_RATE_MAX: f32 = 3.0;

/// Trims the universal income so total trade value grows at roughly
/// `TARGET_INFLATION_PER_MIN`. Inflation is measured as this minute's summed sale
/// value vs. the previous minute's. `meet_and_trade` accumulates `this_window`.
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct IncomeControl {
    /// Current universal income (bucks/sec/noot) — what `income` pays out.
    pub rate: f32,
    /// Smoothed measured inflation (fractional sales growth per minute).
    pub measured_inflation: f32,
    /// Sale value (bucks) summed over the current window.
    pub this_window: f64,
    last_window: f64,
    have_prev: bool,
    elapsed: f32,
}

impl Default for IncomeControl {
    fn default() -> Self {
        Self {
            rate: BUCKS_INCOME,
            measured_inflation: TARGET_INFLATION_PER_MIN,
            this_window: 0.0,
            last_window: 0.0,
            have_prev: false,
            elapsed: 0.0,
        }
    }
}

pub fn income_controller(time: Res<Time>, mut ctrl: ResMut<IncomeControl>) {
    ctrl.elapsed += time.delta_secs();
    if ctrl.elapsed < INCOME_WINDOW {
        return;
    }
    let this = ctrl.this_window;
    // Need a previous (non-empty) minute to define growth; otherwise just rotate.
    if ctrl.have_prev && ctrl.last_window > 0.0 {
        let inflation = ((this - ctrl.last_window) / ctrl.last_window) as f32;
        ctrl.measured_inflation += INCOME_MEAS_ALPHA * (inflation - ctrl.measured_inflation);
        // Integral control: more income → more spending → faster sales growth, so
        // raise income when inflation is below target and cut it when above.
        let error = TARGET_INFLATION_PER_MIN - ctrl.measured_inflation;
        ctrl.rate = (ctrl.rate + INCOME_K * error).clamp(INCOME_RATE_MIN, INCOME_RATE_MAX);
    }
    ctrl.last_window = this;
    ctrl.have_prev = true;
    ctrl.this_window = 0.0;
    ctrl.elapsed = 0.0;
}

// --- Hunger-rate PID (targets a steady death rate) --------------------------
/// Starting appetite gained per second per staple, before the controller adjusts it.
pub const HUNGER_RATE_INIT: f32 = 0.5;
/// Target deaths per minute, as a fraction of the mortal (non-merchant) population.
pub const TARGET_DEATH_FRAC_PER_MIN: f32 = 0.02;
/// PID gains: error is in deaths/min, output is the hunger rate (appetite/sec).
const PID_KP: f32 = 0.05;
const PID_KI: f32 = 0.02;
const PID_KD: f32 = 0.01;
/// Control update interval (s). Deaths are rare, so we measure over a window.
const PID_PERIOD: f32 = 3.0;
/// EMA smoothing on the measured death rate — discrete deaths are noisy.
const PID_MEAS_ALPHA: f32 = 0.4;
const HUNGER_RATE_MIN: f32 = 0.05;
const HUNGER_RATE_MAX: f32 = 3.0;

/// PID controller that trims the global hunger rate so the realized death rate
/// tracks `target_per_min`. Deaths feed back via `deaths_since_update`, bumped by
/// `death_and_respawn`.
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct HungerControl {
    /// Current hunger rate (appetite/sec/staple) — what `hunger_tick` applies.
    pub rate: f32,
    pub target_per_min: f32,
    /// Smoothed measured death rate (deaths/min), for the readout and the error.
    pub measured_per_min: f32,
    /// Deaths counted since the last control update.
    pub deaths_since_update: u32,
    elapsed: f32,
    integral: f32,
    prev_error: f32,
}

impl HungerControl {
    pub fn new(target_per_min: f32) -> Self {
        Self {
            rate: HUNGER_RATE_INIT,
            target_per_min,
            // Seed the measurement at target and the integral at the initial rate so
            // the loop starts from today's behaviour and only trims from there.
            measured_per_min: target_per_min,
            deaths_since_update: 0,
            elapsed: 0.0,
            integral: HUNGER_RATE_INIT / PID_KI,
            prev_error: 0.0,
        }
    }
}

/// Once per `PID_PERIOD`, fold the window's death count into the smoothed rate and
/// step the PID, nudging the hunger rate up when too few are dying and down when
/// too many are.
pub fn hunger_pid(time: Res<Time>, mut ctrl: ResMut<HungerControl>) {
    ctrl.elapsed += time.delta_secs();
    if ctrl.elapsed < PID_PERIOD {
        return;
    }
    let dt = ctrl.elapsed;
    let raw = ctrl.deaths_since_update as f32 / dt * 60.0;
    ctrl.measured_per_min += PID_MEAS_ALPHA * (raw - ctrl.measured_per_min);

    let error = ctrl.target_per_min - ctrl.measured_per_min;
    // Anti-windup: keep the integral term inside the rate range.
    let (i_min, i_max) = (HUNGER_RATE_MIN / PID_KI, HUNGER_RATE_MAX / PID_KI);
    ctrl.integral = (ctrl.integral + error * dt).clamp(i_min, i_max);
    let derivative = (error - ctrl.prev_error) / dt;
    let output = PID_KP * error + PID_KI * ctrl.integral + PID_KD * derivative;
    ctrl.rate = output.clamp(HUNGER_RATE_MIN, HUNGER_RATE_MAX);

    ctrl.prev_error = error;
    ctrl.deaths_since_update = 0;
    ctrl.elapsed = 0.0;
}

/// Age every noot by the simulated time elapsed (frozen while paused).
pub fn age_noots(time: Res<Time>, mut q: Query<&mut NootMeta>) {
    let dt = time.delta_secs();
    for mut m in &mut q {
        m.age += dt;
    }
}

pub fn hunger_tick(time: Res<Time>, ctrl: Res<HungerControl>, mut q: Query<&mut Hunger>) {
    let d = ctrl.rate * time.delta_secs();
    for mut h in &mut q {
        for a in &mut h.staple {
            *a = (*a + d).min(STAPLE_SATIATION);
        }
    }
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
    let gate = |x: f32| ((x - 0.5) / 0.4).clamp(0.0, 1.0);
    W_PHYS * phys + W_SAFE * gate(phys) * safety + W_ESTEEM * gate(phys) * gate(safety) * esteem
}

/// Build the policy's non-positional feature vector and the tile index.
fn features(world: &crate::world::World, pos_idx: usize, hunger: &Hunger, inv: &Inventory, wallet: &Wallet, owns: bool) -> (usize, [f32; N_OTHER]) {
    let o = [
        (wallet.bucks / 100.0).tanh(),
        hunger.staple[0] / STAPLE_SATIATION,
        hunger.staple[1] / STAPLE_SATIATION,
        positional_utility(&world.goods, inv) / ESTEEM_NORM,
        if owns { 1.0 } else { 0.0 },
        1.0, // bias
    ];
    (pos_idx, o)
}

/// Which actions are valid: a move direction is valid iff that neighbour is in
/// bounds; Mine/Refine match the gates in `extract`/`refine`. (Trading is automatic,
/// not an action.) Directions are indexed identically to `hex::neighbors`.
fn action_mask(world: &crate::world::World, claim: &Claim, pos: &TilePos, inv: &Inventory) -> [bool; N_ACT] {
    let mut mask = [false; N_ACT];
    // The map is a torus, so every move direction is always valid.
    for m in mask.iter_mut().take(policy::N_DIRS) {
        *m = true;
    }
    mask[policy::A_MINE] = claim.deposit.is_some_and(|d| {
        let dep = &world.deposits[d];
        let t = &world.tiles[dep.tile];
        t.col == pos.col
            && t.row == pos.row
            && inv.items[goods::item_index(dep.element_slot, GoodForm::Raw)] < CARRY_CAP
    });
    mask[policy::A_REFINE] = claim.deposit.is_none()
        && (0..N_ITEMS).any(|i| {
            matches!(world.goods.role_of(i), ItemRole::Intermediate) && inv.items[i] > 0.0
        });
    mask
}

/// The learned decision step (replaces the heuristic). Once per noot per cadence:
/// observe the reward since the last decision, record the transition, pick a new
/// action from the shared policy, and — if Move — step toward the best-valued
/// neighbour hex. Between decisions the chosen action persists and the extract/
/// refine/trade systems apply it each frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn policy_step(
    time: Res<Time>,
    sim: Res<Sim>,
    ac: Res<ActorCritic>,
    mut trainer: ResMut<Trainer>,
    mut rng: ResMut<SimRng>,
    mut q: Query<(
        &mut TilePos,
        &Claim,
        &Inventory,
        &Hunger,
        &Wallet,
        &mut Action,
        &mut PolicyMemory,
    )>,
) {
    let world = &sim.0;
    let dt = time.delta_secs();
    for (mut pos, claim, inv, hunger, wallet, mut action, mut mem) in &mut q {
        mem.cooldown -= dt;
        if mem.cooldown > 0.0 && !mem.died {
            continue;
        }
        let owns = claim.deposit.is_some();
        let pos_idx = (pos.row * world.cols + pos.col) as usize;
        let (s_pos, s_o) = features(world, pos_idx, hunger, inv, wallet, owns);
        let mask = action_mask(world, claim, &pos, inv);
        let u_now = maslow_utility(hunger, inv, wallet, &world.goods);

        // Close out the previous transition (reward = ΔU, or a death penalty).
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

        // Choose the next action: masked softmax, with an ε chance of a random valid
        // action (the per-noot exploration temperament).
        let act = if rng.0.chance(mem.explore) {
            let valid: Vec<usize> = (0..N_ACT).filter(|&a| mask[a]).collect();
            valid[rng.0.below(valid.len())]
        } else {
            let logits = ac.logits(s_pos, &s_o);
            let probs = policy::masked_softmax(&logits, &mask);
            policy::sample(&probs, &mut rng.0)
        };

        // Indices 0..6 are relative move directions (step to that neighbour hex);
        // the rest are Mine/Refine in place.
        *action = if act == policy::A_MINE {
            Action::Mine
        } else if act == policy::A_REFINE {
            Action::Refine
        } else {
            let (nc, nr) = neighbors(pos.col, pos.row, world.cols, world.rows)[act];
            pos.col = nc;
            pos.row = nr;
            Action::Move
        };

        // Cache this state/action for the next transition.
        mem.last_pos = s_pos;
        mem.last_o = s_o;
        mem.last_mask = mask;
        mem.last_act = act;
        mem.last_u = u_now;
        mem.has_prev = true;
        let tf = terrain_factor(world.tiles[pos_idx].difficulty);
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

/// Advance the resource simulation (deposit regrowth) by the frame's dt.
pub fn simulate(time: Res<Time>, mut sim: ResMut<Sim>) {
    sim.0.tick(time.delta_secs());
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
    time: Res<Time>,
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
        &mut Claim,
        &mut TilePos,
    )>,
) {
    let dt = time.delta_secs();
    let world = &sim.0;
    for (mut hunger, mut inv, mut wallet, mut mem, mut trader, mut meta, mut claim, mut pos) in
        &mut q
    {
        if hunger.fully_starving() {
            hunger.starving_secs += dt;
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
        *trader = Trader::new();
        *meta = NootMeta::new();
        claim.deposit = None;
        pos.col = rng.0.below(world.cols as usize) as i32;
        pos.row = rng.0.below(world.rows as usize) as i32;
    }
}

pub fn extract(
    time: Res<Time>,
    mut sim: ResMut<Sim>,
    mut stats: ResMut<EconStats>,
    mut q: Query<(&Action, &Claim, &TilePos, &mut Inventory, &mut NootMeta)>,
) {
    let dt = time.delta_secs();
    for (action, claim, pos, mut inv, mut meta) in &mut q {
        if *action != Action::Mine {
            continue;
        }
        let Some(deposit) = claim.deposit else {
            continue;
        };
        let dtile = sim.0.deposits[deposit].tile;
        let (dc, dr) = (sim.0.tiles[dtile].col, sim.0.tiles[dtile].row);
        if pos.col != dc || pos.row != dr {
            continue;
        }
        let slot = sim.0.deposits[deposit].element_slot;
        let raw = goods::item_index(slot, GoodForm::Raw);
        if inv.items[raw] >= CARRY_CAP {
            continue;
        }
        // Learning by doing: a seasoned miner pulls more per second.
        let rate = WORK_RATE * skill_factor(meta.experience);
        let got = sim.0.extract_from(deposit, rate, dt) as f32;
        inv.items[raw] += got;
        meta.experience += got;
        stats.produced_window += got;
        stats.produced_total += got as f64;
    }
}

/// A noot with no claim that's standing on an unclaimed deposit claims it. Claims
/// are sticky (first one kept) and tracked solely by the `Claim` components, so a
/// claim frees up automatically when its holder dies and resets to `None`.
pub fn claim_deposits(sim: Res<Sim>, mut q: Query<(&TilePos, &mut Claim)>) {
    let mut taken = vec![false; sim.0.deposits.len()];
    for (_, claim) in &q {
        if let Some(d) = claim.deposit {
            taken[d] = true;
        }
    }
    for (pos, mut claim) in &mut q {
        if claim.deposit.is_some() {
            continue;
        }
        let idx = (pos.row * sim.0.cols + pos.col) as usize;
        if let Some(d) = sim.0.tiles[idx].deposit {
            if !taken[d] {
                claim.deposit = Some(d);
                taken[d] = true;
            }
        }
    }
}

/// Refine held intermediates — but only for noots whose chosen action this tick is
/// `Refine` (set by `choose_action`), faster the more refining experience accrued.
pub fn refine(
    time: Res<Time>,
    sim: Res<Sim>,
    mut q: Query<(&Action, &mut Inventory, &mut NootMeta)>,
) {
    let dt = time.delta_secs();
    for (action, mut inv, mut meta) in &mut q {
        if *action != Action::Refine {
            continue;
        }
        let rate = REFINE_RATE * skill_factor(meta.experience);
        for slot in 0..4 {
            let raw = goods::item_index(slot, GoodForm::Raw);
            if sim.0.goods.role_of(raw) != ItemRole::Intermediate || inv.items[raw] <= 0.0 {
                continue;
            }
            let refined = goods::item_index(slot, GoodForm::Refined);
            let amount = (rate * dt).min(inv.items[raw]);
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

/// The price a noot offers one unit at: its surplus-discounted ask, but never
/// below its cost basis — so freshly mined goods (cost ≈ 0) dump cheap when
/// glutted, while goods bought to flip won't be resold at a loss.
fn seller_ask(goods: &goods::WorldGoods, item: usize, held: f32, cost_basis: f32) -> f32 {
    (base_ask(goods, item) * surplus_discount(held)).max(cost_basis)
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
                    if snaps[si].inv[item] <= 0.0 {
                        continue;
                    }
                    let price =
                        seller_ask(goods, item, snaps[si].inv[item], snaps[si].cost_basis[item]);
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
