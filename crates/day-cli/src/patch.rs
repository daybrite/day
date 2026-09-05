// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day patch` — build an app against a LOCAL checkout, or a FORK, of the crates it takes from git.
//!
//! An app outside this repository declares its framework dependencies from git:
//!
//! ```toml
//! day = { git = "https://github.com/daybrite/day.git" }
//! day-piece-lottie = { git = "https://github.com/daybrite/day-piece-lottie.git" }
//! ```
//!
//! which is what a user's app looks like and what CI should resolve. Two situations need those to
//! come from somewhere else, and Cargo's answer to both is a `[patch]` table:
//!
//! * **Developing the framework (or an external piece) and the app together.** The crates come
//!   from a checkout on disk: `day patch --local ../day --local ../day-piece-lottie`. The table is
//!   machine-local and gitignored.
//! * **Building against a fork.** Every day crate comes from another git repository:
//!   `day patch --git https://github.com/acme/day.git@acme`. Cargo applies the table to the WHOLE
//!   graph, so an external piece that depends on the canonical `daybrite/day` URL builds against
//!   the fork too, unchanged. That table is meant to be committed.
//!
//! Writing either table by hand is the problem this command exists to remove: it is a list of
//! entries that goes stale when a dependency is added, and a MISSING ENTRY DOES NOT FAIL. Cargo
//! simply resolves that crate from the git cache, and the build silently mixes a local (or forked)
//! framework with a published one — green, and testing something other than what you think.
//!
//! Only DIRECT dependencies of each source need an entry. A patched crate's own dependencies are
//! path deps inside the same checkout, so they follow automatically; `--check` is what proves
//! that, by asserting no package from a patched source still carries its git source.
//!
//! The one thing a `[patch]` cannot do is re-point a URL at ITSELF on another ref (cargo refuses:
//! "patches must point to different sources"). That is why external crates depend on the bare
//! canonical URL and let the app's `Cargo.lock` pick the revision, and why `--day-src` clones a
//! ref into a directory and patches to the PATH ([`resolve_day_src`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::CliError;
use crate::meta::Project;
use crate::ops::status;

/// The git URL an app's framework dependencies point at, and the key of the framework's `[patch]`
/// table. A checkout that carries the `day` crate stands for this URL whatever its manifest's
/// `repository` says, so a fork checked out locally still patches the canonical name.
pub(crate) const DAY_GIT: &str = "https://github.com/daybrite/day.git";

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

/// One source URL the table redirects, and where it goes.
pub struct Source {
    /// The URL the dependencies name, as they write it (`https://github.com/daybrite/day.git`).
    pub url: String,
    pub target: Target,
}

/// Where a patched source's crates come from instead.
pub enum Target {
    /// A checkout on disk: every crate the graph takes from the URL becomes a path dependency.
    Checkout(PathBuf),
    /// Another git repository: every crate becomes `{ git = <fork>, <branch|tag|rev> = <ref> }`.
    Fork {
        url: String,
        git_ref: Option<GitRef>,
    },
}

/// How a fork ref is spelled in the table. `--git URL@REF` guesses from the ref's shape; an
/// explicit `URL@tag=v1.2.0` / `@branch=x` / `@rev=<sha>` says so.
#[derive(Clone, Debug, PartialEq)]
pub enum GitRef {
    Branch(String),
    Tag(String),
    Rev(String),
}

impl GitRef {
    /// `tag=v1`, `branch=main`, `rev=<sha>`, or a bare ref: 40 hex digits are a commit, and
    /// everything else a branch. Tags are named explicitly, since `v1.2.0` and `release` are both
    /// plausible branch names and cargo fails a wrong guess with a fetch error minutes later.
    pub fn parse(spec: &str) -> Self {
        if let Some(rest) = spec.strip_prefix("tag=") {
            return GitRef::Tag(rest.to_string());
        }
        if let Some(rest) = spec.strip_prefix("branch=") {
            return GitRef::Branch(rest.to_string());
        }
        if let Some(rest) = spec.strip_prefix("rev=") {
            return GitRef::Rev(rest.to_string());
        }
        if spec.len() == 40 && spec.chars().all(|c| c.is_ascii_hexdigit()) {
            return GitRef::Rev(spec.to_string());
        }
        GitRef::Branch(spec.to_string())
    }

