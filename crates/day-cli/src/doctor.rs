// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day doctor: development-environment diagnosis, grouped by toolkit (DESIGN.md §16.5).
//!
//! Default (`day doctor`): checks the core toolchain plus every toolkit buildable on this host. A
//! missing optional toolkit dependency is a warning (yellow) and doctor still exits 0, because
//! you only need the toolkits you build. Core (rust) failures are always errors.
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
/// all (packaging tools, a booted device). `day doctor verify` reads the same field to decide which
/// toolkits it can build and which it can package (see [`readiness`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Need {
    /// Required to compile for this toolkit; a miss is an error when the toolkit is focused.
    Build,
    /// Build-time but optional: the build still produces a working app, degraded (a skipped
    /// resource blob, a Swift contribution that no dependency makes).
    BuildOptional,
    /// Required by `day pack` for this toolkit's formats; never needed to build.
    Pack,
    /// Packaging still succeeds without it, with a less portable artifact (the linuxdeploy
    /// plugins: without one the AppImage needs a machine that already has the toolkit).
    PackOptional,
    /// A launch-time prerequisite (a booted simulator/emulator, hdc); not needed to compile.
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

/// Full stdout of a command, every line rather than the first, for probes that must scan
/// multi-line output, e.g. `rustc -vV`'s `host:` line.
fn run_out(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd).args(args).output().ok().and_then(|o| {
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    })
}

/// `Some(dir)` if `dir` exists and is a directory; for env-var / SDK-path probes.
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
/// emulator, so having either arch installed is enough to prove the toolchain is set up.
fn have_any_rust_target(triples: &[&str]) -> Option<String> {
    triples.iter().find_map(|t| have_rust_target(t))
}

/// The JDK the Gradle build will use, if it's a version AGP accepts (17 or newer, AGP 9's
/// minimum). Resolves via `day_toolchain::jdk_home()`, the same `$JAVA_HOME`-first resolution the
/// gradle builds use, so doctor diagnoses what the build will run. Because the build trusts
/// `$JAVA_HOME`, a `$JAVA_HOME` pointing at a too-old JDK is a miss even when a newer one is
/// installed elsewhere. The major version is parsed from `java -version` (which prints
/// `openjdk version "26.0.1" …`, or bare `"21"`, to stderr).
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
    let version = text.lines().next().unwrap_or("").trim().to_string();
    // Say where it came from when Day found it rather than being told: Android Studio's bundled
    // runtime is picked over whatever `java` is on PATH, which is worth knowing.
    let bundled = std::env::var_os("JAVA_HOME").is_none()
        && day_toolchain::android_studio_jbr().is_some_and(|jbr| java.starts_with(jbr));
    (major >= 17).then(|| {
        if bundled {
            format!("{version} (Android Studio's bundled JDK)")
        } else {
            version
        }
    })
}

/// The SDK directory, if it exists, noting the Android Studio installation that manages it.
fn android_sdk_probe(sdk: &Path) -> Option<String> {
    let dir = existing_dir(sdk)?;
    Some(match day_toolchain::android_studio_homes().first() {
        Some(studio) => format!("{dir} (Android Studio: {})", studio.display()),
        None => dir,
    })
}

/// How to install an SDK package on this machine: through Android Studio when it is here, and
/// through the SDK's own `sdkmanager` (by full path, since Studio does not put it on PATH).
fn android_sdk_install_hint(what: &str, pkg: &str) -> String {
    let studio = if day_toolchain::android_studio_homes().is_empty() {
        String::new()
    } else {
        format!("in Android Studio, Settings ▸ Languages & Frameworks ▸ Android SDK ▸ {what}; or ")
    };
    let sdkmanager = match day_toolchain::android_sdkmanager() {
        Some(sm) => format!("`{} --install '{pkg}'`", sm.display()),
        None => format!(
            "`sdkmanager --install '{pkg}'` (from the SDK's Command-line Tools, which Android \
             Studio's SDK Tools tab installs)"
        ),
    };
    format!("{studio}{sdkmanager}")
}

