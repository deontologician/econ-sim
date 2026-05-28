//! econ-sim library: the simulation core, shared by the GUI binary (`src/main.rs`)
//! and the headless harness (`src/bin/headless.rs`). Rendering-only modules are
//! behind the `gui` feature; the rest build against core Bevy (ECS/app/time) only,
//! so the headless harness compiles and runs without any GPU/windowing libraries.

use bevy::prelude::*;

pub mod economy;
pub mod elements;
pub mod goods;
pub mod hex;
pub mod history;
pub mod noot;
pub mod policy;
pub mod rng;
pub mod save;
pub mod world;

#[cfg(feature = "gui")]
pub mod graph;
#[cfg(feature = "gui")]
pub mod icon;
#[cfg(feature = "gui")]
pub mod movement;

use rng::Rng;
use world::World;

/// The simulation world (terrain, deposits, goods), as a resource.
#[derive(Resource)]
pub struct Sim(pub World);

/// The deterministic simulation RNG (SplitMix64), as a resource.
#[derive(Resource)]
pub struct SimRng(pub Rng);

/// How tile coordinates map to world pixels (map centred on the origin). GUI only,
/// but kept here so the rendering modules can share it.
#[derive(Resource, Clone, Copy)]
pub struct MapView {
    pub offset: Vec2,
    pub hex_size: f32,
    /// Full map extent in world units, used to fit the camera on launch.
    pub map_w: f32,
    pub map_h: f32,
}
