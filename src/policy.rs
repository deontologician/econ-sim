//! Shared off-policy actor-critic that drives noot behavior (the learned policy that
//! replaces the old `choose_action` heuristic and the per-hex TD navigation).
//!
//! One [`ActorCritic`] brain is shared by every noot; per-noot diversity comes from an
//! exploration ε. Decisions pick among 4 actions (Move/Mine/Refine/Trade); when Move,
//! the critic's value over absolute position chooses the next hex. Trained A2C-style
//! from a shared replay buffer with a slow target critic.
//!
//! The forward pass and backprop were validated against a finite-difference gradient
//! check before landing. Game-specific bits (the Maslow utility, featurization, action
//! masks) live in `economy.rs`; this module is the pure RL machinery.

// This is a numeric kernel: the forward/backward/optimizer loops walk several
// parallel flat weight arrays by the same index (weights, grads, velocities), where
// indexing reads far clearer than zipping a handful of iterators together.
#![allow(clippy::needless_range_loop)]

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::rng::Rng;

/// Action layout: indices 0..6 are the six hex move directions (relative steps),
/// then Mine and Refine. Trading is automatic (not an action) — see `meet_and_trade`.
pub const N_DIRS: usize = 6;
pub const A_MINE: usize = N_DIRS;
pub const A_REFINE: usize = N_DIRS + 1;
pub const N_ACT: usize = N_DIRS + 2;

/// Non-positional state features (position is a separate embedding lookup).
pub const N_OTHER: usize = 6;
/// Hidden width of the shared trunk.
pub const H: usize = 32;

// --- Training hyperparameters ----------------------------------------------
const GAMMA: f32 = 0.95;
const LR: f32 = 1e-3;
const MOMENTUM: f32 = 0.9;
const TAU: f32 = 0.01; // Polyak averaging for the target critic
const ENTROPY_BETA: f32 = 0.01;
const VALUE_COEF: f32 = 0.5;
const ADV_CLIP: f32 = 5.0;
const BATCH: usize = 32;
const WARMUP: usize = 256;
const BUFFER_CAP: usize = 16_384;

/// The shared brain: a 1-hidden-layer MLP with a per-tile position embedding feeding a
/// tanh trunk, then an actor head (4 logits) and a critic head (scalar value). Weights
/// are flat `Vec<f32>` so it serializes straight into the save.
#[derive(Resource, Clone, Serialize, Deserialize, Default)]
pub struct ActorCritic {
    pub n_tiles: usize,
    embed: Vec<f32>,   // n_tiles * H  (position → trunk contribution)
    w_other: Vec<f32>, // H * N_OTHER
    b1: Vec<f32>,      // H
    wa: Vec<f32>,      // N_ACT * H
    ba: Vec<f32>,      // N_ACT
    wv: Vec<f32>,      // H
    bv: f32,
}

impl ActorCritic {
    /// Fresh net sized to `n_tiles`, small random weights (deterministic from `rng`).
    pub fn new(n_tiles: usize, rng: &mut Rng) -> Self {
        let mut r = |scale: f32| (rng.next_f32() * 2.0 - 1.0) * scale;
        Self {
            n_tiles,
            embed: (0..n_tiles * H).map(|_| r(0.1)).collect(),
            w_other: (0..H * N_OTHER).map(|_| r(0.3)).collect(),
            b1: vec![0.0; H],
            wa: (0..N_ACT * H).map(|_| r(0.3)).collect(),
            ba: vec![0.0; N_ACT],
            wv: (0..H).map(|_| r(0.3)).collect(),
            bv: 0.0,
        }
    }

    /// Whether a (deserialized) net matches the current architecture and map size.
    /// A save from a different tile count or action layout fails this and is replaced
    /// with a fresh net rather than indexed out of bounds.
    pub fn fits(&self, n_tiles: usize) -> bool {
        self.n_tiles == n_tiles
            && self.embed.len() == n_tiles * H
            && self.wa.len() == N_ACT * H
            && self.wv.len() == H
    }

