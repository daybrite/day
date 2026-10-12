<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Native project migration audit — October 2026

This historical record preserves the original local migration audit. It is not a release
compatibility guarantee or a current validation report. Use the maintained
[migration procedure](../native-project-migration.md) for new migrations.

27 Day app checkouts under `daybrite/` used generated native build projects in this audit.
The scope included active apps, demos, archived apps, conformance, and the daysite showcase.
Generator templates, disposable `day-launch-video/codex/dry-run-*` recording fixtures, and
Day-Bench's separate non-Day SwiftUI benchmarks were excluded.

Their `.xcodeproj` documents/schemes, Gradle scaffolds, and hvigor scaffolds were removed
from `platform/`. App-owned Apple Swift sources, plists, string catalogs, `.xcconfig`
files, and CLI wrappers remained as inputs. Custom Android themes remained as resource overlays.

## Migrated apps and preserved configuration

| Apps | Customization |
| --- | --- |
| Day-Rise, Day-Bench, Day-Orbit, Day-Yaak/apps/yaak-day, Day-Hub, Day-Brew, Day-Lexicon, Day-Sketch | Standard hosts; Apple source/configuration retained |
| Day-Showcase; daysite/samples/.showcase/app | Exact loopback HTTP exception, blue light/dark palettes, theme items, HarmonyOS vendor `daybrite` |
| Day-Tunes; Day-News | HTTP allowed for arbitrary radio-stream / feed URLs |
| Day-Trader | Charcoal palette, green accent, custom status-bar color |
| Day-Skies | Blue light/dark palettes, custom toolbar style, edge-to-edge opt-in |
| day-piece-{selfupdate,charts,camera,webview,lottie,media}/demo | Standard hosts; native piece contributions still resolved from Cargo metadata |
| day-part-speech/demo; day-godot/demo; day/apps/conformance | Standard hosts |
| old/Day-Viz; old/Day-Rise-custom; old/Day-Time; old/Day-Games | Standard hosts; application/flavor configuration retained |

Source permissions, HarmonyOS shortcut profiles, and localized permission resources were
derived duplicates of Day.toml and Fluent catalogs and regenerated in the build tree.
No custom Xcode target, build phase, package, launch argument, or environment setting was
found in the migrated project documents/schemes. Older framework defaults and comments
were supplied by the generator.

## Manifest and generator changes used

- `[android] cleartext-traffic = true`: HTTP from arbitrary hosts, where explicitly required.
- `[android] cleartext-domains = ["127.0.0.1"]`: exact-host exceptions; the base policy defaults to disallowing cleartext.
- `[android] edge-to-edge = true`: Day's immersive Android chrome opt-in.
- `[android] resource-overlays = ["platform/android/res"]`: native theme/palette XML replaces generated defaults. Deleted files are pruned during regeneration.
- `[harmony] vendor = "daybrite"`: publisher metadata in the staged AppScope manifest.

The policy switches were moved into Day.toml. Arbitrary Android styles stayed native XML.
Android options configured generated hosts; persistent Gradle projects retained their own
settings. HarmonyOS vendor also applied to staged persistent hosts.

The Xcode generator omitted the optional `InfoPlist.xcstrings` reference when an older app
had no catalog. Preparation created that catalog when localization required it.

## Retained exceptions

### App-Fair

Both native projects remained. It had no `resource/icons/` master: iOS referenced an app-owned
`Assets.xcassets`, while Android shipped custom adaptive-icon foreground/background raster
sets. The native files also carried the blue palette and `REQUEST_INSTALL_PACKAGES` /
`QUERY_ALL_PACKAGES`. Android used the older inlined Gradle implementation; the iOS project
set `PRODUCT_NAME = AppFair`. Existing uncommitted edits were preserved.

The proposed follow-up was to reconcile `resource/icon-src/` with Day's layered master while
preserving the shipped adaptive artwork, then:

1. Move installation/query permissions to existing `[permissions.raw] android = [...]`.
2. Preserve the theme with the Android resource overlay option.
3. Move `PRODUCT_NAME = AppFair` into `DayApp.xcconfig` and add the standard Apple CLI wrapper.
4. Replace the native projects and verify the packaged icons and installer.

The blocker was preserving the older icon pipeline; no new permission mechanism was needed.

### old/Day-Matrix

The legacy iOS host used `OTHER_LDFLAGS = -lmatrix` instead of Day's staged-library contract,
lacked the modern icon/resource setup, and had no icon master. Android enabled unrestricted
HTTP for a development homeserver. Both hosts remained intact.

The proposed follow-up was to move HTTP policy into `[android]`, update the legacy
Rust-library/Apple-host contract, and establish an icon source before deriving the projects.
Apple custom flags already belonged in `.xcconfig`; no generic Day.toml linker field was needed.

## Validation recorded during the audit

All 27 migrated apps passed native preparation, read-only `day prepare --check`, and
`day lint` with the rebuilt local CLI. Day-Rise built through generated macOS and iOS
simulator Xcode projects and passed an ARM64 HarmonyOS Rust compile check. Day-Showcase
built an Android APK from its generated Gradle project. Inspection of its packaged network
policy confirmed a false base cleartext setting and an exact `127.0.0.1` exception without
subdomains. Theme outputs matched source overlays byte for byte; HarmonyOS kept vendor
`daybrite`.

Generator, manifest, Xcode, and HarmonyOS staging tests passed, including overlay removal,
read-only drift checks, and missing optional Xcode catalogs. The repository readiness gate
(matrices, formatting, host Clippy) passed. Full builds and UI walkthroughs for every
app/platform remained CI work. No HarmonyOS emulator or acceptance test ran locally.

## Local recovery record

Removed folders, including local IDE state and uncommitted scaffold edits, were moved intact
to each app's ignored `build/day/native-project-backup-20261011/`, alongside its original
Day.toml. These were local recovery files; **`day clean` removes them**. Their current existence
has not been reverified. Future migrations should keep recoverable backups outside build output.

Four apps had stale machine-local Cargo patches to the removed `day-arkui-sys` crate:
archived Rise-custom, Time, Games, and the Lottie demo (including its parent config). Those
obsolete entries were removed for validation, with the original configs backed up there too.

The audit used a rebuilt Day CLI. Apple builds required Xcode 27+. `day open -t <target>`
resolved the generated project; restoring a native project under `platform/` restored persistent
configuration. These local results did not establish that released CLIs supported the migration.
