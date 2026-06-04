//! Shared tech-tree types and the **deterministic, pure** growth logic for the global,
//! live tech tree (plans/035, Phase 2). The leaderboard server (`bin/server.rs`) aggregates
//! per-resource research demand reported by every world and, slowly over time, grows new
//! techs whose inputs are the in-demand resources, with randomized attributes. The client
//! (Phase 3) fetches the tree and applies it.
//!
//! This module is I/O-free and LLM-free: it builds the *draft* of a tech (inputs +
//! randomized attributes) and the naming *prompt*, and provides a `procedural_name` fallback.
//! The actual network naming call lives in the server binary (so the wasm bundle never pulls
//! an HTTP client), which falls back to `procedural_name` when the LLM is unavailable.

use crate::elements::{element, ElementId, ELEMENTS};
use crate::goods::{GoodCategory, GoodForm};
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

/// Number of distinct *global* good identities demand is tracked over: every element in both
/// forms (`Raw`/`Refined`). Stable across all worlds, so the server can sum demand without
/// knowing any world's per-world item table.
pub const N_GLOBAL_ITEMS: usize = ELEMENTS.len() * 2;

/// Global item index for an `(element, form)` pair: `element*2 + form`.
pub fn global_index(id: ElementId, form: GoodForm) -> usize {
    id.0 * 2 + form as usize
}

/// Inverse of [`global_index`].
pub fn global_parts(idx: usize) -> (ElementId, GoodForm) {
    (
        ElementId(idx / 2),
        if idx % 2 == 1 {
            GoodForm::Refined
        } else {
            GoodForm::Raw
        },
    )
}

/// What a constructed tech *does* in the sim. Magnitude is carried separately on the `Tech`.
/// The client applies these in Phase 3; the server only randomizes and stores them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TechEffect {
    /// Edible: eating one reduces hunger (magnitude = hunger removed).
    Nourish,
    /// Held wealth: contributes to positional/esteem utility (magnitude = weight).
    Esteem,
    /// Raises extraction efficiency for its owner/world (magnitude = multiplier ≥ 1).
    ExtractBoost,
    /// Raises refining speed (magnitude = multiplier ≥ 1).
    RefineBoost,
    /// Raises carry capacity (magnitude = multiplier ≥ 1).
    CarryBoost,
}

/// All effect variants, for uniform random selection during growth.
pub const EFFECTS: [TechEffect; 5] = [
    TechEffect::Nourish,
    TechEffect::Esteem,
    TechEffect::ExtractBoost,
    TechEffect::RefineBoost,
    TechEffect::CarryBoost,
];

impl TechEffect {
    /// A short human label (HTML/CLI display, naming prompt).
    pub fn label(self) -> &'static str {
        match self {
            TechEffect::Nourish => "nourishing",
            TechEffect::Esteem => "prestigious",
            TechEffect::ExtractBoost => "extraction-boosting",
            TechEffect::RefineBoost => "refining-boosting",
            TechEffect::CarryBoost => "carry-boosting",
        }
    }

    /// A plausible random magnitude for this effect (consumable food values are larger than
    /// the multiplicative boosts, which sit just above 1×).
    fn random_magnitude(self, rng: &mut Rng) -> f32 {
        match self {
            TechEffect::Nourish => rng.range(2.0, 8.0),
            TechEffect::Esteem => rng.range(1.0, 4.0),
            TechEffect::ExtractBoost | TechEffect::RefineBoost | TechEffect::CarryBoost => {
                rng.range(1.15, 2.0)
            }
        }
    }

    /// Procedural-name suffix evoking the effect.
    fn suffix(self) -> &'static str {
        match self {
            TechEffect::Nourish => "Ration",
            TechEffect::Esteem => "Charm",
            TechEffect::ExtractBoost => "Drill",
            TechEffect::RefineBoost => "Forge",
            TechEffect::CarryBoost => "Pack",
        }
    }
}

/// One ingredient of a tech's recipe, by global element identity.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TechInput {
    pub element: ElementId,
    pub form: GoodForm,
    pub qty: u8,
}

impl TechInput {
    /// The good's display name (refined or raw element name).
    pub fn good_name(&self) -> &'static str {
        let def = element(self.element);
        match self.form {
            GoodForm::Raw => def.name,
            GoodForm::Refined => def.refined,
        }
    }
}

/// The randomized core of a new tech, before it's assigned an id, a name, and a timestamp.
/// Produced purely from demand + an RNG so it's unit-testable; the server turns it into a
/// [`Tech`].
#[derive(Clone, PartialEq, Debug)]
pub struct TechDraft {
    pub inputs: Vec<TechInput>,
    pub effect: TechEffect,
    pub magnitude: f32,
    pub consumable: bool,
    pub category: GoodCategory,
    pub tier: u8,
}

