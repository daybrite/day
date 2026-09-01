// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day patch` — build a standalone app against a LOCAL day checkout instead of the git dependency.
//!
//! An app outside this repository declares its framework dependencies from git:
//!
//! ```toml
//! day = { git = "https://github.com/daybrite/day.git" }
//! ```
//!
//! which is what a user's app looks like and what CI should resolve. Developing the framework and
//! the app together needs those to come from a checkout instead, and Cargo's answer is a `[patch]`
//! table in a machine-local `.cargo/config.toml`. Writing that table by hand is the problem this
//! command exists to remove: it is a list of absolute paths that goes stale when a dependency is
//! added, and a MISSING ENTRY DOES NOT FAIL. Cargo simply resolves that crate from the git cache,
//! and the build silently mixes a local framework with a published one — green, and testing
//! something other than what you think.
//!
//! Only DIRECT dependencies need an entry. A patched crate's own dependencies are path deps inside
//! the same checkout, so they follow automatically; `--check` is what proves that, by asserting no
//! `day*` package in the resolved graph carries a git source.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::CliError;
use crate::meta::Project;
use crate::ops::status;

/// The git URL an app's framework dependencies point at, and the key of the `[patch]` table.
const DAY_GIT: &str = "https://github.com/daybrite/day.git";

/// Every crate a day checkout publishes: package name → absolute directory.
///
/// Read from the checkout's own workspace members rather than a hardcoded list, so a crate added
/// to the framework needs no change here.
/// One triple per platform Day targets. A day crate reaches an app THROUGH the umbrella and the
/// parts, under `[target.'cfg(…)'.dependencies]` tables the app never names itself — so the set to
/// patch is the resolved graph of every platform, not the app's own manifest. Resolving for the
/// host alone is what let `day-android` build from the git cache while a local checkout sat
/// patched in beside it, silently, on every Android build.
const PATCH_TRIPLES: &[&str] = &[
    "aarch64-apple-darwin",
    "aarch64-apple-ios",
    "aarch64-linux-android",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "wasm32-unknown-unknown",
    "aarch64-unknown-linux-ohos",
];

/// Every day crate in `project`'s resolved graph for one platform, whether it comes from the day
/// git remote or (on a re-run, when the patch is already in place) from the checkout itself.
fn day_crates_for(project: &Project, triple: &str, checkout: Option<&Path>) -> Vec<String> {
    let Ok(out) = Command::new("cargo")
        .current_dir(&project.root)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--filter-platform",
            triple,
            // Backends are OPTIONAL dependencies behind per-toolkit features, so the default
            // resolve sees none of them: an app's gtk build would take day-gtk from the git
            // cache while every other crate came from the checkout.
            "--all-features",
        ])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for pkg in doc
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        let manifest_path = pkg.get("manifest_path").and_then(|m| m.as_str());
        let source = pkg.get("source").and_then(|s| s.as_str());
        if needs_patch_entry(name, manifest_path, source, &project.root, checkout) {
            names.push(name.to_string());
        }
    }
    names
}

/// Whether one resolved package is a day crate this project must patch.
///
/// Split out of [`day_crates_for`] so the decision is testable without a resolver — the
/// `day-showcase` regression below is a three-line predicate that took a CI job to notice.
fn needs_patch_entry(
    name: &str,
    manifest_path: Option<&str>,
    source: Option<&str>,
    project_root: &Path,
    checkout: Option<&Path>,
) -> bool {
    if !name.starts_with("day") {
        return false;
    }
    // The project's OWN packages are never crates to patch, whatever they are called. Apps are
    // named `day-<something>` by convention, so the name filter above does not separate them
    // from the framework — and CI checks an app out INSIDE the day workspace
    // (`day/showcase-src`), which puts the app's manifest under the checkout root and made the
    // `from_checkout` arm below claim it. `day patch` then went looking for `day-showcase`
    // among day's own crates and failed every toolkit job.
    if manifest_path.is_some_and(|m| Path::new(m).starts_with(project_root)) {
        return false;
    }
    let from_git = source.is_some_and(|s| {
        s.trim_start_matches("git+")
            .starts_with(DAY_GIT.trim_end_matches(".git"))
    });
    // Already patched to the checkout: keep it, or a second `day patch` would shrink the table
    // it wrote the first time.
    let from_checkout =
        checkout.is_some_and(|root| manifest_path.is_some_and(|m| Path::new(m).starts_with(root)));
    from_git || from_checkout
}

