#!/usr/bin/env bash
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
# Scaffold a project the way a new user's first command does, then check it end to end.
#
#     scripts/ci/scaffold-check.sh [path/to/day] [<platform-toolkit>]
#
# `day new app` is the CLI's largest single output and nothing else in CI runs it end to end: it
# writes Day.toml, the sample sources, a website/, and then per locale a resource/locales/<tag>/
# catalog, a store/<tag>/ listing, an Xcode knownRegions entry and a site.toml locales row. Those
# four surfaces have to agree, and `day lint` is what checks that they do — so a scaffold that
# lints clean is the cheapest proof the whole `day new` → `day localize add` path still works.
#
# With a combo argument the check keeps going: `day pack` builds the scaffold into a release
# artifact, `day rebuild --from-dir` packs the same tree again from a scratch copy and compares
# the two (§20.3 — the scaffold is not in git, which is exactly what --from-dir is for), and on a
# desktop combo the smoke dayscript then drives the REBUILT copy and must leave its screenshot
# behind. Without a combo the check stops after the lint, as it always did.
#
# The locale list is the one daysite's CI scaffolds with (daysite/.github/workflows/ci.yml), which
# is what the language picker there renders. `en` is the template's own default and is left out on
# purpose: passing en-US would stand up a second en-US/ tree beside en/.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$PWD"

DAY="${1:-$ROOT/target/release/day}"
COMBO="${2:-}"
[ -x "$DAY" ] || DAY="$DAY.exe" # windows-msvc
[ -x "$DAY" ] || {
    echo "no day binary at ${1:-$ROOT/target/release/day}" >&2
    exit 1
}
# Absolutize AFTER the existence check: the scaffold happens in a scratch dir outside the
# checkout, where a relative target/<triple>/release/day silently stops resolving — bash then
# fails with 127 on Linux and "No such file or directory" (exit 1) on macOS's bash 3.2, which is
# exactly how this bug shipped twice-disguised.
DAY="$(cd "$(dirname "$DAY")" && pwd)/$(basename "$DAY")"

# Outside the checkout: a Day.toml inside it would sit in the cargo workspace, and
# scripts/ci/assert-pristine.sh would see the tree as dirty.
WORK="$ROOT/../day-scaffold-check"
rm -rf "$WORK"
mkdir -p "$WORK"
cd "$WORK"

# No --toolkit: `day new --no-input` already defaults to every target pair, which is what this
# check wants — the scaffold materializes every host project (Xcode, gradle, ohos, …) and any of
# them can be the pack target below. Naming them here instead pinned the list at the eight that
# existed when it was written, so the three added since went unexercised and the default itself
# was never the thing under test. --local points the day deps at THIS checkout: the pack stage
# builds the app, and it has to build the framework under test, not git main.
"$DAY" new app ci-sample --no-input --local "$ROOT" \
    --locales zh-Hans-CN,es-ES,pt-BR,fr-FR,de-DE,ja-JP,ko-KR,it-IT,ru-RU,ar-SA \
    --locales id-ID,tr-TR,vi-VN,th-TH,pl-PL,nl-NL,zh-Hant-TW,uk-UA,cs-CZ,ms-MY
cd ci-sample

