#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Pre-flight lint: run CI's whole fmt + clippy gate locally, before pushing, so a fmt drift or a
# clippy warning can't reach CI. It exists because the gate is a matrix, not one command: toolkit
# and part crates are not in default-members and each compiles only under its own backend feature
# and/or cross-target, so a bare `cargo clippy` silently skips them; that blind spot has shipped
# `useless_conversion` (day-mdc) and unformatted imports to CI more than once.
#
#     scripts/ci/lint.sh            # run every leg this machine can
#     scripts/ci/lint.sh -q         # quieter: only per-leg pass/fail + the summary
#
# Each leg mirrors a command in .github/workflows/ci.yml under its `RUSTFLAGS: -D warnings`. A leg
# whose toolchain is absent (a rustup target not installed, a GUI lib missing, the wrong OS) is
# skipped with a printed reason, never silently, and the summary lists skips so "green here" is
# never mistaken for "green everywhere". Exit is nonzero if any leg failed.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
ROOT="$PWD"

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

# The showcase app is its own repository now (daybrite/Day-Showcase). Its clippy legs run in a
# checkout of it, built against this checkout via `day patch`, so a framework change is still
# linted against the app that exercises every backend. No checkout ⇒ those legs skip with a reason
# rather than silently disappearing, which is the contract of this script.
SHOWCASE="${SHOWCASE:-$ROOT/../Day-Showcase}"
if [ -f "$SHOWCASE/Day.toml" ]; then
  # Additional local dependencies (one checkout per line) participate in the same patch table.
  # This avoids replacing a developer's external-crate changes with Git sources during lint.
  local_patches=(--local "$ROOT")
  while IFS= read -r checkout; do
    [ -z "$checkout" ] || local_patches+=(--local "$checkout")
  done <<< "${DAY_LINT_LOCAL_CHECKOUTS:-}"
  cargo run -q -p day-cli -- --project "$SHOWCASE" patch "${local_patches[@]}" --check >/dev/null 2>&1 \
    || { printf '\033[33m− the showcase patch table could not be verified; its legs will skip\033[0m\n'; SHOWCASE=""; }
else
  SHOWCASE=""
fi
# Run a clippy leg inside the showcase checkout (the app is no longer `-p showcase` here).
app_leg() {
  local label="$1"; shift
  if [ -z "$SHOWCASE" ]; then
    skip "$label" "no showcase checkout — clone daybrite/Day-Showcase beside day/ or set SHOWCASE=<path>"
    return
  fi
  leg "$label" env -C "$SHOWCASE" "$@"
}

# 1) The mandatory readiness gate checks matrices, formatting, and host Clippy against
# working-tree content. Correct, uncommitted generated files are valid inputs.
leg "readiness (matrices, fmt, host clippy)" bash scripts/ci/check-ready.sh

# 1b) Spelling + the American-English rule (typos.toml, STYLE_GUIDE.md), the local twin of
# ci.yml's `spelling` job. Not installed is a skip rather than a failure, since it is the one leg
# here that needs a tool outside the Rust toolchain; CI still runs it either way.
if command -v typos >/dev/null 2>&1; then
  leg "typos (spelling + en-us)" typos
else
  skip "typos (spelling + en-us)" "not installed — brew install typos-cli, or cargo install typos-cli"
fi

# 2) The showcase exercises the framework through an app's dependency graph.
app_leg "clippy showcase (mock)" cargo clippy --no-default-features --features mock --all-targets

