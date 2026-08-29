#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# The host-portable test run: every workspace member's tests EXCEPT the toolkit backends, which
# need a platform SDK or a system toolkit the host may not have (and whose real coverage is the
# per-combo build+walkthrough jobs — even day-appkit is excluded on macOS so all three OS legs
# run the same set). This script is the one definition of that set; the day-cli-<os> native-arch
# legs and the windows-msys2 job all call it, so the tested roster cannot drift between them.
#
# Why not bare `cargo test`? A flagless cargo command builds only `default-members`, and that
# list is tuned as the QUICK-ITERATION set for editors and local checks — day-lite's oxc tree,
# the CLI, and the piece/part catalog are deliberately kept out of it. CI wants the opposite
# trade: every host-buildable test, once per OS. Until 2026-08 CI ran the bare form, which
# silently skipped more than half the workspace's tests (day-cli's and day-persistence's whole
# suites among them).
#
# A newly added toolkit crate missing from the exclude list fails this script loudly on the
# hosts lacking its SDK — add it below when adding the crate to [workspace] members.
set -euo pipefail
cd "$(dirname "$0")/../.."

# day-sqlite-worker's native engine build (the vendored SQLite + musl shim, docs/persistence.md)
# is a gcc/clang recipe with no MSVC port, and Windows has no native consumer — the engine's
# product form is wasm32 (exercised by the web-dom combo) and its native form exists for the
# unix test hosts. Skip it on Windows rather than carry a cl port nothing ships.
windows_excludes=""
case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*) windows_excludes="--exclude day-sqlite-worker" ;;
esac

# $windows_excludes is unquoted on purpose: empty means no extra words, non-empty splits into
# the two flag words (bash 3.2 on macOS mishandles empty arrays under `set -u`).
exec cargo test --locked --workspace \
    $windows_excludes \
    --exclude day-appkit \
    --exclude day-gtk \
    --exclude day-qt \
    --exclude day-qt-sys \
    --exclude day-uikit \
    --exclude day-android \
    --exclude day-xaml \
    --exclude day-xaml-sys \
    --exclude day-arkui \
    --exclude day-arkui-sys \
    --exclude day-dom \
    "$@"
