//! Command tree (DESIGN.md §16.5). v0: create / build / launch / doctor; the remaining
//! porcelain (sign / pack / lint / script) lands with M6–M8.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::meta;
use crate::ops;
use crate::targets;

#[derive(Parser)]
#[command(name = "day", version, about = "day — cross-platform apps in Rust with native toolkits")]
struct Cli {
    /// Project directory (default: nearest ancestor with day.yaml)
    #[arg(long, global = true)]
    project: Option<PathBuf>,
    /// Output format: plain (default) or json (NDJSON result events)
    #[arg(long, global = true, default_value = "plain")]
    format: String,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new day project
    Create {
        name: String,
        /// Comma-separated target list
        #[arg(long, default_value = "macos-appkit")]
        targets: String,
        /// Application id (reverse-DNS)
        #[arg(long)]
        id: Option<String>,
    },
    /// Build the app for one or more targets
    Build {
        #[arg(short = 'p', long = "platform", required = true)]
        platforms: Vec<String>,
        #[arg(long, default_value = "debug")]
        profile: String,
    },
    /// Build + launch on one or more targets (in parallel)
    Launch {
        #[arg(short = 'p', long = "platform", required = true)]
        platforms: Vec<String>,
        #[arg(long, default_value = "debug")]
        profile: String,
        /// BCP-47 locale override passed to the app
        #[arg(long)]
        locale: Option<String>,
        /// Extra environment K=V passed to the app (repeatable)
        #[arg(long = "env")]
        envs: Vec<String>,
        /// Exit after launch instead of staying attached to logs
        #[arg(long)]
        detach: bool,
        /// dayscript file(s) to execute after launch (repeatable; implies detach)
        #[arg(long = "script")]
        scripts: Vec<PathBuf>,
    },
    /// Build + sign + produce an installable artifact (.dmg / .apk / sim-.app.zip)
    Pack {
        #[arg(short = 'p', long = "platform", required = true)]
        platforms: Vec<String>,
        #[arg(long, default_value = "debug")]
        profile: String,
    },
    /// Check toolchains for every known target
    Doctor,
    /// Check the project for common errors (fluent coverage, ids)
    Lint {
        /// Exit non-zero (10) when findings exist
        #[arg(long)]
        strict: bool,
    },
    /// PLUMBING: invoked by the Xcode script phase (reads Xcode's env)
    #[command(name = "xcode-backend", hide = true)]
    XcodeBackend {
        #[arg(default_value = "build")]
        action: String,
    },
    /// PLUMBING: invoked by the gradle scaffold (reads DAY_* env)
    #[command(name = "gradle-backend", hide = true)]
    GradleBackend {
        #[arg(default_value = "build")]
        action: String,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Doctor => crate::doctor::run(),
        Cmd::Pack { platforms, profile } => {
            with_project(cli.project.as_deref(), |project| {
                for p in &platforms {
                    let Some(target) = targets::find(p) else {
                        eprintln!("error: unknown target {p:?}");
                        return 2;
                    };
                    if let Err(e) = crate::pack::run(project, target, &profile) {
                        eprintln!("error: {e}");
                        return 4;
                    }
                }
                0
            })
        }
        Cmd::Lint { strict } => {
            with_project(cli.project.as_deref(), |project| crate::lint::run(project, strict))
        }
        Cmd::XcodeBackend { .. } => crate::mobile::xcode_backend_build(),
        Cmd::GradleBackend { .. } => crate::mobile::gradle_backend_build(),
        Cmd::Create { name, targets, id } => create(&name, &targets, id.as_deref()),
        Cmd::Build { platforms, profile } => {
            with_project(cli.project.as_deref(), |project| {
                let mut results = Vec::new();
                for p in &platforms {
                    let target = match targets::find(p) {
                        Some(t) => t,
                        None => {
                            eprintln!("error: unknown target {p:?}");
                            return 2;
                        }
                    };
                    match ops::build(project, target, &profile) {
                        Ok(o) => {
                            ops::status(
                                "Built",
                                &format!("{} → {} ({:.1}s)", o.target, o.artifact.display(), o.seconds),
                            );
                            results.push(o);
                        }
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 4;
                        }
                    }
                }
                if cli.format == "json" {
                    print_result_json("build", &results);
                }
                0
            })
        }
        Cmd::Launch { platforms, profile, locale, envs, detach, scripts } => {
            with_project(cli.project.as_deref(), |project| {
                let script_mode = !scripts.is_empty();
                let mut spec = ops::LaunchSpec {
                    locale: locale.clone(),
                    envs: envs
                        .iter()
                        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
                        .collect(),
                    attached: !detach && !script_mode,
                };
                let token = crate::script::make_token();
                let mut handles = Vec::new();
                let mut script_failures = 0usize;
                for (ti, p) in platforms.iter().enumerate() {
                    let port = crate::script::pick_port(ti);
                    if script_mode {
                        spec.envs.retain(|(k, _)| k != "DAYSCRIPT_PORT" && k != "DAYSCRIPT_TOKEN");
                        spec.envs.push(("DAYSCRIPT_PORT".into(), port.to_string()));
                        spec.envs.push(("DAYSCRIPT_TOKEN".into(), token.clone()));
                    }
                    let target = match targets::find(p) {
                        Some(t) => t,
                        None => {
                            eprintln!("error: unknown target {p:?}");
                            return 2;
                        }
                    };
                    let outcome = match ops::build(project, target, &profile) {
                        Ok(o) => o,
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 4;
                        }
                    };
                    match ops::launch(project, target, &outcome, &spec) {
                        Ok(h) => handles.push(h),
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 1;
                        }
                    }
                    if script_mode {
                        match crate::script::run_scripts(
                            project,
                            target,
                            port,
                            &token,
                            &scripts,
                            locale.as_deref(),
                        ) {
                            Ok(run) => {
                                script_failures += run.steps_failed;
                                ops::status(
                                    "Script",
                                    &format!(
                                        "{}: {}/{} steps passed · {} screenshot(s)",
                                        target.name,
                                        run.steps_total - run.steps_failed,
                                        run.steps_total,
                                        run.screenshots.len()
                                    ),
                                );
                            }
                            Err(e) => {
                                eprintln!("error: {e}");
                                return 5;
                            }
                        }
                    }
                }
                if script_mode {
                    return if script_failures > 0 { 5 } else { 0 };
                }
                if spec.attached {
                    let mut code = 0;
                    for h in handles {
                        code = code.max(h.join().unwrap_or(1));
                    }
                    code
                } else {
                    0
                }
            })
        }
    }
}

