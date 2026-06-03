# 034 — Leaderboard live on a Sprite; default reporting; drop self-hosted wasm

## Context

Realizes 033 (which shipped the deploy *tooling* but left the actual Sprite deploy
unverified — it needs a logged-in Fly account, unavailable from the build sandbox at the
time). This session had a 48h Fly token and stood the backend up for real. The user decided
to keep GitHub Pages as the wasm host and wanted only the leaderboard backend on Sprites.

## What shipped

### Leaderboard is live
`src/bin/server.rs` is deployed as a Sprite **service** named `leaderboard` on the
`econ-leaderboard` Sprite (org `josh-kuhn`), public at
**https://econ-leaderboard-bk7w.sprites.app**. Registered with `--http-port 8080` so the
proxy routes inbound HTTP and cold-starts it on first request; `--dir /home/sprite` keeps
`leaderboard.json` on the persistent home FS. Verified end-to-end against the **release**
binary: `POST /submit` 200, `/api/leaderboard` ranked JSON, `/` HTML table, and the entry
survived a service restart (disk reload). Test row cleared, so it starts empty.

### Pages build reports by default (`.github/workflows/deploy.yml`)
`LEADERBOARD_URL` now defaults to the Sprite's `/submit` URL:
`${{ vars.LEADERBOARD_SUBMIT_URL || 'https://econ-leaderboard-bk7w.sprites.app/submit' }}`.
A fresh push to `main` reports out of the box; the repo variable still overrides it (set it
to a dummy value to mute). Chosen over a repo Actions variable because there's no API tool to
set that variable, and the URL is public anyway (it ends up in the wasm).

### Deploy tooling fixed for the current Sprites CLI (`deploy/sprite/`)
The CLI shipped in 033's docs (`rc43`) changed shape; the old script broke. Fixes:
- `sprite exec -- bash -c '…'` — `sprite exec` parses flags before `--`, so the old
  `sprite exec bash -lc` died with `unknown shorthand flag: 'l'`.
- Service registration via `sprite-env services create leaderboard --cmd … --dir /home/sprite
  --http-port 8080 --no-stream` instead of a raw `curl -X PUT /v1/services/…`.
- URL exposure via `sprite update --url-auth public` (falls back to the deprecated
  `sprite url update --auth public`).
- README documents the **headless auth path**: a Fly *deploy* token has no user identity, so
  `sprite login`/`org auth` fail (`no user ID in response`). Instead mint a Sprite token —
  `POST https://api.sprites.dev/v1/organizations/<org-slug>/tokens` with the raw `FlyV1 …`
  value as `Authorization` (no `Bearer`; org **slug**, not the GraphQL id) — then
  `sprite auth setup --token "<org>/<id>/<tid>/<secret>"`.

### Dropped the self-hosted wasm Fly app
The `econ-sim-noots` Fly app (a static-nginx wasm host stood up earlier this session) was
destroyed at the user's request — Pages stays the wasm host. The `fly.toml`/`Dockerfile`/
`fly-deploy.yml` scaffolding remains in the tree as an optional parallel deploy; the app can
be recreated from it with `fly deploy --ha=false`.

## Verification

- Backend: live HTTP checks above, all green; persistence confirmed across a restart.
- `deploy.sh`: ran its steps (create Sprite, clone, `cargo build --features server`, register
  service, public URL) successfully against the live Sprite.
- **Unverified**: the live wasm `fetch` from Pages (needs the next Pages deploy + a device).
  Backend accepts the real snapshot format (a headless `--save` payload submitted cleanly).

## Notes

- Public URL ⇒ anyone can POST; scores spoofable. Server keeps best-GDP per world; add a
  shared-secret header in `submit` if it ever matters.
- The 48h Fly token used here is ephemeral. For redeploys, the user runs `sprite login`
  (browser) or re-mints a Sprite token per the README.
