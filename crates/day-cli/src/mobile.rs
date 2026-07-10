//! Mobile pipelines (DESIGN.md §16.5, §17.4): ios-uikit via xcodebuild + simctl (the Xcode
//! project's script phase calls back into `day xcode-backend build` for the Rust staticlib);
//! android-widget via gradle + adb (the gradle scaffold calls `day gradle-backend build`).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::meta::{Project, find_project};
use crate::ops::{BuildOutcome, LaunchSpec, LogStream, emit_log, status};
use crate::targets::Target;

pub(crate) fn rustup_cargo() -> Result<(PathBuf, PathBuf), String> {
    // Shared lookup: honors RUSTUP_HOME and prefers a stable-* toolchain (docs/environment.md).
    day_toolchain::rustup_cargo()
}

pub(crate) fn run_logged(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if out.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}

/// Make a path absolute without requiring it to exist yet (build-output dirs often don't). Build-tool
/// arguments such as xcodebuild's `SYMROOT` MUST be absolute — a relative one is resolved per-target
/// against each target's own working directory, so an app target and its SwiftPM package dependencies
/// scatter their products into different trees.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}

/// True when a failed xcodebuild is the "a package resource bundle isn't where the app target expected
/// it" class — a stale or split build tree. Worth one clean retry (see [`build_ios`]).
fn is_stale_bundle_failure(out: &std::process::Output) -> bool {
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    all.contains(".bundle") && all.contains("no such file")
}

/// Distill a failed xcodebuild run into something readable. Raw xcodebuild output is mostly a wall of
/// `export FOO=bar` lines; the actionable content is the `error:` lines — surface those first (from
/// both streams), fall back to a non-`export` tail, and add a targeted hint for the resource-bundle
/// "no such file" failure class (a stale/split build tree).
fn diagnose_xcodebuild(out: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut errors: Vec<String> = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .filter(|l| l.starts_with("error:") || l.contains(": error:"))
        .map(str::to_string)
        .collect();
    errors.dedup();

    let mut msg = if errors.is_empty() {
        let tail: Vec<&str> = stdout
            .lines()
            .filter(|l| !l.trim_start().starts_with("export "))
            .rev()
            .take(20)
            .collect();
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    } else {
        errors.join("\n")
    };

    let lower = format!("{stdout}{stderr}").to_lowercase();
    if lower.contains(".bundle") && lower.contains("no such file") {
        msg.push_str(
            "\n\nhint: a SwiftPM package resource bundle wasn't where the app target expected it. \
             This is usually a stale or split build tree — remove build/day/ios-uikit and retry \
             (day launch does this automatically on a resource-bundle failure).",
        );
    }
    msg
}

// ---------------------------------------------------------------------------
// xcode-backend: invoked BY the Xcode script phase with Xcode's env (§17.4)
// ---------------------------------------------------------------------------

