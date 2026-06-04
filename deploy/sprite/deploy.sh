#!/usr/bin/env bash
# Deploy the econ-sim leaderboard server to a Fly.io Sprite.
#
# Prereqs (one-time, on your machine):
#   curl -fsSL https://sprites.dev/install.sh | sh   # install the `sprite` CLI
#   sprite login                                     # browser OAuth with your Fly.io account
#
# Headless / CI (no browser): mint a Sprite token from a FlyV1 token and feed it to the CLI:
#   curl -s -X POST https://api.sprites.dev/v1/organizations/<org-slug>/tokens \
#     -H "Authorization: $FLY_API_TOKEN" -H 'Content-Type: application/json' \
#     -d '{"name":"econ-sim-deploy"}'                # -> {"token":"<org>/<id>/<tid>/<secret>"}
#   sprite auth setup --token "<org>/<id>/<tid>/<secret>"
# (Use the org *slug* — e.g. `josh-kuhn`, not the GraphQL org id — or the mint 401s.)
#
# Then just run this script. It is idempotent — re-run it to ship a new build.
#
#   ./deploy/sprite/deploy.sh
#
# The Sprite's base image already has Rust + git, the URL auto-routes to :8080, and the
# filesystem persists, so the server's leaderboard.json survives sleeps/restarts. The
# server runs as a Sprite "service" (auto-starts on boot, restarts on cold wake); the URL
# wakes the Sprite on the next request, so it costs ~nothing while idle.
set -euo pipefail

SPRITE="${SPRITE_NAME:-econ-leaderboard}"
REPO="${REPO_URL:-https://github.com/deontologician/econ-sim.git}"
BRANCH="${BRANCH:-main}"
SERVICE="leaderboard"
BIN="/home/sprite/econ-sim/target/release/server"

echo "==> Ensuring Sprite '$SPRITE' exists"
if ! sprite list 2>/dev/null | grep -qw "$SPRITE"; then
  sprite create --skip-console "$SPRITE"
fi
sprite use "$SPRITE"

# `sprite exec` parses flags before `--`, so `bash -lc` is read as sprite's own flags
# (`unknown shorthand flag: 'l'`). The `--` separator hands everything after it to the
# remote command untouched.
echo "==> Cloning/updating the repo and building the server (release, --features server)"
sprite exec -- bash -c "
  set -e
  if [ -d /home/sprite/econ-sim/.git ]; then
    git -C /home/sprite/econ-sim fetch --depth 1 origin '$BRANCH'
    git -C /home/sprite/econ-sim checkout -B '$BRANCH' 'origin/$BRANCH'
  else
    git clone --depth 1 --branch '$BRANCH' '$REPO' /home/sprite/econ-sim
  fi
  cd /home/sprite/econ-sim
  cargo build --release --no-default-features --features server --bin server
"

# Forward the LLM tech-naming config to the service env *if present in this shell* — so the
# secret never lives in the repo, only in your deploy environment. Without OPENROUTER_API_KEY
# the server still grows techs, just with procedural names. GROW_TICK_SECS / OPENROUTER_MODEL
# are optional tuning overrides.
ENVPAIRS=""; ENVKEYS=""
# `if` (not `&&`) so an empty value returns 0 — under `set -e` a function ending in a false
# test would abort the whole script.
add_env() {
  if [ -n "$2" ]; then
    ENVPAIRS="${ENVPAIRS:+$ENVPAIRS,}$1=$2"
    ENVKEYS="${ENVKEYS:+$ENVKEYS,}$1"
  fi
}
add_env OPENROUTER_API_KEY "${OPENROUTER_API_KEY:-}"
add_env OPENROUTER_MODEL "${OPENROUTER_MODEL:-}"
add_env GROW_TICK_SECS "${GROW_TICK_SECS:-}"
ENVFLAG=""
[ -n "$ENVPAIRS" ] && ENVFLAG="--env '$ENVPAIRS'"

# `--http-port 8080` tells the Sprite proxy to route inbound HTTP to the server and
# auto-start it on the first request after an idle sleep. `--dir /home/sprite` keeps the
# cwd at the persistent home dir so leaderboard.json lands at /home/sprite/leaderboard.json.
echo "==> (Re)registering the '$SERVICE' service on :8080${ENVKEYS:+ (env: $ENVKEYS)}"
sprite exec -- bash -c "
  sprite-env services delete '$SERVICE' >/dev/null 2>&1 || true
  sprite-env services create '$SERVICE' --cmd '$BIN' --dir /home/sprite --http-port 8080 $ENVFLAG --no-stream
"

echo "==> Making the URL public (so browser sims can POST without a token)"
sprite update --url-auth public 2>/dev/null || sprite url update --auth public

URL="$(sprite info 2>/dev/null | grep -oE 'https://[^ ]*sprites\.(app|dev)' | head -1)"
echo
echo "==> Done. Leaderboard server is live at:"
echo "      ${URL:-<run: sprite info>}"
echo
echo "The Pages build defaults LEADERBOARD_URL to this Sprite (see .github/workflows/"
echo "deploy.yml). To point it elsewhere, set the repo variable LEADERBOARD_SUBMIT_URL to:"
echo "      ${URL%/}/submit"
echo "(Settings → Secrets and variables → Actions → Variables), then re-run the Pages deploy."