fn checkout_crates(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("{}: {e} — is that a day checkout?", manifest.display()))?;
    let doc: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let members = doc
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .ok_or_else(|| format!("{}: no [workspace] members", manifest.display()))?;

    let mut out = BTreeMap::new();
    for m in members.iter().filter_map(|m| m.as_str()) {
        let dir = root.join(m);
        let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let Ok(doc) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        if let Some(name) = doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            out.insert(name.to_string(), dir);
        }
    }
    if out.is_empty() {
        return Err(format!("{}: no workspace members resolved", root.display()));
    }
    Ok(out)
}

/// The project's DIRECT dependencies that come from the day git repository.
fn git_deps(project: &Project, checkout: Option<&Path>) -> Result<Vec<String>, String> {
    let mut names = manifest_git_deps(project)?;
    for triple in PATCH_TRIPLES {
        for name in day_crates_for(project, triple, checkout) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

/// The app's OWN git-sourced day dependencies, read straight from its manifest. The floor under
/// [`git_deps`]: it needs no resolver, so a project that cannot resolve offline still patches the
/// crates it names itself.
fn manifest_git_deps(project: &Project) -> Result<Vec<String>, String> {
    let manifest = project.root.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let doc: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut names = Vec::new();
    // Plain dependencies plus every `[target.<cfg>.dependencies]` table — the backend crates an
    // app pulls in per platform live there, and they are exactly the ones easiest to forget.
    let mut tables: Vec<&toml::Value> = Vec::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(t) = doc.get(key) {
            tables.push(t);
        }
    }
    if let Some(target) = doc.get("target").and_then(|t| t.as_table()) {
        for (_, cfg) in target {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(t) = cfg.get(key) {
                    tables.push(t);
                }
            }
        }
    }
    for table in tables {
        let Some(table) = table.as_table() else {
            continue;
        };
        for (name, spec) in table {
            let from_day_git = spec
                .get("git")
                .and_then(|g| g.as_str())
                .is_some_and(|g| g.trim_end_matches(".git") == DAY_GIT.trim_end_matches(".git"));
            if from_day_git && !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// The `[patch]` table mapping every git-sourced day dependency of `project` to `checkout`, as
/// TOML text — and the number of entries in it.
///
/// One table, two lifetimes: `day patch` writes it to `.cargo/config.toml` and every later build
/// picks it up, while `--day-src` writes it to a scratch file handed to ONE cargo invocation
/// through `--config`. The crate set, the missing-crate error, and the wording are therefore the
/// same for both, which is the point of computing it here.
///
/// `header` leads the file, since the two lifetimes need to say different things about it.
fn patch_table(
    project: &Project,
    checkout: &Path,
    header: &str,
) -> Result<(String, usize), String> {
    let available = checkout_crates(checkout)?;
    let wanted = git_deps(project, Some(checkout))?;
    if wanted.is_empty() {
        return Err(
            "this project has no dependencies from the day git repository — nothing to patch \
             (a workspace member of the day repo needs no patch table)"
                .into(),
        );
    }

    let mut lines = String::from(header);
    lines.push_str(&format!("[patch.{DAY_GIT:?}]\n"));
    let mut written = 0;
    let mut missing = Vec::new();
    for name in &wanted {
        match available.get(name) {
            Some(dir) => {
                lines.push_str(&format!(
                    "{name} = {{ path = {:?} }}\n",
                    dir.display().to_string()
                ));
                written += 1;
            }
            None => missing.push(name.clone()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "{} is not in the day checkout at {} — is it the right checkout, or the wrong branch?",
            missing.join(", "),
            checkout.display()
        ));
    }
    Ok((lines, written))
}

/// Write `.cargo/config.toml` mapping every git-sourced day dependency to the checkout.
fn write_patch(project: &Project, checkout: &Path) -> Result<usize, String> {
    let checkout = checkout
        .canonicalize()
        .map_err(|e| format!("{}: {e}", checkout.display()))?;
    let (lines, written) = patch_table(
        project,
        &checkout,
        "# Generated by `day patch` — machine-local, gitignored, and safe to delete.\n\
         # Builds this project against the day checkout below instead of fetching the git\n\
         # dependency. CI has no such file and resolves git, which is what a user's build does.\n",
    )?;

    let dir = project.root.join(".cargo");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("config.toml");
    std::fs::write(&path, lines).map_err(|e| format!("{}: {e}", path.display()))?;
    status(
        "Patched",
        &format!("{} ({written} crate(s))", path.display()),
    );
    // The config file itself is gitignored, but its EFFECT on Cargo.lock is not: the next cargo
    // command rewrites the lock to record these paths, dropping the `source` line from every day
    // crate. Committed, that lock describes a machine nobody else has — it cannot be resolved, and
    // `cargo update -p day` cannot even find a `day` package in it. Said here because the rewrite
    // happens silently, on the next build, long after this command has scrolled away.
    if project.root.join("Cargo.lock").exists() {
        status(
            "Note",
            "cargo will rewrite Cargo.lock to point at this checkout — do not commit it while \
             patched (`git checkout -- Cargo.lock` before committing, or delete .cargo/config.toml \
             and regenerate the lock)",
        );
    }
    Ok(written)
}

/// Assert that no `day*` package in the resolved graph comes from git.
///
/// The check the whole command exists for. A direct dependency without a patch entry resolves from
/// the git cache and builds green, so a build that believes it is testing this checkout may be
/// testing a published crate for part of the graph. Resolution is asked of cargo rather than read
/// out of `Cargo.lock`, so it is correct for a project that has not locked yet.
/// Day crates still resolving from git, across EVERY platform — not just the host's. A check that
/// asks only the host says "all local" while the Android, Linux, Windows, web, and HarmonyOS
/// builds quietly use the git cache, which is a stale-toolkit bug that looks like the fix not
/// working.
pub fn check(project: &Project) -> Result<Vec<String>, String> {
    let mut from_git: Vec<String> = Vec::new();
    let mut asked = false;
    for triple in PATCH_TRIPLES {
        let out = Command::new("cargo")
            .current_dir(&project.root)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--filter-platform",
                triple,
                "--all-features",
            ])
            .output()
            .map_err(|e| format!("cargo metadata: {e}"))?;
        if !out.status.success() {
            continue;
        }
        asked = true;
        let doc: serde_json::Value =
            serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
        for pkg in doc
            .get("packages")
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
        {
            let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            if !name.starts_with("day") {
                continue;
            }
            if pkg
                .get("source")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s.starts_with("git+"))
                && !from_git.iter().any(|n| n == name)
            {
                from_git.push(name.to_string());
            }
        }
    }
    if !asked {
        return Err("cargo metadata failed for every platform".into());
    }
    from_git.sort();
    Ok(from_git)
}

