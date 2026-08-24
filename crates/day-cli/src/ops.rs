// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Build / launch operations. Desktop = cargo with per-(target, profile) CARGO_TARGET_DIR
//! (§16.5 — parallel targets never contend on the cargo build-dir lock). Mobile pipelines
//! attach here at M5 (xcodebuild + simctl; gradle + adb).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::cli::Profile;
use crate::meta::Project;
use crate::targets::{Target, TargetKind};
use crate::term::{HEADER, LOG_DEBUG, LOG_ERR, LOG_ERROR, LOG_INFO, LOG_OUT, LOG_TRACE, LOG_WARN};

pub struct BuildOutcome {
    pub target: &'static str,
    pub artifact: PathBuf,
    pub seconds: f64,
}

pub(crate) fn cargo_dir(project: &Project, target: &Target, profile: Profile) -> PathBuf {
    project
        .root
        .join("build/day/cargo")
        .join(target.name)
        .join(profile.as_str())
}

pub fn status(prefix: &str, msg: &str) {
    anstream::eprintln!("{HEADER}{prefix:>12}{HEADER:#} {msg}");
}

/// `--verbose` (global flag): forward every sub-command's raw stdout/stderr to the terminal as it
/// runs, instead of capturing it and showing only day's own status lines (and the tool's output on
/// failure). Set once from the parsed flag in `cli::run`; read by the tool-runner helpers.
static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record whether `--verbose` was passed (called once at startup).
pub fn set_verbose(on: bool) {
    VERBOSE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `--verbose` is in effect — build tools consult this to decide whether to forward their
/// sub-commands' output.
pub fn verbose() -> bool {
    VERBOSE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether this process runs inside a GitHub Actions job — the documented signal is
/// `GITHUB_ACTIONS=true`, set for every step of every runner. Shared by the commands that report
/// into a job (`day lint`'s findings, `day checkup`'s combo table).
pub fn github_actions() -> bool {
    std::env::var("GITHUB_ACTIONS").is_ok_and(|v| v == "true")
}

/// Escape a message for a `::warning::`/`::error::` workflow command: GitHub terminates the
/// command at a literal newline and treats `%` as the escape lead-in.
pub fn gha_escape(msg: &str) -> String {
    msg.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Run `cmd` to completion, capturing its stdout and stderr. Under [`verbose`] each stream is ALSO
/// forwarded — verbatim — to day's own logging stream (**stderr**) as it arrives, so a
/// sub-command's raw output streams live while the captured copy still feeds day's own failure
/// diagnostics (the `run_quiet`/`run_tool`/gradle/xcodebuild error text). Forwarding goes to stderr
/// (not stdout) for two reasons: it is where day already writes its status lines, and it keeps
/// stdout clean for `--format json`'s NDJSON result stream (raw tool bytes there would corrupt it).
/// Both pipes are drained concurrently — a tool that fills one while day only read the other would
/// deadlock.
///
/// The default (non-verbose) result is byte-identical to [`Command::output`], so callers that
/// filter on the captured `Output` are unaffected.
pub(crate) fn run_capture(cmd: &mut Command, what: &str) -> Result<std::process::Output, String> {
    use std::io::{Read, Write};
    // stdin closed to match `Command::output`'s contract (a child reading stdin gets immediate EOF
    // rather than the parent's terminal); these tool calls never read it.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: {e}"))?;
    let forward = verbose();
    // piped just above, so both handles are always Some.
    let mut child_out = child.stdout.take().expect("stdout was piped");
    let mut child_err = child.stderr.take().expect("stderr was piped");
    // Byte-chunk tee (not line-based): forwards output verbatim — partial lines and `\r` progress
    // redraws included — so `--verbose` is truly unfiltered, and each chunk is flushed so it lands
    // live rather than sitting in a block buffer when the destination is a pipe.
    fn tee(src: &mut impl Read, forward: bool) -> Vec<u8> {
        let mut collected = Vec::new();
        let mut chunk = [0u8; 8192];
        while let Ok(n) = src.read(&mut chunk) {
            if n == 0 {
                break;
            }
            if forward {
                let mut w = std::io::stderr().lock();
                let _ = w.write_all(&chunk[..n]);
                let _ = w.flush();
            }
            collected.extend_from_slice(&chunk[..n]);
        }
        collected
    }
    let out_reader = std::thread::spawn(move || tee(&mut child_out, forward));
    let stderr = tee(&mut child_err, forward);
    let stdout = out_reader.join().unwrap_or_default();
    let status = child.wait().map_err(|e| format!("{what}: {e}"))?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Run a command to completion, giving up after `limit` and killing the child. `None` means it
/// never finished — or never started.
///
/// Device tooling is what this exists for. `adb`, `hdc` and `simctl` do not fail against a
/// device that has stopped answering; they wait for it, with no deadline of their own. So the
/// cleanup that follows a lost engine — force-stop the app, read the crash buffer — is exactly
/// where a run stops making progress, and a CI job then sits until its own timeout hours later,
/// having already printed the diagnosis it was asked for.
pub fn status_within(cmd: &mut Command, limit: Duration) -> Option<std::process::ExitStatus> {
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Err(_) => return None,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(POLL);
    }
}

/// [`status_within`] for a command whose output is wanted. The pipes are drained on their own
/// threads: a child that fills a pipe buffer would otherwise never exit, and the poll below
/// would wait out the whole limit for a command that had already said everything.
pub fn output_within(cmd: &mut Command, limit: Duration) -> Option<std::process::Output> {
    use std::io::Read;
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let (mut out, mut err) = (child.stdout.take()?, child.stderr.take()?);
    let o = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let e = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(_) => return None,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(POLL);
    };
    Some(std::process::Output {
        status,
        stdout: o.join().unwrap_or_default(),
        stderr: e.join().unwrap_or_default(),
    })
}

/// How often the two waits above look at the child. Short enough that a fast command is not
/// noticeably delayed, long enough that a slow one costs nothing to watch.
const POLL: Duration = Duration::from_millis(50);

/// Ceilings for the device-tool calls on the install/launch path. `adb`, `simctl`, `devicectl`
/// and `hdc` wait for an unresponsive device with no deadline of their own (the same wedge the
/// post-mortem paths above already guard against), so every install and launch call runs under
/// one of these. Generous on purpose: a cold emulator install takes minutes, never ten.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(180);
/// gradle / hvigor assemble the whole app host project — a first run downloads dependencies,
/// so the ceiling is an hour: far above any real build, far below a job timeout.
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(3600);

/// [`run_capture`] with a deadline: the child is killed when `limit` passes and the timeout is
/// reported as an error naming the tool, so a wedged device shows up in minutes instead of
/// holding the run until a job timeout does it.
pub(crate) fn run_capture_within(
    cmd: &mut Command,
    what: &str,
    limit: Duration,
) -> Result<std::process::Output, String> {
    use std::io::{Read, Write};
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: {e}"))?;
    let forward = verbose();
    // piped just above, so both handles are always Some.
    let mut child_out = child.stdout.take().expect("stdout was piped");
    let mut child_err = child.stderr.take().expect("stderr was piped");
    // The same byte-chunk tee as `run_capture`, on a thread per stream — killing the child
    // closes the pipes, which is what lets the reads (and the join below) finish.
    fn tee(src: &mut impl Read, forward: bool) -> Vec<u8> {
        let mut collected = Vec::new();
        let mut chunk = [0u8; 8192];
        while let Ok(n) = src.read(&mut chunk) {
            if n == 0 {
                break;
            }
            if forward {
                let mut w = std::io::stderr().lock();
                let _ = w.write_all(&chunk[..n]);
                let _ = w.flush();
            }
            collected.extend_from_slice(&chunk[..n]);
        }
        collected
    }
    let out_reader = std::thread::spawn(move || tee(&mut child_out, forward));
    let err_reader = std::thread::spawn(move || tee(&mut child_err, forward));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(e) => return Err(format!("{what}: {e}")),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_reader.join();
            let _ = err_reader.join();
            return Err(timeout_message(what, limit));
        }
        std::thread::sleep(POLL);
    };
    Ok(std::process::Output {
        status,
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
    })
}

