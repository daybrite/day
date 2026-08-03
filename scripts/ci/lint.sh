#!/usr/bin/env bash
# Pre-flight lint — run CI's whole fmt + clippy gate locally, BEFORE pushing, so a fmt drift or a
# clippy warning can't reach CI. It exists because the gate is a matrix, not one command: toolkit
# and part crates are NOT in default-members and each compiles only under its own backend feature
# and/or cross-target, so a bare `cargo clippy` silently skips them — that blind spot has shipped
# `useless_conversion` (day-android) and unformatted imports to CI more than once.
#
#     scripts/ci/lint.sh            # run every leg this machine can
#     scripts/ci/lint.sh -q         # quieter: only per-leg pass/fail + the summary
#
# Each leg mirrors a command in .github/workflows/ci.yml under its `RUSTFLAGS: -D warnings`. A leg
# whose toolchain is absent (a rustup target not installed, a GUI lib missing, the wrong OS) is
# SKIPPED with a printed reason — never silently — and the summary lists skips so "green here" is
# never mistaken for "green everywhere". Exit is nonzero if any leg failed.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1

# CI runs the whole gate under -D warnings; match it so a warning fails here too. Overridable.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"
# clippy 1.97 false-positives missing_const_for_thread_local on the bionic/musl cross-targets
# (android/ohos), flagging thread_local!s that already use `const { … }`. The host leg enforces the
# lint accurately; suppress it only on those cross legs, exactly as ci.yml does.
XCROSS=(-A clippy::missing_const_for_thread_local)

QUIET=0; [ "${1:-}" = "-q" ] && QUIET=1
OS="$(uname -s)"
INSTALLED="$(rustup target list --installed 2>/dev/null || true)"
have_target() { grep -qx "$1" <<<"$INSTALLED"; }

FAILED=(); SKIPPED=()
leg() { # leg <label> <cmd...>
  local label="$1"; shift
  printf '\n\033[1m▶ %s\033[0m\n' "$label"
  if [ "$QUIET" = 1 ]; then
    local out; out="$("$@" 2>&1)"; local rc=$?
    [ $rc -eq 0 ] || printf '%s\n' "$out"
  else
    "$@"; local rc=$?
  fi
  if [ $rc -eq 0 ]; then printf '\033[32m✓ %s\033[0m\n' "$label"
  else printf '\033[31m✗ %s (exit %d)\033[0m\n' "$label" "$rc"; FAILED+=("$label"); fi
}
skip() { printf '\033[33m− SKIP %s — %s\033[0m\n' "$1" "$2"; SKIPPED+=("$1: $2"); }

# 1) Formatting — the whole workspace, the exact command CI fails on.
leg "fmt --all --check" cargo fmt --all -- --check

# 2) Host clippy — the default members plus the CLI, dayscript, and mock-backend showcase.
leg "clippy host (default members)"    cargo clippy --locked --all-targets
leg "clippy host day-cli + day-script"  cargo clippy --locked -p day-cli -p day-script --all-targets
leg "clippy host showcase (mock)"       cargo clippy --locked -p showcase --no-default-features --features mock --all-targets

# 3) Cross-target + feature-gated backends. Each pulls in its toolkit crate (day-android, day-arkui,
#    day-appkit, …) — the crates a host clippy never compiles.
if have_target aarch64-linux-android; then
  leg "clippy android (mdc)" cargo clippy --locked --target aarch64-linux-android -p showcase --lib \
    --no-default-features --features mdc -- "${XCROSS[@]}"
else skip "clippy android (mdc)" "rustup target add aarch64-linux-android"; fi

# arkui needs the OpenHarmony NDK for day-arkui-sys's build.rs and ring's C compile — CI exports it;
# skip (don't fail) when it's absent locally, the same posture as a missing rustup target.
if have_target aarch64-unknown-linux-ohos && [ -n "${OHOS_NDK_HOME:-}" ]; then
  leg "clippy harmonyos (arkui)" cargo clippy --locked --target aarch64-unknown-linux-ohos -p showcase --lib \
    --no-default-features --features arkui -- "${XCROSS[@]}"
elif have_target aarch64-unknown-linux-ohos; then
  skip "clippy harmonyos (arkui)" "export OHOS_NDK_HOME to the OpenHarmony NDK native dir"