pub fn xcode_backend_build() -> i32 {
    let get = |k: &str| std::env::var(k).ok();
    let configuration = get("CONFIGURATION").unwrap_or_else(|| "Debug".into());
    let built_products = match get("BUILT_PRODUCTS_DIR") {
        Some(v) => PathBuf::from(v),
        None => {
            eprintln!(
                "day xcode-backend: must run inside an Xcode build (BUILT_PRODUCTS_DIR unset)"
            );
            return 2;
        }
    };
    let platform = get("PLATFORM_NAME").unwrap_or_else(|| "iphonesimulator".into());
    let project_dir = get("PROJECT_DIR").map(PathBuf::from).unwrap_or_default();

    // platform/ios/ → project root two levels up.
    let root = project_dir.join("../..");
    let project = match find_project(Some(&root)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("day xcode-backend: {e}");
            return 2;
        }
    };
    let profile = if configuration.to_lowercase().contains("release") {
        "release"
    } else {
        "debug"
    };
    let triple = match platform.as_str() {
        "iphonesimulator" => "aarch64-apple-ios-sim",
        "iphoneos" => "aarch64-apple-ios",
        other => {
            eprintln!("day xcode-backend: unsupported PLATFORM_NAME {other:?}");
            return 2;
        }
    };
    let (cargo, bin) = match rustup_cargo() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("day xcode-backend: {e}");
            return 3;
        }
    };
    let name = project.manifest.app.name.clone();
    let target_dir = project.root.join("build/day/cargo/ios-uikit").join(profile);
    let mut cmd = Command::new(&cargo);
    // Sanitize Xcode's script-phase env: SDKROOT points at the iphonesimulator SDK (poisoning
    // HOST compiles of proc-macro build scripts), and Xcode's PATH resolves `cc` to the raw
    // toolchain clang, which — unlike the /usr/bin/cc xcrun shim — does NOT auto-select an SDK
    // (ld: library 'System' not found). Reset both; rustc finds per-target SDKs via xcrun.
    for var in [
        "SDKROOT",
        "LIBRARY_PATH",
        "CPATH",
        "IPHONEOS_DEPLOYMENT_TARGET",
        "MACOSX_DEPLOYMENT_TARGET",
    ] {
        cmd.env_remove(var);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    cmd.current_dir(&project.root)
        .env(
            "PATH",
            format!(
                "{}:{home}/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin",
                bin.display()
            ),
        )
        .env("CARGO_TARGET_DIR", &target_dir)
        // `rustc --crate-type staticlib` so the app lib's manifest can stay rlib-only (see the
        // `[lib]` note in the app Cargo.toml); produces the same `lib<name>.a` this expects.
        // `--features` = `uikit` + every standalone piece's `<pkg>/uikit` renderer feature (Tier
        // A.2), so the app needn't re-list per-piece features in its own Cargo.toml.
        .args([
            "rustc",
            "-p",
            &name,
            "--lib",
            "--crate-type",
            "staticlib",
            "--no-default-features",
            "--features",
            &crate::ops::feature_selection(&project, "uikit"),
        ])
        .args(["--target", triple]);
    if profile == "release" {
        cmd.arg("--release");
    }
    if run_logged(&mut cmd, "cargo (ios)").is_err() {
        return 4;
    }
    let lib = target_dir
        .join(triple)
        .join(profile)
        .join(format!("lib{name}.a"));
    let out_dir = built_products.join("day"); // must match pbxproj LIBRARY_SEARCH_PATHS `$(BUILT_PRODUCTS_DIR)/day`
    if std::fs::create_dir_all(&out_dir).is_err() {
        eprintln!("day xcode-backend: cannot create {}", out_dir.display());
        return 4;
    }
    let dest = out_dir.join(format!("lib{name}.a"));
    if let Err(e) = std::fs::copy(&lib, &dest) {
        eprintln!(
            "day xcode-backend: copy {} → {}: {e}",
            lib.display(),
            dest.display()
        );
        return 4;
    }
    // Stage assets/ into the app bundle (§18.1's copy-phase mechanism).
    if let (Some(tbd), Some(res)) = (
        get("TARGET_BUILD_DIR"),
        get("UNLOCALIZED_RESOURCES_FOLDER_PATH"),
    ) {
        let src = project.root.join("assets");
        if src.exists() {
            let dst = PathBuf::from(tbd).join(res).join("assets");
            let _ = std::fs::create_dir_all(&dst);
            if let Ok(entries) = std::fs::read_dir(&src) {
                for e in entries.flatten() {
                    let _ = std::fs::copy(e.path(), dst.join(e.file_name()));
                }
            }
        }
    }
    eprintln!("day xcode-backend: staged {}", dest.display());
    0
}

// ---------------------------------------------------------------------------
// ios-uikit build + launch (porcelain side)
// ---------------------------------------------------------------------------