/// The one wording for a tool that hit its deadline, naming the tool so the reader knows what
/// to go look at (usually a device that stopped answering).
pub(crate) fn timeout_message(what: &str, limit: Duration) -> String {
    format!(
        "{what} did not finish within {}s and was killed — the tool looks wedged; check the \
         device/emulator and run again",
        limit.as_secs()
    )
}

/// The exit code to report for a finished child, with a signal death made VISIBLE.
///
/// `ExitStatus::code()` is `None` when a process was killed by a signal, and mapping that to 0 —
/// which every call site here used to do — turned an app that aborted or segfaulted into a clean
/// exit: `day launch` returned success after a crash, and nothing downstream could tell that the
/// app had died badly. `128 + signo` is the shell's convention (SIGABRT ⇒ 134, SIGSEGV ⇒ 139).
pub fn exit_code_of(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signo) = status.signal() {
            return 128 + signo;
        }
    }
    1
}

/// Whether an exit code says the process was killed by a fatal signal — a crash, not a quit.
pub fn died_on_signal(code: i32) -> bool {
    // 128 + {SIGILL, SIGABRT, SIGBUS(both spellings), SIGFPE, SIGSEGV, SIGSYS}
    matches!(code, 132 | 134 | 135 | 136 | 138 | 139 | 141)
}

/// Export the app identity (Day.toml `[app]`) to a cargo/build or launch command. day-break's
/// `build.rs` bakes these into the binary so crash reports carry id/version/build without
/// reading platform manifests at runtime (docs/break.md); on launch commands they double as the
/// runtime fallback for dev flows whose binary predates the vars.
pub fn apply_app_identity(cmd: &mut Command, project: &Project) {
    for (k, v) in app_identity_env(project) {
        cmd.env(k, v);
    }
}

/// The same variables as a map, for callers that report an environment instead of spawning with
/// one (`desktop_launch_plan`, and `day build --format json` through it).
pub fn app_identity_env(project: &Project) -> BTreeMap<String, OsString> {
    let mut env = BTreeMap::from([
        (
            "DAY_APP_ID".to_string(),
            project.manifest.app.id.clone().into(),
        ),
        (
            "DAY_APP_VERSION".to_string(),
            project.manifest.app.version.clone().into(),
        ),
        (
            "DAY_APP_BUILD".to_string(),
            project.manifest.app.build.to_string().into(),
        ),
    ]);
    env.extend(determinism_env());
    env
}

/// Environment that makes Apple's toolchain stop stamping the clock into its output
/// (DESIGN.md §20.3).
///
/// `libtool` and `ld64` write file modification times into static archives and into the debug map's
/// `OSO` entries, so two builds of identical sources differ by whenever they happened to run.
/// `ZERO_AR_DATE` zeroes both. It is set here rather than in CI so local packs are deterministic
/// too — reproducibility that only holds on the build farm is not worth much.
///
/// Scope: archive and debug-map timestamps only. It does NOT touch `__DATE__`/`__TIME__` (Day uses
/// neither), and it is inert on non-Apple hosts.
pub fn apply_determinism(cmd: &mut Command) {
    for (k, v) in determinism_env() {
        cmd.env(k, v);
    }
}

