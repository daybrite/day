// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Command tree (DESIGN.md §16.5). v0: new / build / launch / doctor; the remaining
//! porcelain (sign / pack / lint / script) lands with M6–M8.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::meta;
use crate::ops;

/// Which failure a [`CliError`] reports, and the one place a kind maps to an exit code
/// (§16.2/§16.3). Everything [`run`] renders funnels through [`ErrKind::exit_code`]; command
/// code that reports a verdict itself (lint findings, icon drift, script failures) quotes the
/// same map instead of a literal, so no code is ever assigned twice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrKind {
    /// Anything without a more specific kind: launch, runtime, and file failures.
    Failure,
    /// Wrong invocation or configuration: bad flags, an unknown target, no project.
    Usage,
    /// The development environment refused: doctor errors, no live session to attach to.
    Env,
    /// A build or artifact-generation step failed (cargo, xcodebuild, gradle, hvigor, icons).
    Build,
    /// A scripted run failed, or the device side of one stopped answering.
    Script,
    /// `day icon --check` found outputs drifted from the master (the CI drift gate).
    Drift,
    /// Signing / notarization (`day sign`, pack's signing stages).
    Sign,
    /// `day lint --strict` findings.
    Lint,
}

impl ErrKind {
    /// The exit-code contract. These numbers are FROZEN: CI walkthroughs assert them.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrKind::Failure => 1,
            ErrKind::Usage => 2,
            ErrKind::Env => 3,
            ErrKind::Build => 4,
            ErrKind::Script | ErrKind::Drift => 5,
            ErrKind::Sign => 6,
            ErrKind::Lint => 10,
        }
    }
}

/// A command failure: the message the old code printed at its ~40 call sites, plus the kind
/// that picks its exit code. Command entry points return `Result<_, CliError>` and [`run`]
/// renders once: `error: <message>` on stderr, exit code from the kind.
///
/// `Debug` so a test can `.expect()` on a `Result<_, CliError>` and read what went wrong.
#[derive(Debug)]
pub struct CliError {
    kind: ErrKind,
    message: String,
}

impl CliError {
    fn new(kind: ErrKind, message: impl Into<String>) -> Self {
        CliError {
            kind,
            message: message.into(),
        }
    }
    pub fn failure(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Failure, message)
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Usage, message)
    }
    pub fn env(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Env, message)
    }
    pub fn build(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Build, message)
    }
    pub fn script(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Script, message)
    }
    pub fn drift(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Drift, message)
    }
    pub fn sign(message: impl Into<String>) -> Self {
        Self::new(ErrKind::Sign, message)
    }
    pub fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Internal helpers keep `Result<T, String>`; a bare String converting at a command boundary
/// is the generic failure (exit 1). Boundaries that know better use a constructor
/// (`.map_err(CliError::build)` and friends).
impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::new(ErrKind::Failure, message)
    }
}

/// `pack::PackError`'s own mapping (Sign → 6, everything else a build failure → 4), preserved.
impl From<crate::pack::PackError> for CliError {
    fn from(e: crate::pack::PackError) -> Self {
        match e {
            crate::pack::PackError::Sign(m) => Self::new(ErrKind::Sign, m),
            crate::pack::PackError::Other(m) => Self::new(ErrKind::Build, m),
        }
    }
}

/// `--profile`, typed: a typo (`relaese`) fails at argument parsing instead of string-comparing
/// its way into the debug branch of every `profile == "release"` site.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    /// The spelling the flag accepts; also the build-directory name and the `DAY_PROFILE` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `--format`, typed: day's own status lines, or NDJSON result events for tooling.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum OutputFormat {
    Plain,
    Json,
}

