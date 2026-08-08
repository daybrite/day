#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Point this clone's git hooks at the tracked .githooks/ directory.
#
# Git does not version .git/hooks, so a hook only exists in the clone that created it. `core.hooksPath`
# is the supported way to keep them in the tree instead: run this once per clone.
#
#     scripts/install-hooks.sh
#
# Undo with `git config --unset core.hooksPath`; skip a single commit with `git commit --no-verify`.
set -euo pipefail
cd "$(dirname "$0")/.."

git config core.hooksPath .githooks
chmod +x .githooks/*

echo "core.hooksPath = $(git config --get core.hooksPath)"
for h in .githooks/*; do
    echo "  $(basename "$h")"
done
