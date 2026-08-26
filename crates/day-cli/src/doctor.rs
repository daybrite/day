// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day doctor — development-environment diagnosis, grouped by toolkit (DESIGN.md §16.5).
//!
//! Default (`day doctor`): checks the core toolchain plus every toolkit buildable on this host. A
//! missing OPTIONAL toolkit dependency is a WARNING (yellow) and doctor still exits 0 — you only need
//! the toolkits you actually build. Core (rust) failures are always errors.
//!
//! Focused (`day doctor --toolkit qt --toolkit android`): the named toolkits' checks become hard
//! ERRORS (a missing piece exits non-zero), and detailed per-OS setup instructions are printed for
//! each requested toolkit. This is what CI uses so a build job fails loudly on a misconfigured
//! environment instead of deep inside cargo/gradle/hvigor.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::targets::host_os;

/// What a missing probe blocks. Only [`Need::Build`] is ever an error: everything else degrades a
/// stage that either still works without it (the resource compilers) or isn't part of compiling at
/// all (packaging tools, a booted device). `day checkup` reads the same field to decide which
/// toolkits it can build and which it can package (see [`readiness`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Need {
    /// Required to compile for this toolkit — a miss is an error when the toolkit is focused.
    Build,
    /// Build-time but optional: the build still produces a working app, degraded (a skipped
    /// resource blob, a Swift contribution that no dependency makes).
    BuildOptional,
    /// Required by `day pack` for this toolkit's formats — never needed to build.
    Pack,
    /// Packaging still succeeds without it, with a less portable artifact (the linuxdeploy
    /// plugins: without one the AppImage needs a machine that already has the toolkit).
    PackOptional,
    /// A launch-time prerequisite (a booted simulator/emulator, hdc) — not needed to compile.
    Launch,
}

/// One environment probe: a label, the resolved detail (`Some` = found), a one-line fix hint, and
/// what its absence blocks.
struct Probe {
    name: &'static str,
    detail: Option<String>,
    fix: String,
    need: Need,
}

impl Probe {
    fn new(name: &'static str, detail: Option<String>, fix: impl Into<String>) -> Self {
        Probe {
            name,
            detail,
            fix: fix.into(),
            need: Need::Build,
        }
    }
    /// Reclassify: everything but [`Need::Build`] reports as a warning rather than an error.
    fn need(mut self, need: Need) -> Self {
        self.need = need;
        self
    }
    /// Whether a miss stays a warning even when the toolkit is focused.
    fn soft(&self) -> bool {
        self.need != Need::Build
    }
}

/// A toolkit's diagnosis: id (matches `--toolkit`), label, the hosts that can build it, its probes,
/// and multi-line setup instructions printed when the toolkit is focused.
struct Group {
    id: &'static str,
    label: &'static str,
    /// Hosts this toolkit builds on (`macos`/`linux`/`windows`), or `["any"]` for cross-compiled.
    hosts: &'static [&'static str],
    probes: Vec<Probe>,
    setup: &'static str,
}

impl Group {
    fn builds_on(&self, host: &str) -> bool {
        self.hosts == ["any"] || self.hosts.contains(&host)
    }
}

// --- probe helpers ---------------------------------------------------------

/// First stdout line of `cmd args` if it exits 0, else `None`. Used to prove a tool runs.
fn run_line(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd).args(args).output().ok().and_then(|o| {
        o.status.success().then(|| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
    })
}

/// Full stdout of a command (not just the first line) — for probes that must scan multi-line
/// output, e.g. `rustc -vV`'s `host:` line.
fn run_out(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd).args(args).output().ok().and_then(|o| {
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    })
}

/// `Some(dir)` if `dir` exists and is a directory — for env-var / SDK-path probes.
fn existing_dir(dir: &Path) -> Option<String> {
    dir.is_dir().then(|| dir.display().to_string())
}

/// Whether a rustup toolchain has `triple`'s std installed (mirrors what cross-compiles need).
fn have_rust_target(triple: &str) -> Option<String> {
    run_line("rustc", &["--print", "target-list"])?; // rustc present at all?
    let out = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
    out.status.success().then_some(())?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.trim() == triple)
        .then(|| triple.to_string())
}

/// The first of `triples` whose std is installed. Android/OHOS builds pick an arch by device vs
/// emulator, so having EITHER arch installed is enough to prove the toolchain is set up.
fn have_any_rust_target(triples: &[&str]) -> Option<String> {
    triples.iter().find_map(|t| have_rust_target(t))
}

