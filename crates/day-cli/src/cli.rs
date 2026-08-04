//! Command tree (DESIGN.md §16.5). v0: new / build / launch / doctor; the remaining
//! porcelain (sign / pack / lint / script) lands with M6–M8.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::meta;
use crate::ops;

#[derive(Parser)]
#[command(
    name = "day",
    version = env!("DAY_VERSION_LONG"),
    about = "Day — cross-platform apps in Rust with native toolkits"
)]
struct Cli {
    /// Project directory (default: nearest ancestor with Day.toml)
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
    /// Print the version, build profile (`*` = debug), and the git ref it was built from
    Version,
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
        /// Targets to launch. Omit it to launch the HOST's default desktop target — appkit on
        /// macOS, XAML on Windows, and on Linux the toolkit matching the running desktop (Qt
        /// under Plasma/LXQt, GTK otherwise).
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        #[arg(long, default_value = "debug")]
        profile: String,
        /// BCP-47 locale override passed to the app
        #[arg(long)]
        locale: Option<String>,
        /// Extra environment K=V passed to the app (repeatable)
        #[arg(long = "env")]
        envs: Vec<String>,
        /// Device to launch on when the target has more than one: an iOS simulator UDID or
        /// name. Without it every BOOTED simulator gets the app — right for a capture sweep,
        /// wrong when you mean one specific device. (Android selects with `ANDROID_SERIAL`.)
        #[arg(long)]
        device: Option<String>,
        /// Exit after launch instead of staying attached to logs
        #[arg(long)]
        detach: bool,
        /// Keep the app running after its dayscript completes (interactive script development:
        /// the session stays drivable via `day drive`)
        #[arg(long)]
        keep_alive: bool,
        /// dayscript file(s) to execute after launch (repeatable; implies detach)
        #[arg(long = "script")]
        scripts: Vec<PathBuf>,
        /// Screenshot set name: saves shots under `build/day/screenshots/<target>/<variant>/`
        /// instead of the locale-derived default — for capturing themed/localized variations
        /// of the same script run (e.g. `--variant dark --env DAY_THEME=dark`)
        #[arg(long)]
        variant: Option<String>,
        /// Reuse the previous build's artifact instead of building (errors if none exists).
        /// For runs whose variants share one binary — theme and locale are runtime inputs —
        /// e.g. CI capture loops that pay xcodebuild/hvigor once, then launch per variant.
        #[arg(long)]
        skip_build: bool,
        /// Run the script(s) once per locale (comma- or space-separated; repeatable). Each run
        /// passes `--locale <l>` and saves screenshots under variant `<l>` — the capture
        /// convention app CIs use. Builds once; later runs reuse the artifact.
        #[arg(long = "locales", requires = "scripts")]
        locales: Vec<String>,
        /// Cross the scripted runs with forced themes (sets DAY_THEME per run). Variants become
        /// `<theme>` for `en` and `<theme>-<locale>` otherwise — the day-CI / gallery
        /// convention (website/gallery.config.mjs variant ids).
        #[arg(long = "themes", requires = "scripts")]
        themes: Vec<String>,
    },
    /// Rebuild a shipped artifact from its own provenance and report whether it matches
    Rebuild {
        /// The artifact to verify (.dmg / .ipa / .apk / .flatpak / .msix / .hap)
        artifact: std::path::PathBuf,
        /// Ignore a tool-version mismatch for one tool (repeatable). `--force-tool=all` ignores
        /// every mismatch. A forced rebuild that differs proves nothing, so this is opt-in.
        #[arg(long = "force-tool")]
        force_tool: Vec<String>,
        /// Keep the temporary checkout and rebuild instead of deleting them
        #[arg(long)]
        keep: bool,
        /// Fail when the payload could not be compared at all, instead of reporting "not checked".
        /// CI wants this: an unopenable container means the code went unverified.
        #[arg(long)]
        strict: bool,
    },
    /// Build + sign + produce installable artifacts (.dmg / .ipa / .apk+.aab / .flatpak / .msix+setup.exe / .hap)
    Pack {
        #[arg(short = 'p', long = "platform", required = true)]
        platforms: Vec<String>,
        /// Pack defaults to release (distribution artifacts); pass debug for a dev-install pack.
        #[arg(long, default_value = "release")]
        profile: String,
        /// Comma-separated format subset (e.g. `--formats apk` to skip the aab)
        #[arg(long)]
        formats: Option<String>,
        /// Skip signing entirely (artifacts are marked unsigned)
        #[arg(long)]
        no_sign: bool,
        /// Sign but skip notarization (macOS)
        #[arg(long)]
        no_notarize: bool,
        /// Submit for notarization without waiting (check later: day sign --notarize-status <id>)
        #[arg(long)]
        no_wait: bool,
        /// Omit the app version from artifact filenames (app.aab, not app-1.0.0.aab) so a
        /// `releases/latest/download/<name>` URL stays stable across releases
        #[arg(long)]
        no_version_in_name: bool,
    },
    /// Signing utilities: --check validates Day.toml signing config (never prints secrets)
    Sign {
        /// Validate signing config resolvability (env vars set, files present)
        #[arg(long)]
        check: bool,
        /// Poll an async notarization submission by id
        #[arg(long = "notarize-status")]
        notarize_status: Option<String>,
    },
    /// Check the development environment, grouped by toolkit
    Doctor {
        /// Focus a toolkit (repeatable): its checks become errors + print setup help.
        /// One of: appkit, uikit, gtk, qt, xaml, android, harmonyos, dom.
        #[arg(long = "toolkit")]
        toolkits: Vec<String>,
    },
    /// App-project maintenance: add platforms/toolkits to an existing app
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Machine-readable project metadata: app identity, targets, per-target overrides, and
    /// the target catalog. IDE tooling (day-vscode) consumes `--json` instead of parsing
    /// Day.toml itself — the envelope is versioned and grow-only.
    Metadata {
        /// Emit the versioned JSON envelope instead of the human summary
        #[arg(long)]
        json: bool,
        /// Emit the Day.toml JSON Schema (for editor TOML validation) and exit
        #[arg(long)]
        schema: bool,
    },
    /// Check the project for common errors (fluent coverage, ids)
    Lint {
        /// Exit non-zero (10) when findings exist
        #[arg(long)]
        strict: bool,
    },
    /// Stop running launches (and drop their sessions)
    Stop {
        /// Target(s) to stop (repeatable)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Stop every recorded session
        #[arg(long)]
        all: bool,
    },
    /// Stop, rebuild, and relaunch targets — "apply my code changes"
    Relaunch {
        /// Target(s) to relaunch (repeatable); omit with --all-running
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Relaunch every recorded session
        #[arg(long)]
        all_running: bool,
        #[arg(long, default_value = "debug")]
        profile: String,
        /// BCP-47 locale override passed to the app
        #[arg(long)]
        locale: Option<String>,
    },
    /// Execute dayscript steps against a RUNNING app (see docs/agent.md)
    Drive {
        /// The target whose live session to drive
        #[arg(short = 'p', long = "platform")]
        platform: String,
        /// JSON array of steps, e.g. '[{"navigate":{"route":"controls"}},{"screenshot":"x"}]'
        #[arg(long)]
        steps_json: String,
    },
    /// Serve Day tools to coding agents over the Model Context Protocol (stdio)
    McpServer {},
    /// HarmonyOS / OpenHarmony helpers (emulator, …)
    Ohos {
        #[command(subcommand)]
        cmd: OhosCmd,
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
        /// Comma-separated toolkits for a NATIVE piece (appkit,gtk,qt,uikit,mdc,xaml).
        /// Omit for a COMPOSITE piece (pure composition; works on every backend with no per-backend code).
        #[arg(long)]
        toolkits: Option<String>,
        /// Force a COMPOSITE piece even if `--toolkits` is given.
        #[arg(long)]
        composite: bool,
        /// Package id (reverse-DNS); default `dev.example.<name>`. Also the piece KIND + Java package.
        #[arg(long)]
        id: Option<String>,
        /// Scaffold `day` deps from the git remote — currently the DEFAULT (the day framework
        /// crates are not yet on crates.io); kept for forward compatibility.
        #[arg(long)]
        git: bool,
        /// Scaffold versioned `day` deps from crates.io, pinned to this CLI's version — for
        /// once the day framework crates are published.
        #[arg(long)]
        registry: bool,
        /// Use `path` deps rooted at a local day checkout (CI / framework development).
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
        /// Scaffold `day` deps from the git remote — currently the DEFAULT (the day framework
        /// crates are not yet on crates.io); kept for forward compatibility.
        #[arg(long)]
        git: bool,
        /// Scaffold versioned `day` deps from crates.io, pinned to this CLI's version — for
        /// once the day framework crates are published.
        #[arg(long)]
        registry: bool,
        /// Use `path` deps rooted at a local day checkout (CI / framework development).
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Never prompt; use flags + defaults only (also implied when stdin is not a terminal).
        #[arg(long)]
        no_input: bool,
    },
    /// Scaffold a new Day app (the canonical app command).
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
        /// Window / app-store display title; default: the name, title-cased (`hello-world` ⇒
        /// `Hello World`).
        #[arg(long)]
        title: Option<String>,
        /// Scaffold from a custom template instead of the built-in one: a local directory, or
        /// a git URL (optionally `#ref`). Files are rendered with {{name}}/{{title}}/{{id}}/…
        /// placeholders in contents and paths (see the docs for the full list + conventions).
        #[arg(long)]
        template: Option<String>,
        /// Back-compat: comma-separated target list (prefer repeated --toolkit).
        #[arg(long, hide = true)]
        targets: Option<String>,
        /// Scaffold `day` deps from the git remote — currently the DEFAULT (the day framework
        /// crates are not yet on crates.io); kept for forward compatibility.
        #[arg(long)]
        git: bool,
        /// Scaffold versioned `day` deps from crates.io, pinned to this CLI's version — for
        /// once the day framework crates are published.
        #[arg(long)]
        registry: bool,
        /// Use `path` deps rooted at a local day checkout (CI / framework development).
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Never prompt; use flags + defaults only (also implied when stdin is not a terminal).
        #[arg(long)]
        no_input: bool,
    },
}