/// `day patch [--local <checkout>] [--check]`.
pub fn run(
    project: &Project,
    local: Option<&Path>,
    check_only: bool,
) -> Result<(), crate::cli::CliError> {
    if let Some(checkout) = local {
        write_patch(project, checkout).map_err(crate::cli::CliError::failure)?;
    }
    match check(project) {
        Ok(from_git) if from_git.is_empty() => {
            status("Verified", "every day crate resolves from a local path");
            Ok(())
        }
        Ok(from_git) => Err(crate::cli::CliError::failure(format!(
            "{} day crate(s) still resolve from git: {} — add them to the [patch] table \
             (`day patch --local <checkout>` rewrites it) or this build mixes a local \
             framework with a published one",
            from_git.len(),
            from_git.join(", ")
        ))),
        Err(e) => {
            // Not fatal without --check: writing the table succeeded, and resolution may need the
            // network the caller does not have.
            status("Warning", &format!("could not verify resolution: {e}"));
            if check_only {
                Err(crate::cli::CliError::failure(format!(
                    "could not verify resolution: {e}"
                )))
            } else {
                Ok(())
            }
        }
    }
}

// --- `--day-src`: the same patch, for exactly one build ------------------------------------------
//
// `day patch` puts you in a mode: the table it writes governs every later build until you delete
// it. `--day-src` answers a different question — "does this branch fix the bug?" — where you build
// the app twice, against two versions of the framework, and look at both. So the table it computes
// never reaches the project: it goes to a scratch file that one cargo invocation reads through
// `--config`, and the run leaves nothing behind.

/// Where the day-src build tree lives, for the process that must find it without the flag.
///
/// The Apple builds run cargo from `day xcode-backend build`, a SEPARATE process xcodebuild calls
/// back into. It learns the day-src the way it already learns the day binary (`DAY_BIN`): one
/// variable naming the directory, from which the cargo config and the build root both derive.
pub const DAY_SRC_DIR_ENV: &str = "DAY_SRC_DIR";