# 20 requested + the template's own en. Counted rather than trusted: `day new` reports each
# `localize add` as it goes, and a tag that silently no-ops would still print a green summary.
n=$(find resource/locales -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
[ "$n" = 21 ] || {
    echo "expected 21 locale catalogs, found $n" >&2
    exit 1
}
# A STARTER key, and translated rather than copied: `day localize add` writes the table in
# starter_l10n.rs for the handful of scaffold strings the CLI knows the meaning of, and everything
# else lands as an English copy under a translate-me header. Naming the key explicitly is what
# keeps this honest — the previous one (`home_greeting`) belonged to a template two rewrites ago,
# and a bare `grep -q` under `set -e` fails with NO output at all, which is how it read as a
# mysterious exit 1 rather than "that key is gone".
grep -q '^nav_welcome = ' resource/locales/ja-JP/app.ftl || {
    echo "resource/locales/ja-JP/app.ftl has no nav_welcome — is it still a scaffold key?" >&2
    echo "  keys the CLI translates: crates/day-cli/src/starter_l10n.rs KEYS" >&2
    exit 1
}

# The scaffold's own Rust must be rustfmt-clean. A user's first `cargo fmt` should be a no-op,
# and for the Day-Rise reference it is stronger than that: its drift check diffs the checkout
# against fresh `day new` output, so a template that formats differently from rustfmt reports as
# drift the moment anyone formats the generated tree.
cargo fmt --all -- --check || {
    echo "the scaffold's Rust is not rustfmt-clean — format the TEMPLATE:" >&2
    echo "  cd day/crates/day-cli/templates/app && rustfmt --edition 2024 src/**/*.rs" >&2
    echo "  (src/main.rs holds {{ident}} in identifier position: substitute, format, restore)" >&2
    exit 1
}

# …and clippy-clean, on the same reasoning: a user's first `cargo clippy` should be quiet, and
# every app scaffolded from this template runs nearly this command in its own CI — the shared
# `build-day-app` preflight is `cargo clippy --workspace --all-targets -- -D warnings`. Without
# this gate the template's lints are found by the GENERATED repositories rather than here, which
# is how `tr(*k)` (an explicit deref clippy does for you) reached Day-Rise's preflight.
#
# `--features mock` where that preflight passes none, because it runs on Linux and this runs
# wherever the combo does: the scaffold declares no default feature, so with none the `day`
# facade compiles without a backend — fine on Linux, where `macos_main!` expands to nothing, and
# a `cannot find function launch` error on macOS, where it does not. `mock` gives it a backend on
# every host. The app's own code is backend-independent, so the lints are the same either way.
#
# It compiles the framework, which the pack stage below then reuses from the same target dir, so
# the real added cost is the lint pass rather than a second build.
cargo clippy --workspace --all-targets --features mock -- -D warnings || {
    echo "the scaffold's Rust is not clippy-clean — fix the TEMPLATE:" >&2
    echo "  day/crates/day-cli/templates/app/src/…" >&2
    echo "  (every app `day new` emits runs this exact command in its own preflight)" >&2
    exit 1
}

# store-placeholder is the one finding a fresh scaffold is meant to have: the listing text stays a
# TODO until a human writes it, and the lint exists to stop that text reaching a store. Every other
# code has to be clear, and --strict turns any of them into a failure.
"$DAY" lint --strict --allow store-placeholder

if [ -n "$COMBO" ]; then
    case "$COMBO" in
        # The combos `day pack` supports (pack/mod.rs) — only these can go on to the rebuild.
        macos-appkit | ios-uikit | android-mdc | linux-gtk | linux-qt | windows-xaml | harmony-arkui) ;;
        *)
            echo "day pack does not support $COMBO yet — stopping after the lint"
            cd "$ROOT"
            rm -rf "$WORK"
            exit 0
            ;;
    esac

    "$DAY" pack -p "$COMBO" --profile release --no-version-in-name

    # The installable containers, not the SBOM/buildinfo sidecars beside them (which are named
    # `<artifact>.sbom-cdx.json` / `.buildinfo.json` / `.buildinfo.deb822`, so matching the
    # container's extension is what separates them). One per target, except Linux: a .flatpak and
    # a .appimage are separate downloads that fail in different places, so both get rebuilt.
    # android's .aab and windows' -setup.exe are deliberately absent — they come from the same
    # payload as the .apk / .msix beside them, and `day rebuild` extracts those.
    case "$COMBO" in
        macos-appkit) EXTS="dmg" ;;
        ios-uikit) EXTS="ipa" ;;
        android-*) EXTS="apk" ;;
        linux-*) EXTS="flatpak appimage" ;;
        windows-*) EXTS="msix" ;;
        harmony-*) EXTS="hap" ;;
        *)
            echo "no container extension known for $COMBO" >&2
            exit 1
            ;;
    esac

    # `day rebuild` scratches in ${TMPDIR:-/tmp}/day-rebuild-<artifact stem>; clear leftovers so
    # the find below cannot pick up a copy some earlier run kept. Once, before the loop — the
    # stems differ per artifact, so the runs do not collide with each other.
    TMP="${TMPDIR:-/tmp}"
    rm -rf "$TMP"/day-rebuild-*
    for EXT in $EXTS; do
        ART="$(find build/day/dist -maxdepth 1 -type f -name "*.$EXT" | head -1)"
        [ -n "$ART" ] || {
            echo "day pack left no .$EXT in build/day/dist" >&2
            ls -la build/day/dist >&2 || true
            exit 1
        }
        echo "== rebuilding $(basename "$ART")"
        "$DAY" rebuild --from-dir "$PWD" --keep --strict "$ART"
    done

    case "$COMBO" in
        macos-appkit | macos-gtk | macos-qt | linux-gtk | linux-qt | windows-xaml | windows-gtk | windows-qt)
            # Desktop: run the smoke dayscript against the REBUILT copy (--keep left it in the
            # scratch), so the thing that gets driven is the tree the rebuild actually packed.
            # find is scoped to the scratch dirs (never the whole temp dir: macOS's is full of
            # unreadable app dirs, and under pipefail find's exit 1 would kill the script even
            # after a successful match); `|| true` keeps a no-match answered by the check below.
            TOML="$(find "$TMP"/day-rebuild-* -maxdepth 3 -path '*/src/*/Day.toml' 2>/dev/null | head -1 || true)"
            [ -n "$TOML" ] || {
                echo "no kept rebuild under $TMP/day-rebuild-*/src — did rebuild --keep run?" >&2
                exit 1
            }
            REBUILT="$(dirname "$TOML")"
            (cd "$REBUILT" && "$DAY" launch -p "$COMBO" --script dayscript/demo.yaml)
            SHOT="$(find "$REBUILT/build/day/screenshots/$COMBO" -name welcome.png 2>/dev/null | head -1 || true)"
            [ -n "$SHOT" ] || {
                echo "the demo dayscript left no build/day/screenshots/$COMBO/*/welcome.png under $REBUILT" >&2
                exit 1
            }
            echo "demo screenshot: $SHOT"
            # Scripted launches can leave the app running; drop the session before cleanup.
            (cd "$REBUILT" && "$DAY" stop --all) || true
            ;;
        *)
            echo "skipping the demo walkthrough: $COMBO needs a device, emulator, or browser driver this check does not manage"
            ;;
    esac
    rm -rf "$TMP"/day-rebuild-*
fi

cd "$ROOT"
rm -rf "$WORK"
