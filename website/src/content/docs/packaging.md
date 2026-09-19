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

`day pack` turns a Day project into an installable package. It builds in release mode, invokes
the platform’s signing and packaging tools, and writes the output, checksum, and signing status
to `build/day/dist/`.

The command runs from the project directory and requires the target’s
[packaging tools](/docs/system-requirements#optional-packaging-tools). For an Android APK:

```bash
day pack -p android-mdc --formats apk
```

Configure release credentials before distributing the result. Without them, Day can produce a
development-signed or unsigned artifact; check the reported [signing tier](#signing-tiers).
If signing fails, start with the [signing checks](/docs/troubleshooting#signing-or-provisioning-fails).

## Artifacts per target

| Target | Output | Distribution |
|---|---|---|
| `macos-appkit` | `.dmg` | Disk image containing the app |
| `ios-uikit` | `.ipa` | Device app archive |
| `android-mdc` | `.apk`, `.aab` | Direct installation or Google Play |
| `linux-gtk`, `linux-qt` | `.flatpak`, `.appimage` | Flatpak bundle or executable with bundled libraries |
| `windows-xaml` | `.msix`, `-setup.exe` | MSIX package or per-user installer |
| `harmony-arkui` | `.hap` | HarmonyOS application package |
| `web-dom` | Static `dist/` directory | Static hosting; use `day build` |

The development combinations `macos-gtk`, `macos-qt`, `windows-gtk`, and `windows-qt` do not have a
`day pack` step. See [platform support](/docs/platforms) for their limitations.

### macOS and iOS

For macOS, Day assembles the `.app`, signs its nested components with `codesign`, and creates a
compressed UDZO disk image with an `/Applications` link. With release credentials configured,
it signs the disk image, submits it through `notarytool`, and staples the notarization ticket.

For iOS, Day runs `xcodebuild archive` for an arm64 device and exports the archive with a generated
`ExportOptions.plist` using `app-store-connect`. Without signing configuration, it produces
`<stem>-ios-uikit-unsigned.ipa` for subsequent signing or sideloading.

### Android

Day runs Gradle’s `assembleRelease` and `bundleRelease` with the configured signing settings.
It verifies the APK with `apksigner` and checks 16 KB page alignment. Use the APK for direct
installation and the AAB for Google Play submission.

### Linux: Flatpak or AppImage

A Flatpak bundle uses a shared runtime: `org.gnome.Platform` for GTK or `org.kde.Platform` for Qt.
The runtime is resolved from Flathub during installation. Install a bundle with:

```bash
flatpak install ./my-app-1.0-linux-gtk-x86_64.flatpak
```

The filename includes the toolkit so GTK and Qt bundles can coexist. The webview crate declares a Qt WebEngine BaseApp requirement, which Day includes when the
binary links that engine. Other dependencies can declare their own
[Flatpak base requirements](/docs/internal/extending#flatpak-dependencies).

An AppImage bundles the toolkit libraries with the executable. Mark it executable before running it:

```bash
chmod +x ./my-app-1.0-linux-gtk-x86_64.appimage
./my-app-1.0-linux-gtk-x86_64.appimage
```

Day prepares an AppDir and invokes [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) with
its GTK or Qt plugin. These plugins collect resources that a library dependency scan can miss,
including image loaders, GIO modules, GSettings schemas, and Qt platform plugins.

Without the matching plugin, packaging can still succeed, but the AppImage requires the toolkit
to be installed on the user’s machine. Day reports this during packaging.

### Windows and HarmonyOS

Windows packaging uses `makeappx` and `signtool` for MSIX, and NSIS for the per-user `-setup.exe`
installer. The NSIS installer does not require elevation, registers with Add/Remove Programs,
and accepts `/S` for silent installation.

HarmonyOS packaging uses hvigor for a release build and `hap-sign-tool` for signing. Day uses
configured release credentials when available, or the public development certificate otherwise.

### Web

There is no web packaging step. Run `day build -p web-dom` and deploy the generated `dist/`
directory to a static host. It contains the HTML host page, JavaScript shim, stylesheet,
WebAssembly module, images, and fonts.

## Packaging options

Use `--formats` to select a subset of output formats, as in the Android example above.
`--no-sign` skips signing, and `--no-notarize` skips macOS notarization. To submit notarization
without waiting for completion, pass `--no-wait` and check it later with
`day sign status <id>`.

Artifact filenames follow this pattern:

```text
<stem>[-<version>]-<target>[-<extra>].<ext>
```

For example: `day-showcase-0.1.0-macos-appkit.dmg`. Set the stem with `[app] artifact` in
`Day.toml`, or override it with `--artifact-name <stem>`; Day converts the value to a filename slug.
Use `--no-version-in-name` for a stable filename suitable for a `releases/latest/download/<name>` URL.

## Signing configuration

Signing lives in `Day.toml` under `[signing]`, with every secret referenced as `${ENV_VAR}`; values
resolve from the environment at pack time and are never stored in the manifest or printed by the
tool:

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

`day sign check` reports each platform's readiness (env vars set, key files present) without
printing any secret value.

## Signing an existing package

`day pack` builds and signs in one step, which is what a project's own CI wants. A distributor
wants the two apart: build the submitted source with no credentials in reach, then sign the
artifact that came out. Building runs the app's own code — build scripts, Gradle plugins, Xcode
run scripts — and signing runs none of it, so the credentials only ever meet a finished file.

```sh
day sign apply app-release.aab                 # signs in place
day sign apply app-release.apk --out signed.apk # leaves the input alone
```

The keys come from `[signing.android]`, resolved strictly: an unset `${VAR}` is an error naming
the variable rather than a quiet drop to the dev keystore, because a dev-signed artifact looks
finished and no store will take it. `.aab` goes through `jarsigner`, `.apk` through `zipalign`
then `apksigner` (v4 off, so no `.idsig` litter), and passwords reach both through the
environment rather than the argument list, which every other process on the machine can read.

The signed copy is written beside the destination and moved into place only after it verifies, so
a failure leaves the unsigned artifact exactly as it was. The report names both digests and the
signing certificate, which is the field a store matches an upload against:

```text
      Signed app-release.aab
      Digest 5f080ef8becf6015… → b9cc48e09326d9a1…
 Certificate sha256 98860a7b9c357cb68fee7d1cb6944238f8316a02ca75972ffc89b56e8ed68e24
```

`--format json` prints the same four fields as `artifact`, `sha256`, `sha256_unsigned` and
`certificate_sha256`, which is what a pipeline records to tie a published binary to the build it
came from.

### Apple packages

An `.ipa` (or a bare `.app`, which comes back out as an `.ipa`) takes the steps Xcode's export
performs, spelled out because no project is present to drive it: embed the provisioning profile,
sign every nested framework and plug-in before the bundle that contains them, sign the app, then
repack with `ditto`.

```sh
day sign apply app.ipa \
  --profile "AppStore.mobileprovision" \
  --identity "iPhone Distribution: The App Fair Project Inc (25KG25YA3R)"
```

`--profile` and `--identity` default to `signing.ios.profile` and `signing.ios.identity`. The
flags matter more here than the Day.toml values do: a distributor signs an artifact built from
someone else's project, whose `Day.toml` names the developer's profile and certificate rather
than the distributor's.

**Entitlements come from the profile** unless `--entitlements <file>` says otherwise. That is what
Xcode does when a project overrides nothing, and it is the safe default in both directions:
entitlements a profile does not grant are rejected at install, and asking for fewer than it grants
silently drops capabilities the app shipped with. The profile's set is what lands —
`application-identifier`, `aps-environment`, `beta-reports-active`, `get-task-allow=false` for a
distribution profile.

The signed bundle is verified with `codesign --verify --deep --strict` before it replaces
anything, so a bad identity or an out-of-order nested signature fails with the artifact untouched.
`certificate_sha256` is the signing certificate itself, the same field the Android path reports.

Signing an Apple package needs macOS: `codesign`, `security` and `ditto` ship with Xcode. `.dmg`
and `.pkg` are named as not-yet-implemented rather than refused as unknown; `day pack -p
macos-appkit` signs and notarizes those during the build.

## Signing tiers

Every artifact carries a tier: **release**, **dev-signed**, or **unsigned**. When a `${VAR}` is
unset (a laptop without the release keys, a fork PR without repository secrets), `day pack` warns
naming the variable and drops that platform to the dev tier (ad-hoc codesign on macOS, the fixed dev
keystore embedded in the CLI on Android, a self-signed certificate on Windows, the unsigned device
`.ipa` on iOS) instead of failing. The Android dev keystore is fixed, so dev builds stay
byte-reproducible and an install from one machine can be upgraded by a build from another. The
result JSON and the console both say so:

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
repository secrets enables release signing without any workflow change. Version tags (`v*`) run the
`release` workflow, which packs every target and attaches the artifacts plus a `SHA256SUMS` file
to a draft GitHub Release.

## macOS App Sandbox

Enable `[sandbox.macos-appkit] enabled = true` in Day.toml to sandbox AppKit builds and
packaged apps. See [macOS sandboxing](/docs/internal/sandbox) for configuration, native file
access, persistent bookmarks, and App Store distribution limitations.