    fn key(&self) -> &'static str {
        match self {
            GitRef::Branch(_) => "branch",
            GitRef::Tag(_) => "tag",
            GitRef::Rev(_) => "rev",
        }
    }

    fn value(&self) -> &str {
        match self {
            GitRef::Branch(v) | GitRef::Tag(v) | GitRef::Rev(v) => v,
        }
    }
}

/// A git URL reduced to what cargo compares: no `git+` scheme prefix, no `?branch=…` query, no
/// `#<sha>` fragment, no trailing slash or `.git`, lowercase. Two dependencies on
/// `https://github.com/daybrite/day.git` and `https://github.com/daybrite/day` are one source to
/// cargo, and must be one source here.
pub(crate) fn canon(url: &str) -> String {
    let url = url.trim().trim_start_matches("git+");
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    url.to_ascii_lowercase()
}

/// The package's git source, canonicalized, when the resolved package is one this project may
/// patch — or `None` for anything else: the project's own packages, registry crates, path deps.
///
/// A package already patched to one of `checkouts` reports that checkout's URL, so a re-run keeps
/// the entry it wrote the first time instead of shrinking the table.
fn package_source(
    manifest_path: Option<&str>,
    source: Option<&str>,
    project_root: &Path,
    checkouts: &[(String, PathBuf)],
) -> Option<String> {
    // The project's OWN packages are never crates to patch, whatever they are called. CI checks
    // an app out INSIDE the day workspace (`day/showcase-src`), which puts the app's manifest
    // under the checkout root and made the checkout arm below claim it.
    if manifest_path.is_some_and(|m| Path::new(m).starts_with(project_root)) {
        return None;
    }
    if let Some(s) = source
        && s.starts_with("git+")
    {
        return Some(canon(s));
    }
    let manifest = manifest_path.map(Path::new)?;
    checkouts
        .iter()
        .find(|(_, root)| manifest.starts_with(root))
        .map(|(url, _)| url.clone())
}

/// Every git-sourced package in `project`'s resolved graph for one platform: name → canonical
/// source URL. Packages already patched to a checkout in `checkouts` count as that checkout's.
fn resolved_git_packages(
    root: &Path,
    triple: &str,
    checkouts: &[(String, PathBuf)],
) -> Vec<(String, String)> {
    let Ok(out) = Command::new("cargo")
        .current_dir(root)
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
    let mut found = Vec::new();
    for pkg in doc
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        let manifest_path = pkg.get("manifest_path").and_then(|m| m.as_str());
        let source = pkg.get("source").and_then(|s| s.as_str());
        if let Some(url) = package_source(manifest_path, source, root, checkouts) {
            found.push((name.to_string(), url));
        }
    }
    found
}

/// Every crate a checkout publishes: package name → absolute directory.
///
/// Read from the checkout's own workspace members rather than a hardcoded list, so a crate added
/// to the framework needs no change here. A single-crate repository (an external piece) is its
/// own one-entry map.
fn checkout_crates(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("{}: {e} — is that a checkout?", manifest.display()))?;
    let doc: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;

    let mut out = BTreeMap::new();
    if let Some(name) = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
    {
        out.insert(name.to_string(), root.to_path_buf());
    }
    let members = doc
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array());
    for m in members.into_iter().flatten().filter_map(|m| m.as_str()) {
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
        return Err(format!(
            "{}: neither a package nor a workspace with members",
            root.display()
        ));
    }
    Ok(out)
}

