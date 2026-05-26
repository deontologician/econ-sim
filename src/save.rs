//! Full game-state persistence to the browser's `localStorage`, so you can save a
//! run, tweak the rules, reload, and **resume** it (not just replay the seed).
//!
//! State is serialized to JSON via serde. Each save carries a `version`; on load we
//! parse to a generic `serde_json::Value`, then **replay every migration step** from
//! the file's version up to [`SAVE_VERSION`] (see [`migrate_step`]) before
//! deserializing into the current `Snapshot`. To evolve the schema: bump
//! `SAVE_VERSION` and add the matching step. A parse failure, a version newer than
//! this build, or a missing migration is treated as "no save" (fresh start), so a
//! stale/corrupt blob never wedges boot.
//!
//! (JSON rather than RON because `serde_json::Value` round-trips our enums
//! faithfully, which the value-level migration replay depends on.)

use serde::{Deserialize, Serialize};

use crate::economy::{EconStats, HungerControl, IncomeControl};
use crate::noot::{Claim, Hunger, Inventory, NootMeta, TilePos, Trader, Wallet};
use crate::world::World;

/// Current save schema version. Bump on any change, and add a [`migrate_step`] arm
/// upgrading the previous version to this one.
pub const SAVE_VERSION: u32 = 1;

/// The persisted parts of one noot. `RouteMemory`'s eligibility trace is transient,
/// so only its learned `value` field (plus `explore`/`homing`) is kept.
#[derive(Serialize, Deserialize, Clone)]
pub struct NootSave {
    pub pos: TilePos,
    pub inv: Inventory,
    pub wallet: Wallet,
    pub hunger: Hunger,
    pub claim: Claim,
    pub trader: Trader,
    pub meta: NootMeta,
    pub explore: f32,
    pub homing: bool,
    pub value: Vec<f32>,
}

/// A complete simulation snapshot: the world, the controllers/stats, and every noot.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub world: World,
    pub hunger: HungerControl,
    pub income: IncomeControl,
    pub stats: EconStats,
    pub noots: Vec<NootSave>,
}

#[cfg(target_arch = "wasm32")]
const SAVE_KEY: &str = "econ-sim-save";

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Load the saved snapshot, replaying any pending migrations to the current schema.
#[cfg(target_arch = "wasm32")]
pub fn load() -> Option<Snapshot> {
    let raw = storage()?.get_item(SAVE_KEY).ok().flatten()?;
    let mut value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let mut version = value.get("version")?.as_u64()? as u32;
    if version > SAVE_VERSION {
        return None; // saved by a newer build than this one
    }
    // Replay every missing migration in order: v → v+1 → … → SAVE_VERSION.
    while version < SAVE_VERSION {
        migrate_step(version, &mut value);
        version += 1;
        value["version"] = serde_json::json!(version);
    }
    serde_json::from_value(value).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn store(snap: &Snapshot) {
    if let (Some(s), Ok(text)) = (storage(), serde_json::to_string(snap)) {
        let _ = s.set_item(SAVE_KEY, &text);
    }
}

/// Upgrade a parsed save in place from `from_version` to `from_version + 1`. `load`
/// calls this for each version the file is behind, so a step only ever sees the
/// shape produced by the previous step. Mutate the JSON tree here — add fields with
/// defaults, rename/restructure keys — since the live `Snapshot` is always newest.
///
/// Add an arm whenever you bump [`SAVE_VERSION`], e.g. for a future v2:
/// ```ignore
/// 1 => { save["new_field"] = serde_json::json!(0.0); }
/// ```
#[cfg(target_arch = "wasm32")]
fn migrate_step(from_version: u32, save: &mut serde_json::Value) {
    // No migrations yet — v1 is the first schema. When you bump SAVE_VERSION, branch
    // on `from_version` and mutate `save` to the next shape, e.g.:
    //   if from_version == 1 { save["new_field"] = serde_json::json!(0.0); }
    let _ = (from_version, save);
}

#[cfg(target_arch = "wasm32")]
pub fn clear() {
    if let Some(s) = storage() {
        let _ = s.remove_item(SAVE_KEY);
    }
}

/// Reload the page — used to start a fresh world after clearing the save.
#[cfg(target_arch = "wasm32")]
pub fn reload_page() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}

// --- Native stubs (the app ships as wasm; keep it compiling on native too) ---
#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Option<Snapshot> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
pub fn store(_snap: &Snapshot) {}
#[cfg(not(target_arch = "wasm32"))]
pub fn clear() {}
#[cfg(not(target_arch = "wasm32"))]
pub fn reload_page() {}
