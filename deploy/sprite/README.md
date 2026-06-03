# Leaderboard server on a Fly.io Sprite

The leaderboard server (`src/bin/server.rs`) is deployed to a [Fly.io
Sprite](https://sprites.dev) — a persistent sandbox VM whose filesystem survives sleeps and
whose URL wakes it on demand. That fits a leaderboard perfectly: it costs ~nothing while
idle, wakes in 100–500ms when a sim POSTs, and keeps `leaderboard.json` across restarts.

Live at **https://econ-leaderboard-bk7w.sprites.app** (the Pages build reports here by
default — see "Turn on reporting" below).

## One-time setup

```bash
curl -fsSL https://sprites.dev/install.sh | sh   # install the `sprite` CLI
sprite login                                     # browser OAuth with your Fly.io account
```

No browser (CI / a Fly deploy token)? Mint a Sprite token from the FlyV1 token and hand it
to the CLI — `sprite login`/`org auth` need a *user* identity, which a deploy token lacks:

```bash
curl -s -X POST https://api.sprites.dev/v1/organizations/<org-slug>/tokens \
  -H "Authorization: $FLY_API_TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"econ-sim-deploy"}'                 # -> {"token":"<org>/<id>/<tid>/<secret>"}
sprite auth setup --token "<org>/<id>/<tid>/<secret>"
```

Use the org **slug** (e.g. `josh-kuhn`), not the GraphQL org id, and pass the raw `FlyV1 …`
value as the `Authorization` header (no `Bearer` prefix) — otherwise the mint returns 401.

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
workflow (`.github/workflows/deploy.yml`) **defaults** it to the deployed Sprite's `/submit`
URL, so a fresh push to `main` reports out of the box — no repo config needed.

To point reporting elsewhere (a different Sprite, or off):

1. Copy the deploy script's printed URL and append `/submit`
   (e.g. `https://econ-leaderboard-yourorg.sprites.app/submit`).
2. GitHub → Settings → Secrets and variables → Actions → **Variables** → set
   `LEADERBOARD_SUBMIT_URL` = that URL. (Set it to a dummy/unreachable value to mute
   reporting.)
3. Re-run the Pages deploy (push to `main` or run the workflow). The variable overrides the
   baked-in default; the app POSTs a full snapshot every 1000 ticks.

## Endpoints

- `GET /` — HTML league table (world name, GDP, resources + tech, last prices, GDP sparkline).
- `GET /api/leaderboard` — the same data as JSON, ranked by GDP.
- `POST /submit` — a full save snapshot (what the game sends).

## Service management (on the Sprite)

`sprite exec` parses its own flags up to a `--`, so pass remote commands after `--`:

```bash
sprite exec -- sprite-env services list
sprite exec -- sprite-env services get leaderboard
sprite exec -- sprite-env services restart leaderboard      # pick up a new binary
sprite exec -- tail -n 50 /.sprite/logs/services/leaderboard.log
```

The service is registered with `--http-port 8080`, so the Sprite proxy routes inbound HTTP
to the server and auto-starts it on the first request after an idle sleep.

## Notes

- **Public URL = anyone can POST.** Fine for a hobby leaderboard, but scores are spoofable;
  the server keeps only the best-GDP run per world name. Add a shared-secret header check in
  `submit` if that ever matters.
- Each report is a full snapshot (~360 KB). At a 1000-tick cadence that's modest; trim or
  gzip the payload if it grows.
