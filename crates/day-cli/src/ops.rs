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
    project.root.join("build/day/cargo").join(target.name).join(profile)
}

pub fn status(prefix: &str, msg: &str) {
    eprintln!("\x1b[1;32m{prefix:>12}\x1b[0m {msg}");
}

pub fn build(project: &Project, target: &'static Target, profile: &str) -> Result<BuildOutcome, String> {
    let start = std::time::Instant::now();
    match target.kind {
        TargetKind::Desktop => {
            let mut cmd = Command::new("cargo");
            cmd.current_dir(&project.root)
                .env("CARGO_TARGET_DIR", cargo_dir(project, target, profile))
                .args(["build", "-p", &project.manifest.app.name, "--no-default-features"])
                .args(["--features", target.toolkit]);
            if profile == "release" {
                cmd.arg("--release");
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
    }
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
            let name = target.name;
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let h = std::thread::spawn(move || {
                let t1 = stdout.map(|s| stream_logs(name, s));
                let t2 = stderr.map(|s| stream_logs(name, s));
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
    }
}

pub fn stream_logs(
    name: &'static str,
    src: impl std::io::Read + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(src).lines().map_while(Result::ok) {
            eprintln!("\x1b[2m[{name}]\x1b[0m {line}");
        }
    })
}
