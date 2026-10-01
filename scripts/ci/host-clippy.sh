#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Shared local/CI lint gate. Keep backend feature combinations in their per-platform jobs.
set -euo pipefail
cd "$(dirname "$0")/../.."

# Match CI even when invoked from a shell without its warning policy.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
cargo clippy --locked --all-targets "$@"
# No workspace member enables dyn-registry; its consumer (day-lite) is a separate repo.
cargo clippy --locked -p day-cli -p day-script -p day-pieces \
    --features day-pieces/dyn-registry --all-targets "$@"
# Persistence and model are outside default-members. Include the live-list adapter and
# worker regression tests, which an app dependency's backend lint does not compile.
cargo clippy --locked -p day-model -p day-persistence \
    --features day-persistence/pieces --all-targets "$@"