#[derive(Parser)]
#[command(
    name = "day",
    version = env!("DAY_VERSION_LONG"),
    about = "Day — cross-platform apps in Rust with native toolkits",
    styles = crate::term::help_styles(),
    before_help = crate::term::help_banner(),
    after_help = crate::term::help_footer(),
    max_term_width = 100
)]
pub(crate) struct Cli {
    /// Project directory (default: nearest Day.toml in this directory or its parents)
    #[arg(long, global = true)]
    project: Option<PathBuf>,
    /// Output format for command results
    #[arg(long, global = true, value_enum, default_value = "plain")]
    format: OutputFormat,
    /// Show output from build tools and other subprocesses (also DAY_VERBOSE=1)
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the CLI version and build information
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    Version,
    /// Create an app, piece, or part; prompt when no kind is given
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    New {
        #[command(subcommand)]
        what: Option<NewKind>,
        /// Print the project-creation options as JSON without creating files
        #[arg(long)]
        describe: bool,
    },
    /// Build an app for one or more targets
    #[command(after_help = "Docs: https://daybrite.dev/docs/project-structure/")]
    Build {
        /// Targets to build (repeatable)
        #[arg(short = 'p', long = "platform", required = true)]
        platforms: Vec<String>,
        /// Build profile
        #[arg(long, value_enum, default_value = "debug")]
        profile: Profile,
        /// Use a Day checkout or Git URL for this build only; leave project settings unchanged
        #[arg(long = "day-src", value_name = "PATH|URL[@REF]")]
        day_src: Option<String>,
    },
    /// Generate app icons for each platform
    #[command(after_help = "Docs: https://daybrite.dev/docs/guide-icons/")]
    Icon {
        /// Source icon (default: resource/icons/icon.svg, day-icon.svg, or icon.png)
        master: Option<PathBuf>,
        /// Check generated icons without writing files; exit 5 if they differ
        #[arg(long, conflicts_with = "generate")]
        check: bool,
        /// Generate icons for these targets (repeatable; default: all)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Create resource/icons/icon.svg and generate platform icons
        #[arg(long, conflicts_with = "master")]
        generate: bool,
        /// Integer or text seed for --generate (default: random; printed in output)
        #[arg(long, requires = "generate")]
        seed: Option<String>,
        /// Allow --generate to replace an existing source icon
        #[arg(long, requires = "generate")]
        overwrite: bool,
        /// Write a preview SVG and PNG to this path; leave the project unchanged
        #[arg(long, requires = "generate", value_name = "FILE.svg")]
        out: Option<PathBuf>,
    },
    /// Build and run an app on one or more targets
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    Launch {
        /// Targets to launch (repeatable; default: this host's desktop toolkit)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Clone and run a trusted Git repository; append @REF for a branch, tag, or commit.
        /// Use --project to select an app within the repository
        #[arg(long, value_name = "URL[@REF]")]
        git: Option<String>,
        /// Clone --git into this directory instead of the cache
        #[arg(long, requires = "git", value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Use a Day checkout or Git URL for this run only; leave project settings unchanged
        #[arg(long = "day-src", value_name = "PATH|URL[@REF]")]
        day_src: Option<String>,
        /// Build profile
        #[arg(long, value_enum, default_value = "debug")]
        profile: Profile,
        /// App locale as a BCP-47 tag, such as en or fr-CA
        #[arg(long)]
        locale: Option<String>,
        /// Environment variable passed to the app as K=V (repeatable)
        #[arg(long = "env")]
        envs: Vec<String>,
        /// Physical iPhone or iPad name or UDID; requires a provisioning profile
        #[arg(long = "ios-device", value_name = "NAME|UDID")]
        ios_device: Option<String>,
        /// iOS simulator name or UDID (default: all booted simulators)
        #[arg(long = "ios-simulator", alias = "device", value_name = "NAME|UDID")]
        ios_simulator: Option<String>,
        /// Android adb serial (default: ANDROID_SERIAL, then all connected devices)
        #[arg(long = "android-device", value_name = "SERIAL")]
        android_device: Option<String>,
        /// OpenHarmony hdc key (default: DAY_OHOS_TARGET, then all reachable devices)
        #[arg(long = "ohos-device", value_name = "KEY")]
        ohos_device: Option<String>,
        /// Leave apps running in the background; stop them with day stop
        #[arg(long, alias = "detached")]
        detach: bool,
        /// Keep the app running after its scripts finish
        #[arg(long)]
        keep_alive: bool,
        /// Record app interactions to a dayscript file
        #[arg(long = "record", value_name = "PATH")]
        record: Option<PathBuf>,
        /// Run a dayscript file after launch (repeatable)
        #[arg(long = "script")]
        scripts: Vec<PathBuf>,
        /// Screenshot variant directory (default: derived from the locale)
        #[arg(long)]
        variant: Option<String>,
        /// Device label in screenshot paths; use device-selection flags to choose hardware
        #[arg(long = "device-slug")]
        device: Option<String>,
        /// Launch the existing build; fail if no build is available
        #[arg(long)]
        skip_build: bool,
        /// Run scripts for each locale (comma- or space-separated; repeatable)
        #[arg(long = "locales", requires = "scripts")]
        locales: Vec<String>,
        /// Run scripts for each theme, setting DAY_THEME (combine with --locales)
        #[arg(long = "themes", requires = "scripts")]
        themes: Vec<String>,
        /// Desktop screenshot size: WxH, WxH@SCALE, or window. Overrides Day.toml and
        /// DAY_CAPTURE_SIZE; default: 2560x1600@2. Ignored on phones and tablets
        #[arg(
            long = "capture-size",
            value_name = "WxH[@SCALE]",
            requires = "scripts"
        )]
        capture_size: Option<String>,
    },
    /// Rebuild a package from its recorded sources and compare the result
    #[command(after_help = "Docs: https://daybrite.dev/docs/reproducible-builds/")]
    Rebuild {
        /// Package to verify (.dmg, .ipa, .apk, .flatpak, .msix, or .hap)
        artifact: std::path::PathBuf,
        /// Ignore a tool-version mismatch (repeatable; use all for every tool)
        #[arg(long = "force-tool")]
        force_tool: Vec<String>,
        /// Keep the temporary checkout and build files
        #[arg(long)]
        keep: bool,
        /// Fail if the package contents cannot be compared
        #[arg(long)]
        strict: bool,
        /// Rebuild from a local project instead of cloning the recorded source
        #[arg(long = "from-dir", value_name = "DIR")]
        from_dir: Option<std::path::PathBuf>,
    },
    /// Build, sign, and package an app for distribution
    #[command(after_help = "Docs: https://daybrite.dev/docs/packaging/")]
    Pack {
        /// Targets to build (repeatable)
        #[arg(short = 'p', long = "platform", required = true)]
        platforms: Vec<String>,
        /// Build profile for the package
        #[arg(long, value_enum, default_value = "release")]
        profile: Profile,
        /// Package formats to produce, comma-separated (for example, apk)
        #[arg(long)]
        formats: Option<String>,
        /// Produce an unsigned package
        #[arg(long)]
        no_sign: bool,
        /// Skip macOS notarization
        #[arg(long)]
        no_notarize: bool,
        /// Submit for notarization without waiting for the result
        #[arg(long)]
        no_wait: bool,
        /// Omit the app version from package filenames
        #[arg(long)]
        no_version_in_name: bool,
        /// Package filename prefix; overrides [app] artifact in Day.toml
        #[arg(long = "artifact-name", value_name = "STEM")]
        artifact_name: Option<String>,
    },
    /// Check signing settings or notarization status
    #[command(after_help = "Docs: https://daybrite.dev/docs/packaging/#signing-configuration")]
    Sign {
        /// Check signing credentials and files without printing secrets
        #[arg(long)]
        check: bool,
        /// Check a notarization submission by ID
        #[arg(long = "notarize-status")]
        notarize_status: Option<String>,
    },
    /// Check installed toolchains and SDKs
    #[command(after_help = "Docs: https://daybrite.dev/docs/system-requirements/")]
    Doctor {
        /// Check a toolkit and fail if required tools are missing (repeatable).
        /// Values: appkit, uikit, gtk, qt, xaml, android, harmonyos, dom
        #[arg(long = "toolkit")]
        toolkits: Vec<String>,
    },
    /// Build and package test apps to check this machine
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#checking-the-machine")]
    Checkup {
        /// Targets to check (repeatable or comma-separated; default: available host targets)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Build profile for both compilation and packaging
        #[arg(long, value_enum, default_value = "debug")]
        profile: Profile,
        /// Build test apps without packaging them
        #[arg(long)]
        no_pack: bool,
        /// Fail if missing tools prevent checking a target supported by this host
        #[arg(long)]
        strict: bool,
        /// Create test projects in this directory instead of a temporary directory
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Keep the generated test projects
        #[arg(long)]
        keep: bool,
        /// Use Day dependencies from Git (default)
        #[arg(long)]
        git: bool,
        /// Use Day dependencies from crates.io at the selected version
        #[arg(long)]
        registry: bool,
        /// Day version to check: release, latest, branch, or commit (default: this CLI)
        #[arg(long = "day-version", value_name = "SPEC")]
        day_version: Option<String>,
        /// Use Day dependencies from a local checkout
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
    },
    /// Manage an existing app project
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Show project settings and supported targets
    #[command(after_help = "Docs: https://daybrite.dev/docs/project-structure/")]
    Metadata {
        /// Print project metadata as JSON
        #[arg(long)]
        json: bool,
        /// Print the Day.toml JSON Schema and exit
        #[arg(long)]
        schema: bool,
    },
    /// Check project files, translations, and element IDs
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#linting")]
    Lint {
        /// Exit 10 if findings remain, except codes listed with --allow
        #[arg(long)]
        strict: bool,
        /// Report this finding code without failing --strict (repeatable)
        #[arg(long = "allow", value_name = "CODE")]
        allow: Vec<String>,
        /// Print findings as JSON (same as --format json)
        #[arg(long)]
        json: bool,
        /// Apply available automatic fixes, excluding allowed finding codes
        #[arg(long)]
        fix: bool,
    },
    /// Change Day dependency sources in .cargo/config.toml, or verify them
    #[command(after_help = "Docs: https://daybrite.dev/docs/local-development/")]
    Patch {
        /// Local Day, piece, or part checkout to use (repeatable)
        #[arg(long, value_name = "CHECKOUT")]
        local: Vec<std::path::PathBuf>,
        /// Day Git fork to use for all dependencies; append @REF for a branch, tag, or commit
        #[arg(long, value_name = "URL[@REF]")]
        git: Option<String>,
        /// Fail if patched dependencies still resolve to the original Git source
        #[arg(long)]
        check: bool,
    },
    /// Generate the icons and source files required by native projects
    #[command(after_help = "Docs: https://daybrite.dev/docs/guide-icons/#native-projects-and-ci")]
    Prepare {
        /// Targets to prepare (repeatable; default: all targets in Day.toml)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Check generated files without writing; exit 5 if files are missing or stale
        #[arg(long, conflicts_with = "migrate")]
        check: bool,
        /// Migrate to generated assets: remove old generated files and update native projects
        #[arg(long)]
        migrate: bool,
    },
    /// Prepare and open a native project in its IDE
    #[command(after_help = "Docs: https://daybrite.dev/docs/project-structure/")]
    Open {
        /// Target to open in Xcode, Android Studio, or DevEco Studio
        #[arg(short = 'p', long = "platform")]
        platform: String,
    },
    /// Create and prepare app store listings
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#store-listings")]
    Store {
        #[command(subcommand)]
        cmd: StoreCmd,
    },
    /// Manage locales across the app, store listings, and website
    #[command(after_help = "Docs: https://daybrite.dev/docs/localization/")]
    Localize {
        #[command(subcommand)]
        cmd: LocalizeCmd,
    },
    /// Build a gallery index from screenshots
    #[command(after_help = "Docs: https://daybrite.dev/docs/dayscript/")]
    Screenshot {
        #[command(subcommand)]
        cmd: ScreenshotCmd,
    },
    /// Tools for web apps
    #[command(after_help = "Docs: https://daybrite.dev/docs/platforms/web-dom/")]
    Web {
        #[command(subcommand)]
        cmd: WebCmd,
    },
    /// Stop apps launched by Day
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    Stop {
        /// Targets to stop (repeatable)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Stop all recorded sessions for this project
        #[arg(long)]
        all: bool,
    },
    /// Stop recorded sessions and remove build outputs
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    Clean {
        /// List files and their sizes without deleting them
        #[arg(long)]
        dry_run: bool,
    },
    /// Stop, rebuild, and restart apps
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    Relaunch {
        /// Targets to restart (repeatable; omit with --all-running)
        #[arg(short = 'p', long = "platform")]
        platforms: Vec<String>,
        /// Restart all recorded sessions for this project
        #[arg(long)]
        all_running: bool,
        /// Build profile
        #[arg(long, value_enum, default_value = "debug")]
        profile: Profile,
        /// App locale as a BCP-47 tag, such as en or fr-CA
        #[arg(long)]
        locale: Option<String>,
    },
    /// Run dayscript steps in a running app
    #[command(after_help = "Docs: https://daybrite.dev/docs/for-agents/")]
    Drive {
        /// Target with the running app to control
        #[arg(short = 'p', long = "platform")]
        platform: String,
        /// Dayscript steps as a JSON array
        #[arg(long)]
        steps_json: String,
    },
    /// Serve Day tools over MCP using standard input and output
    #[command(after_help = "Docs: https://daybrite.dev/docs/for-agents/")]
    McpServer {},
    /// List and manage simulators, emulators, and devices
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#simulators-emulators-and-devices")]
    Devices {
        #[command(subcommand)]
        cmd: DevicesCmd,
    },
    /// Tools for HarmonyOS and OpenHarmony
    #[command(after_help = "Docs: https://daybrite.dev/docs/platforms/harmony-arkui/")]
    Ohos {
        #[command(subcommand)]
        cmd: OhosCmd,
    },
    /// Run the native Xcode build callback (internal)
    #[command(name = "xcode-backend", hide = true)]
    #[command(after_help = "Docs: https://daybrite.dev/docs/project-structure/")]
    XcodeBackend {
        /// Build action requested by the native project
        #[arg(default_value = "build")]
        action: String,
    },
    /// Run the native Gradle build callback (internal)
    #[command(name = "gradle-backend", hide = true)]
    #[command(after_help = "Docs: https://daybrite.dev/docs/project-structure/")]
    GradleBackend {
        /// Build action requested by the native project
        #[arg(default_value = "build")]
        action: String,
    },
}