/// The determinism variables as a map — see `app_identity_env` for why the map form exists.
pub fn determinism_env() -> BTreeMap<String, OsString> {
    BTreeMap::from([
        ("ZERO_AR_DATE".to_string(), OsString::from("1")),
        // Export the resolved epoch so any SOURCE_DATE_EPOCH-aware tool downstream agrees with the
        // value Day stamps into archives itself — flatpak-builder honors it (1.3.1+), as do many
        // compilers and archivers. Passing through the caller's value when they set one, and Day's
        // default otherwise, means one clock governs the whole pack.
        (
            "SOURCE_DATE_EPOCH".to_string(),
            crate::pack::reproducible_epoch().to_string().into(),
        ),
    ])
}

/// The comma-joined `--features` string for a `backend` toolkit: the toolkit feature itself plus the
/// unioned `<pkg>/<backend>` renderer feature of every standalone piece in the app's dependency
/// closure (Tier A.2 — apps no longer fan out per-piece features in their own Cargo.toml).
pub fn feature_selection(project: &Project, backend: &str) -> String {
    let mut features = vec![backend.to_string()];
    features.extend(crate::pieces::feature_union(project, backend));
    features.join(",")
}

/// Where [`build`] records the last successful artifact path for a (target, profile) — the
/// `--skip-build` reuse stamp. One line, the absolute artifact path.
fn artifact_stamp(project: &Project, target: &Target, profile: Profile) -> PathBuf {
    project
        .root
        .join("build/day/artifacts")
        .join(format!("{}-{profile}.path", target.name))
}

/// Reuse the previous [`build`]'s artifact instead of building (`day launch --skip-build`):
/// the artifact is read from the stamp and must still exist. For runs whose variants share one
/// binary (theme/locale are runtime inputs), this drops the per-invocation build overhead —
/// CI's iOS walkthrough pays xcodebuild once instead of once per variant.
pub fn reuse_build(
    project: &Project,
    target: &'static Target,
    profile: Profile,
) -> Result<BuildOutcome, String> {
    let stamp = artifact_stamp(project, target, profile);
    let artifact = std::fs::read_to_string(&stamp)
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.exists())
        .ok_or_else(|| {
            format!(
                "--skip-build: no reusable {} {profile} artifact — build once without the flag first",
                target.name
            )
        })?;
    status(
        "Reusing",
        &format!("{} → {}", target.name, artifact.display()),
    );
    Ok(BuildOutcome {
        target: target.name,
        artifact,
        seconds: 0.0,
    })
}

/// Build for a physical device instead of a simulator/emulator. Separate from [`build`] so the
/// eight callers that can only ever mean "the usual build" keep saying exactly that. Only iOS
/// differs today; every other target builds one artifact that runs in both places.
pub fn build_for_device(
    project: &Project,
    target: &'static Target,
    profile: Profile,
) -> Result<BuildOutcome, String> {
    let start = std::time::Instant::now();
    match target.kind {
        TargetKind::IosSim => {
            let outcome = crate::mobile::build_ios_for(project, target, profile, start, true)?;
            Ok(outcome)
        }
        _ => build(project, target, profile),
    }
}

pub fn build(
    project: &Project,
    target: &'static Target,
    profile: Profile,
) -> Result<BuildOutcome, String> {
    let host = crate::targets::host_os();
    if target.host != "any" && target.host != host {
        return Err(format!(
            "target {} builds on a {} host (this is {})",
            target.name, target.host, host
        ));
    }
    let start = std::time::Instant::now();
    // Stage declared resources (images/ + assets/) into this target's native locations before its
    // platform build runs, so actool/aapt2/rcc/hvigor can process them (§18.3). Best-effort: this
    // needs the toolkit's native resource compiler (rcc / glib-compile-resources / …), which isn't
    // always on PATH (e.g. MSYS2 windows-qt/windows-gtk ship no rcc/glib-compile-resources). When
    // it's missing the resource blob is simply skipped — day loads assets from the filesystem roots
    // (DAY_IMAGE_ROOT) and the app icon rides DAY_APP_ICON — so a missing tool must NOT fail the build.
    if let Err(e) = crate::resources::stage(project, target) {
        status("Warning", &format!("resource staging skipped ({e})"));
    }
    // Day.toml [[shortcuts]] → staged Android shortcut resources, AFTER the image stage that
    // wipes the res tree (docs/deep-links.md "Shortcuts are saved deep links"). Unlike the
    // best-effort staging above, a failure here is a config error (a missing translation, a
    // manifest with no scheme), so it fails the build.
    if target.toolkit == "mdc" {
        crate::shortcuts::sync_android(project)?;
    }
    // macos-appkit through the Xcode host project when the app carries one (§17.4,
    // platform/macos/): a real bundle with identity, icon, and staged resources — the same
    // build a developer gets pressing Run in Xcode. `DAY_MACOS_XCODE=0` opts back into the
    // bare-cargo path (CI capture loops). Falls through to the shared artifact stamping
    // below, so `--skip-build` reuse works on this path too.
    let xcode_macos = target.name == "macos-appkit" && crate::mobile::macos_xcode_enabled(project);
    let outcome = if xcode_macos {
        crate::mobile::build_macos_xcode(project, target, profile, start)
    } else {
        build_native(project, target, profile, start)
    }?;
    // Record the artifact for `--skip-build` reuse ([`reuse_build`]). Best-effort — a failed
    // stamp write must never fail a successful build.
    let stamp = artifact_stamp(project, target, profile);
    if let Some(dir) = stamp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&stamp, outcome.artifact.display().to_string());
    Ok(outcome)
}

