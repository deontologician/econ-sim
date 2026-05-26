//! Noots: the economic agents. ECS components describing what a noot is, owns,
//! wants, and remembers. Behaviour lives in `movement.rs` and `economy.rs`.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::goods::N_ITEMS;
use crate::rng::Rng;

pub const N_STAPLES: usize = 2;

pub const STARTING_BUCKS: f32 = 100.0;
/// Appetite at which a noot is "starving"; 0 means fully fed.
pub const STAPLE_SATIATION: f32 = 10.0;
/// Below this fraction of satiation a staple counts as satisfied.
pub const SATISFIED_FRACTION: f32 = 0.3;
/// At or above this fraction of satiation a staple counts as "starving".
pub const STARVING_FRACTION: f32 = 0.9;
/// A (re)spawned noot's appetite is drawn uniformly from `[0, this × satiation)` —
/// wide enough to decorrelate lifecycles, capped below the starving threshold.
pub const FRESH_HUNGER_SPREAD: f32 = 0.7;
/// A noot stops extracting its claimed deposit once carrying this much raw good.
pub const CARRY_CAP: f32 = 20.0;

// --- Trade / arbitrage (every noot can buy surplus to resell) ---------------
/// A noot's starting "fixed guess" discount on anticipated resale value — how much
/// of a good's market price it dares pay to acquire surplus for resale.
pub const DISCOUNT_INIT: f32 = 0.5;
/// Discount floor/ceiling: a noot never pays under `MIN` or over `MAX` of a good's
/// market ask to buy surplus.
pub const DISCOUNT_MIN: f32 = 0.2;
pub const DISCOUNT_MAX: f32 = 0.95;
/// How fast the learned discount moves: up toward `MAX` on a profitable sale
/// (success breeds optimism), down toward `MIN` on a buy (exposure breeds caution).
pub const DISCOUNT_LR: f32 = 0.04;

/// Marker on every noot entity (distinguishes them from tile/deposit/UI meshes).
#[derive(Component)]
pub struct Noot;

/// Per-noot life stats, surfaced by the noot-colouring overlays. `age` is seconds
/// lived (reset on respawn); `transactions` counts trades made (buys + sells);
/// `experience` is accumulated productive work (mining + refining), driving a
/// slow learning-by-doing speed bonus. All reset on rebirth.
#[derive(Component, Clone, Serialize, Deserialize)]
pub struct NootMeta {
    pub age: f32,
    pub transactions: u32,
    pub experience: f32,
}

impl NootMeta {
    pub fn new() -> Self {
        Self {
            age: 0.0,
            transactions: 0,
            experience: 0.0,
        }
    }
}

/// Which deposit a noot has claimed and may mine, if any. Claims are sticky: a
/// noot keeps its first claim and ignores other unowned deposits it passes.
#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub deposit: Option<usize>,
}

impl Claim {
    pub fn new(deposit: Option<usize>) -> Self {
        Self { deposit }
    }
}

/// Every noot's trading state. Beyond consuming, a noot with spare cash buys
/// surplus on its own account at `discount × market ask`, carries it, and resells
/// for the spread. `discount` is learned (see `DISCOUNT_LR`); `cost_basis` is the
/// running average price paid per held item, so realized margin = price − basis.
#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Trader {
    pub discount: f32,
    pub cost_basis: [f32; N_ITEMS],
}

impl Trader {
    pub fn new() -> Self {
        Self {
            discount: DISCOUNT_INIT,
            cost_basis: [0.0; N_ITEMS],
        }
    }
}

#[derive(Component, Clone, Copy, Serialize, Deserialize)]
pub struct TilePos {
    pub col: i32,
    pub row: i32,
}

#[derive(Component, Clone, Serialize, Deserialize)]
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

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub bucks: f32,
}

/// Per-staple appetite: 0 = full, up to `STAPLE_SATIATION` = starving.
#[derive(Component, Clone, Serialize, Deserialize)]
pub struct Hunger {
    pub staple: [f32; N_STAPLES],
    /// Seconds spent fully starving (all staples maxed); drives death.
    pub starving_secs: f32,
}