/// `day new <piece|part|app>` scaffolds an extension crate or app; `day new` (bare) walks an
/// interactive dialog. Every value-carrying flag has an equivalent question in the dialog (the dialog
/// is the fallback branch of the flags; see `new.rs`), so a value not passed on the command line is
/// asked for when a terminal is present, and defaulted (or reported as required) when it is not. The
/// meta flags `--local` (CI) and `--no-input` have no dialog fallback.
///
/// Scaffolds default to remote (git) day dependencies so they are self-contained; the hidden
/// `--local <path>` (or `DAY_LOCAL` env) redirects to a local day checkout for CI checks of a
/// freshly-scaffolded project against the day tree under test.
/// `day store …`: the canonical listing under `store/`, and the fastlane trees it generates.
#[derive(Subcommand)]
pub enum StoreCmd {
    /// Create store listing files for each locale without replacing existing files
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#store-listings")]
    Init,
    /// Prepare store listings for upload with fastlane
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#store-listings")]
    Stage {
        /// Target to prepare (default: all store targets)
        #[arg(short = 'p', long = "platform")]
        target: Option<String>,
    },
}

/// `day localize …`: the four places a conventional project spells its locale set
/// (`resource/locales/`, `store/`, the iOS `knownRegions`, `website/site.toml`), kept in step.
#[derive(Subcommand)]
pub enum WebCmd {
    /// Print the path to the bundled browser automation driver
    #[command(after_help = "Docs: https://daybrite.dev/docs/platforms/web-dom/")]
    Driver,
}

#[derive(Subcommand)]
pub enum ScreenshotCmd {
    /// Merge screenshot directories into gallery.json
    #[command(after_help = "Docs: https://daybrite.dev/docs/dayscript/")]
    Index {
        /// Screenshot directories (repeatable; default: build/day/screenshots)
        #[arg(long = "screenshot-paths", value_name = "PATH", num_args = 1..)]
        screenshot_paths: Vec<PathBuf>,
        /// Output file (default: gallery.json in the first screenshot directory)
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum LocalizeCmd {
    /// List locales and report differences between app, store, and website
    #[command(after_help = "Docs: https://daybrite.dev/docs/localization/")]
    List,
    /// Add locales to the app, store listings, and website
    #[command(after_help = "Docs: https://daybrite.dev/docs/localization/")]
    Add {
        /// Locale tags, such as fr or de (repeatable or comma-separated)
        locales: Vec<String>,
    },
    /// Remove locales from the project; the default locale cannot be removed
    #[command(after_help = "Docs: https://daybrite.dev/docs/localization/")]
    Remove {
        /// Locale tags to remove (repeatable or comma-separated)
        locales: Vec<String>,
    },
}

#[derive(Subcommand)]
enum NewKind {
    /// Create a reusable UI component
    #[command(after_help = "Docs: https://daybrite.dev/docs/extending/")]
    Piece {
        /// Crate name (prompted if omitted)
        name: Option<String>,
        /// Native toolkits, comma-separated (appkit, gtk, qt, uikit, mdc, xaml).
        /// Omit to create a component using existing pieces
        #[arg(long)]
        toolkits: Option<String>,
        /// Create a component using existing pieces, even when --toolkits is set
        #[arg(long)]
        composite: bool,
        /// Package ID in reverse-DNS form (default: dev.example.<name>)
        #[arg(long)]
        id: Option<String>,
        /// Use Day dependencies from Git (default)
        #[arg(long)]
        git: bool,
        /// Use Day dependencies from crates.io at the selected version
        #[arg(long)]
        registry: bool,
        /// Day dependency version: release, latest, branch, or commit
        #[arg(long = "day-version", value_name = "SPEC")]
        day_version: Option<String>,
        /// Use Day dependencies from a local checkout
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Use flags and defaults without prompting (automatic when stdin is not a terminal)
        #[arg(long)]
        no_input: bool,
        /// Skip the demo app
        #[arg(long)]
        no_demo: bool,
        /// Put Android Java in src/ (default); use false for platform/android/java/
        #[arg(
            long,
            value_name = "BOOL",
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = "true"
        )]
        java_in_src: Option<bool>,
    },
    /// Create a platform service without a user interface
    #[command(after_help = "Docs: https://daybrite.dev/docs/tutorial-part/")]
    Part {
        /// Crate name (prompted if omitted)
        name: Option<String>,
        /// Platforms, comma-separated (macos, ios, android, linux, windows; default: all)
        #[arg(long)]
        platforms: Option<String>,
        /// Package ID in reverse-DNS form (default: dev.example.<name>)
        #[arg(long)]
        id: Option<String>,
        /// Use Day dependencies from Git (default)
        #[arg(long)]
        git: bool,
        /// Use Day dependencies from crates.io at the selected version
        #[arg(long)]
        registry: bool,
        /// Day dependency version: release, latest, branch, or commit
        #[arg(long = "day-version", value_name = "SPEC")]
        day_version: Option<String>,
        /// Use Day dependencies from a local checkout
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Use flags and defaults without prompting (automatic when stdin is not a terminal)
        #[arg(long)]
        no_input: bool,
        /// Put Android Java in src/ (default); use false for platform/android/java/
        #[arg(
            long,
            value_name = "BOOL",
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = "true"
        )]
        java_in_src: Option<bool>,
    },
    /// Create an app
    #[command(after_help = "Docs: https://daybrite.dev/docs/getting-started/")]
    App {
        /// App name (prompted if omitted)
        name: Option<String>,
        /// Targets to support (repeatable or comma-separated; prompted if omitted)
        #[arg(long = "toolkit")]
        toolkits: Vec<String>,
        /// Application ID in reverse-DNS form (default: dev.example.<name>)
        #[arg(long)]
        appid: Option<String>,
        /// Alias for --appid
        #[arg(long)]
        bundleid: Option<String>,
        /// Legacy alias for --appid
        #[arg(long, hide = true)]
        id: Option<String>,
        /// App display name (default: the app name converted to title case)
        #[arg(long)]
        title: Option<String>,
        /// Template directory or Git URL with an optional #REF
        #[arg(long)]
        template: Option<String>,
        /// Skip website configuration files
        #[arg(long = "no-website")]
        no_website: bool,
        /// Initial locales (comma- or space-separated; repeatable; always includes en)
        #[arg(long = "locales")]
        locales: Vec<String>,
        /// Integer or text seed for the app icon (default: application ID)
        #[arg(long = "icon-seed", value_name = "SEED")]
        icon_seed: Option<String>,
        /// Legacy comma-separated target list; prefer --toolkit
        #[arg(long, hide = true)]
        targets: Option<String>,
        /// Use Day dependencies from Git (default)
        #[arg(long)]
        git: bool,
        /// Use Day dependencies from crates.io at the selected version
        #[arg(long)]
        registry: bool,
        /// Day dependency version: release, latest, branch, or commit
        #[arg(long = "day-version", value_name = "SPEC")]
        day_version: Option<String>,
        /// Use Day dependencies from a local checkout
        #[arg(long, hide = true)]
        local: Option<PathBuf>,
        /// Use flags and defaults without prompting (automatic when stdin is not a terminal)
        #[arg(long)]
        no_input: bool,
    },
}

#[derive(clap::Subcommand)]
pub enum AppCmd {
    /// Add targets to Day.toml and create their native projects
    #[command(name = "add-toolkit")]
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/")]
    AddToolkit {
        /// Targets to add, such as android-mdc (repeatable or comma-separated)
        targets: Vec<String>,
        /// Original project template, if custom (directory or Git URL)
        #[arg(long)]
        template: Option<String>,
    },
    /// Move Xcode build settings into DayApp.xcconfig files
    #[command(name = "split-xcconfig")]
    #[command(after_help = "Docs: https://daybrite.dev/docs/project-structure/")]
    SplitXcconfig,
}

