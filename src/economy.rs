//! The economic loop: owners extract, refiners refine, noots meet and trade for
//! bucks, and everyone consumes what they value.
//!
//! Pricing is intentionally simple for v1 (see plans/INTENDED_FEATURES.md): a
//! trade clears at the **seller's fixed ask** when the buyer's willingness-to-pay
//! meets it, the seller values the item less than the ask (so it's surplus to
//! them), and the buyer is solvent. One unit per meeting.

use bevy::prelude::*;

use crate::goods::{self, form_of, GoodForm, ItemRole, N_ITEMS};
use crate::noot::*;
use crate::Sim;

// Production rates.
pub const OWNER_WORK_RATE: f32 = 3.0;
pub const REFINE_RATE: f32 = 2.0;

// STUB: universal income so consumers don't go broke. See INTENDED_FEATURES.md.
pub const BUCKS_INCOME: f32 = 0.6;

// Consumption.
const EAT_VALUE: f32 = 4.0; // appetite removed per staple unit eaten

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

/// Selling income is scaled to roughly staple-welfare magnitude before it feeds
/// the movement reward, so a sale and a meal pull the value field comparably.
const SELL_REWARD_SCALE: f32 = 0.15;

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

/// How long each production/consumption rate sample covers (seconds).
const RATE_WINDOW: f32 = 0.5;

#[derive(Resource, Default)]
pub struct EconStats {
    pub trades_total: u64,
    pub last_price: [f32; N_ITEMS],
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

pub fn income(time: Res<Time>, mut wallets: Query<&mut Wallet>) {
    let d = BUCKS_INCOME * time.delta_secs();
    for mut w in &mut wallets {
        w.bucks += d;
    }
}

pub fn hunger_tick(time: Res<Time>, mut q: Query<(&Role, &mut Hunger)>) {
    let d = HUNGER_RATE * time.delta_secs();
    for (role, mut h) in &mut q {
        // Merchants don't eat, so they don't get hungry (and never starve).
        if matches!(role, Role::Transporter) {
            continue;
        }
        for a in &mut h.staple {
            *a = (*a + d).min(STAPLE_SATIATION);
        }
    }
}

pub fn extract(
    time: Res<Time>,
    mut sim: ResMut<Sim>,
    mut stats: ResMut<EconStats>,
    mut q: Query<(&Role, &TilePos, &mut Inventory)>,
) {
    let dt = time.delta_secs();
    for (role, pos, mut inv) in &mut q {
        let Role::Owner { deposit } = *role else {
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
        let got = sim.0.extract_from(deposit, OWNER_WORK_RATE, dt) as f32;
        inv.items[raw] += got;
        stats.produced_window += got;
        stats.produced_total += got as f64;
    }
}

pub fn refine(time: Res<Time>, sim: Res<Sim>, mut q: Query<(&Role, &mut Inventory)>) {
    let dt = time.delta_secs();
    for (role, mut inv) in &mut q {
        if *role != Role::Refiner {
            continue;
        }
        for slot in 0..4 {
            let raw = goods::item_index(slot, GoodForm::Raw);
            if sim.0.goods.role_of(raw) != ItemRole::Intermediate || inv.items[raw] <= 0.0 {
                continue;
            }
            let refined = goods::item_index(slot, GoodForm::Refined);
            let amount = (REFINE_RATE * dt).min(inv.items[raw]);
            inv.items[raw] -= amount;
            inv.items[refined] += amount;
        }
    }
}

pub fn consume(
    sim: Res<Sim>,
    mut stats: ResMut<EconStats>,
    mut q: Query<(&Role, &mut Inventory, &mut Hunger, &mut RouteMemory)>,
) {
    let dt_goods = &sim.0.goods;
    let mut eaten = 0.0f32;
    let mut utility_gained = 0.0f32;
    for (role, mut inv, mut hunger, mut mem) in &mut q {
        // Transporters carry goods for others; they don't eat the cargo.
        if matches!(role, Role::Transporter) {
            continue;
        }
        let mut reward = 0.0f32;
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
                    // Welfare from no longer being hungry.
                    reward += (eat * EAT_VALUE) / STAPLE_SATIATION;
                }
            }
        }
        mem.pending_reward += reward;
        utility_gained += reward;
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
    role: Role,
    inv: [f32; N_ITEMS],
    bucks: f32,
    hunger: [f32; N_STAPLES],
    satisfied: bool,
    /// Merchants only: learned discount on anticipated resale, and average price
    /// paid per held item. Zeroed for everyone else.
    discount: f32,
    cost_basis: [f32; N_ITEMS],
}

/// What a noot is willing to *pay* to acquire one unit (buyer side).
fn wtp(goods: &goods::WorldGoods, item: usize, s: &Snap) -> f32 {
    // Merchants buy surplus to resell: they'll pay up to a learned discount on the
    // good's market ask (their anticipated resale value).
    if matches!(s.role, Role::Transporter) {
        return match goods.role_of(item) {
            ItemRole::Junk => 0.0,
            _ => s.discount * base_ask(goods, item),
        };
    }
    match goods.role_of(item) {
        ItemRole::Staple(sub) => STAPLE_VALUE * (s.hunger[sub] / STAPLE_SATIATION),
        // Durable luxuries are only bought once fed; marginal worth falls as the
        // noot's *held* stock of that good grows.
        ItemRole::Positional(_) => {
            if s.satisfied {
                POSITIONAL_VALUE / (1.0 + s.inv[item])
            } else {
                0.0
            }
        }
        ItemRole::Intermediate => {
            if matches!(s.role, Role::Refiner) {
                REFINER_WTP_INTERMEDIATE
            } else {
                0.0
            }
        }
        ItemRole::Junk => 0.0,
    }
}

