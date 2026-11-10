#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Shared by local readiness and website CI; does not build or publish the site.
set -euo pipefail
cd "$(dirname "$0")/../.."

if [ ! -d website/node_modules ]; then
  echo 'Website dependencies are missing. Run npm --prefix website ci, then rerun this check.' >&2
  exit 1
fi

bash scripts/ci/docs-symlinks.sh
python3 -B scripts/ci/doc-links.py
npm --prefix website run docs:check
npm --prefix website run docs:test
npm --prefix website run blog:test
npm --prefix website run llms:test
npm --prefix website run strings:check
npm --prefix website run docs:architecture

# New callable names can expose unlinked references even when no Markdown changed.
# Rebuild from this checkout; an old local rustdoc directory cannot validate new APIs.
bash scripts/build-api-docs.sh
node website/scripts/check-api-references.mjs