/// The C compiler the web build's SQLite compile will use: [`day_toolchain::wasm_cc`], the
/// Same resolution `day build -p web-dom` applies, so doctor reports what the build will run.
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
        probes: vec![
            Probe::new(
                "rust",
                run_line("cargo", &["--version"]),
                "install Rust via https://rustup.rs (rustup) or `brew install rust`",
            ),
            // Optional: a project already on disk builds without it. It is what `day launch
            // --git` clones with, what `day new --template <git-url>` fetches with, and what
            // `day rebuild` checks a shipped commit out with.
            Probe::new(
                "git",
                run_line("git", &["--version"]),
                "install git — `day launch --git <url>` and `day rebuild` need it",
            )
            .need(Need::BuildOptional),
        ],
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
        setup: "macOS desktop (AppKit) builds through the `platform/macos/DayApp.xcodeproj`\n\
                host project with xcodebuild, which needs a full Xcode (`xcode-select -s\n\
                /Applications/Xcode.app`) — the command-line tools alone are not enough. No\n\
                extra Rust target: the host toolchain builds the staticlib. An app that\n\
                predates the scaffold adopts it with `day project add-target macos-appkit`.\n\
                Swift contributions (SwiftUI embedding, docs/swiftui.md) build inside the same\n\
                xcodebuild run through the generated DayPieces package.",
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
                "boot a simulator: `day devices boot -p ios-uikit <id>`",
            )
            .need(Need::Launch),
            // Advisory: a build, a launch and a dayscript capture all work without a window.
            // Which app provides one moved in Xcode 27 (Simulator.app out, Device Hub in), so
            // naming the one in use turns "nothing appeared" into an answer.
            Probe::new(
                "simulator-ui",
                crate::devices::simulator_ui().map(|ui| match ui {
                    crate::devices::SimulatorUi::Simulator(p) => {
                        format!("Simulator.app ({})", p.display())
                    }
                    crate::devices::SimulatorUi::DeviceHub(p) => {
                        format!("Device Hub ({})", p.display())
                    }
                }),
                "this Xcode ships neither Simulator.app (26 and earlier) nor Device Hub (27 and \
                 later), so simulators run without a window; builds and captures are unaffected",
            )
            .need(Need::Launch),
            // Orientation (`day devices boot --orientation`, docs/screenshots.md) is the one
            // thing that needs an Xcode floor rather than just "an Xcode": simulators became
            // drivable through `devicectl` in 26.6, and there is no other way to turn one
            // without a GUI session. Advisory, not required; everything else works below it.
            //
            // This looks for a simulated device rather than asking `--help` or grepping for the
            // `"devices"` key, both of which passed on a CI runner that could not turn a
            // simulator: `--help` only proves the binary exists, and `"devices"` matches the empty
            // array a CoreDevice without simulator support returns. Each weaker form reported ✓
            // moments before the boot step failed on the very thing it had vouched for.
            //
            // Reports the CoreDevice version rather than the Xcode one, because that IS the
            // thing that decides (see CORE_DEVICE_FLOOR in devices.rs) and the two come apart:
            // a machine whose selected Xcode is 26.6 still drives simulators if some newer Xcode
            // ran its first launch and left 642.15 installed. Printing the selected Xcode there
            // would name a version that has nothing to do with the answer.
            Probe::new(
                "simulator orientation",
                run_line(
                    "bash",
                    &[
                        "-c",
                        "xcrun devicectl list devices -j - --quiet 2>/dev/null \
                         | grep -q '\"simulated\"' && echo \"CoreDevice $(/usr/libexec/PlistBuddy \
                         -c 'Print :CFBundleVersion' /Library/Developer/PrivateFrameworks/\
                         CoreDevice.framework/Versions/A/Resources/Info.plist)\"",
                    ],
                ),
                "capture in landscape needs Xcode 27+ (it installs the CoreDevice that can drive \
                 simulators; the macOS version does not matter); without it, drop `--orientation`",
            )
            .need(Need::Launch),
        ],
        setup: "iOS (UIKit) cross-compiles via an Xcode script phase and runs on the Simulator.\n\
                Needs: full Xcode (`xcode-select -s /Applications/Xcode.app`), the simulator Rust\n\
                target `rustup target add aarch64-apple-ios-sim`, and a booted simulator to launch\n\
                (`xcrun simctl boot <device>`). Capturing in a chosen ORIENTATION additionally\n\
                needs Xcode 27 or newer, which installs the CoreDevice that lets `devicectl` drive\n\
                simulators — the macOS version does not affect it. iOS builds only on a macOS\n\
                host.",
    }
}

