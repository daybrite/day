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

# Every platform, so the scaffold materializes every host project (Xcode, gradle, ohos, …) and
# any of them can be the pack target below. --local points the day deps at THIS checkout: the
# pack stage builds the app, and it has to build the framework under test, not git main.
"$DAY" new app ci-sample --no-input --local "$ROOT" \
    --toolkit macos-appkit,ios-uikit,android-mdc,linux-gtk,linux-qt,windows-xaml,harmony-arkui,web-dom \
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
grep -q '^home_greeting = ' resource/locales/ja-JP/app.ftl

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
            (cd "$REBUILT" && "$DAY" launch -p "$COMBO" --script dayscript/smoke.yaml)
            SHOT="$(find "$REBUILT/build/day/screenshots/$COMBO" -name smoke.png 2>/dev/null | head -1 || true)"
            [ -n "$SHOT" ] || {
                echo "the smoke dayscript left no build/day/screenshots/$COMBO/*/smoke.png under $REBUILT" >&2
                exit 1
            }
            echo "smoke screenshot: $SHOT"
            # Scripted launches can leave the app running; drop the session before cleanup.
            (cd "$REBUILT" && "$DAY" stop --all) || true
            ;;
        *)
            echo "skipping the smoke launch: $COMBO needs a device, emulator, or browser driver this check does not manage"
            ;;
    esac
    rm -rf "$TMP"/day-rebuild-*
fi

cd "$ROOT"
rm -rf "$WORK"
