#!/usr/bin/env bash
# package-cli.sh — turn the CI's raw day-CLI artifacts into release archives named by Rust
# target triple (the cargo-dist convention: day-<triple>.tar.gz / .zip, binary at archive root).
#
#   package-cli.sh <artifacts-dir> <out-dir>
#
# <artifacts-dir> holds one subdirectory per CI artifact (actions/download-artifact layout):
#   day-macos-x86_64/day    day-macos-aarch64/day
#   day-linux-x86_64/day    day-linux-aarch64/day
#   day-windows-x86_64/day.exe   day-windows-aarch64/day.exe
# A missing artifact is a hard error — a release must never silently drop a platform.

set -euo pipefail

artifacts="${1:?usage: package-cli.sh <artifacts-dir> <out-dir>}"
out="${2:?usage: package-cli.sh <artifacts-dir> <out-dir>}"
mkdir -p "$out"

package() {
  local artifact="$1" triple="$2" bin="$3"
  local src="$artifacts/$artifact/$bin"
  [ -f "$src" ] || { echo "package-cli: missing $src" >&2; exit 1; }
  local stage
  stage="$(mktemp -d)"
  cp "$src" "$stage/$bin"
  chmod +x "$stage/$bin"
  case "$bin" in
    *.exe)
      (cd "$stage" && zip -q "day-$triple.zip" "$bin")
      mv "$stage/day-$triple.zip" "$out/"
      echo "  day-$triple.zip"
      ;;
    *)
      tar -czf "$out/day-$triple.tar.gz" -C "$stage" "$bin"
      echo "  day-$triple.tar.gz"
      ;;
  esac
  rm -rf "$stage"
}

echo "packaging release archives into $out"
package day-macos-x86_64 x86_64-apple-darwin day
package day-macos-aarch64 aarch64-apple-darwin day
package day-linux-x86_64 x86_64-unknown-linux-gnu day
package day-linux-aarch64 aarch64-unknown-linux-gnu day
package day-windows-x86_64 x86_64-pc-windows-msvc day.exe
package day-windows-aarch64 aarch64-pc-windows-msvc day.exe