    /// A same-shaped all-zero net (for gradient/velocity accumulators).
    fn zeros_like(&self) -> Self {
        Self {
            n_tiles: self.n_tiles,
            embed: vec![0.0; self.embed.len()],
            w_other: vec![0.0; self.w_other.len()],
            b1: vec![0.0; H],
            wa: vec![0.0; N_ACT * H],
            ba: vec![0.0; N_ACT],
            wv: vec![0.0; H],
            bv: 0.0,
        }
    }

    /// Trunk hidden activations `tanh(embed[pos] + W_other·o + b1)`.
    fn hidden(&self, pos: usize, o: &[f32; N_OTHER]) -> [f32; H] {
        let mut h = [0.0f32; H];
        let base = pos * H;
        for (j, hj) in h.iter_mut().enumerate() {
            let mut z = self.embed[base + j] + self.b1[j];
            let wrow = j * N_OTHER;
            for (k, &ok) in o.iter().enumerate() {
                z += self.w_other[wrow + k] * ok;
            }
            *hj = z.tanh();
        }
        h
    }

    fn logits_from_hidden(&self, h: &[f32; H]) -> [f32; N_ACT] {
        let mut out = [0.0f32; N_ACT];
        for (a, oa) in out.iter_mut().enumerate() {
            let mut s = self.ba[a];
            let wrow = a * H;
            for (j, &hj) in h.iter().enumerate() {
                s += self.wa[wrow + j] * hj;
            }
            *oa = s;
        }
        out
    }

    fn value_from_hidden(&self, h: &[f32; H]) -> f32 {
        let mut v = self.bv;
        for (j, &hj) in h.iter().enumerate() {
            v += self.wv[j] * hj;
        }
        v
    }

    /// Critic value of a state — used both for the bootstrap target and (over
    /// neighbouring positions) to steer the Move action.
    pub fn value(&self, pos: usize, o: &[f32; N_OTHER]) -> f32 {
        let h = self.hidden(pos, o);
        self.value_from_hidden(&h)
    }

    /// Actor logits at a state.
    pub fn logits(&self, pos: usize, o: &[f32; N_OTHER]) -> [f32; N_ACT] {
        let h = self.hidden(pos, o);
        self.logits_from_hidden(&h)
    }

    /// Accumulate the A2C gradient of one transition into `grad`. `adv` and `y` are
    /// treated as constants (detached), matching the validated backprop.
    #[allow(clippy::too_many_arguments)]
    fn backward_accumulate(
        &self,
        grad: &mut ActorCritic,
        pos: usize,
        o: &[f32; N_OTHER],
        mask: &[bool; N_ACT],
        act: usize,
        y: f32,
        adv: f32,
    ) {
        let h = self.hidden(pos, o);
        let logits = self.logits_from_hidden(&h);
        let pi = masked_softmax(&logits, mask);
        let mut ent = 0.0f32;
        for a in 0..N_ACT {
            if mask[a] && pi[a] > 0.0 {
                ent -= pi[a] * pi[a].ln();
            }
        }
        let v = self.value_from_hidden(&h);

        // Gradient on each action logit (policy gradient + entropy bonus).
        let mut glog = [0.0f32; N_ACT];
        for i in 0..N_ACT {
            if !mask[i] {
                continue;
            }
            let pg = adv * (pi[i] - if i == act { 1.0 } else { 0.0 });
            let eg = ENTROPY_BETA * pi[i] * (pi[i].max(1e-12).ln() + ent);
            glog[i] = pg + eg;
        }
        let dv = VALUE_COEF * (v - y);

        // Heads.
        for j in 0..H {
            grad.wv[j] += dv * h[j];
            for a in 0..N_ACT {
                grad.wa[a * H + j] += glog[a] * h[j];
            }
        }
        grad.bv += dv;
        for a in 0..N_ACT {
            grad.ba[a] += glog[a];
        }

        // Backprop into the trunk.
        let base = pos * H;
        for j in 0..H {
            let mut dh = self.wv[j] * dv;
            for a in 0..N_ACT {
                dh += self.wa[a * H + j] * glog[a];
            }
            let dz = dh * (1.0 - h[j] * h[j]);
            grad.b1[j] += dz;
            for (k, &ok) in o.iter().enumerate() {
                grad.w_other[j * N_OTHER + k] += dz * ok;
            }
            grad.embed[base + j] += dz;
        }
    }

