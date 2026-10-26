#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Generate the Rust API reference (rustdoc) for the public-facing Day crates.
#
#   scripts/build-api-docs.sh                     # → target/doc
#   scripts/build-api-docs.sh --out website/dist/api   # also copy the bundle into a site dir
#
# The crate list lives in website/src/data/api-crates.json (the same file the /docs/reference/api bridge page
# reads), so the reference and the page never drift. Every crate documents portably, with no native
# toolkit and no cross-compiler: optional `model` integrations are included and the `day-app` umbrella
# uses its headless `mock` backend, so this runs on a stock Linux CI runner. Build the Astro site
# first: its footer, theme script, and exact stylesheets are reused through Rustdoc HTML hooks.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$here/website/src/data/api-crates.json"
out=""
site_dir="$here/website/dist"
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --site-dir) site_dir="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

for tool in cargo jq python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' not found on PATH" >&2; exit 1; }
done
[ -f "$manifest" ] || { echo "error: missing $manifest" >&2; exit 1; }

# Reuse the already-built site's actual chrome instead of maintaining an API-only copy.
shell_dir="$(mktemp -d)"
trap 'rm -rf "$shell_dir"' EXIT
python3 "$here/scripts/docs/rustdoc-shell.py" --site-dir "$site_dir" --out "$shell_dir"
export RUSTDOCFLAGS="--html-in-header $shell_dir/head.html --html-before-content $shell_dir/before.html --html-after-content $shell_dir/after.html -D warnings"

cd "$here"
rm -rf target/doc

# Rustdoc discovers local cross-crate link destinations from directories in its output tree.
# With --no-deps, Cargo may document a consumer before its dependency, leaving raw Rust paths
# in the HTML. Reserve every published crate's directory before building, and serialize rustdoc
# so another invocation cannot replace a directory while it is being inspected. Subsequent CI
# bundle and item-link checks still require the actual pages and anchors.
while IFS= read -r crate_dir; do
  mkdir -p "target/doc/$crate_dir"
done < <(jq -r '.groups[].crates[] | .lib // (.pkg | gsub("-"; "_"))' "$manifest")

# Crates with no `features` doc together in one pass; each feature-enabled crate docs on its own
# so cargo resolves the right single-backend feature set.
pflags=(); n=0
while IFS= read -r p; do
  [ -n "$p" ] || continue
  pflags+=(-p "$p"); n=$((n + 1))
done < <(jq -r '.groups[].crates[] | select(.features==null) | .pkg' "$manifest")
echo "==> cargo doc ($n portable crates)"
cargo doc --locked --no-deps --jobs 1 "${pflags[@]}"

while IFS=$'\t' read -r pkg feats; do
  [ -n "$pkg" ] || continue
  echo "==> cargo doc -p $pkg --features $feats"
  cargo doc --locked --no-deps -p "$pkg" --features "$feats"
done < <(jq -r '.groups[].crates[] | select(.features!=null) | "\(.pkg)\t\(.features)"' "$manifest")

# Repoint every "Source" link at the canonical source on GitHub (and drop the local source viewer),
# so the hosted reference links back to the repo rather than a local src/*.html copy.
repo=$(jq -r '.repo // "https://github.com/daybrite/day"' "$manifest")
ref=$(jq -r '.branch // "main"' "$manifest")
echo "==> repoint Source links to $repo (@$ref)"
python3 "$here/scripts/rustdoc-github-source.py" --doc-dir target/doc --repo "$repo" --ref "$ref"

# cargo doc makes no root landing page for a multi-crate build; send /api/ to the styled bridge page.
cat > target/doc/index.html <<'HTML'
<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="refresh" content="0; url=/docs/reference/api">
<title>Day API reference</title><link rel="canonical" href="/docs/reference/api"></head>
<body>Redirecting to the <a href="/docs/reference/api">Day API reference</a>…</body></html>
HTML

count=$(find target/doc -maxdepth 1 -type d -name 'day*' | wc -l | tr -d ' ')
echo "==> generated rustdoc for $count crates into target/doc"

if [ -n "$out" ]; then
  mkdir -p "$out"
  # rsync if present (fast, --delete keeps it clean), else cp -R.
  if command -v rsync >/dev/null 2>&1; then rsync -a --delete target/doc/ "$out/"; else rm -rf "$out"/*; cp -R target/doc/. "$out/"; fi
  echo "==> copied bundle into $out"
fi