/// The per-kind native build pipelines (the pre-dispatch body of [`build`]).
fn build_native(
    project: &Project,
    target: &'static Target,
    profile: Profile,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let outcome = match target.kind {
        TargetKind::Desktop => {
            let mut cmd = Command::new("cargo");
            cmd.current_dir(&project.root)
                .env("CARGO_TARGET_DIR", cargo_dir(project, target, profile));
            apply_app_identity(&mut cmd, project);
            crate::bridge::apply_staged(&mut cmd, project, target.name);
            // The toolkit feature (e.g. `appkit`) + every standalone piece's `<pkg>/<toolkit>`
            // renderer feature, derived from `cargo metadata` — so the app depends on a piece
            // without re-listing its per-backend feature (Tier A.2).
            let features = feature_selection(project, target.toolkit);
            // The macos-appkit Swift prepass (docs/swiftui.md): when dependencies contribute
            // macOS Swift, `swift build` the generated DayPieces package and statically link it.
            // No contributions → `swift_link` is None and the cargo command below is byte-identical
            // to a plain build (no Swift toolchain needed).
            let swift_link = if target.name == "macos-appkit" {
                match crate::pieces::write_macos_pieces(project, false)? {
                    Some(swift) => Some(crate::swift::build_day_pieces(project, profile, &swift)?),
                    None => None,
                }
            } else {
                None
            };
            if target.toolkit == "xaml" {
                // XAML Islands refuses to start unless the app manifest declares
                // `maxversiontested` (§9). rustc's default embedded manifest lacks it, so we
                // embed our own — `cargo rustc -- <link-args>` scopes this to the bin only.
                let manifest = write_xaml_manifest(project, target, profile)?;
                cmd.args(["rustc", "--bin", &project.manifest.app.name])
                    .args(["--no-default-features", "--features", &features]);
                if profile == Profile::Release {
                    cmd.arg("--release");
                }
                cmd.arg("--");
                cmd.arg("-Clink-arg=/MANIFEST:EMBED");
                cmd.arg(format!("-Clink-arg=/MANIFESTINPUT:{}", manifest.display()));
                // Reproducible PE output (§20.3): without this the linker stamps the COFF header
                // and the debug directory with the wall clock, so the same commit built twice
                // differs by exactly those bytes and nothing else. `/Brepro` substitutes a hash of
                // the input, which is what makes the .exe comparable across builds. It rides here
                // rather than in RUSTFLAGS because CI already sets RUSTFLAGS and appending to an
                // inherited value is easy to get wrong; `cargo rustc --` scopes it to this bin.
                cmd.arg("-Clink-arg=/Brepro");
            } else if let Some(link) = &swift_link {
                // Statically link the Swift prepass output. `cargo rustc -- <link-args>` scopes
                // the extra arguments to the final bin (the xaml-manifest precedent above), so
                // gaining or losing Swift contributions relinks one crate, never rebuilds the
                // world. MACOSX_DEPLOYMENT_TARGET matches the Swift objects' floor so ld doesn't
                // warn about mixed minimum versions — an app embedding SwiftUI needs that OS
                // anyway.
                cmd.env("MACOSX_DEPLOYMENT_TARGET", &link.platform);
                cmd.args(["rustc", "--bin", &project.manifest.app.name])
                    .args(["--no-default-features", "--features", &features]);
                if profile == Profile::Release {
                    cmd.arg("--release");
                }
                cmd.arg("--");
                cmd.args(link.rustc_args());
            } else {
                cmd.args([
                    "build",
                    "-p",
                    &project.manifest.app.name,
                    "--no-default-features",
                ])
                .args(["--features", &features]);
                if profile == Profile::Release {
                    cmd.arg("--release");
                }
            }
            status("Building", &format!("{} ({})", target.name, profile));
            let out = cmd.status().map_err(|e| format!("cargo: {e}"))?;
            if !out.success() {
                return Err(format!("cargo build failed for {}", target.name));
            }
            // The desktop binary carries the platform's executable extension (`.exe` on Windows,
            // none elsewhere). `day launch`'s `Command::new` auto-appends it on Windows, but the raw
            // `fs::copy` in `pack` (msix/nsis stage the exe) needs the REAL path — so bake it in here.
            let artifact = cargo_dir(project, target, profile)
                .join(profile.as_str())
                .join(format!(
                    "{}{}",
                    project.manifest.app.name,
                    std::env::consts::EXE_SUFFIX
                ));
            Ok(BuildOutcome {
                target: target.name,
                artifact,
                seconds: start.elapsed().as_secs_f64(),
            })
        }
        TargetKind::IosSim => crate::mobile::build_ios(project, target, profile, start),
        TargetKind::Android => crate::mobile::build_android(project, target, profile, start),
        TargetKind::HarmonyOs => crate::ohos::build_ohos(project, target, profile, start),
        TargetKind::Web => crate::web::build_web(project, target, profile, start),
    }?;
    Ok(outcome)
}

/// Side-by-side manifest that lets an unpackaged app host `Windows.UI.Xaml` islands (§9).
/// The `maxversiontested` element is the specific thing `WindowsXamlManager` demands.
const XAML_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<assembly manifestVersion="1.0" xmlns="urn:schemas-microsoft-com:asm.v1">
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <!-- Windows 10 and Windows 11 -->
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <maxversiontested Id="10.0.22621.0"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>
"#;

fn write_xaml_manifest(
    project: &Project,
    target: &Target,
    profile: Profile,
) -> Result<PathBuf, String> {
    let dir = cargo_dir(project, target, profile);
    std::fs::create_dir_all(&dir).map_err(|e| format!("manifest dir: {e}"))?;
    let path = dir.join("day-xaml.manifest");
    std::fs::write(&path, XAML_MANIFEST).map_err(|e| format!("manifest write: {e}"))?;
    Ok(path)
}

#[derive(Clone)]
pub struct LaunchSpec {
    pub locale: Option<String>,
    pub envs: Vec<(String, String)>,
    pub attached: bool,
    /// Device selection, one field per runtime, so a single `day launch` can name a different
    /// one for each `-p` it was given. Left `None`, a target uses every device of its kind it
    /// can see — right for a capture sweep, wrong when you mean one specific phone.
    ///
    /// The split is not cosmetic: an iOS simulator and an iOS device are different runtimes
    /// (simctl vs devicectl) and, more importantly, different BUILDS — a device needs the
    /// `iphoneos` SDK and code signing, decided before `build` runs.
    pub ios_device: Option<String>,
    pub ios_simulator: Option<String>,
    pub android_device: Option<String>,
    /// OpenHarmony connect key (`hdc -t`). Without it every reachable target gets the app, the
    /// same rule the other two runtimes follow.
    pub ohos_device: Option<String>,
}

