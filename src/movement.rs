//! Noot locomotion + the simple reinforcement loop.
//!
//! Outbound: step hex-to-hex, biased toward a learned `heading`, for `TRIP_LEN`
//! steps. Then return home greedily. On arrival, reinforce the heading if the
//! trip produced a sale, then pick the next heading ε-greedily.

use bevy::prelude::*;

use crate::goods::{self, GoodForm};
use crate::hex::{hex_center, neighbors};
use crate::noot::{Brain, HaulContract, HaulState, Home, Inventory, Role, TilePos, HAUL_SELL_STEPS};
use crate::world::{terrain_factor, World};
use crate::{MapView, Sim, SimRng};

const TRIP_LEN: u32 = 8;
const BASE_STEP_TIME: f32 = 0.35;
const HEADING_BIAS: f32 = 0.7;
const EPSILON: f32 = 0.15;
const WEIGHT_DECAY: f32 = 0.98;
/// Cap on a single trip's reinforcement so weights stay bounded.
const REWARD_CAP: f32 = 3.0;
/// Owners stay on their deposit, extracting, until they carry this much to sell.
const LOAD_THRESHOLD: f32 = 6.0;

pub fn tile_to_pixel(col: i32, row: i32, hex_size: f32, offset: Vec2) -> Vec2 {
    let (x, y) = hex_center(col, row, hex_size);
    Vec2::new(x + offset.x, y + offset.y)
}

fn in_bounds(world: &World, c: i32, r: i32) -> bool {
    c >= 0 && r >= 0 && c < world.cols && r < world.rows
}

fn tile_idx(world: &World, c: i32, r: i32) -> usize {
    (r * world.cols + c) as usize
}

pub fn movement(
    time: Res<Time>,
    mut rng: ResMut<SimRng>,
    sim: Res<Sim>,
    view: Res<MapView>,
    // Transporters move under `haul_movement`; exclude them here so the two
    // systems' `&mut Transform`/`&mut TilePos` accesses stay provably disjoint.
    mut noots: Query<
        (
            &mut TilePos,
            &Home,
            &mut Brain,
            &mut Transform,
            &Role,
            &Inventory,
        ),
        Without<HaulContract>,
    >,
) {
    let world = &sim.0;
    let dt = time.delta_secs();

    for (mut pos, home, mut brain, mut transform, role, inv) in &mut noots {
        // Smoothly glide the sprite toward the current tile centre.
        let target = tile_to_pixel(pos.col, pos.row, world.hex_size, view.offset);
        let t = (dt * 8.0).min(1.0);
        transform.translation.x += (target.x - transform.translation.x) * t;
        transform.translation.y += (target.y - transform.translation.y) * t;

        brain.move_cooldown -= dt;
        if brain.move_cooldown > 0.0 {
            continue;
        }

        if brain.outbound {
            // Owners linger on their deposit (the `extract` system runs while
            // they wait) until they have a load worth touring with.
            let at_home = pos.col == home.col && pos.row == home.row;
            if brain.trip_step == 0 && at_home && !ready_to_depart(world, role, inv) {
                brain.move_cooldown = BASE_STEP_TIME;
                continue;
            }
            let (c, r) = choose_outbound(&mut rng.0, world, pos.col, pos.row, brain.heading);
            pos.col = c;
            pos.row = r;
            brain.trip_step += 1;
            if brain.trip_step >= TRIP_LEN {
                brain.outbound = false;
            }
        } else if pos.col == home.col && pos.row == home.row {
            reinforce_and_reset(&mut brain, &mut rng.0);
        } else {
            let (c, r) = step_toward(world, pos.col, pos.row, home.col, home.row);
            pos.col = c;
            pos.row = r;
        }

        // Difficult terrain makes each step take longer.
        let tf = terrain_factor(world.tiles[tile_idx(world, pos.col, pos.row)].terrain);
        brain.move_cooldown = BASE_STEP_TIME / tf;
    }
}