/// The JDK the Gradle build will use, if it's a version AGP accepts (17 or newer — AGP 9's
/// minimum). Resolves via `day_toolchain::jdk_home()` — the SAME `$JAVA_HOME`-first resolution the
/// gradle builds use, so doctor diagnoses what the build will actually run. Because the build
/// TRUSTS `$JAVA_HOME`, a `$JAVA_HOME` pointing at a too-old JDK is a real miss even when a newer
/// one is installed elsewhere. The major version is parsed from `java -version` (which prints
/// `openjdk version "26.0.1" …` — or bare `"21"` — to stderr).
fn have_jdk() -> Option<String> {
    let java = day_toolchain::jdk_home()?.join("bin").join("java");
    let out = Command::new(&java).arg("-version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stderr);
    // Major version = the first whitespace/quote-delimited token whose leading `N.` or bare `N`
    // parses (`"26.0.1"` → 26, `"21"` → 21). Modern JDKs report the feature version directly.
    let major = text
        .split(|c: char| c.is_whitespace() || c == '"')
        .filter(|t| !t.is_empty())
        .find_map(|t| t.split(['.', '-', '_']).next()?.parse::<u32>().ok())?;
    (major >= 17).then(|| text.lines().next().unwrap_or("").trim().to_string())
}

/// The C compiler the web build's SQLite compile will use — [`day_toolchain::wasm_cc`], the
/// SAME resolution `day build -p web-dom` applies, so doctor reports what the build will run.
/// A set cc-rs variable is the one case that still gets probed here: the build honors it
/// blindly, and doctor's job is to say whether that program can actually emit wasm.
fn have_wasm_cc() -> Option<String> {
    match day_toolchain::wasm_cc() {
        day_toolchain::WasmCc::Env(program) => day_toolchain::emits_wasm32(Path::new(&program))
            .then(|| format!("{program} (from a CC variable; wasm32 backend)")),
        day_toolchain::WasmCc::PathClang => Some("clang (wasm32 backend)".to_string()),
        day_toolchain::WasmCc::Fallback(cc) => Some(format!("{} (auto-selected)", cc.display())),
        day_toolchain::WasmCc::Missing => None,
    }
}

/// Resolve `bin` on PATH (like the shell would); `Some(path)` if found. `bin` may carry `.exe`; on
/// Windows a bare name also matches `<bin>.exe` (else e.g. `glib-compile-resources.exe` reads as
/// missing).
fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) && !bin.ends_with(".exe") {
        vec![bin.to_string(), format!("{bin}.exe")]
    } else {
        vec![bin.to_string()]
    };
    std::env::split_paths(&path).find_map(|dir| {
        names.iter().find_map(|name| {
            let p = dir.join(name);
            p.is_file().then_some(p)
        })
    })
}

