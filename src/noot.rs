//! Noots: the economic agents. ECS components describing what a noot is, owns,
//! wants, and remembers. Behaviour lives in `movement.rs` and `economy.rs`.

use bevy::prelude::*;

use crate::goods::N_ITEMS;

pub const N_STAPLES: usize = 2;
pub const N_POSITIONAL: usize = 2;

pub const STARTING_BUCKS: f32 = 100.0;
/// Appetite at which a noot is "starving"; 0 means fully fed.
pub const STAPLE_SATIATION: f32 = 10.0;
/// Appetite gained per second per staple.
pub const HUNGER_RATE: f32 = 0.5;
/// Below this fraction of satiation a staple counts as satisfied.
pub const SATISFIED_FRACTION: f32 = 0.3;
/// At or above this fraction of satiation a staple counts as "starving".
pub const STARVING_FRACTION: f32 = 0.9;
/// Owners stop extracting once carrying this much of their raw good.
pub const CARRY_CAP: f32 = 20.0;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner { deposit: usize },
    Refiner,
    Consumer,
}

#[derive(Component, Clone, Copy)]
pub struct TilePos {
    pub col: i32,
    pub row: i32,
}

#[derive(Component, Clone, Copy)]
pub struct Home {
    pub col: i32,
    pub row: i32,
}

#[derive(Component)]
pub struct Inventory {
    pub items: [f32; N_ITEMS],
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            items: [0.0; N_ITEMS],
        }
    }
}

#[derive(Component)]
pub struct Wallet {
    pub bucks: f32,
}

/// Per-staple appetite: 0 = full, up to `STAPLE_SATIATION` = starving.
#[derive(Component)]
pub struct Hunger {
    pub staple: [f32; N_STAPLES],
}

impl Hunger {
    pub fn starving() -> Self {
        Self {
            staple: [STAPLE_SATIATION; N_STAPLES],
        }
    }

    /// All staples below the satisfied threshold.
    pub fn satisfied(&self) -> bool {
        self.staple
            .iter()
            .all(|&a| a < STAPLE_SATIATION * SATISFIED_FRACTION)
    }

    /// Any staple appetite at or above the starving threshold.
    pub fn is_starving(&self) -> bool {
        self.staple
            .iter()
            .any(|&a| a >= STAPLE_SATIATION * STARVING_FRACTION)
    }
}

/// Accumulated stock of each positional good (drives logarithmic utility).
#[derive(Component)]
pub struct Positional {
    pub stock: [f32; N_POSITIONAL],
}

/// Simple per-noot learning + walk state.
#[derive(Component)]
pub struct Brain {
    /// Preferred outbound hex direction (0..6).
    pub heading: usize,
    /// Reinforcement weight per direction.
    pub weights: [f32; 6],
    pub trip_step: u32,
    /// True while walking out, false while returning home.
    pub outbound: bool,
    pub sold_this_trip: bool,
    /// Seconds until the next tile step.
    pub move_cooldown: f32,
}

impl Brain {
    pub fn new(heading: usize) -> Self {
        Self {
            heading,
            weights: [1.0; 6],
            trip_step: 0,
            outbound: true,
            sold_this_trip: false,
            move_cooldown: 0.0,
        }
    }
}
