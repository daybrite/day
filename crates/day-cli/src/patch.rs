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
        if !name.starts_with("day") {
            continue;
        }
        let from_git = pkg.get("source").and_then(|s| s.as_str()).is_some_and(|s| {
            s.trim_start_matches("git+")
                .starts_with(DAY_GIT.trim_end_matches(".git"))
        });
        // Already patched to the checkout: keep it, or a second `day patch` would shrink the
        // table it wrote the first time.
        let from_checkout = checkout.is_some_and(|root| {
            pkg.get("manifest_path")
                .and_then(|m| m.as_str())
                .is_some_and(|m| Path::new(m).starts_with(root))
        });
        if from_git || from_checkout {
            names.push(name.to_string());
        }
    }
    names
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

/// Write `.cargo/config.toml` mapping every git-sourced day dependency to the checkout.
fn write_patch(project: &Project, checkout: &Path) -> Result<usize, String> {
    let checkout = checkout
        .canonicalize()
        .map_err(|e| format!("{}: {e}", checkout.display()))?;
    let available = checkout_crates(&checkout)?;
    let wanted = git_deps(project, Some(&checkout))?;
    if wanted.is_empty() {
        return Err(
            "this project has no dependencies from the day git repository — nothing to patch \
             (a workspace member of the day repo needs no patch table)"
                .into(),
        );
    }

    let mut lines = String::from(
        "# Generated by `day patch` — machine-local, gitignored, and safe to delete.\n\
         # Builds this project against the day checkout below instead of fetching the git\n\
         # dependency. CI has no such file and resolves git, which is what a user's build does.\n",
    );
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

    let dir = project.root.join(".cargo");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("config.toml");
    std::fs::write(&path, lines).map_err(|e| format!("{}: {e}", path.display()))?;
    status(
        "Patched",
        &format!("{} ({written} crate(s))", path.display()),
    );
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
