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
# Persistence has its own repository and CI; keep the core observable model covered here.
cargo clippy --locked -p day-model --all-targets "$@"
# Validate both native/default routing and the opt-in HTTP/WebSocket provider.
cargo clippy --locked -p day-part-http --features reqwest --all-targets "$@"
