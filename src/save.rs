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
use crate::policy::ActorCritic;
use crate::world::World;

/// Current save schema version. Bump on any change, and add a [`migrate_step`] arm
/// upgrading the previous version to this one. v2 added the learned `policy`.
pub const SAVE_VERSION: u32 = 2;

/// The persisted parts of one noot. Per-noot RL state (`PolicyMemory`) is transient;
/// only its intrinsic exploration ε is kept (the shared brain is saved separately).
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
}

/// A complete simulation snapshot: the world, the controllers/stats, the shared
/// learned policy, and every noot.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub world: World,
    pub hunger: HungerControl,
    pub income: IncomeControl,
    pub stats: EconStats,
    /// Shared actor-critic brain. `#[serde(default)]` so a pre-policy (v1) save loads
    /// with an empty net, which `setup` then re-initializes to the world's tile count.
    #[serde(default)]
    pub policy: ActorCritic,
    pub noots: Vec<NootSave>,
}

/// Upgrade a parsed save in place from `from_version` to `from_version + 1`. Called for
/// each version the blob is behind, so a step only ever sees the shape produced by the
/// previous step. Mutate the JSON tree here — add fields with defaults, rename/
/// restructure keys — since the live `Snapshot` is always newest.
///
/// Add an arm whenever you bump [`SAVE_VERSION`], e.g. for a future v2:
/// ```ignore
/// 1 => { save["new_field"] = serde_json::json!(0.0); }
/// ```
fn migrate_step(from_version: u32, save: &mut serde_json::Value) {
    // v1 → v2: the shared `policy` field was added. It's `#[serde(default)]`, so a v1
    // save (which lacks it) deserializes with an empty net and `setup` re-initializes
    // it to the world's tile count — no JSON surgery needed here.
    let _ = (from_version, save);
}

/// Replay every pending migration on a parsed blob, then deserialize into the current
/// `Snapshot`. Shared by both backends (localStorage and file). A version newer than
/// this build, or any failure, yields `None` (treated as "no save").
fn snapshot_from_value(mut value: serde_json::Value) -> Option<Snapshot> {
    let mut version = value.get("version")?.as_u64()? as u32;
    if version > SAVE_VERSION {
        return None; // saved by a newer build than this one
    }
    while version < SAVE_VERSION {
        migrate_step(version, &mut value);
        version += 1;
        value["version"] = serde_json::json!(version);
    }
    match serde_json::from_value::<Snapshot>(value.clone()) {
        Ok(snap) => Some(snap),
        Err(_) => {
            // Recovery: the shared `policy` is a large float blob, and if any weight
            // ever went non-finite, `serde_json` wrote it as `null` (which won't parse
            // back as f32), failing the whole load. Drop the policy and retry — the
            // world, noots and stats still resume; the brain (`#[serde(default)]`) just
            // resets to fresh. Better a lost brain than a lost world.
            if let Some(obj) = value.as_object_mut() {
                obj.remove("policy");
            }
            serde_json::from_value(value).ok()
        }
    }
}

// --- Browser backend (localStorage) -----------------------------------------
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
    snapshot_from_value(serde_json::from_str(&raw).ok()?)
}

#[cfg(target_arch = "wasm32")]
pub fn store(snap: &Snapshot) {
    if let (Some(s), Ok(text)) = (storage(), serde_json::to_string(snap)) {
        let _ = s.set_item(SAVE_KEY, &text);
    }
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

// --- Native backend (JSON file) ---------------------------------------------
// The same full-state round-trip the browser does, to a file instead of localStorage,
// so the headless harness has the GUI's save/load/new affordances. The no-arg
// `load`/`store`/`clear` use a default path (overridable via the `ECON_SIM_SAVE` env
// var) so a native GUI build persists too; the headless binary passes explicit paths
// via `load_from`/`store_to`.
#[cfg(not(target_arch = "wasm32"))]
fn default_path() -> String {
    std::env::var("ECON_SIM_SAVE").unwrap_or_else(|_| "econ-sim-save.json".into())
}

/// Read a snapshot from `path` (migrating as needed); `None` if absent/corrupt/newer.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_from(path: &str) -> Option<Snapshot> {
    let raw = std::fs::read_to_string(path).ok()?;
    snapshot_from_value(serde_json::from_str(&raw).ok()?)
}

/// Write a snapshot to `path` as JSON (best-effort; errors are ignored, as on the web).
#[cfg(not(target_arch = "wasm32"))]
pub fn store_to(path: &str, snap: &Snapshot) {
    if let Ok(text) = serde_json::to_string(snap) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Option<Snapshot> {
    load_from(&default_path())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn store(snap: &Snapshot) {
    store_to(&default_path(), snap);
}
#[cfg(not(target_arch = "wasm32"))]
pub fn clear() {
    let _ = std::fs::remove_file(default_path());
}
#[cfg(not(target_arch = "wasm32"))]
pub fn reload_page() {}