/// The git URL a checkout on disk stands for.
///
/// A checkout carrying the `day` crate is the framework, whatever its manifest says — a fork
/// cloned locally must patch the canonical URL the app's dependencies name, not the fork's own.
/// Anything else is identified by its manifest's `repository` (the package's, or the workspace's),
/// which is the URL its consumers depend on.
fn checkout_url(root: &Path, crates: &BTreeMap<String, PathBuf>) -> Result<String, String> {
    if crates.contains_key("day") {
        return Ok(DAY_GIT.to_string());
    }
    let manifest = root.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let doc: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let repo = doc
        .get("package")
        .and_then(|p| p.get("repository"))
        .and_then(|r| r.as_str())
        .or_else(|| {
            doc.get("workspace")
                .and_then(|w| w.get("package"))
                .and_then(|p| p.get("repository"))
                .and_then(|r| r.as_str())
        })
        .filter(|r| !r.trim().is_empty());
    match repo {
        Some(url) => Ok(url.trim().to_string()),
        None => Err(format!(
            "{}: cannot tell which git URL this checkout stands for — set `repository = \
             \"https://…\"` in its [package] (or [workspace.package]) to the URL apps depend on",
            root.display()
        )),
    }
}

/// The app's OWN git-sourced dependencies, read straight from its manifest: name → canonical
/// URL. The floor under [`wanted_by_source`]: it needs no resolver, so a project that cannot
/// resolve offline still patches the crates it names itself.
fn manifest_git_deps(root: &Path) -> Result<Vec<(String, String)>, String> {
    let manifest = root.join("Cargo.toml");
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
            if let Some(git) = spec.get("git").and_then(|g| g.as_str()) {
                let entry = (name.clone(), canon(git));
                if !names.contains(&entry) {
                    names.push(entry);
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

/// For each source, the crates the project takes from it — the manifest's own entries plus every
/// platform's resolved graph — keyed by canonical URL. Sources the graph never names map to an
/// empty list, which the table builder reports.
fn wanted_by_source(
    root: &Path,
    sources: &[Source],
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let checkouts: Vec<(String, PathBuf)> = sources
        .iter()
        .filter_map(|s| match &s.target {
            Target::Checkout(dir) => Some((canon(&s.url), dir.clone())),
            Target::Fork { .. } => None,
        })
        .collect();
    let mut wanted: BTreeMap<String, Vec<String>> = sources
        .iter()
        .map(|s| (canon(&s.url), Vec::new()))
        .collect();
    let mut add = |name: String, url: String| {
        if let Some(list) = wanted.get_mut(&url)
            && !list.contains(&name)
        {
            list.push(name);
        }
    };
    for (name, url) in manifest_git_deps(root)? {
        add(name, url);
    }
    for triple in PATCH_TRIPLES {
        for (name, url) in resolved_git_packages(root, triple, &checkouts) {
            add(name, url);
        }
    }
    for list in wanted.values_mut() {
        list.sort();
    }
    Ok(wanted)
}

/// The `[patch]` tables mapping every git-sourced dependency of `project` from each source to its
/// target, as TOML text — and the number of entries across them.
///
/// One text, two lifetimes: `day patch` writes it to `.cargo/config.toml` and every later build
/// picks it up, while `--day-src` writes it to a scratch file handed to ONE cargo invocation
/// through `--config`. The crate set, the missing-crate error, and the wording are therefore the
/// same for both, which is the point of computing it here.
///
/// `header` leads the file, since the lifetimes need to say different things about it.
fn patch_tables(root: &Path, sources: &[Source], header: &str) -> Result<(String, usize), String> {
    let wanted = wanted_by_source(root, sources)?;
    let mut lines = String::from(header);
    let mut written = 0;
    for source in sources {
        let names = wanted.get(&canon(&source.url)).cloned().unwrap_or_default();
        if names.is_empty() {
            return Err(format!(
                "this project has no dependencies from {} — nothing to patch there (a workspace \
                 member of that repository needs no patch table)",
                source.url
            ));
        }
        lines.push_str(&format!("[patch.{:?}]\n", source.url));
        match &source.target {
            Target::Checkout(checkout) => {
                let available = checkout_crates(checkout)?;
                let mut missing = Vec::new();
                for name in &names {
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
                        "{} is not in the checkout at {} — is it the right checkout, or the \
                         wrong branch?",
                        missing.join(", "),
                        checkout.display()
                    ));
                }
            }
            Target::Fork { url, git_ref } => {
                for name in &names {
                    match git_ref {
                        Some(r) => lines.push_str(&format!(
                            "{name} = {{ git = {url:?}, {} = {:?} }}\n",
                            r.key(),
                            r.value()
                        )),
                        None => lines.push_str(&format!("{name} = {{ git = {url:?} }}\n")),
                    }
                    written += 1;
                }
            }
        }
        lines.push('\n');
    }
    Ok((lines, written))
}

/// Write `.cargo/config.toml` mapping every git-sourced dependency of each source to its target.
fn write_patch(root: &Path, sources: &[Source]) -> Result<usize, String> {
    let forked = sources
        .iter()
        .any(|s| matches!(s.target, Target::Fork { .. }));
    let header = if forked {
        "# Generated by `day patch --git`. Builds this project against the fork below instead of\n\
         # the canonical repository its dependencies name; cargo applies the table to the whole\n\
         # graph, so external pieces follow the fork too. COMMIT this file to keep the fork (the\n\
         # scaffold's .gitignore lists it, since `day patch --local` writes machine-local paths\n\
         # here — remove that line first).\n"
    } else {
        "# Generated by `day patch` — machine-local, gitignored, and safe to delete.\n\
         # Builds this project against the checkout(s) below instead of fetching the git\n\
         # dependency. CI has no such file and resolves git, which is what a user's build does.\n"
    };
    let (lines, written) = patch_tables(root, sources, header)?;

    let dir = root.join(".cargo");
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
    if !forked && root.join("Cargo.lock").exists() {
        status(
            "Note",
            "cargo will rewrite Cargo.lock to point at this checkout — do not commit it while \
             patched (`git checkout -- Cargo.lock` before committing, or delete .cargo/config.toml \
             and regenerate the lock)",
        );
    }
    Ok(written)
}

/// The URLs a project's `.cargo/config.toml` patches — or the framework's, when there is no table
/// yet (so a bare `--check` still asks the question it always asked).
fn patched_urls(root: &Path) -> Vec<String> {
    let path = root.join(".cargo/config.toml");
    let keys = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|doc| {
            doc.get("patch")
                .and_then(|p| p.as_table())
                .map(|t| t.keys().map(|k| canon(k)).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    if keys.is_empty() {
        vec![canon(DAY_GIT)]
    } else {
        keys
    }
}

/// What [`check`] found: packages that should have been patched and were not, and packages from
/// git sources nothing patches.
#[derive(Default)]
pub struct CheckReport {
    /// `(name, url)`: from a patched URL, still resolving from git — a missing table entry.
    pub missing: Vec<(String, String)>,
    /// `(name, url)`: from a git URL the table does not cover — published as far as this build is
    /// concerned, which may or may not be what the developer wants.
    pub unpatched: Vec<(String, String)>,
}

/// Assert that no package from a patched source still comes from git.
///
/// The check the whole command exists for. A direct dependency without a patch entry resolves from
/// the git cache and builds green, so a build that believes it is testing this checkout may be
/// testing a published crate for part of the graph. Resolution is asked of cargo rather than read
/// out of `Cargo.lock`, so it is correct for a project that has not locked yet — and across EVERY
/// platform, not just the host's: a check that asks only the host says "all local" while the
/// Android, Linux, Windows, web, and HarmonyOS builds quietly use the git cache.
pub fn check(root: &Path) -> Result<CheckReport, String> {
    let patched = patched_urls(root);
    let mut report = CheckReport::default();
    let mut asked = false;
    for triple in PATCH_TRIPLES {
        let out = Command::new("cargo")
            .current_dir(root)
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
            let Some(source) = pkg
                .get("source")
                .and_then(|s| s.as_str())
                .filter(|s| s.starts_with("git+"))
            else {
                continue;
            };
            let url = canon(source);
            let entry = (name.to_string(), url.clone());
            let list = if patched.contains(&url) {
                &mut report.missing
            } else {
                &mut report.unpatched
            };
            if !list.contains(&entry) {
                list.push(entry);
            }
        }
    }
    if !asked {
        return Err("cargo metadata failed for every platform".into());
    }
    report.missing.sort();
    report.unpatched.sort();
    Ok(report)
}

/// `day patch [--local <checkout>]… [--git <url>[@<ref>]] [--check]`.
pub fn run(
    root: &Path,
    locals: &[PathBuf],
    git: Option<&str>,
    check_only: bool,
) -> Result<(), CliError> {
    let mut sources = Vec::new();
    for local in locals {
        let checkout = local
            .canonicalize()
            .map_err(|e| CliError::usage(format!("--local {}: {e}", local.display())))?;
        let crates = checkout_crates(&checkout).map_err(CliError::usage)?;
        let url = checkout_url(&checkout, &crates).map_err(CliError::usage)?;
        sources.push(Source {
            url,
            target: Target::Checkout(checkout),
        });
    }
    if let Some(arg) = git {
        let spec = crate::git::parse_spec(arg).map_err(CliError::usage)?;
        if !crate::git::looks_remote(&spec.url) {
            return Err(CliError::usage(format!(
                "--git {arg}: not a git URL — pass the fork like \
                 https://github.com/acme/day.git@<branch> (for a checkout on disk use --local)"
            )));
        }
        if canon(&spec.url) == canon(DAY_GIT) {
            return Err(CliError::usage(format!(
                "--git {arg}: that is the canonical day repository — cargo cannot patch a URL \
                 with itself on another ref (\"patches must point to different sources\"); to \
                 build against a branch or tag of it use `--day-src {arg}` for one build, or \
                 clone it and pass the checkout with --local"
            )));
        }
        sources.push(Source {
            url: DAY_GIT.to_string(),
            target: Target::Fork {
                url: spec.url,
                git_ref: spec.git_ref.as_deref().map(GitRef::parse),
            },
        });
    }
    if !sources.is_empty() {
        write_patch(root, &sources).map_err(CliError::failure)?;
    }
    match check(root) {
        Ok(report) if report.missing.is_empty() => {
            status("Verified", "every patched source resolves away from git");
            if !report.unpatched.is_empty() {
                let list = report
                    .unpatched
                    .iter()
                    .map(|(n, u)| format!("{n} ← {u}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                status(
                    "Note",
                    &format!(
                        "still from git, unpatched: {list} — add `--local <checkout>` for any \
                         you are developing"
                    ),
                );
            }
            Ok(())
        }
        Ok(report) => Err(CliError::failure(format!(
            "{} crate(s) still resolve from a patched source: {} — add them to the [patch] table \
             (`day patch --local <checkout>` / `--git <fork>` rewrites it) or this build mixes a \
             local framework with a published one",
            report.missing.len(),
            report
                .missing
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        Err(e) => {
            // Not fatal without --check: writing the table succeeded, and resolution may need the
            // network the caller does not have.
            status("Warning", &format!("could not verify resolution: {e}"));
            if check_only {
                Err(CliError::failure(format!(
                    "could not verify resolution: {e}"
                )))
            } else {
                Ok(())
            }
        }
    }
}

// --- One copy of each day crate --------------------------------------------------------------

/// Assert that the resolved graph carries exactly ONE copy of every day crate, and say which
/// consumers disagree when it does not.
///
/// Cargo unifies a git dependency only when URL and ref both match, so an app on the bare
/// canonical URL plus a piece that pins `tag = "v0.4.1"` resolves two `day-core`s — and the
/// failure that follows is a wall of "expected `day_core::Piece`, found `day_core::Piece`", or
/// worse, two `RENDERERS` slices and a widget that quietly renders as a placeholder. Asked of
/// cargo here, once, before anything compiles, so the answer is one sentence naming both sources.
///
/// Also the home of the `[package.metadata.day] compat = "X.Y"` note: an external crate says
/// which day minor it was built and tested against, and a mismatch is worth a line before the
/// build rather than a linker error after it.
pub fn verify_graph(project: &Project) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    apply_day_src(&mut cmd);
    let out = cmd
        .current_dir(&project.root)
        .args(["metadata", "--format-version", "1", "--all-features"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        // Resolution itself failing is the build's problem to report, with cargo's own words.
        return Ok(());
    }
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let packages: Vec<&serde_json::Value> = doc
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .collect();

    // name → every distinct (version, source) the graph resolved for it.
    let mut copies: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut day_version: Option<String> = None;
    for pkg in &packages {
        let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        if !name.starts_with("day") {
            continue;
        }
        let manifest = pkg.get("manifest_path").and_then(|m| m.as_str());
        if manifest.is_some_and(|m| Path::new(m).starts_with(&project.root)) {
            continue;
        }
        let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let source = match pkg.get("source").and_then(|s| s.as_str()) {
            Some(s) => s.to_string(),
            None => format!("path {}", manifest.unwrap_or("?")),
        };
        if name == "day" {
            day_version = Some(version.to_string());
        }
        copies
            .entry(name)
            .or_default()
            .push(format!("{version} from {source}"));
    }
    let doubled: Vec<(&str, &Vec<String>)> = copies
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(n, v)| (*n, v))
        .collect();
    if let Some((name, sources)) = doubled.first() {
        let more = if doubled.len() > 1 {
            format!(
                " (and {} more: {})",
                doubled.len() - 1,
                doubled[1..]
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        return Err(format!(
            "{name} is in the dependency graph {} times{more}:\n  {}\n\
             cargo unifies a git dependency only when its URL and ref match, so every crate that \
             depends on day must name the same source: the bare `git = \"{DAY_GIT}\"` with no \
             branch, tag, or rev (Cargo.lock pins the revision), or one `[patch]` table that \
             redirects the URL for the whole graph (`day patch --local <checkout>`, \
             `day patch --git <fork>`, or `--day-src` for one build)",
            sources.len(),
            sources.join("\n  "),
        ));
    }

    // The compat note: external crates declare the day minor they were tested against.
    if let Some(day_version) = day_version {
        let minor = |v: &str| v.splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
        let resolved = minor(&day_version);
        for pkg in &packages {
            let Some(compat) = pkg
                .get("metadata")
                .and_then(|m| m.get("day"))
                .and_then(|d| d.get("compat"))
                .and_then(|c| c.as_str())
            else {
                continue;
            };
            if minor(compat) != resolved {
                let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
                status(
                    "Note",
                    &format!(
                        "{name} declares compat = {compat:?} but this build resolves day \
                         {day_version}; it may need a newer release (or `cargo update`)"
                    ),
                );
            }
        }
    }
    Ok(())
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
    let source = Source {
        url: DAY_GIT.to_string(),
        target: Target::Checkout(src.checkout.clone()),
    };
    let (table, count) = patch_tables(
        &project.root,
        std::slice::from_ref(&source),
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

    /// A day checkout, as far as [`patch_tables`] and [`resolve_day_src`] are concerned: a
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

    /// An external piece checkout: ONE package, identified by its `repository`.
    fn piece_checkout(root: &Path, name: &str, repository: &str) -> PathBuf {
        std::fs::create_dir_all(root).expect("mkdir");
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nrepository = \"{repository}\"\n"
            ),
        )
        .expect("piece");
        root.to_path_buf()
    }

    fn checkout_source(url: &str, dir: &Path) -> Source {
        Source {
            url: url.to_string(),
            target: Target::Checkout(dir.to_path_buf()),
        }
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

        let (table, count) = patch_tables(
            &project.root,
            &[checkout_source(DAY_GIT, &checkout)],
            "# header\n",
        )
        .expect("table");
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

    /// Two sources, two tables: the framework and an external piece each patched to their own
    /// checkout, keyed by the URL the app's dependencies name.
    #[test]
    fn each_source_gets_its_own_table() {
        let tmp = std::env::temp_dir().join(format!("day-patch-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let project = fixture(
            &tmp.join("app"),
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
day-piece-lottie = { git = "https://github.com/daybrite/day-piece-lottie.git" }
"#,
        );
        let day = day_checkout(&tmp.join("day"), &["day"]);
        let lottie = piece_checkout(
            &tmp.join("lottie"),
            "day-piece-lottie",
            "https://github.com/daybrite/day-piece-lottie",
        );
        let sources = [
            checkout_source(DAY_GIT, &day),
            checkout_source("https://github.com/daybrite/day-piece-lottie.git", &lottie),
        ];
        let (table, count) = patch_tables(&project.root, &sources, "").expect("table");
        assert_eq!(count, 2, "{table}");
        assert!(
            table.contains("[patch.\"https://github.com/daybrite/day.git\"]\nday = { path ="),
            "{table}"
        );
        assert!(
            table.contains(
                "[patch.\"https://github.com/daybrite/day-piece-lottie.git\"]\nday-piece-lottie = { path ="
            ),
            "{table}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A fork is a git entry per crate, spelled with the ref kind the caller named or that the
    /// ref's shape implies — and the canonical URL stays the table's key, which is what lets an
    /// external piece depending on it follow the fork unchanged.
    #[test]
    fn a_fork_writes_git_entries_under_the_canonical_key() {
        let tmp = std::env::temp_dir().join(format!("day-patch-fork-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let project = fixture(
            &tmp.join("app"),
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        let fork = Source {
            url: DAY_GIT.to_string(),
            target: Target::Fork {
                url: "https://github.com/acme/day.git".into(),
                git_ref: Some(GitRef::Branch("acme".into())),
            },
        };
        let (table, count) = patch_tables(&project.root, &[fork], "").expect("table");
        assert_eq!(count, 1);
        assert!(
            table.contains("[patch.\"https://github.com/daybrite/day.git\"]"),
            "{table}"
        );
        assert!(
            table
                .contains("day = { git = \"https://github.com/acme/day.git\", branch = \"acme\" }"),
            "{table}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `--git URL@REF`: a commit is 40 hex digits, a tag has to be named, the rest is a branch.
    #[test]
    fn refs_are_spelled_the_way_cargo_wants_them() {
        assert_eq!(GitRef::parse("main"), GitRef::Branch("main".into()));
        assert_eq!(GitRef::parse("tag=v0.4.1"), GitRef::Tag("v0.4.1".into()));
        assert_eq!(
            GitRef::parse("branch=v0.4.1"),
            GitRef::Branch("v0.4.1".into())
        );
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(GitRef::parse(sha), GitRef::Rev(sha.into()));
        assert_eq!(GitRef::parse("rev=abc123"), GitRef::Rev("abc123".into()));
    }

    /// Cargo compares sources by canonical URL, so every spelling of one repository is one key.
    #[test]
    fn urls_canonicalize_the_way_cargo_compares_them() {
        let want = "https://github.com/daybrite/day";
        assert_eq!(canon("https://github.com/daybrite/day.git"), want);
        assert_eq!(canon("https://github.com/daybrite/day/"), want);
        assert_eq!(
            canon("git+https://github.com/daybrite/day.git?branch=main#abcdef"),
            want
        );
        assert_eq!(canon("https://GitHub.com/daybrite/day"), want);
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
        let err = patch_tables(&project.root, &[checkout_source(DAY_GIT, &checkout)], "")
            .expect_err("day-part-http is absent");
        assert!(err.contains("day-part-http"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A checkout with the `day` crate is the framework whatever its manifest says (a fork
    /// cloned locally still patches the canonical URL); anything else is named by `repository`,
    /// and a checkout that names nothing is refused rather than guessed.
    #[test]
    fn a_checkout_is_identified_by_what_it_carries() {
        let tmp = std::env::temp_dir().join(format!("day-patch-ident-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let day = day_checkout(&tmp.join("day"), &["day", "day-core"]);
        let crates = checkout_crates(&day).expect("workspace");
        assert_eq!(checkout_url(&day, &crates).expect("url"), DAY_GIT);

        let lottie = piece_checkout(
            &tmp.join("lottie"),
            "day-piece-lottie",
            "https://github.com/daybrite/day-piece-lottie",
        );
        let crates = checkout_crates(&lottie).expect("package");
        assert_eq!(crates.len(), 1, "a single-crate repository maps itself");
        assert_eq!(
            checkout_url(&lottie, &crates).expect("url"),
            "https://github.com/daybrite/day-piece-lottie"
        );

        let anon = piece_checkout(&tmp.join("anon"), "mystery", "");
        let crates = checkout_crates(&anon).expect("package");
        let err = checkout_url(&anon, &crates).expect_err("no repository");
        assert!(err.contains("repository"), "{err}");
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

    /// An app is named `day-<something>` too, so a name cannot tell one from a framework crate.
    /// CI checks the showcase out INSIDE the day workspace, which put the app's own manifest
    /// under the checkout root — and `day patch` then demanded `day-showcase` be one of day's
    /// crates and failed every toolkit job.
    #[test]
    fn the_app_is_never_a_crate_to_patch() {
        let checkout = Path::new("/w/day");
        let app = Path::new("/w/day/showcase-src");
        let checkouts = [(canon(DAY_GIT), checkout.to_path_buf())];

        // The app itself, nested inside the checkout the way CI arranges it.
        assert_eq!(
            package_source(
                Some("/w/day/showcase-src/Cargo.toml"),
                None,
                app,
                &checkouts
            ),
            None
        );
        // …and a local sub-crate of the app (Day-Matrix has one).
        assert_eq!(
            package_source(
                Some("/w/day/showcase-src/core/Cargo.toml"),
                None,
                app,
                &checkouts
            ),
            None
        );
        // A real framework crate from git reports the framework's URL.
        assert_eq!(
            package_source(
                Some("/home/u/.cargo/git/checkouts/day-abc/1234/crates/day-pieces/Cargo.toml"),
                Some("git+https://github.com/daybrite/day.git#1234"),
                app,
                &checkouts,
            )
            .as_deref(),
            Some("https://github.com/daybrite/day")
        );
        // As does one already resolving from the checkout, or a re-run would shrink the table.
        assert_eq!(
            package_source(
                Some("/w/day/crates/day-core/Cargo.toml"),
                None,
                app,
                &checkouts
            )
            .as_deref(),
            Some("https://github.com/daybrite/day")
        );
        // An external piece from its own repository reports THAT URL.
        assert_eq!(
            package_source(
                Some("/home/u/.cargo/git/checkouts/lottie-1/9/Cargo.toml"),
                Some("git+https://github.com/daybrite/day-piece-lottie.git#9"),
                app,
                &checkouts,
            )
            .as_deref(),
            Some("https://github.com/daybrite/day-piece-lottie")
        );
        // Registry crates are never in scope.
        assert_eq!(
            package_source(
                Some("/home/u/.cargo/registry/src/serde/Cargo.toml"),
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                app,
                &checkouts,
            ),
            None
        );
    }

    /// Which dependencies need a patch entry: the DIRECT ones from git, including the per-target
    /// tables where an app's backend crates live — the easiest ones to forget — each with the
    /// source it comes from.
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
day-piece-lottie = { git = "https://github.com/daybrite/day-piece-lottie.git" }
serde = "1"
day-local = { path = "../elsewhere" }

[target.'cfg(target_os = "android")'.dependencies]
day-part-battery = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        // The manifest floor alone (no resolver: this fixture has no lockfile and no network).
        let deps = manifest_git_deps(&project.root).expect("deps");
        let day = "https://github.com/daybrite/day".to_string();
        assert_eq!(
            deps,
            [
                ("day".to_string(), day.clone()),
                ("day-part-battery".to_string(), day.clone()),
                ("day-part-http".to_string(), day),
                (
                    "day-piece-lottie".to_string(),
                    "https://github.com/daybrite/day-piece-lottie".to_string()
                ),
            ]
        );
        // A path dependency is already local, and a non-day crate is none of our business.
        assert!(!deps.iter().any(|(d, _)| d == "day-local" || d == "serde"));
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
        let deps = manifest_git_deps(&project.root).expect("deps");
        assert_eq!(deps.len(), 1, "the manifest names one crate");
        assert!(
            !deps.iter().any(|(d, _)| d == "day-android"),
            "and the toolkit it pulls in per platform is invisible here — which is why \
             `wanted_by_source` also resolves each platform's graph"
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