/// This run's day-src build directory, once resolved. A process global for the same reason
/// `--verbose` is one ([`crate::ops::verbose`]): it is a fact about the whole run, and threading it
/// through `build` → `build_native` → each platform's builder would touch every caller of a
/// function whose behavior does not otherwise change.
static ACTIVE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// A resolved `--day-src`.
pub struct DaySrc {
    /// The day checkout to build against, absolute.
    pub checkout: PathBuf,
    /// How it was named, for the status line — a path, or `url @ ref`.
    pub label: String,
    /// `build/day/day-src/<slug>`: this run's build root, and where the cargo config is written.
    pub dir: PathBuf,
}

/// Resolve a `--day-src` argument to a day checkout on disk.
///
/// An existing directory is that checkout. Anything else must be a git URL — cloned into the same
/// per-URL-and-ref cache `--git` uses ([`crate::git`]), so two branches of the framework coexist as
/// two checkouts and switching between them stays incremental.
pub fn resolve_day_src(arg: &str, project: &Project) -> Result<DaySrc, CliError> {
    let arg = arg.trim();
    let local = Path::new(arg);
    let (checkout, label) = if local.is_dir() {
        let abs = local
            .canonicalize()
            .map_err(|e| CliError::usage(format!("--day-src {arg}: {e}")))?;
        let label = abs.display().to_string();
        (abs, label)
    } else {
        let spec = crate::git::parse_spec(arg).map_err(CliError::usage)?;
        if !crate::git::looks_remote(&spec.url) {
            return Err(CliError::usage(format!(
                "--day-src {arg}: not a directory, and not a git URL — pass a path to a day \
                 checkout, or a URL like https://github.com/daybrite/day.git@<branch>"
            )));
        }
        let label = match &spec.git_ref {
            Some(r) => format!("{} @ {r}", spec.url),
            None => spec.url.clone(),
        };
        // Named before the clone: fetching a framework branch takes a while, and the line that
        // explains the wait should be on screen before it starts.
        status("Day src", &label);
        let dest = crate::git::checkout(&spec, None)?;
        status("Checkout", &crate::git::display_path(&dest));
        (dest, label)
    };

    // A directory that is not the framework is the likeliest way to mistype this flag, and the
    // failure without this check is a resolver error about a missing crate, several minutes in.
    let crates = checkout_crates(&checkout)
        .map_err(|e| CliError::usage(format!("--day-src {}: {e}", checkout.display())))?;
    if !crates.contains_key("day") {
        return Err(CliError::usage(format!(
            "--day-src {}: a cargo workspace, but not a day checkout (no `day` crate in it)",
            checkout.display()
        )));
    }

    let dir = project.root.join("build/day/day-src").join(slug(&checkout));
    Ok(DaySrc {
        checkout,
        label,
        dir,
    })
}

/// The build-tree name for one day-src: a readable stem plus a hash of what it resolved to.
///
/// Both come from the resolved checkout rather than what was typed, which gets each of them right
/// at once. The stem is the directory's own name — for a git day-src that is the ref, since the
/// cache is keyed by one (`…/daybrite/day/experimental-nav`). The hash keeps two branches apart
/// when their stems collide, and makes `../day` and the absolute path it points at share one tree
/// instead of building the same framework twice.
fn slug(checkout: &Path) -> String {
    let stem = checkout
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "day-src".to_string());
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let digest = {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(checkout.display().to_string().as_bytes());
        h.finalize()
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    format!("{stem}-{digest}")
}