# 3) Cross-target + feature-gated backends. Each pulls in its toolkit crate (day-mdc, day-arkui,
#    day-appkit, …), the crates a host clippy never compiles.
# The Android and OpenHarmony legs compile C too (bundled SQLite in day-persistence, ring under
# day-part-http's fallback TLS, napi-ohos), and cc-rs never finds an NDK's cross clang on its own.
# The same cargo subcommands `day build` drives supply the cross environment here: `cargo ndk-env`
# prints cargo-ndk's (the compilers, archiver and bindgen args for the NDK it located), and
# `cargo ohos clippy` runs clippy inside cargo-ohos's. Without the tool, or without an NDK for it
# to find, the leg skips with the install hint rather than failing, the same posture as a missing
# rustup target.
HAVE_ANDROID_ENV=0
if have_target aarch64-linux-android; then
  if ! command -v cargo-ndk >/dev/null 2>&1; then
    skip "clippy android (mdc)" "cargo install cargo-ndk"
  # `--platform 24`: the scaffold's minSdk (DayAndroidPlugin.MIN_SDK), the level `day build`
  # passes cargo-ndk too, so the lint links against the same sysroot the APK ships for.
  elif NDK_ENV="$(cargo ndk-env -t arm64-v8a --platform 24 2>/dev/null)"; then
    eval "$NDK_ENV"
    HAVE_ANDROID_ENV=1
    app_leg "clippy android (mdc)" cargo clippy --target aarch64-linux-android --lib \
      --no-default-features --features mdc -- "${XCROSS[@]}"
  else skip "clippy android (mdc)" "no Android NDK found (set ANDROID_NDK_HOME)"; fi
else skip "clippy android (mdc)" "rustup target add aarch64-linux-android"; fi

if have_target aarch64-unknown-linux-ohos; then
  if ! command -v cargo-ohos >/dev/null 2>&1; then
    skip "clippy harmonyos (arkui)" "cargo install --locked cargo-ohos (or cargo binstall cargo-ohos)"
  # cargo-ohos resolves the SDK (OHOS_SDK_NATIVE, DevEco Studio, or its own `cargo ohos init sdk`
  # download) and exports it as OHOS_SDK_NATIVE; napi-ohos's build script still reads the NDK as
  # OHOS_NDK_HOME, the one variable cargo-ohos does not set, so bridge it (crates/day-cli/ohos.rs
  # does the same for `day build`).
  elif OHOS_SDK_NATIVE_LINE="$(cargo ohos env --format sh -t aarch64 2>/dev/null | grep '^export OHOS_SDK_NATIVE=')"; then
    eval "$OHOS_SDK_NATIVE_LINE"
    export OHOS_NDK_HOME="$OHOS_SDK_NATIVE"
    app_leg "clippy harmonyos (arkui)" cargo ohos clippy --target aarch64-unknown-linux-ohos --lib \
      --no-default-features --features arkui -- "${XCROSS[@]}"
  else
    skip "clippy harmonyos (arkui)" "no OpenHarmony SDK: cargo ohos init sdk --api 18 --components native,toolchains (or set OHOS_SDK_NATIVE)"
  fi
else skip "clippy harmonyos (arkui)" "rustup target add aarch64-unknown-linux-ohos"; fi

if have_target wasm32-unknown-unknown; then
  # CI only *builds* web-dom (no clippy leg), so this is a local superset: clippy subsumes the
  # build's warning check and additionally keeps the dom backend clippy-clean. The
  # getrandom_backend cfg mirrors what `day build` sets for every web app (crates/day-cli/web.rs):
  # day-dom carries getrandom's custom-backend hook, and getrandom refuses to compile for raw
  # wasm until a backend is chosen.
  app_leg "clippy web-dom (dom)" env "RUSTFLAGS=$RUSTFLAGS --cfg getrandom_backend=\"custom\"" \
    cargo clippy --target wasm32-unknown-unknown --lib \
    --no-default-features --features dom
else skip "clippy web-dom (dom)" "rustup target add wasm32-unknown-unknown"; fi

if [ "$OS" = Darwin ]; then
  app_leg "clippy appkit" cargo clippy --no-default-features --features appkit --all-targets
  if have_target aarch64-apple-ios-sim; then
    app_leg "clippy uikit (ios-sim)" cargo clippy --target aarch64-apple-ios-sim --lib \
      --no-default-features --features uikit
  else skip "clippy uikit (ios-sim)" "rustup target add aarch64-apple-ios-sim"; fi
