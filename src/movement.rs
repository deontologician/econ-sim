//! Noot locomotion + the simple reinforcement loop.
//!
//! Outbound: step hex-to-hex, biased toward a learned `heading`, for `TRIP_LEN`
//! steps. Then return home greedily. On arrival, reinforce the heading if the
//! trip produced a sale, then pick the next heading ε-greedily.

use bevy::prelude::*;

use crate::goods::{self, GoodForm};
use crate::hex::{hex_center, neighbors};
use crate::noot::{Brain, Home, Inventory, Role, TilePos};
use crate::world::{terrain_factor, World};
use crate::{MapView, Sim, SimRng};

const TRIP_LEN: u32 = 8;
const BASE_STEP_TIME: f32 = 0.35;
const HEADING_BIAS: f32 = 0.7;
const EPSILON: f32 = 0.15;
const WEIGHT_DECAY: f32 = 0.98;
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
    mut noots: Query<(
        &mut TilePos,
        &Home,
        &mut Brain,
        &mut Transform,
        &Role,
        &Inventory,
    )>,
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
    if brain.sold_this_trip {
        brain.weights[brain.heading] += 1.0;
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
    brain.sold_this_trip = false;
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
