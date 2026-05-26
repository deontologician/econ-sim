//! Noot sprite locomotion (visual only).
//!
//! Discrete hex steps are decided by the learned policy in `economy::policy_step`
//! (the Move action steps toward the critic's best-valued neighbour). This system
//! just glides each sprite smoothly toward its current tile centre every frame.

use bevy::prelude::*;

use crate::hex::hex_center;
use crate::noot::{Noot, TilePos};
use crate::{MapView, Sim};

pub fn tile_to_pixel(col: i32, row: i32, hex_size: f32, offset: Vec2) -> Vec2 {
    let (x, y) = hex_center(col, row, hex_size);
    Vec2::new(x + offset.x, y + offset.y)
}

pub fn movement(
    time: Res<Time>,
    sim: Res<Sim>,
    view: Res<MapView>,
    mut noots: Query<(&TilePos, &mut Transform), With<Noot>>,
) {
    let hex_size = sim.0.hex_size;
    let t = (time.delta_secs() * 8.0).min(1.0);
    // A torus wrap moves a noot across the whole map in one step; snap instead of
    // gliding so it doesn't streak across the screen.
    let snap2 = (hex_size * 4.0).powi(2);
    for (pos, mut transform) in &mut noots {
        let target = tile_to_pixel(pos.col, pos.row, hex_size, view.offset);
        let cur = transform.translation.truncate();
        if target.distance_squared(cur) > snap2 {
            transform.translation.x = target.x;
            transform.translation.y = target.y;
        } else {
            transform.translation.x += (target.x - cur.x) * t;
            transform.translation.y += (target.y - cur.y) * t;
        }
    }
}