/// Locate Qt's `rcc` (the resource compiler used by §18.3 staging) the same way the stager does:
/// Qt's qmake-queried libexec / host-bins, then PATH.
fn find_rcc() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["rcc.exe", "rcc"]
    } else {
        &["rcc"]
    };
    for qmake in ["qmake6", "qmake"] {
        for var in ["QT_INSTALL_LIBEXECS", "QT_HOST_BINS"] {
            if let Some(dir) = run_line(qmake, &["-query", var]) {
                for name in names {
                    let p = Path::new(&dir).join(name);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    names.iter().find_map(|n| which(n))
}

// --- toolkit groups --------------------------------------------------------

fn core_group() -> Group {
    Group {
        id: "core",
        label: "Core toolchain",
        hosts: &["any"],
        probes: vec![Probe::new(
            "rust",
            run_line("cargo", &["--version"]),
            "install Rust via https://rustup.rs (rustup) or `brew install rust`",
        )],
        setup: "Install the Rust toolchain from https://rustup.rs, or `brew install rust`. Cross-\n\
                compiled targets (iOS/Android/OpenHarmony) additionally need the rustup-managed\n\
                toolchain — Homebrew's rustc ships no cross std.",
    }
}

fn appkit_group() -> Group {
    Group {
        id: "appkit",
        label: "macOS · AppKit",
        hosts: &["macos"],
        probes: vec![
            Probe::new(
                "xcode-clang",
                run_line("xcrun", &["--find", "clang"]),
                "install the Xcode command-line tools: `xcode-select --install`",
            ),
            Probe::new(
                "swift",
                run_line("swift", &["--version"]),
                "install Xcode or the command-line tools — needed only when a dependency embeds \
                 Swift/SwiftUI (docs/swiftui.md)",
            )
            .need(Need::BuildOptional),
        ],
        setup: "macOS desktop (AppKit) needs Apple's clang toolchain: `xcode-select --install`\n\
                (or a full Xcode). No extra Rust target — the host toolchain builds it. An app\n\
                carrying `platform/macos/DayApp.xcodeproj` builds through xcodebuild, which needs\n\
                a full Xcode; `DAY_MACOS_XCODE=0` (or no scaffold) falls back to the bare cargo\n\
                build, where the command-line tools are enough. When a dependency contributes\n\
                macOS Swift (SwiftUI embedding, docs/swiftui.md), that path adds a `swift build`\n\
                prepass and links the result statically — it also needs the `swift` compiler.",
    }
}

fn uikit_group() -> Group {
    Group {
        id: "uikit",
        label: "iOS · UIKit",
        hosts: &["macos"],
        probes: vec![
            Probe::new(
                "xcode",
                run_line("xcodebuild", &["-version"]),
                "install Xcode from the App Store (the iOS build drives xcodebuild)",
            ),
            Probe::new(
                "rust-ios-sim",
                have_rust_target("aarch64-apple-ios-sim"),
                "rustup target add aarch64-apple-ios-sim",
            ),
            Probe::new(
                "simulator",
                run_line(
                    "bash",
                    &["-c", "xcrun simctl list devices booted | grep -m1 Booted"],
                ),
                "boot a simulator: `xcrun simctl boot <device>` (or open Simulator.app)",
            )
            .need(Need::Launch),
        ],
        setup: "iOS (UIKit) cross-compiles via an Xcode script phase and runs on the Simulator.\n\
                Needs: full Xcode (`xcode-select -s /Applications/Xcode.app`), the simulator Rust\n\
                target `rustup target add aarch64-apple-ios-sim`, and a booted simulator to launch\n\
                (`xcrun simctl boot <device>`). iOS builds only on a macOS host.",
    }
}

/// The GTK stack day-gtk compiles against. `toolkits/day-gtk/Cargo.toml` enables the `gtk4`
/// crate's `v4_10` feature and libadwaita's `v1_5`, and those features are exactly what the `-sys`
/// crates hand to pkg-config as a minimum. Kept in step with that manifest by
/// `gtk_minimums_match_day_gtk` below.
const GTK4_MIN: (u32, u32) = (4, 10);
const LIBADWAITA_MIN: (u32, u32) = (1, 5);

/// Is `found` — a pkg-config `--modversion` string like `4.8.3` — at least `min`?
///
/// Major and minor only: every minimum day states is a feature level, and those land on minor
/// releases. A trailing packaging suffix (`4.8.3-1`) is ignored, and a version too malformed to
/// read counts as too old rather than as good enough.
fn version_at_least(found: &str, min: (u32, u32)) -> bool {
    let mut parts = found.trim().split(['.', '-', '~', '+']);
    let Some(major) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    (major, minor) >= min
}

/// A pkg-config module held to a minimum version.
///
/// Present-but-too-old is the case worth separating out: "install GTK 4" is useless advice for
/// someone who has GTK 4 and needs a newer one. Debian 12 ships gtk4 4.8.3, and without this the
/// first sign of trouble is a pkg-config wall from inside `gdk4-sys`, minutes into a build that
/// `day doctor` had already called healthy.
fn pkg_probe(name: &'static str, module: &str, min: (u32, u32), install: &str) -> Probe {
    match run_line("pkg-config", &["--modversion", module]) {
        Some(found) if version_at_least(&found, min) => {
            Probe::new(name, Some(found), install.to_string())
        }
        Some(found) => Probe::new(
            name,
            None,
            format!(
                "{module} is {found}; day needs {}.{} or newer — {install}",
                min.0, min.1
            ),
        ),
        None => Probe::new(name, None, install.to_string()),
    }
}

fn gtk_group() -> Group {
    Group {
        id: "gtk",
        label: "GTK 4 · libadwaita",
        hosts: &["macos", "linux", "windows"],
        probes: vec![
            pkg_probe(
                "gtk4",
                "gtk4",
                GTK4_MIN,
                "install GTK 4 (`brew install gtk4` · `apt install libgtk-4-dev` · MSYS2 mingw-w64-gtk4)",
            ),
            pkg_probe(
                "libadwaita",
                "libadwaita-1",
                LIBADWAITA_MIN,
                "install libadwaita (`brew install libadwaita` · `apt install libadwaita-1-dev`)",
            ),
            // Optional: resource staging (§18.3) is best-effort — a missing `glib-compile-resources`
            // just skips the gresource blob and day loads images from the filesystem roots. So a
            // miss is a warning, not an error (MSYS2 windows-gtk doesn't ship it on PATH).
            Probe::new(
                "glib-compile-resources",
                which("glib-compile-resources").map(|p| p.display().to_string()),
                "install glib tools (bundled with glib/GTK; ships `glib-compile-resources`)",
            )
            .need(Need::BuildOptional),
            // Only `day pack -p linux-gtk` (the .flatpak bundle, §16.5) needs it.
            Probe::new(
                "flatpak-builder",
                which("flatpak-builder").map(|p| p.display().to_string()),
                "install flatpak + flatpak-builder and add the flathub remote (for `day pack`)",
            )
            .need(Need::Pack),
            // The OTHER half of `day pack -p linux-gtk`: the .appimage (§16.5). Without the gtk
            // plugin an AppImage still builds, but carries no GdkPixbuf loaders or GSettings
            // schemas — so both are probed, and the plugin is the optional one.
            Probe::new(
                "linuxdeploy",
                crate::pack::appimage_tool_probe("linuxdeploy"),
                "download linuxdeploy from github.com/linuxdeploy/linuxdeploy/releases (for `day pack` → .appimage)",
            )
            .need(Need::Pack),
            Probe::new(
                "linuxdeploy-plugin-gtk",
                crate::pack::appimage_tool_probe("linuxdeploy-plugin-gtk"),
                "download linuxdeploy-plugin-gtk — without it the AppImage needs a machine that already has GTK",
            )
            .need(Need::PackOptional),
        ],
        setup: "GTK 4 builds on macOS, Linux, and Windows via pkg-config. Day needs gtk4 4.10 or\n\
                newer and libadwaita 1.5 or newer — it builds navigation on AdwNavigationView and\n\
                AdwOverlaySplitView and dialogs on GtkFileDialog/GtkAlertDialog, none of which\n\
                exist below those versions. A distribution that ships an older GTK (Debian 12 has\n\
                gtk4 4.8) cannot build this target; use `-p linux-qt` there, or a newer runtime.\n\
                Install the dev libraries:\n\
                • macOS  — `brew install gtk4 libadwaita pkg-config`\n\
                • Linux  — `apt install libgtk-4-dev libadwaita-1-dev pkg-config`\n\
                • Windows— MSYS2: `pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita`\n\
                  (ARM64 hosts: the CLANGARM64 environment's `mingw-w64-clang-aarch64-` packages),\n\
                  plus a GNU Rust toolchain — MSVC cannot link MSYS2's import libraries:\n\
                  `rustup toolchain install stable-x86_64-pc-windows-gnu` (ARM64:\n\
                  `stable-aarch64-pc-windows-gnullvm`), then build with MSYS2's bin on PATH and\n\
                  RUSTUP_TOOLCHAIN set to it.\n\
                `glib-compile-resources` (ships with glib) compiles bundled resources (§18.3); without\n\
                it images fall back to loose files.",
    }
}

fn qt_group() -> Group {
    Group {
        id: "qt",
        label: "Qt 6 Widgets",
        hosts: &["macos", "linux", "windows"],
        probes: vec![
            Probe::new(
                "qt6-widgets",
                run_line("pkg-config", &["--modversion", "Qt6Widgets"])
                    .or_else(|| run_line("qmake6", &["-query", "QT_VERSION"]))
                    .or_else(|| run_line("qmake", &["-query", "QT_VERSION"])),
                "install Qt 6 (`brew install qt` · `apt install qt6-base-dev` · MSYS2 mingw-w64-qt6-base)",
            ),
            // Optional: like glib-compile-resources, `rcc` staging is best-effort — a miss skips the
            // qresource blob (day loads images from the filesystem roots), so it's a warning, not an
            // error (MSYS2 windows-qt doesn't ship `rcc` on PATH).
            Probe::new(
                "rcc",
                find_rcc().map(|p| p.display().to_string()),
                "install Qt 6 (rcc, the resource compiler, ships in Qt's libexec)",
            )
            .need(Need::BuildOptional),
            // Only `day pack -p linux-qt` (the .flatpak bundle, §16.5) needs it.
            Probe::new(
                "flatpak-builder",
                which("flatpak-builder").map(|p| p.display().to_string()),
                "install flatpak + flatpak-builder and add the flathub remote (for `day pack`)",
            )
            .need(Need::Pack),
            // The OTHER half of `day pack -p linux-qt`: the .appimage (§16.5). Without the qt
            // plugin the image carries no platform plugin, so it cannot open a window on a machine
            // without Qt — hence probing the plugin, not just the tool.
            Probe::new(
                "linuxdeploy",
                crate::pack::appimage_tool_probe("linuxdeploy"),
                "download linuxdeploy from github.com/linuxdeploy/linuxdeploy/releases (for `day pack` → .appimage)",
            )
            .need(Need::Pack),
            Probe::new(
                "linuxdeploy-plugin-qt",
                crate::pack::appimage_tool_probe("linuxdeploy-plugin-qt"),
                "download linuxdeploy-plugin-qt — without it the AppImage needs a machine that already has Qt",
            )
            .need(Need::PackOptional),
        ],
        setup: "Qt 6 Widgets builds on macOS, Linux, and Windows. Install Qt 6 and pkg-config:\n\
                • macOS  — `brew install qt pkg-config`\n\
                • Linux  — `apt install qt6-base-dev qt6-webengine-dev pkg-config`\n\
                • Windows— MSYS2: `pacman -S mingw-w64-x86_64-qt6-base` (ARM64 hosts: the\n\
                  CLANGARM64 environment's `mingw-w64-clang-aarch64-qt6-base`), plus a GNU Rust\n\
                  toolchain — MSVC cannot link MSYS2's import libraries, and the C++ shim is built\n\
                  from pkg-config's flags, which an aqtinstall/online-installer Qt does not ship:\n\
                  `rustup toolchain install stable-x86_64-pc-windows-gnu` (ARM64:\n\
                  `stable-aarch64-pc-windows-gnullvm`), then build with MSYS2's bin on PATH and\n\
                  RUSTUP_TOOLCHAIN set to it.\n\
                `rcc` (Qt's resource compiler, §18.3) is resolved from qmake's libexec; a missing Qt\n\
                means both the build and bundled-resource staging fail.",
    }
}

fn xaml_group() -> Group {
    Group {
        id: "xaml",
        label: "Windows · XAML",
        hosts: &["windows"],
        probes: vec![
            Probe::new(
                "msvc-toolchain",
                // The default rustc must target *-windows-msvc (xaml builds with cl.exe + the SDK).
                // Scan the FULL `rustc -vV` output for the `host:` line — `run_line` returns only line 1
                // (`rustc <version>`), which is why the old check false-negatived on a valid msvc host
                // (and its `bash`+`grep` fallback isn't reliably resolvable from a native process).
                run_out("rustc", &["-vV"]).and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("host: "))
                        .filter(|h| h.contains("windows-msvc"))
                        .map(str::to_string)
                }),
                "rustup default stable-msvc + install the VS 2022 C++ Build Tools",
            ),
            // Only `day pack -p windows-xaml` needs these (§16.5) — makeappx/signtool ship with
            // the Windows SDK, makensis via `choco install nsis`.
            Probe::new(
                "makeappx (Windows SDK)",
                crate::pack::windows_kit_tool_probe("makeappx.exe"),
                "install the Windows 10/11 SDK (for `day pack` msix)",
            )
            .need(Need::Pack),
            Probe::new(
                "makensis",
                // The SAME lookup `day pack` uses (DAY_MAKENSIS → PATH → %ProgramFiles%\NSIS →
                // chocolatey), not a PATH-only `which`: a bare `which` reports missing for the
                // usual `choco install nsis`, whose shim directory a running process's PATH does
                // not pick up — so doctor would contradict the pack that then succeeds, or miss
                // the one that then fails.
                day_toolchain::makensis().map(|p| p.display().to_string()),
                "choco install nsis (for `day pack` setup.exe)",
            )
            .need(Need::Pack),
        ],
        setup: "XAML builds on a Windows host with the MSVC toolchain. Install:\n\
                • the Visual Studio 2022 C++ Build Tools (MSVC + Windows SDK)\n\
                • the MSVC Rust toolchain: `rustup default stable-msvc`\n\
                No runtime installer is needed: Day uses system XAML (in Windows 10/11), not\n\
                the Windows App SDK. XAML cannot build off a Windows host.",
    }
}

