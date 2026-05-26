//! Headless simulation harness — runs the real rollouts without any graphics so the
//! learning can be observed and tuned offline. Drives a bare `bevy_ecs` World +
//! Schedule on the fixed simulation tick (no real-time clock: every system advances
//! the world by exactly one `economy::TICK_DT`), as fast as the CPU allows.
//!
//! Stdout is **JSONL**: one JSON object per sampled tick with the full econ stats
//! (rates are per-tick, so they're speed-invariant). Human-readable progress goes to
//! stderr, keeping stdout a clean stream to pipe into `jq`/pandas/etc.
//!
//! Has headless equivalents of the GUI's functional affordances: `--load PATH` resumes
//! a saved run (the GUI's boot-resume), `--save PATH` writes the final full-state
//! snapshot (the GUI's Save), and omitting `--load` starts fresh from the seed (New).
//! The purely visual affordances (camera, overlays, noot-colour modes, the graphs
//! chart) have no headless form — the JSONL stream *is* their inspection equivalent.
//!
//! Build/run (needs the `headless` feature, which omits Bevy's GUI features so it
//! compiles without GPU/windowing libs):
//!
//! ```text
//! cargo run --release --no-default-features --features headless --bin headless \
//!     -- [seed] [ticks] [sample_every] [--load PATH] [--save PATH]
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
use econ_sim::{save, Sim, SimRng};

const COLS: i32 = 30;
const ROWS: i32 = 22;
const HEX_SIZE: f32 = 26.0;
const N_NOOTS: usize = 56;

const DEFAULT_TICKS: u64 = 60_000;
const DEFAULT_SAMPLE_EVERY: u64 = 600;

/// Parse a seed, accepting either decimal or a `0x`-prefixed hex literal.
fn parse_seed(s: &str) -> Option<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Parsed CLI: `[seed] [ticks] [sample_every]` positionals plus `--load PATH` (resume a
/// saved run, the GUI's boot-resume) and `--save PATH` (write the final state, the GUI's
/// Save button). `--new` ignores any default save file (the GUI's New).
struct Cli {
    seed: u64,
    ticks: u64,
    sample_every: u64,
    load: Option<String>,
    save: Option<String>,
}

fn parse_cli() -> Cli {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut load, mut save) = (None, None);
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--load" => {
                load = args.get(i + 1).cloned();
                i += 2;
            }
            "--save" => {
                save = args.get(i + 1).cloned();
                i += 2;
            }
            other => {
                pos.push(other.to_string());
                i += 1;
            }
        }
    }
    Cli {
        seed: pos.first().and_then(|s| parse_seed(s)).unwrap_or(0x0EC0_5EED),
        ticks: pos.get(1).and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TICKS),
        sample_every: pos
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SAMPLE_EVERY)
            .max(1),
        load,
        save,
    }
}

fn main() {
    let cli = parse_cli();
    let mut rng = Rng::new(cli.seed ^ 0xA5A5_5A5A);
    let mut w = World::new();

    // Resume a saved run if `--load` points at a usable file (mirrors the GUI's
    // boot-resume); otherwise roll a fresh world from the seed (the GUI's New).
    let resumed = cli.load.as_deref().and_then(save::load_from);
    let (world, restore_noots, fresh) = match resumed {
        Some(s) => {
            let n_tiles = (s.world.cols * s.world.rows) as usize;
            let policy = if s.policy.fits(n_tiles) {
                s.policy
            } else {
                ActorCritic::new(n_tiles, &mut rng)
            };
            w.insert_resource(s.hunger);
            w.insert_resource(s.income);
            w.insert_resource(s.stats);
            w.insert_resource(policy);
            (s.world, Some(s.noots), false)
        }
        None => {
            let world = generate(cli.seed, COLS, ROWS, HEX_SIZE);
            let n_tiles = (world.cols * world.rows) as usize;
            w.insert_resource(HungerControl::new(
                economy::TARGET_DEATH_FRAC_PER_TICK * N_NOOTS as f32,
            ));
            w.insert_resource(IncomeControl::default());
            w.insert_resource(EconStats::default());
            w.insert_resource(ActorCritic::new(n_tiles, &mut rng));
            (world, None, true)
        }
    };
    w.insert_resource(Trainer::default());
    w.insert_resource(economy::PriceField::default());

    match restore_noots {
        Some(noots) => {
            for ns in noots {
                w.spawn((
                    Noot,
                    Action::default(),
                    ns.claim,
                    ns.trader,
                    ns.meta,
                    ns.pos,
                    ns.inv,
                    ns.wallet,
                    ns.hunger,
                    PolicyMemory::new(ns.explore),
                ));
            }
        }
        None => {
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
        }
    }
    w.insert_resource(Sim(world));
    w.insert_resource(SimRng(rng));

    // Identical pipeline to the GUI app — both build it from `add_sim_systems`.
    let mut sched = Schedule::default();
    economy::add_sim_systems(&mut sched);

    let pop = w.query::<&Noot>().iter(&w).count();
    let n_deposits = w.resource::<Sim>().0.deposits.len();
    eprintln!(
        "headless seed {:#x}{}: {} noots, {} deposits, {} ticks (sample every {}, dt {:.5}s/tick)",
        cli.seed,
        if fresh { "" } else { " [resumed]" },
        pop,
        n_deposits,
        cli.ticks,
        cli.sample_every,
        economy::TICK_DT
    );

    // Emit the initial state, then one JSONL record per `sample_every` ticks.
    emit_record(&mut w);
    for _ in 0..cli.ticks {
        sched.run(&mut w);
        if w.resource::<EconStats>().ticks.is_multiple_of(cli.sample_every) {
            emit_record(&mut w);
        }
    }

    // Persist the final state if asked (the GUI's Save button).
    if let Some(path) = cli.save {
        save::store_to(&path, &snapshot(&mut w));
        eprintln!("saved final state to {path}");
    }
}

