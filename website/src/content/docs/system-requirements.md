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

Building a Day app requires Rust and the SDK or development libraries for each target.
The requirements below are grouped by development host and target platform. `day doctor` can
check the installed tools and report what is missing.

## Check with day doctor

`day doctor` checks your development host for installed toolchains and reports missing
requirements with setup guidance. The sections below describe the tools needed by each target.

```bash
day doctor                       # every toolkit buildable on this host
day doctor --toolkit android     # focus one toolkit, with full setup instructions
```

Bare `day doctor` treats a missing toolkit as a warning and exits 0, because you only need the
toolkits you build for. Naming a toolkit with `--toolkit` turns its misses into errors and prints
that toolkit's setup text. For a full check, `day doctor verify` scaffolds a throwaway app and builds
(and packs) it for each target; see [CLI & projects](/docs/cli).

For build errors or device connection problems, see [Troubleshooting](/docs/troubleshooting).

## Every host

| What | Version | Where |
|---|---|---|
| Rust | 1.89 or newer | [rustup.rs](https://rustup.rs) |
| The `day` CLI | current release | [Getting started](/docs/getting-started) |
| Git | any | [git-scm.com](https://git-scm.com/downloads) |

Install Rust through **rustup**, not Homebrew or a distro package. Cross-compiled targets (iOS,
Android, HarmonyOS, and the web) need rustup's per-target standard library, which a system rustc
does not carry. `day build` adds the one a target needs the first time it builds for it
(`rustup target add …`, said on the terminal as it happens), so the `rustup target add` lines
in the sections below are for doing that ahead of time, on a machine that will be offline or in
a CI image. `rustup update stable` keeps you current.

Everything else depends on which targets you build.

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
run them for development even though you ship `linux-gtk` and `linux-qt`.

## macOS

The required macOS version depends on Xcode: install the newest
version your macOS supports, and check
[Apple's minimum requirements](https://developer.apple.com/support/xcode/) if you are on an older
release. Continuous integration builds on the current `macos-latest` runner (Apple silicon).

The GTK, Qt, Android, and web targets need Apple’s command-line development tools on macOS:

```bash
xcode-select --install
```

Full Xcode ([App Store](https://apps.apple.com/us/app/xcode/id497799835)) is required for
`ios-uikit`, whose build runs through `xcodebuild`. Point the command-line tools at it once
installed:

```bash
sudo xcode-select -s /Applications/Xcode.app
rustup target add aarch64-apple-ios-sim
```

A scaffolded app carries `platform/macos/DayApp.xcodeproj`, and `macos-appkit` builds through
`xcodebuild`, so it wants full Xcode too. (An app that predates the scaffold adopts it with
`day project add-target macos-appkit`.)

[Homebrew](https://brew.sh) provides the rest:

```bash
brew install gtk4 libadwaita pkg-config    # macos-gtk
brew install qt pkg-config                 # macos-qt
brew install openjdk@21                    # android-mdc
brew install qemu                          # the HarmonyOS emulator
```

Swift is needed only when a dependency embeds SwiftUI ([SwiftUI embedding](/docs/guide-swiftui));
Xcode and the command-line tools both provide it.

## Windows

On Windows 10 or 11, the `windows-xaml` target uses the XAML that ships inside those releases rather
than WinUI 3, so there is no framework runtime for you or your users to install. See the [Windows
platform page](/docs/platforms/windows-xaml) for the details.

For `windows-xaml`, install the
[Visual Studio 2022 C++ Build Tools](https://visualstudio.microsoft.com/downloads/) (MSVC plus the
Windows 10/11 SDK) and the MSVC Rust toolchain:

```powershell
rustup default stable-msvc
```

For `windows-qt` and `windows-gtk`, install [MSYS2](https://www.msys2.org) and build with a **GNU**
Rust toolchain, because MSVC cannot link MSYS2's import libraries, and the C++ shims are built
from pkg-config's flags, which an online-installer Qt does not ship:

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

Day's minimums are library versions: **GTK 4.10 with libadwaita 1.5**, and **Qt 6**. Any
distribution whose repositories carry those development packages works. Continuous integration
builds on Ubuntu 24.04.

```bash
# Debian / Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config     # linux-gtk
sudo apt install qt6-base-dev pkg-config                      # linux-qt
sudo apt install qemu-system-x86 unzip                        # harmony-arkui (emulator)
```

The GTK minimums are hard requirements. Day builds stack navigation on `AdwNavigationView`, and
its file and alert dialogs on `GtkFileDialog` and `GtkAlertDialog`; none of those exist in earlier
releases. Debian 12 ships GTK 4.8 and cannot build `linux-gtk`; run `-p
linux-qt` there, which needs only Qt 6, or build against a newer runtime. `day doctor` reports the
installed versions against these minimums, so run it first; a version miss otherwise surfaces as a
build failure inside `gdk4-sys`.

Fedora, Arch, and openSUSE ship the same libraries under different package names; check
[GTK's installation page](https://www.gtk.org/docs/installations/linux) and
[Qt's](https://doc.qt.io/qt-6/linux.html) for the equivalents.

Day finds both toolkits through `pkg-config`, so it is required.

## Optional: web views

The [web view piece](https://github.com/daybrite/day-piece-webview) needs one extra development package on the Linux
desktop toolkits. Without it the piece compiles out and renders a placeholder; the rest of the app
is unaffected.

| Toolkit | Package | Engine |
|---|---|---|
| GTK | `libwebkitgtk-6.0-dev` (Debian/Ubuntu), `mingw-w64-x86_64-webkitgtk6` (MSYS2) | [WebKitGTK](https://webkitgtk.org) 6 |
| Qt | `qt6-webengine-dev` (Debian/Ubuntu) | [Qt WebEngine](https://doc.qt.io/qt-6/qtwebengine-index.html) |

Two toolkit combinations have no web view package. Homebrew's `webkitgtk` vends the GTK 3
API and has no bottle, so `macos-gtk` builds without a web view. MSYS2 ships no Qt 6 WebEngine, so
`windows-qt` does too.

## Optional: media playback

The [media piece](https://github.com/daybrite/day-piece-media) plays through GStreamer on the
Linux GTK target (`GtkVideo`). The build needs nothing extra, but *playback* needs GTK's
GStreamer module and a decoder for each format. Without a decoder the player stays a blank black
rectangle rather than showing an error.

```bash
# Debian / Ubuntu
sudo apt install libgtk-4-media-gstreamer gstreamer1.0-plugins-good gstreamer1.0-libav
```

`gstreamer1.0-plugins-good` reads MP4 and streams over HTTPS (`qtdemux`, `souphttpsrc`), and
`gstreamer1.0-libav` decodes H.264 video and AAC audio, which is what most MP4 files carry;
Ubuntu's desktop install does not include it. To check a particular file or URL, run
`gst-discoverer-1.0 <url>`: any "Missing plugins" it lists is what the player lacks.

## Optional: packaging tools

These are needed only to produce an installable artifact with `day pack`. [Packaging &
distribution](/docs/packaging) covers the formats themselves.

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
2. Install an **NDK** with `sdkmanager --install "ndk;<version>"`, or from Android Studio's SDK
   Manager under *SDK Tools*. Day uses the newest one under `<sdk>/ndk` unless
   `ANDROID_NDK_HOME` says otherwise.
3. Install a **JDK, version 17 or newer** (`brew install openjdk@21`, or
   [Adoptium](https://adoptium.net)). The Gradle build uses `$JAVA_HOME`, so set it if the `java`
   on your `PATH` is older.
4. Add the Rust target and `cargo-ndk`:

```bash
rustup target add aarch64-linux-android    # arm64 device or emulator
rustup target add x86_64-linux-android     # x86_64 emulator
cargo install cargo-ndk
```

To open, build, or run the app from Android Studio, use **Android Studio 2026.1.4 (Quail 4) or
newer**. Day's Gradle plugin builds with the Android Gradle Plugin 9.4, and Android Studio syncs
only the AGP versions it supports. `day build` and `day launch` run Gradle themselves, so they work
with any Android Studio release.

### Setting up an emulator

Create an AVD in Android Studio's **Device Manager**, or with `avdmanager create avd`, then start
it:

```bash
emulator -avd <name>
adb devices                 # confirm it is listed as `device`
```

Day can list existing AVDs with `day devices list -p android-mdc` and start one with
`day devices boot -p android-mdc AVD_NAME --wait`. See
[Android troubleshooting](/docs/troubleshooting#android-will-not-build-or-find-a-device) if it is not detected. Match the emulator's ABI to an installed Rust target; an x86_64 system image needs
`x86_64-linux-android`. Set `ANDROID_SERIAL` when more than one device or emulator is attached, so
`day launch` and `day drive` act on the one you mean.

A booted emulator is needed only to run an app.

## iOS

`ios-uikit` builds on a macOS host with full Xcode, as covered above. Xcode installs one iOS
simulator runtime; add others from Xcode's settings under *Platforms* (or *Components*, depending
on your Xcode version); Apple documents the flow in
[Installing additional simulator runtimes](https://developer.apple.com/documentation/xcode/installing-additional-simulator-runtimes).

```bash
xcrun simctl list devices          # what exists, and what is booted
xcrun simctl boot "iPhone 16 Pro"  # or open Simulator.app
```

`day launch -p ios-uikit` installs into a booted simulator, so boot one first. Apps ship to a
physical device through Xcode's normal signing setup.

## HarmonyOS

HarmonyOS has two halves with different tool needs, which is why a partial install is common. The
[HarmonyOS platform page](/docs/platforms/harmony-arkui) has the detail.

1. **The Rust cross-compile** needs the OpenHarmony SDK's `native` component (the NDK). Point
   `OHOS_NDK_HOME` at it, and add the targets:

   ```bash
   rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos
   ```

2. **Packaging the `.hap`** needs `hvigor` and `ohpm`, which are not part of the public SDK. They
   ship with the OpenHarmony **command-line-tools**, bundled with
   [DevEco Studio](https://developer.huawei.com/consumer/en/deveco-studio/) or downloadable on
   their own without an account. Put their `bin/` directory on `PATH`. hvigor builds against the
   SDK named by `OHOS_BASE_SDK_HOME`, which must use the versioned layout `<dir>/<api>/…`.

3. **Signing** runs Day's `sign-hap.mjs` under `node`, so `node` must be on `PATH` too.

`hdc`, which installs and launches the app, sits in the SDK's `toolchains/` directory, beside
`native/`; Day finds it there or on `PATH`.

### On Linux

The Linux command-line-tools bundle carries everything above: hvigor, ohpm, a bundled node, and
an API 18 OpenHarmony SDK (`native`, `ets`, `toolchains` with `hdc` and the signing material)
whose tools run natively on a Linux x86_64 host. So one download covers the whole build, with no
separate SDK. It needs about 2.1 GB to download and 6.5 GB unpacked:

```bash
mkdir -p ~/ohos-clt && cd ~/ohos-clt
curl -fSLO https://repo.huaweicloud.com/harmonyos/ohpm/5.1.0/commandline-tools-linux-x64-5.1.0.840.zip
unzip -q commandline-tools-linux-x64-5.1.0.840.zip && rm commandline-tools-linux-x64-5.1.0.840.zip

# hvigor wants a versioned SDK layout; link the bundled SDK in as API 18.
mkdir -p ~/ohos/sdk
ln -sfn ~/ohos-clt/command-line-tools/sdk/default/openharmony ~/ohos/sdk/18
```

Then add these to your shell profile:

```bash
export OHOS_CLT=$HOME/ohos-clt/command-line-tools
export OHOS_NDK_HOME=$OHOS_CLT/sdk/default/openharmony/native
export OHOS_BASE_SDK_HOME=$HOME/ohos/sdk
export PATH=$OHOS_CLT/bin:$OHOS_CLT/tool/node/bin:$OHOS_CLT/sdk/default/openharmony/toolchains:$PATH
```

The bundle's `bin/hvigorw` and `bin/ohpm` run as-is on Linux, using the bundle's own node, and
`tool/node/bin` puts that node on `PATH` for signing, so no separate node install is needed. The
node wrapper scripts in the [HarmonyOS notes](/docs/internal/harmonyos) are only for macOS. `day
doctor --toolkit harmonyos` should now pass every check.

### On macOS

Use the same Linux command-line-tools bundle for hvigor and ohpm, which are pure JavaScript,
through the node wrappers in the [HarmonyOS notes](/docs/internal/harmonyos). The bundle's SDK
tools are Linux binaries, though, so hvigor fails with `spawn ENOEXEC`. Take the NDK, `hdc`, and
`OHOS_BASE_SDK_HOME` from the macOS public SDK (`L2-SDK-MAC-M1-PUBLIC.tar.gz` under
[repo.huaweicloud.com/openharmony/os](https://repo.huaweicloud.com/openharmony/os/)) instead.

### Setting up the Oniro emulator

Day runs the [Oniro](https://oniroproject.org) OpenHarmony emulator directly under QEMU, from a
public image download. You need `qemu-system-x86_64` (`sudo apt install qemu-system-x86`, or
`brew install qemu`) and about 7 GB of disk: a 1.4 GB zip that unpacks to 5.5 GB of images. The
zip holds an `images/` directory, so unpacking it in `~/ohos/emulator` lands the images at the
default location:

```bash
mkdir -p ~/ohos/emulator && cd ~/ohos/emulator
curl -fSLO https://github.com/eclipse-oniro4openharmony/device_board_oniro/releases/download/v6.1/oniro_emulator.zip
unzip -q oniro_emulator.zip && rm oniro_emulator.zip     # → ~/ohos/emulator/images
day devices boot -p harmony-arkui                       # --headless for no window
```

The image comes from the
[device_board_oniro releases](https://github.com/eclipse-oniro4openharmony/device_board_oniro/releases)
(v6.1, OpenHarmony 6.1, is what Day's CI runs). Set `DAY_OHOS_EMULATOR` if you keep the images
anywhere other than `~/ohos/emulator/images`.

To test on **OpenHarmony 7.0**, use the `x86_64_virt` phone package from
[harmony-contrib/ohos-qemu](https://github.com/harmony-contrib/ohos-qemu/releases) instead
(0.7 GB download, 4.7 GB unpacked). `day devices boot` recognizes its layout by its extra
`sys_prod.img` and `chip_prod.img`, so every `day` command works against either image:

```bash
mkdir -p ~/ohos/ohos-qemu && cd ~/ohos/ohos-qemu
curl -fSLO https://github.com/harmony-contrib/ohos-qemu/releases/download/v20260919/openharmony-qemu-x86_64-x86_64_virt-phone.tar.gz
tar xzf openharmony-qemu-x86_64-x86_64_virt-phone.tar.gz
export DAY_OHOS_EMULATOR=~/ohos/ohos-qemu/openharmony-qemu-x86_64-x86_64_virt-phone/images
day devices boot -p harmony-arkui --headless
```

Apps built for API 18 install and run on 7.0 unchanged. Boot prints which OpenHarmony came up. Boot returns once the guest has finished starting, and QEMU keeps
running in the background; `day launch -p harmony-arkui` then installs and starts the app on it.

On an x86_64 Linux host, the emulator runs KVM-accelerated when you can open `/dev/kvm`. That
normally means membership in the `kvm` group:

```bash
sudo usermod -aG kvm $USER      # then log out and back in
```

Without it, Day falls back to TCG software emulation, and boot takes several minutes. The
emulator gets six vCPUs, or as many as the host has cores when that is fewer; set
`DAY_OHOS_SMP` to override. On macOS the emulator always runs under TCG.

On Linux the emulator window is a GTK window drawn with OpenGL, and the guest runs at 640×480
landscape whatever `--device` asks for, because that is the size the window reports to the
guest. `--headless` keeps the requested panel. To watch a headless emulator, take screenshots with
`hdc shell uitest screenCap -p /data/local/tmp/s.png` and `hdc file recv /data/local/tmp/s.png`.

The x86_64 emulator image carries an arm64-only ArkWeb engine, so the web view piece does not
render there. It works on a physical device.

## Web

`web-dom` needs only the wasm target, which the first `day build -p web-dom` adds; to add it
yourself:

```bash
rustup target add wasm32-unknown-unknown
```

`day build -p web-dom` writes a self-contained static site, and `day launch -p web-dom` serves it
and opens a browser.

## What your apps require

These are the minimums your *users* need, which the scaffold sets and you can change in your
project's platform configuration — up to whatever your code needs, or down as far as the
platform's own tooling still allows. The iOS value is day-uikit's own floor: the toolkit guards
the UIKit calls that arrived after iOS 15, and a piece that needs a newer OS declares that
`platform` floor, which `day build` applies to the build. They are unrelated to what your
development machine needs.

| Target | Minimum |
|---|---|
| `macos-appkit` | macOS 13 |
| `ios-uikit` | iOS 15 |
| `android-mdc` | API level 24 (Android 7.0), compiled against API 37 |
| `harmony-arkui` | API level 18 |
| `windows-xaml` | Windows 10 or 11 |
| `linux-gtk` / `linux-qt` | GTK 4.10 with libadwaita 1.5 / Qt 6 |
| `web-dom` | a current browser, served as static files |
