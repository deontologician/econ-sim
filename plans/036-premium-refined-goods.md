# 036 — Premium refined goods (no more junk)

## Context

The resource-price panel showed two goods permanently at ₦0 (e.g. Steel, Lens): they were
never mined, refined, or traded. Root cause was structural, not a tuning bug. `goods::assign`
gives each of the 4 elements **one** role from {Staple·Raw, Staple·Refined, Positional·Raw,
Positional·Refined}. For the two elements consumed in **refined** form, the raw grade is a
useful **Intermediate** (a refiner buys it). But for the two consumed in **raw** form, the
refined grade was left as pure **Junk** — nothing consumes it and nothing refines into a
further good, so `base_ask` was `f32::MAX` and it never changed hands. So 2 of every 8 items
were dead by construction.

The ask: give a mix of resources a benefit and revive the dead goods — but *keep*
intermediates (a raw good that's only useful once refined is legitimate; not everything must
be directly consumable).

## What shipped

The fix is surgical: the orphan refined grade of a raw-consumed good is no longer junk — it
becomes the **premium grade of that same good**, carrying the element's role/sub. Refining is
an *upgrade*, so refining it and buying it both pay off. Intermediates are untouched.

- **`goods.rs` (`assign`)**: for a good consumed *raw*, the refined item now takes the same
  `Staple(sub)`/`Positional(sub)` role (premium grade) instead of `Junk`. Goods consumed
  *refined* keep their raw `Intermediate` exactly as before. New worlds have **no `Junk`**;
  the `Junk` arm survives only for old saves.
- **`economy.rs`**: refined grades are worth more —
  - staples: `EAT_VALUE_REFINED = 8` vs `EAT_VALUE_RAW = 4` (a refined unit clears twice the
    appetite); `eat_value(item)` drives `consume`, and the same potency scales the staple
    `wtp`/`reservation`.
  - luxuries: `positional_grade(item)` weights the refined grade `POSITIONAL_REFINED_MULT = 2`×
    in `positional_utility`, `wtp`, and `reservation`. Combined with the existing per-item
    `Σ ln(1+held)` diminishing returns, noots want a *diverse* basket of luxuries.
  - `maslow_utility` food buffer now sums per staple **element** (appetite sub) across both
    grades, so a fat stack of one grade can't mask a missing element.
  - `refine` lifts **any** held non-junk raw to its refined grade (was: intermediates only);
    `has_refinable` replaces `has_intermediate` for the Refine action mask.
  - `ESTEEM_NORM` 4 → 12 (esteem now spans more positional items, refined-weighted).
- **`world.rs`**: `consumption_rank` unchanged in spirit (still the 4-rank raw/refined ×
  staple/positional table), so deposit density is unchanged.

## Verification (headless)

- Seed 7 (the screenshot's seed), 400k ticks: every one of the 8 items carries a non-zero
  price — `ewma_price ≈ [9.2, 7.3, 5.5, 9.4, 4.3, 10.0, 8.0, 9.9]` — vs the two ₦0 goods in
  the screenshot.
- Cross-seed: **8 of 9 seeds** (1, 2, 3, 7, 42, 99, 777, …) have **no zero-price items** —
  all 8 goods trade. The one exception (seed 123) never builds a refinery across 500k ticks
  (`refineries: 0` the whole run), so its *entire* refined class — both grades 1/3/5/7,
  including the legitimately refined-consumed staples — stays empty while the raw economy runs
  fine (consumption climbs 1346→6988). That's a pre-existing policy slow-start in a hard seed,
  not a regression from this change (which only touches what refined goods are *worth*, not
  whether a refinery gets built). See INTENDED_FEATURES for the refinery-bootstrapping gap.
- `cargo check`/`cargo clippy` clean on the wasm gate.
- **Unverified**: the on-device look (GUI can't run in the sandbox) — needs an on-device
  check that the price panel now shows all 8 goods live and that refined grades read sensibly.

## Notes

- Balance is self-correcting on death rate: the hunger PID re-centres the absolute death rate,
  so doubling refined satiation shifts *what* noots eat, not how many starve.
- The refined grade of a raw-consumed staple is "stronger food"; of a raw-consumed luxury,
  "finer luxury". Both create a raw→refinery→premium supply chain a specialist can profit
  from, on top of the existing intermediate→refined chain.