#[derive(clap::Subcommand)]
pub enum DevicesCmd {
    /// List available simulators, emulators, and connected devices
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#simulators-emulators-and-devices")]
    List {
        /// Filter by target: ios-uikit, android-mdc, or harmony-arkui
        #[arg(short = 'p', long = "platform", value_name = "TARGET")]
        platform: Option<String>,
    },
    /// Start a simulator or emulator
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#simulators-emulators-and-devices")]
    Boot {
        /// Device target: ios-uikit, android-mdc, or harmony-arkui
        #[arg(short = 'p', long = "platform", value_name = "TARGET")]
        platform: String,
        /// Device ID from day devices list; omit when using --device
        #[arg(value_name = "ID")]
        id: Option<String>,
        /// Device name prefix or wildcard pattern, such as "iPhone * Pro Max"; newest match wins
        #[arg(long, value_name = "NAME", conflicts_with = "id")]
        device: Option<String>,
        /// OS version for --device, such as "iOS 26"; newest matching minor version wins
        #[arg(long, value_name = "VERSION", requires = "device")]
        os: Option<String>,
        /// Wait for the device to finish booting
        #[arg(long)]
        wait: bool,
        /// iOS simulator orientation: portrait or landscape
        #[arg(long, value_name = "ORIENTATION")]
        orientation: Option<String>,
        /// Hide the Android or iOS emulator window; ignored on OpenHarmony
        #[arg(long)]
        headless: bool,
    },
    /// Stop a running simulator or emulator
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#simulators-emulators-and-devices")]
    Shutdown {
        /// Device target: ios-uikit, android-mdc, or harmony-arkui
        #[arg(short = 'p', long = "platform", value_name = "TARGET")]
        platform: String,
        /// iOS simulator name or UDID, or Android adb serial or AVD name
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Create or update an Android virtual device
    #[command(after_help = "Docs: https://daybrite.dev/docs/cli/#simulators-emulators-and-devices")]
    Setup {
        /// Device target (android-mdc only)
        #[arg(short = 'p', long = "platform", value_name = "TARGET")]
        platform: String,
        /// Profile ID from avdmanager list device, such as pixel_7
        #[arg(long, value_name = "PROFILE")]
        device: String,
        /// Android API level, such as 36 or android-36
        #[arg(long, value_name = "LEVEL")]
        os: String,
        /// System image ABI (default: this host's architecture)
        #[arg(long, value_name = "ABI")]
        arch: Option<String>,
        /// System image tag (default: google_apis)
        #[arg(long, value_name = "TAG")]
        tag: Option<String>,
        /// Virtual device name (default: derived from profile and API level)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Emulator orientation: portrait or landscape
        #[arg(long, value_name = "ORIENTATION")]
        orientation: Option<String>,
        /// Display density in DPI; does not change screenshot pixel dimensions
        #[arg(long, value_name = "DPI")]
        density: Option<u32>,
        /// Emulator RAM in MB (default: profile value, with at least 4096 for large displays)
        #[arg(long, value_name = "MB")]
        ram: Option<u32>,
    },
}

#[derive(clap::Subcommand)]
pub enum OhosCmd {
    /// Manage the OpenHarmony emulator
    #[command(after_help = "Docs: https://daybrite.dev/docs/platforms/harmony-arkui/")]
    Emulator {
        #[command(subcommand)]
        cmd: EmulatorCmd,
    },
}

#[derive(clap::Subcommand)]
pub enum EmulatorCmd {
    /// Launch the Oniro OpenHarmony emulator
    #[command(after_help = "Docs: https://daybrite.dev/docs/platforms/harmony-arkui/")]
    Launch {
        /// Run without a window; connect through hdc
        #[arg(long)]
        headless: bool,
    },
}

pub fn run() -> i32 {
    let mut cli = Cli::parse();
    // `--verbose`: make the tool-runner helpers forward every sub-command's raw output (ops.rs).
    // `DAY_VERBOSE` is the environment spelling of the same switch ("1"/"true" = on): an
    // explicit `--verbose` always wins, and the variable covers the invocations a flag cannot
    // reach: nested launches a dayscript runner generates, or a whole CI job's worth of
    // commands turned verbose from one `env:` line.
    if !cli.verbose {
        cli.verbose = matches!(
            std::env::var("DAY_VERBOSE").as_deref(),
            Ok("1") | Ok("true")
        );
    }
    crate::ops::set_verbose(cli.verbose);
    // Kick off the background crates.io update check now, so it runs while the command does. Silent for
    // the build-system plumbing callbacks (Xcode/Gradle) and for machine `--format json` output.
    let update = crate::update::spawn(
        cli.format != OutputFormat::Json
            && !matches!(
                cli.command,
                Cmd::XcodeBackend { .. } | Cmd::GradleBackend { .. }
            ),
    );
    let result = dispatch(cli);
    // Non-blocking: nudge only if the crates.io reply already arrived; never waits for it.
    crate::update::finish(update);
    // The one render point: every command failure prints here, and the kind picks the code.
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            e.exit_code()
        }
    }
}

