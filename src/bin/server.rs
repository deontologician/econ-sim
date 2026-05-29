//! Leaderboard server. Sims POST their full save snapshot (`POST /submit`); the server
//! extracts a [`Summary`](econ_sim::leaderboard::Summary), keeps the best-GDP run per world
//! name, persists the table to a JSON file, and serves it as JSON (`/api/leaderboard`) and a
//! simple HTML league table with GDP sparklines (`/`).
//!
//! Run: `cargo run --no-default-features --features server --bin server` (listens on
//! `0.0.0.0:$PORT`, default 8080; store at `$LEADERBOARD_FILE`, default `leaderboard.json`).

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use econ_sim::leaderboard::{summarize, Summary, GDP_SERIES};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct App {
    /// Best-GDP summary per world name.
    store: Arc<Mutex<HashMap<String, Summary>>>,
    file: Arc<String>,
}

#[tokio::main]
async fn main() {
    let file = std::env::var("LEADERBOARD_FILE").unwrap_or_else(|_| "leaderboard.json".into());
    let port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080u16);
    let store = load(&file).unwrap_or_default();
    println!("leaderboard: {} worlds loaded from {file}", store.len());
    let app = App {
        store: Arc::new(Mutex::new(store)),
        file: Arc::new(file),
    };
    let router = Router::new()
        .route("/submit", post(submit))
        .route("/api/leaderboard", get(api))
        .route("/", get(index))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    println!("listening on http://0.0.0.0:{port}");
    axum::serve(listener, router).await.unwrap();
}

/// Accept a posted full snapshot, keep it if it beats this world's recorded GDP.
async fn submit(State(app): State<App>, body: Bytes) -> StatusCode {
    let Ok(raw) = std::str::from_utf8(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    let Some(snap) = econ_sim::save::from_json(raw) else {
        return StatusCode::BAD_REQUEST;
    };
    let summary = summarize(&snap);
    let mut store = app.store.lock().unwrap();
    let keep = store
        .get(&summary.name)
        .is_none_or(|prev| summary.gdp_total >= prev.gdp_total);
    if keep {
        store.insert(summary.name.clone(), summary);
        let snapshot = store.clone();
        drop(store);
        if let Err(e) = save(&app.file, &snapshot) {
            eprintln!("persist failed: {e}");
        }
    }
    StatusCode::OK
}

/// Summaries sorted by GDP, highest first.
fn ranked(app: &App) -> Vec<Summary> {
    let mut v: Vec<Summary> = app.store.lock().unwrap().values().cloned().collect();
    v.sort_by(|a, b| b.gdp_total.total_cmp(&a.gdp_total));
    v
}

async fn api(State(app): State<App>) -> Json<Vec<Summary>> {
    Json(ranked(&app))
}

async fn index(State(app): State<App>) -> Html<String> {
    Html(render(&ranked(&app)))
}

// --- Persistence ------------------------------------------------------------

fn load(path: &str) -> Option<HashMap<String, Summary>> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save(path: &str, store: &HashMap<String, Summary>) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_string(store).unwrap())
}

// --- HTML rendering ---------------------------------------------------------

fn render(entries: &[Summary]) -> String {
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
         h1{{font-weight:600}}table{{border-collapse:collapse;width:100%}}\
         th,td{{padding:6px 10px;border-bottom:1px solid #2a2f38;text-align:left;vertical-align:top}}\
         th{{color:#9aa3b2;font-weight:500}}.num,.px{{font-variant-numeric:tabular-nums}}\
         .name{{font-weight:600;color:#f0d68a}}.px{{color:#a8b0c0}}svg{{display:block}}</style>\
         <h1>noot economies — by GDP</h1>\
         <table><tr><th>#</th><th>world</th><th>GDP</th><th>ticks</th><th>resources (role, tech)</th>\
         <th>last prices</th><th>GDP over time</th></tr>{rows}</table>\
         <p style=color:#6b7280>{} worlds · updates as sims report in</p>",
        entries.len()
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

/// Minimal HTML escaping for the few user-derived strings we echo (world/resource names).
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
