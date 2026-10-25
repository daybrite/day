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
# Persistence has its own CI. Cover the core model and the portable Android runtime helpers.
cargo clippy --locked -p day-model -p day-platform-android -p day-platform-windows --all-targets "$@"
# Validate both native/default routing and the opt-in HTTP/WebSocket provider.
cargo clippy --locked -p day-part-http --features reqwest --all-targets "$@"

# Retained optional pieces have conformance-only code; compile it even without a native SDK.
cargo clippy --locked -p day-piece-settings -p day-piece-text-editor \
    --features day-piece-settings/conformance,day-piece-text-editor/conformance --all-targets "$@"
# The standalone conformance app links the whole provider set through the public facade.
# Its scaffold is generated, so a fresh checkout must prepare it before linting the app.
bash apps/conformance/generate.sh
cargo clippy --locked --manifest-path apps/conformance/Cargo.toml --all-targets "$@"
