# 027 — GPS-taught navigation (value-guided movement)

## Context

The ask: "teach agents how to get home" — when a noot decides to go somewhere, give the
direct route a huge optimism bump in its value graph, but still let it decide each step,
following the route until it arrives or picks a new destination (like a GPS that suggests
but doesn't command). The goal (clarified): flexible, *learned* goal-directed movement and
the "memory of deciding to go somewhere" — not matching the old economy's exact output.

## What was tried (and rejected)

The economy turned out to be hypersensitive to the movement/decision loop, and very
seed-variable on its own (baseline consumption 1001–3452, GDP 2.8M–73M across 3 seeds).

- **Steering by the global critic value at low optimism**: GDP collapsed ~3× — the critic's
  value is a *global* state-value, so it pulled haulers back toward their high-value home
  deposit and trade died.
- **Per-step option re-evaluation** (re-decide the goal each step with a commitment bonus):
  consumption worse on 3/3 matched seeds, deaths worse on 3/3 — re-deciding every step
  undoes the commitment that lets the produce→haul→sell chain finish, so noots dither.

Both were reverted.

## What shipped

`value_guided_step` replaces the deterministic greedy `step_toward` in the Mine/Sell/Refine
navigation branches. Each acting step, a noot scores its six neighbours by
`ROUTE_OPTIMISM * on_route + critic.value(neighbour)` and moves to the best, where
`on_route ∈ {+1,0,−1}` is whether the step shortens the toroidal distance to the committed
target (`PolicyMemory::plan_target` — the remembered destination). `ROUTE_OPTIMISM = 20` is
large versus the critic's value scale, so:

- the route reliably wins → a noot heads to the goal it chose and **doesn't get lost**, and
  trade/consumption survive (the failure mode of low optimism);
- the learned value still breaks ties and can veer the path toward a clearly better tile →
  **flexible, learned** movement rather than a fixed rail.

Features are now computed each acting step (to score neighbours), not only at option
boundaries. The option-level decision, transitions, masks, and reward are unchanged.

## Verification

- `cargo clippy` clean on wasm + native; headless builds.
- 3-seed sanity (60k): population stable, all 30 deposits worked, trade flowing; prod/cons
  comparable-to-better than the deterministic baseline on matched seeds (e.g. seed 0xBEEF
  prod 4081→4890, cons 1001→1858, deaths 176→126).
- Long horizon (180k, seed 0x0EC05EED): prod 4.6k→12.8k, cons 2.7k→7.5k, trade growing,
  deaths regulated — sustained, no late collapse.
- **Unverified**: on-device GUI feel (the point of the feature is how movement *looks*);
  the headless harness only confirms the economy stays healthy.

## Notes

- `ROUTE_OPTIMISM` is the single knob: lower = more deviation/flexibility (and more risk of
  the home-pull that hurt trade), higher = stricter routing. 20 was the balance that kept
  the economy healthy while leaving the learned value a real say. Tune on-device.
- Cost: one feature vector + six `critic.value` evals per noot per acting step (cheap;
  acting is cooldown-gated, not every tick).