else
  skip "clippy appkit"       "macOS only"
  skip "clippy uikit (ios-sim)" "macOS only"
fi

# gtk/qt are portable but need their native libs (pkg-config finds them).
if pkg-config --exists gtk4 2>/dev/null; then
  app_leg "clippy gtk" cargo clippy --no-default-features --features gtk --all-targets
else skip "clippy gtk" "no gtk4 (brew install gtk4 libadwaita)"; fi
if pkg-config --exists Qt6Core 2>/dev/null; then
  app_leg "clippy qt" cargo clippy --no-default-features --features qt --all-targets
else skip "clippy qt" "no Qt6 (brew install qt)"; fi

case "$OS" in
  MINGW*|MSYS*|CYGWIN*)
    app_leg "clippy xaml" cargo clippy --no-default-features --features xaml --all-targets ;;
  *) skip "clippy xaml" "Windows only" ;;
esac

# 4) Headless part crates: no backend feature, not in default-members; bare clippy never reaches
#    them. Lint each on host + the Android cross-target, exactly as ci.yml's loop does.
if [ -d parts ]; then
  for dir in parts/day-part-*/; do
    [ -f "$dir/Cargo.toml" ] || continue
    P="$(basename "$dir")"
    leg "clippy $P (host)" cargo clippy --locked -p "$P" --all-targets
    # Under the cargo-ndk environment the mdc leg above imported, so a part's C compiles here too.
    if [ "$HAVE_ANDROID_ENV" = 1 ]; then
      leg "clippy $P (android)" cargo clippy --locked --target aarch64-linux-android -p "$P" -- "${XCROSS[@]}"
      # …and again with the staged-bridge cfg, which compiles the other half of a bridged part.
      # A daybridge arm in a staged language (Java/Kotlin/Swift/ArkTS/JS) is behind
      # `cfg(day_bridge_staged)`, set only when `day build` has staged the foreign half into the
      # host project; a bare cargo build compiles the `platforms = [other]` fallback instead. So
      # each configuration hides the other's warnings, and the leg above sees only the fallback,
      # which is how an unused import in day-part-clipboard's android arm reached CI as a build
      # failure inside `cargo ndk`, having passed every lint here.
      RUSTFLAGS="$RUSTFLAGS --cfg day_bridge_staged" \
        leg "clippy $P (android, staged bridge)" \
        cargo clippy --locked --target aarch64-linux-android -p "$P" -- "${XCROSS[@]}"
    else skip "clippy $P (android)" "the Android cross environment (see the mdc leg's skip)"; fi
  done
fi

# 5) Website content wiring: every docs/*.md needs its symlink in website/src/content/internal/
# (and no symlink may outlive its doc). Cheap, and it fails on the machine that added the doc
# instead of on a runner after the whole matrix has built, which is how window-image.md reached
# CI. `scripts/ci/docs-symlinks.sh --fix` repairs both directions.
leg "docs symlinks" scripts/ci/docs-symlinks.sh
# Doc cross-references must be real links (bare `docs/foo.md` text is unclickable on the site
# and 404s on GitHub), and every relative link target must exist.
leg "doc links" scripts/ci/doc-links.py
# The web backend and its host shim call each other across two name spaces; a mismatch is either
# a LinkError at instantiate or a silently swallowed TypeError in a DOM handler (a button that
# does nothing). Both are static facts, so they are checked here rather than in a browser.
leg "web shim ABI" scripts/ci/web-shim-abi.py
# How much of the built-in surface the `#[day::test]` cases prove (docs/testing.md). Report only
# while the coverage backlog is open; it gains `--check` once every family is covered.
leg "conformance coverage (kinds, pieces gated)" scripts/ci/conformance-coverage.py --require kinds,pieces

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