fn android_group() -> Group {
    let sdk = crate::mobile::android_sdk_dir();
    let ndk = crate::mobile::find_ndk().ok();
    let adb = sdk.join("platform-tools/adb");
    Group {
        id: "android",
        label: "Android · Material",
        hosts: &["any"],
        probes: vec![
            Probe::new(
                "android-sdk",
                existing_dir(&sdk),
                "install the Android SDK and set ANDROID_HOME (Android Studio, or cmdline-tools)",
            ),
            Probe::new(
                "android-ndk",
                ndk.as_ref().and_then(|p| existing_dir(p)),
                "install an NDK via sdkmanager and/or set ANDROID_NDK_HOME",
            ),
            Probe::new(
                "rust-android",
                have_any_rust_target(&["aarch64-linux-android", "x86_64-linux-android"]),
                "rustup target add aarch64-linux-android (arm64 device/emulator) or x86_64-linux-android (x86_64 emulator)",
            ),
            Probe::new(
                "cargo-ndk",
                run_line("cargo", &["ndk", "--version"]),
                "cargo install cargo-ndk",
            ),
            Probe::new(
                "jdk",
                have_jdk(),
                "install JDK 17 or newer and point JAVA_HOME at it (`brew install openjdk@21`); the Gradle build uses $JAVA_HOME",
            ),
            Probe::new(
                "device",
                which("adb")
                    .or_else(|| adb.is_file().then_some(adb.clone()))
                    .and_then(|adb| {
                        run_line(&adb.display().to_string(), &["devices"]).and_then(|_| {
                            run_line(
                                "bash",
                                &[
                                    "-c",
                                    &format!("{} devices | grep -m1 -w device", adb.display()),
                                ],
                            )
                        })
                    }),
                "start an emulator (`emulator -avd <name>`, or Android Studio's Device Manager) or attach a device",
            )
            .need(Need::Launch),
        ],
        setup: "Android (Material Components) cross-compiles the app to a JNI .so and runs it in a\n\
                Gradle app. Install:\n\
                • the Android SDK — set ANDROID_HOME (or ANDROID_SDK_ROOT); Android Studio installs it\n\
                  at the platform default (~/Library/Android/sdk on macOS; docs/environment.md) otherwise\n\
                • an NDK — via `sdkmanager --install 'ndk;<ver>'`; set ANDROID_NDK_HOME to override\n\
                • the Android Rust target — `rustup target add aarch64-linux-android`\n\
                • `cargo install cargo-ndk`\n\
                • JDK 17 or newer — `brew install openjdk@21` (AGP 9's minimum is 17; the Gradle\n\
                  build uses $JAVA_HOME, so set it if `java` on PATH is older)\n\
                A booted emulator or attached device is needed only to launch, not to build. Create\n\
                an AVD in Android Studio's Device Manager (or `avdmanager create avd`) and start it\n\
                with `emulator -avd <name>` — `day` has no Android-emulator command of its own.",
    }
}