/// Compute the patch table, write it under the day-src build directory, and make it this run's.
///
/// Called once, from the `build`/`launch` dispatch. Everything downstream reads [`ACTIVE`] (or, in
/// the xcode-backend process, [`DAY_SRC_DIR_ENV`]) and needs no argument of its own.
pub fn activate(src: &DaySrc, project: &Project) -> Result<(), CliError> {
    let (table, count) = patch_table(
        project,
        &src.checkout,
        "# Generated by `day launch --day-src` / `day build --day-src` for ONE build.\n\
         # Handed to cargo with `--config`; nothing in the project is modified, and the next\n\
         # build without the flag resolves the git dependency as usual.\n",
    )
    .map_err(CliError::failure)?;
    std::fs::create_dir_all(&src.dir)
        .map_err(|e| CliError::failure(format!("{}: {e}", src.dir.display())))?;
    let file = config_path(&src.dir);
    std::fs::write(&file, table)
        .map_err(|e| CliError::failure(format!("{}: {e}", file.display())))?;

    // An app that is already `day patch`ed has a `[patch]` table of its own in
    // `.cargo/config.toml`. Cargo ranks a `--config` argument above a config FILE, so this run
    // wins — but silently swapping the framework under someone who deliberately patched their
    // project would be the wrong kind of surprise.
    if project.root.join(".cargo/config.toml").is_file() {
        crate::ops::status(
            "Note",
            "this project has a `day patch` table; --day-src overrides it for this build only",
        );
    }
    status("Patched", &format!("{count} day crate(s) → {}", src.label));
    let _ = ACTIVE.set(src.dir.clone());
    Ok(())
}

/// The cargo config a day-src build directory carries.
fn config_path(dir: &Path) -> PathBuf {
    dir.join("cargo-patch.toml")
}

/// This run's day-src build directory, or `None` when `--day-src` was not given.
///
/// Falls back to the environment so `day xcode-backend build` — a separate process, spawned by
/// xcodebuild, which never sees the flag — resolves the same directory the porcelain did.
pub fn day_src_dir() -> Option<PathBuf> {
    ACTIVE
        .get()
        .cloned()
        .or_else(|| std::env::var_os(DAY_SRC_DIR_ENV).map(PathBuf::from))
}

/// This run's day-src as a short name — the build tree's own directory name.
///
/// Rides into the app on `DAY_APP_VERSION`, so a debug build's window title says which framework
/// it was built against (`Day Rise (0.1.0+main-2d77edbf/appkit)`). Two builds of one app running
/// side by side are otherwise two identical title bars, which is the comparison this flag exists
/// for, lost at the last step.
pub fn day_src_tag() -> Option<String> {
    let dir = day_src_dir()?;
    Some(dir.file_name()?.to_string_lossy().into_owned())
}

/// The `DAY_SRC_DIR=<dir>` assignment to hand a build tool that calls `day` back.
///
/// xcodebuild takes it as a build setting, which it exports to script phases as an environment
/// variable — the route `DAY_BIN` already travels. `None` when `--day-src` was not given, so the
/// argument is simply not added.
pub fn day_src_setting() -> Option<String> {
    day_src_dir().map(|d| format!("{DAY_SRC_DIR_ENV}={}", d.display()))
}

/// Point one cargo invocation at the day-src, if there is one.
///
/// Every command that resolves or compiles the app's graph must carry this, compiles and
/// `cargo metadata` alike: a metadata call that resolves WITHOUT the patch reports the wrong
/// feature union, and the app renders `⟨kind⟩` placeholders for every optional piece.
pub fn apply_day_src(cmd: &mut Command) {
    if let Some(dir) = day_src_dir() {
        cmd.arg("--config").arg(config_path(&dir));
    }
}

/// Cargo records the patched sources in `Cargo.lock`, so a build under `--day-src` would leave the
/// project's lockfile rewritten — a tracked file, modified by a flag whose whole promise is that it
/// changes nothing. This snapshots the lock and puts it back.
///
/// Scoped to the BUILD, not the launch: the app may run for a long time afterwards, and the lock
/// should be correct again the moment the compiler is done with it. Restoring costs no rebuild —
/// the next run re-resolves to the same graph, and the fingerprints live in the day-src's own
/// target directory.
pub struct LockGuard {
    path: PathBuf,
    before: Option<Vec<u8>>,
}

impl LockGuard {
    /// Take the snapshot. A no-op (and no guard work on drop) when `--day-src` is not in effect.
    pub fn new(project: &Project) -> Option<Self> {
        day_src_dir()?;
        let path = project.root.join("Cargo.lock");
        let before = std::fs::read(&path).ok();
        crate::signals::register_restore(&path, before.as_deref());
        Some(LockGuard { path, before })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        restore(&self.path, self.before.as_deref());
        crate::signals::forget_restore(&self.path);
    }
}

