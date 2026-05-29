//! Leaderboard payload: a compact summary the server extracts from a full save `Snapshot`.
//! The client POSTs the whole snapshot (reusing the save serializer); the server calls
//! [`summarize`] to pull out just the league-table fields — world identity, GDP, the world's
//! resources + tech, latest asset prices, and the rolled-up economy/price graphs.

use crate::save::Snapshot;
use serde::{Deserialize, Serialize};

/// Labels for the columns of `Summary::stat_history`, mirroring `graph::SERIES` order (which
/// is GUI-gated and so not importable here). GDP is the last column.
pub const STAT_SERIES_LABELS: [&str; 16] = [
    "prod", "cons", "margin", "utility", "trades", "avg ₦", "appetite", "starving", "claimed",
    "hunger", "deaths", "income", "infl", "avg age", "clump", "gdp",
];
/// Column index of GDP within a `stat_history` sample row.
pub const GDP_SERIES: usize = 15;

#[derive(Clone, Serialize, Deserialize)]
pub struct ResourceInfo {
    pub name: String,
    pub refined: String,
    pub role: String,
    /// Tech multiplier (1.0 = unteched); the world's progress on this resource.
    pub efficiency: f32,
}

/// One world's standing, derived from its latest snapshot.
#[derive(Clone, Serialize, Deserialize)]
pub struct Summary {
    pub name: String,
    pub seed: u64,
    /// Cumulative nominal output — the leaderboard's ranking key.
    pub gdp_total: f64,
    pub ticks: u64,
    pub production_rate: f32,
    pub consumption_rate: f32,
    pub gdp_rate: f32,
    /// The world's four resources, with role and tech level.
    pub resources: Vec<ResourceInfo>,
    /// Latest clearing price per tradable item (the asset prices).
    pub prices: Vec<f32>,
    /// Rolled-up economy graph: each row is one sample of [`STAT_SERIES_LABELS`].
    pub stat_history: Vec<Vec<f32>>,
    /// Rolled-up per-item price graph over time.
    pub price_history: Vec<Vec<f32>>,
}

/// Pull the leaderboard summary out of a full save snapshot.
pub fn summarize(snap: &Snapshot) -> Summary {
    let resources = snap
        .world
        .chosen
        .iter()
        .map(|c| {
            let def = crate::elements::element(c.id);
            ResourceInfo {
                name: def.name.to_string(),
                refined: def.refined.to_string(),
                role: match c.role {
                    crate::world::ResourceRole::Replenishable => "replenishable".to_string(),
                    crate::world::ResourceRole::Finite => "finite".to_string(),
                },
                efficiency: c.efficiency,
            }
        })
        .collect();
    Summary {
        name: crate::worldname::world_name(snap.world.seed),
        seed: snap.world.seed,
        gdp_total: snap.stats.gdp_total,
        ticks: snap.stats.ticks,
        production_rate: snap.stats.production_rate,
        consumption_rate: snap.stats.consumption_rate,
        gdp_rate: snap.stats.gdp_rate,
        resources,
        prices: snap.stats.last_sale_price.to_vec(),
        stat_history: snap.stat_history.iter().map(|s| s.to_vec()).collect(),
        price_history: snap.price_history.iter().map(|s| s.to_vec()).collect(),
    }
}
