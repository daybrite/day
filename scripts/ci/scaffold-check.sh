#!/usr/bin/env bash
# Scaffold a project the way a new user's first command does, then lint it.
#
#     scripts/ci/scaffold-check.sh [path/to/day]     # default: target/release/day
#
# `day new app` is the CLI's largest single output and nothing else in CI runs it end to end: it
# writes Day.toml, the sample sources, a website/, and then per locale a resource/locales/<tag>/
# catalog, a store/<tag>/ listing, an Xcode knownRegions entry and a site.toml locales row. Those
# four surfaces have to agree, and `day lint` is what checks that they do — so a scaffold that
# lints clean is the cheapest proof the whole `day new` → `day localize add` path still works.
#
# The locale list is the one daysite's CI scaffolds with (daysite/.github/workflows/ci.yml), which
# is what the language picker there renders. `en` is the template's own default and is left out on
# purpose: passing en-US would stand up a second en-US/ tree beside en/.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$PWD"

DAY="${1:-$ROOT/target/release/day}"
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

# web-dom builds anywhere; ios-uikit is what gives the scaffold a store/ and an Xcode project, so
# all four locale surfaces exist to be checked. A store-less target set would exercise two.
"$DAY" new app ci-sample --toolkit web-dom,ios-uikit --no-input \
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

cd "$ROOT"
rm -rf "$WORK"