fn harmonyos_group() -> Group {
    let ndk = crate::ohos::find_ohos_ndk().ok();
    // hdc ships next to the NDK, in the SDK's sibling toolchains/ dir; also accept it on PATH.
    let hdc = which("hdc").or_else(|| {
        ndk.as_ref().and_then(|n| {
            let c = Path::new(n).parent()?.join("toolchains/hdc");
            c.is_file().then_some(c)
        })
    });
    Group {
        id: "harmonyos",
        label: "HarmonyOS · ArkUI",
        hosts: &["any"],
        probes: vec![
            Probe::new(
                "ohos-ndk",
                ndk.as_ref()
                    .and_then(|p| existing_dir(&Path::new(p).join("llvm/bin")).map(|_| p.clone())),
                "set OHOS_NDK_HOME to the OpenHarmony SDK's `native` dir (see docs/harmonyos.md)",
            ),
            Probe::new(
                "rust-ohos",
                have_rust_target("aarch64-unknown-linux-ohos")
                    .or_else(|| have_rust_target("x86_64-unknown-linux-ohos")),
                "rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos",
            ),
            Probe::new(
                "hvigorw",
                which("hvigorw").map(|p| p.display().to_string()),
                "install the OpenHarmony command-line-tools (hvigor); put its bin/ on PATH",
            ),
            Probe::new(
                "ohpm",
                which("ohpm").map(|p| p.display().to_string()),
                "install the OpenHarmony command-line-tools (ohpm); put its bin/ on PATH",
            ),
            Probe::new(
                "hdc",
                hdc.map(|p| p.display().to_string()),
                "hdc ships with the SDK toolchains/ dir — put it on PATH to install/launch",
            )
            .need(Need::Launch),
        ],
        setup: "HarmonyOS (ArkUI) cross-compiles a Rust cdylib (libentry.so), packages a .hap with\n\
                hvigor, signs it, and installs over hdc. Install:\n\
                • the OpenHarmony SDK `native` component — set OHOS_NDK_HOME to it (login-free: extract\n\
                  the public SDK, see docs/harmonyos.md). `hdc` lives in the sibling toolchains/ dir\n\
                • the OpenHarmony Rust targets — `rustup target add aarch64-unknown-linux-ohos\n\
                  x86_64-unknown-linux-ohos`\n\
                • hvigor + ohpm — from the OpenHarmony command-line-tools (bundled with DevEco Studio);\n\
                  put their bin/ on PATH. These package the .hap and are not part of the public SDK.\n\
                An OpenHarmony emulator (Oniro) or device is needed only to launch, not to build —\n\
                start the bundled Oniro emulator with `day ohos emulator launch`.",
    }
}

