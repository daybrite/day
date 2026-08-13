#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
#
# Keep website/src/content/internal/ in step with docs/.
#
#     scripts/ci/docs-symlinks.sh          # check (CI + the lint pre-flight)
#     scripts/ci/docs-symlinks.sh --fix    # create what's missing, drop what's dangling
#
# The internal reference docs ARE the repo's top-level `docs/*.md`, symlinked into the website's
# content collection one file at a time (website/src/content.config.ts). Two ways that drifts, and
# each fails somewhere far from the cause:
#
#   - A doc added WITHOUT its symlink never reaches the site, and linkcheck only notices when some
#     other page happens to link to it — deep-links.md and recorder-matrix.md were both missing for
#     a day before webview-eval.md finally produced a 404.
#   - A doc DELETED without its symlink leaves the symlink dangling, which fails the Astro build.
#
# So both directions are checked. This script is the single implementation: the website CI job and
# scripts/ci/lint.sh both call it, so the pre-flight catches the drift on the machine that caused
# it instead of ten minutes later on a runner.
set -euo pipefail
cd "$(dirname "$0")/../.."

DOCS_DIR="docs"
LINK_DIR="website/src/content/internal"
# From website/src/content/internal/ back to the repo root: four levels up.
REL_PREFIX="../../../../docs"

fix=false
[ "${1:-}" = "--fix" ] && fix=true

names() { find "$1" -maxdepth 1 -name '*.md' -exec basename {} \; | sort; }

docs="$(names "$DOCS_DIR")"
links="$(names "$LINK_DIR")"
missing="$(comm -23 <(echo "$docs") <(echo "$links"))"
# A dangling symlink has no `docs/` file behind it; `find -name '*.md'` still lists it, which is
# exactly why the comparison is by NAME rather than by readability.
dangling="$(comm -13 <(echo "$docs") <(echo "$links"))"

if $fix; then
    changed=false
    for name in $missing; do
        ln -s "$REL_PREFIX/$name" "$LINK_DIR/$name"
        echo "linked   $LINK_DIR/$name -> $REL_PREFIX/$name"
        changed=true
    done
    for name in $dangling; do
        rm -f "$LINK_DIR/$name"
        echo "removed  $LINK_DIR/$name (its docs/ file is gone)"
        changed=true
    done
    $changed || echo "already in step: $(echo "$docs" | wc -l | tr -d ' ') doc(s)"
    exit 0
fi

status=0
if [ -n "$missing" ]; then
    # `::error::` so the message lands on the job summary when this runs in CI; harmless locally.
    echo "::error::docs/ files with no $LINK_DIR symlink — run scripts/ci/docs-symlinks.sh --fix:"
    echo "$missing"
    status=1
fi
if [ -n "$dangling" ]; then
    echo "::error::$LINK_DIR symlinks whose docs/ file is gone — run scripts/ci/docs-symlinks.sh --fix:"
    echo "$dangling"
    status=1
fi
exit $status
