#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# assert-pristine.sh [dir] — fail unless the checkout has no uncommitted changes (§20.4).
#
# `day pack` records HEAD and a `dirty` flag in the SBOM, and `day rebuild` refuses an artifact
# whose flag is set: a commit cannot describe a tree that has extra files in it. So a packing job
# must keep the checkout pristine — whatever it downloads or generates belongs in `$RUNNER_TEMP`,
# not in the workspace, which on GitHub Actions IS the checkout.
#
# Run this immediately before `day pack`, so a stray path names itself here instead of surfacing a
# job later as "no rebuild can reproduce it" with nothing to point at.
set -euo pipefail

dir="${1:-.}"

changes="$(git -C "$dir" status --porcelain)"
if [ -n "$changes" ]; then
  echo "::error::the working tree is not pristine, so the artifact this job packs could not be rebuilt from its commit"
  printf '%s\n' "$changes"
  echo "Untracked files count too: they are inputs the commit does not describe. Put anything a"
  echo "job downloads or generates under \$RUNNER_TEMP instead of the workspace."
  exit 1
fi
echo "working tree pristine at $(git -C "$dir" rev-parse --short HEAD)"
