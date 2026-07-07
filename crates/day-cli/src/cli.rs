//! Command tree (DESIGN.md §16.5). v0: create / build / launch / doctor; the remaining
//! porcelain (sign / pack / lint / script) lands with M6–M8.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::meta;
use crate::ops;
use crate::targets;

#[derive(Parser)]
#[command(
    name = "day",
    version,
    about = "Day — cross-platform apps in Rust with native toolkits"
)]
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
    /// Legacy alias for `day new app` (wired to this day checkout, non-interactive)
    Create {
        name: String,
        /// Comma-separated target list (default: the host's native target)
        #[arg(long)]
        targets: Option<String>,
        /// Application id (reverse-DNS)
        #[arg(long)]
        id: Option<String>,
    },
    /// Scaffold a new Day project — an app, a piece, or a part (interactive when run bare)
    New {
        #[command(subcommand)]
        what: Option<NewKind>,
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

/// `day new <piece|part|app>` — scaffold an extension crate or app; `day new` (bare) walks an
/// interactive dialog. Every value-carrying flag has an equivalent question in the dialog (the dialog
/// is the fallback branch of the flags — see `new.rs`), so a value not passed on the command line is
/// asked for when a terminal is present, and defaulted (or reported as required) when it is not. The
/// meta flags `--local` (CI) and `--no-input` have no dialog fallback by design.
///
/// Scaffolds default to REMOTE (git) day dependencies so they are self-contained; the hidden
/// `--local <path>` (or `DAY_LOCAL` env) redirects to a local day checkout for CI smoke-tests of a
/// freshly-scaffolded project against the day tree under test.
#[derive(Subcommand)]
enum NewKind {
    /// Scaffold a Day PIECE crate (a reusable widget). No `--toolkits` ⇒ a COMPOSITE piece.
    Piece {
        /// Crate name (prompted if omitted in an interactive terminal).
        name: Option<String>,
        /// Comma-separated toolkits for a NATIVE piece (appkit,gtk,qt,uikit,widget,winui).
        /// Omit for a COMPOSITE piece (pure composition; works on every backend with no per-backend code).
        #[arg(long)]
        toolkits: Option<String>,
        /// Force a COMPOSITE piece even if `--toolkits` is given.
        #[arg(long)]
        composite: bool,
        /// Package id (reverse-DNS); default `dev.example.<name>`. Also the piece KIND + Java package.
        #[arg(long)]
        id: Option<String>,
        /// Use `path` deps rooted at a local day checkout instead of the git remote (CI).
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Never prompt; use flags + defaults only (also implied when stdin is not a terminal).
        #[arg(long)]
        no_input: bool,
    },
    /// Scaffold a Day PART crate (a headless, UI-less capability).
    Part {
        /// Crate name (prompted if omitted in an interactive terminal).
        name: Option<String>,
        /// Comma-separated platforms (macos,ios,android,linux,windows); default: all.
        #[arg(long)]
        platforms: Option<String>,
        /// Package id (reverse-DNS); default `dev.example.<name>`. Also the Java package.
        #[arg(long)]
        id: Option<String>,
        /// Use `path` deps rooted at a local day checkout instead of the git remote (CI).
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Never prompt; use flags + defaults only (also implied when stdin is not a terminal).
        #[arg(long)]
        no_input: bool,
    },
    /// Scaffold a new Day app (the canonical app command; `day create` is a thin alias).
    App {
        /// App name (prompted if omitted in an interactive terminal).
        name: Option<String>,
        /// A target to support (repeatable): e.g. `--toolkit ios-uikit --toolkit macos-appkit`.
        /// Values may also be comma-separated. Omit to choose interactively.
        #[arg(long = "toolkit")]
        toolkits: Vec<String>,
        /// Application id / bundle id (reverse-DNS); default `dev.example.<name>`.
        #[arg(long)]
        appid: Option<String>,
        /// Alias for --appid (Android application id / Apple bundle id).
        #[arg(long)]
        bundleid: Option<String>,
        /// Back-compat alias for --appid.
        #[arg(long, hide = true)]
        id: Option<String>,
        /// Back-compat: comma-separated target list (prefer repeated --toolkit).
        #[arg(long, hide = true)]
        targets: Option<String>,
        /// Use `path` deps rooted at a local day checkout instead of the git remote (CI).
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Never prompt; use flags + defaults only (also implied when stdin is not a terminal).
        #[arg(long)]
        no_input: bool,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Doctor => crate::doctor::run(),
        Cmd::Pack { platforms, profile } => with_project(cli.project.as_deref(), |project| {
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
        }),
        Cmd::Lint { strict } => with_project(cli.project.as_deref(), |project| {
            crate::lint::run(project, strict)
        }),
        Cmd::XcodeBackend { .. } => crate::mobile::xcode_backend_build(),
        Cmd::GradleBackend { .. } => crate::mobile::gradle_backend_build(),
        Cmd::Create { name, targets, id } => {
            // Legacy alias for `day new app`: an app wired to THIS day checkout (local path deps),
            // never interactive. New projects should prefer `day new app` (remote deps).
            let home = day_home();
            crate::new::app(
                Some(&name),
                &[],
                None,
                None,
                id.as_deref(),
                targets.as_deref(),
                Some(home.as_path()),
                true,
            )
        }
        Cmd::New { what } => match what {
            None => crate::new::interactive(),
            Some(NewKind::Piece {
                name,
                toolkits,
                composite,
                id,
                local,
                no_input,
            }) => crate::new::piece(
                name.as_deref(),
                toolkits.as_deref(),
                composite,
                id.as_deref(),
                local.as_deref(),
                no_input,
            ),
            Some(NewKind::Part {
                name,
                platforms,
                id,
                local,
                no_input,
            }) => crate::new::part(
                name.as_deref(),
                platforms.as_deref(),
                id.as_deref(),
                local.as_deref(),
                no_input,
            ),
            Some(NewKind::App {
                name,
                toolkits,
                appid,
                bundleid,
                id,
                targets,
                local,
                no_input,
            }) => crate::new::app(
                name.as_deref(),
                &toolkits,
                appid.as_deref(),
                bundleid.as_deref(),
                id.as_deref(),
                targets.as_deref(),
                local.as_deref(),
                no_input,
            ),
        },
        Cmd::Build { platforms, profile } => with_project(cli.project.as_deref(), |project| {
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
                            &format!(
                                "{} → {} ({:.1}s)",
                                o.target,
                                o.artifact.display(),
                                o.seconds
                            ),
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
        }),
        Cmd::Launch {
            platforms,
            profile,
            locale,
            envs,
            detach,
            scripts,
        } => with_project(cli.project.as_deref(), |project| {
            let script_mode = !scripts.is_empty();
            let mut spec = ops::LaunchSpec {
                locale: locale.clone(),
                envs: envs
                    .iter()
                    .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
                    .collect(),
                attached: !detach && !script_mode,
            };
            // Ctrl-C during an attached run must take the launched apps and their log
            // watchers (simctl / adb logcat) down too — not leave them orphaned.
            if spec.attached {
                crate::signals::install();
            }
            let token = crate::script::make_token();
            let mut handles = Vec::new();
            let mut script_failures = 0usize;
            for (ti, p) in platforms.iter().enumerate() {
                let port = crate::script::pick_port(ti);
                if script_mode {
                    spec.envs
                        .retain(|(k, _)| k != "DAYSCRIPT_PORT" && k != "DAYSCRIPT_TOKEN");
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
                // A target that exited on its own leaves its siblings' log watchers (and
                // any child that outlives its stream) running — reap them before we go.
                crate::signals::kill_all();
                code
            } else {
                0
            }
        }),
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

/// The root of the Day repo (for path deps in created projects; DAY_HOME overrides).
fn day_home() -> PathBuf {
    std::env::var_os("DAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}
