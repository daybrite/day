#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Required working-tree gate before reporting work ready to commit or push.
# Run applicable backend Clippy checks separately (or use the full lint.sh matrix).
set -euo pipefail
cd "$(dirname "$0")/../.."

bash scripts/ci/check-matrices.sh
python3 -B scripts/ci/check-platform-runtimes.py
python3 -B scripts/ci/test-readiness.py
cargo fmt --all -- --check
bash scripts/ci/host-clippy.sh
# The repositories that take crates from this one over git, resolved against HEAD as their CI
# will resolve it. A rename or removal here is not ready until each of them follows, and the
# local patch tables hide that; this is the check that does not (details in the script).
python3 -B scripts/ci/downstream-check.py