impl LaunchSpec {
    /// Whether this launch targets a physical iOS device, which the iOS build has to know.
    pub fn wants_ios_device(&self) -> bool {
        self.ios_device.is_some()
    }
}

/// What this run actually launched onto, remembered for the steps that come AFTER the launch.
///
/// A dayscript run forwards a port and takes screenshots long after `LaunchSpec` is out of scope,
/// and those paths used to pin whichever device enumerated first — so a `--android-device` run
/// forwarded to a bystander phone, and a `--ios-simulator` run photographed the wrong screen.
/// Recording the RESOLVED identity (a simulator UDID, an adb serial, an hdc key) once at launch
/// keeps every later step on the device the user actually named.
///
/// Set-once per process: one `day launch` selects at most one device per runtime, and a second
/// write would mean the selection changed underneath a run in progress.
mod selected {
    use std::sync::OnceLock;

    pub(super) static IOS_SIMULATOR: OnceLock<String> = OnceLock::new();
    pub(super) static ANDROID_SERIAL: OnceLock<String> = OnceLock::new();
    pub(super) static OHOS_KEY: OnceLock<String> = OnceLock::new();
}

/// Record the simulator this run launched on, as a resolved UDID.
pub fn remember_ios_simulator(udid: impl Into<String>) {
    let _ = selected::IOS_SIMULATOR.set(udid.into());
}

/// Record the adb serial this run launched on.
pub fn remember_android_serial(serial: impl Into<String>) {
    let _ = selected::ANDROID_SERIAL.set(serial.into());
}

/// Record the hdc connect key this run launched on.
pub fn remember_ohos_key(key: impl Into<String>) {
    let _ = selected::OHOS_KEY.set(key.into());
}

/// The simulator UDID this run launched on, if it named one.
pub fn selected_ios_simulator() -> Option<&'static str> {
    selected::IOS_SIMULATOR.get().map(String::as_str)
}

/// The adb serial this run launched on, if it named one.
pub fn selected_android_serial() -> Option<&'static str> {
    selected::ANDROID_SERIAL.get().map(String::as_str)
}

/// The hdc connect key this run launched on, if it named one.
pub fn selected_ohos_key() -> Option<&'static str> {
    selected::OHOS_KEY.get().map(String::as_str)
}

/// Everything needed to start a desktop target's own binary: the program, its arguments, the
/// working directory, and the environment Day layers onto the caller's.
///
/// `launch` spawns exactly this, and `day build --format json` reports it verbatim — which is what
/// lets an outside debugger (the VS Code extension delegating to lldb) start the app the way Day
/// would. Having ONE producer is the point: an env var added for a launch but not mirrored here
/// would leave the app resource-less under the debugger, and only there.
pub struct DesktopLaunchPlan {
    /// The executable to start. For a macOS `.app` bundle this is the binary INSIDE it — a
    /// debugger needs a Mach-O to load, and macOS reads the adjacent Info.plist either way.
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Day's ADDITIONS to the inherited environment, not a complete environment block.
    pub env: BTreeMap<String, OsString>,
    /// The wrapper argv this host needs to give the app a display (`xvfb-run`, under
    /// `dbus-run-session` when there is one), if any. `program`/`args` describe the app itself
    /// either way, so a caller that cannot wrap — a debugger launches the binary directly — still
    /// has something to run, and something to warn about.
    pub wrapper: Option<Vec<String>>,
}