fn dom_group() -> Group {
    Group {
        id: "dom",
        label: "Web · DOM",
        hosts: &["any"],
        probes: vec![
            Probe::new(
                "rust-wasm",
                have_rust_target("wasm32-unknown-unknown"),
                "rustup target add wasm32-unknown-unknown",
            ),
            Probe::new(
                "wasm-cc",
                have_wasm_cc(),
                "install a clang with the wasm32 backend (`brew install llvm`, or a swift.org \
                 toolchain — `day build` finds either), or point CC_wasm32_unknown_unknown at \
                 one; needed only when the app enables `persistence` (docs/web.md)",
            )
            .need(Need::BuildOptional),
        ],
        setup: "web-dom (docs/web.md) compiles the app's lib crate to WebAssembly and pairs it with\n\
                the host page embedded in the CLI. The Rust target is the whole toolchain for a\n\
                UI-only app: `rustup target add wasm32-unknown-unknown`. The `persistence` feature\n\
                also compiles the bundled SQLite to wasm, which needs a clang with the wasm32\n\
                backend — Apple's has none. `day build` probes plain `clang`, then Homebrew LLVM\n\
                and swift.org toolchains, exporting what it finds; a set CC_wasm32_unknown_unknown\n\
                (or CC) picks the compiler yourself. `day build -p web-dom` writes a\n\
                self-contained static dist/; `day launch -p web-dom` serves it and opens a browser.",
    }
}

/// Every toolkit group, in presentation order (core first).
fn all_groups() -> Vec<Group> {
    vec![
        core_group(),
        appkit_group(),
        uikit_group(),
        gtk_group(),
        qt_group(),
        xaml_group(),
        android_group(),
        harmonyos_group(),
        dom_group(),
    ]
}

// --- structured readiness (what `day checkup` asks) ------------------------

/// The doctor group id for a target's toolkit. Two mobile toolkits are spelled differently in the
/// two vocabularies — the target table names the backend feature (`mdc`, `arkui`), doctor groups
/// by OS toolchain (`android`, `harmonyos`) — and every caller that bridges them (`day new`'s
/// next-steps hint, `day checkup`'s selection) must bridge them the same way.
pub fn group_id(toolkit: &str) -> &str {
    match toolkit {
        "mdc" => "android",
        "arkui" => "harmonyos",
        other => other,
    }
}

/// A probe that found nothing, with the fix line doctor would have printed.
#[derive(Clone)]
pub struct Missing {
    pub name: &'static str,
    pub fix: String,
}

/// What a toolkit is missing, split by the stage the miss blocks — the answer `day checkup` needs
/// to decide whether it can build a combo, package it, or must skip it with a reason.
/// [`Need::BuildOptional`] / [`Need::PackOptional`] misses are left out: they degrade a stage that
/// still succeeds, so failing or skipping on them would be wrong.
#[derive(Clone, Default)]
pub struct Readiness {
    pub missing_build: Vec<Missing>,
    pub missing_pack: Vec<Missing>,
}

impl Readiness {
    /// Whether every prerequisite for compiling this toolkit is present. (Packaging asks about
    /// `missing_pack` directly — it reports WHICH tool is absent rather than just whether one is.)
    pub fn can_build(&self) -> bool {
        self.missing_build.is_empty()
    }
}

/// Run one toolkit group's probes and report what is missing, by stage. `None` for an id that is
/// not a builtin group (an externally declared toolkit — day has no house knowledge of it).
///
/// This runs the SAME probes `day doctor` prints, so a checkup's skip reason is doctor's own
/// diagnosis rather than a second, drifting copy of it.
pub fn readiness(group: &str) -> Option<Readiness> {
    let g = all_groups().into_iter().find(|g| g.id == group)?;
    let mut out = Readiness::default();
    for p in g.probes {
        if p.detail.is_some() {
            continue;
        }
        let missing = Missing {
            name: p.name,
            fix: p.fix,
        };
        match p.need {
            Need::Build => out.missing_build.push(missing),
            Need::Pack => out.missing_pack.push(missing),
            Need::BuildOptional | Need::PackOptional | Need::Launch => {}
        }
    }
    Some(out)
}

