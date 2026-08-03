#!/usr/bin/env bash
# Build and launch the Day showcase on one or more targets.
#
#     scripts/launch-showcase.sh macos-appkit
#     scripts/launch-showcase.sh desktop
#     scripts/launch-showcase.sh macos-appkit web-dom
#     scripts/launch-showcase.sh mobile --env DAY_DEMO_ROUTE=canvas
#
# Each argument is either a platform-toolkit name (`macos-appkit`, `windows-xaml`, …) or one of the
# symbolic groups:
#
#     desktop   every desktop target this host can build (macOS: appkit, gtk, qt)
#     mobile    every phone-class target this host can build (iOS, Android, HarmonyOS) — each needs
#               its simulator/emulator already running; `day launch` says which is missing
#     web       web-dom, served over loopback with a browser opened on it
#
# Anything starting with `-` is passed straight through to `day launch`, so `--env K=V`,
# `--locale`, `--script`, and `--detach` all work. `--dry-run` prints the targets an argument list
# expands to and stops, without building anything.
#
# Symlink it anywhere and call it from anywhere — `ln -s .../day/scripts/launch-showcase.sh
# ~/bin/showcase`. The script resolves its own path through the link chain, so it always finds the
# checkout it actually lives in, whatever the caller's working directory is.
#
# The app is built in RELEASE: a debug build of a UI framework indicates nothing about what a user
# would experience, and the showcase exists to be looked at. The `day` CLI itself is built from this
# checkout in debug — it is the build tool, not the thing being measured.
#
# Written for the bash macOS ships (3.2): no `mapfile`, and array expansions are guarded.
set -euo pipefail

step() { printf '\033[1m▶ %s\033[0m\n' "$*"; }
die()  { printf '\033[31merror: %s\033[0m\n' "$*" >&2; exit 1; }

# Resolve this file through any symlinks, so the checkout is found relative to the SCRIPT rather
# than to wherever it was linked from — `ln -s .../day/scripts/launch-showcase.sh ~/bin/showcase`
# has to keep working. Done by hand rather than with `readlink -f` or `realpath`, neither of which
# is portable to the BSD userland macOS ships. Each hop resolves a relative target against the
# directory of the link that named it, which is what the kernel does.
SELF="${BASH_SOURCE[0]}"
hops=0
while [ -L "$SELF" ]; do
  hops=$(( hops + 1 ))
  # A symlink loop is otherwise an infinite one; 40 is the kernel's own ELOOP ceiling.
  [ "$hops" -le 40 ] || die "symlink loop resolving ${BASH_SOURCE[0]}"
  link="$(readlink "$SELF")"
  case "$link" in
    /*) SELF="$link" ;;
    *)  SELF="$(cd "$(dirname "$SELF")" && pwd)/$link" ;;
  esac
done
SCRIPT_DIR="$(cd "$(dirname "$SELF")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SHOWCASE="$ROOT/apps/showcase"

# A link left behind after the checkout moved would otherwise fail deep inside cargo with nothing
# pointing back at the real cause.
[ -f "$ROOT/Cargo.toml" ] && [ -d "$SHOWCASE" ] \
  || die "$ROOT is not a day checkout (resolved from $SELF) — the script must live in <day>/scripts/"

# The header comment IS the help text: print the block after the shebang, up to the first line that
# is not a comment. Deriving the range rather than hardcoding one means editing the header can never
# silently truncate `--help` mid-sentence.
usage() { awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$SELF"; }

TARGET_ARGS=()
PASSTHROUGH=()
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    -h|--help) usage; exit 0 ;;
    -n|--dry-run) DRY_RUN=1 ;;
    -*) PASSTHROUGH+=("$arg") ;;
    *) TARGET_ARGS+=("$arg") ;;
  esac
done
[ ${#TARGET_ARGS[@]} -gt 0 ] || { usage; exit 2; }

# --- the CLI from this checkout ----------------------------------------------------------------
# Even a dry run builds it: the target catalog it prints is the whole point of resolving here.
step "Building the day CLI"
( cd "$ROOT" && cargo build -p day-cli >/dev/null ) || die "failed to build day-cli in $ROOT"
DAY="$ROOT/target/debug/day"

# --- expand the arguments into target names ------------------------------------------------------
# Resolved from `day metadata --json` rather than a list kept here: the CLI's target catalog carries
# each target's kind and the host OS that can build it, so a target added to day is picked up by the
# groups automatically and a typo is checked against the real catalog.
IFS='' read -r -d '' RESOLVE <<'PY' || true
import json, sys

meta_path, args = sys.argv[1], sys.argv[2:]
with open(meta_path) as f:
    data = json.load(f)

host = data["host"]["os"]
catalog = data["targetCatalog"]
buildable = [t for t in catalog if t["host"] in (host, "any")]

GROUPS = {
    "desktop": lambda t: t["kind"] == "desktop",
    "mobile": lambda t: t["kind"] in ("iosSim", "android", "harmonyOs"),
    "web": lambda t: t["kind"] == "web",
}

by_name = {t["name"]: t for t in catalog}
out, seen = [], set()
for arg in args:
    if arg in GROUPS:
        names = [t["name"] for t in buildable if GROUPS[arg](t)]
        if not names:
            sys.exit("no %s target can be built on %s" % (arg, host))
    elif arg in by_name:
        target = by_name[arg]
        if target["host"] not in (host, "any"):
            sys.exit("%s needs a %s host; this is %s" % (arg, target["host"], host))
        names = [arg]
    else:
        sys.exit(
            "unknown target or group: %s\n  groups:  %s\n  targets: %s"
            % (arg, " ".join(GROUPS), " ".join(sorted(by_name)))
        )
    for name in names:
        if name not in seen:
            seen.add(name)
            out.append(name)
print("\n".join(out))
PY

META="$(mktemp -t day-showcase-meta)"
trap 'rm -f "$META"' EXIT
"$DAY" --project "$SHOWCASE" metadata --json > "$META" || die "could not read the target catalog"

RESOLVED="$(python3 -c "$RESOLVE" "$META" "${TARGET_ARGS[@]}")" || exit 1

TARGETS=()
while IFS= read -r line; do
  [ -n "$line" ] && TARGETS+=("$line")
done <<< "$RESOLVED"
[ ${#TARGETS[@]} -gt 0 ] || die "no targets to launch"

if [ "$DRY_RUN" = 1 ]; then
  printf '%s\n' "${TARGETS[@]}"
  exit 0
fi

PLATFORM_FLAGS=()
for t in "${TARGETS[@]}"; do PLATFORM_FLAGS+=(-p "$t"); done

# --- launch ---------------------------------------------------------------------------------------
step "Launching showcase (release): ${TARGETS[*]}"
exec "$DAY" --project "$SHOWCASE" launch "${PLATFORM_FLAGS[@]}" \
  --profile release ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}
