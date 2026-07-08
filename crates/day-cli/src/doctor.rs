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

/// One environment probe: a label, the resolved detail (`Some` = found), and a one-line fix hint.
struct Probe {
    name: &'static str,
    detail: Option<String>,
    fix: String,
    /// A launch-time prerequisite (booted simulator/emulator), not a build one — stays a warning
    /// even when its toolkit is focused, since it isn't needed to compile.
    soft: bool,
}

impl Probe {
    fn new(name: &'static str, detail: Option<String>, fix: impl Into<String>) -> Self {
        Probe {
            name,
            detail,
            fix: fix.into(),
            soft: false,
        }
    }
    fn soft(mut self) -> Self {
        self.soft = true;
        self
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
        probes: vec![Probe::new(
            "xcode-clang",
            run_line("xcrun", &["--find", "clang"]),
            "install the Xcode command-line tools: `xcode-select --install`",
        )],
        setup: "macOS desktop (AppKit) builds as a plain cargo binary and needs Apple's clang\n\
                toolchain: `xcode-select --install` (or a full Xcode). No extra Rust target — the\n\
                host toolchain builds it.",
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
            .soft(),
        ],
        setup: "iOS (UIKit) cross-compiles via an Xcode script phase and runs on the Simulator.\n\
                Needs: full Xcode (`xcode-select -s /Applications/Xcode.app`), the simulator Rust\n\
                target `rustup target add aarch64-apple-ios-sim`, and a booted simulator to launch\n\
                (`xcrun simctl boot <device>`). iOS builds only on a macOS host.",
    }
}

fn gtk_group() -> Group {
    Group {
        id: "gtk",
        label: "GTK 4 · libadwaita",
        hosts: &["macos", "linux", "windows"],
        probes: vec![
            Probe::new(
                "gtk4",
                run_line("pkg-config", &["--modversion", "gtk4"]),
                "install GTK 4 (`brew install gtk4` · `apt install libgtk-4-dev` · MSYS2 mingw-w64-gtk4)",
            ),
            Probe::new(
                "libadwaita",
                run_line("pkg-config", &["--modversion", "libadwaita-1"]),
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
            .soft(),
        ],
        setup: "GTK 4 builds on macOS, Linux, and Windows via pkg-config. Install the dev libraries:\n\
                • macOS  — `brew install gtk4 libadwaita pkg-config`\n\
                • Linux  — `apt install libgtk-4-dev libadwaita-1-dev pkg-config`\n\
                • Windows— MSYS2: `pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita`\n\
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
                "install Qt 6 (`brew install qt` · `apt install qt6-base-dev` · aqtinstall on Windows)",
            ),
            // Optional: like glib-compile-resources, `rcc` staging is best-effort — a miss skips the
            // qresource blob (day loads images from the filesystem roots), so it's a warning, not an
            // error (MSYS2 windows-qt doesn't ship `rcc` on PATH).
            Probe::new(
                "rcc",
                find_rcc().map(|p| p.display().to_string()),
                "install Qt 6 (rcc, the resource compiler, ships in Qt's libexec)",
            )
            .soft(),
        ],
        setup: "Qt 6 Widgets builds on macOS, Linux, and Windows. Install Qt 6 and pkg-config:\n\
                • macOS  — `brew install qt pkg-config`\n\
                • Linux  — `apt install qt6-base-dev qt6-webengine-dev pkg-config`\n\
                • Windows— install Qt (aqtinstall or the online installer) and put its bin/ on PATH\n\
                `rcc` (Qt's resource compiler, §18.3) is resolved from qmake's libexec; a missing Qt\n\
                means both the build and bundled-resource staging fail.",
    }
}

fn winui_group() -> Group {
    Group {
        id: "winui",
        label: "Windows · WinUI 3",
        hosts: &["windows"],
        probes: vec![Probe::new(
            "msvc-toolchain",
            // The default rustc must target *-windows-msvc (winui builds with cl.exe + the SDK).
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
        )],
        setup: "WinUI 3 builds on a Windows host with the MSVC toolchain. Install:\n\
                • the Visual Studio 2022 C++ Build Tools (MSVC + Windows SDK)\n\
                • the MSVC Rust toolchain: `rustup default stable-msvc`\n\
                • the Windows App SDK runtime (for XAML Islands at launch)\n\
                WinUI cannot build off a Windows host.",
    }
}

