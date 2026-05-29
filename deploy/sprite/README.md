# Leaderboard server on a Fly.io Sprite

The leaderboard server (`src/bin/server.rs`) is deployed to a [Fly.io
Sprite](https://sprites.dev) — a persistent sandbox VM whose filesystem survives sleeps and
whose URL wakes it on demand. That fits a leaderboard perfectly: it costs ~nothing while
idle, wakes in 100–500ms when a sim POSTs, and keeps `leaderboard.json` across restarts.

## One-time setup

```bash
curl -fsSL https://sprites.dev/install.sh | sh   # install the `sprite` CLI
sprite org auth                                  # log in with your Fly.io account
```

## Deploy / redeploy

```bash
./deploy/sprite/deploy.sh
```

Idempotent: creates the Sprite if needed, clones/pulls this repo, builds the server in
release with `--features server`, (re)registers it as a Sprite **service** on port 8080,
and makes the URL public. Re-run it to ship a new build. Env overrides: `SPRITE_NAME`,
`REPO_URL`, `BRANCH`.

The Sprite base image already ships Rust + git, and HTTP auto-routes to `:8080`, so there's
no Dockerfile/volume to manage. The server defaults to `PORT=8080` and writes
`leaderboard.json` in its working dir (`/home/sprite`), which persists.

## Turn on reporting from the game

The wasm client only POSTs when `LEADERBOARD_URL` is baked in at build time. The Pages
workflow reads it from a repo variable:

1. After deploy, copy the printed URL and append `/submit`
   (e.g. `https://econ-leaderboard-yourorg.sprites.dev/submit`).
2. GitHub → Settings → Secrets and variables → Actions → **Variables** → new variable
   `LEADERBOARD_SUBMIT_URL` = that URL.
3. Re-run the Pages deploy (push to `main` or run the workflow). The app now POSTs a full
   snapshot every 1000 ticks.

Leave the variable unset to disable reporting (the default).

## Endpoints

- `GET /` — HTML league table (world name, GDP, resources + tech, last prices, GDP sparkline).
- `GET /api/leaderboard` — the same data as JSON, ranked by GDP.
- `POST /submit` — a full save snapshot (what the game sends).

## Service management (on the Sprite)

```bash
sprite exec sprite-env services list
sprite exec sprite-env services get leaderboard
# stream logs for 30s while debugging a (re)start:
sprite exec bash -lc "sprite-env curl -X PUT '/v1/services/leaderboard?duration=30s' -d '{\"cmd\":\"/bin/bash\",\"args\":[\"-lc\",\"cd /home/sprite && exec /home/sprite/econ-sim/target/release/server\"]}'"
```

## Notes

- **Public URL = anyone can POST.** Fine for a hobby leaderboard, but scores are spoofable;
  the server keeps only the best-GDP run per world name. Add a shared-secret header check in
  `submit` if that ever matters.
- Each report is a full snapshot (~360 KB). At a 1000-tick cadence that's modest; trim or
  gzip the payload if it grows.