/// Keep the app Info.plist's `UIAppFonts` array in sync with the project's `fonts/` directory
/// (§18.4). iOS resolves the listed paths relative to the main bundle; the files themselves ride
/// the DayPieces resource bundle (`DayPieces_DayPieces.bundle/fonts/…`, staged by
/// `write_ios_pieces`), and day-uikit ALSO registers them with CoreText at launch, so a plist
/// that iOS declines to honor still resolves. The managed key is rewritten (or removed) on every
/// build — idempotent, so a committed plist only changes when `fonts/` changes.
fn sync_uiappfonts(project: &Project) -> Result<(), String> {
    // The scaffold's app target is Runner/ (older scaffolds used DayApp/).
    let plist = [
        "platform/ios/Runner/Info.plist",
        "platform/ios/DayApp/Info.plist",
    ]
    .iter()
    .map(|rel| project.root.join(rel))
    .find(|p| p.exists());
    let Some(plist) = plist else {
        return Ok(());
    };
    let fonts = crate::resources::scan_fonts(project)?;
    // Remove-then-insert keeps the array exactly equal to fonts/ (plutil -remove fails
    // harmlessly when the key is absent).
    let _ = Command::new("plutil")
        .args(["-remove", "UIAppFonts"])
        .arg(&plist)
        .output();
    if fonts.is_empty() {
        return Ok(());
    }
    let paths: Vec<String> = fonts
        .iter()
        .filter_map(|f| f.path.file_name().and_then(|n| n.to_str()))
        .map(|n| format!("DayPieces_DayPieces.bundle/fonts/{n}"))
        .collect();
    let json = serde_json::to_string(&paths).expect("UIAppFonts json");
    let out = Command::new("plutil")
        .args(["-replace", "UIAppFonts", "-json", &json])
        .arg(&plist)
        .output()
        .map_err(|e| format!("plutil: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "could not update UIAppFonts in {}: {}",
            plist.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

pub fn build_ios(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let configuration = if profile == "release" {
        "Release"
    } else {
        "Debug"
    };
    // SYMROOT MUST be absolute: xcodebuild resolves a relative build path against each target's own
    // working directory, so the Runner app target and its SwiftPM package dependencies (e.g. Lottie,
    // whose resource bundle the app copies) would land their products in different trees and the copy
    // would fail with "no such file … .bundle". `project.root` is absolute (see meta::find_project),
    // but absolutize here too so this invariant is enforced at the one place that actually matters.
    let symroot = absolute(&project.root.join("build/day/ios-uikit"))?;
    let day_bin = std::env::current_exe().map_err(|e| e.to_string())?;
    // Generate the local DayPieces SwiftPM package (piece Swift shims + SwiftPM deps) that the
    // .xcodeproj links, from every piece's [package.metadata.day.ios] — before xcodebuild resolves it.
    crate::pieces::write_ios_pieces(project)?;
    // Bundled fonts (§18.4): iOS additionally requires every custom font file to be listed in
    // the app Info.plist's UIAppFonts array. Keep the checked-in plist in sync with fonts/.
    sync_uiappfonts(project)?;
    status(
        "Building",
        &format!("{} (xcodebuild {configuration})", target.name),
    );
    let xcodebuild = || {
        let mut cmd = Command::new("xcodebuild");
        cmd.current_dir(project.root.join("platform/ios"))
            .args(["-project", "DayApp.xcodeproj", "-target", "Runner"])
            .args([
                "-configuration",
                configuration,
                "-sdk",
                "iphonesimulator",
                "-arch",
                "arm64",
            ])
            .arg(format!("SYMROOT={}", symroot.display()))
            .arg(format!("DAY_BIN={}", day_bin.display()))
            .arg("build")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.output().map_err(|e| format!("xcodebuild: {e}"))
    };
    let mut out = xcodebuild()?;
    if !out.status.success() && is_stale_bundle_failure(&out) {
        // A SwiftPM package resource bundle landed in the wrong tree (stale/split build products).
        // Clear this target's build tree and retry once from clean — self-heals the common case.
        status("Rebuilding", "ios-uikit (clearing stale build tree)");
        let _ = std::fs::remove_dir_all(&symroot);
        out = xcodebuild()?;
    }
    if !out.status.success() {
        return Err(format!("xcodebuild failed:\n{}", diagnose_xcodebuild(&out)));
    }
    // The Runner target's product bundle is named after the app's PRODUCT_NAME (per app), so locate
    // the single `.app` in the products dir rather than assuming a fixed name.
    let products = symroot.join(format!("{configuration}-iphonesimulator"));
    let app = std::fs::read_dir(&products)
        .map_err(|e| format!("reading {}: {e}", products.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("app"))
        .ok_or_else(|| format!("no .app bundle in {}", products.display()))?;
    Ok(BuildOutcome {
        target: target.name,
        artifact: app,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// UDIDs of every currently-booted iOS simulator (`simctl list devices booted`). All simulators on
/// a given host share the host arch, so the one `aarch64-apple-ios-sim` build runs on each.
pub(crate) fn booted_sims() -> Vec<String> {
    let out = match Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains("(Booted)"))
        .filter_map(|l| {
            // The UDID is the parenthesized 36-char group before "(Booted)".
            l.split(['(', ')'])
                .map(str::trim)
                .find(|t| t.len() == 36 && t.split('-').count() == 5)
                .map(str::to_string)
        })
        .collect()
}

pub fn launch_ios(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let bundle_id = project.manifest.app.id.clone();
    let sims = booted_sims();
    if sims.is_empty() {
        return Err(
            "no booted iOS simulator (open Simulator.app or `xcrun simctl boot <device>`); \
                    physical devices need code signing and aren't supported here"
                .into(),
        );
    }
    let multi = sims.len() > 1;
    let mut log_threads = Vec::new();
    for udid in &sims {
        run_logged(
            Command::new("xcrun")
                .args(["simctl", "install", udid])
                .arg(&outcome.artifact),
            &format!("simctl install ({udid})"),
        )?;
        let _ = Command::new("xcrun")
            .args(["simctl", "terminate", udid, &bundle_id])
            .status();
        let mut cmd = Command::new("xcrun");
        cmd.args(["simctl", "launch"]);
        if spec.attached {
            // `--console` (not `--console-pty`) keeps the app's stdout and stderr on
            // simctl's separate fds, so we can colour them apart.
            cmd.arg("--console");
        }
        cmd.args([udid.as_str(), &bundle_id]);
        for (k, v) in &spec.envs {
            cmd.env(format!("SIMCTL_CHILD_{k}"), v);
        }
        if let Some(locale) = &spec.locale {
            cmd.env("SIMCTL_CHILD_DAY_LOCALE", locale);
        }
        status(
            "Launching",
            &format!("ios-uikit ({bundle_id}) on simulator {udid}"),
        );
        if spec.attached {
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| format!("simctl launch: {e}"))?;
            crate::signals::register_child(child.id());
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            // Multi-sim runs tag each stream with the UDID so the interleaved logs read apart.
            let (out_label, err_label) = if multi {
                (
                    format!("{}:{}", outcome.target, udid),
                    format!("{}:{}", outcome.target, udid),
                )
            } else {
                (outcome.target.to_string(), outcome.target.to_string())
            };
            log_threads.push(std::thread::spawn(move || {
                let t1 = stdout.map(|s| stream_logs_labeled(out_label, LogStream::Out, s));
                let t2 = stderr.map(|s| stream_logs_labeled(err_label, LogStream::Err, s));
                let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                if let Some(t) = t1 {
                    let _ = t.join();
                }
                if let Some(t) = t2 {
                    let _ = t.join();
                }
                code
            }));
        } else {
            run_logged(&mut cmd, &format!("simctl launch ({udid})"))?;
        }
    }
    Ok(std::thread::spawn(move || {
        let mut code = 0;
        for t in log_threads {
            if let Ok(c) = t.join()
                && c != 0
                && code == 0
            {
                code = c;
            }
        }
        code
    }))
}

/// Like `ops::stream_logs` but with an owned label (so per-device threads can carry a serial/UDID).
fn stream_logs_labeled(
    label: String,
    stream: LogStream,
    src: impl std::io::Read + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(src).lines().map_while(Result::ok) {
            emit_log(&label, stream, &line);
        }
    })
}

// ---------------------------------------------------------------------------
// android-widget (gradle + adb) — scaffold lands next; see gradle_backend_build
// ---------------------------------------------------------------------------

pub fn gradle_backend_build() -> i32 {
    // Invoked by the gradle scaffold with DAY_PROJECT_ROOT + DAY_PROFILE + DAY_OUT set.
    let root = match std::env::var("DAY_PROJECT_ROOT") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            eprintln!("day gradle-backend: DAY_PROJECT_ROOT unset (run via the gradle scaffold)");
            return 2;
        }
    };
    let profile = std::env::var("DAY_PROFILE").unwrap_or_else(|_| "debug".into());
    let out = std::env::var("DAY_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("build/day/jniLibs"));
    let project = match find_project(Some(&root)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("day gradle-backend: {e}");
            return 2;
        }
    };
    build_android_so(&project, &profile, &out, &android_build_abis())
        .map(|_| 0)
        .unwrap_or(4)
}

/// A connected Android device or emulator, with the ABI it actually runs (queried, not guessed —
/// an emulator matches the host arch, a phone is usually arm64, so we ask each one).
pub(crate) struct AndroidDevice {
    pub serial: String,
    pub abi: String,
}

/// `adb` with an optional device selector (`-s <serial>`). Multi-device installs/launches MUST
/// pin the serial, or adb errors ("more than one device/emulator").
fn adb(serial: Option<&str>) -> Command {
    let mut c = Command::new("adb");
    if let Some(s) = serial {
        c.args(["-s", s]);
    }
    c
}

/// Every device in `adb devices` in the `device` state, paired with its primary ABI
/// (`ro.product.cpu.abi`). `DAY_ANDROID_ABI`, when set, overrides the queried ABI for every device
/// (CI's KVM emulator leg pins `x86_64`). Empty when nothing is connected.
pub(crate) fn android_devices() -> Vec<AndroidDevice> {
    let forced = std::env::var("DAY_ANDROID_ABI").ok();
    let out = match Command::new("adb").arg("devices").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1) // "List of devices attached"
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?;
            if it.next() != Some("device") {
                return None; // skip offline/unauthorized
            }
            let abi = forced.clone().unwrap_or_else(|| {
                Command::new("adb")
                    .args(["-s", serial, "shell", "getprop", "ro.product.cpu.abi"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "arm64-v8a".into())
            });
            Some(AndroidDevice {
                serial: serial.to_string(),
                abi,
            })
        })
        .collect()
}

