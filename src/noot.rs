//! Noots: the economic agents. ECS components describing what a noot is, owns,
//! wants, and remembers. Behaviour lives in `movement.rs` and `economy.rs`.

use bevy::prelude::*;

use crate::goods::N_ITEMS;

pub const N_STAPLES: usize = 2;

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

// --- Transporters (principal–agent hauling) ---------------------------------
/// Owner's cut of a haul's sale revenue; the transporter keeps `1 - SHARE`.
pub const PRINCIPAL_SHARE: f32 = 0.6;
/// Raw units a transporter loads per pickup. Kept above the owner's depart
/// threshold so a single pickup drops the owner back below it (keeping the
/// owner home extracting instead of touring to sell).
pub const HAUL_CAPACITY: f32 = 12.0;
/// Steps a transporter wanders selling before being forced to return & settle.
pub const HAUL_SELL_STEPS: u32 = 12;
/// Only hire a hauler for an owner carrying at least this much raw to move.
pub const MIN_HIRE: f32 = 4.0;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner { deposit: usize },
    Refiner,
    Consumer,
    Transporter,
}

/// A transporter's progress through one hauling contract.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HaulState {
    /// No contract; waiting to be matched to an owner.
    Idle,
    /// Walking to the employer's deposit to collect cargo.
    ToPickup,
    /// At the deposit, loading cargo from the employer.
    Loading,
    /// Wandering with cargo, selling to whoever's nearby.
    Selling,
    /// Walking back to the employer to hand over their share.
    Returning,
}

/// Hauling assignment carried by every transporter (idle when unmatched).
#[derive(Component)]
pub struct HaulContract {
    pub state: HaulState,
    /// The owner who hired this transporter; `None` only while `Idle`.
    pub employer: Option<Entity>,
    /// Index into `World::deposits` of the employer's deposit.
    pub deposit: usize,
    /// Raw item index being hauled.
    pub cargo_item: usize,
    /// Bucks banked from selling this contract's cargo (the owner's share is a
    /// fraction of this, settled on return).
    pub proceeds: f32,
    pub sell_steps: u32,
}

impl HaulContract {
    pub fn idle() -> Self {
        Self {
            state: HaulState::Idle,
            employer: None,
            deposit: 0,
            cargo_item: 0,
            proceeds: 0.0,
            sell_steps: 0,
        }
    }
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
    /// Seconds spent fully starving (all staples maxed); drives death.
    pub starving_secs: f32,
}

impl Hunger {
    /// A freshly (re)spawned noot: moderately hungry, not yet starving, so it
    /// gets a fair window to find food before the death timer can bite.
    pub fn fresh() -> Self {
        Self {
            staple: [STAPLE_SATIATION * 0.5; N_STAPLES],
            starving_secs: 0.0,
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

    /// Every staple pinned at maximum appetite — utterly out of food.
    pub fn fully_starving(&self) -> bool {
        self.staple.iter().all(|&a| a >= STAPLE_SATIATION - 1e-3)
    }

    /// Welfare from *not* being hungry: 1.0 per fully-fed staple, 0.0 when a
    /// staple is fully starving.
    pub fn utility(&self) -> f32 {
        self.staple
            .iter()
            .map(|&a| (STAPLE_SATIATION - a) / STAPLE_SATIATION)
            .sum()
    }
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
    /// Welfare (utility) gained this trip — the reinforcement signal. Earned by
    /// consuming what you acquired, so buying to eat is rewarded just like
    /// selling was.
    pub trip_reward: f32,
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
            trip_reward: 0.0,
            move_cooldown: 0.0,
        }
    }
}