/// Compute the plan without spawning anything and without printing. The `xvfb-run` probe lives
/// here because it decides the wrapper, but the narration belongs to `launch`: a `build` that only
/// wants the plan must stay quiet.
pub fn desktop_launch_plan(
    project: &Project,
    target: &'static Target,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<DesktopLaunchPlan, String> {
    let mut env: BTreeMap<String, OsString> = BTreeMap::new();

    // Headless CI (a linux host with no display server): give the toolkit what the CI shims used
    // to wrap around the CLI — xvfb sized to `[window]` (the root-capture screenshot fallback
    // then frames exactly the app), the WebKit flags for gtk, the xcb platform for qt. Qt could
    // render displayless (QT_QPA_PLATFORM=offscreen, the previous plumbing), but X selections
    // need a display server to broker them, so the system clipboard (day-part-clipboard's xclip)
    // was a silent no-op there and every copy/paste walkthrough step failed empty-handed. This
    // knowledge lived in TWO workflow files (day's ci.yml and build-day-app.yml) and drifted
    // between them; the CLI knows the target and the window, so it decides.
    let wrap = headless_wrap(
        target.toolkit,
        crate::targets::host_os(),
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some(),
        project.manifest.window.width,
        project.manifest.window.height,
    );
    let wrapper = match &wrap {
        HeadlessWrap::Xvfb { width, height } => {
            // Probe rather than assume: without xvfb-run, gtk's bare run at least fails with the
            // toolkit's own display error (more actionable than "No such file or directory" from
            // the wrapper), and qt still has a displayless platform to fall back to.
            if Command::new("xvfb-run").arg("--help").output().is_ok() {
                // …and under a session bus, when one can be had. A headless runner has no D-Bus
                // session, and GTK's file dialogs are portal-backed: with no bus `g_bus_get`
                // yields NULL and GTK carries on with it, which shows up as
                //   g_dbus_connection_send_message_with_reply_finish:
                //     assertion 'G_IS_DBUS_CONNECTION (connection)' failed
                // and then SIGSEGV on the NEXT dialog. `dbus-run-session` starts a private bus for
                // the app's lifetime and tears it down after, so the portal call gets a real
                // connection — and fails cleanly (no portal service answers) instead of
                // dereferencing nothing.
                let bus = Command::new("dbus-run-session")
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success());
                let mut w: Vec<String> = Vec::new();
                if bus {
                    w.push("dbus-run-session".to_string());
                    w.push("--".to_string());
                }
                w.push("xvfb-run".to_string());
                w.push("-a".to_string());
                w.push("-s".to_string());
                w.push(format!("-screen 0 {width}x{height}x24"));
                match target.toolkit {
                    "gtk" => {
                        env.insert(
                            "WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS".to_string(),
                            OsString::from("1"),
                        );
                        env.insert(
                            "WEBKIT_DISABLE_COMPOSITING_MODE".to_string(),
                            OsString::from("1"),
                        );
                    }
                    // Pinned, not autodetected: an inherited QT_QPA_PLATFORM (the pre-CLI CI
                    // shims exported `offscreen`) would defeat the display xvfb just provided.
                    // A `--env` override still wins — `spec.envs` lands after this.
                    "qt" => {
                        env.insert("QT_QPA_PLATFORM".to_string(), OsString::from("xcb"));
                    }
                    _ => {}
                }
                Some(w)
            } else if target.toolkit == "qt" {
                // No xvfb-run to make a display: the offscreen platform still renders and
                // drives every dayscript step except the system clipboard (X selections need
                // a display server to broker them).
                env.insert("QT_QPA_PLATFORM".to_string(), OsString::from("offscreen"));
                None
            } else {
                None
            }
        }
        HeadlessWrap::None => None,
    };

    // An Xcode-built `.app` bundle (platform/macos/, §17.4): name its inner binary. macOS resolves
    // the adjacent Info.plist, so bundle identity, the Dock icon (compiled appiconset), and
    // `Contents/Resources` are all REAL — none of the bare-binary environment below applies, and
    // naming the binary rather than `open`ing the bundle keeps stdio attached for log streaming
    // and dayscript.
    let bundled = outcome.artifact.extension().and_then(|e| e.to_str()) == Some("app");
    let program = if bundled {
        // The executable is named by the pbxproj's PRODUCT_NAME, not the crate — take the bundle's
        // single Contents/MacOS entry rather than guess.
        let macos_dir = outcome.artifact.join("Contents/MacOS");
        std::fs::read_dir(&macos_dir)
            .ok()
            .and_then(|rd| rd.flatten().map(|e| e.path()).next())
            .ok_or_else(|| format!("no executable under {}", macos_dir.display()))?
    } else {
        outcome.artifact.clone()
    };

    if !bundled {
        env.insert(
            "DAY_ASSET_ROOT".to_string(),
            project.root.join("resource/assets").into_os_string(),
        );
        env.insert(
            "DAY_IMAGE_ROOT".to_string(),
            project.root.join("resource/images").into_os_string(),
        );
        // The vector raster cache (docs/vectors.md): how the file-loading desktop backends resolve
        // `vector(…)` names — written by resources::stage at build. The FALLBACK rasters, not the
        // whole cache: a dev launch has to fail the same way a shipped app would, or a broken
        // vector path stays hidden behind a stand-in PNG right where it would be caught.
        env.insert(
            "DAY_VECTOR_RASTER_ROOT".to_string(),
            crate::resources::vector_fallback_dir(project, target.toolkit).into_os_string(),
        );
        // The glyph SVGs themselves — day-appkit prefers these (NSImage renders SVG at display
        // size on macOS 11+), so vectors stay vector on the desktop too.
        env.insert(
            "DAY_VECTOR_SVG_ROOT".to_string(),
            crate::resources::vector_svg_dir(project).into_os_string(),
        );
        // The XAML geometry — day-xaml draws these as real Path geometry, which is what keeps a
        // Windows glyph vector at any size and lets a tint be a brush rather than a second asset
        // (docs/vectors.md).
        env.insert(
            "DAY_VECTOR_XAML_ROOT".to_string(),
            crate::resources::vector_xaml_dir(project).into_os_string(),
        );
        // Bundled fonts (§18.4): the desktop backends register every file in this directory with
        // the platform font system at startup.
        env.insert(
            "DAY_FONT_ROOT".to_string(),
            project.root.join("resource/fonts").into_os_string(),
        );
        env.extend(app_identity_env(project));
    }

    // App icon (§18.2): the backend applies it to the dock / taskbar at startup (NSApp icon,
    // QApplication window icon, GTK icon theme, Win32 WM_SETICON). A bundled launch needs none of
    // this — the compiled appiconset is the Dock icon.
    if let Some(icon) = (!bundled)
        .then(|| crate::resources::app_icon(project, target.toolkit))
        .flatten()
    {
        env.insert("DAY_APP_ICON".to_string(), icon.clone().into_os_string());
        if target.toolkit == "gtk" && cfg!(target_os = "linux") {
            // GTK4 window icons are THEMED-name only: stage the icon into a hicolor layout keyed
            // by the app id and point the backend's icon-theme search at it.
            let theme = project.root.join("build/day/gtk/icons");
            let apps = theme.join("hicolor/512x512/apps");
            let _ = std::fs::create_dir_all(&apps);
            let name = &project.manifest.app.id;
            if std::fs::copy(&icon, apps.join(format!("{name}.png"))).is_ok() {
                env.insert("DAY_ICON_THEME_DIR".to_string(), theme.into_os_string());
                env.insert("DAY_ICON_NAME".to_string(), name.clone().into());
            }
        }
    }
    if target.toolkit == "gtk" {
        env.insert("GSK_RENDERER".to_string(), OsString::from("cairo"));
        // Native GResource blob (§18.3) — day-gtk registers it + loads via g_resources_*.
        let g = crate::resources::gtk::gresource_path(project);
        if g.exists() {
            env.insert("DAY_GRESOURCE".to_string(), g.into_os_string());
        }
    }
    if target.toolkit == "qt" {
        // Native Qt resource blob (§18.3) — the day-qt shim registers it (QResource).
        let q = crate::resources::qt::qresource_path(project);
        if q.exists() {
            env.insert("DAY_QRESOURCE".to_string(), q.into_os_string());
        }
    }
    if let Some(locale) = &spec.locale {
        env.insert("DAY_LOCALE".to_string(), locale.clone().into());
    }
    for (k, v) in &spec.envs {
        env.insert(k.clone(), v.clone().into());
    }

    Ok(DesktopLaunchPlan {
        program,
        args: Vec::new(),
        cwd: project.root.clone(),
        env,
        wrapper,
    })
}

