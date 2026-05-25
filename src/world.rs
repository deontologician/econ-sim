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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Terrain {
    Easy,
    Difficult,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResourceRole {
    Replenishable,
    Finite,
}

/// Difficult terrain throttles growth, extraction, and movement.
pub fn terrain_factor(t: Terrain) -> f32 {
    match t {
        Terrain::Easy => 1.0,
        Terrain::Difficult => 0.55,
    }
}

pub struct ChosenElement {
    pub id: ElementId,
    pub role: ResourceRole,
    /// Tech multiplier: boosts replenishable growth and finite extraction. Starts 1.0.
    pub efficiency: f32,
    /// Cumulative units pulled out of the ground (HUD stat).
    pub extracted_total: f64,
}

pub struct Tile {
    pub col: i32,
    pub row: i32,
    pub terrain: Terrain,
    pub deposit: Option<usize>,
}

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

pub struct World {
    pub seed: u64,
    pub cols: i32,
    pub rows: i32,
    pub hex_size: f32,
    pub tiles: Vec<Tile>,
    pub chosen: Vec<ChosenElement>,
    pub deposits: Vec<Deposit>,
    pub goods: WorldGoods,
}

const DEPOSITS_PER_ELEMENT: usize = 3;
const SMOOTHING_PASSES: usize = 4;
const INITIAL_DIFFICULT_CHANCE: f32 = 0.45;

impl World {
    /// Advance the resource simulation by `dt` seconds. Only regrows
    /// replenishable deposits; extraction is driven by agents.
    pub fn tick(&mut self, dt: f32) {
        for di in 0..self.deposits.len() {
            let slot = self.deposits[di].element_slot;
            let tf = terrain_factor(self.tiles[self.deposits[di].tile].terrain);
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
        let tf = terrain_factor(self.tiles[self.deposits[di].tile].terrain);
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
    let mut world = World {
        seed,
        cols,
        rows,
        hex_size,
        tiles,
        chosen,
        deposits: Vec::new(),
        goods: world_goods,
    };
    place_deposits(&mut rng, &mut world);
    world
}

fn generate_terrain(rng: &mut Rng, cols: i32, rows: i32) -> Vec<Tile> {
    let count = (cols * rows) as usize;
    let idx = |c: i32, r: i32| (r * cols + c) as usize;

    let mut terr: Vec<Terrain> = (0..count)
        .map(|_| {
            if rng.chance(INITIAL_DIFFICULT_CHANCE) {
                Terrain::Difficult
            } else {
                Terrain::Easy
            }
        })
        .collect();

    // Cellular-automata smoothing turns the noise into clustered regions of
    // easy/difficult terrain. Out-of-bounds counts as difficult so the map
    // gets rugged edges rather than a clean rectangle.
    for _ in 0..SMOOTHING_PASSES {
        let mut next = terr.clone();
        for r in 0..rows {
            for c in 0..cols {
                let mut difficult = 0;
                for (nc, nr) in neighbors(c, r) {
                    if nc < 0 || nr < 0 || nc >= cols || nr >= rows {
                        difficult += 1;
                    } else if terr[idx(nc, nr)] == Terrain::Difficult {
                        difficult += 1;
                    }
                }
                next[idx(c, r)] = if difficult >= 4 {
                    Terrain::Difficult
                } else if difficult <= 2 {
                    Terrain::Easy
                } else {
                    terr[idx(c, r)]
                };
            }
        }
        terr = next;
    }

    let mut tiles = Vec::with_capacity(count);
    for r in 0..rows {
        for c in 0..cols {
            tiles.push(Tile {
                col: c,
                row: r,
                terrain: terr[idx(c, r)],
                deposit: None,
            });
        }
    }
    tiles
}

fn place_deposits(rng: &mut Rng, world: &mut World) {
    for slot in 0..world.chosen.len() {
        let role = world.chosen[slot].role;
        // Replenishables thrive on easy land; finite stocks hide in hard terrain.
        let prefer = match role {
            ResourceRole::Replenishable => Terrain::Easy,
            ResourceRole::Finite => Terrain::Difficult,
        };
        for _ in 0..DEPOSITS_PER_ELEMENT {
            let Some(tile) = pick_empty_tile(rng, &world.tiles, prefer) else {
                break;
            };
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
    }
}

fn pick_empty_tile(rng: &mut Rng, tiles: &[Tile], prefer: Terrain) -> Option<usize> {
    let n = tiles.len();
    let mut fallback = None;
    for _ in 0..40 {
        let t = rng.below(n);
        if tiles[t].deposit.is_some() {
            continue;
        }
        if tiles[t].terrain == prefer {
            return Some(t);
        }
        fallback = Some(t);
    }
    fallback.or_else(|| (0..n).find(|&t| tiles[t].deposit.is_none()))
}
