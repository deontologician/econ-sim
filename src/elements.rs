//! The pool of ~25 candidate "elements". Each playthrough draws four of these
//! to be the building blocks of that world. Colours are sRGB and used to tint
//! the deposit markers on the map.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ElementId(pub usize);

pub struct ElementDef {
    pub name: &'static str,
    pub color: (f32, f32, f32),
}

pub const ELEMENTS: [ElementDef; 25] = [
    ElementDef { name: "Lightning", color: (0.96, 0.90, 0.30) },
    ElementDef { name: "Fire", color: (0.95, 0.35, 0.15) },
    ElementDef { name: "Slime", color: (0.45, 0.85, 0.35) },
    ElementDef { name: "Acid", color: (0.66, 0.95, 0.20) },
    ElementDef { name: "Water", color: (0.25, 0.55, 0.95) },
    ElementDef { name: "Sugar", color: (0.98, 0.92, 0.88) },
    ElementDef { name: "Wood", color: (0.55, 0.38, 0.20) },
    ElementDef { name: "Ice", color: (0.70, 0.90, 0.98) },
    ElementDef { name: "Stone", color: (0.55, 0.55, 0.58) },
    ElementDef { name: "Sand", color: (0.93, 0.85, 0.55) },
    ElementDef { name: "Oil", color: (0.18, 0.15, 0.22) },
    ElementDef { name: "Gold", color: (0.95, 0.78, 0.25) },
    ElementDef { name: "Iron", color: (0.60, 0.62, 0.66) },
    ElementDef { name: "Salt", color: (0.92, 0.92, 0.96) },
    ElementDef { name: "Steam", color: (0.85, 0.88, 0.92) },
    ElementDef { name: "Ash", color: (0.42, 0.40, 0.38) },
    ElementDef { name: "Crystal", color: (0.70, 0.85, 0.96) },
    ElementDef { name: "Mud", color: (0.42, 0.32, 0.22) },
    ElementDef { name: "Smoke", color: (0.52, 0.52, 0.55) },
    ElementDef { name: "Wind", color: (0.80, 0.92, 0.85) },
    ElementDef { name: "Light", color: (1.00, 0.97, 0.80) },
    ElementDef { name: "Shadow", color: (0.22, 0.20, 0.30) },
    ElementDef { name: "Copper", color: (0.80, 0.50, 0.30) },
    ElementDef { name: "Sulfur", color: (0.92, 0.85, 0.30) },
    ElementDef { name: "Honey", color: (0.90, 0.65, 0.20) },
];

pub fn element(id: ElementId) -> &'static ElementDef {
    &ELEMENTS[id.0]
}

pub fn element_count() -> usize {
    ELEMENTS.len()
}
