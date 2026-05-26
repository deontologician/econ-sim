//! Full game-state persistence to the browser's `localStorage`, so you can save a
//! run, tweak the rules, reload, and **resume** it (not just replay the seed).
//!
//! State is serialized to RON via serde. Migrations: bump [`SAVE_VERSION`] and add a
//! handler when the schema breaks; for *additive* changes mark new fields
//! `#[serde(default)]` so older saves still load. A parse failure or version
//! mismatch is treated as "no save" (fresh start), so a stale/corrupt blob never
//! wedges boot.

use serde::{Deserialize, Serialize};

use crate::economy::{EconStats, HungerControl, IncomeControl};
use crate::noot::{Claim, Hunger, Inventory, NootMeta, TilePos, Trader, Wallet};
use crate::world::World;

/// Bump on any breaking schema change (then migrate or discard older versions).
pub const SAVE_VERSION: u32 = 2;

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

/// Load and validate the saved snapshot, if any.
#[cfg(target_arch = "wasm32")]
pub fn load() -> Option<Snapshot> {
    let raw = storage()?.get_item(SAVE_KEY).ok().flatten()?;
    let snap: Snapshot = ron::from_str(&raw).ok()?;
    (snap.version == SAVE_VERSION).then_some(snap)
}

#[cfg(target_arch = "wasm32")]
pub fn store(snap: &Snapshot) {
    if let (Some(s), Ok(text)) = (storage(), ron::to_string(snap)) {
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
