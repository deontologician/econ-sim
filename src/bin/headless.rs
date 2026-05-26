//! Headless simulation harness — runs the real rollouts without any graphics so the
//! learning can be observed and tuned offline. Drives a bare `bevy_ecs` World +
//! Schedule on the fixed simulation tick (no real-time clock: every system advances
//! the world by exactly one `economy::TICK_DT`), as fast as the CPU allows.
//!
//! Stdout is **JSONL**: one JSON object per sampled tick with the full econ stats
//! (rates are per-tick, so they're speed-invariant). Human-readable progress goes to
//! stderr, keeping stdout a clean stream to pipe into `jq`/pandas/etc.
//!
//! Build/run (needs the `headless` feature, which omits Bevy's GUI features so it
//! compiles without GPU/windowing libs):
//!
//! ```text
//! cargo run --release --no-default-features --features headless --bin headless \
//!     -- [seed] [ticks] [sample_every]
//! ```

use bevy::prelude::*;

use econ_sim::economy::{self, EconStats, HungerControl, IncomeControl};
use econ_sim::goods::ItemRole;
use econ_sim::noot::{
    Action, Claim, Hunger, Inventory, NootMeta, Noot, TilePos, Trader, Wallet, EXPLORE_MAX,
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

const DEFAULT_TICKS: u64 = 60_000;
const DEFAULT_SAMPLE_EVERY: u64 = 600;

fn arg<T: std::str::FromStr>(n: usize, default: T) -> T {
    std::env::args()
        .nth(n)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let seed: u64 = arg(1, 0x0EC0_5EED);
    let ticks: u64 = arg(2, DEFAULT_TICKS);
    let sample_every: u64 = arg(3, DEFAULT_SAMPLE_EVERY).max(1);

    let world = generate(seed, COLS, ROWS, HEX_SIZE);
    let n_tiles = (world.cols * world.rows) as usize;
    let mut rng = Rng::new(seed ^ 0xA5A5_5A5A);
    let ac = ActorCritic::new(n_tiles, &mut rng);

    let mut w = World::new();
    w.insert_resource(HungerControl::new(
        economy::TARGET_DEATH_FRAC_PER_TICK * N_NOOTS as f32,
    ));
    w.insert_resource(IncomeControl::default());
    w.insert_resource(EconStats::default());
    w.insert_resource(ac);
    w.insert_resource(Trainer::default());

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

    // Same order (and chaining) the GUI app runs the sim schedule in.
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

    let n_deposits = w.resource::<Sim>().0.deposits.len();
    eprintln!(
        "headless seed {:#x}: {} noots, {} deposits, {} ticks (sample every {}, dt {:.5}s/tick)",
        seed, N_NOOTS, n_deposits, ticks, sample_every, economy::TICK_DT
    );

    // Emit the initial state, then one JSONL record per `sample_every` ticks.
    emit_record(&mut w);
    for _ in 0..ticks {
        sched.run(&mut w);
        if w.resource::<EconStats>().ticks.is_multiple_of(sample_every) {
            emit_record(&mut w);
        }
    }
}

/// Print one JSONL line of the full econ state at the current tick.
fn emit_record(w: &mut World) {
    let stats = w.resource::<EconStats>().clone();
    let hunger = w.resource::<HungerControl>().clone();
    let income = w.resource::<IncomeControl>().clone();

    // Population aggregates (one pass over the noots).
    let goods = w.resource::<Sim>().0.goods.clone();
    let mut q = w.query::<(&Action, &Hunger, &Claim, &Wallet, &NootMeta, &Trader, &Inventory)>();
    let mut act = [0u64; 3];
    let (mut starving, mut claimed, mut n) = (0u64, 0u64, 0u64);
    let (mut bucks, mut appetite, mut experience, mut discount, mut positional) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (a, h, c, wal, m, tr, inv) in q.iter(w) {
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
        bucks += wal.bucks as f64;
        appetite += (h.staple.iter().sum::<f32>() / h.staple.len() as f32) as f64;
        experience += m.experience as f64;
        discount += tr.discount as f64;
        positional += (0..econ_sim::goods::N_ITEMS)
            .filter(|&i| matches!(goods.role_of(i), ItemRole::Positional(_)))
            .map(|i| inv.items[i])
            .sum::<f32>() as f64;
        n += 1;
    }
    let nf = n.max(1) as f64;

    let record = serde_json::json!({
        "tick": stats.ticks,
        "trades_total": stats.trades_total,
        "production_rate": stats.production_rate,
        "consumption_rate": stats.consumption_rate,
        "merchant_profit_rate": stats.merchant_profit_rate,
        "utility_rate": stats.utility_rate,
        "produced_total": stats.produced_total,
        "consumed_total": stats.consumed_total,
        "merchant_profit_total": stats.merchant_profit_total,
        "utility_total": stats.utility_total,
        "ewma_price": stats.ewma_price,
        "hunger_rate": hunger.rate,
        "deaths_per_tick": hunger.measured_per_tick,
        "deaths_per_tick_target": hunger.target_per_tick,
        "income_rate": income.rate,
        "sales_inflation": income.measured_inflation,
        "pop": n,
        "starving": starving,
        "claimed": claimed,
        "act_move": act[0],
        "act_mine": act[1],
        "act_refine": act[2],
        "mean_bucks": bucks / nf,
        "mean_appetite": appetite / nf,
        "mean_experience": experience / nf,
        "mean_discount": discount / nf,
        "mean_positional": positional / nf,
    });
    println!("{}", record);
}