#[derive(clap::Subcommand)]
pub enum AppCmd {
    /// Add target(s) to this app: appends to Day.toml `targets:` (comments/formatting
    /// preserved) and materializes any native host projects (platform/…) the targets need,
    /// from the SAME template `day new app` used.
    #[command(name = "add-toolkit")]
    AddToolkit {
        /// Target(s) to add, e.g. `android-mdc` (repeatable / comma-separated)
        targets: Vec<String>,
        /// The template the app was scaffolded from, when not the built-in one (dir or git URL)
        #[arg(long)]
        template: Option<String>,
    },
}

#[derive(clap::Subcommand)]
pub enum OhosCmd {
    /// Manage the OpenHarmony emulator
    Emulator {
        #[command(subcommand)]
        cmd: EmulatorCmd,
    },
}

#[derive(clap::Subcommand)]
pub enum EmulatorCmd {
    /// Launch the Oniro/OpenHarmony QEMU emulator as a native window (no VNC/password)
    Launch {
        /// No window (hdc-only) — for CI / headless hosts.
        #[arg(long)]
        headless: bool,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    // Kick off the background crates.io update check now, so it runs while the command does. Silent for
    // the build-system plumbing callbacks (Xcode/Gradle) and for machine `--format json` output.
    let update = crate::update::spawn(
        cli.format != "json"
            && !matches!(
                cli.command,
                Cmd::XcodeBackend { .. } | Cmd::GradleBackend { .. }
            ),
    );
    let code = match cli.command {
        Cmd::Version => {
            println!("day {}", env!("DAY_VERSION_LONG"));
            0
        }
        Cmd::Doctor { toolkits } => {
            // Doctor works outside a project too, so external discovery is best-effort: inside a
            // project, declared toolkits join the report; elsewhere (or on a discovery failure)
            // the builtin groups stand alone.
            let external = crate::meta::find_project(cli.project.as_deref())
                .ok()
                .and_then(|p| crate::external::resolve(&p).ok())
                .unwrap_or(&[]);
            crate::doctor::run(&toolkits, external)
        }
        Cmd::Rebuild {
            artifact,
            force_tool,
            keep,
            strict,
        } => {
            let opts = crate::rebuild::Options {
                force_tools: force_tool,
                keep,
                strict,
            };
            match crate::rebuild::run(&artifact, &opts) {
                Ok(code) => code,
                Err(e) => {
                    crate::ops::status("Error", &e);
                    1
                }
            }
        }
        Cmd::Pack {
            platforms,
            profile,
            formats,
            no_sign,
            no_notarize,
            no_wait,
            no_version_in_name,
        } => with_project(cli.project.as_deref(), |project| {
            let opts = crate::pack::PackOptions {
                profile,
                formats: formats
                    .as_deref()
                    .map(|s| s.split(',').map(|f| f.trim().to_string()).collect()),
                no_sign,
                no_notarize,
                no_wait,
                version_in_name: !no_version_in_name,
            };
            let mut outcomes = Vec::new();
            for p in &platforms {
                let target = match crate::external::find_target(project, p) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                if crate::external::is_external(target) {
                    // Packaging is per-OS CLI code (dmg/msix/flatpak/…) that a declared desktop
                    // target has none of. Say so plainly rather than fall into some builtin arm.
                    eprintln!(
                        "error: day pack does not support externally declared targets yet — \
                         {} builds and launches, but packaging is Stage 1 (docs/extending.md)",
                        target.name
                    );
                    return 2;
                }
                match crate::pack::run(project, target, &opts) {
                    Ok(o) => outcomes.push(o),
                    Err(e) => {
                        eprintln!("error: {}", e.message());
                        return e.exit_code();
                    }
                }
            }
            if cli.format == "json" {
                print_pack_json(&outcomes);
            }
            0
        }),
        Cmd::Sign {
            check,
            notarize_status,
        } => with_project(cli.project.as_deref(), |project| {
            if let Some(id) = &notarize_status {
                return crate::sign::notarize_status(project, id);
            }
            if check {
                return crate::sign::check(project);
            }
            eprintln!("error: day sign needs --check or --notarize-status <id>");
            2
        }),
        Cmd::App {
            cmd: AppCmd::AddToolkit { targets, template },
        } => with_project(cli.project.as_deref(), |project| {
            crate::new::add_toolkit(project, &targets, template.as_deref())
        }),
        Cmd::Metadata { json, schema } => {
            if schema {
                // Static — the schema needs no project; usable before one exists.
                println!("{}", include_str!("../resources/day-toml.schema.json"));
                return 0;
            }
            with_project(cli.project.as_deref(), |project| {
                crate::metadata::run(project, json)
            })
        }
        Cmd::Lint { strict } => with_project(cli.project.as_deref(), |project| {
            crate::lint::run(project, strict)
        }),
        Cmd::Stop { platforms, all } => with_project(cli.project.as_deref(), |project| {
            let names: Vec<String> = if all {
                crate::sessions::list(&project.root)
                    .into_iter()
                    .map(|s| s.target)
                    .collect()
            } else {
                platforms
            };
            if names.is_empty() {
                eprintln!("error: nothing to stop (no -p targets and no recorded sessions)");
                return 2;
            }
            for name in &names {
                let target = match crate::external::find_target(project, name) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                crate::script::terminate(project, target);
                crate::sessions::remove(&project.root, name);
                ops::status("Stopped", name);
            }
            0
        }),
        Cmd::Relaunch {
            platforms,
            all_running,
            profile,
            locale,
        } => with_project(cli.project.as_deref(), |project| {
            let names: Vec<String> = if all_running || platforms.is_empty() {
                crate::sessions::list(&project.root)
                    .into_iter()
                    .map(|s| s.target)
                    .collect()
            } else {
                platforms
            };
            if names.is_empty() {
                eprintln!("error: no running sessions — `day launch -p <target>` first");
                return 2;
            }
            let spec = ops::LaunchSpec {
                locale,
                // Same debug title tag as a plain launch (docs/windows.md); no script drives a
                // relaunch, so the tag carries version and toolkit only.
                envs: vec![(
                    "DAY_APP_VERSION".into(),
                    project.manifest.app.version.clone(),
                )],
                attached: false,
                // A relaunch re-targets whatever the previous launch put on screen, so it keeps
                // the unfiltered behaviour rather than inventing a device to prefer.
                device: None,
            };
            for (ti, name) in names.iter().enumerate() {
                let target = match crate::external::find_target(project, name) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                crate::script::terminate(project, target);
                let outcome = match ops::build(project, target, &profile) {
                    Ok(o) => o,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 4;
                    }
                };
                let mut spec = spec.clone();
                let port = crate::script::pick_port(ti);
                let token = crate::script::make_token();
                spec.envs.push(("DAYSCRIPT_PORT".into(), port.to_string()));
                spec.envs.push(("DAYSCRIPT_TOKEN".into(), token.clone()));
                match ops::launch(project, target, &outcome, &spec) {
                    Ok(_) => {
                        crate::sessions::record(
                            &project.root,
                            crate::sessions::Session {
                                target: name.clone(),
                                app_id: project.manifest.resolve(name).id,
                                profile: profile.clone(),
                                engine_port: port,
                                engine_token: token,
                                started_at: crate::sessions::now_millis(),
                            },
                        );
                        ops::status("Relaunched", name);
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 1;
                    }
                }
            }
            0
        }),
        Cmd::Drive {
            platform,
            steps_json,
        } => with_project(cli.project.as_deref(), |project| {
            let target = match crate::external::find_target(project, &platform) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 2;
                }
            };
            crate::drive::run(project, target, &steps_json)
        }),
        Cmd::McpServer {} => with_project(cli.project.as_deref(), crate::mcp::run),
        Cmd::Ohos {
            cmd:
                OhosCmd::Emulator {
                    cmd: EmulatorCmd::Launch { headless },
                },
        } => match crate::ohos::emulator_launch(headless) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {e}");
                5
            }
        },
        Cmd::XcodeBackend { .. } => crate::mobile::xcode_backend_build(),
        Cmd::GradleBackend { .. } => crate::mobile::gradle_backend_build(),
        Cmd::New { what } => match what {
            None => crate::new::interactive(),
            Some(NewKind::Piece {
                name,
                toolkits,
                composite,
                id,
                git,
                registry,
                local,
                no_input,
            }) => crate::new::piece(
                name.as_deref(),
                toolkits.as_deref(),
                composite,
                id.as_deref(),
                local.as_deref(),
                git,
                registry,
                no_input,
            ),
            Some(NewKind::Part {
                name,
                platforms,
                id,
                git,
                registry,
                local,
                no_input,
            }) => crate::new::part(
                name.as_deref(),
                platforms.as_deref(),
                id.as_deref(),
                local.as_deref(),
                git,
                registry,
                no_input,
            ),
            Some(NewKind::App {
                name,
                toolkits,
                appid,
                bundleid,
                id,
                title,
                template,
                targets,
                git,
                registry,
                local,
                no_input,
            }) => crate::new::app(
                name.as_deref(),
                &toolkits,
                appid.as_deref(),
                bundleid.as_deref(),
                id.as_deref(),
                title.as_deref(),
                template.as_deref(),
                targets.as_deref(),
                local.as_deref(),
                git,
                registry,
                no_input,
            ),
        },
        Cmd::Build { platforms, profile } => with_project(cli.project.as_deref(), |project| {
            let mut results = Vec::new();
            for p in &platforms {
                let target = match crate::external::find_target(project, p) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("error: {e}");
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
            keep_alive,
            scripts,
            variant,
            skip_build,
            locales,
            themes,
            device,
        } => with_project(cli.project.as_deref(), |project| {
            // No `-p`: run what this machine natively is. Announced rather than assumed — the
            // chosen target decides which toolkit gets built, so a silent pick would be a
            // surprising several-minute build of something the caller did not name.
            let platforms = if platforms.is_empty() {
                let default = crate::targets::host_default();
                ops::status("Defaulting", &format!("{default} (no --platform given)"));
                vec![default.to_string()]
            } else {
                platforms.clone()
            };
            let script_mode = !scripts.is_empty();
            let mut spec = ops::LaunchSpec {
                locale: locale.clone(),
                device: device.clone(),
                envs: envs
                    .iter()
                    .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
                    .collect(),
                // Attachment follows `--detach` alone, NOT whether a script runs: a scripted
                // launch streams the app's console output the same as a plain launch. (A
                // `--keep-alive` scripted run additionally keeps `day` in the foreground after the
                // script so that output stays visible while the app lives — see below.)
                attached: !detach,
            };
            // Ctrl-C during an attached run must take the launched apps and their log
            // watchers (simctl / adb logcat) down too — not leave them orphaned.
            if spec.attached {
                crate::signals::install();
            }
            // The debug window-title tag (docs/windows.md): which build, which toolkit, and —
            // when a script is driving — which script. The app reads these off the environment
            // and only shows them in a debug build.
            spec.envs.push((
                "DAY_APP_VERSION".into(),
                project.manifest.app.version.clone(),
            ));
            if script_mode {
                // The app is launched once and the scripts run in sequence against it, so the
                // title names all of them.
                let names: Vec<String> = scripts
                    .iter()
                    .map(|s| {
                        std::path::Path::new(s)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| s.to_string_lossy().into_owned())
                    })
                    .collect();
                spec.envs.push(("DAY_SCRIPT".into(), names.join(",")));
            }
            let token = crate::script::make_token();
            // The capture matrix (--themes × --locales): the scripted runs each target performs,
            // with the variant names both CI shapes already produce — the day-CI/gallery
            // `<theme>`/`<theme>-<locale>` convention and the app-CI `<locale>` convention are
            // preserved byte-for-byte so existing artifact layouts survive the move from YAML
            // loops into the CLI. No flags = one run with the plain --locale/--variant.
            let matrix = match capture_matrix(&themes, &locales, &locale, &variant, &envs) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 2;
                }
            };
            let mut handles = Vec::new();
            let mut script_failures = 0usize;
            for (ti, p) in platforms.iter().enumerate() {
                let port = crate::script::pick_port(ti);
                // The dayscript engine rides EVERY launch (loopback, token-gated): scripted runs
                // drive it immediately, and interactive launches stay drivable later via the
                // session registry (`day drive` / `day relaunch` / agents — docs/agent.md).
                spec.envs
                    .retain(|(k, _)| k != "DAYSCRIPT_PORT" && k != "DAYSCRIPT_TOKEN");
                spec.envs.push(("DAYSCRIPT_PORT".into(), port.to_string()));
                spec.envs.push(("DAYSCRIPT_TOKEN".into(), token.clone()));
                let target = match crate::external::find_target(project, p) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                let built = if skip_build {
                    ops::reuse_build(project, target, &profile)
                } else {
                    ops::build(project, target, &profile)
                };
                let outcome = match built {
                    Ok(o) => o,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 4;
                    }
                };
                for (ri, capture) in matrix.iter().enumerate() {
                    // Per-run spec: the matrix's locale and theme ride the SAME channels the
                    // old YAML loops used (the --locale plumbing and the DAY_THEME env).
                    let mut run_spec = spec.clone();
                    if let Some(l) = &capture.locale {
                        run_spec.locale = Some(l.clone());
                    }
                    if let Some(t) = &capture.theme {
                        run_spec.envs.push(("DAY_THEME".into(), t.clone()));
                    }
                    let mut attempt = 0u32;
                    loop {
                        if ri > 0 || attempt > 0 {
                            // The previous run's app still holds the engine port — stop it the
                            // way `day stop` does before the next instance binds.
                            crate::script::terminate(project, target);
                        }
                        match ops::launch(project, target, &outcome, &run_spec) {
                            Ok(h) => {
                                crate::sessions::record(
                                    &project.root,
                                    crate::sessions::Session {
                                        target: p.clone(),
                                        app_id: project.manifest.resolve(p).id,
                                        profile: profile.clone(),
                                        engine_port: port,
                                        engine_token: token.clone(),
                                        started_at: crate::sessions::now_millis(),
                                    },
                                );
                                handles.push(h);
                            }
                            Err(e) => {
                                eprintln!("error: {e}");
                                return 1;
                            }
                        }
                        if !script_mode {
                            break;
                        }
                        match crate::script::run_scripts(
                            project,
                            target,
                            port,
                            &token,
                            &scripts,
                            run_spec.locale.as_deref(),
                            capture.variant.as_deref(),
                            keep_alive,
                            spec.attached,
                        ) {
                            Ok(run) => {
                                script_failures += run.steps_failed;
                                let tag = capture
                                    .variant
                                    .as_deref()
                                    .map(|v| format!(" [{v}]"))
                                    .unwrap_or_default();
                                ops::status(
                                    "Script",
                                    &format!(
                                        "{}{tag}: {}/{} steps passed · {} screenshot(s)",
                                        target.name,
                                        run.steps_total - run.steps_failed,
                                        run.steps_total,
                                        run.screenshots.len()
                                    ),
                                );
                                break;
                            }
                            // The iOS simulator's known app-death flake: the engine died with
                            // ZERO failed steps. Retry the (idempotent) run once — the logic
                            // both CI workflows used to grep logs for, now typed. A loss AFTER
                            // a failed step is a failing run that then died: report it.
                            Err(crate::script::ScriptError::EngineLost {
                                steps_failed: 0, ..
                            }) if target.kind == crate::targets::TargetKind::IosSim
                                && attempt == 0 =>
                            {
                                eprintln!(
                                    "warning: engine connection lost (flaky simulator \
                                     app-death) — retrying the script once"
                                );
                                // Keep the annotation CI used to emit from its own retry wrapper.
                                if std::env::var_os("GITHUB_ACTIONS").is_some() {
                                    println!(
                                        "::warning::engine connection lost (flaky simulator \
                                         app-death) — retrying the script once"
                                    );
                                }
                                attempt += 1;
                            }
                            // An engine loss that survived the retry policy: count it and
                            // move to the NEXT matrix run instead of abandoning the rest —
                            // the CI loops this replaced continued per variant (OHOS relies
                            // on it under TCG), and the final exit code still reports failure.
                            Err(crate::script::ScriptError::EngineLost {
                                steps_failed, ..
                            }) => {
                                eprintln!(
                                    "error: engine connection lost — abandoning this variant"
                                );
                                script_failures += steps_failed.max(1);
                                break;
                            }
                            // Anything else is a runner/config error (bad script, bad flags):
                            // abort outright, as before.
                            Err(e) => {
                                eprintln!("error: {e}");
                                return 5;
                            }
                        }
                    }
                    // Between matrix runs the app must exit so the next launch re-binds the
                    // engine port (each variant is a fresh process, as the CI loops had it).
                    if script_mode && ri + 1 < matrix.len() {
                        crate::script::terminate(project, target);
                    }
                }
            }
            // A scripted run returns once its script(s) finish — EXCEPT an attached
            // `--keep-alive` run, which stays in the foreground streaming the app's console
            // output until the app exits or the run is stopped (so output is visible during AND
            // after the script, exactly like a plain attached launch). Detached scripted runs
            // (agents) and non-keep-alive scripted runs (CI) return here without blocking on
            // device log pumps that never EOF; attached runs already streamed logs live while
            // the script drove the app.
            //
            // Reap the tracked children first (`--keep-alive` is what asks for the app to stay
            // running). Returning without this leaves the log pumps (`adb logcat`, `simctl
            // launch --console`) orphaned holding the inherited stdout/stderr — in CI the step's
            // pipe then never reaches EOF and the job hangs after the final "steps passed" line.
            if script_mode && !(spec.attached && keep_alive) {
                crate::signals::kill_all();
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
                if script_mode && script_failures > 0 {
                    5
                } else {
                    code
                }
            } else {
                0
            }
        }),
    };
    // Non-blocking: nudge only if the crates.io reply already arrived; never waits for it.
    crate::update::finish(update);
    code
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

