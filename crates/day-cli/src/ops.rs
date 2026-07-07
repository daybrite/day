//! Build / launch operations. Desktop = cargo with per-(target, profile) CARGO_TARGET_DIR
//! (§16.5 — parallel targets never contend on the cargo build-dir lock). Mobile pipelines
//! attach here at M5 (xcodebuild + simctl; gradle + adb).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::meta::Project;
use crate::targets::{Target, TargetKind};

pub struct BuildOutcome {
    pub target: &'static str,
    pub artifact: PathBuf,
    pub seconds: f64,
}

fn cargo_dir(project: &Project, target: &Target, profile: &str) -> PathBuf {
    project
        .root
        .join("build/day/cargo")
        .join(target.name)
        .join(profile)
}

pub fn status(prefix: &str, msg: &str) {
    eprintln!("\x1b[1;32m{prefix:>12}\x1b[0m {msg}");
}

/// The comma-joined `--features` string for a `backend` toolkit: the toolkit feature itself plus the
/// unioned `<pkg>/<backend>` renderer feature of every standalone piece in the app's dependency
/// closure (Tier A.2 — apps no longer fan out per-piece features in their own Cargo.toml).
pub fn feature_selection(project: &Project, backend: &str) -> String {
    let mut features = vec![backend.to_string()];
    features.extend(crate::pieces::feature_union(project, backend));
    features.join(",")
}

pub fn build(
    project: &Project,
    target: &'static Target,
    profile: &str,
) -> Result<BuildOutcome, String> {
    let host = crate::targets::host_os();
    if target.host != "any" && target.host != host {
        return Err(format!(
            "target {} builds on a {} host (this is {})",
            target.name, target.host, host
        ));
    }
    let start = std::time::Instant::now();
    match target.kind {
        TargetKind::Desktop => {
            let mut cmd = Command::new("cargo");
            cmd.current_dir(&project.root)
                .env("CARGO_TARGET_DIR", cargo_dir(project, target, profile));
            // The toolkit feature (e.g. `appkit`) + every standalone piece's `<pkg>/<toolkit>`
            // renderer feature, derived from `cargo metadata` — so the app depends on a piece
            // without re-listing its per-backend feature (Tier A.2).
            let features = feature_selection(project, target.toolkit);
            if target.toolkit == "winui" {
                // XAML Islands refuses to start unless the app manifest declares
                // `maxversiontested` (§9). rustc's default embedded manifest lacks it, so we
                // embed our own — `cargo rustc -- <link-args>` scopes this to the bin only.
                let manifest = write_winui_manifest(project, target, profile)?;
                cmd.args(["rustc", "--bin", &project.manifest.app.name])
                    .args(["--no-default-features", "--features", &features]);
                if profile == "release" {
                    cmd.arg("--release");
                }
                cmd.arg("--");
                cmd.arg("-Clink-arg=/MANIFEST:EMBED");
                cmd.arg(format!("-Clink-arg=/MANIFESTINPUT:{}", manifest.display()));
            } else {
                cmd.args([
                    "build",
                    "-p",
                    &project.manifest.app.name,
                    "--no-default-features",
                ])
                .args(["--features", &features]);
                if profile == "release" {
                    cmd.arg("--release");
                }
            }
            status("Building", &format!("{} ({})", target.name, profile));
            let out = cmd.status().map_err(|e| format!("cargo: {e}"))?;
            if !out.success() {
                return Err(format!("cargo build failed for {}", target.name));
            }
            let artifact = cargo_dir(project, target, profile)
                .join(profile)
                .join(&project.manifest.app.name);
            Ok(BuildOutcome {
                target: target.name,
                artifact,
                seconds: start.elapsed().as_secs_f64(),
            })
        }
        TargetKind::IosSim => crate::mobile::build_ios(project, target, profile, start),
        TargetKind::Android => crate::mobile::build_android(project, target, profile, start),
        TargetKind::HarmonyOs => crate::ohos::build_ohos(project, target, profile, start),
    }
}

/// Side-by-side manifest that lets an unpackaged app host `Windows.UI.Xaml` islands (§9).
/// The `maxversiontested` element is the specific thing `WindowsXamlManager` demands.
const WINUI_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
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

fn write_winui_manifest(
    project: &Project,
    target: &Target,
    profile: &str,
) -> Result<PathBuf, String> {
    let dir = cargo_dir(project, target, profile);
    std::fs::create_dir_all(&dir).map_err(|e| format!("manifest dir: {e}"))?;
    let path = dir.join("day-winui.manifest");
    std::fs::write(&path, WINUI_MANIFEST).map_err(|e| format!("manifest write: {e}"))?;
    Ok(path)
}

pub struct LaunchSpec {
    pub locale: Option<String>,
    pub envs: Vec<(String, String)>,
    pub attached: bool,
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
            let mut cmd = Command::new(&outcome.artifact);
            cmd.current_dir(&project.root)
                .env("DAY_ASSET_ROOT", project.root.join("assets"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if target.toolkit == "gtk" {
                cmd.env("GSK_RENDERER", "cairo");
            }
            if let Some(locale) = &spec.locale {
                cmd.env("DAY_LOCALE", locale);
            }
            for (k, v) in &spec.envs {
                cmd.env(k, v);
            }
            status("Launching", target.name);
            let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
            crate::signals::register_child(child.id());
            let name = target.name;
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let h = std::thread::spawn(move || {
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
            });
            Ok(h)
        }
        TargetKind::IosSim => crate::mobile::launch_ios(project, outcome, spec),
        TargetKind::Android => crate::mobile::launch_android(project, outcome, spec),
        TargetKind::HarmonyOs => crate::ohos::launch_ohos(project, outcome, spec),
    }
}

/// Which standard stream a forwarded line came from — sets its colour and destination.
#[derive(Clone, Copy)]
pub enum LogStream {
    /// App stdout: blue, forwarded to our stdout.
    Out,
    /// App stderr: yellow, forwarded to our stderr.
    Err,
}

/// Print one already-classified log line with the `[target]` prefix and stream colour.
/// Public so the mobile log pumps (logcat/simctl) can reuse the exact formatting.
pub fn emit_log(name: &str, stream: LogStream, line: &str) {
    match stream {
        // 34 = blue, 33 = yellow; the whole line is coloured so streams read apart at a glance.
        LogStream::Out => println!("\x1b[34m[{name}]\x1b[0m {line}"),
        LogStream::Err => eprintln!("\x1b[33m[{name}]\x1b[0m {line}"),
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