/// The GTK stack day-gtk compiles against. `toolkits/day-gtk/Cargo.toml` enables the `gtk4`
/// crate's `v4_10` feature and libadwaita's `v1_5`, and those features are exactly what the `-sys`
/// crates hand to pkg-config as a minimum. Kept in step with that manifest by
/// `gtk_minimums_match_day_gtk` below.
const GTK4_MIN: (u32, u32) = (4, 10);
const LIBADWAITA_MIN: (u32, u32) = (1, 5);

/// Is `found` (a pkg-config `--modversion` string like `4.8.3`) at least `min`?
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
            // Optional: resource staging (§18.3) is best-effort; a missing `glib-compile-resources`
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
            // The other half of `day pack -p linux-gtk`: the .appimage (§16.5). Without the gtk
            // plugin an AppImage still builds, but carries no GdkPixbuf loaders or GSettings
            // schemas, so both are probed, and the plugin is the optional one.
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
                newer and libadwaita 1.5 or newer — it builds stack navigation on\n\
                AdwNavigationView and dialogs on GtkFileDialog/GtkAlertDialog, none of which\n\
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
            // Optional: like glib-compile-resources, `rcc` staging is best-effort; a miss skips
            // the qresource blob (day loads images from the filesystem roots), so it's a warning,
            // not an error (MSYS2 windows-qt doesn't ship `rcc` on PATH).
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
            // The other half of `day pack -p linux-qt`: the .appimage (§16.5). Without the qt
            // plugin the image carries no platform plugin, so it cannot open a window on a machine
            // without Qt, hence probing the plugin as well as the tool.
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
                // Scan the full `rustc -vV` output for the `host:` line; `run_line` returns only
                // line 1 (`rustc <version>`), which is why the old check false-negatived on a valid
                // msvc host (and its `bash`+`grep` fallback isn't reliably resolvable from a native
                // process).
                run_out("rustc", &["-vV"]).and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("host: "))
                        .filter(|h| h.contains("windows-msvc"))
                        .map(str::to_string)
                }),
                "rustup default stable-msvc + install the VS 2022 C++ Build Tools",
            ),
            // Only `day pack -p windows-xaml` needs these (§16.5): makeappx/signtool ship with
            // the Windows SDK, makensis via `choco install nsis`.
            Probe::new(
                "makeappx (Windows SDK)",
                crate::pack::windows_kit_tool_probe("makeappx.exe"),
                "install the Windows 10/11 SDK (for `day pack` msix)",
            )
            .need(Need::Pack),
            Probe::new(
                "makensis",
                // The same lookup `day pack` uses (DAY_MAKENSIS → PATH → %ProgramFiles%\NSIS →
                // chocolatey), not a PATH-only `which`: a bare `which` reports missing for the
                // usual `choco install nsis`, whose shim directory a running process's PATH does
                // not pick up, so doctor would contradict the pack that then succeeds, or miss
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
                android_sdk_probe(&sdk),
                "install Android Studio, which installs the SDK (or the standalone command-line \
                 tools); set ANDROID_HOME if it is not at the platform default or where Studio's \
                 settings say",
            ),
            Probe::new(
                "android-ndk",
                ndk.as_ref().and_then(|p| existing_dir(p)),
                format!(
                    "install an NDK: {} (ANDROID_NDK_HOME overrides)",
                    android_sdk_install_hint("SDK Tools ▸ NDK (Side by side)", "ndk;<version>")
                ),
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
                "install Android Studio (its bundled JDK is used automatically), or a JDK 17 or \
                 newer and point JAVA_HOME at it (`brew install openjdk@21`); the Gradle build \
                 uses $JAVA_HOME",
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
                • Android Studio, which installs the SDK at the platform default and bundles a JDK;\n\
                  Day finds both, and the SDK location Studio's settings record (docs/environment.md).\n\
                  Set ANDROID_HOME (or ANDROID_SDK_ROOT) for an SDK elsewhere\n\
                • an NDK — Studio's SDK Manager (SDK Tools ▸ NDK), or the SDK's\n\
                  `cmdline-tools/latest/bin/sdkmanager --install 'ndk;<ver>'`; ANDROID_NDK_HOME overrides\n\
                • the Android Rust target — `rustup target add aarch64-linux-android`\n\
                • `cargo install cargo-ndk`\n\
                • JDK 17 or newer — Android Studio's bundled one is used when JAVA_HOME is unset;\n\
                  otherwise `brew install openjdk@21` and set JAVA_HOME (AGP 9's minimum is 17)\n\
                A booted emulator or attached device is needed only to launch, not to build. Create\n\
                an AVD in Android Studio's Device Manager (or `avdmanager create avd`) and start it\n\
                with `emulator -avd <name>` — `day` has no Android-emulator command of its own.",
    }
}