/// The command dispatch. Each arm yields the command's verdict code (usually 0) or a
/// [`CliError`]; nothing in here prints an `error:` line; [`run`] does that once.
fn dispatch(cli: Cli) -> Result<i32, CliError> {
    match cli.command {
        Cmd::Version => {
            println!("day {}", env!("DAY_VERSION_LONG"));
            Ok(0)
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
        // Project-less, like doctor and new: checkup SCAFFOLDS the projects it checks.
        Cmd::Checkup {
            platforms,
            profile,
            no_pack,
            strict,
            dir,
            keep,
            git,
            registry,
            day_version,
            local,
        } => crate::checkup::run(&crate::checkup::Options {
            platforms,
            profile,
            no_pack,
            strict,
            dir,
            keep,
            git,
            registry,
            day_version,
            local,
            json: cli.format == OutputFormat::Json,
            verbose: cli.verbose,
        }),
        Cmd::Rebuild {
            artifact,
            force_tool,
            keep,
            strict,
            from_dir,
        } => {
            let opts = crate::rebuild::Options {
                force_tools: force_tool,
                keep,
                strict,
                from_dir,
            };
            crate::rebuild::run(&artifact, &opts).map_err(CliError::failure)
        }
        Cmd::Pack {
            platforms,
            profile,
            formats,
            no_sign,
            no_notarize,
            no_wait,
            no_version_in_name,
            artifact_name,
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
                artifact_name,
            };
            let mut outcomes = Vec::new();
            for p in &platforms {
                let target = crate::external::find_target(project, p).map_err(CliError::usage)?;
                if crate::external::is_external(target) {
                    // Packaging is per-OS CLI code (dmg/msix/flatpak/…) that a declared desktop
                    // target has none of. Say so plainly rather than fall into some builtin arm.
                    return Err(CliError::usage(format!(
                        "day pack does not support externally declared targets yet — \
                         {} builds and launches, but packaging is Stage 1 (docs/extending.md)",
                        target.name
                    )));
                }
                outcomes.push(crate::pack::run(project, target, &opts)?);
            }
            if cli.format == OutputFormat::Json {
                print_pack_json(&outcomes);
            }
            Ok(0)
        }),
        Cmd::Sign {
            check,
            notarize_status,
        } => with_project(cli.project.as_deref(), |project| {
            if let Some(id) = &notarize_status {
                return crate::sign::notarize_status(project, id);
            }
            if check {
                return Ok(crate::sign::check(project));
            }
            Err(CliError::usage(
                "day sign needs --check or --notarize-status <id>",
            ))
        }),
        Cmd::App {
            cmd: AppCmd::AddToolkit { targets, template },
        } => with_project(cli.project.as_deref(), |project| {
            crate::new::add_toolkit(project, &targets, template.as_deref()).map(|()| 0)
        }),
        Cmd::App {
            cmd: AppCmd::SplitXcconfig,
        } => with_project(cli.project.as_deref(), |project| {
            for platform in ["ios", "macos"] {
                crate::xcconfig::ensure_split(project, platform).map_err(CliError::failure)?;
            }
            Ok(0)
        }),
        Cmd::Metadata { json, schema } => {
            if schema {
                // Static: the schema needs no project; usable before one exists.
                println!("{}", include_str!("../resources/day-toml.schema.json"));
                return Ok(0);
            }
            with_project(cli.project.as_deref(), |project| {
                crate::metadata::run(project, json).map(|()| 0)
            })
        }
        Cmd::Lint {
            strict,
            allow,
            json,
            fix,
        } => with_project(cli.project.as_deref(), |project| {
            let json = json || cli.format == OutputFormat::Json;
            Ok(crate::lint::run(project, strict, &allow, json, fix))
        }),
        Cmd::Patch { local, git, check } => {
            // A piece or part crate is a cargo package with no Day.toml, and it depends on day
            // from git exactly like an app, so `day patch` takes any cargo package root, and
            // only falls back to the Day-project search when the directory is not one.
            let root = match meta::find_project(cli.project.as_deref()) {
                Ok(project) => project.root,
                Err(e) => {
                    let dir = match cli.project.clone() {
                        Some(p) => p,
                        None => std::env::current_dir()
                            .map_err(|e| CliError::failure(format!("current directory: {e}")))?,
                    };
                    if !dir.join("Cargo.toml").is_file() {
                        return Err(CliError::usage(e));
                    }
                    dir.canonicalize()
                        .map_err(|e| CliError::failure(format!("{}: {e}", dir.display())))?
                }
            };
            crate::patch::run(&root, &local, git.as_deref(), check).map(|()| 0)
        }
        Cmd::Store { cmd } => with_project(cli.project.as_deref(), |project| {
            crate::store::run(project, &cmd).map(|()| 0)
        }),
        // Project-less: the driver is the CLI's own resource, usable before any checkout.
        Cmd::Web { cmd } => match cmd {
            WebCmd::Driver => match crate::web::materialize_driver() {
                Ok(path) => {
                    println!("{}", path.display());
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    Ok(1)
                }
            },
        },
        Cmd::Screenshot { cmd } => with_project(cli.project.as_deref(), |project| match cmd {
            ScreenshotCmd::Index {
                screenshot_paths,
                out,
            } => {
                let opts = crate::screenshot::IndexOptions {
                    screenshot_paths,
                    out,
                };
                crate::screenshot::index(project, &opts)
                    .map(|_| 0)
                    .map_err(CliError::failure)
            }
        }),
        Cmd::Localize { cmd } => with_project(cli.project.as_deref(), |project| {
            crate::localize::run(project, &cmd).map(|()| 0)
        }),
        Cmd::Clean { dry_run } => with_project(cli.project.as_deref(), |project| {
            crate::clean::run(project, dry_run)
                .map(|_| 0)
                .map_err(CliError::failure)
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
                return Err(CliError::usage(
                    "nothing to stop (no -p targets and no recorded sessions)",
                ));
            }
            for name in &names {
                let target =
                    crate::external::find_target(project, name).map_err(CliError::usage)?;
                crate::script::terminate(project, target);
                crate::sessions::remove(&project.root, name);
                ops::status("Stopped", name);
            }
            Ok(0)
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
                return Err(CliError::usage(
                    "no running sessions — `day launch -p <target>` first",
                ));
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
                // the unfiltered behavior rather than inventing a device to prefer.
                ios_device: None,
                ios_simulator: None,
                android_device: None,
                ohos_device: None,
            };
            for (ti, name) in names.iter().enumerate() {
                let target =
                    crate::external::find_target(project, name).map_err(CliError::usage)?;
                crate::script::terminate(project, target);
                let outcome = ops::build(project, target, profile).map_err(CliError::build)?;
                let mut spec = spec.clone();
                let port = crate::script::pick_port(ti);
                let token = crate::script::make_token();
                spec.envs.push(("DAYSCRIPT_PORT".into(), port.to_string()));
                spec.envs.push(("DAYSCRIPT_TOKEN".into(), token.clone()));
                ops::launch(project, target, &outcome, &spec).map_err(CliError::failure)?;
                crate::sessions::record(
                    &project.root,
                    crate::sessions::Session {
                        target: name.clone(),
                        app_id: project.manifest.resolve(name).id,
                        profile: profile.to_string(),
                        engine_port: port,
                        engine_token: token,
                        started_at: crate::sessions::now_millis(),
                    },
                );
                ops::status("Relaunched", name);
            }
            Ok(0)
        }),
        Cmd::Drive {
            platform,
            steps_json,
        } => with_project(cli.project.as_deref(), |project| {
            let target =
                crate::external::find_target(project, &platform).map_err(CliError::usage)?;
            crate::drive::run(project, target, &steps_json)
        }),
        Cmd::McpServer {} => with_project(cli.project.as_deref(), |project| {
            Ok(crate::mcp::run(project))
        }),
        // No project needed: what is plugged into this machine has nothing to do with which app
        // is open, and the editor asks before a project is chosen.
        Cmd::Devices {
            cmd: DevicesCmd::List { platform },
        } => crate::devices::list(platform.as_deref(), cli.format == OutputFormat::Json),
        Cmd::Devices {
            cmd:
                DevicesCmd::Boot {
                    platform,
                    id,
                    device,
                    os,
                    wait,
                    orientation,
                    headless,
                },
        } => crate::devices::boot(
            platform.as_str(),
            &crate::devices::BootSpec {
                id: id.as_deref(),
                device: device.as_deref(),
                os: os.as_deref(),
                wait,
                orientation: orientation.as_deref(),
                headless,
            },
        ),
        Cmd::Devices {
            cmd: DevicesCmd::Shutdown { platform, id },
        } => crate::devices::shutdown(platform.as_str(), id.as_str()),
        Cmd::Devices {
            cmd:
                DevicesCmd::Setup {
                    platform,
                    device,
                    os,
                    arch,
                    tag,
                    name,
                    orientation,
                    density,
                    ram,
                },
        } => crate::devices::setup(
            platform.as_str(),
            &crate::devices::SetupSpec {
                name: name.as_deref(),
                device: device.as_str(),
                os: os.as_str(),
                arch: arch.as_deref(),
                tag: tag.as_deref(),
                orientation: orientation.as_deref(),
                ram,
                density,
            },
        ),
        Cmd::Ohos {
            cmd:
                OhosCmd::Emulator {
                    cmd: EmulatorCmd::Launch { headless },
                },
        } => crate::ohos::emulator_launch(headless)
            .map(|()| 0)
            .map_err(CliError::script),
        Cmd::XcodeBackend { action } => match action.as_str() {
            "build" => crate::mobile::xcode_backend_build().map(|()| 0),
            "stage-resources" => crate::mobile::xcode_backend_stage_resources().map(|()| 0),
            "stage-strings" => crate::mobile::xcode_backend_stage_strings().map(|()| 0),
            other => Err(CliError::usage(format!(
                "day xcode-backend: unknown action {other:?}"
            ))),
        },
        Cmd::GradleBackend { .. } => crate::mobile::gradle_backend_build().map(|()| 0),
        // clap cannot express `conflicts_with` against a SUBCOMMAND, so the combination is
        // rejected here rather than silently ignoring one half of what was asked for.
        Cmd::New { what, describe } if describe => {
            if what.is_some() {
                Err(CliError::usage(
                    "`day new --describe` describes every kind — drop the subcommand.",
                ))
            } else {
                println!("{}", crate::new::describe());
                Ok(0)
            }
        }
        Cmd::New { what, .. } => match what {
            None => crate::new::interactive().map(|()| 0),
            Some(NewKind::Piece {
                name,
                toolkits,
                composite,
                id,
                git,
                registry,
                day_version,
                local,
                no_input,
                no_demo,
                java_in_src,
            }) => crate::new::piece(
                name.as_deref(),
                toolkits.as_deref(),
                composite,
                id.as_deref(),
                local.as_deref(),
                git,
                registry,
                day_version.as_deref(),
                no_input,
                no_demo,
                java_in_src,
            )
            .map(|()| 0),
            Some(NewKind::Part {
                name,
                platforms,
                id,
                git,
                registry,
                day_version,
                local,
                no_input,
                java_in_src,
            }) => crate::new::part(
                name.as_deref(),
                platforms.as_deref(),
                id.as_deref(),
                local.as_deref(),
                git,
                registry,
                day_version.as_deref(),
                no_input,
                java_in_src,
            )
            .map(|()| 0),
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
                day_version,
                local,
                no_input,
                no_website,
                locales,
                icon_seed,
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
                day_version.as_deref(),
                no_input,
                no_website,
                &locales,
                icon_seed.as_deref(),
            )
            .map(|()| 0),
        },
        Cmd::Icon {
            master,
            check,
            platforms,
            generate,
            seed,
            overwrite,
            out,
        } => {
            let seed_value = generate.then(|| crate::icon::resolve_seed(seed.as_deref()));
            // Preview mode stands alone: an SVG+PNG pair at the given path, no project.
            if let (true, Some(path)) = (generate, out.as_ref()) {
                let seed_value = seed_value.unwrap_or_default();
                crate::icon::generate_preview(path, seed_value).map_err(CliError::build)?;
                crate::ops::status(
                    "Generated",
                    &format!("{} (seed {seed_value})", path.display()),
                );
                return Ok(0);
            }
            with_project(cli.project.as_deref(), |project| {
                if let Some(seed_value) = seed_value {
                    let dest = crate::icon::generate_master(project, seed_value, overwrite)
                        .map_err(CliError::build)?;
                    crate::ops::status(
                        "Generated",
                        &format!("{} (seed {seed_value})", dest.display()),
                    );
                }
                let opts = crate::icon::IconOptions {
                    master: master.clone(),
                    check,
                    platforms: platforms.clone(),
                };
                match crate::icon::run(project, &opts) {
                    Ok(n) => {
                        if cli.format == OutputFormat::Json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "event": "result", "command": "icon", "ok": true, "outputs": n,
                                })
                            );
                        }
                        Ok(0)
                    }
                    Err(crate::icon::IconError::Drift(lines)) => {
                        for l in &lines {
                            crate::ops::status("Drift", l);
                        }
                        if cli.format == OutputFormat::Json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "event": "result", "command": "icon", "ok": false, "drift": lines,
                                })
                            );
                        }
                        Err(CliError::drift(
                            "host files are missing or stale — run `day prepare`",
                        ))
                    }
                    Err(crate::icon::IconError::Other(e)) => Err(CliError::build(e)),
                }
            })
        }
        Cmd::Prepare {
            platforms,
            check,
            migrate,
        } => with_project(cli.project.as_deref(), |project| {
            if migrate {
                crate::icon::migrate(project).map_err(CliError::build)?;
                return Ok(0);
            }
            let opts = crate::icon::IconOptions {
                master: None,
                check,
                platforms: platforms.clone(),
            };
            match crate::icon::run(project, &opts) {
                Ok(n) => {
                    // The HarmonyOS host's ArkTS is staged from the day-arkui crate, not
                    // checked in (docs/harmonyos.md); put it in place too, so the project
                    // DevEco Studio opens on a fresh clone has its abilities and pages. A
                    // `--check` writes nothing.
                    let harmony = platforms.is_empty()
                        && project
                            .manifest
                            .app
                            .targets
                            .iter()
                            .any(|t| t == "harmony-arkui")
                        || platforms.iter().any(|p| p == "harmony-arkui");
                    if harmony && !check {
                        crate::ohos::stage_host(project).map_err(CliError::build)?;
                    }
                    // Day's Gradle plugins are staged from the day-android crate the same way, so
                    // Android Studio can sync a fresh clone before its first `day build`.
                    let android = platforms.is_empty()
                        && project
                            .manifest
                            .app
                            .targets
                            .iter()
                            .any(|t| t == "android-mdc")
                        || platforms.iter().any(|p| p == "android-mdc");
                    if android && !check {
                        crate::pieces::stage_android_gradle_plugin(project)
                            .map_err(CliError::build)?;
                    }
                    if cli.format == OutputFormat::Json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "event": "result", "command": "prepare", "ok": true, "outputs": n,
                            })
                        );
                    }
                    Ok(0)
                }
                Err(crate::icon::IconError::Drift(lines)) => {
                    for l in &lines {
                        crate::ops::status("Stale", l);
                    }
                    if cli.format == OutputFormat::Json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "event": "result", "command": "prepare", "ok": false, "stale": lines,
                            })
                        );
                    }
                    Err(CliError::drift(
                        "host files are missing or stale — run `day prepare`",
                    ))
                }
                Err(crate::icon::IconError::Other(e)) => Err(CliError::build(e)),
            }
        }),
        Cmd::Open { platform } => with_project(cli.project.as_deref(), |project| {
            let target =
                crate::external::find_target(project, &platform).map_err(CliError::usage)?;
            crate::icon::ensure(project, &[target.name]).map_err(CliError::build)?;
            if target.name == "harmony-arkui" {
                crate::ohos::stage_host(project).map_err(CliError::build)?;
            }
            // Android Studio syncs through Day's Gradle plugin, staged from the day-android crate.
            if target.name == "android-mdc" {
                crate::pieces::stage_android_gradle_plugin(project).map_err(CliError::build)?;
            }
            crate::ops::open_native(project, target).map(|()| 0)
        }),
        Cmd::Build {
            platforms,
            profile,
            day_src,
        } => with_project(cli.project.as_deref(), |project| {
            use_day_src(day_src.as_deref(), project)?;
            // Cargo records the patched sources in Cargo.lock; the guard puts it back when the
            // build phase ends, so a flag that promises to change nothing leaves nothing changed.
            let _lock = crate::patch::LockGuard::new(project);
            // One copy of every day crate, before anything compiles (crate::patch::verify_graph).
            crate::patch::verify_graph(project).map_err(CliError::usage)?;
            let mut results = Vec::new();
            for p in &platforms {
                let target = crate::external::find_target(project, p).map_err(CliError::usage)?;
                let o = ops::build(project, target, profile).map_err(CliError::build)?;
                ops::status(
                    "Built",
                    &format!(
                        "{} → {} ({:.1}s)",
                        o.target,
                        o.artifact.display(),
                        o.seconds
                    ),
                );
                results.push((target, o));
            }
            if cli.format == OutputFormat::Json {
                print_result_json("build", project, &results);
            }
            Ok(0)
        }),
        Cmd::Launch {
            platforms,
            git,
            dir,
            profile,
            locale,
            envs,
            ios_device,
            ios_simulator,
            android_device,
            ohos_device,
            detach,
            keep_alive,
            record,
            scripts,
            variant,
            device,
            skip_build,
            locales,
            themes,
            capture_size,
            day_src,
        } => {
            // `--git` only decides where the launch starts from. It clones (or updates) the
            // repository and hands back the Day project directory inside it, so `find_project`
            // and the whole launch body below see an ordinary checkout (crate::git).
            let start = match &git {
                Some(arg) => {
                    let spec = crate::git::parse_spec(arg).map_err(CliError::usage)?;
                    Some(crate::git::prepare(
                        &spec,
                        dir.as_deref(),
                        cli.project.as_deref(),
                    )?)
                }
                None => cli.project.clone(),
            };
            with_project(start.as_deref(), |project| {
                use_day_src(day_src.as_deref(), project)?;
                crate::patch::verify_graph(project).map_err(CliError::usage)?;
                // No `-p`: run what this machine natively is. Announced rather than assumed: the
                // chosen target decides which toolkit gets built, so a silent pick would be a
                // surprising several-minute build of something the caller did not name.
                let platforms = if platforms.is_empty() {
                    let default = crate::targets::host_default();
                    ops::status("Defaulting", &format!("{default} (no --platform given)"));
                    vec![default.to_string()]
                } else {
                    platforms.clone()
                };
                // Under `--git` a relative `--script` that isn't in the invoking directory is
                // looked up in the checkout, so a repository's own walkthrough runs by the name
                // it carries there (crate::git::script_path).
                let scripts: Vec<PathBuf> = match git {
                    Some(_) => scripts
                        .iter()
                        .map(|p| crate::git::script_path(p, &project.root))
                        .collect(),
                    None => scripts.clone(),
                };
                let script_mode = !scripts.is_empty();
                let mut spec = ops::LaunchSpec {
                    locale: locale.clone(),
                    ios_device: ios_device.clone(),
                    ios_simulator: ios_simulator.clone(),
                    android_device: android_device.clone(),
                    ohos_device: ohos_device.clone(),
                    envs: envs
                        .iter()
                        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
                        .collect(),
                    // Attachment follows `--detach` alone, not whether a script runs: a scripted
                    // launch streams the app's console output the same as a plain launch. (A
                    // `--keep-alive` scripted run additionally keeps `day` in the foreground after the
                    // script so that output stays visible while the app lives; see below.)
                    attached: !detach,
                };
                // Ctrl-C during an attached run must take the launched apps and their log
                // watchers (simctl / adb logcat) down too, not leave them orphaned.
                if spec.attached {
                    crate::signals::install();
                }
                // The debug window-title tag (docs/windows.md): which build, which toolkit, and,
                // when a script is driving, which script. The app reads these off the environment
                // and only shows them in a debug build.
                //
                // Under `--day-src` the version carries the day-src too (`0.1.0+main-2d77edbf`),
                // which is what the flag exists for: two builds of the same app, running at
                // once, are otherwise two identical title bars. It rides the version rather than
                // a new variable because the framework version being compared may predate any
                // variable added today, and every `day` already reads this one.
                spec.envs.push((
                    "DAY_APP_VERSION".into(),
                    match crate::patch::day_src_tag() {
                        Some(tag) => format!("{}+{tag}", project.manifest.app.version),
                        None => project.manifest.app.version.clone(),
                    },
                ));
                // `--record` (§14.6): the app's `day_script::init` reads `DAY_RECORD` and starts a
                // headless recorder that flushes a replayable dayscript to the path for its lifetime.
                // Absolutize against the invoking CWD: the app process runs from the project root,
                // so a relative path would otherwise land somewhere the user did not mean.
                if let Some(path) = &record {
                    let abs = if path.is_absolute() {
                        path.clone()
                    } else {
                        std::env::current_dir()
                            .map(|d| d.join(path))
                            .unwrap_or_else(|_| path.clone())
                    };
                    spec.envs
                        .push(("DAY_RECORD".into(), abs.to_string_lossy().into_owned()));
                }
                if script_mode {
                    // A scripted run is unattended, so a panic's backtrace has to be in the log the
                    // first time; nobody is there to re-run it with RUST_BACKTRACE set. The app's
                    // stderr is already streamed, so this is what turns "thread panicked at …" into
                    // a stack. An explicit `--env RUST_BACKTRACE=…` wins (it is in `envs` already).
                    if !envs.iter().any(|kv| kv.starts_with("RUST_BACKTRACE=")) {
                        spec.envs.push(("RUST_BACKTRACE".into(), "1".into()));
                    }
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
                // with the variant names both CI shapes already produce: the day-CI/gallery
                // `<theme>`/`<theme>-<locale>` convention and the app-CI `<locale>` convention are
                // preserved byte-for-byte so existing artifact layouts survive the move from YAML
                // loops into the CLI. No flags = one run with the plain --locale/--variant.
                let matrix = capture_matrix(&themes, &locales, &locale, &variant, &envs)
                    .map_err(CliError::usage)?;
                // The capture size (Day.toml `[screenshots]`): a scripted run's desktop-class
                // captures are a stated pixel size, not whatever window the app happens to
                // open. Resolved once; applied per target below, since a phone ignores it.
                let capture_size = if script_mode {
                    crate::screenshot::capture_size(project, capture_size.as_deref())
                        .map_err(CliError::usage)?
                } else {
                    None
                };
                let mut handles = Vec::new();
                let mut launched: Vec<(&'static crate::targets::Target, std::time::SystemTime)> =
                    Vec::new();
                let mut script_failures = 0usize;
                // Engine losses across the whole run: the cap that keeps a dead app from
                // relaunching once per variant until the job's own timeout kills it.
                let mut losses = 0usize;
                // The variables the capture size added for the previous target.
                let mut capture_keys: Vec<String> = Vec::new();
                for (ti, p) in platforms.iter().enumerate() {
                    let port = crate::script::pick_port(ti);
                    // The dayscript engine rides every launch (loopback, token-gated): scripted runs
                    // drive it immediately, and interactive launches stay drivable later via the
                    // session registry (`day drive` / `day relaunch` / agents; docs/agent.md).
                    spec.envs
                        .retain(|(k, _)| k != "DAYSCRIPT_PORT" && k != "DAYSCRIPT_TOKEN");
                    spec.envs.push(("DAYSCRIPT_PORT".into(), port.to_string()));
                    spec.envs.push(("DAYSCRIPT_TOKEN".into(), token.clone()));
                    let target =
                        crate::external::find_target(project, p).map_err(CliError::usage)?;
                    // The capture size rides to the app as DAY_WINDOW + DAY_CAPTURE_SCALE, for
                    // the targets it applies to. Dropped and re-added per target, like the port.
                    spec.envs.retain(|(k, _)| !capture_keys.contains(k));
                    capture_keys.clear();
                    if let Some(size) =
                        capture_size.filter(|_| crate::screenshot::is_desktop_class(target))
                    {
                        let given: Vec<(String, String)> = envs
                            .iter()
                            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
                            .collect();
                        let (w, h) = size.points();
                        ops::status(
                            "Capture",
                            &format!(
                                "{}x{} px ({w}x{h} points at {}x) for {}",
                                size.width, size.height, size.scale, target.name
                            ),
                        );
                        for (k, v) in crate::screenshot::capture_envs(size, &given) {
                            capture_keys.push(k.clone());
                            spec.envs.push((k, v));
                        }
                        // AppKit captures are the window server's pixels, at the scale of the
                        // display the window is on: where no attached display has the capture's
                        // scale (a CI runner's is 1x), a virtual one does, and the app opens
                        // its window there.
                        if target.toolkit == "appkit"
                            && !given.iter().any(|(k, _)| k == "DAY_WINDOW_SCREEN")
                            && let Some(id) = crate::screenshot::capture_display(size)
                        {
                            capture_keys.push("DAY_WINDOW_SCREEN".into());
                            spec.envs.push(("DAY_WINDOW_SCREEN".into(), id));
                        }
                    }
                    // The lock guard is scoped to the build, not to the run: the app may stay up
                    // for a long time afterwards, and the project's Cargo.lock should be correct
                    // again the moment the compiler is done with it.
                    let built = {
                        let _lock = crate::patch::LockGuard::new(project);
                        if skip_build {
                            ops::reuse_build(project, target, profile)
                        } else if spec.wants_ios_device() {
                            ops::build_for_device(project, target, profile)
                        } else {
                            ops::build(project, target, profile)
                        }
                    };
                    let outcome = built.map_err(CliError::build)?;
                    for (ri, capture) in matrix.iter().enumerate() {
                        // Per-run spec: the matrix's locale and theme ride the same channels the
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
                                // The previous run's app still holds the engine port; stop it the
                                // way `day stop` does before the next instance binds.
                                crate::script::terminate(project, target);
                            }
                            // Stamped before the launch so the post-mortem can tell this run's
                            // crash report from one an earlier run left in the same directory.
                            let launched_at = std::time::SystemTime::now();
                            let h = ops::launch(project, target, &outcome, &run_spec)
                                .map_err(CliError::failure)?;
                            // Kept beside the handle so a crash can be diagnosed when it is
                            // joined: a plain `day launch` has no script to lose its engine,
                            // so the join is the only place the app's death is observed.
                            launched.push((target, launched_at));
                            crate::sessions::record(
                                &project.root,
                                crate::sessions::Session {
                                    target: p.clone(),
                                    app_id: project.manifest.resolve(p).id,
                                    profile: profile.to_string(),
                                    engine_port: port,
                                    engine_token: token.clone(),
                                    started_at: crate::sessions::now_millis(),
                                },
                            );
                            handles.push(h);
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
                                device.as_deref(),
                                keep_alive,
                                spec.attached,
                            ) {
                                // A single retryable failure and nothing else: the shape a race
                                // leaves behind (an element not realized yet, an assert that lost
                                // to a transition or a page load) rather than a broken app. Re-run
                                // the variant once, the same budget the app-death arm below has
                                // always had, extended to the other way a flake presents. Two
                                // failures, or one the engine called final, is a verdict: report
                                // it. The retry is announced, so a green run that needed one is
                                // still visible as a flake in the log rather than passing silently.
                                Ok(run)
                                    if run.steps_failed == 1
                                        && run.retryable_failed == 1
                                        && attempt == 0 =>
                                {
                                    eprintln!(
                                        "warning: one retryable step failed — retrying the script \
                                     once before calling it a failure"
                                    );
                                    if std::env::var_os("GITHUB_ACTIONS").is_some() {
                                        println!(
                                            "::warning::one retryable step failed — retrying the \
                                         script once before calling it a failure"
                                        );
                                    }
                                    attempt += 1;
                                }
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
                                // zero failed steps. Retry the (idempotent) run once, the logic
                                // both CI workflows used to grep logs for, now typed. A loss after
                                // a failed step is a failing run that then died: report it.
                                Err(crate::script::ScriptError::EngineLost {
                                    steps_failed: 0,
                                    ..
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
                                    // Say why it died before retrying: the retry usually passes, and
                                    // then the only record of the flake is this one line.
                                    crate::diagnose::after_app_death(project, target, launched_at);
                                    attempt += 1;
                                }
                                // An engine loss that survived the retry policy: count it and
                                // move to the next matrix run instead of abandoning the rest;
                                // the CI loops this replaced continued per variant (OHOS relies
                                // on it under TCG), and the final exit code still reports failure.
                                Err(crate::script::ScriptError::EngineLost {
                                    steps_failed,
                                    ref detail,
                                }) => {
                                    eprintln!(
                                        "error: engine connection lost ({detail}) — abandoning this variant"
                                    );
                                    let crashed = crate::diagnose::after_app_death(
                                        project,
                                        target,
                                        launched_at,
                                    );
                                    // A device that no longer answers takes every remaining variant
                                    // with it: the next launch would install onto it, and `adb` and
                                    // `hdc` wait for a wedged device rather than failing. That wait
                                    // is what turned this arm's diagnosis into a six-hour job.
                                    let device_lost = !crate::script::device_alive(target);
                                    script_failures += steps_failed.max(1);
                                    losses += 1;
                                    // A crash ends the run. Every remaining variant would relaunch
                                    // a build that just died and fail the same way, minutes at a
                                    // time, which is how a crashed walkthrough used to run out the
                                    // job's timeout instead of reporting the crash it had already
                                    // found. A loss with no crash artifact stays per-variant (a
                                    // slow emulator drops the connection and the next variant often
                                    // passes), but not forever: two in a row is a pattern, not a
                                    // hiccup.
                                    if crashed || device_lost || losses >= 2 {
                                        let why = if crashed {
                                            "the app crashed"
                                        } else if device_lost {
                                            "the device stopped answering"
                                        } else {
                                            "the engine was lost twice"
                                        };
                                        crate::signals::kill_all();
                                        return Err(CliError::script(format!(
                                            "{why} — abandoning the remaining variants \
                                         ({} of {} run)",
                                            ri + 1,
                                            matrix.len()
                                        )));
                                    }
                                    break;
                                }
                                // Anything else is a runner/config error (bad script, bad flags):
                                // abort outright, as before.
                                Err(e) => return Err(CliError::script(e.to_string())),
                            }
                        }
                        // Between matrix runs the app must exit so the next launch re-binds the
                        // engine port (each variant is a fresh process, as the CI loops had it).
                        if script_mode && ri + 1 < matrix.len() {
                            crate::script::terminate(project, target);
                        }
                    }
                }
                // A scripted run returns once its script(s) finish, except an attached
                // `--keep-alive` run, which stays in the foreground streaming the app's console
                // output until the app exits or the run is stopped (so output is visible during and
                // after the script, exactly like a plain attached launch). Detached scripted runs
                // (agents) and non-keep-alive scripted runs (CI) return here without blocking on
                // device log pumps that never EOF; attached runs already streamed logs live while
                // the script drove the app.
                //
                // Reap the tracked children first (`--keep-alive` is what asks for the app to stay
                // running). Returning without this leaves the log pumps (`adb logcat`, `simctl
                // launch --console`) orphaned holding the inherited stdout/stderr; in CI the step's
                // pipe then never reaches EOF and the job hangs after the final "steps passed" line.
                if script_mode && !(spec.attached && keep_alive) {
                    crate::signals::kill_all();
                    return Ok(if script_failures > 0 {
                        ErrKind::Script.exit_code()
                    } else {
                        0
                    });
                }
                if spec.attached {
                    let mut code = 0;
                    for (i, h) in handles.into_iter().enumerate() {
                        let one = h.join().unwrap_or(1);
                        // A fatal signal is a crash, not a quit: say what happened while the crash
                        // artifacts are still fresh. Closing the window exits 0 and prints nothing.
                        if ops::died_on_signal(one)
                            && let Some((target, at)) = launched.get(i)
                        {
                            eprintln!("error: {} died on a fatal signal (exit {one})", target.name);
                            crate::diagnose::after_app_death(project, target, *at);
                        }
                        code = code.max(one);
                    }
                    // A target that exited on its own leaves its siblings' log watchers (and
                    // any child that outlives its stream) running; reap them before we go.
                    crate::signals::kill_all();
                    Ok(if script_mode && script_failures > 0 {
                        ErrKind::Script.exit_code()
                    } else {
                        code
                    })
                } else {
                    Ok(0)
                }
            })
        }
    }
}

