---
title: Troubleshooting
description: Diagnose setup, build, device, signing, and launch problems in a Day project.
order: 5
section: How-to guides
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
| A video is black on Linux | [Media playback](#a-video-plays-as-a-black-rectangle-on-linux) |

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
| `windows-winui` | `winui` |
| `harmony-arkui` | `harmonyos` |
| `web-dom` | `dom` |

Install the missing tools using [system requirements](/docs/reference/system-requirements), then repeat the
focused check. A successful check means those tools were found; it does not validate your app’s
code, signing credentials, or device connection.

## `day` is not found

If `cargo` is also missing, install [Rust through rustup](https://rustup.rs), then open a new
terminal. Install the Day CLI:

```sh
cargo install --git https://github.com/daybrite/day day-cli
```

If installation succeeds but the shell cannot find `day`, open a new terminal. Rustup normally
adds Cargo's executable directory to PATH, but existing terminals and editors can retain the
old environment. Restart your editor if its integrated terminal still cannot find the command.

On macOS or Linux, with a default Rust installation, enable it in the current Bash or Zsh session:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
day --version
```

If new terminals still need this fix, add the `export` line once to `~/.zshrc` for Zsh or
`~/.bashrc` for interactive Bash. Bash login shells use a login profile such as `~/.bash_profile`;
it can source `~/.bashrc` to share the setting.

To run the installed command without changing PATH:

```sh
"$HOME/.cargo/bin/day" --version
```

On Windows PowerShell, the equivalent direct invocation is:

```powershell
& "$env:USERPROFILE\.cargo\bin\day.exe" --version
```

For a persistent Windows fix, add `%USERPROFILE%\.cargo\bin` to your user `Path` in the Windows
environment-variable settings, then reopen the terminal.

These paths assume Cargo's default installation directory. If you set `CARGO_HOME`,
`CARGO_INSTALL_ROOT`, Cargo's `install.root`, or `cargo install --root`, use the actual installation
directory's `bin` folder instead. See [Cargo's installation-root rules](https://doc.rust-lang.org/cargo/commands/cargo-install.html).

`cargo run` builds and runs a local project's executable; it does not launch a package installed
by `cargo install`. `cargo run day-cli` is therefore not a replacement for `day`.
See the [Cargo run reference](https://doc.rust-lang.org/cargo/commands/cargo-run.html).

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
day build -t macos-appkit
```

Use a target configured in your app’s `Day.toml` and supported on your host. If the build succeeds
but `day launch` fails, move on to device or runtime checks. Deleting build caches makes the next
build start over and usually does not fix a missing SDK or source error.

## Rust cannot find `core` or `std`

An error such as `can't find crate for core` means the Rust standard library for the
requested target is not installed. `day build` adds a missing target through rustup before it
compiles, so this is reached only with a Rust that is not rustup's, or when that install
failed (no network, say). List the installed targets:

```bash
rustup target list --installed
```

Add the target named in the build error. For the web backend:

```bash
rustup target add wasm32-unknown-unknown
```

For mobile builds, choose the target that matches the device or simulator architecture; the
[requirements guide](/docs/reference/system-requirements) lists the choices. Install it for the Rust
toolchain your project uses, as shown by `rustup show active-toolchain` inside the
project directory. Rustup’s [cross-compilation guide](https://rust-lang.github.io/rustup/cross-compilation.html)
explains target installation; platform SDKs and linkers are separate requirements.

## Xcode or the iOS Simulator is not ready

Errors mentioning `xcodebuild`, an invalid developer directory, or a missing Apple SDK often mean
that full Xcode 27 or newer is absent or the command-line tools point elsewhere. Check:

```bash
xcode-select -p
xcodebuild -version
```

Generated macOS app projects and iOS builds need full Xcode 27 or newer. If Xcode is installed in its usual
location, select it with:

```bash
sudo xcode-select -s /Applications/Xcode.app
```

Open Xcode and complete any first-launch component installation or license prompts. If you keep
Xcode elsewhere, use that installation’s path. See [macOS requirements](/docs/reference/system-requirements#macos).

For an iOS launch, check the available simulators:

```bash
day devices list -t ios-uikit
```

If none are available, install an iOS Simulator runtime through Xcode’s settings. If one is
available but stopped, replace `SIMULATOR_ID` with its listed ID:

```bash
day devices boot -t ios-uikit SIMULATOR_ID --wait
day launch -t ios-uikit
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
JDK. Follow the [Android setup instructions](/docs/reference/system-requirements#android), then repeat doctor.

Installing Android Studio gives you the SDK and a JDK, but not everything the build needs. These are
the errors a fresh Studio install typically produces, in the order a first build meets them:

| Error | Cause and fix |
|---|---|
| `no Android NDK found (set ANDROID_NDK_HOME)` | Studio's SDK Manager does not install an NDK by default. Check *NDK (Side by side)* under *Settings ▸ Languages & Frameworks ▸ Android SDK ▸ SDK Tools*, or run `<sdk>/cmdline-tools/latest/bin/sdkmanager --install "ndk;<version>"`. |
| `sdkmanager: command not found` | Studio keeps the command-line tools inside the SDK and off `PATH`, and installs them only when *Android SDK Command-line Tools* is checked under *SDK Tools*. Run them by full path, `<sdk>/cmdline-tools/latest/bin/sdkmanager`. `day doctor` prints the exact path when it finds one. |
| `Error on ZipFile unknown archive` while installing the emulator | `sdkmanager` could not unpack its downloaded archive. `day devices setup` retries recognized archive and transfer failures up to three times; the SDK manager verifies the previous download before reusing it. If all attempts fail, the error names the package and includes the SDK output. Check the download connection or proxy before retrying setup. An incomplete emulator directory does not count as an installed package. |
| `error: no such command: ndk` | `cargo-ndk` is a separate Cargo tool, not part of Studio or the NDK: run `cargo install cargo-ndk`. |
| `SDK location not found. Define a valid SDK location with an ANDROID_HOME environment variable…` | Older `day` releases did not pass the SDK they found on to Gradle. Update `day`, or set `ANDROID_HOME` to your SDK. |
| Gradle fails with an unsupported Java or class file version | The system `java` is newer (or older) than the Gradle build supports (17 through 26). With `JAVA_HOME` unset, Day uses Android Studio's bundled JDK when it can find Studio, and `day doctor` reports "(Android Studio's bundled JDK)" when it does. For a Studio install outside the usual locations, set `ANDROID_STUDIO_HOME` to it, or point `JAVA_HOME` at a JDK 17 through 26. |
| The emulator exits with `Cannot find AVD system path. Please define ANDROID_SDK_ROOT` | The AVD's system image is not installed. The SDK location is fine; an AVD stays listed after its image is removed or the SDK is replaced. `day devices boot` now names the missing package. Install it from Studio's SDK Manager (*SDK Platforms*, with *Show Package Details*), or with `sdkmanager --install "system-images;android-34;google_apis;x86_64"` using the name it reports. |
| `ANDROID_HOME` unset but the SDK is not in the default location | Day reads the SDK location that Android Studio's own settings record (*Settings ▸ Languages & Frameworks ▸ Android SDK*), so an SDK moved there is found. Set `ANDROID_HOME` to override. |

If Android Studio's Gradle sync reports "The project is using an incompatible version (AGP 9.4.0)
of the Android Gradle plugin", update Android Studio to 2026.1.4 or newer. Day's Gradle plugin
builds with that AGP release, which older Android Studio versions cannot sync. `day build` works
either way.

For a launch failure:

```bash
day devices list -t android-mdc
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

You can also boot an existing AVD with `day devices boot -t android-mdc AVD_NAME --wait`, using
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
newer; the Qt backend requires Qt 6. [Linux requirements](/docs/reference/system-requirements#linux) lists the
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

`windows-winui` needs the MSVC toolchain, Visual Studio C++ Build Tools, and the Windows SDK.
A missing `link.exe` points to that setup. Windows GTK and Qt builds use MSYS2 packages and a
GNU-compatible Rust toolchain; mixing their import libraries with MSVC causes linking failures.
Follow the [Windows setup instructions](/docs/reference/system-requirements#windows) for your backend and
host architecture, then run its focused doctor check in the same terminal.

A `windows-winui` development build that links but offers a download page at start-up is missing
the Windows App Runtime 2.5.1 or newer; `day doctor --toolkit winui` reports it.

## HarmonyOS builds Rust but cannot package or launch

A successful Rust build only confirms the native compilation tools are available. Packaging
also needs `hvigor` and `ohpm`; installation and launch need `hdc` and a reachable device or
emulator.

```bash
day doctor --toolkit harmonyos
day devices list -t harmony-arkui
```

Check which SDK `cargo ohos env` resolves (`error: no such command: ohos` means `cargo-ohos` is
not installed: `cargo install --locked cargo-ohos`) and the command-line tools installation
against the [HarmonyOS requirements](/docs/reference/system-requirements#harmonyos). If doctor passes but the device list
is empty, finish the emulator or device setup before retrying the launch.

These emulator failures have specific causes:

- **The boot falls back to slow TCG on Linux.** `/dev/kvm` exists but this user can't open
  it. Add yourself to the `kvm` group (`sudo usermod -aG kvm $USER`), then log in again.
- **`day launch` installs the app but retries `aa start` until it gives up with error 10106102**
  ("the device screen is locked"). The lock screen was never dismissed. `day launch` swipes it
  away with a synthetic gesture sized to the emulator's screen; if the lock screen persists,
  swipe up in the emulator window yourself, or start the installed app from its launcher icon.
  Earlier versions of `day` aimed that swipe at the requested panel and missed on a windowed
  Linux emulator, whose screen followed its GTK window (640×480).
- **The emulator window is blank, reads "Display output is not active", or runs at 640×480
  whatever `--device` says.** That is QEMU's GTK window, which hands the guest its own size.
  Current versions of `day devices boot` open an SDL window, as the Oniro image's `run.sh` does,
  and refuse a QEMU built without SDL (Homebrew's). Install your distribution's QEMU:
  `sudo apt install qemu-system-x86 qemu-system-gui`, and remove other builds from `PATH`.
- **Over a remote-desktop connection, the emulator's pointer doesn't follow yours** (it stays
  in the middle of the screen, or jumps to where you entered the window and stops there) while
  clicks and scroll gestures still work. With the OpenHarmony 7.0 (ohos-qemu) image, current
  versions of `day devices boot` fix this: they give the guest an absolute pointer and keep your
  desktop cursor visible over the window (`show-cursor=on`). Hidden, the cursor is what makes an
  RDP client such as Thincast switch to relative "game" mouse input, which the window never
  receives. Update `day` and boot again.

  The Oniro 6.1 image can't be fixed that way: its input service doesn't map an absolute pointer
  to the screen, and its relative mouse moves only while QEMU's window has grabbed your pointer,
  which a remote-desktop session doesn't support. Boot it with the image's own launcher in its
  headless mode instead, and view it over VNC, where QEMU turns your pointer positions into
  motion itself:

  ```bash
  bash ~/ohos/emulator/images/run.sh --headless   # VNC on port 5900, hdc on 127.0.0.1:55555
  remmina -c vnc://127.0.0.1:5900                  # or any VNC viewer
  hdc tconn 127.0.0.1:55555
  day launch -t harmony-arkui
  ```

  `run.sh` serves VNC on all network interfaces with no password, so anyone who can reach this
  machine on port 5900 can see and use the emulator; firewall the port on a shared network. Add
  `-r 1280x800` for a tablet screen.
- **Every `hdc` command hangs.** The guest is asleep, perhaps after `power-shell suspend`,
  which suspends `hdcd` too. Restart the emulator.
- **`day devices boot` times out, and `hdc` answers `Bind tartget session is dead`** (the typo is
  hdc's), although `hdc list targets` shows the emulator as `Connected`. The emulator has booted;
  the `hdc` server on your machine is holding a session from an earlier emulator. Restart the
  server and reconnect with `hdc kill -r` and then `hdc tconn 127.0.0.1:55555`.
- **The emulator window shows a 640×480 landscape screen although you asked for a phone.** On
  Linux, QEMU's GTK window hands the guest its own 640×480 size, and the guest adopts it over the
  requested panel. `day devices boot -t harmony-arkui --device phone --headless` keeps the 360×720
  panel, at the cost of having no window; `hdc shell snapshot_display` and `uinput` then reach
  the screen.
- **`day launch` ends by itself, or `day drive` reports no live session, after a first drive.**
  Older versions of `day` re-added the dayscript forward on every drive; hdc refuses a duplicate
  with `[Fail]TCP Port listen failed`, and `day` recycled the hdc server in response, which
  dropped every forward and ended the launch's log stream. Update `day`, then relaunch.
- **The network status reading is unavailable on the emulator.** [`day_part_connectivity::status()`](https://daybrite.dev/api/day_part_connectivity/fn.status.html)
  returns `None`, and Day-Showcase reads "Connectivity unavailable". The device log (`hdc shell
  hilog -x`) shows `IPCObjectStub: OnRemoteRequest: unknown code:12 desc:*.INetConnService`
  at the same moment. The emulator image's network-management service does not answer the
  NetConn C API, so every call fails with 201 even though the app holds
  `ohos.permission.GET_NETWORK_INFO`. Test connectivity on a real device.

If every feature that calls into ArkTS (HTTP, resources, permission prompts) fails with
`platform runtime unavailable` on HarmonyOS, update `day` and the app's Day dependency. Older
releases looked their ArkTS bridge up in the global symbol table, which cannot see `libentry.so`:
HarmonyOS loads it with local symbol visibility.

## Signing or provisioning fails

A simulator build can succeed while a device build or release package fails to sign. From your
project directory, check the signing configuration:

```bash
day sign check
```

This checks whether configured environment variables and files can be resolved. It does not
prove that a certificate or provisioning profile is valid for the app, device, and distribution
method. Read the signing tool’s error, then check the app identifier, team, certificate, and
profile it names. [Packaging and distribution](/docs/how-to/packaging) documents Day’s signing settings.
Do not post private keys, passwords, or signing credentials when asking for help.

## The web build does not open correctly

Use Day’s local server rather than opening the generated HTML through a `file://` URL:

```bash
day launch -t web-dom
```

If compilation fails, run `day doctor --toolkit dom` and check that the wasm Rust target is
installed. If the page loads but the app does not start, inspect the browser’s console and network
panel for failed JavaScript or WebAssembly requests. See the
[web platform guide](/docs/reference/platforms/web-dom) for build and hosting details.

These are the errors you may meet running the web build's tests and checks:

| Error | Cause and fix |
|---|---|
| A scripted run (`day launch -t web-dom --script …`) fails at its first `screenshot` step | A page cannot screenshot itself; the runner needs the headless browser driver. Install Playwright and set `DAY_WEB_DRIVER`, `DAY_WEB_DRIVER_PLAYWRIGHT` and `DAY_WEB_DRIVER_BROWSER` as the [web platform guide](/docs/reference/platforms/web-dom#running-dayscripts) shows. Interactive `day launch` never needs it. |
| A scripted web run stops with `engine connection lost` after the driver prints `Playwright requires Node.js 20 or higher` | An older `node` is first on `PATH`. The OpenHarmony command-line tools bundle Node 18, so sourcing their environment (for `harmony-arkui`) shadows a newer system Node. Run the web script from a shell without it, or name the Node to use in the driver command: `DAY_WEB_DRIVER="/path/to/node $(day web driver)"`. |
| File-storage steps fail under WebKit on Linux | Playwright's Linux WebKit has no Origin Private File System, which [`day-part-fs`](/docs/reference/fs) needs. Set `DAY_WEB_DRIVER_BROWSER=chromium`. |
| `cargo check --target wasm32-unknown-unknown` on a Day crate fails in `getrandom` with "The wasm32/64-unknown-unknown are not supported by default" | `day build` routes getrandom to day-dom's entropy bridge with a cfg flag that plain `cargo` does not pass. Pass it yourself: `RUSTFLAGS='--cfg getrandom_backend="custom"' cargo check --target wasm32-unknown-unknown -p <crate>`. The `wasm_js` backend that getrandom's error message suggests needs the wasm-bindgen runtime, which Day's web build does not use. |
| A plain `cargo check` of `day-dom` passes but the web build fails | `day-dom` compiles only for `wasm32` (the whole crate is `#![cfg(target_arch = "wasm32")]`), so a host check compiles nothing. Check it for the web target as above. |

## The app starts but does not work as expected

Keep the launch terminal open and look for a panic or platform error when the problem happens.
[Logging](/docs/how-to/logging) explains where Day sends app logs; [crash reporting](/docs/how-to/guide-crash-reporting)
covers collecting failures from deployed apps.

If only one feature fails, check its platform support and permissions. For example, a web view
placeholder on a desktop target may mean its optional engine was not included; see
[web view requirements](/docs/reference/system-requirements#optional-web-views). Camera and other protected
features may also need [permission configuration](/docs/how-to/guide-permissions).

Two [`.searchable()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.searchable) symptoms come from older Day releases and are fixed by updating `day` and the
app's Day dependency:

- **The toolbar search field loses focus after one letter, or clearing it leaves the list
  filtered** (GTK, AppKit, Qt). A letter that changed the selected page used to rebuild the whole
  toolbar, and the search field with it. The toolbar is now updated in place, and the field keeps
  its focus and text ([commands and toolbars](/docs/how-to/guide-commands)).
- **An Android app shows no search field.** The field goes above the navigation list on Android.
  An intermediate release stopped installing it there.

## A video plays as a black rectangle on Linux

On `linux-gtk`, the media piece draws with `GtkVideo`, which hands the file to GStreamer. When
GStreamer has no decoder for the video's codec, nothing reaches the widget. Current versions of
the media piece then show GTK's error icon over the video, and hovering it names the missing
decoder; older ones leave the video plain black. Either way, GStreamer can say what the file
needs:

```bash
gst-discoverer-1.0 https://example.com/video.mp4
```

A "Missing plugins" line names the gap. For the usual MP4 (H.264 video, AAC audio) on
Debian or Ubuntu, install the decoders, then restart the app:

```bash
sudo apt install gstreamer1.0-libav
```

[Optional: media playback](/docs/reference/system-requirements#optional-media-playback) lists the full
set. `libEGL warning: failed to get driver name for fd -1` and `MESA: error: ZINK: failed to
choose pdev` in the same console are a separate, harmless matter: the app can't open the GPU
(`/dev/dri/renderD128`), so GTK renders in software; everything else still draws. It happens
when the device grants access only to the `render` group and the desktop's own seat user, as in a
remote session. Adding yourself to the group (`sudo usermod -aG render $USER`, then log in
again) gives the app the GPU.

## An app is silent on Linux

[`day-part-sound`](/docs/reference/sound) reaches the sound server through libpulse, which serves PulseAudio and
PipeWire alike. Three things can leave it silent:

- **libpulse is not installed.** [`day_part_sound::is_supported()`](https://daybrite.dev/api/day_part_sound/fn.is_supported.html) is false. This is usual in a
  minimal container. Install `libpulse0` (Debian, Ubuntu) or `pulseaudio-libs` (Fedora).
- **No sound server is running.** The library loads but nothing answers, which is the case on
  a CI runner. `pactl info` reports the server when there is one. On a PipeWire desktop the
  `pipewire-pulse` service provides it.
- **A Flatpak built by hand lacks the audio socket.** `day pack` adds `--socket=pulseaudio` for
  any app that depends on the part. A manifest written by hand needs the same line under
  `finish-args`.

Run with `DAY_LOG=debug` to see each `play` and `loop` call arrive, and a
`no sound server answered` line when the connection fails.

## A dayscript run fails on Linux

These come from the desktop a scripted run (`day launch -t linux-gtk --script …`) runs on, not from
the script:

| Symptom | Cause and fix |
|---|---|
| `engine connection lost (could not connect to the dayscript engine …)` right after `lifecycle: WillLaunch` (linux-gtk) | Another copy of the app is already running. A GTK app is single-instance, so the new launch hands itself to the running copy and exits. Quit the other copy first. |
| Every `screenshot` step fails with `ui transitions still settling` (linux-gtk) | The app's window never draws a new frame, which GNOME does for a window that is covered or offscreen on Wayland. Keep the window visible, or run it as an X11 window with `GDK_BACKEND=x11` in the environment of `day launch`. |
| The app crashes on a web view page; its output shows `bwrap: setting up uid map: Permission denied` and `Failed to fully launch dbus-proxy` (linux-gtk) | WebKitGTK runs web content in a bubblewrap sandbox, which needs unprivileged user namespaces, and Ubuntu 24.04's AppArmor restricts them (`sysctl kernel.apparmor_restrict_unprivileged_userns` reads `1`). For a local test run only, pass `--env WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1` to `day launch`. |
| Web view steps fail with `QtWebEngine: no Qt6 WebEngine in this build` (linux-qt) | Qt WebEngine is optional and not installed. Install `qt6-webengine-dev` ([optional web views](/docs/reference/system-requirements#optional-web-views)) and rebuild. |

## Still stuck?

Try the smallest app that reproduces the problem on one target. When you
[report an issue](https://github.com/daybrite/day/issues), include:

- The command you ran and the first relevant error, with enough surrounding output to identify it.
- Your development OS and architecture, target, and device or emulator details.
- `day --version`, `rustc --version`, and the focused `day doctor` output.
- A small reproduction or the source revision and steps needed to reproduce the failure.

Remove credentials and personal information from logs before sharing them. Mention whether a
newly created app fails too; that helps distinguish machine setup from a project-specific problem.

## Diagnose contract mismatches

| Symptom | Check |
| --- | --- |
| A label stays at its initial value | Pass a signal or tracked closure instead of a value read during construction |
| Read of disposed signal | Move shared state above the page/window scope or cancel the callback that outlives it |
| `Signal` fails a `Send` bound | Use `Setter` for worker delivery; [`on_main`](https://daybrite.dev/api/day_reactive/fn.on_main.html) also requires a `Send` closure |
| A request succeeds but returns an error page | Check `Response::status`; HTTP 4xx/5xx are responses, not transport failures |
| Leaving a page does not stop work | [`day::task`](https://daybrite.dev/api/day/fn.task.html) is not scope-owned; abort its handle or use a `Resource` |
| Permission status is `Unknown` | Await `status_future` instead of relying on the first synchronous cache read |
| A generated resource name exists but loading fails | Check staging and runtime resource availability separately from code generation |

See [reactivity](/docs/explanation/reactivity), [HTTP](/docs/how-to/guide-http),
[permissions](/docs/how-to/guide-permissions), and [resources](/docs/how-to/resources).
These checks are based on the current source contracts. SDK-specific remediation and external
media-extension behavior above remain unverified on targets not exercised in the phase report.