fn android_group() -> Group {
    let sdk = crate::mobile::android_sdk_dir();
    let ndk = crate::mobile::find_ndk().ok();
    let adb = sdk.join("platform-tools/adb");
    Group {
        id: "android",
        label: "Android · android.widget",
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
                have_rust_target("aarch64-linux-android"),
                "rustup target add aarch64-linux-android",
            ),
            Probe::new(
                "cargo-ndk",
                run_line("cargo", &["ndk", "--version"]),
                "cargo install cargo-ndk",
            ),
            Probe::new(
                "jdk21",
                run_line("/opt/homebrew/opt/openjdk@21/bin/java", &["--version"])
                    .or_else(|| run_line("bash", &["-c", "java -version 2>&1 | grep -m1 '21\\.'"])),
                "install JDK 21 (`brew install openjdk@21`); newer JDKs break the AGP jdk-image transform",
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
                "start an emulator (`day android emulator launch`) or attach a device",
            )
            .soft(),
        ],
        setup: "Android (android.widget) cross-compiles the app to a JNI .so and runs it in a Gradle\n\
                app. Install:\n\
                • the Android SDK — set ANDROID_HOME (or ANDROID_SDK_ROOT); Android Studio installs it\n\
                  at ~/Library/Android/sdk (macOS) by default\n\
                • an NDK — via `sdkmanager --install 'ndk;<ver>'`; set ANDROID_NDK_HOME to override\n\
                • the Android Rust target — `rustup target add aarch64-linux-android`\n\
                • `cargo install cargo-ndk`\n\
                • JDK 21 — `brew install openjdk@21` (JDK 22+ breaks the AGP jdk-image transform)\n\
                A booted emulator or attached device is needed only to launch, not to build.",
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
            .soft(),
        ],
        setup: "HarmonyOS (ArkUI) cross-compiles a Rust cdylib (libentry.so), packages a .hap with\n\
                hvigor, signs it, and installs over hdc. Install:\n\
                • the OpenHarmony SDK `native` component — set OHOS_NDK_HOME to it (login-free: extract\n\
                  the public SDK, see docs/harmonyos.md). `hdc` lives in the sibling toolchains/ dir\n\
                • the OpenHarmony Rust targets — `rustup target add aarch64-unknown-linux-ohos\n\
                  x86_64-unknown-linux-ohos`\n\
                • hvigor + ohpm — from the OpenHarmony command-line-tools (bundled with DevEco Studio);\n\
                  put their bin/ on PATH. These package the .hap and are not part of the public SDK.\n\
                An OpenHarmony emulator (Oniro) or device is needed only to launch, not to build.",
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
        winui_group(),
        android_group(),
        harmonyos_group(),
    ]
}

// --- rendering -------------------------------------------------------------

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Outcome of reporting one group: how many hard errors and soft/optional warnings it surfaced.
#[derive(Default)]
struct Tally {
    errors: u32,
    warnings: u32,
}

/// Print one group's header + probe lines. `hard` = a non-soft miss is an error (else a warning);
/// `show_setup` = append the detailed setup block (focused toolkits only).
fn report_group(g: &Group, host: &str, hard: bool, show_setup: bool) -> Tally {
    eprintln!("{BOLD}{}{RESET}", g.label);
    let mut t = Tally::default();
    // A focused toolkit that can't build on this host is itself an error.
    if hard && !g.builds_on(host) {
        eprintln!(
            "  {RED}✗{RESET} {:<14} builds on {:?}, not this {host} host",
            "host", g.hosts
        );
        t.errors += 1;
    }
    for p in &g.probes {
        match &p.detail {
            Some(d) => eprintln!("  {GREEN}✓{RESET} {:<14} {d}", p.name),
            None if hard && !p.soft => {
                eprintln!("  {RED}✗{RESET} {:<14} {}", p.name, p.fix);
                t.errors += 1;
            }
            None => {
                eprintln!("  {YELLOW}⚠{RESET} {:<14} {}", p.name, p.fix);
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
    eprintln!("  {DIM}── setup ──{RESET}");
    for line in g.setup.lines() {
        eprintln!("  {DIM}{line}{RESET}");
    }
    eprintln!();
}

/// `day doctor [--toolkit <id>]…`. `focus` holds the requested toolkit ids (empty = default scan).
pub fn run(focus: &[String]) -> i32 {
    let host = host_os();
    let groups = all_groups();

    // Validate any requested ids up front so a typo is a clear error, not a silent no-op.
    let known: Vec<&str> = groups.iter().map(|g| g.id).collect();
    for f in focus {
        if !known.contains(&f.as_str()) {
            eprintln!(
                "error: unknown toolkit {f:?} — choose from {}",
                known
                    .iter()
                    .filter(|k| **k != "core")
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return 2;
        }
    }

    if focus.is_empty() {
        eprintln!(
            "{DIM}Scanning all toolkits buildable on this {host} host. Missing OPTIONAL toolkit\n\
             dependencies are warnings; run `day doctor --toolkit <id>` for hard checks + setup help.{RESET}\n"
        );
    } else {
        eprintln!(
            "{DIM}Focused check: {} (missing pieces are errors).{RESET}\n",
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
                    "{BOLD}{}{RESET}  {DIM}n/a — builds on {:?}{RESET}",
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

    eprintln!();
    if total.errors > 0 {
        eprintln!(
            "{RED}{BOLD}✗ {} error(s){RESET}, {} warning(s).",
            total.errors, total.warnings
        );
        3
    } else if total.warnings > 0 {
        eprintln!(
            "{YELLOW}⚠ {} warning(s){RESET} — optional toolkits not fully set up. Fine unless you build them.",
            total.warnings
        );
        0
    } else {
        eprintln!("{GREEN}{BOLD}✓ all good{RESET}");
        0
    }
}
