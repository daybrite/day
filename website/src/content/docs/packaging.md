---
title: Packaging & distribution
description: day pack — building signed, standalone, installable packages for every platform, and the signing configuration in Day.toml.
order: 32
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

`day pack -p <target>` builds the app in release mode, signs it, and produces a **standalone
installable artifact** in `build/day/dist/`, with a SHA-256 checksum and a signing tier in the
result output. One command per platform, the platform's own signing tools underneath. Day
orchestrates `codesign`/`notarytool`, `xcodebuild -exportArchive`, Gradle signing,
`flatpak-builder`, `linuxdeploy`, `makeappx`/`signtool`/`makensis`, and `hap-sign-tool`; it never reimplements
them.

## Artifacts per target

| target | artifact | notes |
|---|---|---|
| `macos-appkit` | `.dmg` | assembled `.app` (Info.plist, icons, assets) → inside-out `codesign --timestamp -o runtime` → UDZO dmg (with an `/Applications` drop link) → dmg signature → `notarytool submit --wait` → `stapler staple` |
| `ios-uikit` | `.ipa` | `xcodebuild archive` (device, arm64) → `-exportArchive` with a generated `ExportOptions.plist` (`app-store-connect`); without signing config: an unsigned device build, packaged as `<stem>-ios-uikit-unsigned.ipa` for sideloading or your own signing |
| `android-mdc` | `.apk` + `.aab` | Gradle `assembleRelease` + `bundleRelease` with a release `signingConfig`; verified with `apksigner` and checked for 16 KB page alignment |
| `linux-gtk` / `linux-qt` | `.flatpak` | single-file bundle; the runtime supplies the toolkit (GTK 4 ⇒ `org.gnome.Platform`, Qt 6 ⇒ `org.kde.Platform`) and resolves from Flathub at install time — `flatpak install ./my-app-1.0-linux-gtk-x86_64.flatpak` needs no other setup (the target combo is part of the name, so the gtk and qt bundles coexist). A Qt app that links QtWebEngine also carries the Qt WebEngine BaseApp, since no runtime ships it — that is ~87 MB of Chromium, so `day pack` adds it only when the binary actually links it |
| `linux-gtk` / `linux-qt` | `.appimage` | one executable that carries its own GTK/Qt — `chmod +x` and run, with no package manager, no runtime download and no root. `day pack` stages the AppDir and hands the bundling to [`linuxdeploy`](https://github.com/linuxdeploy/linuxdeploy) plus its `gtk`/`qt` plugin, which is what picks up the pieces an `ldd` closure misses (GdkPixbuf loaders, GIO modules, GSettings schemas, Qt platform plugins). Without the plugin the image still builds and still runs, on a machine that already has the toolkit — `day pack` says so loudly rather than letting a user discover it |
| `windows-xaml` | `.msix` + `-setup.exe` | `makeappx` + `signtool` for the MSIX; an NSIS per-user installer (no elevation, ARP entry, silent `/S`) for classic direct download |
| `harmony-arkui` | `.hap` | hvigor release build, signed with your release material via `hap-sign-tool` (or the public dev certificate without it) |
| `web-dom` | — | there is no pack step: `day build -p web-dom` writes a self-contained `dist/` (host page, shim, stylesheet, `.wasm`, images, fonts) that you deploy to any static host as-is |

`--formats` narrows the set (`day pack -p android-mdc --formats apk`); `--no-sign` and
`--no-notarize` skip stages; `--no-wait` submits notarization asynchronously (poll with
`day sign --notarize-status <id>`). Artifact names follow one pattern,
`<stem>[-<version>]-<target>[-<extra>].<ext>` (`day-showcase-0.1.0-macos-appkit.dmg`):
`--artifact-name <stem>` overrides the stem (always slugged; `[app] artifact` in Day.toml sets it
per project), and `--no-version-in-name` drops the version so a
`releases/latest/download/<name>` URL stays stable.

## Signing configuration

Signing lives in `Day.toml` under `[signing]`, with every secret referenced as `${ENV_VAR}`;
values resolve from the environment at pack time and are **never** stored in the manifest or
printed by the tool:

```toml
[signing.macos]
identity = "${DAY_SIGN_MACOS_IDENTITY}"   # "Developer ID Application: …"

[signing.macos.notarize]
key-id = "${DAY_NOTARY_KEY_ID}"           # App Store Connect API key
issuer = "${DAY_NOTARY_ISSUER}"
key-path = "${DAY_NOTARY_KEY}"

[signing.ios]
team = "${DAY_APPLE_TEAM}"
key-id = "${DAY_ASC_KEY_ID}"              # ASC key for -allowProvisioningUpdates in CI
issuer = "${DAY_ASC_ISSUER}"
key-path = "${DAY_ASC_KEY}"

[signing.android]
keystore = "${DAY_ANDROID_KEYSTORE}"
key-alias = "${DAY_ANDROID_KEY_ALIAS}"
store-pass = "${DAY_KS_PASS}"
key-pass = "${DAY_KEY_PASS}"

[signing.windows]
provider = "self-signed-dev"              # or signtool-cert-store | azure-artifact-signing

[signing.ohos]
keystore = "${DAY_OHOS_KEYSTORE}"
key-alias = "${DAY_OHOS_KEY_ALIAS}"
store-pass = "${DAY_OHOS_KS_PASS}"
key-pass = "${DAY_OHOS_KEY_PASS}"
cert = "${DAY_OHOS_CERT}"
profile = "${DAY_OHOS_PROFILE}"
```

`day sign --check` reports each platform's readiness (env vars set, key files present) without
echoing a single secret value.

## Signing tiers

Every artifact carries a tier: **release**, **dev-signed**, or **unsigned**. When a `${VAR}` is
unset (a laptop without the release keys, a fork PR without repository secrets), `day pack`
**warns naming the variable and drops that platform to the dev tier** (ad-hoc codesign on macOS,
the fixed dev keystore embedded in the CLI on Android, a self-signed certificate on Windows, the
unsigned device `.ipa` on iOS) instead of failing. The Android dev keystore is deliberately fixed
rather than generated per machine: dev builds stay byte-reproducible, and an install from one
machine can be upgraded by a build from another. The result JSON and the console both say so:

```text
     Warning signing.macos.identity: ${DAY_SIGN_MACOS_IDENTITY} is not set — degrading to the dev signing tier
      Packed …/day-showcase-0.1.0-macos-appkit.dmg (dmg, dev-signed) sha256:ec51fa5f02ab…
     Warning day-showcase-0.1.0-macos-appkit.dmg is dev-signed — NOT distributable
```

A *resolved* configuration that is broken (a keystore path that doesn't exist, a rejected
notarization) is a hard failure with exit code 6.

## Continuous integration

Every CI run packs the showcase on each platform job and uploads the results as `dist-<target>`
artifacts, so the packaging path is exercised on every push, at the dev tier. Adding the `DAY_*`
repository secrets lights up release signing with no workflow changes. Version tags (`v*`) run the
`release` workflow, which packs every target and attaches the artifacts plus a `SHA256SUMS` file
to a draft GitHub Release.
