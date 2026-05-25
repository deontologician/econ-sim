//! Tiny versioned save to the browser's `localStorage`, so a world persists across
//! reloads — letting you tweak the rules and replay the same map. The blob is a flat
//! `key=value;` string behind a version number, with a migration step so older saves
//! still load after the schema changes.
//!
//! Today the save is just the world seed: the map (terrain, deposits, chosen
//! elements) regenerates deterministically from it, and the simulation restarts on
//! that fixed map. Richer snapshots (live noot/economy state) can be added later as
//! new fields here plus a migration arm.

/// Bump when the saved schema changes; add a `migrate` arm for the old version.
pub const SAVE_VERSION: u32 = 1;

/// The persisted game state.
#[derive(Clone, Copy)]
pub struct Save {
    pub seed: u64,
}

#[cfg(target_arch = "wasm32")]
const SAVE_KEY: &str = "econ-sim-save";

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Load and migrate the saved game, if any.
#[cfg(target_arch = "wasm32")]
pub fn load() -> Option<Save> {
    let raw = storage()?.get_item(SAVE_KEY).ok().flatten()?;
    parse_and_migrate(&raw)
}

#[cfg(target_arch = "wasm32")]
pub fn store(save: &Save) {
    if let Some(s) = storage() {
        let _ = s.set_item(SAVE_KEY, &format!("v={};seed={}", SAVE_VERSION, save.seed));
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

/// Parse a `key=value;` blob, then migrate it up to `SAVE_VERSION`.
#[cfg(target_arch = "wasm32")]
fn parse_and_migrate(raw: &str) -> Option<Save> {
    let mut version = 0u32;
    let mut seed: Option<u64> = None;
    for kv in raw.split(';') {
        if kv.is_empty() {
            continue;
        }
        let (k, v) = kv.split_once('=')?;
        match k.trim() {
            "v" => version = v.trim().parse().ok()?,
            "seed" => seed = v.trim().parse().ok(),
            _ => {} // ignore unknown keys so newer fields don't break older builds
        }
    }
    migrate(version, seed)
}

/// Upgrade a parsed save to the current schema. Unknown/newer versions are
/// discarded (treated as "no save") so a stale or corrupt blob never wedges boot.
#[cfg(target_arch = "wasm32")]
fn migrate(version: u32, seed: Option<u64>) -> Option<Save> {
    match version {
        // v1: { seed }. When the schema grows, add arms here that fill the new
        // fields with sensible defaults when loading a save from an older version.
        1 => Some(Save { seed: seed? }),
        _ => None,
    }
}

// --- Native stubs (the app ships as wasm; keep it compiling on native too) ---
#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Option<Save> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
pub fn store(_save: &Save) {}
#[cfg(not(target_arch = "wasm32"))]
pub fn clear() {}
#[cfg(not(target_arch = "wasm32"))]
pub fn reload_page() {}