impl Hunger {
    /// A freshly (re)spawned noot: each staple's appetite is randomized over a wide
    /// spread (never high enough to start starving). The jitter decorrelates noot
    /// lifecycles so the population doesn't march to starvation in lockstep — deaths
    /// trickle continuously instead of arriving in synchronized waves, which is what
    /// the hunger-rate PID needs to regulate smoothly.
    pub fn fresh(rng: &mut Rng) -> Self {
        Self {
            staple: std::array::from_fn(|_| {
                rng.range(0.0, STAPLE_SATIATION * FRESH_HUNGER_SPREAD)
            }),
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

// --- Route learning (per-hex TD(λ) value estimates) -------------------------
/// Range of intrinsic explore/exploit ratios (ε) drawn per noot at birth: the
/// chance, each step, of a random move instead of climbing the learned value
/// gradient. Low = exploiter (beelines to known-good spots), high = wanderer.
pub const EXPLORE_MIN: f32 = 0.03;
pub const EXPLORE_MAX: f32 = 0.30;

/// TD learning rate.
const TD_ALPHA: f32 = 0.1;
/// Discount applied to the next tile's value.
const TD_GAMMA: f32 = 0.9;
/// Eligibility-trace decay (the λ in TD(λ)) — spreads credit back along the
/// path so reward at a destination raises the value of the hexes that led there.
const TD_LAMBDA: f32 = 0.8;
/// Drop a tile from the live trace once its eligibility falls below this.
const TRACE_CUTOFF: f32 = 0.02;

/// A noot's learned sense of *where good things happen*: a per-hex value estimate
/// over the whole map, trained online with TD(λ). Reward earned on a tile
/// (welfare from eating, income from selling) is banked in `pending_reward` and
/// folded into the estimate on the next step, so high-value regions pull future
/// movement up the gradient.
#[derive(Component)]
pub struct RouteMemory {
    /// Value estimate per tile, indexed `row * cols + col`.
    pub value: Vec<f32>,
    /// Eligibility trace per tile; only entries listed in `active` are non-zero.
    elig: Vec<f32>,
    /// Tiles whose eligibility trace is currently live.
    active: Vec<usize>,
    /// Reward accrued on the current tile, credited on the next step.
    pub pending_reward: f32,
    /// For a noot with a claim: heading back to its deposit to extract a load.
    pub homing: bool,
    /// Intrinsic explore/exploit ratio (ε): per-step chance of a random move.
    pub explore: f32,
    /// Seconds until the next tile step.
    pub move_cooldown: f32,
}

impl RouteMemory {
    pub fn new(n_tiles: usize, homing: bool, explore: f32) -> Self {
        Self {
            value: vec![0.0; n_tiles],
            elig: vec![0.0; n_tiles],
            active: Vec::new(),
            pending_reward: 0.0,
            homing,
            explore,
            move_cooldown: 0.0,
        }
    }

    /// Rebuild from a saved value field (the eligibility trace is transient, so it
    /// starts empty). `value.len()` is the tile count.
    pub fn restored(value: Vec<f32>, homing: bool, explore: f32) -> Self {
        let elig = vec![0.0; value.len()];
        Self {
            value,
            elig,
            active: Vec::new(),
            pending_reward: 0.0,
            homing,
            explore,
            move_cooldown: 0.0,
        }
    }

    /// TD(λ) update for a step from tile `from` to tile `to`, crediting the
    /// `reward` accrued while sitting on `from`.
    pub fn learn(&mut self, from: usize, to: usize, reward: f32) {
        let delta = reward + TD_GAMMA * self.value[to] - self.value[from];
        if self.elig[from] == 0.0 {
            self.active.push(from);
        }
        self.elig[from] = 1.0; // replacing trace
        let mut active = std::mem::take(&mut self.active);
        active.retain(|&t| {
            self.value[t] += TD_ALPHA * delta * self.elig[t];
            self.elig[t] *= TD_GAMMA * TD_LAMBDA;
            let live = self.elig[t] >= TRACE_CUTOFF;
            if !live {
                self.elig[t] = 0.0;
            }
            live
        });
        self.active = active;
    }
}
