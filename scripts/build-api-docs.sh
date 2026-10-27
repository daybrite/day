#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Build rustdoc for the published Day crates the way docs.rs does, to check the site's API links.
#
#   scripts/build-api-docs.sh      # → target/api-check/x86_64-unknown-linux-gnu/doc
#
# The site publishes no API reference of its own: every API link goes to docs.rs
# (https://docs.rs/<package>/latest/<library>/…). This build is only what
# scripts/docs/check-api-links.py and website/scripts/check-api-references.mjs read, so a link to a
# page or anchor docs.rs will not have fails before it is published. It matches docs.rs's build:
# docs.rs's default target, each crate's `[package.metadata.docs.rs]` features, `DOCS_RS=1` for
# build scripts, and `--cfg docsrs`. It documents this checkout, so a link to an API added since the
# last release passes here and resolves on docs.rs once that release is published.
#
# The crate list is website/src/data/api-crates.json, which the /docs/reference/api page also reads.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$here/website/src/data/api-crates.json"
target="x86_64-unknown-linux-gnu" # docs.rs's default target
target_dir="$here/target/api-check"
for tool in cargo jq rustup; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' not found on PATH" >&2; exit 1; }
done
[ -f "$manifest" ] || { echo "error: missing $manifest" >&2; exit 1; }
cd "$here"
rustup target add "$target" >/dev/null

export DOCS_RS=1
export RUSTDOCFLAGS="--cfg docsrs -D warnings"
doc_dir="$target_dir/$target/doc"
rm -rf "$doc_dir"

# Each crate's docs.rs feature list, as cargo reads it: `pkg<TAB>feature,feature` (empty for none).
features="$(cargo metadata --no-deps --format-version 1 --offline \
  | jq -r '.packages[] | "\(.name)\t\((.metadata.docs.rs.features // []) | join(","))"')"
plain=(); n=0
while IFS= read -r pkg; do
  feats="$(awk -F'\t' -v p="$pkg" '$1 == p { print $2 }' <<< "$features")"
  if [ -n "$feats" ]; then
    echo "==> cargo doc -p $pkg --features $feats"
    cargo doc --locked --no-deps --target "$target" --target-dir "$target_dir" -p "$pkg" --features "$feats"
  else
    plain+=(-p "$pkg"); n=$((n + 1))
  fi
done < <(jq -r '.groups[].crates[].pkg' "$manifest")
echo "==> cargo doc ($n crates with default features)"
cargo doc --locked --no-deps --target "$target" --target-dir "$target_dir" "${plain[@]}"

count=$(find "$doc_dir" -maxdepth 1 -type d -name 'day*' | wc -l | tr -d ' ')
echo "==> rustdoc for $count crates in ${doc_dir#"$here/"} (link checks only; not published)"
