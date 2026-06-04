//! Leaderboard + tech-tree server. Sims POST their full save snapshot (`POST /submit`); the
//! server keeps the best-GDP run per world (the leaderboard) **and** aggregates the
//! per-resource research demand each world reports to grow a single, global, live **tech
//! tree** (plans/035, Phase 2). Slowly over time it grows new techs whose inputs are the
//! most-researched resources across all worlds, randomizes their attributes, and names them
//! with a cheap LLM (OpenRouter; procedural fallback). The tree is returned in the `/submit`
//! response and served at `GET /tech`; the leaderboard at `GET /api/leaderboard` and `/`.
//!
//! Run: `cargo run --no-default-features --features server --bin server`. Env:
//! `PORT` (8080), `LEADERBOARD_FILE` (leaderboard.json), `TECH_FILE` (tech_state.json),
//! `OPENROUTER_API_KEY` (naming; procedural fallback if unset), `OPENROUTER_MODEL`.

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use econ_sim::goods::GoodForm;
use econ_sim::leaderboard::{summarize, Summary, GDP_SERIES};
use econ_sim::rng::Rng;
use econ_sim::tech::{self, Tech, TechDraft, TechTree, N_GLOBAL_ITEMS};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default seconds between growth attempts (override with `GROW_TICK_SECS`). Each attempt is
/// gated by a demand-driven probability ([`tech::growth_chance`]), so the tree grows
/// gradually.
const DEFAULT_GROW_TICK_SECS: u64 = 60;
/// Default OpenRouter model (cheap, Qwen-class). Override with `OPENROUTER_MODEL`.
const DEFAULT_MODEL: &str = "qwen/qwen-2.5-7b-instruct";

#[derive(Clone)]
struct App {
    /// Best-GDP summary per world name (the leaderboard).
    store: Arc<Mutex<HashMap<String, Summary>>>,
    file: Arc<String>,
    /// The shared tech state: demand + grown techs.
    tech: Arc<Mutex<TechShared>>,
    tech_file: Arc<String>,
    /// Server-side growth RNG (nondeterministic seed; growth needn't be reproducible).
    rng: Arc<Mutex<Rng>>,
}

/// Persisted tech state. `per_world` holds each world's *latest cumulative* research demand
/// (length [`N_GLOBAL_ITEMS`]) so resubmissions overwrite rather than double-count; the
/// global histogram is their sum.
#[derive(Clone, Default, Serialize, Deserialize)]
struct TechShared {
    per_world: HashMap<String, Vec<f64>>,
    techs: Vec<Tech>,
    next_id: u64,
    last_grow_unix: u64,
}

#[tokio::main]
async fn main() {
    let file = std::env::var("LEADERBOARD_FILE").unwrap_or_else(|_| "leaderboard.json".into());
    let tech_file = std::env::var("TECH_FILE").unwrap_or_else(|_| "tech_state.json".into());
    let port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080u16);

    let store = load(&file).unwrap_or_default();
    println!("leaderboard: {} worlds loaded from {file}", store.len());
    let tech_shared = load_tech(&tech_file).unwrap_or_default();
    println!(
        "tech: {} techs, {} worlds reporting demand (from {tech_file})",
        tech_shared.techs.len(),
        tech_shared.per_world.len()
    );

    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let app = App {
        store: Arc::new(Mutex::new(store)),
        file: Arc::new(file),
        tech: Arc::new(Mutex::new(tech_shared)),
        tech_file: Arc::new(tech_file),
        rng: Arc::new(Mutex::new(Rng::new(seed))),
    };

    tokio::spawn(grow_loop(app.clone()));

    let router = Router::new()
        .route("/submit", post(submit))
        .route("/api/leaderboard", get(api))
        .route("/tech", get(tech_handler))
        .route("/", get(index))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    println!("listening on http://0.0.0.0:{port}");
    axum::serve(listener, router).await.unwrap();
}

