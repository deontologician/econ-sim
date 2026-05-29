# 032 — Leaderboard server + world names

## Context

Stand up a server-side component so sims can report in and be ranked. Asks: Rust + JSON;
each sim POSTs a snapshot every ~1K ticks, out of band of the wasm loop; reuse the save
infra; a small leaderboard of max GDP, the world's resources, tech tree, asset prices and
graphs; and each world gets a generated fantasy name shown bottom-left in place of the
removed "tap a noot" tip. Decisions confirmed with the user: **full save snapshot** as the
payload, **axum + tokio** server.

## What shipped

### World name (`worldname.rs`)
`world_name(seed)` builds a deterministic fantasy name (onset+middle+ending, e.g.
"Toparlia", "Isholria"). Shown in the bottom-left panel when nothing is selected; the
"tap a noot…" tip is gone. Stable across save/reload and derivable by the server from the
seed.

### Leaderboard payload (`leaderboard.rs`, in the lib)
`Summary` + `summarize(&Snapshot)` extract the league-table fields from a full snapshot:
name, seed, `gdp_total` (ranking key), ticks, prod/cons/gdp rates, the four resources
(name, refined form, role, tech `efficiency`), latest per-item prices, and the rolled-up
`stat_history`/`price_history` series (the graphs). Pure serde; the server reuses it.

### Client submit (`main.rs`)
`submit_leaderboard` system: every `LEADERBOARD_EVERY_TICKS` (1000) it builds the same
`save::Snapshot` the Save button does and POSTs it via `fetch`, **fire-and-forget** (the
Promise is dropped, so it never blocks the fixed-tick loop). Gated on `LEADERBOARD_URL`,
which is **`None` by default** — the public deploy phones home nowhere until you set it to
your server (`Some("https://host/submit")`). Off-wasm it's a no-op.

### Server (`src/bin/server.rs`, `--features server`)
axum + tokio, feature-gated with optional deps so the wasm/phone build never pulls them.
- `POST /submit` — body is a full snapshot (parsed via new `save::from_json`, reusing the
  migration + non-finite recovery); keeps the **best-GDP** summary per world name.
- `GET /api/leaderboard` — summaries as JSON, sorted by GDP desc.
- `GET /` — an HTML league table (rank, world, GDP, ticks, resources+role+tech, last
  prices, and an inline-SVG GDP sparkline from `stat_history`).
- Persists the table to `$LEADERBOARD_FILE` (default `leaderboard.json`); `PORT` env
  (default 8080). Run: `cargo run --no-default-features --features server --bin server`.

## Verification

- `cargo clippy` clean on all three feature sets (wasm/gui, headless, server); lib tests pass.
- End-to-end native test: started the server, POSTed two real headless snapshots →
  `/submit` 200, `/api/leaderboard` ranked them by GDP ("Isholria" 11.7M > "Belammoor"
  3.8M) with correct resources/roles/tech/prices, `/` rendered, store persisted to disk.
- **Unverified**: the live wasm `fetch` (no browser here) and the world name's on-screen
  placement. Sparklines were empty in the test because the **headless** harness doesn't
  sample graph history — real GUI clients do, so the graphs populate in production.

## Notes / follow-ups

- Payload is ~360 KB per report (a full snapshot includes the policy net + every noot). Fine
  at a 1K-tick cadence, but a trimmed payload or gzip is the obvious optimisation if it
  matters; true byte-diffs were considered and skipped (low value at this cadence).
- `STAT_SERIES_LABELS` in `leaderboard.rs` mirrors the GUI-gated `graph::SERIES` order; if
  that list changes, update both.
- Hosting is the user's to provide; the client URL const is the single switch to turn it on.
