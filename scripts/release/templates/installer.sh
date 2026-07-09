#!/bin/sh
# day-installer.sh — install the `day` CLI on macOS or Linux.
#
# Rendered from scripts/release/templates/installer.sh by render-installers.py for release
# __DAY_VERSION__ (URLs, sizes, and sha256 checksums below are baked in per release, in the
# style of cargo-dist's shell installer). Usage:
#
#   curl --proto '=https' --tlsv1.2 -LsSf __DAY_INSTALLER_BASE__/day-installer.sh | sh
#
# Options (environment):
#   DAY_INSTALL_DIR   install directory (default: $CARGO_HOME/bin, ~/.cargo/bin, or ~/.local/bin)

set -eu

APP_NAME="day"
APP_VERSION="__DAY_VERSION__"
BASE_URL="__DAY_BASE_URL__"

say() { printf '%s\n' "$1" >&2; }
err() { say "day-installer: error: $1"; exit 1; }

need_cmd() { command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"; }

# --- platform detection ------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac
case "$os" in
  Darwin) triple="$arch-apple-darwin" ;;
  Linux) triple="$arch-unknown-linux-gnu" ;;
  *) err "unsupported OS: $os (Windows: use day-installer.ps1)" ;;
esac

artifact="day-$triple.tar.gz"

# Baked-in checksum + size per target (rendered per release).
case "$triple" in
  x86_64-apple-darwin)
    sha256="__SHA256_x86_64_apple_darwin__"
    size="__SIZE_x86_64_apple_darwin__"
    ;;
  aarch64-apple-darwin)
    sha256="__SHA256_aarch64_apple_darwin__"
    size="__SIZE_aarch64_apple_darwin__"
    ;;
  x86_64-unknown-linux-gnu)
    sha256="__SHA256_x86_64_unknown_linux_gnu__"
    size="__SIZE_x86_64_unknown_linux_gnu__"
    ;;
  aarch64-unknown-linux-gnu)
    sha256="__SHA256_aarch64_unknown_linux_gnu__"
    size="__SIZE_aarch64_unknown_linux_gnu__"
    ;;
  *) err "no prebuilt binary for $triple" ;;
esac

# --- download ---------------------------------------------------------------
need_cmd tar
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

url="$BASE_URL/$artifact"
say "downloading $APP_NAME $APP_VERSION ($triple, $size bytes)"
say "  from $url"
if command -v curl >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSfL "$url" -o "$tmp/$artifact"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$url" -O "$tmp/$artifact"
else
  err "need curl or wget"
fi

# --- verify -------------------------------------------------------------------
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$artifact" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$tmp/$artifact" | cut -d' ' -f1)"
else
  err "need sha256sum or shasum to verify the download"
fi
[ "$actual" = "$sha256" ] || err "checksum mismatch for $artifact
  expected: $sha256
  actual:   $actual"
say "verified sha256:$sha256"

tar -xzf "$tmp/$artifact" -C "$tmp"
[ -f "$tmp/day" ] || err "archive did not contain the day binary"

# --- install ---------------------------------------------------------------------
# Directory preference mirrors cargo-dist: explicit override, then cargo's bin dir when
# present (already on most Rust developers' PATH), then ~/.local/bin.
if [ -n "${DAY_INSTALL_DIR:-}" ]; then
  dest="$DAY_INSTALL_DIR"
elif [ -n "${CARGO_HOME:-}" ] && [ -d "$CARGO_HOME/bin" ]; then
  dest="$CARGO_HOME/bin"
elif [ -d "$HOME/.cargo/bin" ]; then
  dest="$HOME/.cargo/bin"
else
  dest="$HOME/.local/bin"
fi
mkdir -p "$dest"
install -m 755 "$tmp/day" "$dest/day"
say "installed $dest/day"

case ":$PATH:" in
  *":$dest:"*) ;;
  *) say ""
     say "note: $dest is not on your PATH. Add it, e.g.:"
     say "  export PATH=\"$dest:\$PATH\"" ;;
esac

say ""
say "run 'day --version' to verify, and 'day doctor' to check platform toolchains."