fn with_project(start: Option<&std::path::Path>, f: impl FnOnce(&meta::Project) -> i32) -> i32 {
    match meta::find_project(start) {
        Ok(p) => f(&p),
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn print_result_json(command: &str, results: &[ops::BuildOutcome]) {
    let targets: Vec<serde_json::Value> = results
        .iter()
        .map(|o| {
            serde_json::json!({
                "target": o.target, "ok": true, "code": 0,
                "artifacts": [{"path": o.artifact}], "seconds": o.seconds,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({"event": "result", "command": command, "ok": true, "targets": targets})
    );
}

/// The root of the day repo (for path deps in created projects; DAY_HOME overrides).
fn day_home() -> PathBuf {
    std::env::var_os("DAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn create(name: &str, targets_csv: &str, id: Option<&str>) -> i32 {
    let dir = PathBuf::from(name);
    if dir.exists() {
        eprintln!("error: {name:?} already exists");
        return 1;
    }
    let id = id.map(String::from).unwrap_or_else(|| format!("dev.example.{name}"));
    let day = day_home().canonicalize().unwrap_or_else(|_| day_home());
    let day = day.display();
    let targets_yaml = targets_csv
        .split(',')
        .map(|t| format!("  - {}", t.trim()))
        .collect::<Vec<_>>()
        .join("\n");

    let files: &[(&str, String)] = &[
        (
            "day.yaml",
            format!(
                "day: 1\napp:\n  name: {name}\n  id: {id}\n  title: {name}\n  version: 0.1.0\n  build: 1\ntargets:\n{targets_yaml}\nwindow:\n  width: 480\n  height: 640\n"
            ),
        ),
        (
            "Cargo.toml",
            format!(
                r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[features]
appkit = ["day/appkit"]
gtk = ["day/gtk"]
qt = ["day/qt"]
mock = ["day/mock"]

[dependencies]
day = {{ path = "{day}/crates/day" }}

[[bin]]
name = "{name}"
path = "src/main.rs"

[workspace]
"#
            ),
        ),
        (
            "src/lib.rs",
            r#"use day::prelude::*;

pub fn root() -> AnyPiece {
    let count = Signal::new(0i64);
    column((
        label("Hello, day!").font(Font::Title),
        row((
            button("−").action(move || count.update(|c| *c -= 1)).id("dec"),
            label(move || format!("{}", count.get())).id("count"),
            button("+").action(move || count.update(|c| *c += 1)).id("inc"),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .padding(16.0)
    .any()
}
"#
            .to_string(),
        ),
        (
            "src/main.rs",
            format!(
                r#"fn main() {{
    day::launch(
        day::WindowOptions {{
            title: "{name}".into(),
            size: day::prelude::Size::new(480.0, 640.0),
            min_size: None,
        }},
        {name}::root,
    );
}}
"#
            ),
        ),
        (".gitignore", "/target\n/build\n".to_string()),
    ];

    for (path, content) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&full, content) {
            eprintln!("error writing {}: {e}", full.display());
            return 1;
        }
    }
    ops::status("Created", &format!("{name}/ ({} files)", files.len()));
    eprintln!("\n  next:\n    cd {name}\n    day doctor\n    day launch -p macos-appkit\n");
    0
}
