# Plan 006 — PID control of the hunger rate (death-rate target)

## Context

The hunger rate was a fixed constant (`HUNGER_RATE = 0.5`), so the resulting death
rate was whatever the economy happened to produce — could be zero (everyone fed) or
a massacre. This adds a closed loop: a PID controller trims the global hunger rate so
the realized death rate tracks a setpoint of **2% of the mortal population per
minute**.

## What shipped (`economy.rs`, `main.rs`)

- `HUNGER_RATE` const is gone; the live rate lives in a new `HungerControl` resource
  (`rate`, seeded at the old 0.5). `hunger_tick` reads `ctrl.rate` instead.
- **Setpoint:** `target_per_min = 0.02 × n_eaters`, where `n_eaters` is the mortal
  population (owners + refiners + consumers; merchants don't starve). Computed once
  at `setup` from the world.
- **Feedback:** `death_and_respawn` bumps `ctrl.deaths_since_update` on each death.
- **Controller** (`hunger_pid`, every `PID_PERIOD = 3 s`):
  - measured death rate = `deaths_in_window / window × 60`, smoothed by an EMA
    (`PID_MEAS_ALPHA`) because discrete deaths are noisy;
  - `error = target − measured` (deaths/min);
  - positional PID `rate = Kp·e + Ki·∫e + Kd·de/dt`, with the integral providing the
    steady-state operating point. Anti-windup clamps the integral so `Ki·∫e` stays
    within the rate range; the output is clamped to `[HUNGER_RATE_MIN, MAX]`.
  - Direction: too few deaths ⇒ raise hunger; too many ⇒ lower it.
- **Warm start:** `measured` is seeded at the target and the integral at the initial
  rate, so the loop begins from today's behaviour and only trims from there (rather
  than slamming the rate at boot).
- **HUD:** new line `deaths {measured}/min → target {target}   hunger rate {rate}`.

## Tunables / caveats
`PID_KP/KI/KD`, `PID_PERIOD`, `PID_MEAS_ALPHA`, and the rate clamps are the levers.
Gains are **unvalidated** (can't run here). Because deaths are rare (target < ~1/min)
the loop is inherently slow, and there's an expected boot transient: the first ~30 s
are death-free (spawn half-fed + 20 s grace), so the integral winds the rate up, then
pulls back once deaths begin. If the economy can't reach the target even at
`HUNGER_RATE_MAX` (food too abundant) the rate just saturates — the controller does
its best within physical limits. Watch the HUD `deaths/min` settle toward `target`.

## Files
`noot.rs` (drop `HUNGER_RATE`), `economy.rs` (`HungerControl` + `hunger_pid`,
`hunger_tick` reads the resource), `main.rs` (insert resource, schedule, death
feedback, HUD), `plans/INTENDED_FEATURES.md`.