/// Accept a posted full snapshot: update the leaderboard (best GDP per world) and the world's
/// research-demand contribution, then return the current tech tree so the client can apply
/// it. A malformed body is rejected; everything else returns the tree.
async fn submit(State(app): State<App>, body: Bytes) -> Result<Json<TechTree>, StatusCode> {
    let raw = std::str::from_utf8(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let snap = econ_sim::save::from_json(raw).ok_or(StatusCode::BAD_REQUEST)?;
    let summary = summarize(&snap);
    let name = summary.name.clone();
    let demand = demand_vec_from_summary(&summary);

    // Leaderboard: keep the best-GDP run per world.
    {
        let mut store = app.store.lock().unwrap();
        let keep = store
            .get(&name)
            .is_none_or(|prev| summary.gdp_total >= prev.gdp_total);
        if keep {
            store.insert(name.clone(), summary);
            let snapshot = store.clone();
            drop(store);
            if let Err(e) = save(&app.file, &snapshot) {
                eprintln!("leaderboard persist failed: {e}");
            }
        }
    }

    // Tech demand: record this world's latest cumulative demand (overwrite, not add).
    if demand.iter().any(|&d| d > 0.0) {
        let snapshot = {
            let mut shared = app.tech.lock().unwrap();
            shared.per_world.insert(name, demand);
            shared.clone()
        };
        if let Err(e) = save_tech(&app.tech_file, &snapshot) {
            eprintln!("tech persist failed: {e}");
        }
    }

    Ok(Json(current_tree(&app)))
}

/// The background grower: every [`GROW_TICK_SECS`], with a demand-driven probability, mint one
/// new tech from the aggregate demand, name it (LLM or procedural), and persist.
async fn grow_loop(app: App) {
    let secs = std::env::var("GROW_TICK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&s| s > 0)
        .unwrap_or(DEFAULT_GROW_TICK_SECS);
    let mut ticker = tokio::time::interval(Duration::from_secs(secs));
    ticker.tick().await; // consume the immediate first tick
    loop {
        ticker.tick().await;
        let now = now_unix();
        // Decide and reserve an id under the locks; drop them before the (slow) naming call.
        let prepared: Option<(TechDraft, u64)> = {
            let mut shared = app.tech.lock().unwrap();
            let demand = global_demand(&shared);
            let total: f64 = demand.iter().sum();
            let mut rng = app.rng.lock().unwrap();
            if rng.chance(tech::growth_chance(total)) {
                tech::draft_tech(&demand, &mut rng).map(|d| {
                    let id = shared.next_id;
                    shared.next_id += 1;
                    shared.last_grow_unix = now;
                    (d, id)
                })
            } else {
                None
            }
        };
        let Some((draft, id)) = prepared else {
            continue;
        };
        let name = name_tech(&draft).await;
        let grown = Tech::from_draft(draft, id, name, now);
        println!(
            "grew tech #{id} '{}'  [{}]  {} ×{:.2}  ({}, tier {})",
            grown.name,
            grown.inputs_label(),
            grown.effect.label(),
            grown.magnitude,
            if grown.consumable { "consumable" } else { "durable" },
            grown.tier,
        );
        let snapshot = {
            let mut shared = app.tech.lock().unwrap();
            shared.techs.push(grown);
            shared.clone()
        };
        if let Err(e) = save_tech(&app.tech_file, &snapshot) {
            eprintln!("tech persist failed: {e}");
        }
    }
}

/// Name a drafted tech via OpenRouter; fall back to a procedural name when the key is unset
/// or the call fails. The blocking HTTP call runs off the async runtime.
async fn name_tech(draft: &TechDraft) -> String {
    let key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let fallback = tech::procedural_name(draft);
    if key.trim().is_empty() {
        return fallback;
    }
    let model = std::env::var("OPENROUTER_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
    let prompt = tech::naming_prompt(draft);
    match tokio::task::spawn_blocking(move || openrouter_name(&key, &model, &prompt)).await {
        Ok(Some(name)) => name,
        _ => fallback,
    }
}

/// One blocking OpenRouter chat-completion call returning a sanitized name, or `None` on any
/// error (network, auth, unexpected JSON). Never panics.
fn openrouter_name(key: &str, model: &str, prompt: &str) -> Option<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system",
             "content": "You name crafted items for a crafting game. Output only the name."},
            {"role": "user", "content": prompt},
        ],
        "max_tokens": 16,
        "temperature": 0.9,
    });
    let resp = ureq::post("https://openrouter.ai/api/v1/chat/completions")
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(20))
        .send_json(body)
        .ok()?;
    let v: serde_json::Value = resp.into_json().ok()?;
    let content = v.get("choices")?.get(0)?.get("message")?.get("content")?.as_str()?;
    tech::sanitize_name(content)
}

// --- Demand helpers ---------------------------------------------------------

/// Map a world's reported per-resource research demand (element name + form) to a global
/// demand vector indexed by [`tech::global_index`].
fn demand_vec_from_summary(s: &Summary) -> Vec<f64> {
    let mut v = vec![0.0; N_GLOBAL_ITEMS];
    for r in &s.research {
        if let Some(id) = econ_sim::elements::id_by_name(&r.element) {
            let form = if r.form == "refined" {
                GoodForm::Refined
            } else {
                GoodForm::Raw
            };
            v[tech::global_index(id, form)] = r.demand;
        }
    }
    v
}

/// Sum every world's latest demand into the global histogram.
fn global_demand(shared: &TechShared) -> Vec<f64> {
    let mut v = vec![0.0; N_GLOBAL_ITEMS];
    for dv in shared.per_world.values() {
        for (i, &d) in dv.iter().enumerate().take(N_GLOBAL_ITEMS) {
            v[i] += d;
        }
    }
    v
}

