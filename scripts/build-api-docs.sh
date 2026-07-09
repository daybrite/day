#!/usr/bin/env bash
# Generate the Rust API reference (rustdoc) for the public-facing Day crates.
#
#   scripts/build-api-docs.sh                     # → target/doc
#   scripts/build-api-docs.sh --out website/dist/api   # also copy the bundle into a site dir
#
# The crate list lives in website/src/data/api-crates.json (the same file the /docs/api bridge page
# reads), so the reference and the page never drift. Every crate documents PORTABLY — no native
# toolkit, no cross-compiler: the core crates need no features and the `day` umbrella uses its headless
# `mock` backend — so this runs on a stock Linux CI runner. A small "back to Day docs" pill is injected
# into every rustdoc page via rustdoc's stable --html-in-header / --html-after-content.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$here/website/src/data/api-crates.json"
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

for tool in cargo jq python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' not found on PATH" >&2; exit 1; }
done
[ -f "$manifest" ] || { echo "error: missing $manifest" >&2; exit 1; }

# --- a back-to-Day link injected into every rustdoc page via rustdoc's stable html-in-header /
# html-after-content hooks. It is `position: fixed`, so it is removed from rustdoc's CSS-grid body
# layout (a flow element would land in a stray grid track) and floats as a small corner pill — robust
# across rustdoc versions. Links are root-relative → same origin the /api bundle is served from.
hdr="$(mktemp)"; after="$(mktemp)"
trap 'rm -f "$hdr" "$after"' EXIT
cat > "$hdr" <<'CSS'
<style>
  .day-api-bar{position:fixed;right:16px;bottom:16px;z-index:1000;
    padding:.42rem .8rem;background:#201512;border:1px solid #B7410E;border-radius:999px;
    display:flex;align-items:center;gap:.55rem;box-shadow:0 2px 12px rgba(0,0,0,.45);
    font:600 .8rem/1 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif;}
  .day-api-bar a{color:#EFA94A;text-decoration:none;}
  .day-api-bar a:hover{text-decoration:underline;}
  .day-api-bar .day-api-home{color:#EFA94A;font-weight:800;letter-spacing:-.01em;}
  .day-api-bar .sep{color:#B7410E;}
  @media (max-width:700px){.day-api-bar{display:none;}}
</style>
CSS
cat > "$after" <<'HTML'
<div class="day-api-bar">
  <a class="day-api-home" href="/docs/api">Day API ↩</a>
  <a href="/docs/overview">Guides</a><span class="sep">·</span>
  <a href="/">daybrite.dev</a>
</div>
HTML

export RUSTDOCFLAGS="--html-in-header $hdr --html-after-content $after"

cd "$here"
rm -rf target/doc

# Crates with no `features` doc together in one pass; each `features` crate (only `day`) docs on its own
# so cargo resolves the right single-backend feature set.
pflags=(); n=0
while IFS= read -r p; do
  [ -n "$p" ] || continue
  pflags+=(-p "$p"); n=$((n + 1))
done < <(jq -r '.groups[].crates[] | select(.features==null) | .pkg' "$manifest")
echo "==> cargo doc ($n portable crates)"
cargo doc --no-deps "${pflags[@]}"

while IFS=$'\t' read -r pkg feats; do
  [ -n "$pkg" ] || continue
  echo "==> cargo doc -p $pkg --features $feats"
  cargo doc --no-deps -p "$pkg" --features "$feats"
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
<meta http-equiv="refresh" content="0; url=/docs/api">
<title>Day API reference</title><link rel="canonical" href="/docs/api"></head>
<body>Redirecting to the <a href="/docs/api">Day API reference</a>…</body></html>
HTML

count=$(find target/doc -maxdepth 1 -type d -name 'day*' | wc -l | tr -d ' ')
echo "==> generated rustdoc for $count crates into target/doc"

if [ -n "$out" ]; then
  mkdir -p "$out"
  # rsync if present (fast, --delete keeps it clean), else cp -R.
  if command -v rsync >/dev/null 2>&1; then rsync -a --delete target/doc/ "$out/"; else rm -rf "$out"/*; cp -R target/doc/. "$out/"; fi
  echo "==> copied bundle into $out"
fi
