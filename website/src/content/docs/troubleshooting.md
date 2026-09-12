---
title: Troubleshooting
description: Diagnose setup, build, device, signing, and launch problems in a Day project.
order: 5
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

If your first app will not build or launch, start with the toolchain check below. If you already
have an error, jump to the matching symptom. Error wording varies between SDK versions.

| Symptom | Where to start |
|---|---|
| `day` is not found | [CLI installation](#day-is-not-found) |
| Missing SDK, compiler, or build tool | [Check the toolchain](#check-the-toolchain) |
| Download failure or a slow first build | [First-build problems](#the-first-build-is-slow-or-fails) |
| Rust cannot find `core` or `std` | [Missing Rust target](#rust-cannot-find-core-or-std) |
| Xcode or simulator errors | [Apple targets](#xcode-or-the-ios-simulator-is-not-ready) |
| Android SDK, Java, or device errors | [Android](#android-will-not-build-or-find-a-device) |
| GTK, libadwaita, or Qt cannot be found | [Linux libraries](#linux-cannot-find-gtk-libadwaita-or-qt) |
| Windows linker errors | [Windows toolchains](#windows-linking-fails) |
| HarmonyOS builds Rust but cannot package or launch | [HarmonyOS tools](#harmonyos-builds-rust-but-cannot-package-or-launch) |
| Signing or provisioning fails | [Signing](#signing-or-provisioning-fails) |
| The web build does not open correctly | [Web launches](#the-web-build-does-not-open-correctly) |
| The app exits or a feature is missing | [Runtime problems](#the-app-starts-but-does-not-work-as-expected) |

## Check the toolchain

Run these commands in the same terminal you use to build the app:

```bash
day --version
rustc --version
rustup show active-toolchain
day doctor
```

A plain `day doctor` checks the toolkits available on your development host. Missing optional
tools appear as warnings; you do not need to install every toolkit. Focus the check on the one
that failed to get setup instructions and a failing exit status for missing requirements:

```bash
day doctor --toolkit android
```

Doctor uses toolkit names, while build and launch commands use target names:

| Build target | Doctor toolkit |
|---|---|
| `macos-appkit` | `appkit` |
| `ios-uikit` | `uikit` |
| `android-mdc` | `android` |
| `linux-gtk`, `macos-gtk`, `windows-gtk` | `gtk` |
| `linux-qt`, `macos-qt`, `windows-qt` | `qt` |
| `windows-xaml` | `xaml` |
| `harmony-arkui` | `harmonyos` |
| `web-dom` | `dom` |

Install the missing tools using [system requirements](/docs/system-requirements), then repeat the
focused check. A successful check means those tools were found; it does not validate your app’s
code, signing credentials, or device connection.

## `day` is not found

If `cargo` is also missing, install [Rust through rustup](https://rustup.rs), then open a new
terminal. Otherwise, install the CLI:

```bash
cargo install day-cli
```

If installation succeeds but `day --version` still fails, check that Cargo’s executable directory
is on `PATH`. With the default Cargo location, that is `~/.cargo/bin` on macOS and Linux, or
`%USERPROFILE%\.cargo\bin` on Windows. A custom `CARGO_HOME` changes that location.
Restart your editor if its integrated terminal still has the old environment.

## The first build is slow or fails

The first build downloads dependencies and compiles the selected backend. Later builds can reuse
that work. Look at the last output before deciding that a build has stopped:

- **Downloading or updating a Git repository:** a timeout, proxy error, or authentication failure
  points to network access. Check access to the URL printed by Cargo, then retry the same command.
- **Compiling crates:** this is expected on a first build. Switching targets or build profiles can
  require more compilation.
- **Waiting for a build-directory lock:** another build may be using the same output directory.
  Check for a build running in your editor or another terminal.
- **Compilation failed:** find the first specific error above the final failure summary. A missing
  library belongs to the platform setup checks below; a Rust error with a source location usually
  needs a code change.

Run one target at a time while diagnosing the problem. For example, on a macOS host:

```bash
day build -p macos-appkit
```

Use a target configured in your app’s `Day.toml` and supported on your host. If the build succeeds
but `day launch` fails, move on to device or runtime checks. Deleting build caches makes the next
build start over and usually does not fix a missing SDK or source error.

## Rust cannot find `core` or `std`

An error such as `can't find crate for core` can mean that the Rust standard library for the
requested target is not installed. List the installed targets:

```bash
rustup target list --installed
```

Add the target named in the build error. For the web backend:

```bash
rustup target add wasm32-unknown-unknown
```

For mobile builds, choose the target that matches the device or simulator architecture; the
[requirements guide](/docs/system-requirements) lists the choices. Install it for the Rust
toolchain your project uses, as shown by `rustup show active-toolchain` inside the
project directory. Rustup’s [cross-compilation guide](https://rust-lang.github.io/rustup/cross-compilation.html)
explains target installation; platform SDKs and linkers are separate requirements.

## Xcode or the iOS Simulator is not ready

Errors mentioning `xcodebuild`, an invalid developer directory, or a missing Apple SDK often mean
that full Xcode is absent or the command-line tools point elsewhere. Check:

```bash
xcode-select -p
xcodebuild -version
```

Generated macOS app projects and iOS builds need full Xcode. If Xcode is installed in its usual
location, select it with:

```bash
sudo xcode-select -s /Applications/Xcode.app
```

Open Xcode and complete any first-launch component installation or license prompts. If you keep
Xcode elsewhere, use that installation’s path. See [macOS requirements](/docs/system-requirements#macos).

For an iOS launch, check the available simulators:

```bash
day devices list -p ios-uikit
```

If none are available, install an iOS Simulator runtime through Xcode’s settings. If one is
available but stopped, replace `SIMULATOR_ID` with its listed ID:

```bash
day devices boot -p ios-uikit SIMULATOR_ID --wait
day launch -p ios-uikit
```

A physical iPhone or iPad also needs device setup and signing. See Apple’s
[guide to running on simulated or physical devices](https://developer.apple.com/documentation/xcode/running-your-app-on-simulated-or-physical-devices)
and the [signing checks below](#signing-or-provisioning-fails).

## Android will not build or find a device

Start with `day doctor --toolkit android`. A missing SDK or NDK requires the corresponding
component in Android Studio’s SDK Manager. If your SDK is outside its usual location, set
`ANDROID_HOME` to that SDK directory. Check `ANDROID_NDK_HOME` too if you have explicitly set it.

For Java or Gradle compatibility errors, check the JDK selected by `JAVA_HOME`. Day’s Gradle
builds use that setting, so a newer `java` on `PATH` does not fix a `JAVA_HOME` pointing at an older
JDK. Follow the [Android setup instructions](/docs/system-requirements#android), then repeat doctor.

For a launch failure:

```bash
day devices list -p android-mdc
adb devices
```

If `adb` is missing, install Android SDK Platform Tools and add the SDK’s `platform-tools`
directory to `PATH`. Interpret its device list as follows:

| Result | What to do |
|---|---|
| No device listed | Start an AVD in Android Studio, or connect a device with USB debugging enabled. |
| `unauthorized` | Unlock the device and accept its debugging authorization prompt. |
| `offline` | Wait for boot to finish; if it remains offline, reconnect the device or restart the emulator. |
| More than one device | Set `ANDROID_SERIAL` to the serial of the device you intend to use. |
| `device` | The connection is ready; check the subsequent install or launch error. |

You can also boot an existing AVD with `day devices boot -p android-mdc AVD_NAME --wait`, using
its name from the Day device list. The emulator’s architecture must match an installed Rust
target. Android’s [ADB documentation](https://developer.android.com/tools/adb) covers device
connections and selection in more detail.

## Linux cannot find GTK, libadwaita, or Qt

An error from `pkg-config`, `gdk4-sys`, or a native build script can mean that development packages
are missing or too old. Having GTK or Qt applications installed does not mean their development
headers are installed.

```bash
day doctor --toolkit gtk
day doctor --toolkit qt
```

Run the check for the toolkit you use. Day’s GTK backend requires GTK 4.10 and libadwaita 1.5 or
newer; the Qt backend requires Qt 6. [Linux requirements](/docs/system-requirements#linux) lists the
packages. If your distribution supplies older libraries, use a newer development environment
or choose a backend whose requirements it meets.

If the packages are installed in a custom location, check whether `pkg-config` can find their
`.pc` files. Configure `PKG_CONFIG_PATH` for that installation instead of copying library files
into system directories.

## Windows linking fails

Check which toolchain the project is using:

```powershell
rustup show active-toolchain
```

`windows-xaml` needs the MSVC toolchain, Visual Studio C++ Build Tools, and the Windows SDK.
A missing `link.exe` points to that setup. Windows GTK and Qt builds use MSYS2 packages and a
GNU-compatible Rust toolchain; mixing their import libraries with MSVC causes linking failures.
Follow the [Windows setup instructions](/docs/system-requirements#windows) for your backend and
host architecture, then run its focused doctor check in the same terminal.

## HarmonyOS builds Rust but cannot package or launch

A successful Rust build only confirms the native compilation tools are available. Packaging
also needs `hvigor` and `ohpm`; installation and launch need `hdc` and a reachable device or
emulator.

```bash
day doctor --toolkit harmonyos
day devices list -p harmony-arkui
```

Check `OHOS_NDK_HOME` and the command-line tools installation against the
[HarmonyOS requirements](/docs/system-requirements#harmonyos). If doctor passes but the device list
is empty, finish the emulator or device setup before retrying the launch.

## Signing or provisioning fails

A simulator build can succeed while a device build or release package fails to sign. From your
project directory, check the signing configuration:

```bash
day sign --check
```

This checks whether configured environment variables and files can be resolved. It does not
prove that a certificate or provisioning profile is valid for the app, device, and distribution
method. Read the signing tool’s error, then check the app identifier, team, certificate, and
profile it names. [Packaging and distribution](/docs/packaging) documents Day’s signing settings.
Do not post private keys, passwords, or signing credentials when asking for help.

## The web build does not open correctly

Use Day’s local server rather than opening the generated HTML through a `file://` URL:

```bash
day launch -p web-dom
```

If compilation fails, run `day doctor --toolkit dom` and check that the wasm Rust target is
installed. If the page loads but the app does not start, inspect the browser’s console and network
panel for failed JavaScript or WebAssembly requests. See the
[web platform guide](/docs/platforms/web-dom) for build and hosting details.

## The app starts but does not work as expected

Keep the launch terminal open and look for a panic or platform error when the problem happens.
[Logging](/docs/logging) explains where Day sends app logs; [crash reporting](/docs/guide-crash-reporting)
covers collecting failures from deployed apps.

If only one feature fails, check its platform support and permissions. For example, a web view
placeholder on a desktop target may mean its optional engine was not included; see
[web view requirements](/docs/system-requirements#optional-web-views). Camera and other protected
features may also need [permission configuration](/docs/guide-permissions).

## Still stuck?

Try the smallest app that reproduces the problem on one target. When you
[report an issue](https://github.com/daybrite/day/issues), include:

- The command you ran and the first relevant error, with enough surrounding output to identify it.
- Your development OS and architecture, target, and device or emulator details.
- `day --version`, `rustc --version`, and the focused `day doctor` output.
- A small reproduction or the source revision and steps needed to reproduce the failure.

Remove credentials and personal information from logs before sharing them. Mention whether a
newly created app fails too; that helps distinguish machine setup from a project-specific problem.