else skip "clippy harmonyos (arkui)" "rustup target add aarch64-unknown-linux-ohos"; fi

if have_target wasm32-unknown-unknown; then
  # CI only *builds* web-dom (no clippy leg), so this is a local superset — clippy subsumes the
  # build's warning check and additionally keeps the dom backend clippy-clean.
  leg "clippy web-dom (dom)" cargo clippy --locked --target wasm32-unknown-unknown -p showcase --lib \
    --no-default-features --features dom
else skip "clippy web-dom (dom)" "rustup target add wasm32-unknown-unknown"; fi

if [ "$OS" = Darwin ]; then
  leg "clippy appkit" cargo clippy --locked -p showcase --no-default-features --features appkit --all-targets
  if have_target aarch64-apple-ios-sim; then
    leg "clippy uikit (ios-sim)" cargo clippy --locked --target aarch64-apple-ios-sim -p showcase --lib \
      --no-default-features --features uikit
  else skip "clippy uikit (ios-sim)" "rustup target add aarch64-apple-ios-sim"; fi
else
  skip "clippy appkit"       "macOS only"
  skip "clippy uikit (ios-sim)" "macOS only"
fi

# gtk/qt are portable but need their native libs (pkg-config finds them).
if pkg-config --exists gtk4 2>/dev/null; then
  leg "clippy gtk" cargo clippy --locked -p showcase --no-default-features --features gtk --all-targets
else skip "clippy gtk" "no gtk4 (brew install gtk4 libadwaita)"; fi
if pkg-config --exists Qt6Core 2>/dev/null; then
  leg "clippy qt" cargo clippy --locked -p showcase --no-default-features --features qt --all-targets
else skip "clippy qt" "no Qt6 (brew install qt)"; fi

case "$OS" in
  MINGW*|MSYS*|CYGWIN*)
    leg "clippy xaml" cargo clippy --locked -p showcase --no-default-features --features xaml --all-targets ;;
  *) skip "clippy xaml" "Windows only" ;;
esac

# 4) Headless part crates: no backend feature, not in default-members — bare clippy never reaches
#    them. Lint each on host + the Android cross-target, exactly as ci.yml's loop does.
if [ -d parts ]; then
  for dir in parts/day-part-*/; do
    [ -f "$dir/Cargo.toml" ] || continue
    P="$(basename "$dir")"
    leg "clippy $P (host)" cargo clippy --locked -p "$P" --all-targets
    if have_target aarch64-linux-android; then
      leg "clippy $P (android)" cargo clippy --locked --target aarch64-linux-android -p "$P" -- "${XCROSS[@]}"
    else skip "clippy $P (android)" "rustup target add aarch64-linux-android"; fi
  done
fi

# 4) Generated conformance tables (docs/duty-matrix.md, docs/coverage-matrix.md) — the same drift
# checks CI runs. Two ways to fail: changing the Toolkit trait or a backend without regenerating,
# or changing the SHAPE the generators detect (renaming a realize match arm, moving the kinds
# table) so a generator silently stops seeing what it measures — that one emits an empty table
# rather than an error, so only the diff catches it. Runs last because it rewrites the two files
# in place: on failure they are left regenerated, so `git diff` shows exactly what moved.
drift() { # drift <generator> <generated-file>
  "$1" >/dev/null && git diff --exit-code -- "$2" >/dev/null
}
leg "duty-matrix drift" drift scripts/ci/duty-matrix.sh docs/duty-matrix.md
leg "coverage-matrix drift" drift scripts/ci/coverage-matrix.sh docs/coverage-matrix.md

# ── summary ────────────────────────────────────────────────────────────────────────────────────
printf '\n\033[1m── lint summary ──\033[0m\n'
[ ${#SKIPPED[@]} -eq 0 ] || { printf '\033[33mskipped:\033[0m\n'; printf '  %s\n' "${SKIPPED[@]}"; }
if [ ${#FAILED[@]} -eq 0 ]; then
  printf '\033[32mPASS — every leg this machine ran is clean.\033[0m\n'
  exit 0
else
  printf '\033[31mFAIL (%d): %s\033[0m\n' "${#FAILED[@]}" "${FAILED[*]}"
  exit 1
fi
