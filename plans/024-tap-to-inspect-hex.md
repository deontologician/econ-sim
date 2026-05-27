# 024 — Tap a hex to inspect it

## Context

You could tap a noot to follow it, but tapping a shop/refinery/deposit/terrain told you
nothing — "shops and refineries don't help me out". There was no way to ask "what is this
hex and what is it producing?".

## What shipped

- `SelectedHex(Option<usize>)` resource — the tapped tile, mutually exclusive with the
  noot `Selection`.
- `pick_selection`: a tap still prefers the nearest noot under it (follow); on a miss it
  now picks the **nearest tile centre** and stores it as the inspected hex (clearing the
  noot selection). Replaces the old "tap a deposit → follow its owner" behaviour.
- `describe_hex` fills the existing bottom panel with: terrain + work-speed %, then either
  - a **deposit** — element, replenishable/finite stock, what it produces (raw → refined)
    with each form's role (food / needs refining / luxury), and being-mined vs unclaimed;
  - a **shop / refinery** — owned vs abandoned, and what it's for; or
  - **open ground** — could build here.
  (`role_word` glosses item roles for the readout.)
- `HexHighlight`: a white ring spawned at setup, positioned/shown on the inspected hex by
  `update_hex_highlight`.

## Verification

- `cargo clippy` clean on `wasm32-unknown-unknown` and native headless.
- **Unverified**: the on-device render (panel text, highlight ring) — clean wasm compile
  only, as the GUI can't run in the sandbox.

## Notes

- Tapping empty terrain now inspects it (shows "open ground") rather than clearing the
  selection. Re-tap a noot to follow again.
- Future: tapping a structure could also surface/jump to its owner; show live extraction
  / regrowth rates.
