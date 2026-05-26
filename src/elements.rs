//! The pool of ~25 candidate "elements". Each playthrough draws four of these
//! to be the building blocks of that world. Colours are sRGB and used as each
//! element's theme colour for its procedural icon (`icon.rs`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ElementId(pub usize);

pub struct ElementDef {
    pub name: &'static str,
    /// Hardcoded first-level refined product. Whichever four elements a world
    /// draws, their refined forms are always on hand. (Later an LLM fills in a
    /// deeper tech tree here.)
    pub refined: &'static str,
    pub color: (f32, f32, f32),
}

pub const ELEMENTS: [ElementDef; 25] = [
    ElementDef { name: "Lightning", refined: "Battery", color: (0.96, 0.90, 0.30) },
    ElementDef { name: "Fire", refined: "Ember", color: (0.95, 0.35, 0.15) },
    ElementDef { name: "Slime", refined: "Gel", color: (0.45, 0.85, 0.35) },
    ElementDef { name: "Acid", refined: "Reagent", color: (0.66, 0.95, 0.20) },
    ElementDef { name: "Water", refined: "Tonic", color: (0.25, 0.55, 0.95) },
    ElementDef { name: "Sugar", refined: "Candy", color: (0.98, 0.92, 0.88) },
    ElementDef { name: "Wood", refined: "Plank", color: (0.55, 0.38, 0.20) },
    ElementDef { name: "Ice", refined: "Coolant", color: (0.70, 0.90, 0.98) },
    ElementDef { name: "Stone", refined: "Brick", color: (0.55, 0.55, 0.58) },
    ElementDef { name: "Sand", refined: "Glass", color: (0.93, 0.85, 0.55) },
    ElementDef { name: "Oil", refined: "Fuel", color: (0.18, 0.15, 0.22) },
    ElementDef { name: "Gold", refined: "Ingot", color: (0.95, 0.78, 0.25) },
    ElementDef { name: "Iron", refined: "Steel", color: (0.60, 0.62, 0.66) },
    ElementDef { name: "Salt", refined: "Cure", color: (0.92, 0.92, 0.96) },
    ElementDef { name: "Steam", refined: "Turbine", color: (0.85, 0.88, 0.92) },
    ElementDef { name: "Ash", refined: "Lye", color: (0.42, 0.40, 0.38) },
    ElementDef { name: "Crystal", refined: "Lens", color: (0.70, 0.85, 0.96) },
    ElementDef { name: "Mud", refined: "Clay", color: (0.42, 0.32, 0.22) },
    ElementDef { name: "Smoke", refined: "Incense", color: (0.52, 0.52, 0.55) },
    ElementDef { name: "Wind", refined: "Sail", color: (0.80, 0.92, 0.85) },
    ElementDef { name: "Light", refined: "Beacon", color: (1.00, 0.97, 0.80) },
    ElementDef { name: "Shadow", refined: "Veil", color: (0.22, 0.20, 0.30) },
    ElementDef { name: "Copper", refined: "Wire", color: (0.80, 0.50, 0.30) },
    ElementDef { name: "Sulfur", refined: "Match", color: (0.92, 0.85, 0.30) },
    ElementDef { name: "Honey", refined: "Mead", color: (0.90, 0.65, 0.20) },
];

pub fn element(id: ElementId) -> &'static ElementDef {
    &ELEMENTS[id.0]
}

pub fn element_count() -> usize {
    ELEMENTS.len()
}