/// The lowest price at which a noot will *part with* one unit it holds (seller
/// side). For durable positional goods this is the marginal worth of keeping it
/// (which shrinks as holdings grow), discounted by hunger — a starving, goods-rich
/// noot sells cheaply to buy food, while a fed one sheds only its surplus.
fn reservation(goods: &goods::WorldGoods, item: usize, s: &Snap) -> f32 {
    // A merchant won't sell below what it paid: its cost basis is the floor, so
    // every completed resale clears at a non-negative margin.
    if matches!(s.role, Role::Transporter) {
        return s.cost_basis[item];
    }
    match goods.role_of(item) {
        ItemRole::Staple(sub) => STAPLE_VALUE * (s.hunger[sub] / STAPLE_SATIATION),
        ItemRole::Positional(_) => {
            let marginal = POSITIONAL_VALUE / (1.0 + s.inv[item]);
            let hunger_frac = s
                .hunger
                .iter()
                .copied()
                .fold(0.0f32, f32::max)
                / STAPLE_SATIATION;
            marginal * (1.0 - POSITIONAL_SELL_URGENCY * hunger_frac).max(0.0)
        }
        ItemRole::Intermediate => {
            if matches!(s.role, Role::Refiner) {
                REFINER_WTP_INTERMEDIATE
            } else {
                0.0
            }
        }
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
/// more a producer is drowning in its own good, the cheaper it dumps the surplus.
fn surplus_discount(held: f32) -> f32 {
    if held <= SURPLUS_FREE {
        1.0
    } else {
        (1.0 / (1.0 + SURPLUS_K * (held - SURPLUS_FREE))).max(SURPLUS_FLOOR)
    }
}

/// The price a seller offers one unit at. Merchants charge the full market ask
/// (their margin came from buying cheap); producers discount surplus they're
/// glutted on, opening the spread merchants live off.
fn seller_ask(goods: &goods::WorldGoods, item: usize, role: Role, held: f32) -> f32 {
    let base = base_ask(goods, item);
    if matches!(role, Role::Transporter) {
        base
    } else {
        base * surplus_discount(held)
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
    mut stats: ResMut<EconStats>,
    mut q: Query<(
        Entity,
        &Transform,
        &Role,
        &mut Inventory,
        &mut Wallet,
        &Hunger,
        &mut RouteMemory,
        Option<&mut Merchant>,
    )>,
) {
    let goods = &sim.0.goods;
    let radius2 = (sim.0.hex_size * TRADE_RADIUS_FACTOR).powi(2);

    // Snapshot (immutable read) so we can reason about pairs without aliasing.
    let mut snaps: Vec<Snap> = q
        .iter()
        .map(|(e, t, role, inv, wal, hunger, _route, merchant)| {
            let (discount, cost_basis) =
                merchant.map_or((0.0, [0.0; N_ITEMS]), |m| (m.discount, m.cost_basis));
            Snap {
                e,
                pos: t.translation.truncate(),
                role: *role,
                inv: inv.items,
                bucks: wal.bucks,
                hunger: hunger.staple,
                satisfied: hunger.satisfied(),
                discount,
                cost_basis,
            }
        })
        .collect();

    let mut txs: Vec<Tx> = Vec::new();

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
                    let price = seller_ask(goods, item, snaps[si].role, snaps[si].inv[item]);
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
                stats.last_price[item] = price;
            }
        }
    }

    // Apply to the ECS, one entity borrow at a time.
    for tx in txs {
        let base = base_ask(goods, tx.item);

        // Buyer side.
        if let Ok((_, _, _, mut inv, mut wal, _, mut route, merchant)) = q.get_mut(tx.buyer) {
            let held_before = inv.items[tx.item];
            inv.items[tx.item] += 1.0;
            wal.bucks -= tx.price;
            if let Some(mut m) = merchant {
                // Merchant acquiring surplus: bank the discounted anticipated
                // profit where it was found (so the field learns where surplus is),
                // average in the cost, and grow more cautious (discount → MIN).
                route.pending_reward += SELL_REWARD_SCALE * m.discount * (base - tx.price).max(0.0);
                let total = m.cost_basis[tx.item] * held_before + tx.price;
                m.cost_basis[tx.item] = total / (held_before + 1.0);
                m.discount = (m.discount - DISCOUNT_LR * (m.discount - DISCOUNT_MIN)).max(DISCOUNT_MIN);
            }
        }

        // Seller side.
        if let Ok((_, _, _, mut inv, mut wal, _, mut route, merchant)) = q.get_mut(tx.seller) {
            inv.items[tx.item] -= 1.0;
            wal.bucks += tx.price;
            match merchant {
                Some(mut m) => {
                    // Merchant realizing a resale: reward the actual margin where
                    // the buyer was, and let success breed optimism (discount → MAX).
                    let margin = tx.price - m.cost_basis[tx.item];
                    route.pending_reward += SELL_REWARD_SCALE * margin;
                    m.discount = (m.discount + DISCOUNT_LR * (DISCOUNT_MAX - m.discount)).min(DISCOUNT_MAX);
                    stats.merchant_profit_window += margin.max(0.0);
                    stats.merchant_profit_total += margin.max(0.0) as f64;
                }
                // Ordinary sellers learn where buyers are from the income itself.
                None => route.pending_reward += tx.price * SELL_REWARD_SCALE,
            }
        }
    }
}
