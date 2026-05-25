//! Noot locomotion + the per-hex value-learning loop.
//!
//! Every noot carries a `RouteMemory` — a TD(λ) value estimate over the map. Each
//! step it moves ε-greedily up the value gradient (toward its best-valued
//! neighbour), banking any reward earned on the tile it leaves and folding it into
//! the estimate, so productive regions pull future movement. Owners are the
//! exception: when sold out they home to their deposit to extract a fresh load,
//! then rejoin the value-driven wander to sell it. Merchants free-roam like
//! everyone else, their field shaped by buy/resale rewards.

use bevy::prelude::*;

use crate::goods::{self, GoodForm};
use crate::hex::{hex_center, neighbors};
use crate::noot::{Inventory, Role, RouteMemory, TilePos};
use crate::world::{terrain_factor, World};
use crate::{MapView, Sim, SimRng};

const BASE_STEP_TIME: f32 = 0.35;
/// Chance of a random (exploratory) step instead of the greedy value step.
const EPSILON: f32 = 0.12;
/// Owners extract until carrying this much raw, then tour to sell it.
const LOAD_THRESHOLD: f32 = 6.0;
/// Owners head back to refill once their raw stock falls to this.
const SELL_DONE: f32 = 1.0;
/// Value differences below this count as a tie (broken at random).
const VALUE_EPS: f32 = 1e-4;

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
        &mut RouteMemory,
        &mut Transform,
        &Role,
        &Inventory,
    )>,
) {
    let world = &sim.0;
    let dt = time.delta_secs();

    for (mut pos, mut mem, mut transform, role, inv) in &mut noots {
        // Smoothly glide the sprite toward the current tile centre.
        let target = tile_to_pixel(pos.col, pos.row, world.hex_size, view.offset);
        let t = (dt * 8.0).min(1.0);
        transform.translation.x += (target.x - transform.translation.x) * t;
        transform.translation.y += (target.y - transform.translation.y) * t;

        mem.move_cooldown -= dt;
        if mem.move_cooldown > 0.0 {
            continue;
        }

        let from = tile_idx(world, pos.col, pos.row);
        let next = match *role {
            Role::Owner { deposit } => {
                let slot = world.deposits[deposit].element_slot;
                let raw = goods::item_index(slot, GoodForm::Raw);
                let stock = inv.items[raw];
                if mem.homing && stock >= LOAD_THRESHOLD {
                    mem.homing = false;
                } else if !mem.homing && stock <= SELL_DONE {
                    mem.homing = true;
                }
                if mem.homing {
                    let dtile = world.deposits[deposit].tile;
                    let (dc, dr) = (world.tiles[dtile].col, world.tiles[dtile].row);
                    if pos.col == dc && pos.row == dr {
                        None // sit on the deposit; `extract` fills the load
                    } else {
                        Some(step_toward(world, pos.col, pos.row, dc, dr))
                    }
                } else {
                    Some(value_step(&mut rng.0, world, &mem.value, pos.col, pos.row))
                }
            }
            _ => Some(value_step(&mut rng.0, world, &mem.value, pos.col, pos.row)),
        };

        if let Some((c, r)) = next {
            let to = tile_idx(world, c, r);
            let reward = mem.pending_reward;
            mem.pending_reward = 0.0;
            mem.learn(from, to, reward);
            pos.col = c;
            pos.row = r;
        }

        // Difficult terrain makes each step take longer.
        let tf = terrain_factor(world.tiles[tile_idx(world, pos.col, pos.row)].terrain);
        mem.move_cooldown = BASE_STEP_TIME / tf;
    }
}

/// ε-greedy step up the learned value gradient: usually move to the highest-value
/// in-bounds neighbour (ties broken at random, so an all-zero field gives an
/// unbiased walk), occasionally a random neighbour to keep exploring.
fn value_step(
    rng: &mut crate::rng::Rng,
    world: &World,
    value: &[f32],
    col: i32,
    row: i32,
) -> (i32, i32) {
    let inb: Vec<(i32, i32)> = neighbors(col, row)
        .into_iter()
        .filter(|&(c, r)| in_bounds(world, c, r))
        .collect();
    if inb.is_empty() {
        return (col, row);
    }
    if rng.chance(EPSILON) {
        return inb[rng.below(inb.len())];
    }
    let mut best_val = f32::MIN;
    let mut best: Vec<(i32, i32)> = Vec::new();
    for (c, r) in inb {
        let v = value[tile_idx(world, c, r)];
        if v > best_val + VALUE_EPS {
            best_val = v;
            best.clear();
            best.push((c, r));
        } else if v >= best_val - VALUE_EPS {
            best.push((c, r));
        }
    }
    best[rng.below(best.len())]
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