/// Gather the full game state into a `Snapshot` (the headless equivalent of the GUI's
/// Save: world, controllers, stats, the shared policy, and every noot).
fn snapshot(w: &mut World) -> save::Snapshot {
    let mut q = w.query::<(
        &TilePos,
        &Inventory,
        &Wallet,
        &Hunger,
        &Claim,
        &Trader,
        &NootMeta,
        &PolicyMemory,
    )>();
    let noots = q
        .iter(w)
        .map(|(pos, inv, wal, hun, claim, trader, meta, mem)| save::NootSave {
            pos: *pos,
            inv: inv.clone(),
            wallet: wal.clone(),
            hunger: hun.clone(),
            claim: claim.clone(),
            trader: trader.clone(),
            meta: meta.clone(),
            explore: mem.explore,
        })
        .collect();
    save::Snapshot {
        version: save::SAVE_VERSION,
        world: w.resource::<Sim>().0.clone(),
        hunger: w.resource::<HungerControl>().clone(),
        income: w.resource::<IncomeControl>().clone(),
        stats: w.resource::<EconStats>().clone(),
        policy: w.resource::<ActorCritic>().clone(),
        noots,
    }
}

/// Print one JSONL line of the full econ state at the current tick.
fn emit_record(w: &mut World) {
    let stats = w.resource::<EconStats>().clone();
    let hunger = w.resource::<HungerControl>().clone();
    let income = w.resource::<IncomeControl>().clone();

    // Population aggregates (one pass over the noots).
    let sim_ref = w.resource::<Sim>();
    let goods = sim_ref.0.goods.clone();
    let (cols, rows) = (sim_ref.0.cols, sim_ref.0.rows);
    let mut q =
        w.query::<(&Action, &Hunger, &Claim, &Wallet, &NootMeta, &Trader, &Inventory, &TilePos)>();
    let mut act = [0u64; 4];
    let (mut starving, mut claimed, mut n) = (0u64, 0u64, 0u64);
    let (mut bucks, mut appetite, mut experience, mut age, mut discount, mut positional) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut transactions = 0.0f64;
    let mut tiles: Vec<(i32, i32)> = Vec::new();
    for (a, h, c, wal, m, tr, inv, tp) in q.iter(w) {
        match a {
            Action::Move => act[0] += 1,
            Action::Mine => act[1] += 1,
            Action::Refine => act[2] += 1,
            Action::Idle => act[3] += 1,
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
        transactions += m.transactions as f64;
        age += m.age as f64;
        discount += tr.discount as f64;
        positional += (0..econ_sim::goods::N_ITEMS)
            .filter(|&i| matches!(goods.role_of(i), ItemRole::Positional(_)))
            .map(|i| inv.items[i])
            .sum::<f32>() as f64;
        tiles.push((tp.col, tp.row));
        n += 1;
    }
    let nf = n.max(1) as f64;
    let mean_nn_dist = econ_sim::hex::mean_nearest_neighbor(&tiles, cols, rows);

    // Spatial trade concentration: the share of all cleared trades that happened in the
    // busiest 5% of hexes (an agglomeration / "cities" index — high means commerce
    // pools into a few marketplaces), plus how many hexes have ever seen a trade.
    let (trade_top5_share, trade_active_hexes) = {
        let mut h = stats.trade_hexes.clone();
        let active = h.iter().filter(|&&c| c > 0).count();
        let total: u64 = h.iter().map(|&c| c as u64).sum();
        h.sort_unstable_by(|a, b| b.cmp(a));
        let k = (h.len() as f64 * 0.05).ceil() as usize;
        let top: u64 = h.iter().take(k).map(|&c| c as u64).sum();
        let share = if total > 0 { top as f64 / total as f64 } else { 0.0 };
        (share, active)
    };

    let record = serde_json::json!({
        "tick": stats.ticks,
        "trades_total": stats.trades_total,
        "production_rate": stats.production_rate,
        "consumption_rate": stats.consumption_rate,
        "merchant_profit_rate": stats.merchant_profit_rate,
        "utility_rate": stats.utility_rate,
        "gdp_rate": stats.gdp_rate,
        "gdp_total": stats.gdp_total,
        "mean_haul_dist": stats.mean_haul_dist,
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
        "act_idle": act[3],
        "mean_bucks": bucks / nf,
        "mean_appetite": appetite / nf,
        "mean_experience": experience / nf,
        "mean_transactions": transactions / nf,
        "mean_age": age / nf,
        "mean_discount": discount / nf,
        "mean_positional": positional / nf,
        "mean_nn_dist": mean_nn_dist,
        "trade_top5_share": trade_top5_share,
        "trade_active_hexes": trade_active_hexes,
    });
    println!("{}", record);
}
