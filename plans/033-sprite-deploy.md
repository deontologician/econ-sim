# 033 — Deploy the leaderboard server to a Fly.io Sprite

## Context

Follow-up to 032 (leaderboard server). The user picked **Fly.io Sprites** for hosting —
persistent sandbox VMs with an always-on HTTP URL that wakes on demand and a filesystem that
survives sleeps. That's a clean fit: the server's `leaderboard.json` persists for free, the
VM costs ~nothing while idle, and it wakes in 100–500ms when a sim POSTs. The base image
already ships Rust + git, and the URL auto-routes to port 8080, so there's no
Dockerfile/volume to manage.

## What shipped

### Compile-time submit URL (`main.rs`)
`LEADERBOARD_URL` is now `option_env!("LEADERBOARD_URL")` (empty/unset → `None`), so the URL
is baked in by the build instead of edited in source per deploy. Reporting stays off by
default.

### Pages build wiring (`.github/workflows/deploy.yml`)
The "Build wasm bundle" step gets `env: LEADERBOARD_URL: ${{ vars.LEADERBOARD_SUBMIT_URL }}`.
Set that repo variable to the Sprite's `…/submit` URL to turn reporting on; leave it unset to
keep the public build phoning home nowhere. No code change to toggle.

### Deploy tooling (`deploy/sprite/`)
- `deploy.sh` — idempotent: ensures the Sprite exists, clones/pulls the repo, builds the
  server (`--release --no-default-features --features server`), (re)registers it as a Sprite
  **service** on :8080 (`/bin/bash -lc 'cd /home/sprite && exec …/server'` so cwd is the
  persistent home and signals reach the process), makes the URL public, and prints the URL +
  the next step. Re-run to ship a new build.
- `README.md` — one-time CLI setup, deploy, how to flip on reporting via the repo variable,
  the endpoints, service-management commands, and the spoofability/payload-size caveats.

## Verification

- `cargo check`/`clippy` clean on wasm both with and without `LEADERBOARD_URL` set (the
  `const fn` empty-guard compiles); release server build succeeds.
- Smoke-tested the **release** binary end-to-end: `POST /submit` 200, `/api/leaderboard`
  ranked JSON, `/` 200.
- **Unverified**: the actual Sprite deploy (needs the user's Fly.io auth — can't run from the
  sandbox) and the live wasm `fetch`. The deploy steps follow the current Sprites docs
  (services, public URL, :8080 routing, persistent FS, Rust in the base image).

## Notes

- Public URL ⇒ anyone can POST; scores are spoofable. Server keeps best-GDP per world; add a
  shared-secret header in `submit` if it ever matters.
- Service auto-starts on boot and restarts on cold wake; the deploy deletes+recreates it so a
  rebuilt binary is picked up.