// --- rendering -------------------------------------------------------------

// The palette lives in one place now — `crate::term` (anstyle styles; printed through anstream,
// which strips the escapes when stderr isn't a color terminal).
use crate::term::{BOLD, DIM, ERROR, ERROR_BOLD, SUCCESS, SUCCESS_BOLD, WARN};
use anstream::eprintln;

/// Outcome of reporting one group: how many hard errors and soft/optional warnings it surfaced.
#[derive(Default)]
struct Tally {
    errors: u32,
    warnings: u32,
}

/// Print one group's header + probe lines. `hard` = a non-soft miss is an error (else a warning);
/// `show_setup` = append the detailed setup block (focused toolkits only).
fn report_group(g: &Group, host: &str, hard: bool, show_setup: bool) -> Tally {
    eprintln!("{BOLD}{}{BOLD:#}", g.label);
    let mut t = Tally::default();
    // A focused toolkit that can't build on this host is itself an error.
    if hard && !g.builds_on(host) {
        eprintln!(
            "  {ERROR}✗{ERROR:#} {:<14} builds on {:?}, not this {host} host",
            "host", g.hosts
        );
        t.errors += 1;
    }
    for p in &g.probes {
        match &p.detail {
            Some(d) => eprintln!("  {SUCCESS}✓{SUCCESS:#} {:<14} {d}", p.name),
            None if hard && !p.soft() => {
                eprintln!("  {ERROR}✗{ERROR:#} {:<14} {}", p.name, p.fix);
                t.errors += 1;
            }
            None => {
                eprintln!("  {WARN}⚠{WARN:#} {:<14} {}", p.name, p.fix);
                t.warnings += 1;
            }
        }
    }
    if show_setup {
        eprint_setup(g);
    }
    t
}

/// Print a group's detailed setup instructions (focused mode only).
fn eprint_setup(g: &Group) {
    eprintln!("  {DIM}── setup ──{DIM:#}");
    for line in g.setup.lines() {
        eprintln!("  {DIM}{line}{DIM:#}");
    }
    eprintln!();
}

