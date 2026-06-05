# 037 — Research screen + discovery reward

## Context

Two asks: (1) is there a reward signal for research, and can we shape it to pull noots toward
*discovery*; (2) a research screen showing "what is being researched", plus a graph of research
over time.

Before: the only research signal was a flat `RESEARCH_BONUS = 0.03` added whenever a Research
option completed legally — enough to keep the action in the repertoire (headless: 22–31
research actions/window, `research_total` into the millions), but discovery itself earned
nothing directly. And the only tech UI was the Tech panel (discovered techs); there was no
view of study demand, undiscovered techs, or research-over-time.

## What shipped

### Discovery reward (`economy.rs`, `policy.rs`)
- New `DISCOVERY_BONUS = 1.5`, a one-time reward to the noot whose Research action unlocks a
  new tech world-wide. `PolicyMemory` gains a `discovered` flag (mirrors `died`): `discover`
  sets it on the first noot to hold a tech's full input set; `policy_step` adds the bonus when
  that noot's option closes, then clears it. Non-farmable (a tech discovers once), so the bonus
  can be large without being exploitable.

### Research screen (`main.rs`)
- New **Research** sidebar button → full-screen `ResearchPanel` (scroll_y), mirroring the Tech
  panel, with a tap-guard so it swallows map taps while open. Four live sections:
  - **Activity headline**: cumulative effort, current rate, # noots researching now, and
    discovered/total tech counts.
  - **Trend graph**: a research-rate-over-time sparkline. New `ResearchTrend(Vec<f32>)`
    resource (capped, not persisted) is fed `stats.research_rate` from `sample_stats`;
    `update_research_panel` re-rasterizes it into a `GraphAssets.research` texture. No save
    migration (the strip's persisted series are untouched).
  - **Study demand by good**: a fixed row per item with a role-coloured bar whose width is the
    good's share of the busiest good's `research_demand`, plus a name + cumulative-effort
    caption. Updated every frame the panel is open.
  - **Coming up**: the catalog's *undiscovered* techs with their recipes (rebuilt on a
    catalog/discovered signature change), complementing the Tech panel's discovered view.

## Verification (headless)

- Seed 7 `--seed-tech`, 200k ticks: discovery still reaches all 6 techs by tick 50k with the
  new reward (no regression); `research_demand` populates and varies across goods (raws
  16.7k/125k/52.7k/34k early, refined grades growing to 10.7k/24.4k/2.6k/14.5k by 200k) — the
  exact data the demand bars render.
- `cargo check` + `cargo clippy` clean on the wasm gate.
- **Unverified**: the panel's on-device look (GUI can't run in the sandbox) — layout/legibility
  of the bars, trend chart, and tech list need an on-device pass.

## Notes

- The discovery bonus mainly sharpens behavior in the *live* setting where the server grows
  techs gradually; with a full seeded catalog discovery was already fast, so the headless delta
  is small by construction. `DISCOVERY_BONUS` is the tuning knob.
