#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Lint one toolkit backend's own crates, in this workspace, under the workspace lint policy.
#
#     scripts/ci/toolkit-clippy.sh <appkit|uikit|gtk|qt|material|arkui|dom|winui> [cargo args…]
#
# The showcase legs (`cargo clippy --features <toolkit>` in the app's checkout) compile the
# backend crate but do not lint it: Clippy checks workspace members only, and there the backend
# is a path dependency of another workspace. So each backend crate, its platform crate, the
# facade under that backend's feature and the tweaks are linted here by name, for the target the
# backend builds for. Shared by scripts/ci/lint.sh and .github/workflows/ci.yml.
#
# The caller supplies the cross environment where one is needed (cargo-ndk's for `material`;
# `arkui` runs inside cargo-ohos's), exactly as for the showcase legs.
set -euo pipefail
cd "$(dirname "$0")/../.."

TOOLKIT="${1:?usage: toolkit-clippy.sh <toolkit> [cargo args…]}"
shift

# Match CI even when invoked from a shell without its warning policy.
case " ${RUSTFLAGS:-} " in
    *" -D warnings "*) ;;
    *) export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings" ;;
esac

TWEAKS=(-p day-tweak-button-bezel -p day-tweak-slider-tick-marks -p day-tweak-tooltip -p day-tweak-tree-style)
# clippy 1.97 false-positives missing_const_for_thread_local on the bionic/musl cross-targets,
# flagging thread_local!s that already use `const { … }`. The host legs enforce the lint.
XCROSS=(-A clippy::missing_const_for_thread_local)

case "$TOOLKIT" in
    appkit)
        cargo clippy --locked -p day-toolkit-appkit -p day-platform-macos -p day-piece-swiftui \
            -p day-app "${TWEAKS[@]}" --features day-app/appkit --all-targets "$@"
        ;;
    uikit)
        cargo clippy --locked --target aarch64-apple-ios-sim -p day-toolkit-uikit \
            -p day-piece-swiftui -p day-app "${TWEAKS[@]}" --features day-app/uikit --lib "$@"
        ;;
    gtk)
        cargo clippy --locked -p day-toolkit-gtk -p day-platform-linux -p day-app "${TWEAKS[@]}" \
            --features day-app/gtk --all-targets "$@"
        ;;
    qt)
        cargo clippy --locked -p day-toolkit-qt -p day-toolkit-qt-sys -p day-platform-linux \
            -p day-app "${TWEAKS[@]}" --features day-app/qt --all-targets "$@"
        ;;
    material)
        cargo clippy --locked --target aarch64-linux-android -p day-toolkit-material \
            -p day-platform-android -p day-app "${TWEAKS[@]}" --features day-app/material --lib \
            "$@" -- "${XCROSS[@]}"
        ;;
    arkui)
        cargo ohos clippy --locked --target aarch64-unknown-linux-ohos -p day-toolkit-arkui \
            -p day-app "${TWEAKS[@]}" --features day-app/arkui --lib "$@" -- "${XCROSS[@]}"
        ;;
    dom)
        # The getrandom_backend cfg mirrors what `day build` sets for every web app
        # (crates/day-cli/web.rs): day-toolkit-dom carries getrandom's custom-backend hook.
        RUSTFLAGS="$RUSTFLAGS --cfg getrandom_backend=\"custom\"" \
            cargo clippy --locked --target wasm32-unknown-unknown -p day-toolkit-dom -p day-app \
            "${TWEAKS[@]}" --features day-app/dom --lib "$@"
        ;;
    winui)
        cargo clippy --locked -p day-toolkit-winui -p day-toolkit-xaml-sys -p day-platform-windows \
            -p day-app "${TWEAKS[@]}" --features day-app/winui --all-targets "$@"
        ;;
    *)
        echo "toolkit-clippy.sh: unknown toolkit '$TOOLKIT'" >&2
        exit 2
        ;;
esac