fn print_pack_json(outcomes: &[crate::pack::PackOutcome]) {
    let targets: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| {
            let artifacts: Vec<serde_json::Value> = o
                .artifacts
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "path": a.path, "kind": a.kind,
                        "sha256": a.sha256, "signed": a.tier.as_str(),
                    })
                })
                .collect();
            serde_json::json!({
                "target": o.target, "ok": true, "code": 0,
                "artifacts": artifacts, "seconds": o.seconds,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({"event": "result", "command": "pack", "ok": true, "targets": targets})
    );
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

/// One scripted run of the capture matrix: which theme to force, which locale to pass, and the
/// variant directory the screenshots land in.
#[derive(Debug)]
struct CaptureRun {
    theme: Option<String>,
    locale: Option<String>,
    variant: Option<String>,
}

/// Accept both `--themes light,dark` and `--themes "light dark"` — the CI matrix variables are
/// space-separated and pass through as one argument.
fn split_list(raw: &[String]) -> Vec<String> {
    raw.iter()
        .flat_map(|s| s.split([',', ' ']))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Expand `--themes` × `--locales` into runs, preserving BOTH existing artifact conventions
/// byte-for-byte (they predate this flag and live on in the gallery config and the app CIs):
///
/// - locales alone → one run per locale, `--locale <l>` passed for every locale INCLUDING the
///   default, variant `<l>` — what the app CIs' shell loops produced.
/// - themes × locales → variant `<theme>` when the locale is `en` (run in the default locale,
///   no `--locale` flag), `<theme>-<locale>` otherwise — the day-CI / gallery convention
///   (website/gallery.config.mjs lists exactly these ids).
///
/// The `en` asymmetry between the modes is deliberate compatibility, not design: renaming
/// artifact directories would break every consumer that globs them.
fn capture_matrix(
    themes_raw: &[String],
    locales_raw: &[String],
    locale: &Option<String>,
    variant: &Option<String>,
    envs: &[String],
) -> Result<Vec<CaptureRun>, String> {
    let themes = split_list(themes_raw);
    let locales = split_list(locales_raw);
    if themes.is_empty() && locales.is_empty() {
        return Ok(vec![CaptureRun {
            theme: None,
            locale: locale.clone(),
            variant: variant.clone(),
        }]);
    }
    if variant.is_some() {
        return Err(
            "--variant names ONE run; --themes/--locales name each run themselves — drop \
             --variant"
                .into(),
        );
    }
    if locale.is_some() && !locales.is_empty() {
        return Err("--locale conflicts with --locales (which passes a locale per run)".into());
    }
    if !themes.is_empty() && envs.iter().any(|kv| kv.starts_with("DAY_THEME=")) {
        return Err("--themes sets DAY_THEME per run — drop the --env DAY_THEME override".into());
    }
    let mut out = Vec::new();
    if themes.is_empty() {
        for l in &locales {
            out.push(CaptureRun {
                theme: None,
                locale: Some(l.clone()),
                variant: Some(l.clone()),
            });
        }
    } else if locales.is_empty() {
        for t in &themes {
            out.push(CaptureRun {
                theme: Some(t.clone()),
                locale: locale.clone(),
                variant: Some(t.clone()),
            });
        }
    } else {
        for t in &themes {
            for l in &locales {
                let (run_locale, run_variant) = if l == "en" {
                    (None, t.clone())
                } else {
                    (Some(l.clone()), format!("{t}-{l}"))
                };
                out.push(CaptureRun {
                    theme: Some(t.clone()),
                    locale: run_locale,
                    variant: Some(run_variant),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod capture_matrix_tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The day-CI shape: themes × locales, with `en` implicit — the exact variant ids the
    /// gallery config lists.
    #[test]
    fn themes_cross_locales_the_gallery_way() {
        let m = capture_matrix(&v(&["light dark"]), &v(&["en,fr"]), &None, &None, &[]).unwrap();
        let got: Vec<(Option<&str>, Option<&str>, Option<&str>)> = m
            .iter()
            .map(|r| {
                (
                    r.theme.as_deref(),
                    r.locale.as_deref(),
                    r.variant.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (Some("light"), None, Some("light")),
                (Some("light"), Some("fr"), Some("light-fr")),
                (Some("dark"), None, Some("dark")),
                (Some("dark"), Some("fr"), Some("dark-fr")),
            ]
        );
    }

    /// The app-CI shape: locales alone, every locale passed explicitly, variant = locale.
    #[test]
    fn locales_alone_the_app_ci_way() {
        let m = capture_matrix(&[], &v(&["en", "fr"]), &None, &None, &[]).unwrap();
        let got: Vec<(Option<&str>, Option<&str>)> = m
            .iter()
            .map(|r| (r.locale.as_deref(), r.variant.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![(Some("en"), Some("en")), (Some("fr"), Some("fr"))]
        );
    }

    /// No matrix flags: one run carrying the plain --locale/--variant through unchanged.
    #[test]
    fn no_flags_is_one_plain_run() {
        let m = capture_matrix(&[], &[], &Some("ar".into()), &Some("rtl".into()), &[]).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].locale.as_deref(), Some("ar"));
        assert_eq!(m[0].variant.as_deref(), Some("rtl"));
        assert!(m[0].theme.is_none());
    }

    /// The conflicts each produce an instruction, not a mystery.
    #[test]
    fn conflicts_are_rejected_with_instructions() {
        assert!(
            capture_matrix(&v(&["light"]), &[], &None, &Some("x".into()), &[])
                .unwrap_err()
                .contains("--variant")
        );
        assert!(
            capture_matrix(&[], &v(&["fr"]), &Some("ar".into()), &None, &[])
                .unwrap_err()
                .contains("--locales")
        );
        assert!(
            capture_matrix(&v(&["dark"]), &[], &None, &None, &v(&["DAY_THEME=light"]))
                .unwrap_err()
                .contains("DAY_THEME")
        );
    }
}
