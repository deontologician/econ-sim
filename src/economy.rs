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
    /// Cumulative units picked up by transporters for hauling.
    pub hauled_total: f64,
    /// Cumulative welfare (utility) realized through consumption.
    pub utility_total: f64,
    /// Most recent windowed rates, in units (or utility) per second.
    pub production_rate: f32,
    pub consumption_rate: f32,
    pub hauled_rate: f32,
    pub utility_rate: f32,
    // Accumulators for the in-progress rate window.
    produced_window: f32,
    consumed_window: f32,
    hauled_window: f32,
    utility_window: f32,
    window_elapsed: f32,
}

/// Convert the running production/consumption/haul tallies into per-second
/// rates, once per `RATE_WINDOW` so the HUD numbers don't jitter every frame.
pub fn update_rates(time: Res<Time>, mut stats: ResMut<EconStats>) {
    stats.window_elapsed += time.delta_secs();
    if stats.window_elapsed >= RATE_WINDOW {
        let inv = 1.0 / stats.window_elapsed;
        stats.production_rate = stats.produced_window * inv;
        stats.consumption_rate = stats.consumed_window * inv;
        stats.hauled_rate = stats.hauled_window * inv;
        stats.utility_rate = stats.utility_window * inv;
        stats.produced_window = 0.0;
        stats.consumed_window = 0.0;
        stats.hauled_window = 0.0;
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

pub fn hunger_tick(time: Res<Time>, mut q: Query<&mut Hunger>) {
    let d = HUNGER_RATE * time.delta_secs();
    for mut h in &mut q {
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
    mut q: Query<(&Role, &mut Inventory, &mut Hunger, &mut Brain)>,
) {
    let dt_goods = &sim.0.goods;
    let mut eaten = 0.0f32;
    let mut utility_gained = 0.0f32;
    for (role, mut inv, mut hunger, mut brain) in &mut q {
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
        brain.trip_reward += reward;
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
}

/// What a noot is willing to *pay* to acquire one unit (buyer side).
fn wtp(goods: &goods::WorldGoods, item: usize, s: &Snap) -> f32 {
    // Transporters are pure logistics agents: they haul and sell, never buy.
    if matches!(s.role, Role::Transporter) {
        return 0.0;
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
    // Transporters never want their cargo; they sell whatever they carry.
    if matches!(s.role, Role::Transporter) {
        return 0.0;
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

fn ask(goods: &goods::WorldGoods, item: usize) -> f32 {
    match goods.role_of(item) {
        ItemRole::Intermediate => ASK_INTERMEDIATE,
        ItemRole::Staple(_) | ItemRole::Positional(_) => match form_of(item) {
            GoodForm::Raw => ASK_RAW_CONSUMABLE,
            GoodForm::Refined => ASK_REFINED_CONSUMABLE,
        },
        ItemRole::Junk => f32::MAX,
    }
}

struct Tx {
    buyer: Entity,
    seller: Entity,
    item: usize,
    price: f32,
}

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
    )>,
    // Separate query (HaulContract isn't in the tuple above, so no conflict):
    // record a hauling seller's revenue so it can settle the owner's share.
    mut contracts: Query<&mut HaulContract>,
) {
    let goods = &sim.0.goods;
    let radius2 = (sim.0.hex_size * TRADE_RADIUS_FACTOR).powi(2);

    // Snapshot (immutable read) so we can reason about pairs without aliasing.
    let mut snaps: Vec<Snap> = q
        .iter()
        .map(|(e, t, role, inv, wal, hunger)| Snap {
            e,
            pos: t.translation.truncate(),
            role: *role,
            inv: inv.items,
            bucks: wal.bucks,
            hunger: hunger.staple,
            satisfied: hunger.satisfied(),
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
                    let price = ask(goods, item);
                    let buyer_wtp = wtp(goods, item, &snaps[bi]);
                    let seller_res = reservation(goods, item, &snaps[si]);
                    if buyer_wtp >= price && seller_res < price && snaps[bi].bucks >= price {
                        let surplus = buyer_wtp - price;
                        if best.map_or(true, |(_, _, _, _, s)| surplus > s) {
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
        if let Ok((_, _, _, mut inv, mut wal, _)) = q.get_mut(tx.buyer) {
            inv.items[tx.item] += 1.0;
            wal.bucks -= tx.price;
        }
        if let Ok((_, _, _, mut inv, mut wal, _)) = q.get_mut(tx.seller) {
            inv.items[tx.item] -= 1.0;
            wal.bucks += tx.price;
        }
        // If the seller is a transporter mid-haul, tally the take so the owner's
        // share can be paid back on settlement. (Borrow is separate from `q`.)
        if let Ok(mut contract) = contracts.get_mut(tx.seller) {
            if matches!(contract.state, HaulState::Selling | HaulState::Returning) {
                contract.proceeds += tx.price;
            }
        }
    }
}

/// Match each idle transporter to the owner most in need of hauling (the one
/// carrying the most raw, above `MIN_HIRE`). v1 stub for a real labor market:
/// each owner is claimed by at most one transporter per assignment pass.
pub fn haul_assign(
    sim: Res<Sim>,
    mut transporters: Query<(&TilePos, &mut HaulContract)>,
    owners: Query<(Entity, &Role, &Inventory)>,
) {
    // Pass 1: collect owners already being served so we don't double-book.
    let mut claimed: Vec<Entity> = Vec::new();
    let mut any_idle = false;
    for (_, contract) in &transporters {
        match contract.employer {
            Some(e) if contract.state != HaulState::Idle => claimed.push(e),
            _ => any_idle = true,
        }
    }
    if !any_idle {
        return;
    }

    // Candidate owners with a worthwhile load, richest first.
    let mut candidates: Vec<(Entity, usize, f32)> = Vec::new(); // owner, deposit, raw
    for (e, role, inv) in &owners {
        let Role::Owner { deposit } = *role else {
            continue;
        };
        let slot = sim.0.deposits[deposit].element_slot;
        let raw = goods::item_index(slot, GoodForm::Raw);
        if inv.items[raw] >= MIN_HIRE {
            candidates.push((e, deposit, inv.items[raw]));
        }
    }
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Pass 2: hand each idle transporter the best unclaimed owner.
    for (_, mut contract) in &mut transporters {
        if contract.state != HaulState::Idle {
            continue;
        }
        let Some(&(owner, deposit, _)) = candidates.iter().find(|(e, _, _)| !claimed.contains(e))
        else {
            break; // no owners left to serve this pass
        };
        let slot = sim.0.deposits[deposit].element_slot;
        contract.state = HaulState::ToPickup;
        contract.employer = Some(owner);
        contract.deposit = deposit;
        contract.cargo_item = goods::item_index(slot, GoodForm::Raw);
        contract.proceeds = 0.0;
        contract.sell_steps = 0;
        claimed.push(owner);
    }
}

/// A transporter standing on its employer's deposit loads cargo from the
/// owner's inventory, then sets off to sell.
pub fn haul_loading(
    sim: Res<Sim>,
    mut stats: ResMut<EconStats>,
    mut transporters: Query<(Entity, &TilePos, &mut HaulContract)>,
    mut invs: Query<&mut Inventory>,
) {
    for (te, pos, mut contract) in &mut transporters {
        if contract.state != HaulState::Loading {
            continue;
        }
        let Some(employer) = contract.employer else {
            contract.state = HaulState::Idle;
            continue;
        };
        let dtile = sim.0.deposits[contract.deposit].tile;
        if pos.col != sim.0.tiles[dtile].col || pos.row != sim.0.tiles[dtile].row {
            continue;
        }
        // Two distinct entities → simultaneous mutable borrows are safe.
        let mut loaded = 0.0f32;
        if let Ok([mut t_inv, mut e_inv]) = invs.get_many_mut([te, employer]) {
            let take = e_inv.items[contract.cargo_item].min(HAUL_CAPACITY);
            e_inv.items[contract.cargo_item] -= take;
            t_inv.items[contract.cargo_item] += take;
            loaded = take;
        }
        stats.hauled_window += loaded;
        stats.hauled_total += loaded as f64;
        // With cargo, go sell; empty-handed, head straight back to settle.
        contract.sell_steps = 0;
        contract.state = if loaded > 0.0 {
            HaulState::Selling
        } else {
            HaulState::Returning
        };
    }
}

/// A returning transporter on its employer's deposit pays the owner's share of
/// the take, returns any unsold cargo, and goes idle (ready to be rehired).
pub fn haul_settle(
    sim: Res<Sim>,
    mut transporters: Query<(Entity, &TilePos, &mut HaulContract)>,
    mut wallets: Query<&mut Wallet>,
    mut invs: Query<&mut Inventory>,
) {
    for (te, pos, mut contract) in &mut transporters {
        if contract.state != HaulState::Returning {
            continue;
        }
        let Some(employer) = contract.employer else {
            *contract = HaulContract::idle();
            continue;
        };
        let dtile = sim.0.deposits[contract.deposit].tile;
        if pos.col != sim.0.tiles[dtile].col || pos.row != sim.0.tiles[dtile].row {
            continue;
        }

        let payout = contract.proceeds * PRINCIPAL_SHARE;
        if let Ok([mut t_w, mut e_w]) = wallets.get_many_mut([te, employer]) {
            let pay = payout.min(t_w.bucks.max(0.0));
            t_w.bucks -= pay;
            e_w.bucks += pay;
        }
        if let Ok([mut t_i, mut e_i]) = invs.get_many_mut([te, employer]) {
            let left = t_i.items[contract.cargo_item];
            if left > 0.0 {
                t_i.items[contract.cargo_item] -= left;
                e_i.items[contract.cargo_item] += left;
            }
        }
        *contract = HaulContract::idle();
    }
}
