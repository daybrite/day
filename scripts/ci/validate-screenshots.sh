#!/usr/bin/env bash
# validate-screenshots.sh <screenshots-root> — content-validate walkthrough screenshots (§20).
#
# A capture that decodes but is blank (transparent snapshot, unpainted window) compresses to a
# handful of distinct colors; a real day window has hundreds. Every PNG under the root must
# contain at least MIN_COLORS (default 64) distinct colors, and at least one PNG must exist.
# Emits a per-shot gallery table to $GITHUB_STEP_SUMMARY when running under Actions.
set -euo pipefail

root="${1:?usage: validate-screenshots.sh <screenshots-root>}"
min="${MIN_COLORS:-64}"
here="$(cd "$(dirname "$0")" && pwd)"

fail=0
found=0
rows=""

while IFS= read -r png; do
    found=1
    if [[ "$(uname)" == "Darwin" ]]; then
        stats="$(swift "$here/imgstat.swift" "$png")"
    else
        stats="$(identify -format '%w %h %k' "$png")"
    fi
    read -r w h k <<<"$stats"
    status="ok"
    if ((k < min)); then
        status="BLANK (<$min colors)"
        fail=1
    fi
    size="$(wc -c <"$png" | tr -d ' ')"
    rel="${png#"$root"/}"
    echo "$rel: ${w}x${h} colors=$k bytes=$size $status"
    rows+="| $rel | ${w}×${h} | $k | $size | $status |"$'\n'
done < <(find "$root" -name '*.png' | sort)

if ((!found)); then
    echo "error: no screenshots found under $root" >&2
    fail=1
fi

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    {
        echo "### Walkthrough screenshots"
        echo ""
        echo "| shot | dims | distinct colors | bytes | status |"
        echo "|---|---|---|---|---|"
        printf '%s' "$rows"
        echo ""
    } >>"$GITHUB_STEP_SUMMARY"
fi

exit "$fail"
