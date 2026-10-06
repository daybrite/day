#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
#
# Regenerate the conformance app's scaffold from THIS checkout's app template (docs/testing.md).
#
# Only the app's own files are source: Cargo.toml (the checkout's crates by path, with the
# `conformance` feature), src/, dayscript/, README.md and this script. The rest (Day.toml,
# build.rs, resource/ and the platform/ host projects) is `day new app` output, written fresh
# here so the app never runs on a host project an older template wrote. CI runs this as the
# conformance call's `setup-command`; run it once after a clone, and again after a template
# change, before `day test`.
#
#     apps/conformance/generate.sh                     # the checkout's own CLI (cargo run)
#     DAY_BIN=/path/to/day apps/conformance/generate.sh
#
# build/ and target/ are left alone: they are caches, and a regenerated scaffold reuses them.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
if [ -n "${DAY_BIN:-}" ]; then
  day=("$DAY_BIN")
else
  day=(cargo run --manifest-path "$repo/Cargo.toml" -q -p day-cli --)
fi

# Every target the CLI ships, so the app can be driven wherever `day test` runs.
targets=macos-appkit,ios-uikit,android-mdc,macos-gtk,macos-qt,linux-gtk,linux-qt,windows-winui,windows-xaml,harmony-arkui,web-dom

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# Quiet unless it fails: its "next: cd conformance" advice is about the scratch copy.
if ! log="$(cd "$scratch" && "${day[@]}" new app conformance --toolkit "$targets" \
  --appid dev.daybrite.conformance --title "Day Conformance" --no-github --no-website \
  --no-input 2>&1)"; then
  echo "$log" >&2
  exit 1
fi

# `day new` links two HarmonyOS media directories to icons it rendered under the scratch
# copy's build/. Copied, those links dangle, and Git Bash on Windows cannot create a dangling
# link at all. `day prepare` makes them again on every build (the template ignores them).
find "$scratch/conformance/platform" -type l -exec rm -f {} +

# An overlay: each generated part is replaced whole, and nothing else in the app is touched.
for part in Day.toml build.rs resource platform; do
  rm -rf "${here:?}/$part"
  cp -R "$scratch/conformance/$part" "$here/$part"
done
echo "regenerated the conformance app's scaffold in $here"