/// Launch a built artifact; returns a join handle streaming prefixed logs.
pub fn launch(
    project: &Project,
    target: &'static Target,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    match target.kind {
        TargetKind::Desktop => {
            let plan = desktop_launch_plan(project, target, outcome, spec)?;
            let mut cmd = match &plan.wrapper {
                Some(w) => {
                    status(
                        "Headless",
                        &format!("wrapping in {} (no DISPLAY on this host)", w.join(" ")),
                    );
                    let mut c = Command::new(&w[0]);
                    c.args(&w[1..]).arg(&plan.program).args(&plan.args);
                    c
                }
                None => {
                    if plan
                        .env
                        .get("QT_QPA_PLATFORM")
                        .is_some_and(|v| v == "offscreen")
                    {
                        status(
                            "Headless",
                            "QT_QPA_PLATFORM=offscreen (no DISPLAY and no xvfb-run on this host)",
                        );
                    }
                    let mut c = Command::new(&plan.program);
                    c.args(&plan.args);
                    c
                }
            };
            cmd.current_dir(&plan.cwd);
            for (k, v) in &plan.env {
                cmd.env(k, v);
            }
            if spec.attached {
                cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            } else {
                // Detached: the day process exits after spawning — piped stdio would close
                // with it and the app's next log write would die on SIGPIPE. The app must also
                // leave day's PROCESS GROUP: task runners (VS Code) dispose the pty when the
                // task's root process exits, and the resulting SIGHUP to the pty's foreground
                // group would kill a keep-alive app that stayed in it.
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    cmd.process_group(0);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
                }
            }
            status("Launching", target.name);
            let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
            crate::signals::register_app_child(child.id());
            let name = target.name;
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let h = std::thread::spawn(move || {
                let t1 = stdout.map(|s| stream_logs(name, LogStream::Out, s));
                let t2 = stderr.map(|s| stream_logs(name, LogStream::Err, s));
                let code = child.wait().map(exit_code_of).unwrap_or(1);
                if let Some(t) = t1 {
                    let _ = t.join();
                }
                if let Some(t) = t2 {
                    let _ = t.join();
                }
                code
            });
            Ok(h)
        }
        TargetKind::IosSim => crate::mobile::launch_ios(project, outcome, spec),
        TargetKind::Android => crate::mobile::launch_android(project, outcome, spec),
        TargetKind::HarmonyOs => crate::ohos::launch_ohos(project, outcome, spec),
        TargetKind::Web => crate::web::launch_web(project, outcome, spec),
    }
}

/// Which standard stream a forwarded line came from — sets its destination, and its color when
/// the line carries no level of its own.
#[derive(Clone, Copy)]
pub enum LogStream {
    /// App stdout, forwarded to our stdout.
    Out,
    /// App stderr, forwarded to our stderr.
    Err,
}

/// The width Day's logger pads its level column to (`docs/logging.md`).
const LEVEL_WIDTH: usize = 5;

/// The color for one of Day's level words, or `None` if this isn't one.
fn level_style(word: &str) -> Option<anstyle::Style> {
    Some(match word {
        "ERROR" => LOG_ERROR,
        "WARN" => LOG_WARN,
        "INFO" => LOG_INFO,
        "DEBUG" => LOG_DEBUG,
        "TRACE" => LOG_TRACE,
        _ => return None,
    })
}

/// Render one forwarded line: `[target] LEVEL rest`, colored by level.
///
/// Day's logger writes every level to **stderr**, so coloring by stream — the only severity signal
/// available back when an app had just two file descriptors — painted an entire debug run yellow.
/// Now that each line arrives as `LEVEL target: message` the level is right there, and it colors
/// the `[target]` prefix too so a scan down the left column finds the errors.
///
/// Anything not in that format keeps the old stream color: a bare `println!`, a Qt warning on
/// stderr, a raw logcat line. The destination always follows the stream, never the level — an
/// `ERROR` an app wrote to stdout stays on stdout.
fn format_log(name: &str, stream: LogStream, line: &str) -> String {
    let leveled = line
        .split_once(' ')
        .and_then(|(word, rest)| Some((word, level_style(word)?, rest.trim_start())));
    match leveled {
        // Re-pad the level: splitting on the first space ate the alignment the logger wrote.
        Some((word, style, rest)) => {
            format!("{style}[{name}] {word:<LEVEL_WIDTH$}{style:#} {rest}")
        }
        None => {
            let style = match stream {
                LogStream::Out => LOG_OUT,
                LogStream::Err => LOG_ERR,
            };
            format!("{style}[{name}]{style:#} {line}")
        }
    }
}

/// Print one already-classified log line. Public so the mobile log pumps (logcat/simctl) can reuse
/// the exact formatting. `anstream` strips the color when our own output isn't a terminal.
pub fn emit_log(name: &str, stream: LogStream, line: &str) {
    let out = format_log(name, stream, line);
    match stream {
        LogStream::Out => anstream::println!("{out}"),
        LogStream::Err => anstream::eprintln!("{out}"),
    }
}

