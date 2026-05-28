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

/// The single action a noot takes this tick, chosen by the learned policy in
/// `economy::policy_step`. Mining and refining are mutually exclusive — picking one
/// means forgoing the other this tick.
#[derive(Component, Clone, Copy, PartialEq, Eq, Default)]
pub enum Action {
    /// Took a relative directional step this tick (which neighbour is resolved by
    /// the policy's chosen direction in `economy::policy_step`).
    #[default]
    Move,
    /// Extract from the claimed deposit underfoot.
    Mine,
    /// Convert a held intermediate into its refined good.
    Refine,
    /// Holding position — e.g. lingering at a market while a committed Sell plan waits
    /// for a buyer. Distinct from Move so it incurs no hauling (carry) hunger cost.
    Idle,
    /// Build (or rebuild over) a shop on the current tile this tick.
    BuildShop,
    /// Build (or rebuild over) a refinery on the current tile this tick.
    BuildRefinery,
}

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

impl Default for NootMeta {
    fn default() -> Self {
        Self::new()
    }
}

/// A noot's persistent identity: a randomly generated first + last name (like
/// "Tim Dorphindel") kept for the life of the entity, plus an incarnation counter bumped
/// each rebirth. Unlike the rest of a noot's state, the name **survives death** — only
/// `incarnation` advances, so the same noot returns as "Tim Dorphindel the 2nd", "the
/// 3rd", and so on.
#[derive(Component, Clone, Serialize, Deserialize, Default)]
pub struct NootName {
    pub first: String,
    pub last: String,
    /// 1 on the first life; +1 on each reincarnation. 0 marks a not-yet-named noot
    /// (e.g. a pre-names save), which the spawner replaces with a fresh random name.
    #[serde(default)]
    pub incarnation: u32,
}

impl NootName {
    pub fn random(rng: &mut Rng) -> Self {
        Self {
            first: random_first(rng),
            last: random_last(rng),
            incarnation: 1,
        }
    }

    /// True for the `Default` placeholder (no name assigned yet).
    pub fn is_unnamed(&self) -> bool {
        self.incarnation == 0 || self.first.is_empty()
    }

    /// Note a rebirth: the same name returns one incarnation later.
    pub fn reincarnate(&mut self) {
        self.incarnation += 1;
    }

    /// "First Last" on the first life, "First Last the Nth" after reincarnating.
    pub fn display(&self) -> String {
        if self.incarnation > 1 {
            format!("{} {} the {}", self.first, self.last, ordinal(self.incarnation))
        } else {
            format!("{} {}", self.first, self.last)
        }
    }
}

/// English ordinal: 1→"1st", 2→"2nd", 3→"3rd", 11→"11th", 22→"22nd", ...
fn ordinal(n: u32) -> String {
    let suffix = if (11..=13).contains(&(n % 100)) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}

/// A short, mostly-human first name.
const FIRST_NAMES: &[&str] = &[
    "Tim", "Bex", "Otho", "Mara", "Pim", "Gus", "Edda", "Ren", "Vera", "Cob", "Nim", "Sable",
    "Hal", "Juno", "Wim", "Tilda", "Bram", "Lux", "Odette", "Finn", "Greta", "Moss", "Pippa",
    "Quill", "Roan", "Sten", "Una", "Vance", "Wren", "Yara", "Zeb", "Ada", "Bo", "Clem", "Doro",
];

/// Last-name syllables assembled into fantasy surnames like "Dorphindel".
const LAST_STARTS: &[&str] = &[
    "Dor", "Bran", "Thal", "Mor", "Fen", "Gal", "Wyn", "Cor", "Drav", "El", "Hal", "Kor", "Lor",
    "Mal", "Nor", "Quil", "Ran", "Syl", "Tor", "Vel", "Wren", "Zor", "Brim", "Cael", "Grim",
];
const LAST_MIDS: &[&str] = &[
    "phin", "an", "or", "il", "wyn", "ad", "en", "oth", "ar", "el", "und", "ish", "av", "om", "in",
];
const LAST_ENDS: &[&str] = &[
    "del", "dor", "wyn", "ar", "ius", "eth", "ow", "fell", "gard", "more", "ven", "wick", "rim",
    "den", "vale", "ric", "burn", "field", "thorn", "mere",
];

fn pick<'a>(rng: &mut Rng, xs: &[&'a str]) -> &'a str {
    xs[rng.below(xs.len())]
}

fn random_first(rng: &mut Rng) -> String {
    pick(rng, FIRST_NAMES).to_string()
}

fn random_last(rng: &mut Rng) -> String {
    let mut s = String::from(pick(rng, LAST_STARTS));
    // Roughly half the surnames carry a middle syllable, for length variety.
    if rng.chance(0.5) {
        s.push_str(pick(rng, LAST_MIDS));
    }
    s.push_str(pick(rng, LAST_ENDS));
    s
}

/// Generic hex ownership: the one improved tile a noot owns — a deposit it mines, or a
/// shop/refinery it built or adopted. At most one per noot. Sticky until death, when it
/// abandons (the deposit reopens; a structure stands for another noot to take or rebuild).
#[derive(Component, Clone, Serialize, Deserialize, Default)]
pub struct Claim {
    /// Tile index of the owned improvement, or `None` if the noot owns nothing yet.
    #[serde(default)]
    pub hex: Option<usize>,
}

impl Claim {
    pub fn new(hex: Option<usize>) -> Self {
        Self { hex }
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

impl Default for Trader {
    fn default() -> Self {
        Self::new()
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

impl Default for Inventory {
    fn default() -> Self {
        Self::new()
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

// --- Exploration ------------------------------------------------------------
/// Range of intrinsic explore/exploit ratios (ε) drawn per noot at birth: the
/// chance, each decision, of a random move instead of the critic-greedy one. Low =
/// exploiter (beelines to known-good spots), high = wanderer. Held in `PolicyMemory`.
pub const EXPLORE_MIN: f32 = 0.03;
pub const EXPLORE_MAX: f32 = 0.30;

