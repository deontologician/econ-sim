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
const POSITIONAL_CONSUME_RATE: f32 = 1.5;

// Asking prices (bucks) by item kind.
const ASK_RAW_CONSUMABLE: f32 = 6.0;
const ASK_INTERMEDIATE: f32 = 5.0;
const ASK_REFINED_CONSUMABLE: f32 = 12.0;

// Valuations (bucks).
const REFINER_WTP_INTERMEDIATE: f32 = 8.0;
const STAPLE_VALUE: f32 = 20.0; // WTP when starving
const POSITIONAL_VALUE: f32 = 40.0; // first-unit WTP, then /(1+stock)

const TRADE_RADIUS_FACTOR: f32 = 1.7; // × hex_size

#[derive(Resource, Default)]
pub struct EconStats {
    pub trades_total: u64,
    pub last_price: [f32; N_ITEMS],
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

pub fn consume(sim: Res<Sim>, mut q: Query<(&mut Inventory, &mut Hunger, &mut Positional)>) {
    let dt_goods = &sim.0.goods;
    for (mut inv, mut hunger, mut positional) in &mut q {
        // Staples first (satisficing: eat only to satiation, surplus unused).
        for item in 0..N_ITEMS {
            if let ItemRole::Staple(sub) = dt_goods.role_of(item) {
                if inv.items[item] > 0.0 && hunger.staple[sub] > 0.0 {
                    let needed = hunger.staple[sub] / EAT_VALUE;
                    let eat = inv.items[item].min(needed);
                    inv.items[item] -= eat;
                    hunger.staple[sub] = (hunger.staple[sub] - eat * EAT_VALUE).max(0.0);
                }
            }
        }
        // Positional goods only once staples are satisfied.
        if hunger.satisfied() {
            for item in 0..N_ITEMS {
                if let ItemRole::Positional(sub) = dt_goods.role_of(item) {
                    if inv.items[item] > 0.0 {
                        let c = inv.items[item].min(POSITIONAL_CONSUME_RATE);
                        inv.items[item] -= c;
                        positional.stock[sub] += c;
                    }
                }
            }
        }
    }
}

struct Snap {
    e: Entity,
    pos: Vec2,
    role: Role,
    inv: [f32; N_ITEMS],
    bucks: f32,
    hunger: [f32; N_STAPLES],
    satisfied: bool,
    positional: [f32; N_POSITIONAL],
}

fn wtp(goods: &goods::WorldGoods, item: usize, s: &Snap) -> f32 {
    match goods.role_of(item) {
        ItemRole::Staple(sub) => STAPLE_VALUE * (s.hunger[sub] / STAPLE_SATIATION),
        ItemRole::Positional(sub) => {
            if s.satisfied {
                POSITIONAL_VALUE / (1.0 + s.positional[sub])
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
        &Positional,
        &mut Brain,
    )>,
) {
    let goods = &sim.0.goods;
    let radius2 = (sim.0.hex_size * TRADE_RADIUS_FACTOR).powi(2);

    // Snapshot (immutable read) so we can reason about pairs without aliasing.
    let mut snaps: Vec<Snap> = q
        .iter()
        .map(|(e, t, role, inv, wal, hunger, pos, _brain)| Snap {
            e,
            pos: t.translation.truncate(),
            role: *role,
            inv: inv.items,
            bucks: wal.bucks,
            hunger: hunger.staple,
            satisfied: hunger.satisfied(),
            positional: pos.stock,
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
                    let seller_wtp = wtp(goods, item, &snaps[si]);
                    if buyer_wtp >= price && seller_wtp < price && snaps[bi].bucks >= price {
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
        if let Ok((_, _, _, mut inv, mut wal, _, _, _)) = q.get_mut(tx.buyer) {
            inv.items[tx.item] += 1.0;
            wal.bucks -= tx.price;
        }
        if let Ok((_, _, _, mut inv, mut wal, _, _, mut brain)) = q.get_mut(tx.seller) {
            inv.items[tx.item] -= 1.0;
            wal.bucks += tx.price;
            brain.sold_this_trip = true;
        }
    }
}
