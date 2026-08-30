#!/bin/sh
# Runs the day CLI with the arguments given, wherever it is installed.
# The Xcode script phases call this rather than `day` directly: `day build` exports DAY_BIN, but
# a build started from the Xcode GUI runs on Xcode's own minimal PATH, with no shell profile and
# so no ~/.cargo/bin, and has to find the CLI itself. https://daybrite.dev/docs/cli

if [ -z "${DAY_BIN:-}" ]; then
    DAY_BIN="$(command -v day || true)"
fi
if [ -z "$DAY_BIN" ]; then
    for candidate in "$HOME/.cargo/bin/day" /opt/homebrew/bin/day /usr/local/bin/day; do
        if [ -x "$candidate" ]; then
            DAY_BIN="$candidate"
            break
        fi
    done
fi
if [ -z "$DAY_BIN" ]; then
    echo "error: the day CLI was not found. Install it with 'cargo install day-cli', or set DAY_BIN in the scheme's run-script environment." >&2
    exit 1
fi

exec "$DAY_BIN" "$@"
