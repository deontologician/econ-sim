//! World state and generation. This is the pure simulation core: it has no
//! dependency on Bevy so the model stays easy to reason about and test.
//!
//! A world draws four elements from the pool and assigns each a resource role:
//! two are *replenishable* (regenerate at a slow steady rate, boostable by tech)
//! and two are *finite* (a large fixed stock extracted with diminishing returns
//! that tech can only partly offset). Roles are assigned per playthrough.
//!
//! Resources are **labor-gated**: deposits hold an extractable `stock` that
//! `World::tick` only *regrows* (for replenishables). Turning stock into carried
//! goods happens via [`World::extract_from`], called when an owner noot works.

use crate::elements::{element_count, ElementId};
use crate::goods::{self, WorldGoods};
use crate::hex::neighbors;
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ResourceRole {
    Replenishable,
    Finite,
}

/// Hardest terrain runs at `1 - DIFFICULTY_SLOWDOWN` of the easiest.
const DIFFICULTY_SLOWDOWN: f32 = 0.55;

/// Speed multiplier (growth, extraction, movement) for a tile's continuous
/// difficulty in `[0, 1]`: 1.0 on the easiest ground, falling linearly toward
/// `1 - DIFFICULTY_SLOWDOWN` on a cliff.
pub fn terrain_factor(difficulty: f32) -> f32 {
    1.0 - DIFFICULTY_SLOWDOWN * difficulty.clamp(0.0, 1.0)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ChosenElement {
    pub id: ElementId,
    pub role: ResourceRole,
    /// Tech multiplier: boosts replenishable growth and finite extraction. Starts 1.0.
    pub efficiency: f32,
    /// Cumulative units pulled out of the ground (HUD stat).
    pub extracted_total: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Tile {
    pub col: i32,
    pub row: i32,
    /// Continuous movement/work difficulty in `[0, 1]` (0 = easy, 1 = cliff).
    pub difficulty: f32,
    pub deposit: Option<usize>,
    /// Index into `World::structures` of a noot-built shop/refinery on this tile, if any.
    #[serde(default)]
    pub structure: Option<usize>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum DepositKind {
    /// Standing `stock` regrows toward `capacity` at `rate` (× efficiency × terrain).
    Replenishable {
        rate: f32,
        stock: f64,
        capacity: f64,
    },
    /// A large fixed stock; extraction slows as it is depleted, never regrows.
    Finite { remaining: f64, initial: f64 },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Deposit {
    /// Index into `World::chosen` (0..4).
    pub element_slot: usize,
    /// Index into `World::tiles`.
    pub tile: usize,
    pub kind: DepositKind,
}

impl Deposit {
    /// Units currently available to extract.
    pub fn available(&self) -> f64 {
        match &self.kind {
            DepositKind::Replenishable { stock, .. } => *stock,
            DepositKind::Finite { remaining, .. } => *remaining,
        }
    }
}

/// What a noot built on a hex. Both are waypoints a noot returns to; a Refinery is also
/// the only place refining can happen (a noot must stand on one).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureKind {
    Shop,
    Refinery,
}

/// A noot-built structure on a tile. Persistent: it outlives its builder and, while
/// unclaimed, can be adopted or built over. Ownership isn't stored here — like deposits,
/// it's derived from whichever noot's `Claim::hex` points at this tile.
#[derive(Clone, Serialize, Deserialize)]
pub struct Structure {
    pub tile: usize,
    pub kind: StructureKind,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct World {
    pub seed: u64,
    pub cols: i32,
    pub rows: i32,
    pub hex_size: f32,
    pub tiles: Vec<Tile>,
    pub chosen: Vec<ChosenElement>,
    pub deposits: Vec<Deposit>,
    pub goods: WorldGoods,
    /// Noot-built structures (shops and refineries), created during play.
    #[serde(default)]
    pub structures: Vec<Structure>,
    /// Per-tile **road strength** in `[0, 1]` (indexed `row * cols + col`), a decaying
    /// "desire-path" field: every noot step deposits a little on the tile it enters and the
    /// whole field decays exponentially each tick, so well-trodden corridors stay bright and
    /// abandoned ones fade. Roads cut the movement cost on their tile and pull the
    /// value-guided step toward them, so traffic self-reinforces into basins. Sized to the
    /// map; `#[serde(default)]` + lazy resizing in `accumulate_traffic` keeps pre-road saves
    /// loading (empty → rebuilt on first tick).
    #[serde(default)]
    pub road: Vec<f32>,
}

impl World {
    /// Place a structure of `kind` on `tile`: replace an existing (unclaimed) structure's
    /// kind in place if there is one, else append a new one. Returns the structure index.
    pub fn build_structure(&mut self, tile: usize, kind: StructureKind) -> usize {
        if let Some(idx) = self.tiles[tile].structure {
            self.structures[idx].kind = kind;
            idx
        } else {
            let idx = self.structures.len();
            self.structures.push(Structure { tile, kind });
            self.tiles[tile].structure = Some(idx);
            idx
        }
    }

    /// Whether `tile` has no deposit and no structure (open ground for a fresh build).
    pub fn tile_empty(&self, tile: usize) -> bool {
        self.tiles[tile].deposit.is_none() && self.tiles[tile].structure.is_none()
    }

    /// The kind of structure on `tile`, if any.
    pub fn structure_kind(&self, tile: usize) -> Option<StructureKind> {
        self.tiles[tile].structure.map(|i| self.structures[i].kind)
    }
}

/// Deposits come in clusters (a "field" of the same element) of `DEPOSITS_PER_CLUSTER`
/// within `CLUSTER_RADIUS` tiles, so resources read as related fields, not lone tiles.
/// How *many* clusters a slot gets depends on what its consumable good is — see
/// `CLUSTERS_BY_CONSUMPTION_RANK` — so the staples a noot eats are common and the
/// luxuries are scarce.
const DEPOSITS_PER_CLUSTER: usize = 3;
/// Clusters per slot, indexed by `consumption_rank`: the raw staple eaten directly is
/// most common, then the staple that must be refined to eat, then the raw positional
/// luxury, then the refined positional luxury (rarest). Total deposits =
/// `(sum) × DEPOSITS_PER_CLUSTER` = `(4+3+2+1) × 3 = 30`.
const CLUSTERS_BY_CONSUMPTION_RANK: [usize; 4] = [4, 3, 2, 1];
const CLUSTER_RADIUS: i32 = 2;
const SMOOTHING_PASSES: usize = 5;
/// How much a smoothing pass keeps a hex's own difficulty vs. its neighbours'
/// mean. Lower = smoother, so terrain clumps into larger regions.
const SMOOTH_SELF_WEIGHT: f32 = 0.25;
/// Exponent applied to the normalized field (>1 skews toward easy ground, so
/// really rough terrain is rarer).
const ROUGHNESS_GAMMA: f32 = 1.7;
/// Per-hex chance of seeding a cliff: a sharp jump to near-max difficulty.
const CLIFF_CHANCE: f32 = 0.018;
/// Number of low-difficulty corridors carved across the map (passable channels
/// even through otherwise rough terrain).
const N_CHANNELS: usize = 3;

impl World {
    /// Advance the resource simulation by `dt` seconds. Only regrows
    /// replenishable deposits; extraction is driven by agents.
    pub fn tick(&mut self, dt: f32) {
        for di in 0..self.deposits.len() {
            let slot = self.deposits[di].element_slot;
            let tf = terrain_factor(self.tiles[self.deposits[di].tile].difficulty);
            let eff = self.chosen[slot].efficiency;
            if let DepositKind::Replenishable {
                rate,
                stock,
                capacity,
            } = &mut self.deposits[di].kind
            {
                let room = (*capacity - *stock).max(0.0);
                // Logistic-ish: growth tapers as the deposit fills.
                let growth =
                    *rate as f64 * eff as f64 * tf as f64 * dt as f64 * (room / *capacity);
                *stock = (*stock + growth).min(*capacity);
            }
        }
    }

    /// Pull up to `base_work * dt` units (scaled by efficiency/terrain, and by
    /// remaining fraction for finite deposits) out of a deposit. Returns the
    /// amount extracted, which the caller adds to a noot's inventory.
    pub fn extract_from(&mut self, di: usize, base_work: f32, dt: f32) -> f64 {
        let slot = self.deposits[di].element_slot;
        let tf = terrain_factor(self.tiles[self.deposits[di].tile].difficulty);
        let eff = self.chosen[slot].efficiency;

        let taken = match &mut self.deposits[di].kind {
            DepositKind::Replenishable { stock, .. } => {
                // Replenishable harvest is gated by available stock, not tech;
                // tech instead speeds regrowth (see `tick`).
                let want = (base_work * tf * dt) as f64;
                let take = want.min(*stock);
                *stock -= take;
                take
            }
            DepositKind::Finite {
                remaining, initial, ..
            } => {
                if *remaining <= 0.0 {
                    0.0
                } else {
                    let frac = (*remaining / *initial) as f32;
                    let want = (base_work * eff * tf * frac * dt) as f64;
                    let take = want.min(*remaining);
                    *remaining -= take;
                    take
                }
            }
        };
        self.chosen[slot].extracted_total += taken;
        taken
    }

    /// Aggregate fraction of finite stock remaining for an element slot, or
    /// `None` for replenishable elements.
    pub fn remaining_fraction(&self, slot: usize) -> Option<f32> {
        let mut remaining = 0.0f64;
        let mut initial = 0.0f64;
        for dep in &self.deposits {
            if dep.element_slot != slot {
                continue;
            }
            if let DepositKind::Finite {
                remaining: r,
                initial: i,
            } = &dep.kind
            {
                remaining += *r;
                initial += *i;
            }
        }
        if initial > 0.0 {
            Some((remaining / initial) as f32)
        } else {
            None
        }
    }

    pub fn deposit_count(&self, slot: usize) -> usize {
        self.deposits
            .iter()
            .filter(|d| d.element_slot == slot)
            .count()
    }
}

pub fn generate(seed: u64, cols: i32, rows: i32, hex_size: f32) -> World {
    let mut rng = Rng::new(seed);

    // Draw four distinct elements.
    let mut ids: Vec<usize> = (0..element_count()).collect();
    rng.shuffle(&mut ids);

    // Assign resource roles per world: exactly two replenishable, two finite.
    let mut roles = [
        ResourceRole::Replenishable,
        ResourceRole::Replenishable,
        ResourceRole::Finite,
        ResourceRole::Finite,
    ];
    rng.shuffle(&mut roles);

    let chosen: Vec<ChosenElement> = (0..4)
        .map(|i| ChosenElement {
            id: ElementId(ids[i]),
            role: roles[i],
            efficiency: 1.0,
            extracted_total: 0.0,
        })
        .collect();

    // Assign consumption roles (staple/positional x raw/refined) to the slots.
    let world_goods = goods::assign(&mut rng);

    let tiles = generate_terrain(&mut rng, cols, rows);
    let road = vec![0.0; tiles.len()];
    let mut world = World {
        seed,
        cols,
        rows,
        hex_size,
        tiles,
        chosen,
        deposits: Vec::new(),
        goods: world_goods,
        structures: Vec::new(),
        road,
    };
    place_deposits(&mut rng, &mut world);
    world
}

fn generate_terrain(rng: &mut Rng, cols: i32, rows: i32) -> Vec<Tile> {
    let count = (cols * rows) as usize;
    let idx = |c: i32, r: i32| (r * cols + c) as usize;

    // Start from white noise in [0, 1].
    let mut d: Vec<f32> = (0..count).map(|_| rng.next_f32()).collect();

    // Relax toward the neighbourhood mean so difficulty varies *continuously* and
    // every hex is anchored to its surroundings. The map is a torus, so neighbours
    // wrap and there are no edges (seamless terrain).
    for _ in 0..SMOOTHING_PASSES {
        let mut next = d.clone();
        for r in 0..rows {
            for c in 0..cols {
                let mut sum = 0.0;
                for (nc, nr) in neighbors(c, r, cols, rows) {
                    sum += d[idx(nc, nr)];
                }
                let mean = sum / 6.0;
                next[idx(c, r)] =
                    SMOOTH_SELF_WEIGHT * d[idx(c, r)] + (1.0 - SMOOTH_SELF_WEIGHT) * mean;
            }
        }
        d = next;
    }

    // Smoothing compresses the range toward the mean; stretch it back to [0, 1]
    // so the gradients use the full difficulty span.
    let (lo, hi) = d
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    let span = (hi - lo).max(1e-3);
    for x in &mut d {
        // Normalize, then skew toward easy ground so really rough terrain is rare.
        *x = ((*x - lo) / span).powf(ROUGHNESS_GAMMA);
    }

    // Cliffs: sparse, sharp jumps that the smoothing would otherwise erase. Each
    // seed (and, at random, some neighbours, so it reads as a short ridge rather
    // than a dot) is pushed to near-max difficulty.
    for r in 0..rows {
        for c in 0..cols {
            if rng.chance(CLIFF_CHANCE) {
                d[idx(c, r)] = rng.range(0.85, 1.0);
                for (nc, nr) in neighbors(c, r, cols, rows) {
                    if rng.chance(0.4) {
                        d[idx(nc, nr)] = rng.range(0.8, 1.0);
                    }
                }
            }
        }
    }

    // Channels: meandering low-difficulty corridors carved last, so they stay
    // passable even where they cut through cliffs and rough patches.
    carve_channels(rng, cols, rows, &mut d);

    let mut tiles = Vec::with_capacity(count);
    for r in 0..rows {
        for c in 0..cols {
            tiles.push(Tile {
                col: c,
                row: r,
                difficulty: d[idx(c, r)],
                deposit: None,
                structure: None,
            });
        }
    }
    tiles
}

/// Carve `N_CHANNELS` meandering low-difficulty corridors edge to edge. Each
/// advances along one axis and wanders on the other, widening by one hex so the
/// channel is comfortably walkable.
fn carve_channels(rng: &mut Rng, cols: i32, rows: i32, d: &mut [f32]) {
    let idx = |c: i32, r: i32| (r * cols + c) as usize;
    let mut clear = |c: i32, r: i32, rng: &mut Rng| {
        if c >= 0 && r >= 0 && c < cols && r < rows {
            d[idx(c, r)] = rng.range(0.05, 0.2);
        }
    };
    for _ in 0..N_CHANNELS {
        if rng.chance(0.5) {
            // Horizontal corridor: sweep across columns, wander on the row.
            let mut r = rng.below(rows as usize) as i32;
            for c in 0..cols {
                clear(c, r, rng);
                clear(c, r + 1, rng);
                if rng.chance(0.45) {
                    r = (r + if rng.chance(0.5) { 1 } else { -1 }).clamp(0, rows - 1);
                }
            }
        } else {
            // Vertical corridor: sweep down rows, wander on the column.
            let mut c = rng.below(cols as usize) as i32;
            for r in 0..rows {
                clear(c, r, rng);
                clear(c + 1, r, rng);
                if rng.chance(0.45) {
                    c = (c + if rng.chance(0.5) { 1 } else { -1 }).clamp(0, cols - 1);
                }
            }
        }
    }
}

/// How common a slot's deposits should be, by what its consumable good is (lower =
/// more common): the raw staple eaten directly, then the staple that must be refined
/// to eat, then the raw positional luxury, then the refined positional luxury (rarest).
fn consumption_rank(good: &goods::ConsumableGood) -> usize {
    use goods::{GoodCategory::*, GoodForm::*};
    match (good.category, good.form) {
        (Staple, Raw) => 0,
        (Staple, Refined) => 1,
        (Positional, Raw) => 2,
        (Positional, Refined) => 3,
    }
}

fn place_deposits(rng: &mut Rng, world: &mut World) {
    for slot in 0..world.chosen.len() {
        let role = world.chosen[slot].role;
        // Replenishables thrive on easy land; finite stocks hide in hard terrain.
        let prefer_hard = matches!(role, ResourceRole::Finite);
        // Common staples get more fields than scarce luxuries (see the rank table).
        let clusters = CLUSTERS_BY_CONSUMPTION_RANK[consumption_rank(&world.goods.goods[slot])];
        // Seed each cluster centre on the preferred terrain, then pack the rest nearby.
        for _ in 0..clusters {
            let Some(center) = pick_empty_tile(rng, &world.tiles, prefer_hard) else {
                continue;
            };
            push_deposit(rng, world, slot, role, center);
            for _ in 1..DEPOSITS_PER_CLUSTER {
                let Some(tile) = pick_near_empty(rng, world, center, CLUSTER_RADIUS) else {
                    break;
                };
                push_deposit(rng, world, slot, role, tile);
            }
        }
    }
}

fn push_deposit(rng: &mut Rng, world: &mut World, slot: usize, role: ResourceRole, tile: usize) {
    let kind = match role {
        ResourceRole::Replenishable => {
            let capacity = rng.range(30.0, 60.0) as f64;
            DepositKind::Replenishable {
                rate: rng.range(0.6, 1.8),
                stock: capacity * 0.5,
                capacity,
            }
        }
        ResourceRole::Finite => {
            let initial = rng.range(800.0, 1600.0) as f64;
            DepositKind::Finite {
                remaining: initial,
                initial,
            }
        }
    };
    let di = world.deposits.len();
    world.deposits.push(Deposit {
        element_slot: slot,
        tile,
        kind,
    });
    world.tiles[tile].deposit = Some(di);
}

/// An empty tile within `radius` (in col/row) of `center`, for packing a cluster.
fn pick_near_empty(rng: &mut Rng, world: &World, center: usize, radius: i32) -> Option<usize> {
    let (cc, cr) = (world.tiles[center].col, world.tiles[center].row);
    let span = (2 * radius + 1) as usize;
    for _ in 0..30 {
        let c = (cc + rng.below(span) as i32 - radius).clamp(0, world.cols - 1);
        let r = (cr + rng.below(span) as i32 - radius).clamp(0, world.rows - 1);
        let t = (r * world.cols + c) as usize;
        if world.tiles[t].deposit.is_none() {
            return Some(t);
        }
    }
    None
}

fn pick_empty_tile(rng: &mut Rng, tiles: &[Tile], prefer_hard: bool) -> Option<usize> {
    let n = tiles.len();
    let mut fallback = None;
    for _ in 0..40 {
        let t = rng.below(n);
        if tiles[t].deposit.is_some() {
            continue;
        }
        if (tiles[t].difficulty > 0.5) == prefer_hard {
            return Some(t);
        }
        fallback = Some(t);
    }
    fallback.or_else(|| (0..n).find(|&t| tiles[t].deposit.is_none()))
}
