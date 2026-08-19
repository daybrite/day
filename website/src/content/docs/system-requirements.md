---
title: System requirements
description: What to install on a macOS, Windows, or Linux development host to build Day apps — required and optional packages per target, and setting up the Android, iOS, and HarmonyOS emulators.
order: 4
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Every Day app builds from one Rust toolchain plus the SDK of whichever platform you target. This
page lists what each development host needs, and what each target adds on top. Where an official
installer or guide exists, this page links to it rather than restating it — those instructions
change, and theirs is the copy that stays current.

## Check with day doctor

`day doctor` probes this host and prints what is present, what is missing, and the command that
fixes each miss. It is the source of truth; everything below is a written-out version of the same
checks.

```bash
day doctor                       # every toolkit buildable on this host
day doctor --toolkit android     # focus one toolkit, with full setup instructions
```

Bare `day doctor` treats a missing toolkit as a warning and exits 0, because you only need the
toolkits you build for. Naming a toolkit with `--toolkit` turns its misses into errors and prints
that toolkit's setup text. To go further and prove the answer, `day checkup` scaffolds a throwaway
app and builds (and packs) it for each target — see [CLI & projects](/docs/cli).

## Every host

| What | Version | Where |
|---|---|---|
| Rust | 1.89 or newer | [rustup.rs](https://rustup.rs) |
| The `day` CLI | current release | [Getting started](/docs/getting-started) |
| Git | any | [git-scm.com](https://git-scm.com/downloads) |

Install Rust through **rustup**, not Homebrew or a distro package. Cross-compiled targets (iOS,
Android, HarmonyOS, and the web) need rustup's per-target standard library, which a system rustc
does not carry. `rustup update stable` keeps you current.

Nothing else is universal. The rest depends on which targets you build.

## Which targets build on which host

| Target | macOS | Linux | Windows |
|---|:--:|:--:|:--:|
| `macos-appkit` | ✅ | — | — |
| `ios-uikit` | ✅ | — | — |
| `windows-xaml` | — | — | ✅ |
| `linux-gtk` / `windows-gtk` / `macos-gtk` | ✅ | ✅ | ✅ |
| `linux-qt` / `windows-qt` / `macos-qt` | ✅ | ✅ | ✅ |
| `android-mdc` | ✅ | ✅ | ✅ |
| `harmony-arkui` | ✅ | ✅ | ✅ |
| `web-dom` | ✅ | ✅ | ✅ |

Apple's toolkits build only on macOS, and XAML only on Windows, because both compile against SDKs
that ship with the host OS. GTK and Qt are portable, so a macOS or Windows machine can build and
run them for development even though `linux-gtk` and `linux-qt` are what you ship.

## macOS

Day pins no minimum macOS version of its own. The binding constraint is Xcode: install the newest
version your macOS supports, and check
[Apple's minimum requirements](https://developer.apple.com/support/xcode/) if you are on an older
release. Continuous integration builds on the current `macos-latest` runner (Apple silicon).

**Command-line tools** cover `macos-appkit`, and are enough for the GTK, Qt, Android, and web
targets too:

```bash
xcode-select --install
```

**Full Xcode** ([App Store](https://apps.apple.com/us/app/xcode/id497799835)) is required for
`ios-uikit`, whose build runs through `xcodebuild`. Point the command-line tools at it once
installed:

```bash
sudo xcode-select -s /Applications/Xcode.app
rustup target add aarch64-apple-ios-sim
```

A scaffolded app carries `platform/macos/DayApp.xcodeproj`, so `macos-appkit` also builds through
`xcodebuild` by default and wants full Xcode. Setting `DAY_MACOS_XCODE=0` falls back to a bare
cargo build, where the command-line tools suffice.

**[Homebrew](https://brew.sh)** provides the rest:

```bash
brew install gtk4 libadwaita pkg-config    # macos-gtk
brew install qt pkg-config                 # macos-qt
brew install openjdk@21                    # android-mdc
brew install qemu                          # the HarmonyOS emulator
```

Swift is needed only when a dependency embeds SwiftUI ([SwiftUI embedding](/docs/guide-swiftui));
Xcode and the command-line tools both provide it.

## Windows

**Windows 10 or 11.** The `windows-xaml` target uses the XAML that ships inside those releases
rather than WinUI 3, so there is no framework runtime for you or your users to install. See the
[Windows platform page](/docs/platforms/windows-xaml) for what that means in practice.

For `windows-xaml`, install the
[Visual Studio 2022 C++ Build Tools](https://visualstudio.microsoft.com/downloads/) (MSVC plus the
Windows 10/11 SDK) and the MSVC Rust toolchain:

```powershell
rustup default stable-msvc
```

For `windows-qt` and `windows-gtk`, install [MSYS2](https://www.msys2.org) and build with a **GNU**
Rust toolchain — MSVC cannot link MSYS2's import libraries, and the C++ shims are built from
pkg-config's flags, which an online-installer Qt does not ship:

```bash
pacman -S mingw-w64-x86_64-qt6-base                              # Qt
pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita      # GTK
rustup toolchain install stable-x86_64-pc-windows-gnu
```

On ARM64 hosts, use the CLANGARM64 environment's `mingw-w64-clang-aarch64-` packages and the
`stable-aarch64-pc-windows-gnullvm` toolchain. Build with MSYS2's `bin` on `PATH` and
`RUSTUP_TOOLCHAIN` set to the GNU toolchain; the
[Windows page](/docs/platforms/windows-xaml#qt-and-gtk-on-a-windows-host) walks through it.

## Linux

Day requires library versions rather than distro versions: **GTK 4 with libadwaita 1**, and **Qt
6**. Any distribution whose repositories carry those development packages works. Continuous
integration builds on Ubuntu 24.04.

```bash
# Debian / Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config     # linux-gtk
sudo apt install qt6-base-dev pkg-config                      # linux-qt
```

Fedora, Arch, and openSUSE ship the same libraries under their own names; check
[GTK's installation page](https://www.gtk.org/docs/installations/linux) and
[Qt's](https://doc.qt.io/qt-6/linux.html) for the equivalents.

`pkg-config` is how Day finds both toolkits, so it is required rather than convenient.

## Optional: web views

The [web view piece](/docs/internal/webview) needs one extra development package on the Linux
desktop toolkits. Without it the piece compiles out and renders a placeholder; the rest of the app
is unaffected.

| Toolkit | Package | Engine |
|---|---|---|
| GTK | `libwebkitgtk-6.0-dev` (Debian/Ubuntu), `mingw-w64-x86_64-webkitgtk6` (MSYS2) | [WebKitGTK](https://webkitgtk.org) 6 |
| Qt | `qt6-webengine-dev` (Debian/Ubuntu) | [Qt WebEngine](https://doc.qt.io/qt-6/qtwebengine-index.html) |

Two gaps are worth knowing before you go looking for them. Homebrew's `webkitgtk` vends the GTK 3
API and has no bottle, so `macos-gtk` builds without a web view. MSYS2 ships no Qt 6 WebEngine, so
`windows-qt` does too.

## Optional: packaging tools

None of these are needed to build or run an app — only to produce an installable artifact with
`day pack`. [Packaging & distribution](/docs/packaging) covers the formats themselves.

| Target | Tool | Install |
|---|---|---|
| `linux-gtk`, `linux-qt` | `flatpak-builder` | [flatpak.org](https://flatpak.org/setup/), plus the Flathub remote |
| `linux-gtk`, `linux-qt` | `linuxdeploy` and its GTK or Qt plugin | [linuxdeploy releases](https://github.com/linuxdeploy/linuxdeploy/releases) |
| `windows-xaml` | `makeappx`, `signtool` | the Windows 10/11 SDK (installed with the Build Tools) |
| `windows-xaml` | `makensis` | [NSIS](https://nsis.sourceforge.io), or `choco install nsis` |

Without the `linuxdeploy` GTK or Qt plugin an AppImage still builds, but it will only run on a
machine that already has the toolkit installed.

## Android

Android cross-compiles the app to a JNI shared library and runs it inside a Gradle app, so it needs
the Android SDK, an NDK, and a JDK regardless of which host you are on.

1. Install the **Android SDK**, most easily through
   [Android Studio](https://developer.android.com/studio), or the standalone
   [command-line tools](https://developer.android.com/tools). Day finds it at the platform default
   (`~/Library/Android/sdk` on macOS, `%LOCALAPPDATA%\Android\Sdk` on Windows, `~/Android/Sdk` on
   Linux); set `ANDROID_HOME` if yours is elsewhere.
2. Install an **NDK** — `sdkmanager --install "ndk;<version>"`, or Android Studio's SDK Manager
   under *SDK Tools*. Day uses the newest one under `<sdk>/ndk` unless `ANDROID_NDK_HOME` says
   otherwise.
3. Install a **JDK, version 17 or newer** (`brew install openjdk@21`, or
   [Adoptium](https://adoptium.net)). The Gradle build uses `$JAVA_HOME`, so set it if the `java`
   on your `PATH` is older.
4. Add the Rust target and `cargo-ndk`:

```bash
rustup target add aarch64-linux-android    # arm64 device or emulator
rustup target add x86_64-linux-android     # x86_64 emulator
cargo install cargo-ndk
```

### Setting up an emulator

Create an AVD in Android Studio's **Device Manager**, or with `avdmanager create avd`, then start
it:

```bash
emulator -avd <name>
adb devices                 # confirm it is listed as `device`
```

The `day` CLI has no Android-emulator command of its own; use Android Studio or the SDK's own
tools. Match the emulator's ABI to an installed Rust target — an x86_64 system image needs
`x86_64-linux-android`. Set `ANDROID_SERIAL` when more than one device or emulator is attached, so
`day launch` and `day drive` act on the one you mean.

A booted emulator is needed only to run an app, not to build one.

## iOS

`ios-uikit` builds on a macOS host with full Xcode, as covered above. Xcode installs one iOS
simulator runtime; add others from Xcode's settings under *Platforms* (or *Components*, depending
on your Xcode version) — Apple documents the flow in
[Installing additional simulator runtimes](https://developer.apple.com/documentation/xcode/installing-additional-simulator-runtimes).

```bash
xcrun simctl list devices          # what exists, and what is booted
xcrun simctl boot "iPhone 16 Pro"  # or open Simulator.app
```

`day launch -p ios-uikit` installs into a booted simulator, so boot one first. Apps ship to a
physical device through Xcode's normal signing setup; nothing extra is needed for the Simulator.

## HarmonyOS

HarmonyOS has two halves with different tool needs, which is why a partial install is common. The
[HarmonyOS platform page](/docs/platforms/harmony-arkui) has the detail.

1. **The Rust cross-compile** needs the OpenHarmony SDK's `native` component, which downloads
   without a Huawei account from
   [repo.huaweicloud.com](https://repo.huaweicloud.com/openharmony/os/). Point `OHOS_NDK_HOME` at
   it, and add the targets:

   ```bash
   rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos
   ```

2. **Packaging the `.hap`** needs `hvigor` and `ohpm`, which are not part of the public SDK. They
   ship with the OpenHarmony **command-line-tools**, bundled with
   [DevEco Studio](https://developer.huawei.com/consumer/en/deveco-studio/) or downloadable on
   their own. Put their `bin/` directories on `PATH`.

`hdc`, which installs and launches the app, sits in the SDK's sibling `toolchains/` directory; Day
finds it there or on `PATH`.

### Setting up an emulator

Day runs the [Oniro](https://oniroproject.org) OpenHarmony emulator directly under QEMU, with no
DevEco Studio and no account:

```bash
brew install qemu                        # or your distro's qemu-system-x86_64
# download oniro_emulator.zip and unpack its images, then:
export DAY_OHOS_EMULATOR=~/ohos/emulator/images
day ohos emulator launch                 # --headless for CI
```

The image comes from the
[device_board_oniro releases](https://github.com/eclipse-oniro4openharmony/device_board_oniro/releases)
(v6.1 is what Day's own CI runs). `DAY_OHOS_EMULATOR` defaults to `~/ohos/emulator/images`, so you
can skip the variable by unpacking there.

The x86_64 emulator image carries an arm64-only ArkWeb engine, so the web view piece does not
render there. It works on a physical device.

## Web

`web-dom` needs one thing:

```bash
rustup target add wasm32-unknown-unknown
```

`day build -p web-dom` writes a self-contained static site, and `day launch -p web-dom` serves it
and opens a browser. Rust is the whole toolchain; there is no Node.js or bundler step.

## What your apps require

These are the minimums your *users* need, which the scaffold sets and you can raise in your
project's platform configuration. They are unrelated to what your development machine needs.

| Target | Minimum |
|---|---|
| `macos-appkit` | macOS 13 |
| `ios-uikit` | iOS 15 |
| `android-mdc` | API level 24 (Android 7.0), compiled against API 35 |
| `harmony-arkui` | API level 18 |
| `windows-xaml` | Windows 10 or 11 |
| `linux-gtk` / `linux-qt` | GTK 4 with libadwaita 1 / Qt 6 |
| `web-dom` | a current browser, served as static files |