/// Transporters navigate by contract state rather than the learned-heading
/// wander: walk to the employer's deposit, then (once loaded) wander selling,
/// then walk back to settle. Reuses `step_toward`/`choose_outbound`.
pub fn haul_movement(
    time: Res<Time>,
    mut rng: ResMut<SimRng>,
    sim: Res<Sim>,
    view: Res<MapView>,
    mut q: Query<(
        &mut TilePos,
        &mut Transform,
        &mut Brain,
        &Inventory,
        &mut HaulContract,
    )>,
) {
    let world = &sim.0;
    let dt = time.delta_secs();

    for (mut pos, mut transform, mut brain, inv, mut contract) in &mut q {
        // Glide the sprite toward the current tile centre (same as `movement`).
        let target = tile_to_pixel(pos.col, pos.row, world.hex_size, view.offset);
        let t = (dt * 8.0).min(1.0);
        transform.translation.x += (target.x - transform.translation.x) * t;
        transform.translation.y += (target.y - transform.translation.y) * t;

        // Idle: waiting for assignment. Loading: waiting for `haul_loading`.
        if matches!(contract.state, HaulState::Idle | HaulState::Loading) {
            continue;
        }

        brain.move_cooldown -= dt;
        if brain.move_cooldown > 0.0 {
            continue;
        }

        let dtile = world.deposits[contract.deposit].tile;
        let (tc, tr) = (world.tiles[dtile].col, world.tiles[dtile].row);
        let at_deposit = pos.col == tc && pos.row == tr;

        match contract.state {
            HaulState::ToPickup => {
                if at_deposit {
                    contract.state = HaulState::Loading; // `haul_loading` fills cargo
                } else {
                    let (c, r) = step_toward(world, pos.col, pos.row, tc, tr);
                    pos.col = c;
                    pos.row = r;
                }
            }
            HaulState::Selling => {
                let (c, r) = choose_outbound(&mut rng.0, world, pos.col, pos.row, brain.heading);
                pos.col = c;
                pos.row = r;
                contract.sell_steps += 1;
                let sold_out = inv.items[contract.cargo_item] <= 0.0;
                if sold_out || contract.sell_steps >= HAUL_SELL_STEPS {
                    contract.state = HaulState::Returning;
                }
            }
            HaulState::Returning => {
                if !at_deposit {
                    let (c, r) = step_toward(world, pos.col, pos.row, tc, tr);
                    pos.col = c;
                    pos.row = r;
                }
                // On arrival, `haul_settle` finalizes and resets to Idle.
            }
            HaulState::Idle | HaulState::Loading => {}
        }

        let tf = terrain_factor(world.tiles[tile_idx(world, pos.col, pos.row)].terrain);
        brain.move_cooldown = BASE_STEP_TIME / tf;
    }
}

fn ready_to_depart(world: &World, role: &Role, inv: &Inventory) -> bool {
    match role {
        Role::Owner { deposit } => {
            let slot = world.deposits[*deposit].element_slot;
            let raw = goods::item_index(slot, GoodForm::Raw);
            inv.items[raw] >= LOAD_THRESHOLD
        }
        _ => true,
    }
}

fn choose_outbound(
    rng: &mut crate::rng::Rng,
    world: &World,
    col: i32,
    row: i32,
    heading: usize,
) -> (i32, i32) {
    let ns = neighbors(col, row);
    let in_bound: Vec<(i32, i32)> = ns
        .iter()
        .copied()
        .filter(|&(c, r)| in_bounds(world, c, r))
        .collect();
    if in_bound.is_empty() {
        return (col, row);
    }

    // Bias toward the preferred heading when that neighbour is on the map.
    let preferred = ns[heading];
    if in_bounds(world, preferred.0, preferred.1) && rng.chance(HEADING_BIAS) {
        return preferred;
    }
    in_bound[rng.below(in_bound.len())]
}

fn step_toward(world: &World, col: i32, row: i32, hc: i32, hr: i32) -> (i32, i32) {
    let (hx, hy) = hex_center(hc, hr, 1.0);
    let mut best = (col, row);
    let mut best_d2 = f32::MAX;
    for (c, r) in neighbors(col, row) {
        if !in_bounds(world, c, r) {
            continue;
        }
        let (x, y) = hex_center(c, r, 1.0);
        let d2 = (x - hx).powi(2) + (y - hy).powi(2);
        if d2 < best_d2 {
            best_d2 = d2;
            best = (c, r);
        }
    }
    best
}

fn reinforce_and_reset(brain: &mut Brain, rng: &mut crate::rng::Rng) {
    // Reinforce the heading by the welfare this trip produced (consuming what
    // was bought/sold-for), so productive directions are favoured next time.
    if brain.trip_reward > 0.0 {
        brain.weights[brain.heading] += brain.trip_reward.min(REWARD_CAP);
    }
    for w in &mut brain.weights {
        *w *= WEIGHT_DECAY;
    }

    brain.heading = if rng.chance(EPSILON) {
        rng.below(6)
    } else {
        argmax(&brain.weights)
    };
    brain.trip_step = 0;
    brain.outbound = true;
    brain.trip_reward = 0.0;
}

fn argmax(weights: &[f32; 6]) -> usize {
    let mut best = 0;
    for i in 1..6 {
        if weights[i] > weights[best] {
            best = i;
        }
    }
    best
}