fn current_tree(app: &App) -> TechTree {
    TechTree {
        techs: app.tech.lock().unwrap().techs.clone(),
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- Handlers ---------------------------------------------------------------

/// Summaries sorted by GDP, highest first.
fn ranked(app: &App) -> Vec<Summary> {
    let mut v: Vec<Summary> = app.store.lock().unwrap().values().cloned().collect();
    v.sort_by(|a, b| b.gdp_total.total_cmp(&a.gdp_total));
    v
}

async fn api(State(app): State<App>) -> Json<Vec<Summary>> {
    Json(ranked(&app))
}

async fn tech_handler(State(app): State<App>) -> Json<TechTree> {
    Json(current_tree(&app))
}

async fn index(State(app): State<App>) -> Html<String> {
    let techs = app.tech.lock().unwrap().techs.clone();
    Html(render(&ranked(&app), &techs))
}

// --- Persistence ------------------------------------------------------------

fn load(path: &str) -> Option<HashMap<String, Summary>> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save(path: &str, store: &HashMap<String, Summary>) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_string(store).unwrap())
}

fn load_tech(path: &str) -> Option<TechShared> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save_tech(path: &str, shared: &TechShared) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_string(shared).unwrap())
}

// --- HTML rendering ---------------------------------------------------------

fn render(entries: &[Summary], techs: &[Tech]) -> String {
    let mut rows = String::new();
    for (i, e) in entries.iter().enumerate() {
        let res = e
            .resources
            .iter()
            .map(|r| format!("{} ({}, ×{:.2})", esc(&r.name), &r.role[..3], r.efficiency))
            .collect::<Vec<_>>()
            .join("<br>");
        let prices = e
            .prices
            .iter()
            .map(|p| format!("{p:.1}"))
            .collect::<Vec<_>>()
            .join(" · ");
        let gdp_series: Vec<f32> = e
            .stat_history
            .iter()
            .filter_map(|s| s.get(GDP_SERIES).copied())
            .collect();
        rows.push_str(&format!(
            "<tr><td>{}</td><td class=name>{}</td><td class=num>{:.0}</td><td class=num>{}</td>\
             <td>{res}</td><td class=px>{prices}</td><td>{}</td></tr>",
            i + 1,
            esc(&e.name),
            e.gdp_total,
            e.ticks,
            sparkline(&gdp_series),
        ));
    }
    format!(
        "<!doctype html><meta charset=utf-8><title>noot leaderboard</title>\
         <style>body{{background:#14171c;color:#dfe3ea;font:14px/1.4 system-ui,sans-serif;margin:24px}}\
         h1,h2{{font-weight:600}}table{{border-collapse:collapse;width:100%;margin-bottom:32px}}\
         th,td{{padding:6px 10px;border-bottom:1px solid #2a2f38;text-align:left;vertical-align:top}}\
         th{{color:#9aa3b2;font-weight:500}}.num,.px{{font-variant-numeric:tabular-nums}}\
         .name{{font-weight:600;color:#f0d68a}}.px{{color:#a8b0c0}}.tech{{color:#9ad6c0}}svg{{display:block}}</style>\
         <h1>noot economies — by GDP</h1>\
         <table><tr><th>#</th><th>world</th><th>GDP</th><th>ticks</th><th>resources (role, tech)</th>\
         <th>last prices</th><th>GDP over time</th></tr>{rows}</table>\
         <p style=color:#6b7280>{} worlds · updates as sims report in</p>\
         {}",
        entries.len(),
        render_techs(techs),
    )
}

/// The grown tech tree, newest first.
fn render_techs(techs: &[Tech]) -> String {
    if techs.is_empty() {
        return "<h2>tech tree</h2><p style=color:#6b7280>no techs grown yet — \
                they emerge as noots research the most-demanded resources</p>"
            .to_string();
    }
    let mut rows = String::new();
    for t in techs.iter().rev() {
        rows.push_str(&format!(
            "<tr><td class=name>{}</td><td class=tech>{}</td><td>{} ×{:.2}</td>\
             <td>{}</td><td>{}</td><td class=num>{}</td></tr>",
            esc(&t.name),
            esc(&t.inputs_label()),
            t.effect.label(),
            t.magnitude,
            if t.consumable { "consumable" } else { "durable" },
            match t.category {
                econ_sim::goods::GoodCategory::Staple => "staple",
                econ_sim::goods::GoodCategory::Positional => "positional",
            },
            t.tier,
        ));
    }
    format!(
        "<h2>tech tree</h2>\
         <table><tr><th>name</th><th>recipe</th><th>effect</th><th>durability</th>\
         <th>kind</th><th>tier</th></tr>{rows}</table>"
    )
}

/// A tiny inline-SVG sparkline of `vals`, min–max scaled to a small box.
fn sparkline(vals: &[f32]) -> String {
    if vals.len() < 2 {
        return String::new();
    }
    let (w, h) = (120.0f32, 28.0f32);
    let (lo, hi) = vals
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (hi - lo).max(1e-6);
    let pts = vals
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f32 / (vals.len() - 1) as f32 * w;
            let y = h - (v - lo) / span * h;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<svg width={w} height={h} viewBox='0 0 {w} {h}'>\
         <polyline fill=none stroke='#f0d68a' stroke-width=1.5 points='{pts}'/></svg>"
    )
}

/// Minimal HTML escaping for the few user-derived strings we echo (world/resource/tech names).
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
