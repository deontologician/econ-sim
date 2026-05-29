//! Deterministic fantasy world name from the world seed, so a world always carries the same
//! name across save/reload and the leaderboard server can derive it from the seed too.

use crate::rng::Rng;

// Built syllable-by-syllable: onset + middle + ending, e.g. "Top" + "ar" + "lia" = Toparlia.
const ONSET: [&str; 28] = [
    "Top", "Mar", "Vel", "Cor", "Zan", "Bel", "Thar", "Qua", "Lor", "Syl", "Drav", "Eld", "Fen",
    "Gor", "Hal", "Ish", "Kael", "Nim", "Oss", "Pyr", "Rho", "Tal", "Umb", "Vyr", "Wen", "Xan",
    "Yor", "Zeph",
];
const MIDDLE: [&str; 12] = [
    "ar", "or", "el", "an", "il", "un", "ad", "en", "ys", "ol", "ir", "am",
];
const ENDING: [&str; 16] = [
    "lia", "ria", "dor", "neth", "mar", "wyn", "gard", "heim", "vale", "spire", "reach", "fell",
    "moor", "crest", "thys", "dell",
];

/// A stable fantasy name for the world with this seed (e.g. "Toparlia"). Salted so it doesn't
/// correlate with worldgen's own use of the seed.
pub fn world_name(seed: u64) -> String {
    let mut rng = Rng::new(seed ^ 0x_5EED_FACE_5A17_0001);
    let onset = ONSET[rng.below(ONSET.len())];
    let middle = MIDDLE[rng.below(MIDDLE.len())];
    let ending = ENDING[rng.below(ENDING.len())];
    format!("{onset}{middle}{ending}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_nonempty() {
        let a = world_name(0x0EC0_5EED);
        assert_eq!(a, world_name(0x0EC0_5EED)); // deterministic
        assert!(a.len() >= 5);
        // Different seeds usually differ (not a hard guarantee, but should here).
        assert_ne!(world_name(1), world_name(2));
    }
}