/// The set of ABIs to build for: the distinct ABIs of the connected devices, or — with nothing
/// connected (e.g. `day build` on CI before the emulator boots) — the `DAY_ANDROID_ABI` override
/// or the arm64-v8a default, so packaging still succeeds.
fn android_build_abis() -> Vec<String> {
    let mut abis: Vec<String> = android_devices().into_iter().map(|d| d.abi).collect();
    abis.sort();
    abis.dedup();
    if abis.is_empty() {
        abis.push(std::env::var("DAY_ANDROID_ABI").unwrap_or_else(|_| "arm64-v8a".into()));
    }
    abis
}

/// Cross-compile the app cdylib for every ABI in `abis` into `out/<abi>/lib<name>.so` (one
/// `cargo ndk -t <abi> …` invocation covering them all).
fn build_android_so(
    project: &Project,
    profile: &str,
    out: &Path,
    abis: &[String],
) -> Result<(), String> {
    let (cargo, bin) = rustup_cargo()?;
    let name = project.manifest.app.name.clone();
    let ndk_home = find_ndk()?;
    let target_dir = project
        .root
        .join("build/day/cargo/android-widget")
        .join(profile);
    let mut cmd = Command::new(&cargo);
    cmd.current_dir(&project.root)
        .env(
            "PATH",
            format!(
                "{}:{}/.cargo/bin:{}",
                bin.display(),
                std::env::var("HOME").unwrap_or_default(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("ANDROID_NDK_HOME", &ndk_home)
        .arg("ndk");
    for abi in abis {
        cmd.args(["-t", abi]);
    }
    cmd.arg("-o")
        .arg(out)
        // `rustc --crate-type cdylib` so the app lib's manifest can stay rlib-only (see the
        // `[lib]` note in the app Cargo.toml); produces the same `lib<name>.so` this expects.
        // `--features` = `widget` + every standalone piece's `<pkg>/widget` renderer feature (Tier
        // A.2), so the app needn't re-list per-piece features in its own Cargo.toml.
        .arg("rustc")
        .args([
            "-p",
            &name,
            "--lib",
            "--crate-type",
            "cdylib",
            "--no-default-features",
            "--features",
            &crate::ops::feature_selection(project, "widget"),
        ]);
    if profile == "release" {
        cmd.arg("--release");
    }
    run_logged(&mut cmd, "cargo ndk")?;
    Ok(())
}

/// The Android SDK root: `ANDROID_HOME`, else `ANDROID_SDK_ROOT`, else the macOS default location.
/// Shared with `day doctor` so its diagnosis matches what the build actually probes.
pub(crate) fn android_sdk_dir() -> PathBuf {
    // Shared lookup: ANDROID_HOME / ANDROID_SDK_ROOT, then the per-OS default install location
    // (docs/environment.md).
    day_toolchain::android_sdk_dir()
}

pub(crate) fn find_ndk() -> Result<PathBuf, String> {
    if let Ok(v) = std::env::var("ANDROID_NDK_HOME") {
        return Ok(PathBuf::from(v));
    }
    let sdk = android_sdk_dir();
    let ndk_dir = sdk.join("ndk");
    let mut versions: Vec<_> = std::fs::read_dir(&ndk_dir)
        .map_err(|_| "no Android NDK found (set ANDROID_NDK_HOME)")?
        .flatten()
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions.pop().ok_or_else(|| "empty ndk dir".into())
}

pub fn build_android(
    project: &Project,
    target: &'static Target,
    profile: &str,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    // 1) Rust .so, one per connected device's ABI (so an app built with an arm64 phone AND an
    //    x86_64 emulator attached carries both). Also invoked by gradle's callback; building here
    //    keeps `day build` primary.
    let jni_out = project.root.join("build/day/jniLibs");
    let abis = android_build_abis();
    status(
        "Building",
        &format!("{} (cargo-ndk {})", target.name, abis.join(" ")),
    );
    build_android_so(project, profile, &jni_out, &abis)?;

    // Convey day.yaml identity/version to the Gradle scaffold (§17.5) on every build, so
    // applicationId/versionCode/versionName never go stale in the checked-in scaffold.
    crate::pack::android::write_app_properties(project)?;

    // 2) Discover standalone-piece Android contributions (own Java / Gradle deps) and stage them
    //    for the Gradle build to pick up — a piece ships its backend without editing Day.
    crate::pieces::write_android_manifest(project)?;

    // 3) Gradle assemble.
    let task = if profile == "release" {
        "assembleRelease"
    } else {
        "assembleDebug"
    };
    status("Building", &format!("{} (gradle {task})", target.name));
    let day_bin = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("gradle");
    cmd.current_dir(project.root.join("platform/android"))
        .env("DAY_BIN", &day_bin)
        .env("DAY_PROJECT_ROOT", &project.root)
        .env("DAY_PROFILE", profile)
        .args([task, "-q", "--console=plain"]);
    // Gradle 9 + AGP 9 need JDK 17–21 (newer JDKs break the AGP jdk-image transform). Respect
    // the caller's JAVA_HOME (CI pins 21 via setup-java); default to Homebrew's 21 when unset.
    if std::env::var_os("JAVA_HOME").is_none()
        && let Some(jdk) = day_toolchain::jdk21_home()
    {
        cmd.env("JAVA_HOME", jdk);
    }
    let out = cmd.output().map_err(|e| format!("gradle: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = text.lines().rev().take(30).collect();
        return Err(format!(
            "gradle failed:\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    let apk_name = if profile == "release" {
        "app-release.apk"
    } else {
        "app-debug.apk"
    };
    let apk_dir = project
        .root
        .join("platform/android/app/build/outputs/apk")
        .join(profile);
    // An unsigned release build is emitted as `app-release-unsigned.apk` — fall back to whatever
    // single .apk the build produced rather than assuming the signed name.
    let conventional = apk_dir.join(apk_name);
    let apk = if conventional.exists() {
        conventional
    } else {
        std::fs::read_dir(&apk_dir)
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|x| x.to_str()) == Some("apk"))
            })
            .unwrap_or(conventional)
    };
    Ok(BuildOutcome {
        target: target.name,
        artifact: apk,
        seconds: start.elapsed().as_secs_f64(),
    })
}

pub fn launch_android(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let app_id = project.manifest.app.id.clone();
    let devices = android_devices();
    if devices.is_empty() {
        return Err("no Android device/emulator connected (check `adb devices`)".into());
    }
    // Install + launch on EVERY connected device; the one APK already carries each device's ABI.
    let mut log_threads = Vec::new();
    for dev in &devices {
        run_logged(
            adb(Some(&dev.serial))
                .args(["install", "-r"])
                .arg(&outcome.artifact),
            &format!("adb install ({})", dev.serial),
        )?;
        // adb shell joins args into ONE device-shell command line — extras must be shell-quoted.
        let mut cmd = adb(Some(&dev.serial));
        cmd.args([
            "shell",
            "am",
            "start",
            "-n",
            &format!("{app_id}/dev.daybrite.day.bridge.DayActivity"),
        ]);
        for (k, v) in &spec.envs {
            let quoted = format!("'{}'", v.replace('\'', ""));
            if k == "AUTODRIVE" {
                cmd.args(["--es", "day.autodrive", &quoted]);
            } else {
                cmd.args(["--es", &format!("day.env.{k}"), &quoted]);
            }
        }
        if let Some(locale) = &spec.locale {
            cmd.args(["--es", "day.locale", &format!("'{locale}'")]);
        }
        status(
            "Launching",
            &format!("android-widget ({app_id}) on {} ({})", dev.serial, dev.abi),
        );
        run_logged(&mut cmd, &format!("am start ({})", dev.serial))?;
        if spec.attached {
            // One-device runs keep the bare `[android-widget]` prefix; multi-device runs append
            // the serial so the interleaved log streams read apart.
            let label = if devices.len() > 1 {
                format!("{}:{}", outcome.target, dev.serial)
            } else {
                outcome.target.to_string()
            };
            log_threads.push(stream_logcat(dev.serial.clone(), app_id.clone(), label));
        }
    }
    // The returned handle joins every device's log pump; its exit code is the first non-zero.
    Ok(std::thread::spawn(move || {
        let mut code = 0;
        for t in log_threads {
            if let Ok(c) = t.join()
                && c != 0
                && code == 0
            {
                code = c;
            }
        }
        code
    }))
}

/// Stream one device's app logs (day-android redirects the app's stdout/stderr into logcat under
/// tag `day`). `-v tag` prefixes each line with `<prio>/day:`; map the priority to a stream
/// (I→stdout/blue, E/W/F→stderr/yellow) and re-prefix with `label`.
fn stream_logcat(serial: String, app_id: String, label: String) -> std::thread::JoinHandle<i32> {
    std::thread::spawn(move || {
        let pid = (0..20)
            .find_map(|_| {
                let p = adb(Some(&serial))
                    .args(["shell", "pidof", "-s", &app_id])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                if p.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    None
                } else {
                    Some(p)
                }
            })
            .unwrap_or_default();
        if pid.is_empty() {
            emit_log(
                &label,
                LogStream::Err,
                "app pid not found; logs unavailable",
            );
            return 1;
        }
        // Clear this device's backlog so we only stream this run's output.
        let _ = adb(Some(&serial)).args(["logcat", "-c"]).status();
        let mut child = match adb(Some(&serial))
            .args(["logcat", "--pid", &pid, "-v", "tag", "day:V", "*:S"])
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                emit_log(&label, LogStream::Err, &format!("adb logcat: {e}"));
                return 1;
            }
        };
        crate::signals::register_child(child.id());
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let (prio, msg) = match line.split_once(':') {
                    Some((head, rest)) => {
                        (head.trim().chars().next().unwrap_or('I'), rest.trim_start())
                    }
                    None => ('I', line.as_str()),
                };
                let stream = if prio == 'E' || prio == 'F' || prio == 'W' {
                    LogStream::Err
                } else {
                    LogStream::Out
                };
                emit_log(&label, stream, msg);
            }
        }
        child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0)
    })
}
