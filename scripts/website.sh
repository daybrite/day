#!/usr/bin/env bash
# Build and launch the day website (website/) locally.
#
#   scripts/website.sh            # install deps + start the dev server (http://localhost:4321/)
#   scripts/website.sh dev        # same as above
#   scripts/website.sh build      # production build into website/dist
#   scripts/website.sh preview     # build, then serve the production output
#
# Screenshots for the gallery are produced by CI, not locally — the gallery integration emits
# placeholder tiles for local builds automatically, so no artifacts are required here.
set -euo pipefail

cmd="${1:-dev}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
site="$here/website"

cd "$site"

if ! command -v npm >/dev/null 2>&1; then
  echo "error: npm not found. Install Node.js >= 22.12 (https://nodejs.org)." >&2
  exit 1
fi

# Install once (or when the lockfile changed). Prefer the reproducible `npm ci` when a lockfile
# exists; fall back to `npm install` on first run to create one.
if [ ! -d node_modules ]; then
  if [ -f package-lock.json ]; then
    echo "==> npm ci"
    npm ci
  else
    echo "==> npm install"
    npm install
  fi
fi

case "$cmd" in
  dev)
    echo "==> starting dev server (http://localhost:4321/) — Ctrl-C to stop"
    exec npm run dev
    ;;
  build)
    echo "==> building production site into website/dist"
    exec npm run build
    ;;
  preview)
    echo "==> building, then serving the production output"
    npm run build
    exec npm run preview
    ;;
  *)
    echo "usage: scripts/website.sh [dev|build|preview]" >&2
    exit 2
    ;;
esac
