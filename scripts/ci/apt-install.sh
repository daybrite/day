#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Install apt packages on a CI runner, surviving a mirror that is briefly down. The same script
# as daybrite/actions' setup-day-deps/apt-install.sh, for the jobs here that install by hand.
#
#     scripts/ci/apt-install.sh [apt-get install options…] <package>…
#
# Ubuntu's mirrors answer hosted runners with 503s and dropped connections in bursts that last a
# minute or two. apt's own `Acquire::Retries` asks again at once, so all five tries land inside
# the same burst. This runs the whole `update` + `install` up to four times with a growing pause
# between attempts. apt keeps every archive it already fetched, so an attempt downloads only
# what the last one missed.
set -uo pipefail

# Per-request retries and timeouts: without a timeout apt waits on a silent mirror until the
# job's own limit; with one it gives up on the request and asks again.
printf 'Acquire::Retries "5";\nAcquire::http::Timeout "30";\nAcquire::https::Timeout "30";\n' \
  | sudo tee /etc/apt/apt.conf.d/80-day-ci-timeouts >/dev/null

attempts=4
for attempt in $(seq 1 "$attempts"); do
  # A failed index refresh is not fatal: apt goes on with the indexes it has, and a package whose
  # version moved fails in `install`, which is what this loop retries.
  sudo apt-get update || echo "::warning::apt-get update failed on attempt $attempt; using the indexes at hand"
  if sudo apt-get install -y "$@"; then
    exit 0
  fi
  if [ "$attempt" = "$attempts" ]; then
    break
  fi
  pause=$((attempt * 30))
  echo "::warning::apt-get install failed (attempt $attempt of $attempts); trying again in ${pause}s"
  sleep "$pause"
done
echo "::error::apt-get install failed $attempts times: $*"
exit 100