    /// SGD-with-momentum step: `online -= LR · velocity`, `velocity = MOMENTUM·velocity
    /// + grad/BATCH`. `vel` mirrors the weight layout.
    fn apply(&mut self, grad: &ActorCritic, vel: &mut ActorCritic, inv_batch: f32) {
        macro_rules! step {
            ($field:ident) => {
                for i in 0..self.$field.len() {
                    vel.$field[i] = MOMENTUM * vel.$field[i] + grad.$field[i] * inv_batch;
                    self.$field[i] -= LR * vel.$field[i];
                }
            };
        }
        step!(embed);
        step!(w_other);
        step!(b1);
        step!(wa);
        step!(ba);
        step!(wv);
        vel.bv = MOMENTUM * vel.bv + grad.bv * inv_batch;
        self.bv -= LR * vel.bv;
    }

    /// Polyak-average `self` toward `online` (used to drift the target critic).
    fn polyak_toward(&mut self, online: &ActorCritic) {
        macro_rules! mv {
            ($field:ident) => {
                for i in 0..self.$field.len() {
                    self.$field[i] += TAU * (online.$field[i] - self.$field[i]);
                }
            };
        }
        mv!(embed);
        mv!(w_other);
        mv!(b1);
        mv!(wa);
        mv!(ba);
        mv!(wv);
        self.bv += TAU * (online.bv - self.bv);
    }
}

/// Softmax over the valid (unmasked) actions; masked entries are 0.
pub fn masked_softmax(logits: &[f32; N_ACT], mask: &[bool; N_ACT]) -> [f32; N_ACT] {
    let mut m = f32::MIN;
    for a in 0..N_ACT {
        if mask[a] && logits[a] > m {
            m = logits[a];
        }
    }
    let mut out = [0.0f32; N_ACT];
    let mut sum = 0.0f32;
    for a in 0..N_ACT {
        if mask[a] {
            out[a] = (logits[a] - m).exp();
            sum += out[a];
        }
    }
    if sum > 0.0 {
        for v in &mut out {
            *v /= sum;
        }
    }
    out
}

/// Sample an action index from a probability vector.
pub fn sample(probs: &[f32; N_ACT], rng: &mut Rng) -> usize {
    let r = rng.next_f32();
    let mut acc = 0.0;
    for (a, &p) in probs.iter().enumerate() {
        acc += p;
        if r < acc {
            return a;
        }
    }
    // Fallback (numerical drift): last valid action.
    (0..N_ACT).rev().find(|&a| probs[a] > 0.0).unwrap_or(0)
}

// --- Replay buffer ----------------------------------------------------------
/// One MDP transition. `mask` is the action mask at `s` (needed for the actor's
/// masked softmax); the V-critic bootstrap needs no mask at `s2`.
#[derive(Clone)]
pub struct Transition {
    pub pos: usize,
    pub o: [f32; N_OTHER],
    pub mask: [bool; N_ACT],
    pub act: usize,
    pub r: f32,
    pub pos2: usize,
    pub o2: [f32; N_OTHER],
    pub done: bool,
}

#[derive(Default)]
struct ReplayBuffer {
    items: Vec<Transition>,
    head: usize,
}

impl ReplayBuffer {
    fn push(&mut self, t: Transition) {
        if self.items.len() < BUFFER_CAP {
            self.items.push(t);
        } else {
            self.items[self.head] = t;
            self.head = (self.head + 1) % BUFFER_CAP;
        }
    }
    fn sample(&self, rng: &mut Rng) -> &Transition {
        &self.items[rng.below(self.items.len())]
    }
}