/// Put a file back the way it was — or remove it, if it was not there. Shared with the interrupt
/// path in [`crate::signals`], which cannot run a `Drop`.
pub(crate) fn restore(path: &Path, before: Option<&[u8]>) {
    match before {
        Some(bytes) => {
            if std::fs::read(path).ok().as_deref() != Some(bytes) {
                let _ = std::fs::write(path, bytes);
            }
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, manifest: &str) -> Project {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join("Cargo.toml"), manifest).expect("Cargo.toml");
        std::fs::write(
            dir.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.app\"\n",
        )
        .expect("Day.toml");
        crate::meta::find_project(Some(dir)).expect("project")
    }

    /// A day checkout, as far as [`patch_table`] and [`resolve_day_src`] are concerned: a
    /// workspace whose members include a `day` crate.
    fn day_checkout(root: &Path, members: &[&str]) -> PathBuf {
        let list = members
            .iter()
            .map(|m| format!("\"crates/{m}\""))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::create_dir_all(root).expect("mkdir");
        std::fs::write(
            root.join("Cargo.toml"),
            format!("[workspace]\nresolver = \"2\"\nmembers = [{list}]\n"),
        )
        .expect("workspace");
        for m in members {
            let dir = root.join("crates").join(m);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(
                dir.join("Cargo.toml"),
                format!("[package]\nname = \"{m}\"\nversion = \"0.1.0\"\n"),
            )
            .expect("member");
        }
        root.to_path_buf()
    }

    /// The table names every git-sourced day dependency, and points each at the checkout. This is
    /// the text both lifetimes use — `day patch` writes it to `.cargo/config.toml`, `--day-src`
    /// hands it to one cargo run.
    #[test]
    fn the_table_maps_each_git_dep_to_the_checkout() {
        let tmp = std::env::temp_dir().join(format!("day-src-table-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let project = fixture(
            &tmp.join("app"),
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
day-build = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        let checkout = day_checkout(&tmp.join("day"), &["day", "day-build", "day-core"]);

        let (table, count) = patch_table(&project, &checkout, "# header\n").expect("table");
        assert_eq!(count, 2, "only the crates the app actually names");
        assert!(
            table.contains("[patch.\"https://github.com/daybrite/day.git\"]"),
            "{table}"
        );
        assert!(table.contains("day = { path ="), "{table}");
        assert!(table.contains("day-build = { path ="), "{table}");
        assert!(
            !table.contains("day-core = {"),
            "an unused entry would make cargo warn on every build: {table}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The wrong-branch case: a day-src that no longer carries a crate the app depends on must
    /// say so by name, not fail in the resolver several minutes later.
    #[test]
    fn a_checkout_missing_a_crate_names_it() {
        let tmp = std::env::temp_dir().join(format!("day-src-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let project = fixture(
            &tmp.join("app"),
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
day-part-http = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        let checkout = day_checkout(&tmp.join("day"), &["day"]);
        let err = patch_table(&project, &checkout, "").expect_err("day-part-http is absent");
        assert!(err.contains("day-part-http"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The slug keys the build tree. Two refs of the framework must not share one, and two
    /// spellings of one directory must not build twice.
    #[test]
    fn the_slug_is_readable_stable_and_distinct() {
        // `../day` and `/w/day` are the same checkout once resolved, so they share a build tree.
        let a = slug(Path::new("/w/day"));
        assert_eq!(a, slug(Path::new("/w/day")));
        assert!(a.starts_with("day-"), "{a} should stay readable");

        // A git day-src is cached per ref, so the directory name IS the branch.
        let nav = slug(Path::new("/c/git/github.com/daybrite/day/experimental-nav"));
        let fix = slug(Path::new("/c/git/github.com/daybrite/day/fix-482"));
        assert_ne!(nav, fix, "two branches, two build trees");
        assert!(nav.starts_with("experimental-nav-"), "{nav}");
        assert!(
            nav.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "{nav} has to be a directory name on every host"
        );
    }

    /// The flag promises to leave the project alone, and cargo rewrites Cargo.lock to record the
    /// patched sources. Both directions: a lock that existed comes back, one that did not is gone.
    #[test]
    fn the_lock_is_put_back() {
        let tmp = std::env::temp_dir().join(format!("day-src-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let lock = tmp.join("Cargo.lock");

        std::fs::write(&lock, b"original\n").expect("write");
        std::fs::write(&lock, b"rewritten by cargo\n").expect("write");
        restore(&lock, Some(b"original\n"));
        assert_eq!(std::fs::read(&lock).expect("read"), b"original\n");

        restore(&lock, None);
        assert!(
            !lock.exists(),
            "a lock that did not exist is not left behind"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// An app is named `day-<something>` too, so the `day` prefix cannot tell one from a
    /// framework crate. CI checks the showcase out INSIDE the day workspace, which put the app's
    /// own manifest under the checkout root — and `day patch` then demanded `day-showcase` be one
    /// of day's crates and failed every toolkit job.
    #[test]
    fn the_app_is_never_a_crate_to_patch() {
        let checkout = Path::new("/w/day");
        let app = Path::new("/w/day/showcase-src");

        // The app itself, nested inside the checkout the way CI arranges it.
        assert!(!needs_patch_entry(
            "day-showcase",
            Some("/w/day/showcase-src/Cargo.toml"),
            None,
            app,
            Some(checkout),
        ));
        // …and a local sub-crate of the app (Day-Matrix has one).
        assert!(!needs_patch_entry(
            "day-matrix-core",
            Some("/w/day/showcase-src/core/Cargo.toml"),
            None,
            app,
            Some(checkout),
        ));
        // A real framework crate from git still needs its entry.
        assert!(needs_patch_entry(
            "day-pieces",
            Some("/home/u/.cargo/git/checkouts/day-abc/1234/crates/day-pieces/Cargo.toml"),
            Some("git+https://github.com/daybrite/day.git#1234"),
            app,
            Some(checkout),
        ));
        // As does one already resolving from the checkout, or a re-run would shrink the table.
        assert!(needs_patch_entry(
            "day-core",
            Some("/w/day/crates/day-core/Cargo.toml"),
            None,
            app,
            Some(checkout),
        ));
        // Non-day packages are never in scope.
        assert!(!needs_patch_entry(
            "serde",
            Some("/home/u/.cargo/registry/src/serde/Cargo.toml"),
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            app,
            Some(checkout),
        ));
    }

    /// Which dependencies need a patch entry: the DIRECT ones from the day git repo, including the
    /// per-target tables where an app's backend crates live — the easiest ones to forget.
    #[test]
    fn git_deps_are_collected_from_every_dependency_table() {
        let tmp = std::env::temp_dir().join(format!("day-patch-deps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let project = fixture(
            &tmp,
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
day-part-http = { git = "https://github.com/daybrite/day.git" }
serde = "1"
day-local = { path = "../elsewhere" }

[target.'cfg(target_os = "android")'.dependencies]
day-part-battery = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        // The manifest floor alone (no resolver: this fixture has no lockfile and no network).
        let deps = manifest_git_deps(&project).expect("deps");
        assert_eq!(deps, ["day", "day-part-battery", "day-part-http"]);
        // A path dependency is already local, and a non-day crate is none of our business.
        assert!(!deps.iter().any(|d| d == "day-local" || d == "serde"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The manifest floor is a floor, not the answer: a toolkit crate reaches an app only
    /// through the umbrella, under a `cfg(target_os)` table nobody writes by hand, and patching
    /// just what the manifest names is what let a stale `day-android` build for months.
    #[test]
    fn the_manifest_floor_does_not_see_transitive_toolkits() {
        let tmp = std::env::temp_dir().join(format!("day-patch-transitive-{}", std::process::id()));
        let project = fixture(
            &tmp,
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        let deps = manifest_git_deps(&project).expect("deps");
        assert_eq!(deps, ["day"], "the manifest names one crate");
        assert!(
            !deps.iter().any(|d| d == "day-android"),
            "and the toolkit it pulls in per platform is invisible here — which is why \
             `git_deps` also resolves each platform's graph"
        );
        assert!(
            PATCH_TRIPLES.contains(&"aarch64-linux-android")
                && PATCH_TRIPLES.contains(&"wasm32-unknown-unknown"),
            "every platform Day ships has to be asked"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The checkout's crate map comes from its workspace, so a new framework crate needs no edit
    /// here — verified against this very repository.
    #[test]
    fn the_checkout_map_is_read_from_the_workspace() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root");
        let crates = checkout_crates(here).expect("this repo is a day checkout");
        assert!(crates.contains_key("day"), "the facade crate");
        assert!(crates.contains_key("day-cli"), "this crate");
        assert!(
            crates["day"].join("Cargo.toml").is_file(),
            "the mapped path is a real crate dir"
        );
    }
}
