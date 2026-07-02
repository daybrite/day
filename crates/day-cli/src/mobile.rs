//! Mobile pipelines (DESIGN.md §16.5, §17.4): ios-uikit via xcodebuild + simctl (the Xcode
//! project's script phase calls back into `day xcode-backend build` for the Rust staticlib);
//! android-widget via gradle + adb (the gradle scaffold calls `day gradle-backend build`).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::meta::{Project, find_project};
use crate::ops::{BuildOutcome, LaunchSpec, LogStream, emit_log, status, stream_logs};
use crate::targets::Target;

fn rustup_cargo() -> Result<(PathBuf, PathBuf), String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let toolchains = PathBuf::from(&home).join(".rustup/toolchains");
    let entry = std::fs::read_dir(&toolchains)
        .map_err(|_| "no rustup toolchains (cross-std needs rustup, not Homebrew rust)")?
        .flatten()
        .next()
        .ok_or("empty rustup toolchains dir")?;
    let bin = entry.path().join("bin");
    Ok((bin.join("cargo"), bin))
}

fn run_logged(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if out.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
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
        .args([
            "build",
            "-p",
            &name,
            "--lib",
            "--no-default-features",
            "--features",
            "uikit",
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
    let out_dir = built_products.join("day");
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
    let symroot = project.root.join("build/day/ios-uikit");
    let day_bin = std::env::current_exe().map_err(|e| e.to_string())?;
    status(
        "Building",
        &format!("{} (xcodebuild {configuration})", target.name),
    );
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
        .stdout(std::process::Stdio::piped());
    let out = cmd.output().map_err(|e| format!("xcodebuild: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout);
        let tail: Vec<&str> = text.lines().rev().take(30).collect();
        return Err(format!(
            "xcodebuild failed:\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    let app = symroot
        .join(format!("{configuration}-iphonesimulator"))
        .join("Showcase.app");
    Ok(BuildOutcome {
        target: target.name,
        artifact: app,
        seconds: start.elapsed().as_secs_f64(),
    })
}

pub fn launch_ios(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let bundle_id = project.manifest.app.id.clone();
    run_logged(
        Command::new("xcrun")
            .args(["simctl", "install", "booted"])
            .arg(&outcome.artifact),
        "simctl install",
    )?;
    let _ = Command::new("xcrun")
        .args(["simctl", "terminate", "booted", &bundle_id])
        .status();
    let mut cmd = Command::new("xcrun");
    cmd.args(["simctl", "launch"]);
    if spec.attached {
        // `--console` (not `--console-pty`) keeps the app's stdout and stderr on
        // simctl's separate fds, so we can colour them apart.
        cmd.arg("--console");
    }
    cmd.args(["booted", &bundle_id]);
    for (k, v) in &spec.envs {
        cmd.env(format!("SIMCTL_CHILD_{k}"), v);
    }
    if let Some(locale) = &spec.locale {
        cmd.env("SIMCTL_CHILD_DAY_LOCALE", locale);
    }
    status(
        "Launching",
        &format!("ios-uikit ({bundle_id}) on the booted simulator"),
    );
    if spec.attached {
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("simctl launch: {e}"))?;
        crate::signals::register_child(child.id());
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let name = outcome.target;
        Ok(std::thread::spawn(move || {
            let t1 = stdout.map(|s| stream_logs(name, LogStream::Out, s));
            let t2 = stderr.map(|s| stream_logs(name, LogStream::Err, s));
            let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
            if let Some(t) = t1 {
                let _ = t.join();
            }
            if let Some(t) = t2 {
                let _ = t.join();
            }
            code
        }))
    } else {
        run_logged(&mut cmd, "simctl launch")?;
        Ok(std::thread::spawn(|| 0))
    }
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
    build_android_so(&project, &profile, &out)
        .map(|_| 0)
        .unwrap_or(4)
}

/// Android ABI to build (`DAY_ANDROID_ABI` override; default arm64-v8a). CI's KVM emulator
/// runners are x86_64-only, so the e2e leg exports `DAY_ANDROID_ABI=x86_64` (§20).
fn android_abi() -> String {
    std::env::var("DAY_ANDROID_ABI").unwrap_or_else(|_| "arm64-v8a".into())
}

fn build_android_so(project: &Project, profile: &str, out: &Path) -> Result<PathBuf, String> {
    let (cargo, bin) = rustup_cargo()?;
    let name = project.manifest.app.name.clone();
    let ndk_home = find_ndk()?;
    let abi = android_abi();
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
        .args(["ndk", "-t", &abi, "-o"])
        .arg(out)
        .arg("build")
        .args([
            "-p",
            &name,
            "--lib",
            "--no-default-features",
            "--features",
            "widget",
        ]);
    if profile == "release" {
        cmd.arg("--release");
    }
    run_logged(&mut cmd, "cargo ndk")?;
    Ok(out.join(&abi).join(format!("lib{name}.so")))
}

fn find_ndk() -> Result<PathBuf, String> {
    if let Ok(v) = std::env::var("ANDROID_NDK_HOME") {
        return Ok(PathBuf::from(v));
    }
    let sdk = std::env::var("ANDROID_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/Android/sdk")
        });
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
    // 1) Rust .so (also invoked by gradle's callback; building here keeps `day build` primary).
    let jni_out = project.root.join("build/day/jniLibs");
    status(
        "Building",
        &format!("{} (cargo-ndk arm64-v8a)", target.name),
    );
    build_android_so(project, profile, &jni_out)?;

    // 2) Gradle assemble.
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
    if std::env::var_os("JAVA_HOME").is_none() {
        let brew_jdk = Path::new("/opt/homebrew/opt/openjdk@21");
        if brew_jdk.exists() {
            cmd.env("JAVA_HOME", brew_jdk);
        }
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
    let apk = project
        .root
        .join("platform/android/app/build/outputs/apk")
        .join(profile)
        .join(apk_name);
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
    run_logged(
        Command::new("adb")
            .args(["install", "-r"])
            .arg(&outcome.artifact),
        "adb install",
    )?;
    // adb shell joins args into ONE device-shell command line — extras must be shell-quoted.
    let mut cmd = Command::new("adb");
    cmd.args([
        "shell",
        "am",
        "start",
        "-n",
        &format!("{app_id}/dev.day.bridge.DayActivity"),
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
        &format!("android-widget ({app_id}) on the connected device"),
    );
    run_logged(&mut cmd, "am start")?;
    if spec.attached {
        // Stream the app's stdout/stderr (redirected into logcat under tag `day` by
        // day-android). `-v tag` prefixes each line with `<prio>/day:`; we map the
        // priority to a stream (I→stdout/blue, E→stderr/yellow) and re-prefix.
        let id = app_id.clone();
        let name = outcome.target;
        Ok(std::thread::spawn(move || {
            let pid = (0..20)
                .find_map(|_| {
                    let p = Command::new("adb")
                        .args(["shell", "pidof", "-s", &id])
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
                emit_log(name, LogStream::Err, "app pid not found; logs unavailable");
                return 1;
            }
            // Clear the backlog so we only stream this run's output.
            let _ = Command::new("adb").args(["logcat", "-c"]).status();
            let mut child = match Command::new("adb")
                .args(["logcat", "--pid", &pid, "-v", "tag", "day:V", "*:S"])
                .stdout(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    emit_log(name, LogStream::Err, &format!("adb logcat: {e}"));
                    return 1;
                }
            };
            crate::signals::register_child(child.id());
            if let Some(out) = child.stdout.take() {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    // `-v tag` line: "<P>/day: <message>" (or "<P>/day( pid): ..." on some
                    // builds); split off the priority and the tag header.
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
                    emit_log(name, stream, msg);
                }
            }
            child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0)
        }))
    } else {
        Ok(std::thread::spawn(|| 0))
    }
}