pub fn stream_logs(
    name: &'static str,
    stream: LogStream,
    src: impl std::io::Read + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(src).lines().map_while(Result::ok) {
            emit_log(name, stream, &line);
        }
    })
}

/// How to run a desktop target on a host with no display server.
#[derive(Debug, PartialEq)]
pub(crate) enum HeadlessWrap {
    None,
    Xvfb { width: u32, height: u32 },
}

/// The decision alone, display-state and host passed in — testable on any machine. Only the
/// in-repo linux toolkits get house treatment; an external toolkit (docs/extending.md) manages
/// its own headless story.
pub(crate) fn headless_wrap(
    toolkit: &str,
    host: &str,
    display_present: bool,
    width: f64,
    height: f64,
) -> HeadlessWrap {
    if host != "linux" || display_present {
        return HeadlessWrap::None;
    }
    match toolkit {
        // Both linux toolkits take a real (virtual) X display. Qt's offscreen platform could
        // render without one, but X selections need a display server, so the system clipboard
        // (parts/day-part-clipboard/src/linux.rs) would be a silent no-op; the offscreen
        // fallback survives in `desktop_launch_plan` for hosts without xvfb-run.
        "gtk" | "qt" => HeadlessWrap::Xvfb {
            width: width.max(1.0) as u32,
            height: height.max(1.0) as u32,
        },
        _ => HeadlessWrap::None,
    }
}

#[cfg(test)]
mod log_format_tests {
    use super::*;

    /// What the terminal actually receives, escapes and all, with ESC made visible.
    fn rendered(stream: LogStream, line: &str) -> String {
        format_log("macos-appkit", stream, line).replace('\u{1b}', "^")
    }

    #[test]
    fn a_level_line_is_colored_by_level_not_by_stream() {
        // Day's logger sends every level to stderr, so these all arrive on `Err`. Before, that
        // made the whole run yellow; each must now carry its own color.
        let err = rendered(LogStream::Err, "ERROR my_app: the database is unreadable");
        assert!(err.starts_with("^[31m[macos-appkit] ERROR^[0m "), "{err}");
        let info = rendered(LogStream::Err, "INFO  my_app: importing 412 rows");
        assert!(info.starts_with("^[32m[macos-appkit] INFO ^[0m "), "{info}");
        let debug = rendered(LogStream::Err, "DEBUG day_core::nav: restoring");
        assert!(
            debug.starts_with("^[34m[macos-appkit] DEBUG^[0m "),
            "{debug}"
        );
    }

    #[test]
    fn the_level_column_keeps_its_padding_and_the_message_stays_plain() {
        // Splitting on the first space eats the logger's alignment; it has to be put back, or
        // `WARN`/`INFO` lines sit a column left of `ERROR`/`DEBUG`.
        assert_eq!(
            rendered(LogStream::Err, "WARN  day_gtk: no bundled font"),
            "^[33m[macos-appkit] WARN ^[0m day_gtk: no bundled font"
        );
    }

    #[test]
    fn a_line_with_no_level_keeps_the_stream_color() {
        // A bare `println!`, a Qt warning, a raw logcat line: nothing to read a level from.
        assert_eq!(
            rendered(LogStream::Out, "just some output"),
            "^[34m[macos-appkit]^[0m just some output"
        );
        assert_eq!(
            rendered(LogStream::Err, "QWidget: cannot create"),
            "^[33m[macos-appkit]^[0m QWidget: cannot create"
        );
        // A lone word can't be split into level + rest, and must not panic.
        assert!(rendered(LogStream::Err, "ERROR").contains("[macos-appkit]^[0m ERROR"));
    }
}

#[cfg(test)]
mod headless_tests {
    use super::*;

    #[test]
    fn linux_without_a_display_wraps_both_toolkits_in_xvfb() {
        // qt too, not offscreen: without a display server there is nothing to broker X
        // selections, so under offscreen the system clipboard read back nothing and every
        // copy/paste walkthrough step failed.
        for toolkit in ["gtk", "qt"] {
            assert_eq!(
                headless_wrap(toolkit, "linux", false, 960.0, 640.0),
                HeadlessWrap::Xvfb {
                    width: 960,
                    height: 640
                }
            );
        }
    }

    #[test]
    fn a_display_or_a_nonlinux_host_or_a_foreign_toolkit_runs_bare() {
        assert_eq!(
            headless_wrap("gtk", "linux", true, 1.0, 1.0),
            HeadlessWrap::None
        );
        assert_eq!(
            headless_wrap("gtk", "macos", false, 1.0, 1.0),
            HeadlessWrap::None
        );
        // External toolkits (Stage 0) own their headless behavior.
        assert_eq!(
            headless_wrap("wxwidgets", "linux", false, 1.0, 1.0),
            HeadlessWrap::None
        );
    }
}

/// The deadline the device paths depend on: a tool that waits forever must not be able to make
/// `day launch` wait forever. Unix-only, because the fixtures are `sleep` and `echo`.
#[cfg(all(test, unix))]
mod wait_tests {
    use super::*;

    #[test]
    fn a_command_that_will_not_finish_is_killed_and_reported_unfinished() {
        let start = Instant::now();
        assert!(
            status_within(Command::new("sleep").arg("30"), Duration::from_millis(300)).is_none()
        );
        assert!(
            output_within(Command::new("sleep").arg("30"), Duration::from_millis(300)).is_none()
        );
        // Both waits together, nowhere near the 30s the children asked for.
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_command_that_finishes_reports_its_status_and_output() {
        let status = status_within(&mut Command::new("true"), Duration::from_secs(30));
        assert!(status.is_some_and(|s| s.success()));
        let out = output_within(Command::new("echo").arg("ok"), Duration::from_secs(30))
            .expect("echo finishes well inside 30s");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
    }
}