fn with_project(
    start: Option<&std::path::Path>,
    f: impl FnOnce(&meta::Project) -> Result<i32, CliError>,
) -> Result<i32, CliError> {
    let p = meta::find_project(start).map_err(CliError::usage)?;
    f(&p)
}

/// Resolve `--day-src` and make it this run's framework, if it was given.
///
/// Everything downstream (the cargo invocations and the build paths) reads it back from
/// [`crate::patch`] rather than being handed it, the way `--verbose` works, so no builder's
/// signature changes for a flag that does not change what it does.
fn use_day_src(day_src: Option<&str>, project: &meta::Project) -> Result<(), CliError> {
    let Some(arg) = day_src else {
        return Ok(());
    };
    let src = crate::patch::resolve_day_src(arg, project)?;
    crate::patch::activate(&src, project)
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

/// The `build` result event.
///
/// A desktop target also carries a `launch` object (the exact program, working directory, and
/// environment `day launch` would spawn it with), so a caller that starts the binary itself gets
/// the same app Day would have started. That is what the VS Code extension hands to lldb when it
/// delegates a debug session; without the environment the app comes up with no resources,
/// vectors, or identity, and the difference is invisible until something is missing on screen.
/// Device and browser runtimes have no local program to name, so they carry no `launch`.
fn print_result_json(
    command: &str,
    project: &meta::Project,
    results: &[(&'static crate::targets::Target, ops::BuildOutcome)],
) {
    let targets: Vec<serde_json::Value> = results
        .iter()
        .map(|(target, o)| {
            let mut entry = serde_json::json!({
                "target": o.target, "ok": true, "code": 0,
                "artifacts": [{"path": o.artifact}], "seconds": o.seconds,
            });
            if target.kind == crate::targets::TargetKind::Desktop {
                // Best-effort: the build itself succeeded, and the only way the plan fails is a
                // `.app` with nothing under Contents/MacOS, which the launch path diagnoses far
                // better than a truncated result event could.
                let spec = ops::LaunchSpec {
                    locale: None,
                    envs: Vec::new(),
                    attached: true,
                    ios_device: None,
                    ios_simulator: None,
                    android_device: None,
                    ohos_device: None,
                };
                if let Ok(plan) = ops::desktop_launch_plan(project, target, o, &spec) {
                    let env: serde_json::Map<String, serde_json::Value> = plan
                        .env
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                serde_json::Value::String(v.to_string_lossy().into_owned()),
                            )
                        })
                        .collect();
                    entry["launch"] = serde_json::json!({
                        "program": plan.program,
                        "args": plan.args,
                        "cwd": plan.cwd,
                        "env": env,
                        "wrapper": plan.wrapper,
                    });
                }
            }
            entry
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

/// Accept both `--themes light,dark` and `--themes "light dark"`: the CI matrix variables are
/// space-separated and pass through as one argument. Shared with the other list-valued flags
/// (`day new app --locales`, `day localize add/remove`) so every list splits the same way.
pub(crate) fn split_list(raw: &[String]) -> Vec<String> {
    raw.iter()
        .flat_map(|s| s.split([',', ' ']))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Expand `--themes` × `--locales` into runs, preserving both existing artifact conventions
/// byte-for-byte (they predate this flag and live on in the gallery config and the app CIs):
///
/// - locales alone → one run per locale, `--locale <l>` passed for every locale including the
///   default, variant `<l>` (what the app CIs' shell loops produced).
/// - themes × locales → variant `<theme>` when the locale is `en` (run in the default locale,
///   no `--locale` flag), `<theme>-<locale>` otherwise, the day-CI / gallery convention
///   (website/gallery.config.mjs lists exactly these ids).
///
/// The `en` asymmetry between the modes is kept for compatibility: renaming artifact
/// directories would break every consumer that globs them.
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
mod error_tests {
    use super::*;

    /// The exit-code contract, one assertion per kind. These numbers are frozen (CI
    /// walkthroughs assert them), so a change here is a breaking change, not a refactor.
    #[test]
    fn every_kind_maps_to_its_frozen_code() {
        assert_eq!(ErrKind::Failure.exit_code(), 1);
        assert_eq!(ErrKind::Usage.exit_code(), 2);
        assert_eq!(ErrKind::Env.exit_code(), 3);
        assert_eq!(ErrKind::Build.exit_code(), 4);
        assert_eq!(ErrKind::Script.exit_code(), 5);
        assert_eq!(ErrKind::Drift.exit_code(), 5);
        assert_eq!(ErrKind::Sign.exit_code(), 6);
        assert_eq!(ErrKind::Lint.exit_code(), 10);
    }

    /// A bare String converts to the generic failure (exit 1) with its text intact (the
    /// gradual-conversion path for `Result<T, String>` helpers), and the constructors pick
    /// the specific kinds.
    #[test]
    fn strings_convert_and_constructors_pick_kinds() {
        let e = CliError::from("cargo build failed".to_string());
        assert_eq!(e.exit_code(), 1);
        assert_eq!(e.to_string(), "cargo build failed");
        assert_eq!(CliError::failure("x").exit_code(), 1);
        assert_eq!(CliError::usage("x").exit_code(), 2);
        assert_eq!(CliError::env("x").exit_code(), 3);
        assert_eq!(CliError::build("x").exit_code(), 4);
        assert_eq!(CliError::script("x").exit_code(), 5);
        assert_eq!(CliError::drift("x").exit_code(), 5);
        assert_eq!(CliError::sign("x").exit_code(), 6);
    }

    /// PackError keeps its documented mapping (§16.3) through the conversion.
    #[test]
    fn pack_errors_keep_their_codes() {
        let sign = CliError::from(crate::pack::PackError::Sign("s".into()));
        assert_eq!(sign.exit_code(), 6);
        assert_eq!(sign.to_string(), "s");
        let other = CliError::from(crate::pack::PackError::Other("o".into()));
        assert_eq!(other.exit_code(), 4);
        assert_eq!(other.to_string(), "o");
    }

    /// The two ValueEnums accept exactly today's spellings, and only those.
    #[test]
    fn profile_and_format_parse_strictly() {
        assert_eq!(Profile::from_str("debug", false).unwrap(), Profile::Debug);
        assert_eq!(
            Profile::from_str("release", false).unwrap(),
            Profile::Release
        );
        assert_eq!(Profile::Release.as_str(), "release");
        assert!(Profile::from_str("relaese", false).is_err());
        assert_eq!(
            OutputFormat::from_str("plain", false).unwrap(),
            OutputFormat::Plain
        );
        assert_eq!(
            OutputFormat::from_str("json", false).unwrap(),
            OutputFormat::Json
        );
        assert!(OutputFormat::from_str("yaml", false).is_err());
    }

    /// A typo'd `--profile` is a clap parse error. Before the ValueEnum it reached the build
    /// and string-compared its way into the debug branch.
    #[test]
    fn a_profile_typo_fails_argument_parsing() {
        assert!(
            Cli::try_parse_from(["day", "build", "-p", "macos-appkit", "--profile", "relaese"])
                .is_err()
        );
        let cli =
            Cli::try_parse_from(["day", "build", "-p", "macos-appkit", "--profile", "release"])
                .expect("release parses");
        match cli.command {
            Cmd::Build { profile, .. } => assert_eq!(profile, Profile::Release),
            _ => unreachable!("parsed a build command"),
        }
    }

    /// `--git` exists so that neither `-p` nor a project on disk is needed. `--dir`
    /// without it is meaningless, and clap rejects the pair rather than silently ignoring one.
    #[test]
    fn git_parses_without_a_platform_and_dir_requires_it() {
        let cli = Cli::try_parse_from([
            "day",
            "launch",
            "--git",
            "https://github.com/daybrite/Day-Rise.git@main",
        ])
        .expect("--git alone parses");
        match cli.command {
            Cmd::Launch {
                git,
                platforms,
                dir,
                ..
            } => {
                assert_eq!(
                    git.as_deref(),
                    Some("https://github.com/daybrite/Day-Rise.git@main")
                );
                assert!(platforms.is_empty(), "no -p is the host default");
                assert!(dir.is_none());
            }
            _ => unreachable!("parsed a launch command"),
        }
        assert!(Cli::try_parse_from(["day", "launch", "--dir", "/tmp/x"]).is_err());
    }

    /// `--day-src` is on both halves of the inner loop, since comparing two framework versions
    /// means building with each and running each.
    #[test]
    fn day_src_parses_on_build_and_launch() {
        let url = "https://github.com/daybrite/day.git@experimental-nav";
        let cli = Cli::try_parse_from(["day", "launch", "--day-src", url]).expect("launch");
        match cli.command {
            Cmd::Launch { day_src, .. } => assert_eq!(day_src.as_deref(), Some(url)),
            _ => unreachable!("parsed a launch command"),
        }
        let cli =
            Cli::try_parse_from(["day", "build", "-p", "macos-appkit", "--day-src", "../day"])
                .expect("build");
        match cli.command {
            Cmd::Build { day_src, .. } => assert_eq!(day_src.as_deref(), Some("../day")),
            _ => unreachable!("parsed a build command"),
        }
    }
}

#[cfg(test)]
mod capture_matrix_tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The day-CI shape: themes × locales, with `en` implicit; the exact variant ids the
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