/// A grown tech: a recipe (inputs) that constructs one output good with the rolled
/// attributes. Shared between server (grows it) and client (applies it).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Tech {
    pub id: u64,
    pub name: String,
    pub inputs: Vec<TechInput>,
    pub effect: TechEffect,
    pub magnitude: f32,
    pub consumable: bool,
    pub category: GoodCategory,
    pub tier: u8,
    /// Unix seconds when grown — ordering and the "slowly over time" readout.
    pub created_unix: u64,
}

impl Tech {
    /// Assemble a [`Tech`] from a draft plus the server-assigned id/name/time.
    pub fn from_draft(draft: TechDraft, id: u64, name: String, created_unix: u64) -> Self {
        Self {
            id,
            name,
            inputs: draft.inputs,
            effect: draft.effect,
            magnitude: draft.magnitude,
            consumable: draft.consumable,
            category: draft.category,
            tier: draft.tier,
            created_unix,
        }
    }

    /// `"Iron + Plank"` style recipe rendering.
    pub fn inputs_label(&self) -> String {
        self.inputs
            .iter()
            .map(|i| {
                if i.qty > 1 {
                    format!("{}×{}", i.qty, i.good_name())
                } else {
                    i.good_name().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// The tree the server serves and the client fetches.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TechTree {
    pub techs: Vec<Tech>,
}

// --- Growth (pure, deterministic given the RNG) -----------------------------

/// Up to this many inputs per tech recipe.
const MAX_INPUTS: usize = 3;
/// Demand at which the per-attempt growth chance reaches half of [`MAX_GROW_CHANCE`]
/// (logistic in total demand). Research demand accrues fast (~thousands per world over a few
/// thousand ticks), so this is set high enough that a lone, briefly-running world doesn't
/// immediately spawn techs.
const GROW_DEMAND_SCALE: f64 = 20_000.0;
/// Ceiling on the per-attempt growth probability, so growth stays gradual even under heavy
/// aggregate demand.
const MAX_GROW_CHANCE: f32 = 0.5;

/// Per-attempt probability that a new tech is grown, rising with total aggregate demand and
/// saturating at [`MAX_GROW_CHANCE`]. The server rolls this once per growth tick.
pub fn growth_chance(total_demand: f64) -> f32 {
    if total_demand <= 0.0 {
        return 0.0;
    }
    let x = (total_demand / GROW_DEMAND_SCALE) as f32;
    (x / (1.0 + x)).min(MAX_GROW_CHANCE)
}

/// Sample one global item index ∝ demand, skipping any already in `exclude`. `None` if no
/// eligible demand remains.
pub fn weighted_pick(demand: &[f64], exclude: &[usize], rng: &mut Rng) -> Option<usize> {
    let total: f64 = demand
        .iter()
        .enumerate()
        .filter(|(i, _)| !exclude.contains(i))
        .map(|(_, &d)| d.max(0.0))
        .sum();
    if total <= 0.0 {
        return None;
    }
    let mut r = rng.next_f32() as f64 * total;
    for (i, &d) in demand.iter().enumerate() {
        if exclude.contains(&i) || d <= 0.0 {
            continue;
        }
        r -= d;
        if r <= 0.0 {
            return Some(i);
        }
    }
    // Floating-point slack: return the last eligible index.
    (0..demand.len())
        .rev()
        .find(|i| !exclude.contains(i) && demand[*i] > 0.0)
}

/// Draft a new tech from the aggregate demand histogram: 1–[`MAX_INPUTS`] distinct inputs
/// sampled ∝ demand, then randomized attributes. `None` when there's no demand to draw from.
pub fn draft_tech(demand: &[f64], rng: &mut Rng) -> Option<TechDraft> {
    let n_inputs = 1 + rng.below(MAX_INPUTS);
    let mut chosen: Vec<usize> = Vec::new();
    for _ in 0..n_inputs {
        match weighted_pick(demand, &chosen, rng) {
            Some(gi) => chosen.push(gi),
            None => break,
        }
    }
    if chosen.is_empty() {
        return None;
    }
    let inputs: Vec<TechInput> = chosen
        .iter()
        .map(|&gi| {
            let (element, form) = global_parts(gi);
            TechInput {
                element,
                form,
                qty: 1 + rng.below(3) as u8,
            }
        })
        .collect();
    let effect = EFFECTS[rng.below(EFFECTS.len())];
    let tier = 1 + inputs.iter().filter(|i| i.form == GoodForm::Refined).count() as u8;
    Some(TechDraft {
        magnitude: effect.random_magnitude(rng),
        effect,
        consumable: rng.chance(0.5),
        category: if rng.chance(0.5) {
            GoodCategory::Staple
        } else {
            GoodCategory::Positional
        },
        tier,
        inputs,
    })
}

// --- Naming -----------------------------------------------------------------

/// Deterministic fallback name when the LLM is unavailable: the primary input's good name +
/// an effect-evoking suffix, e.g. `Iron Drill`, `Plank Ration`.
pub fn procedural_name(draft: &TechDraft) -> String {
    let primary = draft
        .inputs
        .first()
        .map(|i| i.good_name())
        .unwrap_or("Curio");
    format!("{} {}", primary, draft.effect.suffix())
}

/// The user-prompt half of the LLM naming request (the server adds the system instruction and
/// the HTTP plumbing). Describes the recipe and rolled attributes and asks for one short,
/// plausible item name — the example the user gave: `durable stick + iron → shovel`.
pub fn naming_prompt(draft: &TechDraft) -> String {
    let ingredients = draft
        .inputs
        .iter()
        .map(|i| i.good_name())
        .collect::<Vec<_>>()
        .join(" + ");
    let durability = if draft.consumable {
        "consumable"
    } else {
        "durable"
    };
    let category = match draft.category {
        GoodCategory::Staple => "everyday staple",
        GoodCategory::Positional => "luxury status good",
    };
    format!(
        "Invent a name for a crafted item made by combining: {ingredients}. \
         It is {durability}, a {category}, and is {} (tier {}). \
         Reply with ONLY the item name, 1-3 words, no quotes or punctuation. \
         For example, durable Wood + Iron might be \"Shovel\".",
        draft.effect.label(),
        draft.tier,
    )
}

/// Clean an LLM name response into a short, safe display name; empty/garbage → `None` so the
/// caller falls back to [`procedural_name`].
pub fn sanitize_name(raw: &str) -> Option<String> {
    let line = raw.lines().find(|l| !l.trim().is_empty())?.trim();
    let cleaned: String = line
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '.' || c == '*' || c.is_whitespace())
        .chars()
        .take(40)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_index_roundtrips() {
        for idx in 0..N_GLOBAL_ITEMS {
            let (id, form) = global_parts(idx);
            assert_eq!(global_index(id, form), idx);
        }
    }

    #[test]
    fn weighted_pick_respects_demand_and_exclude() {
        let mut demand = vec![0.0; N_GLOBAL_ITEMS];
        demand[4] = 10.0;
        demand[7] = 90.0;
        let mut rng = Rng::new(1);
        let mut counts = [0u32; 2];
        for _ in 0..2000 {
            match weighted_pick(&demand, &[], &mut rng) {
                Some(4) => counts[0] += 1,
                Some(7) => counts[1] += 1,
                other => panic!("picked outside demand: {other:?}"),
            }
        }
        // 7 is ~9× as likely as 4.
        assert!(counts[1] > counts[0] * 4, "counts={counts:?}");
        // Excluding the only heavy index forces the other.
        assert_eq!(weighted_pick(&demand, &[7], &mut rng), Some(4));
        assert_eq!(weighted_pick(&demand, &[4, 7], &mut rng), None);
    }

    #[test]
    fn draft_tech_pulls_from_demand_only() {
        let mut demand = vec![0.0; N_GLOBAL_ITEMS];
        demand[2] = 50.0;
        demand[3] = 50.0;
        let mut rng = Rng::new(42);
        for _ in 0..200 {
            let d = draft_tech(&demand, &mut rng).expect("demand present");
            assert!(!d.inputs.is_empty() && d.inputs.len() <= MAX_INPUTS);
            for i in &d.inputs {
                let gi = global_index(i.element, i.form);
                assert!(gi == 2 || gi == 3, "input from zero-demand slot: {gi}");
                assert!(i.qty >= 1 && i.qty <= 3);
            }
            assert!(d.magnitude > 0.0);
            assert!(!procedural_name(&d).is_empty());
            assert!(!naming_prompt(&d).is_empty());
        }
        // No demand ⇒ no draft.
        assert!(draft_tech(&vec![0.0; N_GLOBAL_ITEMS], &mut rng).is_none());
    }

    #[test]
    fn growth_chance_monotonic_and_capped() {
        assert_eq!(growth_chance(0.0), 0.0);
        assert!(growth_chance(1000.0) < growth_chance(100_000.0));
        assert!(growth_chance(1e12) <= MAX_GROW_CHANCE + 1e-6);
    }

    #[test]
    fn sanitize_trims_quotes_and_caps() {
        assert_eq!(sanitize_name("  \"Shovel\".  "), Some("Shovel".to_string()));
        assert_eq!(sanitize_name("Iron Drill\nextra"), Some("Iron Drill".to_string()));
        assert_eq!(sanitize_name("   "), None);
    }
}
