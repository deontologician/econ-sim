#!/usr/bin/env bash
# Deploy the econ-sim leaderboard server to a Fly.io Sprite.
#
# Prereqs (one-time, on your machine):
#   curl -fsSL https://sprites.dev/install.sh | sh   # install the `sprite` CLI
#   sprite org auth                                  # log in with your Fly.io account
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

echo "==> Cloning/updating the repo and building the server (release, --features server)"
sprite exec bash -lc "
  set -euo pipefail
  if [ -d /home/sprite/econ-sim/.git ]; then
    git -C /home/sprite/econ-sim fetch --depth 1 origin '$BRANCH'
    git -C /home/sprite/econ-sim checkout -B '$BRANCH' 'origin/$BRANCH'
  else
    git clone --depth 1 --branch '$BRANCH' '$REPO' /home/sprite/econ-sim
  fi
  cd /home/sprite/econ-sim
  cargo build --release --no-default-features --features server --bin server
"

echo "==> (Re)registering the '$SERVICE' service on :8080"
# Replace any prior service so the new binary is picked up. Wrap in bash so cwd is the
# persistent home dir (leaderboard.json lands at /home/sprite/leaderboard.json) and signals
# reach the server via exec. PORT defaults to 8080 (the Sprite's HTTP route).
sprite exec bash -lc "sprite-env services delete '$SERVICE' >/dev/null 2>&1 || true"
sprite exec bash -lc "sprite-env curl -X PUT '/v1/services/$SERVICE?duration=4s' -d '$(cat <<JSON
{"cmd":"/bin/bash","args":["-lc","cd /home/sprite && exec $BIN"]}
JSON
)'"

echo "==> Making the URL public (so browser sims can POST without a token)"
sprite url update --auth public

URL="$(sprite url 2>/dev/null | grep -oE 'https://[^ ]+' | head -1)"
echo
echo "==> Done. Leaderboard server is live at:"
echo "      ${URL:-<run: sprite url>}"
echo
echo "Next: set the GitHub repo variable LEADERBOARD_SUBMIT_URL to:"
echo "      ${URL%/}/submit"
echo "(Settings → Secrets and variables → Actions → Variables), then re-run the Pages"
echo "deploy. The wasm app will start POSTing snapshots every 1000 ticks."
