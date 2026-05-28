//! Bounded, whole-run time-series history for the HUD graphs.
//!
//! A [`RollupHistory`] keeps the *entire* run at a fixed memory budget: recent samples
//! stay at full resolution while older ones are progressively averaged into wider
//! buckets. When the bucket count would exceed [`HISTORY_CAP`], the oldest adjacent pair
//! of equal resolution is merged (weighted by how many raw samples each covers), which
//! yields roughly power-of-two tiers — newest buckets span one sample, growing toward the
//! past. Rendering plots buckets index-spaced, so the old (rolled-up) end is
//! time-compressed: an at-a-glance overview of the whole run that survives save/reload.
//!
//! Lives in the (always-compiled) core rather than the gui-only `graph` module so the
//! save layer can persist it without pulling in any rendering code.

use serde::{Deserialize, Serialize};

/// Number of stat series in the graph strip. Kept here (not in the gui-only `graph`
/// module) so the persisted history can be sized without the gui feature; `graph::SERIES`
/// is declared with exactly this many entries, so the two can't drift.
pub const N_STAT_SERIES: usize = 16;

/// Maximum buckets a [`RollupHistory`] retains. Older buckets are downsampled to stay
/// within this, so the series spans the whole run at bounded size (and bounded save size).
pub const HISTORY_CAP: usize = 480;

/// A time series of `N`-wide samples that retains the whole run at bounded size by
/// rolling up (averaging) older windows. See the module docs for the tiering scheme.
///
/// The const `N` types the [`push`](Self::push) API and the merge width; buckets are
/// stored as `Vec<f32>` (each of length `N`) because serde has no blanket impl for
/// const-generic arrays.
#[derive(Clone, Serialize, Deserialize)]
pub struct RollupHistory<const N: usize> {
    /// Bucket means (each `N` wide), oldest → newest.
    buckets: Vec<Vec<f32>>,
    /// How many raw samples each bucket covers (parallel to `buckets`). The newest are 1;
    /// they grow toward the past as rollups merge them.
    spans: Vec<u32>,
}

impl<const N: usize> Default for RollupHistory<N> {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            spans: Vec::new(),
        }
    }
}

impl<const N: usize> RollupHistory<N> {
    /// Append a fresh full-resolution sample, rolling up the oldest end if over cap.
    pub fn push(&mut self, sample: [f32; N]) {
        self.buckets.push(sample.to_vec());
        self.spans.push(1);
        if self.buckets.len() > HISTORY_CAP {
            self.compact();
        }
    }

    /// Remove exactly one bucket by merging the oldest adjacent pair of *equal* span
    /// (falling back to the two oldest), so resolution coarsens from the old end first.
    fn compact(&mut self) {
        let n = self.buckets.len();
        // Earliest index whose span matches its right neighbour (finest, oldest pair).
        let merge_at = (0..n - 1)
            .find(|&i| self.spans[i] == self.spans[i + 1])
            .unwrap_or(0);
        let (a, b) = (merge_at, merge_at + 1);
        let (sa, sb) = (self.spans[a] as f32, self.spans[b] as f32);
        let total = sa + sb;
        let merged: Vec<f32> = (0..N)
            .map(|k| (self.buckets[a][k] * sa + self.buckets[b][k] * sb) / total)
            .collect();
        self.buckets[a] = merged;
        self.spans[a] += self.spans[b];
        self.buckets.remove(b);
        self.spans.remove(b);
    }

    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Bucket means oldest → newest (what the sparklines plot), each slice `N` wide.
    pub fn iter(&self) -> impl Iterator<Item = &[f32]> {
        self.buckets.iter().map(Vec::as_slice)
    }

    /// The most recent (full-resolution) sample, if any.
    pub fn back(&self) -> Option<&[f32]> {
        self.buckets.last().map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_within_cap_and_keeps_newest_full_res() {
        let mut h = RollupHistory::<1>::default();
        for i in 0..(HISTORY_CAP * 10) {
            h.push([i as f32]);
        }
        assert!(h.len() <= HISTORY_CAP);
        // The newest bucket is the last raw sample, untouched by rollups.
        assert_eq!(h.back().unwrap()[0], (HISTORY_CAP * 10 - 1) as f32);
    }

    #[test]
    fn rollup_preserves_the_running_mean_roughly() {
        // Pushing a constant must average to that constant regardless of rollups.
        let mut h = RollupHistory::<1>::default();
        for _ in 0..(HISTORY_CAP * 5) {
            h.push([7.0]);
        }
        for b in h.iter() {
            assert!((b[0] - 7.0).abs() < 1e-3);
        }
    }

    #[test]
    fn survives_a_json_round_trip() {
        // The whole point of the feature: history persists through a save/reload.
        let mut h = RollupHistory::<3>::default();
        for i in 0..(HISTORY_CAP * 3) {
            let f = i as f32;
            h.push([f, f * 2.0, f * 0.5]);
        }
        let json = serde_json::to_string(&h).unwrap();
        let back: RollupHistory<3> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), h.len());
        let (a, b): (Vec<_>, Vec<_>) = (h.iter().collect(), back.iter().collect());
        assert_eq!(a, b);
    }
}