/// `day doctor [--toolkit <id>]…`. `focus` holds the requested toolkit ids (empty = default scan).
/// The Ok value is the report's verdict code: 0, or exit 3 when errors were tallied — the report
/// itself already printed, so a non-zero verdict is not an extra `error:` line.
pub fn run(
    focus: &[String],
    external: &[crate::external::ExternalToolkit],
) -> Result<i32, crate::cli::CliError> {
    let host = host_os();
    let groups = all_groups();

    // Validate any requested ids up front so a typo is a clear error, not a silent no-op.
    let mut known: Vec<&str> = groups.iter().map(|g| g.id).collect();
    for e in external {
        known.push(e.target.name);
        known.push(e.target.toolkit);
    }
    for f in focus {
        if !known.contains(&f.as_str()) {
            return Err(crate::cli::CliError::usage(format!(
                "unknown toolkit {f:?} — choose from {}",
                known
                    .iter()
                    .filter(|k| **k != "core")
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }

    if focus.is_empty() {
        eprintln!(
            "{DIM}Scanning all toolkits buildable on this {host} host. Missing OPTIONAL toolkit\n\
             dependencies are warnings; run `day doctor --toolkit <id>` for hard checks + setup help.{DIM:#}\n"
        );
    } else {
        eprintln!(
            "{DIM}Focused check: {} (missing pieces are errors).{DIM:#}\n",
            focus.join(", ")
        );
    }

    let mut total = Tally::default();
    for g in &groups {
        let focused = focus.iter().any(|f| f == g.id);
        // Core's misses are always hard errors (rust is required for everything); otherwise a miss
        // is hard only when the toolkit is focused.
        let hard = focused || g.id == "core";

        if focus.is_empty() {
            // Default scan: skip cross-host toolkits (a dim n/a line instead of noise).
            if g.id != "core" && !g.builds_on(host) {
                eprintln!(
                    "{BOLD}{}{BOLD:#}  {DIM}n/a — builds on {:?}{DIM:#}",
                    g.label, g.hosts
                );
                continue;
            }
        } else if g.id != "core" && !focused {
            // Focused run: report only core + the requested toolkits.
            continue;
        }

        let t = report_group(g, host, hard, focused);
        total.errors += t.errors;
        total.warnings += t.warnings;
    }

    // Externally declared toolkits (docs/extending.md): one line per declaration, running the
    // crate's own probe where it gave one. The probe is the crate author's claim about what the
    // toolkit needs; day has no house knowledge of it, which is the point of the seam.
    for e in external {
        let focused = focus
            .iter()
            .any(|f| f == e.target.name || f == e.target.toolkit);
        if !focus.is_empty() && !focused {
            continue;
        }
        eprintln!(
            "{BOLD}{}{BOLD:#}  {DIM}external — declared by {}{DIM:#}",
            e.target.label, e.crate_name
        );
        match &e.doctor {
            None => eprintln!("  {DIM}– no doctor probe declared{DIM:#}"),
            Some(cmd) => {
                let mut parts = cmd.split_whitespace();
                let bin = parts.next().unwrap_or_default();
                let args: Vec<&str> = parts.collect();
                match run_line(bin, &args) {
                    Some(d) => eprintln!("  {SUCCESS}✓{SUCCESS:#} {:<14} {d}", e.target.toolkit),
                    None => {
                        eprintln!(
                            "  {ERROR}✗{ERROR:#} {:<14} `{cmd}` failed — see {}'s setup docs",
                            e.target.toolkit, e.crate_name
                        );
                        if focused {
                            total.errors += 1;
                        } else {
                            total.warnings += 1;
                        }
                    }
                }
            }
        }
    }

    eprintln!();
    if total.errors > 0 {
        eprintln!(
            "{ERROR_BOLD}✗ {} error(s){ERROR_BOLD:#}, {} warning(s).",
            total.errors, total.warnings
        );
        Ok(crate::cli::ErrKind::Env.exit_code())
    } else if total.warnings > 0 {
        eprintln!(
            "{WARN}⚠ {} warning(s){WARN:#} — optional toolkits not fully set up. Fine unless you build them.",
            total.warnings
        );
        Ok(0)
    } else {
        eprintln!("{SUCCESS_BOLD}✓ all good{SUCCESS_BOLD:#}");
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::TARGETS;

    /// Every shipped target's toolkit maps onto a real doctor group. The two vocabularies drifted
    /// once already (`harmony-arkui`'s rename), and the failure mode is silent: `day new`'s hint
    /// and `day checkup`'s selection would name a toolkit doctor rejects as unknown.
    #[test]
    fn every_target_toolkit_has_a_doctor_group() {
        let groups: Vec<&str> = all_groups().iter().map(|g| g.id).collect();
        for t in TARGETS {
            let id = group_id(t.toolkit);
            assert!(
                groups.contains(&id),
                "{}: toolkit {:?} maps to {id:?}, which is not a doctor group ({groups:?})",
                t.name,
                t.toolkit
            );
            assert!(readiness(id).is_some(), "{id} has no readiness report");
        }
    }

    /// Every toolkit group states at least one BUILD prerequisite. A group whose probes were all
    /// reclassified as optional would report "ready" on a machine with nothing installed, and
    /// `day checkup` would select it and fail deep inside cargo instead of skipping with a fix.
    #[test]
    fn every_toolkit_group_states_a_build_prerequisite() {
        for g in all_groups() {
            if g.id == "core" {
                continue;
            }
            assert!(
                g.probes.iter().any(|p| p.need == Need::Build),
                "{} has no Need::Build probe",
                g.id
            );
        }
    }

    /// An unknown id is `None`, not a panic or an empty (= "ready") report — externally declared
    /// toolkits reach `readiness` by name.
    #[test]
    fn unknown_group_has_no_readiness() {
        assert!(readiness("not-a-toolkit").is_none());
    }

    #[test]
    fn versions_compare_by_feature_level() {
        // The case that started this: Debian 12's GTK against what day-gtk compiles for.
        assert!(!version_at_least("4.8.3", (4, 10)));
        assert!(version_at_least("4.10.0", (4, 10)));
        assert!(version_at_least("4.22.4", (4, 10)));
        // 10 is not "less than 8" — a string compare would say it is.
        assert!(version_at_least("4.10", (4, 8)));
        assert!(version_at_least("5.0.0", (4, 10)));
        // A packaging suffix is not part of the version.
        assert!(version_at_least("1.5.0-2ubuntu1", (1, 5)));
        // Unreadable counts as too old: a probe that cannot tell must not report ready.
        assert!(!version_at_least("", (4, 10)));
        assert!(!version_at_least("unknown", (4, 10)));
    }

    /// The minimums doctor reports are the ones the BUILD will enforce, so they have to track
    /// `toolkits/day-gtk/Cargo.toml`. Bumping the crate feature without this constant would leave
    /// doctor calling a machine ready for a build that then fails in `gdk4-sys`.
    #[test]
    fn gtk_minimums_match_day_gtk() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../toolkits/day-gtk/Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
        let feature = |crate_name: &str| -> String {
            let line = text
                .lines()
                .find(|l| l.trim_start().starts_with(crate_name))
                .unwrap_or_else(|| panic!("no {crate_name} dependency in {}", manifest.display()));
            let at = line
                .find("\"v")
                .unwrap_or_else(|| panic!("no version feature in {line:?}"));
            line[at + 2..]
                .split('"')
                .next()
                .unwrap_or_default()
                .to_string()
        };
        assert_eq!(
            feature("gtk4"),
            format!("{}_{}", GTK4_MIN.0, GTK4_MIN.1),
            "GTK4_MIN and day-gtk's gtk4 feature disagree",
        );
        assert_eq!(
            feature("libadwaita"),
            format!("{}_{}", LIBADWAITA_MIN.0, LIBADWAITA_MIN.1),
            "LIBADWAITA_MIN and day-gtk's libadwaita feature disagree",
        );
    }
}