/// Transient training state (target critic, momentum velocities, replay buffer).
/// Never serialized — rebuilt to match the online net's shape on demand.
#[derive(Resource, Default)]
pub struct Trainer {
    target: ActorCritic,
    vel: ActorCritic,
    buffer: ReplayBuffer,
    sized: bool,
}

impl Trainer {
    pub fn record(&mut self, t: Transition) {
        self.buffer.push(t);
    }

    fn ensure_sized(&mut self, online: &ActorCritic) {
        if !self.sized || self.target.n_tiles != online.n_tiles {
            self.target = online.clone();
            self.vel = online.zeros_like();
            self.sized = true;
        }
    }

    /// One minibatch A2C update on `online` (no-op until the buffer is warm).
    pub fn train(&mut self, online: &mut ActorCritic, rng: &mut Rng) {
        if self.buffer.items.len() < WARMUP {
            return;
        }
        self.ensure_sized(online);
        let mut grad = online.zeros_like();
        for _ in 0..BATCH {
            let t = self.buffer.sample(rng).clone();
            let v = online.value(t.pos, &t.o);
            let v2 = if t.done {
                0.0
            } else {
                self.target.value(t.pos2, &t.o2)
            };
            let y = t.r + GAMMA * v2;
            let adv = (y - v).clamp(-ADV_CLIP, ADV_CLIP);
            online.backward_accumulate(&mut grad, t.pos, &t.o, &t.mask, t.act, y, adv);
        }
        online.apply(&grad, &mut self.vel, 1.0 / BATCH as f32);
        self.target.polyak_toward(online);
    }
}

/// Per-noot policy state: exploration ε, decision cadence, the cached last
/// (state, action) for forming the next transition, and a death flag. Transient
/// (not serialized; reset on spawn/respawn).
#[derive(Component)]
pub struct PolicyMemory {
    pub explore: f32,
    pub cooldown: f32,
    pub has_prev: bool,
    pub last_pos: usize,
    pub last_o: [f32; N_OTHER],
    pub last_mask: [bool; N_ACT],
    pub last_act: usize,
    pub last_u: f32,
    pub died: bool,
}

impl PolicyMemory {
    pub fn new(explore: f32) -> Self {
        Self {
            explore,
            cooldown: 0.0,
            has_prev: false,
            last_pos: 0,
            last_o: [0.0; N_OTHER],
            last_mask: [true; N_ACT],
            last_act: 0,
            last_u: 0.0,
            died: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn masked_softmax_sums_to_one_and_masks() {
        let p = masked_softmax(&[1.0, 2.0, 3.0, 0.5], &[true, false, true, true]);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert_eq!(p[1], 0.0);
        assert!(p[2] > p[0]); // higher logit → higher probability
    }

    #[test]
    fn sample_returns_the_only_valid_action() {
        let mut rng = Rng::new(42);
        assert_eq!(sample(&[0.0, 1.0, 0.0, 0.0], &mut rng), A_MINE);
    }

    #[test]
    fn net_is_sized_and_finite() {
        let mut rng = Rng::new(7);
        let ac = ActorCritic::new(50, &mut rng);
        assert_eq!(ac.n_tiles, 50);
        let o = [0.1, 0.2, 0.3, 0.0, 1.0, 1.0];
        assert!(ac.value(10, &o).is_finite());
        assert!(ac.logits(10, &o).iter().all(|x| x.is_finite()));
    }

    #[test]
    fn training_raises_value_of_a_rewarded_state() {
        let mut rng = Rng::new(1);
        let mut ac = ActorCritic::new(20, &mut rng);
        let mut tr = Trainer::default();
        let o = [0.0; N_OTHER];
        let before = ac.value(5, &o);
        // A terminal transition with reward 1: V(5) should learn toward 1.
        for _ in 0..(WARMUP + 100) {
            tr.record(Transition {
                pos: 5,
                o,
                mask: [true; N_ACT],
                act: A_MINE,
                r: 1.0,
                pos2: 5,
                o2: o,
                done: true,
            });
        }
        for _ in 0..200 {
            tr.train(&mut ac, &mut rng);
        }
        assert!(ac.value(5, &o) > before);
    }
}
