// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day checkup` (DESIGN.md §16.5) — prove this machine can take a user from `day new` to a
//! shippable artifact, per platform-toolkit combo.
//!
//! One command for what the scheduled install workflow used to spell out in YAML: run the doctor
//! probes (failing fast), then for every combo this host can build, scaffold a fresh app in a
//! temporary directory, build it, and package it. A combo whose prerequisites are missing is
//! SKIPPED with doctor's own fix line when the selection was automatic, and is an ERROR when the
//! caller named it with `-p` — naming a combo asserts it works here.
//!
//! The three steps run as sub-processes of a day CLI, the way [`crate::rebuild`] re-invokes
//! `day pack`: the point of the command is to exercise the real user-facing commands — their
//! argument parsing, their working directory, their exit codes — not the library functions
//! underneath them. `--format json` on the build/pack children gives back the artifact paths, which
//! is where the reported sizes come from.
//!
//! Which CLI is `--day-version`'s answer ([`prepare`]): this binary by default, or one
//! `cargo install`ed at the named release, branch, or commit. The same spec pins the scaffold's
//! `day` dependencies, so the tool under test and the framework under test are always the same day.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anstream::eprintln;

use crate::cli::{CliError, ErrKind, Profile};
use crate::doctor::{self, Readiness};
use crate::new::DaySource;
use crate::ops::status;
use crate::targets::{self, Target};
use crate::term::{BOLD, DIM, ERROR, ERROR_BOLD, SUCCESS, SUCCESS_BOLD, WARN};

/// The app every combo scaffolds. Short on purpose: it becomes a path component under the scratch
/// root, and the deepest cargo target paths beneath it are what run into Windows' path limit.
const APP: &str = "checkup";
const APP_ID: &str = "dev.example.checkup";
/// The `day` git remote, for `--day-version <branch|commit>` installs.
const GIT_URL: &str = "https://github.com/daybrite/day.git";

pub struct Options {
    /// Combos to check (repeatable / comma-separated). Empty = every combo this host can build.
    pub platforms: Vec<String>,
    /// The profile BOTH the build and the pack use — one compile, not two.
    pub profile: Profile,
    /// Stop after the build (what the install workflow did before packaging joined the check).
    pub no_pack: bool,
    /// A combo this host could have checked but is not set up for — or a pack skipped for missing
    /// tooling — is a failure. Combos that build on another OS are never counted (see [`Skip`]).
    pub strict: bool,
    /// Where to scaffold (default: a fresh directory under the system temp dir).
    pub dir: Option<PathBuf>,
    /// Keep the scaffolded projects instead of deleting them.
    pub keep: bool,
    /// `day new` dependency source, passed straight through.
    pub git: bool,
    pub registry: bool,
    pub local: Option<PathBuf>,
    /// Which `day` to check: a release (`0.2.0`), `latest`, a branch (`main`), or a commit. The
    /// CLI that runs new/build/pack is installed at that version, and the scaffold it writes
    /// depends on the same one. Omitted = this binary, and `day new`'s own default.
    pub day_version: Option<String>,
    /// `--format json`: emit the NDJSON result event.
    pub json: bool,
    /// `--verbose`: forward it to every child so their tool output streams.
    pub verbose: bool,
}

/// One combo's place in the run: checked, or skipped with a reason.
#[derive(Debug)]
struct Slot {
    target: &'static Target,
    skip: Option<Skip>,
}

/// Why a combo is out of the run.
#[derive(Debug)]
struct Skip {
    reason: String,
    /// The combo is out because this machine is not set up for it — the skip `--strict` fails on.
    /// A combo that builds on another OS, or an experimental one nobody asked for, is out by
    /// definition rather than by omission, and no amount of installing here would change that.
    fixable: bool,
}

impl Skip {
    /// Out by definition: `--strict` leaves these alone.
    fn inherent(reason: String) -> Self {
        Skip {
            reason,
            fixable: false,
        }
    }
    /// Out because something is missing here.
    fn fixable(reason: String) -> Self {
        Skip {
            reason,
            fixable: true,
        }
    }
}

/// What a checked combo produced.
#[derive(Default)]
struct Report {
    build_seconds: f64,
    pack_seconds: Option<f64>,
    /// `(display name, bytes)` for every artifact the pack produced (or the build's own artifact
    /// when the target has no pack pipeline).
    artifacts: Vec<(String, u64)>,
    /// Why no packaging happened, when that is expected rather than a failure.
    pack_note: Option<String>,
    /// The pack was skipped because its tooling is missing — the one pack skip that is about this
    /// machine rather than about the target, and so the one `--strict` counts.
    pack_missing_tools: bool,
    /// The step that failed, if one did.
    failure: Option<String>,
}

impl Report {
    fn total_seconds(&self) -> f64 {
        self.build_seconds + self.pack_seconds.unwrap_or(0.0)
    }
}

