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

/// How much one **global** good identity has been studied in this world — the per-resource
/// research demand the server aggregates across all worlds to grow the shared tech tree
/// (plans/035). Self-describing (`element` + `form`) so the server needs no per-world item
/// table: a local item index is mapped here to its `(element, form)` global key.
#[derive(Clone, Serialize, Deserialize)]
pub struct ResearchDemand {
    /// Raw element name (e.g. "Iron"); pairs with `refined` for the refined form.
    pub element: String,
    pub refined: String,
    /// "raw" or "refined" — which form of the element was studied.
    pub form: String,
    /// Cumulative research effort on this good in this world.
    pub demand: f64,
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
    /// Per-resource research demand (global element identity), for the server's tech tree.
    /// Only goods studied at least once appear. `#[serde(default)]` so older snapshots and
    /// older servers interoperate during rollout.
    #[serde(default)]
    pub research: Vec<ResearchDemand>,
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
    // Map each local item index (slot*2 + form) to its global (element, form) identity, so
    // the server can sum demand across worlds without knowing any world's item table.
    let research = snap
        .stats
        .research_demand
        .iter()
        .enumerate()
        .filter(|(_, &d)| d > 0.0)
        .map(|(i, &demand)| {
            let def = crate::elements::element(snap.world.chosen[i / 2].id);
            ResearchDemand {
                element: def.name.to_string(),
                refined: def.refined.to_string(),
                form: if i % 2 == 1 { "refined" } else { "raw" }.to_string(),
                demand,
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
        research,
        stat_history: snap.stat_history.iter().map(|s| s.to_vec()).collect(),
        price_history: snap.price_history.iter().map(|s| s.to_vec()).collect(),
    }
}
