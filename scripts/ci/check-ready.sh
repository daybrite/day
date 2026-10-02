#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Required working-tree gate before reporting work ready to commit or push.
# Run applicable backend Clippy checks separately (or use the full lint.sh matrix).
set -euo pipefail
cd "$(dirname "$0")/../.."

bash scripts/ci/check-matrices.sh
cargo fmt --all -- --check
bash scripts/ci/host-clippy.sh