fn harmonyos_group() -> Group {
    use crate::ohos::BinaryFormat;
    let host = host_os();
    let native = BinaryFormat::for_host(host);
    let ndk = crate::ohos::find_ohos_ndk().ok().map(PathBuf::from);
    // hdc ships next to the NDK, in the SDK's sibling toolchains/ dir; also accept it on PATH.
    let hdc = crate::ohos::find_tool("hdc").or_else(|| {
        let c = ndk
            .as_ref()?
            .parent()?
            .join("toolchains")
            .join(crate::ohos::exe_name("hdc"));
        c.is_file().then_some(c)
    });
    let tool = |name: &str| crate::ohos::find_tool(name).map(|p| p.display().to_string());
    // Where the command-line tools' bin/ lives on each host, for the PATH hints.
    let clt_hint = match host {
        // The Linux bundle's tools are pure JavaScript, so macOS runs them through node wrappers
        // (docs/harmonyos.md); its SDK binaries are Linux ones and don't run there.
        "macos" => {
            "the OpenHarmony command-line-tools (DevEco Studio's, or the Linux bundle run \
                    through node wrappers, see docs/harmonyos.md); put hvigorw/ohpm on PATH"
        }
        _ => {
            "the OpenHarmony command-line-tools (bundled with DevEco Studio, or the standalone \
              download); put their bin/ on PATH"
        }
    };
    Group {
        id: "harmonyos",
        label: "HarmonyOS · ArkUI",
        hosts: &["any"],
        probes: vec![
            ndk_probe(ndk.as_deref(), native),
            Probe::new(
                "rust-ohos",
                have_rust_target("aarch64-unknown-linux-ohos")
                    .or_else(|| have_rust_target("x86_64-unknown-linux-ohos")),
                "rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos",
            ),
            sdk_probe(
                std::env::var_os("OHOS_BASE_SDK_HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from),
                native,
                project_compile_sdk(),
            ),
            Probe::new("hvigorw", tool("hvigorw"), format!("install {clt_hint}")),
            Probe::new("ohpm", tool("ohpm"), format!("install {clt_hint}")),
            // `day build` signs the .hap with sign-hap.mjs under node.
            Probe::new(
                "node",
                which("node").map(|p| p.display().to_string()),
                match host {
                    "linux" => {
                        "signing runs node: put the command-line-tools' tool/node/bin on PATH, \
                         or install Node.js"
                    }
                    "macos" => "signing runs node: `brew install node`",
                    _ => "signing runs node: install Node.js (nodejs.org) and put it on PATH",
                },
            ),
            Probe::new(
                "hdc",
                hdc.map(|p| p.display().to_string()),
                "hdc ships in the SDK's toolchains/ dir beside `native`; put it on PATH to \
                 install and launch",
            )
            .need(Need::Launch),
            Probe::new(
                "qemu",
                // On Linux the emulator's window is SDL's, so a QEMU built without it (Homebrew's,
                // GTK-only) doesn't count as installed.
                which("qemu-system-x86_64")
                    .filter(|p| {
                        host != "linux"
                            || crate::ohos::qemu_has_display(&p.to_string_lossy(), "sdl")
                    })
                    .map(|p| p.display().to_string()),
                match host {
                    "linux" => {
                        "the emulator needs your distribution's QEMU, with its SDL display: `sudo \
                         apt install qemu-system-x86 qemu-system-gui` (Fedora: \
                         `qemu-system-x86-core qemu-ui-sdl`). Homebrew's QEMU has no SDL display"
                    }
                    "macos" => "the emulator needs qemu-system-x86_64: `brew install qemu`",
                    _ => {
                        "the emulator needs qemu-system-x86_64: install QEMU (qemu.org/download) \
                         and put it on PATH"
                    }
                },
            )
            .need(Need::Launch),
            emulator_images_probe(&crate::ohos::emulator_images_dir()),
        ]
        .into_iter()
        .chain(kvm_probe(host))
        .collect(),
        setup: "HarmonyOS (ArkUI) cross-compiles a Rust cdylib (libentry.so), packages a .hap with\n\
                hvigor, signs it with node, and installs over hdc. Install:\n\
                • the OpenHarmony SDK for THIS host OS — its `native` component as OHOS_NDK_HOME, and\n\
                  the SDK as OHOS_BASE_SDK_HOME in the versioned layout hvigor requires,\n\
                  <dir>/<api>/{ets,native,toolchains,…} (a symlink `18` → the SDK works). On Linux\n\
                  the command-line-tools bundle carries it (sdk/default/openharmony); on macOS and\n\
                  Windows use that host's public SDK. `hdc` lives in its toolchains/ dir\n\
                • the OpenHarmony Rust targets — `rustup target add aarch64-unknown-linux-ohos\n\
                  x86_64-unknown-linux-ohos`\n\
                • hvigor + ohpm — from the OpenHarmony command-line-tools (bundled with DevEco Studio);\n\
                  put their bin/ on PATH. These package the .hap and are not part of the public SDK.\n\
                • node on PATH (signing)\n\
                An OpenHarmony emulator (Oniro) or device is needed only to launch, not to build:\n\
                install QEMU, unpack an emulator image (Oniro for OpenHarmony 6.x, or ohos-qemu's\n\
                x86_64_virt for 7.0; neither is bundled, see docs/harmonyos.md), then\n\
                `day devices boot -p harmony-arkui`.",
    }
}

/// The emulator images `day devices boot` would start: an Oniro directory (OpenHarmony 6.x) or an
/// ohos-qemu `x86_64_virt` one (7.0), whichever layout the directory holds, complete.
fn emulator_images_probe(dir: &Path) -> Probe {
    use crate::ohos::EmulatorImage;
    let get = "unpack Oniro's oniro_emulator.zip (OpenHarmony 6.x) or an ohos-qemu \
               x86_64_virt package (7.0) there, or set DAY_OHOS_EMULATOR to its images/ dir";
    let probe = |detail: Option<String>, fix: String| {
        Probe::new("emu-images", detail, fix).need(Need::Launch)
    };
    let image = match EmulatorImage::detect(dir) {
        Ok(image) => image,
        Err(e) => return probe(None, e),
    };
    let missing = image.missing(dir);
    if missing.is_empty() {
        probe(
            Some(format!("{} ({})", dir.display(), image.label())),
            String::new(),
        )
    } else if missing.len() == image.files().len() {
        probe(
            None,
            format!("no emulator images at {}: {get}", dir.display()),
        )
    } else {
        probe(
            None,
            format!(
                "{} ({}) is missing {}",
                dir.display(),
                image.label(),
                missing.join(", ")
            ),
        )
    }
}

/// The NDK (`OHOS_NDK_HOME`): present, with a clang this host can run. An SDK for another OS
/// (the Linux command-line-tools' NDK on a Mac, say) is found but fails to link.
fn ndk_probe(ndk: Option<&Path>, native: Option<crate::ohos::BinaryFormat>) -> Probe {
    let fix_missing = "set OHOS_NDK_HOME to the OpenHarmony SDK's `native` dir, from the SDK for this host OS \
         (see docs/harmonyos.md)";
    let Some(ndk) = ndk else {
        return Probe::new("ohos-ndk", None, fix_missing);
    };
    if !ndk.join("llvm").join("bin").is_dir() {
        return Probe::new(
            "ohos-ndk",
            None,
            format!("{} has no llvm/bin: {fix_missing}", ndk.display()),
        );
    }
    let clang = ndk
        .join("llvm")
        .join("bin")
        .join(crate::ohos::exe_name("clang"));
    match (crate::ohos::BinaryFormat::of_file(&clang), native) {
        (Some(found), Some(want)) if found != want => Probe::new(
            "ohos-ndk",
            None,
            format!(
                "{} is the {} NDK, which this {} host can't run: use the SDK for this host OS",
                ndk.display(),
                found.os(),
                want.os()
            ),
        ),
        _ => Probe::new("ohos-ndk", Some(ndk.display().to_string()), fix_missing),
    }
}

/// `OHOS_BASE_SDK_HOME`, the SDK hvigor packages against: set, in the versioned layout
/// (`<dir>/<api>/…`; an unversioned root fails with "The SDK management mode has changed"),
/// holding the API level the project compiles against, with tools this host can run (hvigor
/// spawns `restool` and friends directly, so a foreign SDK fails with `spawn ENOEXEC`).
fn sdk_probe(
    base: Option<PathBuf>,
    native: Option<crate::ohos::BinaryFormat>,
    compile_sdk: Option<u32>,
) -> Probe {
    let versioned = "a directory holding the SDK under its API level, e.g. <dir>/18 → the \
                     SDK root (the one with ets/, native/, toolchains/)";
    let Some(base) = base else {
        return Probe::new(
            "ohos-sdk",
            None,
            format!("hvigor needs OHOS_BASE_SDK_HOME: set it to {versioned}"),
        );
    };
    let levels = crate::ohos::sdk_api_levels(&base);
    let Some(&newest) = levels.last() else {
        return Probe::new(
            "ohos-sdk",
            None,
            format!(
                "OHOS_BASE_SDK_HOME ({}) has no API-level subdirectory; hvigor wants {versioned}",
                base.display()
            ),
        );
    };
    let level = match compile_sdk {
        Some(want) if !levels.contains(&want) => {
            return Probe::new(
                "ohos-sdk",
                None,
                format!(
                    "this project compiles against API {want}, but OHOS_BASE_SDK_HOME ({}) holds \
                     {}: add {}/{want}",
                    base.display(),
                    join_levels(&levels),
                    base.display()
                ),
            );
        }
        Some(want) => want,
        None => newest,
    };
    // Probe this host's name first, then the other one: a Linux or Mac SDK on Windows has a bare
    // `restool` (no `.exe`), and a Windows SDK elsewhere has only `restool.exe`. Looking for the
    // host's name alone would find nothing in exactly the foreign SDK this check exists to catch.
    let toolchains = base.join(level.to_string()).join("toolchains");
    let found = [
        crate::ohos::exe_name("restool"),
        "restool".into(),
        "restool.exe".into(),
    ]
    .iter()
    .find_map(|name| crate::ohos::BinaryFormat::of_file(&toolchains.join(name)));
    if let (Some(found), Some(want)) = (found, native)
        && found != want
    {
        return Probe::new(
            "ohos-sdk",
            None,
            format!(
                "{} is a {} SDK, which this {} host can't run (hvigor fails with spawn \
                 ENOEXEC): point OHOS_BASE_SDK_HOME at this host's SDK",
                base.join(level.to_string()).display(),
                found.os(),
                want.os()
            ),
        );
    }
    Probe::new(
        "ohos-sdk",
        Some(format!("{} (API {})", base.display(), join_levels(&levels))),
        "",
    )
}

fn join_levels(levels: &[u32]) -> String {
    levels
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `compileSdkVersion` of the Day project containing the working directory, from its hvigor
/// build profile, so the SDK probe can ask for that exact API level. `None` outside a project.
fn project_compile_sdk() -> Option<u32> {
    let cwd = std::env::current_dir().ok()?;
    let root = cwd.ancestors().find(|d| d.join("Day.toml").is_file())?;
    ["harmony", "ohos"].iter().find_map(|dir| {
        let profile = root.join("platform").join(dir).join("build-profile.json5");
        parse_compile_sdk(&std::fs::read_to_string(profile).ok()?)
    })
}

/// The first `compileSdkVersion` number in a build-profile.json5 (JSON5: the key may be quoted
/// or bare, and comments may mention it, so a comment line is skipped).
fn parse_compile_sdk(profile: &str) -> Option<u32> {
    profile
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .find_map(|line| {
            let rest = line.split("compileSdkVersion").nth(1)?;
            let rest = rest.trim_start_matches(['"', '\'']).trim_start();
            let rest = rest.strip_prefix(':')?.trim_start();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
}

/// KVM for the emulator (Linux only: macOS and Windows run it under TCG). A `/dev/kvm` this user
/// can't open makes QEMU fall back to TCG, which boots in minutes rather than seconds.
fn kvm_probe(host: &str) -> Option<Probe> {
    if host != "linux" {
        return None;
    }
    let kvm = Path::new("/dev/kvm");
    let usable = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(kvm)
        .is_ok();
    Some(
        Probe::new(
            "kvm",
            usable.then(|| "/dev/kvm (the emulator runs KVM-accelerated)".to_string()),
            if kvm.exists() {
                "/dev/kvm isn't usable by this user, so the emulator runs slowly under TCG: \
                 `sudo usermod -aG kvm $USER`, then log in again"
            } else {
                "no /dev/kvm, so the emulator runs slowly under TCG: enable virtualization \
                 (VT-x/AMD-V) in the firmware and load kvm_intel or kvm_amd"
            },
        )
        .need(Need::Launch),
    )
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

// --- structured readiness (what `day doctor verify` asks) ------------------------

/// The doctor group id for a target's toolkit. Two mobile toolkits are spelled differently in the
/// two vocabularies (the target table names the backend feature (`mdc`, `arkui`), doctor groups
/// by OS toolchain (`android`, `harmonyos`)), and every caller that bridges them (`day new`'s
/// next-steps hint, `day doctor verify`'s selection) must bridge them the same way.
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

/// What a toolkit is missing, split by the stage the miss blocks: the answer `day doctor verify` needs
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
    /// `missing_pack` directly; it reports which tool is absent rather than just whether one is.)
    pub fn can_build(&self) -> bool {
        self.missing_build.is_empty()
    }
}

/// Run one toolkit group's probes and report what is missing, by stage. `None` for an id that is
/// not a builtin group (an externally declared toolkit; day has no house knowledge of it).
///
/// This runs the same probes `day doctor` prints, so verification reports doctor's own skip reasons
/// rather than a second copy of its diagnosis.
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

// The palette lives in one place now, `crate::term` (anstyle styles; printed through anstream,
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
/// The Ok value is the report's verdict code: 0, or exit 3 when errors were tallied. The report
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
    // crate's probe where it gave one. The probe is the crate author's claim about what the
    // toolkit needs; day has no house knowledge of it, and the declaration exists so it never
    // needs any.
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
    /// and `day doctor verify`'s selection would name a toolkit doctor rejects as unknown.
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

    /// Every toolkit group states at least one build prerequisite. A group whose probes were all
    /// reclassified as optional would report "ready" on a machine with nothing installed, and
    /// `day doctor verify` would select it and fail deep inside cargo instead of skipping with a fix.
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

    /// An unknown id is `None`, not a panic or an empty (= "ready") report; externally declared
    /// toolkits reach `readiness` by name.
    #[test]
    fn binary_formats_are_recognized_by_magic() {
        use crate::ohos::BinaryFormat as F;
        assert_eq!(F::detect(b"\x7fELF\x02"), Some(F::Elf));
        assert_eq!(F::detect(b"MZ\x90\x00"), Some(F::Pe));
        // 64- and 32-bit Mach-O in both byte orders, and a universal (fat) binary.
        for head in [
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xce, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xfe, 0xed, 0xfa, 0xce],
            [0xca, 0xfe, 0xba, 0xbe],
        ] {
            assert_eq!(F::detect(&head), Some(F::MachO), "{head:x?}");
        }
        assert_eq!(F::detect(b"#!/b"), None);
        assert_eq!(F::detect(b"\x7fE"), None);
        assert_eq!(F::for_host("linux"), Some(F::Elf));
        assert_eq!(F::for_host("macos"), Some(F::MachO));
        assert_eq!(F::for_host("windows"), Some(F::Pe));
        assert_eq!(F::for_host("other"), None);
    }

    #[test]
    fn windows_tools_resolve_to_batch_files_too() {
        assert_eq!(crate::ohos::tool_file_names("hvigorw", false), ["hvigorw"]);
        assert_eq!(
            crate::ohos::tool_file_names("hvigorw", true),
            ["hvigorw", "hvigorw.exe", "hvigorw.bat", "hvigorw.cmd"]
        );
    }

    #[test]
    fn compile_sdk_version_is_read_from_the_build_profile() {
        let profile = r#"{
          "app": {
            // compileSdkVersion is the API level hvigor builds against
            "products": [{ "name": "default", "compileSdkVersion": 18, "compatibleSdkVersion": 12 }]
          }
        }"#;
        assert_eq!(parse_compile_sdk(profile), Some(18));
        assert_eq!(parse_compile_sdk("{ compileSdkVersion: 20 }"), Some(20));
        assert_eq!(parse_compile_sdk("{ 'compileSdkVersion' : 12 }"), Some(12));
        assert_eq!(parse_compile_sdk("// compileSdkVersion: 9\n{}"), None);
        assert_eq!(parse_compile_sdk("{}"), None);
    }

    /// A scratch directory for one test, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "day-doctor-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        /// Create `rel` (and its parents) holding `bytes`.
        fn file(&self, rel: &str, bytes: &[u8]) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sdk_levels_are_numeric_dirs_holding_sdk_components() {
        let t = Scratch::new("levels");
        t.file("18/toolchains/restool", b"x");
        t.file("20/ets/api.d.ts", b"x");
        t.file("12/readme.txt", b"x"); // no SDK component: not a level
        t.file("default/toolchains/restool", b"x"); // not numeric
        assert_eq!(crate::ohos::sdk_api_levels(&t.0), [18, 20]);
        assert!(crate::ohos::sdk_api_levels(&t.0.join("missing")).is_empty());
    }

    #[test]
    fn sdk_probe_checks_layout_level_and_host_format() {
        use crate::ohos::BinaryFormat as F;
        // Unset, and an unversioned root.
        assert!(sdk_probe(None, Some(F::Elf), None).detail.is_none());
        let flat = Scratch::new("flat");
        flat.file("toolchains/restool", b"\x7fELF");
        let p = sdk_probe(Some(flat.0.clone()), Some(F::Elf), None);
        assert!(
            p.detail.is_none() && p.fix.contains("no API-level"),
            "{}",
            p.fix
        );

        let t = Scratch::new("sdk");
        t.file("18/toolchains/restool", b"\x7fELF\x02\x01");
        // Linux host, Linux SDK: fine, with or without a project level.
        assert!(
            sdk_probe(Some(t.0.clone()), Some(F::Elf), None)
                .detail
                .is_some()
        );
        assert!(
            sdk_probe(Some(t.0.clone()), Some(F::Elf), Some(18))
                .detail
                .is_some()
        );
        // The project wants a level the SDK lacks.
        let p = sdk_probe(Some(t.0.clone()), Some(F::Elf), Some(20));
        assert!(p.detail.is_none() && p.fix.contains("API 20"), "{}", p.fix);
        // The Linux SDK on a Mac, and on Windows.
        for host in [F::MachO, F::Pe] {
            let p = sdk_probe(Some(t.0.clone()), Some(host), None);
            assert!(
                p.detail.is_none() && p.fix.contains("Linux SDK"),
                "{}",
                p.fix
            );
        }
        // On Windows the tool is restool.exe: a PE one there is native.
        let w = Scratch::new("sdk-win");
        w.file(
            &format!("18/toolchains/{}", crate::ohos::exe_name("restool")),
            b"MZ\x90\x00",
        );
        let p = sdk_probe(Some(w.0.clone()), Some(F::Pe), None);
        assert!(p.detail.is_some(), "{}", p.fix);
    }

    #[test]
    fn ndk_probe_rejects_an_ndk_for_another_host() {
        use crate::ohos::BinaryFormat as F;
        assert!(ndk_probe(None, Some(F::Elf)).detail.is_none());
        let t = Scratch::new("ndk");
        let clang = format!("llvm/bin/{}", crate::ohos::exe_name("clang"));
        t.file(&clang, b"\x7fELF\x02\x01");
        assert!(ndk_probe(Some(&t.0), Some(F::Elf)).detail.is_some());
        let p = ndk_probe(Some(&t.0), Some(F::MachO));
        assert!(
            p.detail.is_none() && p.fix.contains("Linux NDK"),
            "{}",
            p.fix
        );
        let empty = Scratch::new("ndk-empty");
        let p = ndk_probe(Some(&empty.0), Some(F::Elf));
        assert!(
            p.detail.is_none() && p.fix.contains("no llvm/bin"),
            "{}",
            p.fix
        );
    }

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
        // 10 is not "less than 8"; a string compare would say it is.
        assert!(version_at_least("4.10", (4, 8)));
        assert!(version_at_least("5.0.0", (4, 10)));
        // A packaging suffix is not part of the version.
        assert!(version_at_least("1.5.0-2ubuntu1", (1, 5)));
        // Unreadable counts as too old: a probe that cannot tell must not report ready.
        assert!(!version_at_least("", (4, 10)));
        assert!(!version_at_least("unknown", (4, 10)));
    }

    /// The minimums doctor reports are the ones the build will enforce, so they have to track
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
