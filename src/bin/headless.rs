//! Headless simulation harness — runs the real rollouts without any graphics so the
//! learning can be observed in a terminal. Drives a bare `bevy_ecs` World + Schedule
//! with a manually-advanced clock (deterministic, fast), printing aggregate stats.
//!
//! Build/run (needs the `headless` feature, which omits Bevy's GUI features so it
//! compiles without GPU/windowing libraries):
//!
//! ```text
//! cargo run --release --no-default-features --features headless --bin headless
//! ```

use std::time::Duration;

use bevy::prelude::*;

use econ_sim::economy::{self, EconStats, HungerControl, IncomeControl};
use econ_sim::noot::{
    Action, Claim, Hunger, Inventory, Noot, NootMeta, TilePos, Trader, Wallet, EXPLORE_MAX,
    EXPLORE_MIN, STARTING_BUCKS,
};
use econ_sim::policy::{ActorCritic, PolicyMemory, Trainer};
use econ_sim::rng::Rng;
use econ_sim::world::generate;
use econ_sim::{Sim, SimRng};

const COLS: i32 = 30;
const ROWS: i32 = 22;
const HEX_SIZE: f32 = 26.0;
const N_NOOTS: usize = 56;
const DT: f32 = 1.0 / 30.0; // simulated seconds per step
const MINUTES: f32 = 5.0;
const PRINT_EVERY_SECS: f32 = 15.0;

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0EC0_5EED);
    let world = generate(seed, COLS, ROWS, HEX_SIZE);
    let n_tiles = (world.cols * world.rows) as usize;
    let mut rng = Rng::new(seed ^ 0xA5A5_5A5A);
    let ac = ActorCritic::new(n_tiles, &mut rng);

    let mut w = World::new();
    w.insert_resource(HungerControl::new(
        economy::TARGET_DEATH_FRAC_PER_MIN * N_NOOTS as f32,
    ));
    w.insert_resource(IncomeControl::default());
    w.insert_resource(EconStats::default());
    w.insert_resource(ac);
    w.insert_resource(Trainer::default());
    w.insert_resource(Time::<()>::default());

    for _ in 0..N_NOOTS {
        let col = rng.below(COLS as usize) as i32;
        let row = rng.below(ROWS as usize) as i32;
        w.spawn((
            Noot,
            Action::default(),
            Claim::new(None),
            Trader::new(),
            NootMeta::new(),
            TilePos { col, row },
            Inventory::new(),
            Wallet {
                bucks: STARTING_BUCKS,
            },
            Hunger::fresh(&mut rng),
            PolicyMemory::new(rng.range(EXPLORE_MIN, EXPLORE_MAX)),
        ));
    }
    w.insert_resource(Sim(world));
    w.insert_resource(SimRng(rng));

    // Same order the GUI app runs the sim groups in.
    let mut sched = Schedule::default();
    sched.add_systems(
        (
            economy::simulate,
            economy::income,
            economy::income_controller,
            economy::hunger_tick,
            economy::hunger_pid,
            economy::age_noots,
            economy::policy_step,
            economy::claim_deposits,
            economy::extract,
            economy::refine,
            economy::meet_and_trade,
            economy::consume,
            economy::death_and_respawn,
            economy::update_rates,
            economy::train_policy,
        )
            .chain(),
    );

    let steps = (MINUTES * 60.0 / DT) as usize;
    let print_every = (PRINT_EVERY_SECS / DT) as usize;
    println!(
        "headless seed {:#x}: {} noots, {} deposits, {} steps ({:.0} min @ {:.0} Hz)",
        seed,
        N_NOOTS,
        w.resource::<Sim>().0.deposits.len(),
        steps,
        MINUTES,
        1.0 / DT
    );
    println!("   t  prod  cons  util  trades | deaths/min(tgt) hung starv | claim | bucks  exp | move/mine/refine | income infl%");

    for step in 0..=steps {
        if step % print_every == 0 {
            print_stats(&mut w, step as f32 * DT);
        }
        w.resource_mut::<Time>().advance_by(Duration::from_secs_f32(DT));
        sched.run(&mut w);
    }
}

fn print_stats(w: &mut World, t: f32) {
    let (prod, cons, util, trades) = {
        let s = w.resource::<EconStats>();
        (
            s.production_rate,
            s.consumption_rate,
            s.utility_rate,
            s.trades_total,
        )
    };
    let (dpm, tgt) = {
        let h = w.resource::<HungerControl>();
        (h.measured_per_min, h.target_per_min)
    };
    let (irate, infl) = {
        let i = w.resource::<IncomeControl>();
        (i.rate, i.measured_inflation)
    };

    let mut q = w.query::<(&Action, &Hunger, &Claim, &Wallet, &NootMeta)>();
    let mut act = [0u32; 3];
    let (mut starving, mut claimed, mut n) = (0u32, 0u32, 0u32);
    let (mut bucks, mut exp, mut hsum) = (0.0f32, 0.0f32, 0.0f32);
    for (a, h, c, wal, m) in q.iter(w) {
        match a {
            Action::Move => act[0] += 1,
            Action::Mine => act[1] += 1,
            Action::Refine => act[2] += 1,
        }
        if h.is_starving() {
            starving += 1;
        }
        if c.deposit.is_some() {
            claimed += 1;
        }
        bucks += wal.bucks;
        exp += m.experience;
        hsum += h.staple.iter().sum::<f32>() / h.staple.len() as f32;
        n += 1;
    }
    let nf = n.max(1) as f32;
    println!(
        "{:5.0} {:5.1} {:5.1} {:5.2} {:6} | {:5.2}({:.2}) {:4.1} {:3} | {:3} | {:5.0} {:4.0} | {:3}/{:3}/{:3} | {:.2} {:+.2}",
        t, prod, cons, util, trades,
        dpm, tgt, hsum / nf, starving,
        claimed,
        bucks / nf, exp / nf,
        act[0], act[1], act[2],
        irate, infl * 100.0,
    );
}
