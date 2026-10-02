#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Check generated tables against source content, even when changes are unstaged.
# --staged checks the index instead, so an unstaged repair cannot hide a broken commit.
set -euo pipefail
cd "$(dirname "$0")/../.."

case "${1:-}" in
  --staged)
    snapshot="$(mktemp -d "${TMPDIR:-/tmp}/day-matrices.XXXXXX")"
    trap 'rm -rf "$snapshot"' EXIT
    # Export the complete index: generators also inspect manifests and file existence.
    # A source-extension allow-list can silently drop inputs and invent matrix drift.
    git checkout-index --all --prefix="$snapshot/"
    bash "$snapshot/scripts/ci/check-matrices.sh"
    ;;
  "")
    for name in duty coverage recorder; do
      bash "scripts/ci/$name-matrix.sh" --check
    done
    ;;
  *) echo "usage: bash scripts/ci/check-matrices.sh [--staged]" >&2; exit 2 ;;
esac