/// Resolve `-p` into targets: comma- or repeat-separated, deduped, order preserved. `Err` is a
/// usage error (exit 2) — a name that is not a target, or one this host cannot build at all.
///
/// Empty in, empty out: that is the automatic selection, which [`plan`] makes.
fn resolve(requested: &[String], host: &str) -> Result<Vec<&'static Target>, String> {
    let mut out: Vec<&'static Target> = Vec::new();
    for name in crate::cli::split_list(requested) {
        let Some(target) = targets::find(&name) else {
            return Err(format!(
                "unknown target {name:?}\n       choose from: {}",
                targets::TARGETS
                    .iter()
                    .map(|t| t.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        if !builds_on(target, host) {
            return Err(format!(
                "target {} builds on a {} host (this is {host})",
                target.name, target.host
            ));
        }
        if !out.iter().any(|t| t.name == target.name) {
            out.push(target);
        }
    }
    Ok(out)
}

/// Which combos to check, and why the rest are out.
///
/// Pure, with the environment injected through `ready`, so the selection rules are testable
/// without a toolchain: `Err` is an environment failure (exit 3), a `Slot` with `skip` is reported
/// and counted, and `skip: None` means "check it".
fn plan(
    host: &str,
    requested: &[&'static Target],
    ready: &dyn Fn(&str) -> Option<Readiness>,
) -> Result<Vec<Slot>, String> {
    if requested.is_empty() {
        // Automatic: everything this host can build, today, with what is installed. Experimental
        // targets stay out — a default checkup should not spend minutes on a combo the user has
        // not opted into.
        let mut slots = Vec::new();
        for target in targets::TARGETS {
            let skip = if !builds_on(target, host) {
                Some(Skip::inherent(format!("builds on a {} host", target.host)))
            } else if target.experimental {
                Some(Skip::inherent(format!(
                    "experimental (name it with `-p {}` to check it)",
                    target.name
                )))
            } else {
                match ready(doctor::group_id(target.toolkit)) {
                    None => Some(Skip::fixable("no builtin toolkit checks".into())),
                    Some(r) if !r.can_build() => {
                        Some(Skip::fixable(missing_line(&r.missing_build)))
                    }
                    Some(_) => None,
                }
            };
            slots.push(Slot { target, skip });
        }
        return Ok(slots);
    }

    // Named explicitly: the caller asserts these work here, so an unbuildable one is an error
    // rather than a quiet skip. Silence is how a CI job that lost a prerequisite goes green.
    // (The focused `day doctor` above reaches the same verdict first, with the setup text; this
    // is the invariant that keeps the two from drifting apart.)
    let mut slots = Vec::new();
    for target in requested {
        if let Some(r) = ready(doctor::group_id(target.toolkit))
            && !r.can_build()
        {
            return Err(format!(
                "{} is not set up on this host — {}",
                target.name,
                missing_line(&r.missing_build)
            ));
        }
        slots.push(Slot { target, skip: None });
    }
    Ok(slots)
}

fn builds_on(target: &Target, host: &str) -> bool {
    target.host == "any" || target.host == host
}

fn skip_reason(slot: &Slot) -> &str {
    slot.skip.as_ref().map(|s| s.reason.as_str()).unwrap_or("")
}

/// The `day` a run checks: the CLI binary that runs new/build/pack, and the version its scaffold
/// depends on. Both halves come from one `--day-version`, so the tool and the framework it
/// scaffolds against can never drift apart within a run.
struct Under {
    /// `None` = this binary, with `day new`'s own default dependency source (today's behavior).
    source: Option<DaySource>,
    bin: PathBuf,
}

impl Under {
    fn label(&self) -> String {
        match &self.source {
            Some(s) => s.label(),
            None => format!("this CLI ({})", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Resolve `--day-version` into a CLI to run: this binary when the spec names the version it
/// already is, otherwise `cargo install` into the run's scratch directory.
///
/// The installed CLI must be able to PIN its scaffold (`day new --day-version`), or the check
/// would build a release's CLI against the git remote's default branch and report the result as
/// that release. A CLI that predates the flag is therefore refused, with the alternative named.
fn prepare(spec: Option<&str>, root: &Path, opts: &Options) -> Result<Under, CliError> {
    let Some(spec) = spec else {
        return Ok(Under {
            source: None,
            bin: std::env::current_exe()
                .map_err(|e| CliError::env(format!("locating this day binary: {e}")))?,
        });
    };
    if opts.local.is_some() {
        // Caught here rather than inside the first `day new`, which would report it as a combo
        // failure after the CLI install had already been paid for.
        return Err(CliError::usage(
            "--day-version names a published day; --local builds against a checkout. Pass one.",
        ));
    }
    // `latest` is resolved HERE, once: the child is handed the concrete version, so a release
    // published mid-run cannot leave the CLI and the scaffold on different days.
    let source = DaySource::parse(spec).map_err(CliError::usage)?;

    if let DaySource::Release(v) = &source
        && v == env!("CARGO_PKG_VERSION")
    {
        status(
            "Day",
            &format!("{v} — already this binary, installing nothing"),
        );
        return Ok(Under {
            source: Some(source),
            bin: std::env::current_exe()
                .map_err(|e| CliError::env(format!("locating this day binary: {e}")))?,
        });
    }

    let cli_root = root.join("cli");
    status(
        "Installing",
        &format!("day-cli {} → {}", source.label(), cli_root.display()),
    );
    let mut cmd = Command::new("cargo");
    cmd.arg("install");
    match &source {
        DaySource::Release(v) => {
            cmd.args(["day-cli", "--version", v]);
        }
        DaySource::Branch(b) => {
            cmd.args(["--git", GIT_URL, "--branch", b, "day-cli"]);
        }
        DaySource::Rev(r) => {
            cmd.args(["--git", GIT_URL, "--rev", r, "day-cli"]);
        }
    }
    // `--locked` so the install resolves the dependency versions the release (or the branch's
    // Cargo.lock) was tested with, rather than whatever is newest today.
    cmd.args(["--locked", "--root"]).arg(&cli_root);
    let ok = cmd
        .status()
        .map_err(|e| CliError::env(format!("running cargo install: {e}")))?;
    if !ok.success() {
        return Err(CliError::env(format!(
            "cargo install day-cli ({}) failed — see the output above",
            source.label()
        )));
    }
    let bin = cli_root
        .join("bin")
        .join(if cfg!(windows) { "day.exe" } else { "day" });
    if !bin.is_file() {
        return Err(CliError::env(format!("no day binary at {}", bin.display())));
    }

    // Can that CLI pin its own scaffold? Ask it, rather than assuming from the version number.
    let help = Command::new(&bin)
        .args(["new", "app", "--help"])
        .output()
        .map_err(|e| CliError::env(format!("running {}: {e}", bin.display())))?;
    let help = String::from_utf8_lossy(&help.stdout);
    if !help.contains("--day-version") {
        return Err(CliError::usage(format!(
            "day-cli {label} predates `day new --day-version`, so its scaffold cannot be \
             pinned to it: the app would build against the git remote's default branch, and \
             the result would not describe {label}. Name a day whose CLI carries the flag.",
            label = source.label(),
        )));
    }
    if opts.verbose {
        status("Day", &format!("{} at {}", source.label(), bin.display()));
    }
    Ok(Under {
        source: Some(source),
        bin,
    })
}

/// `qt6-widgets missing: install Qt 6 (…)` — doctor's probe name and its fix line, so a skip
/// reason tells the reader the same thing `day doctor` would.
fn missing_line(missing: &[doctor::Missing]) -> String {
    missing
        .iter()
        .map(|m| format!("{} missing: {}", m.name, m.fix))
        .collect::<Vec<_>>()
        .join("; ")
}

/// `day checkup`. Exit codes (via the cli.rs kind→code map): 2 usage, 3 environment (doctor
/// failed, a strict skip, or nothing to check), 4 a build or pack failed. Verdicts the combo
/// report already explains come back as Ok(code); everything else is a typed error.
pub fn run(opts: &Options) -> Result<i32, CliError> {
    let host = targets::host_os();
    let started = std::time::Instant::now();

    // One probe pass per toolkit group, cached: `doctor::readiness` runs real processes, and the
    // selection asks about the same group once per target that uses it.
    let mut cache: std::collections::HashMap<String, Option<Readiness>> =
        std::collections::HashMap::new();
    for t in targets::TARGETS {
        let id = doctor::group_id(t.toolkit).to_string();
        cache
            .entry(id.clone())
            .or_insert_with(|| doctor::readiness(&id));
    }
    let lookup = |id: &str| -> Option<Readiness> { cache.get(id).cloned().flatten() };

    let requested = resolve(&opts.platforms, host).map_err(CliError::usage)?;

    // Step 1: the environment, and stop here if it is broken. With combos named explicitly their
    // toolkits are FOCUSED — misses are errors and the setup text prints — which is what the
    // per-combo CI jobs relied on `day doctor --toolkit <t>` for.
    let mut focus: Vec<String> = Vec::new();
    for t in &requested {
        let id = doctor::group_id(t.toolkit).to_string();
        if !focus.contains(&id) {
            focus.push(id);
        }
    }
    if doctor::run(&focus, &[])? != 0 {
        return Err(CliError::env(
            "the environment check failed — checkup stops here",
        ));
    }

    let slots = plan(host, &requested, &lookup).map_err(CliError::env)?;

    let checking: Vec<&Slot> = slots.iter().filter(|s| s.skip.is_none()).collect();
    let skipped: Vec<&Slot> = slots.iter().filter(|s| s.skip.is_some()).collect();
    if checking.is_empty() {
        // Report-style: the head line plus one dim line per skipped combo, in reading order —
        // printed here rather than folded into one CliError message; only the CODE comes from
        // the map.
        eprintln!(
            "error: nothing to check on this {host} host — every combo was skipped.\n       \
             Run `day doctor` for what is missing, or name a combo with `-p <target>`."
        );
        for s in &skipped {
            eprintln!("  {DIM}{:<16}{DIM:#} {}", s.target.name, skip_reason(s));
        }
        return Ok(ErrKind::Env.exit_code());
    }

    // `--strict` says every combo this machine could check must be checked, and that verdict is
    // already knowable — report it now rather than after ten minutes of builds that cannot change
    // it. (A CI cell naming its combo with `-p` has no skips to trip on; this is the bare-checkup
    // path.) The pack stages are the strict skips that can only be judged later.
    let unchecked: Vec<&&Slot> = skipped
        .iter()
        .filter(|s| s.skip.as_ref().is_some_and(|k| k.fixable))
        .collect();
    if opts.strict && !unchecked.is_empty() {
        // One line per skipped combo (report-style, as above); the code comes from the map.
        for s in &unchecked {
            eprintln!(
                "error: --strict — {} was skipped: {}",
                s.target.name,
                skip_reason(s)
            );
        }
        return Ok(ErrKind::Env.exit_code());
    }

    let root = opts.dir.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("day-checkup-{}", std::process::id()))
    });
    std::fs::create_dir_all(&root)
        .map_err(|e| CliError::build(format!("{}: {e}", root.display())))?;
    // Which day is under test — and, when `--day-version` names one, the CLI install that gets it.
    // Before any combo runs: a bad spec or a failed install is not worth a build first.
    let under = match prepare(opts.day_version.as_deref(), &root, opts) {
        Ok(u) => u,
        Err(e) => {
            // A half-prepared run still made (and may have installed a CLI into) the scratch root.
            if !opts.keep && opts.dir.is_none() {
                let _ = std::fs::remove_dir_all(&root);
            }
            return Err(e);
        }
    };
    status(
        "Checkup",
        &format!(
            "{} ({} combo(s), {} skipped) against day {} in {}",
            checking
                .iter()
                .map(|s| s.target.name)
                .collect::<Vec<_>>()
                .join(", "),
            checking.len(),
            skipped.len(),
            under.label(),
            root.display()
        ),
    );

    // Step 2: scaffold → build → pack, one combo at a time. Each combo gets its own project so a
    // single-target scaffold is exercised per combo (the path that broke silently when
    // harmony-arkui was renamed), and a failure in one cannot contaminate the next.
    let mut reports: Vec<(&'static Target, Report)> = Vec::new();
    for slot in &checking {
        let target = slot.target;
        let dir = root.join(target.name);
        if opts.dir.is_some() && dir.exists() {
            // A directory the CALLER named. Whatever is in it is theirs, and checkup deletes what
            // it scaffolds — so refuse rather than clear it out.
            return Err(CliError::usage(format!(
                "{} already exists — remove it, or point --dir somewhere else",
                dir.display()
            )));
        }
        // Our own pid-scoped scratch: a leftover can only be ours (a killed run, a reused pid).
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| CliError::build(format!("{}: {e}", dir.display())))?;
        // Packaging tools are warnings in doctor, so a missing one skips the pack stage with a
        // reason rather than failing the combo — `--strict` is what turns that skip red.
        let missing_pack = lookup(doctor::group_id(target.toolkit))
            .map(|r| r.missing_pack)
            .unwrap_or_default();
        reports.push((target, check_one(target, &dir, opts, &under, &missing_pack)));
        // Delete each combo as it finishes rather than all of them at the end: a debug build tree
        // is gigabytes, and five combos held at once is disk a laptop (or a runner) may not have.
        // The sizes above were measured while the artifacts existed.
        if !opts.keep {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    if opts.keep {
        status("Kept", &root.display().to_string());
    } else if opts.dir.is_some() {
        // The caller's directory stays; the CLI this run installed into it does not.
        let _ = std::fs::remove_dir_all(root.join("cli"));
    } else {
        // Only the scratch root checkup created. A `--dir` the caller named may hold anything
        // else of theirs; the per-combo directories above are all this run is entitled to remove.
        let _ = std::fs::remove_dir_all(&root);
    }

    // Step 3: the report — per combo, with the build time and the size of what it packed.
    let failed = reports.iter().filter(|(_, r)| r.failure.is_some()).count();
    let pack_skips = reports.iter().filter(|(_, r)| r.pack_missing_tools).count();
    let day = under.label();
    summarize(&reports, &skipped, started.elapsed().as_secs_f64(), &day);
    if crate::ops::github_actions() {
        write_step_summary(&reports, &skipped, started.elapsed().as_secs_f64(), &day);
    }
    if opts.json {
        print_json(
            &reports,
            &skipped,
            started.elapsed().as_secs_f64(),
            failed == 0,
            &day,
        );
    }

    if failed > 0 {
        // The combo report above already named each failure; the verdict is the build code.
        Ok(ErrKind::Build.exit_code())
    } else if opts.strict && pack_skips > 0 {
        // Combo skips already returned above; what is left is a pack stage whose tooling this
        // machine does not have (doctor reports those as warnings, so nothing else fails on them).
        Err(CliError::env(format!(
            "--strict — {pack_skips} pack step(s) skipped for missing tooling"
        )))
    } else {
        Ok(0)
    }
}

/// Scaffold, build, and pack one combo. Every step's failure is recorded rather than raised: the
/// remaining combos still run, which is the `fail-fast: false` the per-combo CI jobs had.
fn check_one(
    target: &'static Target,
    dir: &Path,
    opts: &Options,
    under: &Under,
    missing_pack: &[doctor::Missing],
) -> Report {
    let mut report = Report::default();
    status("Scaffold", &format!("{} in {}", target.name, dir.display()));

    let new_args = new_app_args(target, opts, under);
    match day(&under.bin, &new_args, dir, false, opts) {
        Ok(run) if run.code == 0 => {}
        Ok(run) => {
            report.failure = Some(format!("`day new app` failed (exit {})", run.code));
            return report;
        }
        Err(e) => {
            report.failure = Some(format!("`day new app`: {e}"));
            return report;
        }
    }
    let project = dir.join(APP);

    let build = day(
        &under.bin,
        &[
            "build".into(),
            "-p".into(),
            target.name.into(),
            "--profile".into(),
            opts.profile.to_string(),
            "--format".into(),
            "json".into(),
        ],
        &project,
        true,
        opts,
    );
    let built = match build {
        Ok(run) if run.code == 0 => run,
        Ok(run) => {
            report.failure = Some(format!("`day build` failed (exit {})", run.code));
            return report;
        }
        Err(e) => {
            report.failure = Some(format!("`day build`: {e}"));
            return report;
        }
    };
    // No "Built" line here: the child already printed its own, with the artifact path.
    report.build_seconds = built.seconds;

    // Packaging. A target with no pack pipeline (GTK/Qt off their native OS, web-dom) is not a
    // failure — `pack_support` carries day's own explanation, so the report quotes it.
    if opts.no_pack {
        report.pack_note = Some("--no-pack".into());
    } else if let Err(why) = pack_support(target) {
        report.pack_note = Some(why);
    } else if !missing_pack.is_empty() {
        report.pack_note = Some(missing_line(missing_pack));
        report.pack_missing_tools = true;
    }
    if report.pack_note.is_none() {
        let packed = day(
            &under.bin,
            &[
                "pack".into(),
                "-p".into(),
                target.name.into(),
                "--profile".into(),
                opts.profile.to_string(),
                "--format".into(),
                "json".into(),
            ],
            &project,
            true,
            opts,
        );
        match packed {
            Ok(run) if run.code == 0 => {
                report.pack_seconds = Some(run.seconds);
                report.artifacts = artifacts_from(run.json.as_ref(), &project);
                status(
                    "Packed",
                    &format!(
                        "{} in {:.1}s — {}",
                        target.name,
                        run.seconds,
                        artifact_summary(&report.artifacts)
                    ),
                );
            }
            Ok(run) => {
                report.failure = Some(format!("`day pack` failed (exit {})", run.code));
            }
            Err(e) => report.failure = Some(format!("`day pack`: {e}")),
        }
    }
    // Nothing was packed, so the build's own artifact is what this combo produced — report its
    // size rather than leaving the row blank.
    if report.artifacts.is_empty() && report.failure.is_none() {
        report.artifacts = artifacts_from(built.json.as_ref(), &project);
    }
    report
}

/// The `day new app` command line for one combo.
///
/// Pure, and tested, because of the last clause: the scaffold is pinned to the RESOLVED spec that
/// [`prepare`] installed the CLI from — never the word `latest`, which the child would resolve for
/// itself at a different moment and possibly to a different release.
fn new_app_args(target: &Target, opts: &Options, under: &Under) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "new".into(),
        "app".into(),
        APP.into(),
        "--toolkit".into(),
        target.name.into(),
        "--appid".into(),
        APP_ID.into(),
        "--no-input".into(),
    ];
    // The dependency source rides through untouched: `day new`'s own default is the published
    // user path, and that is what a checkup is checking.
    if opts.git {
        args.push("--git".into());
    }
    if opts.registry {
        args.push("--registry".into());
    }
    if let Some(source) = &under.source {
        args.push("--day-version".into());
        args.push(source.spec());
    }
    if let Some(p) = &opts.local {
        args.push("--local".into());
        args.push(p.display().to_string());
    }
    args
}

/// Whether `day pack` has a pipeline for this target, or day's own explanation of why it has none.
fn pack_support(target: &'static Target) -> Result<(), String> {
    crate::pack::default_formats(target)
        .map(|_| ())
        // The refusals explain themselves at length (what to pack instead, how to develop the
        // combo meanwhile); a summary row wants the reason alone.
        .map_err(|e| first_sentence(&e, 100))
}

/// The first sentence of `text`, capped at `max` characters and cut at a word boundary — a table
/// cell, not an essay. The full text is what `day pack` prints when the user goes and runs it.
fn first_sentence(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let sentence = match flat.find(". ") {
        Some(i) => &flat[..i],
        None => flat.trim_end_matches('.'),
    };
    match sentence.char_indices().nth(max) {
        None => sentence.to_string(),
        Some((i, _)) => {
            let head = &sentence[..i];
            let cut = head.rfind(char::is_whitespace).unwrap_or(head.len());
            format!("{}…", head[..cut].trim_end())
        }
    }
}

/// Read the artifact paths out of a child's `--format json` result event and stat each one.
fn artifacts_from(json: Option<&serde_json::Value>, project: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Some(targets) = json
        .and_then(|j| j.get("targets"))
        .and_then(|t| t.as_array())
    else {
        return out;
    };
    for t in targets {
        let Some(list) = t.get("artifacts").and_then(|a| a.as_array()) else {
            continue;
        };
        for a in list {
            let Some(path) = a.get("path").and_then(|p| p.as_str()) else {
                continue;
            };
            let path = Path::new(path);
            let abs = if path.is_absolute() {
                path.to_path_buf()
            } else {
                project.join(path)
            };
            let name = abs
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            out.push((name, bytes_on_disk(&abs)));
        }
    }
    out
}

/// Bytes on disk. Directories are walked: a macOS `.app` and an iOS `.app` are the build artifact
/// for their targets, and "0 B" for a bundle would be a lie rather than a missing number.
fn bytes_on_disk(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_symlink() {
        return 0; // counted once, at its target, if that target is inside the tree
    }
    if !meta.is_dir() {
        return meta.len();
    }
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            total += bytes_on_disk(&e.path());
        }
    }
    total
}

/// `12.4 MB` — decimal units, the way a download page or a store listing reports a size.
fn human_bytes(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "KB")];
    for (scale, unit) in UNITS {
        if n >= scale {
            return format!("{:.1} {unit}", n as f64 / scale as f64);
        }
    }
    format!("{n} B")
}

fn artifact_summary(artifacts: &[(String, u64)]) -> String {
    if artifacts.is_empty() {
        return "no artifact".into();
    }
    artifacts
        .iter()
        .map(|(name, bytes)| format!("{name} ({})", human_bytes(*bytes)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One `day` sub-process. stderr is inherited so its status lines stream live; stdout is captured
/// only when the caller wants the `--format json` result event back (day writes its own status
/// lines to stderr precisely so stdout stays parseable).
struct Child {
    code: i32,
    seconds: f64,
    json: Option<serde_json::Value>,
}

fn day(
    bin: &Path,
    args: &[String],
    cwd: &Path,
    capture: bool,
    opts: &Options,
) -> Result<Child, String> {
    let mut cmd = Command::new(bin);
    cmd.current_dir(cwd).args(args);
    if opts.verbose {
        cmd.arg("--verbose");
    }
    cmd.stderr(Stdio::inherit());
    cmd.stdout(if capture {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    let start = std::time::Instant::now();
    let out = cmd
        .output()
        .map_err(|e| format!("running {}: {e}", bin.display()))?;
    let seconds = start.elapsed().as_secs_f64();
    // The result event is the last JSON object on stdout; anything else there (a nudge, a tool
    // that ignored the convention) is skipped rather than treated as a parse failure.
    let json = capture
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        .and_then(|text| {
            text.lines()
                .rev()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
                .find(|v| v.get("event").and_then(|e| e.as_str()) == Some("result"))
        });
    Ok(Child {
        code: out.status.code().unwrap_or(1),
        seconds,
        json,
    })
}

// --- reporting -------------------------------------------------------------

fn summarize(reports: &[(&'static Target, Report)], skipped: &[&Slot], total: f64, day: &str) {
    eprintln!();
    eprintln!("{BOLD}checkup{BOLD:#} {DIM}— day {day}{DIM:#}");
    for (target, r) in reports {
        let pack = match (&r.pack_seconds, &r.pack_note) {
            (Some(s), _) => format!("pack {s:.1}s"),
            (None, Some(note)) => format!("pack n/a — {note}"),
            (None, None) => "pack —".into(),
        };
        match &r.failure {
            None => eprintln!(
                "  {SUCCESS}✓{SUCCESS:#} {:<16} build {:.1}s  {pack}  total {:.1}s  {}",
                target.name,
                r.build_seconds,
                r.total_seconds(),
                artifact_summary(&r.artifacts),
            ),
            Some(why) => {
                eprintln!(
                    "  {ERROR}✗{ERROR:#} {:<16} {why} (after {:.1}s)",
                    target.name,
                    r.total_seconds()
                );
                // A red cell in a scheduled run is the whole product of this command; make it an
                // annotation too so the job page names the combo without opening the log.
                if crate::ops::github_actions() {
                    println!(
                        "::error title=day checkup {}::{}",
                        target.name,
                        crate::ops::gha_escape(why)
                    );
                }
            }
        }
    }
    for s in skipped {
        eprintln!(
            "  {WARN}⚠{WARN:#} {:<16} skipped — {}",
            s.target.name,
            skip_reason(s)
        );
    }
    let failed = reports.iter().filter(|(_, r)| r.failure.is_some()).count();
    let passed = reports.len() - failed;
    eprintln!();
    if failed > 0 {
        eprintln!(
            "{ERROR_BOLD}✗ {failed} failed{ERROR_BOLD:#}, {passed} passed, {} skipped — {:.1}s total",
            skipped.len(),
            total
        );
    } else {
        eprintln!(
            "{SUCCESS_BOLD}✓ {passed} passed{SUCCESS_BOLD:#}, {} skipped — {:.1}s total",
            skipped.len(),
            total
        );
    }
}

/// The same table on the job's run-summary page: what was built, how long it took, and how big the
/// result is — the numbers a scheduled run exists to keep an eye on.
fn write_step_summary(
    reports: &[(&'static Target, Report)],
    skipped: &[&Slot],
    total: f64,
    day: &str,
) {
    let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return;
    };
    use std::fmt::Write as _;
    let mut md = format!("## day checkup — day {day}\n\n");
    md.push_str("| combo | result | build | pack | total | artifacts |\n");
    md.push_str("| --- | --- | --: | --: | --: | --- |\n");
    for (target, r) in reports {
        let pack = match (&r.pack_seconds, &r.pack_note) {
            (Some(s), _) => format!("{s:.1}s"),
            (None, Some(note)) => format!("n/a — {}", note.replace('|', "\\|")),
            (None, None) => "—".into(),
        };
        let artifacts = if r.artifacts.is_empty() {
            "—".to_string()
        } else {
            r.artifacts
                .iter()
                .map(|(name, bytes)| format!("`{name}` {}", human_bytes(*bytes)))
                .collect::<Vec<_>>()
                .join("<br>")
        };
        let (icon, result) = match &r.failure {
            None => ("✅", "passed".to_string()),
            Some(why) => ("❌", why.replace('|', "\\|")),
        };
        let _ = writeln!(
            md,
            "| {icon} `{}` | {result} | {:.1}s | {pack} | {:.1}s | {artifacts} |",
            target.name,
            r.build_seconds,
            r.total_seconds()
        );
    }
    for s in skipped {
        let _ = writeln!(
            md,
            "| ⚠️ `{}` | skipped — {} | — | — | — | — |",
            s.target.name,
            skip_reason(s).replace('|', "\\|")
        );
    }
    let failed = reports.iter().filter(|(_, r)| r.failure.is_some()).count();
    let _ = writeln!(
        md,
        "\n{} passed, {failed} failed, {} skipped — {total:.1}s total\n",
        reports.len() - failed,
        skipped.len()
    );
    // Appending, not truncating: earlier steps' summaries are theirs to keep. Best-effort — a
    // failed summary write must never fail the checkup.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        use std::io::Write as _;
        let _ = file.write_all(md.as_bytes());
    }
}

fn print_json(
    reports: &[(&'static Target, Report)],
    skipped: &[&Slot],
    total: f64,
    ok: bool,
    day: &str,
) {
    let mut targets: Vec<serde_json::Value> = reports
        .iter()
        .map(|(target, r)| {
            let artifacts: Vec<serde_json::Value> = r
                .artifacts
                .iter()
                .map(|(name, bytes)| serde_json::json!({"name": name, "bytes": bytes}))
                .collect();
            serde_json::json!({
                "target": target.name,
                "ok": r.failure.is_none(),
                "status": if r.failure.is_none() { "passed" } else { "failed" },
                "error": r.failure,
                "build_seconds": r.build_seconds,
                "pack_seconds": r.pack_seconds,
                "pack_skipped": r.pack_note,
                "seconds": r.total_seconds(),
                "artifacts": artifacts,
            })
        })
        .collect();
    targets.extend(skipped.iter().map(|s| {
        serde_json::json!({
            "target": s.target.name, "ok": true, "status": "skipped",
            "reason": skip_reason(s),
            // Whether setting this machine up would have let the combo run — what `--strict` acts on.
            "fixable": s.skip.as_ref().is_some_and(|k| k.fixable),
        })
    }));
    println!(
        "{}",
        serde_json::json!({
            "event": "result", "command": "checkup", "ok": ok,
            "day": day, "seconds": total, "targets": targets,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host where everything is installed.
    fn all_ready(_: &str) -> Option<Readiness> {
        Some(Readiness::default())
    }

    fn missing_build(name: &'static str) -> Option<Readiness> {
        Some(Readiness {
            missing_build: vec![doctor::Missing {
                name,
                fix: format!("install {name}"),
            }],
            missing_pack: Vec::new(),
        })
    }

    fn names(slots: &[Slot]) -> Vec<&'static str> {
        slots
            .iter()
            .filter(|s| s.skip.is_none())
            .map(|s| s.target.name)
            .collect()
    }

    /// `-p …` the way `run` applies it: resolve, then plan.
    fn plan_for(
        host: &str,
        requested: &[&str],
        ready: &dyn Fn(&str) -> Option<Readiness>,
    ) -> Result<Vec<Slot>, String> {
        let raw: Vec<String> = requested.iter().map(|s| s.to_string()).collect();
        let resolved = resolve(&raw, host)?;
        plan(host, &resolved, ready)
    }

    /// Bare `day checkup` on a fully-equipped macOS host: the macOS-hosted and host-agnostic
    /// combos, and neither the Linux/Windows ones nor the experimental web target.
    #[test]
    fn auto_selects_what_this_host_can_build() {
        let slots = plan_for("macos", &[], &all_ready).unwrap();
        let checked = names(&slots);
        assert!(checked.contains(&"macos-appkit"));
        assert!(checked.contains(&"ios-uikit"));
        assert!(checked.contains(&"android-mdc"), "host: any");
        assert!(!checked.contains(&"linux-gtk"));
        assert!(!checked.contains(&"windows-xaml"));
        assert!(!checked.contains(&"web-dom"), "experimental");
        let web = slots.iter().find(|s| s.target.name == "web-dom").unwrap();
        assert!(skip_reason(web).contains("experimental"));
    }

    /// `--strict` fails on combos this machine could have checked and didn't. A combo that builds
    /// on another OS, or an experimental one nobody named, is out by definition — counting those
    /// would make `day checkup --strict` impossible to pass anywhere.
    #[test]
    fn only_a_machine_this_host_could_fix_counts_as_a_strict_skip() {
        let ready = |id: &str| {
            if id == "qt" {
                missing_build("qt6-widgets")
            } else {
                Some(Readiness::default())
            }
        };
        let slots = plan_for("macos", &[], &ready).unwrap();
        let fixable: Vec<&str> = slots
            .iter()
            .filter(|s| s.skip.as_ref().is_some_and(|k| k.fixable))
            .map(|s| s.target.name)
            .collect();
        assert_eq!(fixable, vec!["macos-qt"], "only the unequipped toolkit");
    }

    /// A missing prerequisite is a skip with doctor's fix line — not a failure, and not silence.
    #[test]
    fn auto_skips_an_unready_toolkit_with_the_fix() {
        let ready = |id: &str| {
            if id == "qt" {
                missing_build("qt6-widgets")
            } else {
                Some(Readiness::default())
            }
        };
        let slots = plan_for("macos", &[], &ready).unwrap();
        assert!(!names(&slots).contains(&"macos-qt"));
        let qt = slots.iter().find(|s| s.target.name == "macos-qt").unwrap();
        let why = skip_reason(qt);
        assert!(why.contains("qt6-widgets missing"), "{why}");
        assert!(why.contains("install qt6-widgets"), "{why}");
    }

    /// Naming combos checks exactly those, comma- or repeat-separated, deduped, in the order given.
    #[test]
    fn explicit_targets_are_checked_verbatim() {
        let slots = plan_for(
            "macos",
            &["ios-uikit,macos-appkit", "ios-uikit"],
            &all_ready,
        )
        .unwrap();
        assert_eq!(names(&slots), vec!["ios-uikit", "macos-appkit"]);
    }

    /// Naming a combo asserts it works here, so an unready one is an error — the silent skip is
    /// what would let a CI job whose prerequisite install broke report success.
    #[test]
    fn explicit_target_that_is_not_set_up_is_an_error() {
        let ready = |id: &str| {
            if id == "qt" {
                missing_build("qt6-widgets")
            } else {
                Some(Readiness::default())
            }
        };
        let e = plan_for("macos", &["macos-qt"], &ready).unwrap_err();
        assert!(e.contains("macos-qt"), "{e}");
        assert!(e.contains("qt6-widgets missing"), "{e}");
    }

    /// The two usage errors, each naming the fix. These are `resolve`'s (exit 2), not the
    /// environment's (exit 3) — no probe can make `windows-xaml` build on a Mac.
    #[test]
    fn unknown_and_cross_host_targets_are_usage_errors() {
        let e = resolve(&["macos-swiftui".to_string()], "macos").unwrap_err();
        assert!(e.contains("unknown target"), "{e}");
        assert!(e.contains("macos-appkit"), "the list is offered: {e}");

        let e = resolve(&["ios-uikit".to_string()], "linux").unwrap_err();
        assert!(e.contains("builds on a macos host"), "{e}");
    }

    /// An externally declared toolkit has no builtin probes; automatic selection leaves it out
    /// rather than assuming it is ready.
    #[test]
    fn auto_skips_a_toolkit_with_no_builtin_checks() {
        let none = |_: &str| -> Option<Readiness> { None };
        let slots = plan_for("macos", &[], &none).unwrap();
        assert!(names(&slots).is_empty());
        assert!(slots.iter().all(|s| {
            skip_reason(s).contains("no builtin toolkit checks")
                || !builds_on(s.target, "macos")
                || s.target.experimental
        }));
    }

    /// A pack refusal becomes a row-sized reason, not the whole paragraph day prints when you go
    /// and run the pack yourself.
    #[test]
    fn pack_refusals_shrink_to_a_row() {
        let note = pack_support(targets::find("macos-gtk").unwrap()).unwrap_err();
        assert_eq!(
            note,
            "pack for macos-gtk means bundling the toolkit into the package — deferred (DESIGN.md DP-7)"
        );
        assert!(pack_support(targets::find("macos-appkit").unwrap()).is_ok());

        assert_eq!(first_sentence("one two. three four.", 72), "one two");
        assert_eq!(first_sentence("no full stop here", 72), "no full stop here");
        // Over the cap: cut back to a word boundary rather than mid-word.
        assert_eq!(first_sentence("aaaa bbbb cccc", 9), "aaaa…");
    }

    fn opts_for(day_version: Option<&str>) -> Options {
        Options {
            platforms: Vec::new(),
            profile: Profile::Debug,
            no_pack: false,
            strict: false,
            dir: None,
            keep: false,
            git: false,
            registry: false,
            local: None,
            day_version: day_version.map(String::from),
            json: false,
            verbose: false,
        }
    }

    /// The scaffold is pinned to the day the CLI came from, by its RESOLVED spec. Passing `latest`
    /// through would let the child resolve it again — and a release published in between would
    /// leave the CLI and the app it scaffolds on different days, which is the one thing
    /// `--day-version` exists to prevent.
    #[test]
    fn the_scaffold_is_pinned_to_the_resolved_day() {
        let target = targets::find("macos-gtk").unwrap();
        let under = Under {
            // What `--day-version latest` becomes once crates.io has answered.
            source: Some(DaySource::Release("0.2.0".into())),
            bin: PathBuf::from("day"),
        };
        let args = new_app_args(target, &opts_for(Some("latest")), &under);
        let i = args.iter().position(|a| a == "--day-version").unwrap();
        assert_eq!(args[i + 1], "0.2.0");
        assert!(!args.contains(&"latest".to_string()));

        // No --day-version: nothing is passed, and `day new` keeps its own default.
        let under = Under {
            source: None,
            bin: PathBuf::from("day"),
        };
        let args = new_app_args(target, &opts_for(None), &under);
        assert!(!args.contains(&"--day-version".to_string()));
        assert!(args.contains(&"--no-input".to_string()));
    }

    #[test]
    fn sizes_read_the_way_a_download_page_prints_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(12_400_000), "12.4 MB");
        assert_eq!(human_bytes(1_500_000_000), "1.5 GB");
    }

    /// The artifact rows come from the child's `--format json` result event, and a relative path
    /// (what `day build` reports) resolves against the project it was built in.
    #[test]
    fn artifacts_are_read_from_the_child_result_event() {
        let dir = std::env::temp_dir().join(format!("day-checkup-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("app.dmg");
        std::fs::write(&file, vec![7u8; 2048]).unwrap();
        let json = serde_json::json!({
            "event": "result", "command": "pack", "ok": true,
            "targets": [{"target": "macos-appkit", "artifacts": [{"path": "app.dmg"}]}],
        });
        let got = artifacts_from(Some(&json), &dir);
        assert_eq!(got, vec![("app.dmg".to_string(), 2048)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
