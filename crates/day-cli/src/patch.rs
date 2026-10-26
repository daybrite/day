// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day patch`: build an app against a local checkout, a fork, or the `main` branch of the crates
//! it takes from crates.io (or from git).
//!
//! An app outside this repository declares its framework dependencies by version:
//!
//! ```toml
//! day = { package = "day-app", version = "0.6" }
//! day-piece-lottie = "0.4"
//! ```
//!
//! which is what a user's app looks like and what a scheduled CI run resolves. Three situations
//! need those to come from somewhere else, and Cargo's answer to each is a `[patch]` table:
//!
//! * **Developing the framework (or an external piece) and the app together.** The crates come
//!   from a checkout on disk: `day patch --local ../day --local ../day-piece-lottie`. The table is
//!   machine-local and gitignored.
//! * **Building against the unreleased framework.** `day patch --main` points every Day crate in
//!   the graph at the head of its repository's default branch, pinned to that commit. CI runs it
//!   on a push, so an app is tested against the `main` it will meet in the next release.
//! * **Building against a fork.** `day patch --git https://github.com/acme/day.git@acme` takes
//!   every crate of that repository from the fork. Cargo applies the table to the whole graph, so
//!   an external piece that depends on the published crates builds against the fork too,
//!   unchanged. That table is meant to be committed.
//!
//! Writing any of these tables by hand is the problem this command exists to remove: it is a list
//! of entries that goes stale when a dependency is added, and a missing entry does not fail. Cargo
//! resolves that crate from crates.io, and the build silently mixes a local (or forked) framework
//! with a published one: green, and testing something other than what you think.
//!
//! A crates.io table starts with every crate the source carries, because a graph that needs an
//! unpublished version cannot be resolved to ask which ones it uses; the command then resolves
//! under the table and drops the entries cargo reported unused. `--check` proves the result, by
//! asserting no package from a patched repository still comes from crates.io (or from its git
//! URL).
//!
//! An entry only takes part when its version satisfies what the graph asks for. A checkout whose
//! version is older than the newest release, or a `main` that has moved to the next minor while
//! the app still asks for this one, is passed over without an error; `--check` names both cases.
//!
//! Apps that still depend on day from git get the same treatment under a table keyed by the git
//! URL. The one thing such a table cannot do is re-point a URL at itself on another ref (cargo
//! refuses: "patches must point to different sources"), which is why `--day-src` clones a ref into
//! a directory and patches to the PATH ([`resolve_day_src`]).
//!
//! Both tables also exist for one run only: `day launch --day-src <path|url>` takes a day
//! checkout or ref, and `--patch-local <checkout>` takes anything `day patch --local` would, and
//! both hand the same table to that run's cargo through `--config` ([`resolve_run_patch`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::CliError;
use crate::meta::Project;
use crate::ops::status;

/// The git URL an app's framework dependencies point at, and the key of the framework's `[patch]`
/// table. A checkout that carries the `day` crate stands for this URL whatever its manifest's
/// `repository` says, so a fork checked out locally still patches the canonical name.
pub(crate) const DAY_GIT: &str = "https://github.com/daybrite/day.git";

/// The umbrella crate's package name. Apps depend on it under the key `day`, its library name
/// (`day = { package = "day-app", … }`); a patch table names the package.
pub(crate) const DAY_PACKAGE: &str = "day-app";

/// The `[patch]` key for crates.io, which every published Day crate comes from.
const CRATES_IO: &str = "crates-io";

/// Where Day's own repositories live. `--main` follows a crate to its repository's default branch
/// only when crates.io names a repository here, so a third-party `day-*` crate is left alone.
const DAY_ORG: &str = "https://github.com/daybrite/";

/// One triple per platform Day targets. A day crate reaches an app through the umbrella and the
/// parts, under `[target.'cfg(…)'.dependencies]` tables the app never names itself, so the set to
/// patch is the resolved graph of every platform, not the app's manifest. Resolving for the
/// host alone is what let `day-toolkit-material` build from the git cache while a local checkout sat
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

/// One repository the table redirects, and where its crates go.
pub struct Source {
    /// The repository the crates stand for, as their consumers name it
    /// (`https://github.com/daybrite/day.git`): the key of a git-sourced table, and the
    /// `repository` crates.io records for each of its published crates.
    pub url: String,
    pub target: Target,
    /// Whether the caller named this source (`--local`, `--git`, `--day-src`). One `--main` found
    /// by itself is dropped quietly when the graph takes nothing from it, since an optional
    /// dependency's repository is reached through a manifest that a feature never turns on.
    pub named: bool,
}

/// Where a patched source's crates come from instead.
pub enum Target {
    /// A checkout on disk: every crate the graph takes from the URL becomes a path dependency.
    Checkout(PathBuf),
    /// Another git repository: every crate becomes `{ git = <fork>, <branch|tag|rev> = <ref> }`.
    /// `checkout` is a shallow copy of that ref, which says which crates the repository carries.
    Fork {
        url: String,
        git_ref: Option<GitRef>,
        checkout: PathBuf,
    },
}

impl Source {
    /// The directory on disk that carries this source's crates.
    fn dir(&self) -> &Path {
        match &self.target {
            Target::Checkout(dir) | Target::Fork { checkout: dir, .. } => dir,
        }
    }

    /// How the source reads in a status line or an error: the directory, or `url @ ref`.
    fn label(&self) -> String {
        match &self.target {
            Target::Checkout(dir) => dir.display().to_string(),
            Target::Fork { url, git_ref, .. } => match git_ref {
                Some(r) => format!("{url} @ {}", r.value()),
                None => url.clone(),
            },
        }
    }

    /// The table entry that takes `name` from this source: `{ path = … }` or `{ git = …, … }`.
    fn entry(&self, name: &str, dir: &Path) -> String {
        match &self.target {
            Target::Checkout(_) => {
                format!("{name} = {{ path = {:?} }}\n", dir.display().to_string())
            }
            Target::Fork { url, git_ref, .. } => match git_ref {
                Some(r) => format!(
                    "{name} = {{ git = {url:?}, {} = {:?} }}\n",
                    r.key(),
                    r.value()
                ),
                None => format!("{name} = {{ git = {url:?} }}\n"),
            },
        }
    }
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

/// Is `manifest` inside `root`, comparing paths that may differ only in Windows' verbatim prefix?
///
/// `Path::canonicalize` returns `\\?\C:\…` on Windows (which is what a checkout root is),
/// while cargo reports plain `C:\…` manifest paths. `Path::starts_with` matches prefix
/// components, and `Prefix::VerbatimDisk` never equals `Prefix::Disk`, so the raw comparison is
/// always false between the two forms. That only bites on a re-run, once the crates already
/// resolve from the checkout as path deps: `source` is no longer `git+…`, so the path is the only
/// thing left to recognize them by, and every transitive crate silently drops out of the table.
fn within(manifest: &Path, root: &Path) -> bool {
    fn plain(p: &Path) -> PathBuf {
        let text = p.as_os_str().to_string_lossy();
        match text.strip_prefix(r"\\?\") {
            Some(rest) => PathBuf::from(rest),
            None => p.to_path_buf(),
        }
    }
    manifest.starts_with(root) || plain(manifest).starts_with(plain(root))
}

/// The package's git source, canonicalized, when the resolved package is one this project may
/// patch, or `None` for anything else: the project's own packages, registry crates, path deps.
///
/// A package already patched to one of `checkouts` reports that checkout's URL, so a re-run keeps
/// the entry it wrote the first time instead of shrinking the table.
fn package_source(
    manifest_path: Option<&str>,
    source: Option<&str>,
    project_root: &Path,
    checkouts: &[(String, PathBuf)],
) -> Option<String> {
    // The project's packages are never crates to patch, whatever they are called. CI checks
    // an app out inside the day workspace (`day/showcase-src`), which puts the app's manifest
    // under the checkout root and made the checkout arm below claim it.
    if manifest_path.is_some_and(|m| within(Path::new(m), project_root)) {
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
        .find(|(_, root)| within(manifest, root))
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
            // Backends are optional dependencies behind per-toolkit features, so the default
            // resolve sees none of them: an app's gtk build would take day-toolkit-gtk from the git
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
/// to the framework needs no change here. Also include independent packages in the same Git
/// checkout when their repository metadata identifies the same source (for example galleries).
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
    if let Ok(url) = checkout_url(root, &out)
        && let Ok(files) = Command::new("git")
            .args([
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ])
            .current_dir(root)
            .output()
        && files.status.success()
    {
        for name in files.stdout.split(|b| *b == 0) {
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            if !name.ends_with("/Cargo.toml") {
                continue;
            }
            let path = root.join(name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = toml::from_str::<toml::Value>(&text) else {
                continue;
            };
            let Some(package) = doc.get("package") else {
                continue;
            };
            let Some(repo) = package.get("repository").and_then(|v| v.as_str()) else {
                continue;
            };
            if canon(repo) != canon(&url) {
                continue;
            }
            let Some(name) = package.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(dir) = path.parent() else { continue };
            if let Some(previous) = out.insert(name.to_string(), dir.to_path_buf())
                && previous != dir
            {
                return Err(format!(
                    "duplicate package {name} in {} and {}",
                    previous.display(),
                    dir.display()
                ));
            }
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

/// Every crate a checkout publishes: package name → version as its manifest declares it (a member
/// on `version.workspace = true` takes the workspace's).
fn checkout_versions(root: &Path) -> BTreeMap<String, String> {
    let read = |dir: &Path| -> Option<toml::Value> {
        toml::from_str(&std::fs::read_to_string(dir.join("Cargo.toml")).ok()?).ok()
    };
    let mut out = BTreeMap::new();
    let Some(ws) = read(root) else {
        return out;
    };
    let inherited = ws
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let named = |doc: &toml::Value| -> Option<(String, String)> {
        let pkg = doc.get("package")?;
        let name = pkg.get("name")?.as_str()?.to_string();
        let version = match pkg.get("version")? {
            toml::Value::String(v) => v.clone(),
            _ => inherited.clone()?,
        };
        Some((name, version))
    };
    if let Some((name, version)) = named(&ws) {
        out.insert(name, version);
    }
    let members = ws
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array());
    for m in members.into_iter().flatten().filter_map(|m| m.as_str()) {
        if let Some(doc) = read(&root.join(m))
            && let Some((name, version)) = named(&doc)
        {
            out.insert(name, version);
        }
    }
    out
}

/// Why a patch that was written is absent from the graph, when the reason is cargo's version rule.
///
/// A `[patch]` entry is one more candidate, not an override, and it takes part only when its
/// version is one the graph accepts. Two ways that fails, both silently: the source carries a
/// LOWER version than the newest release (or the git URL's tip), and cargo keeps the newest it can
/// see; or it carries a version the project's requirement does not accept, such as a `main` that
/// moved to the next minor while the app still asks for this one. `None` when the versions agree,
/// which leaves the plain missing-entry explanation standing.
fn outranked(sources: &[Source], missing: &[(String, String, String)]) -> Option<String> {
    sources.iter().find_map(|source| {
        let ours_all = checkout_versions(source.dir());
        let url = canon(&source.url);
        let (name, _, theirs) = missing
            .iter()
            .find(|(n, u, v)| *u == url && ours_all.get(n).is_some_and(|ours| ours != v))?;
        let ours = &ours_all[name];
        if newer(ours, theirs) {
            return Some(format!(
                "{} carries {name} v{ours}, which the version this project asks for does not \
                 accept (the graph resolved v{theirs} from the published crates); raise the \
                 requirement in Cargo.toml so it takes v{ours}",
                source.label()
            ));
        }
        Some(match &source.target {
            Target::Checkout(dir) => format!(
                "cargo keeps the newest version it can see, and the published crates offer \
                 {name} v{theirs} where the checkout at {} has v{ours}; bring the checkout up to \
                 date (git pull), or lock the app to the checkout's version first \
                 (`cargo update -p {name} --precise {ours}`) and run `day patch` again",
                dir.display()
            ),
            Target::Fork { .. } => format!(
                "cargo keeps the newest version it can see, and the published crates offer \
                 {name} v{theirs} where {} has v{ours}; the release is ahead of that ref",
                source.label()
            ),
        })
    })
}

/// `a > b`, comparing the numeric `major.minor.patch` core and ignoring any pre-release or build
/// suffix. Unparseable input compares as "not newer".
fn newer(a: &str, b: &str) -> bool {
    let core = |v: &str| -> Option<(u64, u64, u64)> {
        let v = v.split(['-', '+']).next()?;
        let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
        Some((it.next()??, it.next()??, it.next()??))
    };
    matches!((core(a), core(b)), (Some(x), Some(y)) if x > y)
}

/// The git URL a checkout on disk stands for.
///
/// A checkout carrying the `day-app` crate is the framework, whatever its manifest says: a fork
/// cloned locally must patch the canonical URL the app's dependencies name, not the fork's own.
/// Anything else is identified by its manifest's `repository` (the package's, or the workspace's),
/// which is the URL its consumers depend on.
fn checkout_url(root: &Path, crates: &BTreeMap<String, PathBuf>) -> Result<String, String> {
    if crates.contains_key(DAY_PACKAGE) {
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

/// The app's git-sourced dependencies, read straight from its manifest: name → canonical
/// URL. The floor under [`wanted_by_source`]: it needs no resolver, so a project that cannot
/// resolve offline still patches the crates it names itself.
fn manifest_git_deps(root: &Path) -> Result<Vec<(String, String)>, String> {
    let manifest = root.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let doc: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut names = Vec::new();
    // Plain dependencies plus every `[target.<cfg>.dependencies]` table: the backend crates an
    // app pulls in per platform live there, and they are exactly the ones easiest to forget.
    let tables = dependency_tables(&doc);
    for table in tables {
        let Some(table) = table.as_table() else {
            continue;
        };
        for (key, spec) in table {
            if let Some(git) = spec.get("git").and_then(|g| g.as_str()) {
                // A patch entry names the package, which `package =` renames away from the key.
                let name = spec.get("package").and_then(|p| p.as_str()).unwrap_or(key);
                let entry = (name.to_string(), canon(git));
                if !names.contains(&entry) {
                    names.push(entry);
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

/// For each source, the crates the project takes from it (the manifest's own entries plus every
/// platform's resolved graph), keyed by canonical URL. Sources the graph never names map to an
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
    // A crate already resolving from a checkout counts as taken from that checkout's git URL only
    // on a re-run of a git-sourced patch: the project's table keys that URL today, and one of the
    // project's own manifests still names it. A crate there through the crates.io table came
    // from crates.io, and a table left from before the project moved to crates.io is stale;
    // claiming either for git would write a second table nothing uses.
    let keyed = git_keys(&root.join(".cargo/config.toml"));
    let mut named: Vec<String> = manifest_git_deps(root)?
        .into_iter()
        .map(|(_, u)| u)
        .collect();
    if let Ok(own) = checkout_crates(root) {
        for dir in own.values() {
            named.extend(
                manifest_git_deps(dir)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(_, u)| u),
            );
        }
    }
    let rerun: Vec<(String, PathBuf)> = checkouts
        .iter()
        .filter(|(url, _)| keyed.contains(url) && named.contains(url))
        .cloned()
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
    // A coordinated rename may not resolve against Git at all yet. Read the selected local
    // consumers too, so a renamed transitive dependency is patched before the first resolve.
    for (_, checkout) in &checkouts {
        let packages = checkout_crates(checkout)?;
        for directory in packages.values().chain(std::iter::once(checkout)) {
            for (name, url) in manifest_git_deps(directory)? {
                add(name, url);
            }
        }
    }
    for triple in PATCH_TRIPLES {
        for (name, url) in resolved_git_packages(root, triple, &rerun) {
            add(name, url);
        }
    }
    for list in wanted.values_mut() {
        list.sort();
    }
    Ok(wanted)
}

/// The `[patch]` tables for `sources`, as TOML text, and the number of entries across them, before
/// any resolve has said which crates.io entries go unused: what [`settle`] writes first.
#[cfg(test)]
fn patch_tables(root: &Path, sources: &[Source], header: &str) -> Result<(String, usize), String> {
    let wanted = wanted_by_source(root, sources)?;
    render(sources, &wanted, header, &BTreeSet::new())
}

/// The tables themselves, from what [`wanted_by_source`] found.
///
/// Two kinds. The crates the project takes from a source's git URL get a table keyed by that
/// URL, listing exactly those. The crates it takes from crates.io cannot be listed that way: a
/// graph that asks for a version crates.io does not have yet does not resolve without the table,
/// so there is no graph to ask. The crates.io table therefore lists every crate the source
/// carries, less `unused`, the ones a resolve under the full table left out.
fn render(
    sources: &[Source],
    wanted: &BTreeMap<String, Vec<String>>,
    header: &str,
    unused: &BTreeSet<String>,
) -> Result<(String, usize), String> {
    let mut lines = String::from(header);
    let mut registry = String::new();
    let mut written = 0;
    // Which source each crates.io entry came from: cargo rejects a key written twice.
    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    for source in sources {
        let available = checkout_crates(source.dir())?;
        let names = wanted.get(&canon(&source.url)).cloned().unwrap_or_default();
        if !names.is_empty() {
            if let Target::Fork { url, .. } = &source.target
                && canon(url) == canon(&source.url)
            {
                return Err(format!(
                    "this project takes {} from git at {url}, and cargo cannot patch a URL with \
                     itself on another ref (\"patches must point to different sources\"); depend \
                     on the published crates instead (`{} = \"…\"`), or clone the ref and pass \
                     the checkout with --local",
                    names.join(", "),
                    names[0]
                ));
            }
            lines.push_str(&format!("[patch.{:?}]\n", source.url));
            let mut missing = Vec::new();
            for name in &names {
                match available.get(name) {
                    Some(dir) => {
                        lines.push_str(&source.entry(name, dir));
                        written += 1;
                    }
                    None => missing.push(name.clone()),
                }
            }
            if missing.iter().any(|n| n == "day") && available.contains_key(DAY_PACKAGE) {
                return Err(format!(
                    "{} names its umbrella `{DAY_PACKAGE}`, but this graph still depends on a \
                     package called `day`: the app's manifest needs \
                     `day = {{ package = \"{DAY_PACKAGE}\", … }}`, and a piece or part from git \
                     that still says `day` needs that change too (patch its checkout with \
                     another --local)",
                    source.label()
                ));
            }
            if !missing.is_empty() {
                let place = match &source.target {
                    Target::Checkout(dir) => format!("the checkout at {}", dir.display()),
                    Target::Fork { .. } => source.label(),
                };
                return Err(format!(
                    "{} is not in {place} — is it the right checkout, or the wrong branch?",
                    missing.join(", ")
                ));
            }
            lines.push('\n');
        }
        let mut published = 0;
        for (name, dir) in &available {
            if unused.contains(name) {
                continue;
            }
            if let Some(other) = owner.insert(name.clone(), source.label()) {
                return Err(format!(
                    "{name} is in both {other} and {} — a crate can be patched to one place; \
                     drop one of them",
                    source.label()
                ));
            }
            registry.push_str(&source.entry(name, dir));
            published += 1;
        }
        written += published;
        if names.is_empty() && published == 0 && source.named {
            return Err(format!(
                "this project takes nothing from {} — nothing to patch there (a workspace member \
                 of that repository needs no patch table)",
                source.url
            ));
        }
    }
    if !registry.is_empty() {
        lines.push_str(&format!("[patch.{CRATES_IO}]\n"));
        lines.push_str(&registry);
        lines.push('\n');
    }
    Ok((lines, written))
}

/// Write the tables for `sources` to `config`, resolve the project under them, and write them
/// again without the crates.io entries cargo reported unused. Returns the entries left.
///
/// One text, two lifetimes: `day patch` writes it to `.cargo/config.toml` and every later build
/// picks it up, while `--day-src` writes it to a scratch file handed to one cargo invocation
/// through `--config`. The crate set, the missing-crate error, and the wording are therefore the
/// same for both, which is why it is computed here; `header` leads the file, since the lifetimes
/// need to say different things about it.
///
/// An entry cargo passed over for a crate the graph still takes from crates.io is kept, not
/// dropped: that is the version rule at work (an outranked checkout, a requirement the patched
/// version does not meet), and [`check`] names it. Dropping it would hide exactly that.
///
/// `one_run` hands the file to cargo with `--config`, as `--day-src` builds do; otherwise it is the
/// project's `.cargo/config.toml`, which cargo finds by itself.
fn settle(
    root: &Path,
    sources: &[Source],
    header: &str,
    config: &Path,
    one_run: bool,
) -> Result<usize, String> {
    let wanted = wanted_by_source(root, sources)?;
    let (table, written) = render(sources, &wanted, header, &BTreeSet::new())?;
    std::fs::write(config, table).map_err(|e| format!("{}: {e}", config.display()))?;
    let flag = one_run.then_some(config);
    // A lock that holds a patched crate at another version keeps it over the table.
    let urls = git_keys(config);
    relock(root, &urls, &registry_entries(config), flag)?;
    if let Err(e) = resolve(root, flag) {
        status(
            "Warning",
            &format!(
                "could not resolve under the table, so it keeps every crate the sources carry \
                 (cargo warns about each one it does not use): {e}"
            ),
        );
        return Ok(written);
    }
    // `cargo metadata --all-features` turns on the project's own features only. A piece's
    // backend feature that `day build` adds (`<piece>/<toolkit>`) can bring in an optional crate
    // that resolution never saw, so a crate the manifests name stays in the table.
    let named = registry_deps(root);
    let unused: BTreeSet<String> = unused_entries(root)
        .into_iter()
        .filter(|n| !named.contains(n))
        .collect();
    if unused.is_empty() {
        return Ok(written);
    }
    let (table, written) = render(sources, &wanted, header, &unused)?;
    std::fs::write(config, table).map_err(|e| format!("{}: {e}", config.display()))?;
    // Once more, so the lock forgets the `[[patch.unused]]` records of the entries just dropped.
    resolve(root, flag)?;
    Ok(written)
}

/// Resolve the project's whole graph (every platform, every feature), which also brings
/// `Cargo.lock` up to date with the patch table in effect.
fn resolve(root: &Path, config: Option<&Path>) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["metadata", "--format-version", "1", "--all-features"]);
    if let Some(cfg) = config {
        cmd.arg("--config").arg(cfg);
    }
    let out = cmd.output().map_err(|e| format!("cargo metadata: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The crates.io entries `Cargo.lock` records as unused that the graph does not take from
/// crates.io either: the crates of a patched repository this project never reaches.
fn unused_entries(root: &Path) -> BTreeSet<String> {
    let Some(doc) = read_lock(root) else {
        return BTreeSet::new();
    };
    let names = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(|p| p.as_array())
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .collect()
    };
    let published: BTreeSet<String> = doc
        .get("package")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter(|p| {
            p.get("source")
                .and_then(|s| s.as_str())
                .is_some_and(from_registry)
        })
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();
    names(doc.get("patch").and_then(|p| p.get("unused")))
        .into_iter()
        .filter(|n| !published.contains(n))
        .collect()
}

/// Whether a lock or metadata `source` is a registry (crates.io, by either protocol).
fn from_registry(source: &str) -> bool {
    source.starts_with("registry+") || source.starts_with("sparse+")
}

fn read_lock(root: &Path) -> Option<toml::Value> {
    toml::from_str(&std::fs::read_to_string(root.join("Cargo.lock")).ok()?).ok()
}

/// Write `.cargo/config.toml` mapping every dependency of each source to its target.
fn write_patch(root: &Path, sources: &[Source], header: &str) -> Result<usize, String> {
    let dir = root.join(".cargo");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("config.toml");
    let written = settle(root, sources, header, &path, false)?;
    status(
        "Patched",
        &format!("{} ({written} crate(s))", path.display()),
    );
    // The config file itself is gitignored, but its effect on Cargo.lock is not: cargo has
    // rewritten the lock to record these paths, dropping the `source` line from every day crate.
    // Committed, that lock describes a machine nobody else has. Said here because nothing else
    // will say it before the commit.
    let local = sources
        .iter()
        .any(|s| matches!(s.target, Target::Checkout(_)));
    if local && root.join("Cargo.lock").exists() {
        status(
            "Note",
            "Cargo.lock now points at the checkout — do not commit it while patched \
             (`git checkout -- Cargo.lock` before committing, or delete .cargo/config.toml and \
             regenerate the lock)",
        );
    }
    Ok(written)
}

/// The git URLs a project's `.cargo/config.toml` patches, or the framework's, when there is no
/// table at all (so a bare `--check` still asks the question it always asked).
fn patched_urls(root: &Path) -> Vec<String> {
    let config = root.join(".cargo/config.toml");
    let keys = git_keys(&config);
    if keys.is_empty() && registry_entries(&config).is_empty() {
        vec![canon(DAY_GIT)]
    } else {
        keys
    }
}

/// The URLs a cargo config file's git-keyed `[patch]` tables redirect, canonical; empty when there
/// is no such file or table.
fn git_keys(config: &Path) -> Vec<String> {
    patch_doc(config)
        .and_then(|doc| {
            doc.get("patch").and_then(|p| p.as_table()).map(|t| {
                t.keys()
                    .filter(|k| k.as_str() != CRATES_IO)
                    .map(|k| canon(k))
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default()
}

/// The `[patch.crates-io]` entries of a cargo config file: name → entry.
fn registry_entries(config: &Path) -> BTreeMap<String, toml::Value> {
    patch_doc(config)
        .and_then(|doc| {
            doc.get("patch")
                .and_then(|p| p.get(CRATES_IO))
                .and_then(|t| t.as_table())
                .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        })
        .unwrap_or_default()
}

fn patch_doc(config: &Path) -> Option<toml::Value> {
    toml::from_str(&std::fs::read_to_string(config).ok()?).ok()
}

/// The repositories a `[patch.crates-io]` table redirects, canonical: the URL of each git entry,
/// and the repository each path entry's checkout stands for ([`checkout_url`]).
fn patched_repositories(config: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut tops: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in registry_entries(config).values() {
        if let Some(git) = entry.get("git").and_then(|g| g.as_str()) {
            out.insert(canon(git));
        } else if let Some(path) = entry.get("path").and_then(|p| p.as_str()) {
            let dir = Path::new(path);
            // The checkout is the nearest directory carrying `.git`; a crate directory alone may
            // not say which repository it belongs to, since its `repository` can be inherited.
            let top = dir
                .ancestors()
                .find(|a| a.join(".git").exists())
                .unwrap_or(dir)
                .to_path_buf();
            if tops.insert(top.clone())
                && let Ok(crates) = checkout_crates(&top)
                && let Ok(url) = checkout_url(&top, &crates)
            {
                out.insert(canon(&url));
            }
        }
    }
    out
}

// --- The lock ----------------------------------------------------------------------------------

/// Packages `Cargo.lock` still takes from one of `urls`, or from crates.io under one of the names
/// in `registry`: name, version, and the source as a package-id spec cargo accepts (a git URL with
/// its query and fragment removed, or the registry's own source string).
///
/// A `[patch]` entry is a candidate, not an order: when the lock already holds the crate from its
/// source at some other version, cargo keeps what is locked and warns that the patch "was not used
/// in the crate graph", then compiles the locked copy, and an app written against the checkout
/// fails to compile against a framework it never asked for. Cargo's own remedy is
/// `cargo update -p <pkg>` for those packages, which is what [`relock`] runs.
fn stale_in_lock(
    root: &Path,
    urls: &[String],
    registry: &BTreeMap<String, toml::Value>,
) -> Vec<(String, String, String)> {
    let Some(doc) = read_lock(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for pkg in doc
        .get("package")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        let field = |k: &str| pkg.get(k).and_then(|v| v.as_str());
        let (Some(name), Some(version), Some(source)) =
            (field("name"), field("version"), field("source"))
        else {
            continue;
        };
        if from_registry(source) {
            if registry.contains_key(name) {
                out.push((name.to_string(), version.to_string(), source.to_string()));
            }
            continue;
        }
        let Some(git) = source.strip_prefix("git+") else {
            continue;
        };
        if !urls.contains(&canon(git)) {
            continue;
        }
        let bare = git.split_once('#').map_or(git, |(u, _)| u);
        let bare = bare.split_once('?').map_or(bare, |(u, _)| u);
        out.push((name.to_string(), version.to_string(), bare.to_string()));
    }
    out
}

/// Relock whatever the lock still takes from a patched source, so the table applies to this build.
///
/// Only those packages, by full package id: the rest of the lock (every other crates.io version
/// the app has) stays as it is, so the build differs from an unpatched one by the patched crates
/// alone. Offline first, since the packages move onto paths and nothing needs fetching; a cold
/// index is the one reason that fails, and the online run covers it. `config` is the one-run table
/// handed to cargo with `--config`; `None` reads the project's `.cargo/config.toml` as a build
/// would. Returns how many packages moved.
pub(crate) fn relock(
    root: &Path,
    urls: &[String],
    registry: &BTreeMap<String, toml::Value>,
    config: Option<&Path>,
) -> Result<usize, String> {
    let stale = stale_in_lock(root, urls, registry);
    if stale.is_empty() {
        return Ok(0);
    }
    let specs: Vec<String> = stale
        .iter()
        .map(|(name, version, url)| format!("{url}#{name}@{version}"))
        .collect();
    let run = |offline: bool| -> Result<(), String> {
        let mut cmd = Command::new("cargo");
        cmd.current_dir(root).arg("update");
        if offline {
            cmd.arg("--offline");
        }
        if let Some(cfg) = config {
            cmd.arg("--config").arg(cfg);
        }
        for spec in &specs {
            cmd.arg("-p").arg(spec);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("could not run cargo update: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    if let Err(offline) = run(true)
        && let Err(online) = run(false)
    {
        return Err(format!(
            "cargo update -p {} failed: {online} (offline: {offline})",
            specs.join(" -p ")
        ));
    }
    status(
        "Relocked",
        &format!(
            "{} crate(s) Cargo.lock still took from their source, onto the patch",
            stale.len()
        ),
    );
    Ok(stale.len())
}

/// What [`check`] found: packages that should have been patched and were not, and packages from
/// Day's sources nothing patches.
#[derive(Default)]
pub struct CheckReport {
    /// `(name, repository, version)`: from a patched repository, still resolving from git or
    /// crates.io; a missing table entry, or a patch cargo passed over for the version rule (see
    /// [`outranked`]).
    pub missing: Vec<(String, String, String)>,
    /// `(name, url)`: from a git URL the table does not cover; published as far as this build is
    /// concerned, which may or may not be what the developer wants.
    pub unpatched: Vec<(String, String)>,
    /// `(name, repository)`: Day crates from crates.io whose repository the table does not cover,
    /// when it covers some; the published side of a build that mixes the two.
    pub published: Vec<(String, String)>,
}

/// Assert that no package from a patched source still comes from git or crates.io.
///
/// The check the whole command exists for. A crate without a patch entry resolves from crates.io
/// (or the git cache) and builds green, so a build that believes it is testing this checkout may
/// be testing a published crate for part of the graph. Resolution is asked of cargo rather than
/// read out of `Cargo.lock`, so it is correct for a project that has not locked yet, and across
/// every platform rather than only the host's: a check that asks only the host says "all local"
/// while the Android, Linux, Windows, web, and HarmonyOS builds quietly use the published crates.
///
/// A crates.io package counts as patched-but-missed when the table names it, or when crates.io
/// records a repository the table patches: a crate the checkout no longer carries is the same
/// miss.
pub fn check(root: &Path) -> Result<CheckReport, String> {
    let patched = patched_urls(root);
    let config = root.join(".cargo/config.toml");
    let named = registry_entries(&config);
    let repos = patched_repositories(&config);
    let org = canon(DAY_ORG);
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
            let version = pkg
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let Some(source) = pkg.get("source").and_then(|s| s.as_str()) else {
                continue;
            };
            if from_registry(source) {
                let repo = pkg
                    .get("repository")
                    .and_then(|r| r.as_str())
                    .map(canon)
                    .unwrap_or_default();
                if named.contains_key(name) || repos.contains(&repo) {
                    let shown = if repo.is_empty() {
                        CRATES_IO.into()
                    } else {
                        repo
                    };
                    let entry = (name.to_string(), shown, version.to_string());
                    if !report.missing.contains(&entry) {
                        report.missing.push(entry);
                    }
                } else if !repos.is_empty() && repo.starts_with(&format!("{org}/")) {
                    let entry = (name.to_string(), repo);
                    if !report.published.contains(&entry) {
                        report.published.push(entry);
                    }
                }
                continue;
            }
            if !source.starts_with("git+") {
                continue;
            }
            let url = canon(source);
            // Where the crates.io table sends its git entries: the patch itself, not a miss.
            if repos.contains(&url) && named.contains_key(name) {
                continue;
            }
            if patched.contains(&url) {
                let entry = (name.to_string(), url, version.to_string());
                if !report.missing.contains(&entry) {
                    report.missing.push(entry);
                }
            } else {
                let entry = (name.to_string(), url);
                if !report.unpatched.contains(&entry) {
                    report.unpatched.push(entry);
                }
            }
        }
    }
    if !asked {
        return Err("cargo metadata failed for every platform".into());
    }
    report.missing.sort();
    report.unpatched.sort();
    report.published.sort();
    Ok(report)
}

/// The source a `--local` / `--patch-local` checkout stands for: the URL its crates are depended
/// on by ([`checkout_url`]), redirected to the directory. `flag` names the option in errors.
fn local_source(local: &Path, flag: &str) -> Result<Source, CliError> {
    let checkout = local
        .canonicalize()
        .map_err(|e| CliError::usage(format!("{flag} {}: {e}", local.display())))?;
    let crates = checkout_crates(&checkout)
        .map_err(|e| CliError::usage(format!("{flag} {}: {e}", local.display())))?;
    let url = checkout_url(&checkout, &crates)
        .map_err(|e| CliError::usage(format!("{flag} {}: {e}", local.display())))?;
    Ok(Source {
        url,
        target: Target::Checkout(checkout),
        named: true,
    })
}

/// The source a `--git <url>[@<ref>]` stands for: the repository the fork's crates are published
/// from (`daybrite/day` for a fork of the framework, the `repository` its manifest names
/// otherwise), redirected to the fork. The ref is fetched shallowly to learn which crates it
/// carries.
fn fork_source(arg: &str) -> Result<Source, CliError> {
    let spec = crate::git::parse_spec(arg).map_err(CliError::usage)?;
    if !crate::git::looks_remote(&spec.url) {
        return Err(CliError::usage(format!(
            "--git {arg}: not a git URL — pass the repository like \
             https://github.com/acme/day.git@<branch> (for a checkout on disk use --local)"
        )));
    }
    let git_ref = spec.git_ref.as_deref().map(GitRef::parse);
    let (checkout, _) = shallow(&spec.url, git_ref.as_ref()).map_err(CliError::failure)?;
    let crates =
        checkout_crates(&checkout).map_err(|e| CliError::usage(format!("--git {arg}: {e}")))?;
    let url = checkout_url(&checkout, &crates)
        .map_err(|e| CliError::usage(format!("--git {arg}: {e}")))?;
    Ok(Source {
        url,
        target: Target::Fork {
            url: spec.url,
            git_ref,
            checkout,
        },
        named: true,
    })
}

/// A shallow copy of `url` at `git_ref` (the default branch when `None`) in the CLI's cache, and
/// the commit it holds.
///
/// Only the manifests are read from it, to learn which crates the repository carries; cargo
/// fetches the source itself. One commit deep and refetched every time, so it always says what the
/// ref holds now, and a repository with a long history costs one tree.
fn shallow(url: &str, git_ref: Option<&GitRef>) -> Result<(PathBuf, String), String> {
    let mut dir = crate::git::cache_dir()?.join("patch");
    for segment in crate::git::repo_slug(url)? {
        dir.push(segment);
    }
    let refspec = match git_ref {
        None => "HEAD".to_string(),
        Some(GitRef::Tag(t)) => format!("refs/tags/{t}"),
        Some(r) => r.value().to_string(),
    };
    dir.push(match git_ref {
        None => "HEAD".to_string(),
        Some(r) => r.value().replace('/', "-"),
    });
    let git = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git")
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .map_err(|e| format!("git: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    };
    status(
        "Fetching",
        &match git_ref {
            Some(r) => format!("{url} @ {}", r.value()),
            None => url.to_string(),
        },
    );
    if !dir.join(".git").is_dir() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        git(&["init", "-q"])?;
        git(&["remote", "add", "origin", url])?;
    }
    git(&["fetch", "-q", "--depth", "1", "origin", &refspec])
        .map_err(|e| format!("could not fetch {url} @ {refspec}: {e}"))?;
    git(&["checkout", "-q", "--force", "FETCH_HEAD"])?;
    // An earlier commit's files would count as packages of this one.
    git(&["clean", "-q", "-f", "-d", "-x"])?;
    let sha = git(&["rev-parse", "HEAD"])?;
    Ok((dir, sha))
}

/// The repository crates.io records for `name`, or `None` when crates.io has no such crate.
fn crates_io_repository(name: &str) -> Result<Option<String>, String> {
    let ua = format!(
        "day-cli/{} (+{})",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY"),
    );
    let resp = ureq::get(format!("https://crates.io/api/v1/crates/{name}"))
        .header("User-Agent", &ua)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .call();
    let body = match resp {
        Ok(r) => r
            .into_body()
            .read_to_string()
            .map_err(|e| format!("crates.io {name}: {e}"))?,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(format!("crates.io {name}: {e}")),
    };
    let doc: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("crates.io {name}: {e}"))?;
    Ok(doc
        .get("crate")
        .and_then(|c| c.get("repository"))
        .and_then(|r| r.as_str())
        .filter(|r| !r.trim().is_empty())
        .map(|r| r.trim().trim_end_matches('/').to_string()))
}

/// The `day-*` crates a checkout's manifests ask crates.io for: plain versions, in every
/// dependency table of each package [`manifest_dirs`] reads.
fn registry_deps(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for dir in manifest_dirs(root) {
        let Some(doc) = patch_doc(&dir.join("Cargo.toml")) else {
            continue;
        };
        for table in dependency_tables(&doc) {
            for (key, spec) in table.as_table().into_iter().flatten() {
                let name = match spec {
                    toml::Value::String(_) => key.as_str(),
                    toml::Value::Table(t)
                        if !t.contains_key("git")
                            && !t.contains_key("path")
                            && !t.contains_key("workspace") =>
                    {
                        t.get("package").and_then(|p| p.as_str()).unwrap_or(key)
                    }
                    _ => continue,
                };
                if name.starts_with("day-") {
                    out.insert(name.to_string());
                }
            }
        }
    }
    out
}

/// The packages whose manifests say what a checkout takes from crates.io: the root package, its
/// workspace, every package [`checkout_crates`] counts as the checkout's, and every package any
/// of those reaches through a `path` dependency. A piece's demo reaches the piece itself that way
/// (`path = ".."`), and the piece's own requirements are part of the demo's graph.
fn manifest_dirs(root: &Path) -> Vec<PathBuf> {
    let mut queue = vec![root.to_path_buf()];
    if let Ok(crates) = checkout_crates(root) {
        queue.extend(crates.into_values());
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    while let Some(dir) = queue.pop() {
        if !seen.insert(dir.canonicalize().unwrap_or_else(|_| dir.clone())) {
            continue;
        }
        let Some(doc) = patch_doc(&dir.join("Cargo.toml")) else {
            continue;
        };
        for table in dependency_tables(&doc) {
            for (_, spec) in table.as_table().into_iter().flatten() {
                if let Some(path) = spec.get("path").and_then(|p| p.as_str()) {
                    queue.push(dir.join(path));
                }
            }
        }
        out.push(dir);
    }
    out
}

/// Every dependency table of a manifest: the workspace's, the package's three, and each
/// `[target.<cfg>]` set.
fn dependency_tables(doc: &toml::Value) -> Vec<&toml::Value> {
    let mut tables: Vec<&toml::Value> = Vec::new();
    if let Some(dependencies) = doc.get("workspace").and_then(|w| w.get("dependencies")) {
        tables.push(dependencies);
    }
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
    tables
}

/// Fetches a repository's default branch: the checkout, and the commit it holds.
type Fetch<'a> = dyn FnMut(&str) -> Result<(PathBuf, String), String> + 'a;

/// Looks up the repository crates.io records for a crate.
type Lookup<'a> = dyn FnMut(&str) -> Result<Option<String>, String> + 'a;

/// The repositories `--main` follows, each at the head of its default branch.
///
/// Starting from the crates the project asks crates.io for, every `day-*` crate that no source in
/// `given` carries is looked up on crates.io; a repository under [`DAY_ORG`] is fetched, its
/// crates count as covered, and its own manifests are read the same way, so a piece that depends
/// on another piece brings that one's repository too. A crate crates.io has never heard of is left
/// for the resolver to report. `lookup` and `fetch` are [`crates_io_repository`] and [`shallow`]
/// outside tests.
fn upstream_sources(
    root: &Path,
    given: &[Source],
    lookup: &mut Lookup,
    fetch: &mut Fetch,
) -> Result<Vec<Source>, String> {
    let org = canon(DAY_ORG);
    let mut covered: BTreeSet<String> = BTreeSet::new();
    let mut followed: BTreeSet<String> = given.iter().map(|s| canon(&s.url)).collect();
    let mut queue: Vec<PathBuf> = vec![root.to_path_buf()];
    for source in given {
        covered.extend(checkout_crates(source.dir())?.into_keys());
        queue.push(source.dir().to_path_buf());
    }
    let mut asked: BTreeSet<String> = BTreeSet::new();
    let mut found = Vec::new();
    while let Some(dir) = queue.pop() {
        for name in registry_deps(&dir) {
            if covered.contains(&name) || !asked.insert(name.clone()) {
                continue;
            }
            let Some(repo) = lookup(&name)? else {
                continue;
            };
            if !canon(&repo).starts_with(&format!("{org}/")) || !followed.insert(canon(&repo)) {
                continue;
            }
            let (checkout, sha) = fetch(&repo)?;
            covered.extend(checkout_crates(&checkout)?.into_keys());
            queue.push(checkout.clone());
            status(
                "Following",
                &format!("{repo} @ {}", &sha[..sha.len().min(12)]),
            );
            found.push(Source {
                url: repo.clone(),
                target: Target::Fork {
                    url: repo,
                    git_ref: Some(GitRef::Rev(sha)),
                    checkout,
                },
                named: false,
            });
        }
    }
    Ok(found)
}

/// One table per URL: cargo rejects a `[patch."…"]` key written twice, and the second checkout
/// would otherwise lose silently to the first in a table that merged them. Named now, as the two
/// flags that collide, rather than as cargo's duplicate-key parse error against a generated file.
fn distinct(sources: &[Source]) -> Result<(), String> {
    for (i, a) in sources.iter().enumerate() {
        if let Some(b) = sources[..i].iter().find(|b| canon(&b.url) == canon(&a.url)) {
            return Err(format!(
                "{} and {} both stand for {} — a source can be patched to one place; drop one \
                 of them",
                b.label(),
                a.label(),
                a.url
            ));
        }
    }
    Ok(())
}

/// `day patch [--local <checkout>]… [--git <url>[@<ref>]]… [--main] [--check]`.
pub fn run(
    root: &Path,
    locals: &[PathBuf],
    gits: &[String],
    main: bool,
    check_only: bool,
) -> Result<(), CliError> {
    let mut sources = Vec::new();
    for local in locals {
        sources.push(local_source(local, "--local")?);
    }
    for arg in gits {
        sources.push(fork_source(arg)?);
    }
    distinct(&sources).map_err(CliError::usage)?;
    if main {
        let upstream = upstream_sources(
            root,
            &sources,
            &mut |name| crates_io_repository(name),
            &mut |url| shallow(url, None),
        )
        .map_err(CliError::failure)?;
        if upstream.is_empty() && sources.is_empty() {
            return Err(CliError::usage(
                "--main: this project asks crates.io for no Day crate — nothing to follow",
            ));
        }
        sources.extend(upstream);
    }
    if !sources.is_empty() {
        let local = sources
            .iter()
            .any(|s| matches!(s.target, Target::Checkout(_)));
        let header = if main {
            "# Generated by `day patch --main`. Builds this project against the head of each Day\n\
             # repository's default branch, pinned to the commit it had when this was written,\n\
             # instead of the published crates. Run it again to move on; CI writes it on a push.\n"
        } else if local {
            "# Generated by `day patch` — machine-local, gitignored, and safe to delete.\n\
             # Builds this project against the checkout(s) below instead of the published crates.\n\
             # CI has no such file and resolves crates.io, which is what a user's build does.\n"
        } else {
            "# Generated by `day patch --git`. Builds this project against the fork below instead\n\
             # of the published crates; cargo applies the table to the whole graph, so external\n\
             # pieces follow the fork too. COMMIT this file to keep the fork (the scaffold's\n\
             # .gitignore lists it, since `day patch --local` writes machine-local paths here —\n\
             # remove that line first).\n"
        };
        write_patch(root, &sources, header).map_err(CliError::failure)?;
    }
    match check(root) {
        Ok(report) if report.missing.is_empty() => {
            status(
                "Verified",
                "every patched source resolves away from crates.io and git",
            );
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
            if !report.published.is_empty() {
                let mut by_repo: BTreeMap<&str, usize> = BTreeMap::new();
                for (_, repo) in &report.published {
                    *by_repo.entry(repo.as_str()).or_default() += 1;
                }
                let list = by_repo
                    .iter()
                    .map(|(r, n)| format!("{n} from {r}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                status(
                    "Note",
                    &format!(
                        "still from crates.io: {list} — add `--local <checkout>` for any you are \
                         developing"
                    ),
                );
            }
            Ok(())
        }
        Ok(report) => {
            let names = report
                .missing
                .iter()
                .map(|(n, _, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let why = outranked(&sources, &report.missing).unwrap_or_else(|| {
                "add them to the [patch] table (`day patch --local <checkout>` / `--git <fork>` \
                 / `--main` rewrites it) or this build mixes a patched framework with a \
                 published one"
                    .to_string()
            });
            Err(CliError::failure(format!(
                "{} crate(s) still resolve from a patched source's published copy: {names} — {why}",
                report.missing.len()
            )))
        }
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

/// Assert that the resolved graph carries exactly one copy of every day crate, and say which
/// consumers disagree when it does not.
///
/// Cargo unifies a git dependency only when URL and ref both match, so an app on the bare
/// canonical URL plus a piece that pins `tag = "v0.4.1"` resolves two `day-core`s, and the
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
        if name == DAY_PACKAGE {
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
             every crate that depends on day must take it from the same source: the published \
             crates (a version requirement, which Cargo.lock pins), or one `[patch]` table that \
             redirects them for the whole graph (`day patch --local <checkout>`, `--main`, \
             `--git <fork>`, or `--day-src` for one build). A git dependency unifies only when \
             its URL and ref both match, so a piece that still takes day from git (\
             `git = \"{DAY_GIT}\"`) brings a second copy",
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
// it. `--day-src` answers a different question ("does this branch fix the bug?") where you build
// the app twice, against two versions of the framework, and look at both. So the table it computes
// never reaches the project: it goes to a scratch file that one cargo invocation reads through
// `--config`, and the run leaves nothing behind.

/// Where the day-src build tree lives, for the process that must find it without the flag.
///
/// The Apple builds run cargo from `day xcode-backend build`, a separate process xcodebuild calls
/// back into. It learns the day-src the way it already learns the day binary (`DAY_BIN`): one
/// variable naming the directory, from which the cargo config and the build root both derive.
pub const DAY_SRC_DIR_ENV: &str = "DAY_SRC_DIR";

/// This run's day-src build directory, once resolved. A process global for the same reason
/// `--verbose` is one ([`crate::ops::verbose`]): it is a fact about the whole run, and threading it
/// through `build` → `build_native` → each platform's builder would touch every caller of a
/// function whose behavior does not otherwise change.
static ACTIVE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// A resolved `--day-src`: a day checkout on disk and how it was named.
pub struct DaySrc {
    /// The day checkout to build against, absolute.
    pub checkout: PathBuf,
    /// How it was named, for the status line: a path, or `url @ ref`.
    pub label: String,
}

/// Resolve a `--day-src` argument to a day checkout on disk.
///
/// An existing directory is that checkout. Anything else must be a git URL, cloned into the same
/// per-URL-and-ref cache `--git` uses ([`crate::git`]), so two branches of the framework coexist as
/// two checkouts and switching between them stays incremental.
pub fn resolve_day_src(arg: &str) -> Result<DaySrc, CliError> {
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
    if !crates.contains_key(DAY_PACKAGE) {
        return Err(CliError::usage(format!(
            "--day-src {}: a cargo workspace, but not a day checkout (no `day-app` crate in it) — \
             for a piece or part checkout use --patch-local",
            checkout.display()
        )));
    }

    Ok(DaySrc { checkout, label })
}

/// The `[patch]` table one `build`/`launch` runs under: what `--day-src` and every `--patch-local`
/// resolved to, and the build tree that holds it.
pub struct RunPatch {
    /// The day-src first, when there is one, then each `--patch-local` in the order given.
    pub sources: Vec<Source>,
    /// How each source was named, in the same order, for the status line.
    pub labels: Vec<String>,
    /// `build/day/day-src/<slug>`: this run's build root, and where the cargo config is written.
    pub dir: PathBuf,
}

/// Resolve `--day-src` and `--patch-local` together into this run's table, or `None` when
/// neither was given.
///
/// One table because cargo takes one: a day checkout from `--day-src` and a piece checkout from
/// `--patch-local` are two sources in it, exactly as `day patch --local ../day --local ../piece`
/// writes them. A `--patch-local` that names a day checkout is the same source `--day-src` would
/// resolve, so the two spellings share a build tree; naming it both ways is refused, the way two
/// `--local`s for one URL are ([`distinct`]).
pub fn resolve_run_patch(
    day_src: Option<&str>,
    locals: &[PathBuf],
    project: &Project,
) -> Result<Option<RunPatch>, CliError> {
    let mut sources = Vec::new();
    let mut labels = Vec::new();
    if let Some(arg) = day_src {
        let src = resolve_day_src(arg)?;
        sources.push(Source {
            url: DAY_GIT.to_string(),
            target: Target::Checkout(src.checkout),
            named: true,
        });
        labels.push(src.label);
    }
    for local in locals {
        let source = local_source(local, "--patch-local")?;
        labels.push(source.label());
        sources.push(source);
    }
    if sources.is_empty() {
        return Ok(None);
    }
    distinct(&sources).map_err(CliError::usage)?;
    let checkouts: Vec<PathBuf> = sources
        .iter()
        .filter_map(|s| match &s.target {
            Target::Checkout(dir) => Some(dir.clone()),
            Target::Fork { .. } => None,
        })
        .collect();
    let dir = project
        .root
        .join("build/day/day-src")
        .join(slug(&checkouts));
    Ok(Some(RunPatch {
        sources,
        labels,
        dir,
    }))
}

/// The build-tree name for one run's patch: a readable stem plus a hash of what it resolved to.
///
/// Both come from the resolved checkouts rather than what was typed, which gets each of them right
/// at once. The stem is the first directory's name; for a git day-src that is the ref, since the
/// cache is keyed by one (`…/daybrite/day/experimental-nav`). The hash keeps two branches apart
/// when their stems collide, makes `../day` and the absolute path it points at share one tree
/// instead of building the same framework twice, and covers every checkout, so the framework
/// alone and the framework plus a patched piece are two trees. A single checkout hashes exactly
/// as it did before `--patch-local`, so no existing build tree is orphaned.
fn slug(checkouts: &[PathBuf]) -> String {
    let stem = checkouts
        .first()
        .and_then(|c| c.file_name())
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
        let joined = checkouts
            .iter()
            .map(|c| c.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        h.update(joined.as_bytes());
        h.finalize()
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    format!("{stem}-{digest}")
}

/// Compute the patch table, write it under the run's build directory, and make it this run's.
///
/// Called once, from the `build`/`launch` dispatch. Everything downstream reads [`ACTIVE`] (or, in
/// the xcode-backend process, [`DAY_SRC_DIR_ENV`]) and needs no argument of its own.
pub fn activate(run: &RunPatch, project: &Project) -> Result<(), CliError> {
    // Computing the table resolves the graph, and any resolve may rewrite the lock (cargo drops
    // the `[[patch.unused]]` records a failed build left, say). Snapshot-only: the table is not
    // in effect yet, so there is nothing to relock onto.
    let _lock = LockGuard::snapshot(project);
    std::fs::create_dir_all(&run.dir)
        .map_err(|e| CliError::failure(format!("{}: {e}", run.dir.display())))?;
    let file = config_path(&run.dir);
    let count = settle(
        &project.root,
        &run.sources,
        "# Generated by `day launch` / `day build` with --day-src or --patch-local, for ONE build.\n\
         # Handed to cargo with `--config`; nothing in the project is modified, and the next\n\
         # build without the flag resolves the published crates as usual.\n",
        &file,
        true,
    )
    .map_err(CliError::failure)?;

    // An app that is already `day patch`ed has a `[patch]` table of its own in
    // `.cargo/config.toml`. Cargo ranks a `--config` argument above a config file, so this run
    // wins, but silently swapping the framework under someone who patched their project would be
    // the wrong kind of surprise.
    if project.root.join(".cargo/config.toml").is_file() {
        crate::ops::status(
            "Note",
            "this project has a `day patch` table; --day-src / --patch-local override it for \
             this build only",
        );
    }
    status(
        "Patched",
        &format!("{count} crate(s) → {}", run.labels.join(" + ")),
    );
    let _ = ACTIVE.set(run.dir.clone());
    Ok(())
}

/// The cargo config a day-src build directory carries.
fn config_path(dir: &Path) -> PathBuf {
    dir.join("cargo-patch.toml")
}

/// This run's day-src build directory, or `None` when `--day-src` was not given.
///
/// Falls back to the environment so `day xcode-backend build` (a separate process, spawned by
/// xcodebuild, which never sees the flag) resolves the same directory the porcelain did.
pub fn day_src_dir() -> Option<PathBuf> {
    ACTIVE
        .get()
        .cloned()
        .or_else(|| std::env::var_os(DAY_SRC_DIR_ENV).map(PathBuf::from))
}

/// This run's day-src as a short name: the build tree's directory name.
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
/// variable, the route `DAY_BIN` already travels. `None` when `--day-src` was not given, so the
/// argument is not added.
pub fn day_src_setting() -> Option<String> {
    day_src_dir().map(|d| format!("{DAY_SRC_DIR_ENV}={}", d.display()))
}

/// Point one cargo invocation at the day-src, if there is one.
///
/// Every command that resolves or compiles the app's graph must carry this, compiles and
/// `cargo metadata` alike: a metadata call that resolves without the patch reports the wrong
/// feature union, and the app renders `⟨kind⟩` placeholders for every optional piece.
pub fn apply_day_src(cmd: &mut Command) {
    if let Some(dir) = day_src_dir() {
        cmd.arg("--config").arg(config_path(&dir));
    }
}

/// Cargo records the patched sources in `Cargo.lock`, so a build under `--day-src` or
/// `--patch-local` would leave the project's lockfile rewritten: a tracked file, modified by a
/// flag whose promise is that it changes nothing. This snapshots the lock and puts it back.
///
/// It also brings the lock onto the patch first ([`relock`]), since a lock that holds the
/// framework from git at another version would otherwise win over the table and the build would
/// quietly compile the git copy. That relock runs once per process and its result is replayed by
/// every later guard, so the second target of a launch pays no resolver run for it.
///
/// Scoped to the build, not the launch: the app may run for a long time afterwards, and the lock
/// should be correct again the moment the compiler is done with it. Restoring costs no rebuild;
/// the next run re-resolves to the same graph, and the fingerprints live in the day-src's own
/// target directory.
pub struct LockGuard {
    path: PathBuf,
    before: Option<Vec<u8>>,
}

/// The lock as it stands once relocked onto this run's patch, replayed by each guard after the
/// first (`None` until then, or when nothing needed relocking).
static RELOCKED: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

impl LockGuard {
    /// Take the snapshot alone: the lock comes back as it was, and nothing is relocked.
    fn snapshot(project: &Project) -> Self {
        let path = project.root.join("Cargo.lock");
        let before = std::fs::read(&path).ok();
        crate::signals::register_restore(&path, before.as_deref());
        LockGuard { path, before }
    }

    /// Take the snapshot and bring the lock onto the patch. A no-op (and no guard work on drop)
    /// when neither `--day-src` nor `--patch-local` is in effect.
    pub fn new(project: &Project) -> Result<Option<Self>, CliError> {
        let Some(dir) = day_src_dir() else {
            return Ok(None);
        };
        let guard = LockGuard::snapshot(project);

        let mut relocked = RELOCKED.lock().unwrap_or_else(|e| e.into_inner());
        match relocked.as_ref() {
            Some(bytes) => {
                std::fs::write(&guard.path, bytes)
                    .map_err(|e| CliError::failure(format!("{}: {e}", guard.path.display())))?;
            }
            None => {
                let config = config_path(&dir);
                let urls = git_keys(&config);
                let moved = relock(
                    &project.root,
                    &urls,
                    &registry_entries(&config),
                    Some(&config),
                )
                .map_err(CliError::failure)?;
                if moved > 0 {
                    *relocked = std::fs::read(&guard.path).ok();
                }
            }
        }
        Ok(Some(guard))
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        restore(&self.path, self.before.as_deref());
        crate::signals::forget_restore(&self.path);
    }
}

/// Put a file back the way it was, or remove it if it was not there. Shared with the interrupt
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

    /// An external piece checkout: One package, identified by its `repository`.
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
            named: true,
        }
    }

    #[test]
    fn local_consumers_supply_inherited_dependencies_before_resolution() {
        let tmp =
            std::env::temp_dir().join(format!("day-patch-local-graph-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let app = fixture(
            &tmp.join("app"),
            "[package]\nname='app'\nversion='0.1.0'\n[dependencies]\nservice={git='https://example.invalid/service'}\n",
        );
        let service = piece_checkout(
            &tmp.join("service"),
            "service",
            "https://example.invalid/service",
        );
        std::fs::write(service.join("Cargo.toml"), "[package]\nname='service'\nversion='0.1.0'\nrepository='https://example.invalid/service'\n[workspace]\n[workspace.dependencies]\nday-platform-android={git='https://github.com/daybrite/day.git'}\n[dependencies]\nday-platform-android.workspace=true\n").unwrap();
        let day = day_checkout(&tmp.join("day"), &["day-app", "day-platform-android"]);
        let wanted = wanted_by_source(
            &app.root,
            &[
                checkout_source(DAY_GIT, &day),
                checkout_source("https://example.invalid/service", &service),
            ],
        )
        .unwrap();
        assert_eq!(wanted[&canon(DAY_GIT)], ["day-platform-android"]);
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn independent_gallery_packages_belong_to_the_same_checkout() {
        let tmp = std::env::temp_dir().join(format!("day-patch-gallery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = piece_checkout(&tmp, "piece", "https://example.invalid/piece");
        piece_checkout(
            &root.join("gallery"),
            "piece-gallery",
            "https://example.invalid/piece",
        );
        piece_checkout(
            &root.join("fixture"),
            "unrelated",
            "https://example.invalid/other",
        );
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
        let packages = checkout_crates(&root).unwrap();
        assert!(packages.contains_key("piece-gallery"));
        assert!(!packages.contains_key("unrelated"));
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// The table names every git-sourced day dependency, and points each at the checkout. This is
    /// the text both lifetimes use: `day patch` writes it to `.cargo/config.toml`, `--day-src`
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
day = { package = "day-app", git = "https://github.com/daybrite/day.git" }
day-build = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        let checkout = day_checkout(&tmp.join("day"), &["day-app", "day-build", "day-core"]);

        let (table, count) = patch_tables(
            &project.root,
            &[checkout_source(DAY_GIT, &checkout)],
            "# header\n",
        )
        .expect("table");
        let (git, published) = table.split_once("[patch.crates-io]").expect("{table}");
        assert!(
            git.contains("[patch.\"https://github.com/daybrite/day.git\"]"),
            "{table}"
        );
        assert!(git.contains("day-app = { path ="), "{table}");
        assert!(git.contains("day-build = { path ="), "{table}");
        assert!(
            !git.contains("day-core = {"),
            "the git table names only the crates the app takes from git: {table}"
        );
        // Before a resolve, the crates.io table carries everything; `settle` drops the unused.
        assert!(published.contains("day-core = { path ="), "{table}");
        assert_eq!(count, 2 + 3, "{table}");
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
day = { package = "day-app", git = "https://github.com/daybrite/day.git" }
day-piece-lottie = { git = "https://github.com/daybrite/day-piece-lottie.git" }
"#,
        );
        let day = day_checkout(&tmp.join("day"), &["day-app"]);
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
        assert_eq!(
            count,
            2 + 2,
            "two git entries, and both crates for crates.io: {table}"
        );
        assert!(
            table.contains("[patch.\"https://github.com/daybrite/day.git\"]\nday-app = { path ="),
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
    /// ref's shape implies, and the canonical URL stays the table's key, which is what lets an
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
day = { package = "day-app", git = "https://github.com/daybrite/day.git" }
"#,
        );
        let fork = Source {
            url: DAY_GIT.to_string(),
            target: Target::Fork {
                url: "https://github.com/acme/day.git".into(),
                git_ref: Some(GitRef::Branch("acme".into())),
                checkout: day_checkout(&tmp.join("fork"), &["day-app"]),
            },
            named: true,
        };
        let (table, count) = patch_tables(&project.root, &[fork], "").expect("table");
        assert_eq!(
            count, 2,
            "the git entry, and the same crate for crates.io: {table}"
        );
        assert!(
            table.contains("[patch.\"https://github.com/daybrite/day.git\"]"),
            "{table}"
        );
        assert!(
            table.contains(
                "day-app = { git = \"https://github.com/acme/day.git\", branch = \"acme\" }"
            ),
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

    /// The version rule: a checkout behind the URL's tip loses to git, every crate at once, the
    /// moment a release bump lands upstream, and the guard then names the two versions instead
    /// of suggesting a table entry that is already there.
    #[test]
    fn an_outranked_checkout_is_explained_by_version() {
        let tmp = std::env::temp_dir().join(format!("day-patch-outranked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let checkout = day_checkout(&tmp.join("day"), &["day-app", "day-core"]);
        assert_eq!(
            checkout_versions(&checkout)
                .get("day-core")
                .map(String::as_str),
            Some("0.1.0")
        );

        let sources = [checkout_source(DAY_GIT, &checkout)];
        let at = |v: &str| {
            [
                ("day-app".to_string(), canon(DAY_GIT), v.to_string()),
                ("day-core".to_string(), canon(DAY_GIT), v.to_string()),
            ]
        };
        let why = outranked(&sources, &at("0.2.0")).expect("a newer main outranks the checkout");
        assert!(
            why.contains("day-app v0.2.0") && why.contains("has v0.1.0"),
            "{why}"
        );
        // Equal versions: the patch would have shadowed git, so the reason lies elsewhere.
        assert!(outranked(&sources, &at("0.1.0")).is_none());
        // A checkout AHEAD of what resolved: the requirement is what keeps it out.
        let why = outranked(&sources, &at("0.0.1")).expect("a requirement the checkout misses");
        assert!(why.contains("does not accept"), "{why}");
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
day = { package = "day-app", git = "https://github.com/daybrite/day.git" }
day-part-http = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        let checkout = day_checkout(&tmp.join("day"), &["day-app"]);
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
        let day = day_checkout(&tmp.join("day"), &["day-app", "day-core"]);
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
        let a = slug(&[PathBuf::from("/w/day")]);
        assert_eq!(a, slug(&[PathBuf::from("/w/day")]));
        assert!(a.starts_with("day-"), "{a} should stay readable");

        // A git day-src is cached per ref, so the directory name IS the branch.
        let nav = slug(&[PathBuf::from(
            "/c/git/github.com/daybrite/day/experimental-nav",
        )]);
        let fix = slug(&[PathBuf::from("/c/git/github.com/daybrite/day/fix-482")]);
        assert_ne!(nav, fix, "two branches, two build trees");

        // The framework alone and the framework plus a patched piece build in different trees,
        // under the framework's name either way.
        let with_piece = slug(&[
            PathBuf::from("/w/day"),
            PathBuf::from("/w/day-piece-lottie"),
        ]);
        assert_ne!(a, with_piece, "another checkout, another build tree");
        assert!(with_piece.starts_with("day-"), "{with_piece}");
        assert!(nav.starts_with("experimental-nav-"), "{nav}");
        assert!(
            nav.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "{nav} has to be a directory name on every host"
        );
    }

    /// A lock that holds a patched URL at any version is what makes cargo skip the table; only
    /// those packages are named, by a spec cargo takes, and path or crates.io packages never are.
    #[test]
    fn the_lock_names_what_still_comes_from_a_patched_url() {
        let tmp = std::env::temp_dir().join(format!("day-stale-lock-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("mkdir");
        std::fs::write(
            tmp.join("Cargo.lock"),
            r#"version = 4

[[package]]
name = "day-app"
version = "0.4.10"
source = "git+https://github.com/daybrite/day.git#9996c0743ca8390a5c6f09eb1631caee55fd2a52"

[[package]]
name = "day-core"
version = "0.4.10"
source = "git+https://github.com/daybrite/day.git?branch=main#9996c0743ca8390a5c6f09eb1631caee55fd2a52"

[[package]]
name = "day-app"
version = "0.5.0"

[[package]]
name = "day-piece-lottie"
version = "0.2.0"
source = "git+https://github.com/daybrite/day-piece-lottie#abc"

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .expect("lock");
        let stale = stale_in_lock(&tmp, &[canon(DAY_GIT)], &BTreeMap::new());
        assert_eq!(
            stale,
            vec![
                (
                    "day-app".to_string(),
                    "0.4.10".to_string(),
                    "https://github.com/daybrite/day.git".to_string()
                ),
                (
                    "day-core".to_string(),
                    "0.4.10".to_string(),
                    "https://github.com/daybrite/day.git".to_string()
                ),
            ]
        );
        assert!(
            stale_in_lock(
                &tmp,
                &[canon("https://github.com/nobody/x")],
                &BTreeMap::new()
            )
            .is_empty()
        );
        assert!(stale_in_lock(&tmp.join("absent"), &[canon(DAY_GIT)], &BTreeMap::new()).is_empty());
        // A crate the crates.io table names is stale while the lock takes it from the registry,
        // and is spelled with the registry's own source so the spec is unambiguous.
        let named = BTreeMap::from([("serde".to_string(), toml::Value::Boolean(true))]);
        assert_eq!(
            stale_in_lock(&tmp, &[], &named),
            vec![(
                "serde".to_string(),
                "1.0.0".to_string(),
                "registry+https://github.com/rust-lang/crates.io-index".to_string()
            )]
        );
        let _ = std::fs::remove_dir_all(&tmp);
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
    /// CI checks the showcase out inside the day workspace, which put the app's own manifest
    /// under the checkout root, and `day patch` then demanded `day-showcase` be one of day's
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
        // An external piece from its own repository reports that repository's URL.
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

    /// Which dependencies need a patch entry: the direct ones from git, including the per-target
    /// tables where an app's backend crates live (the easiest ones to forget), each with the
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
day = { package = "day-app", git = "https://github.com/daybrite/day.git" }
day-part-http = { git = "https://github.com/daybrite/day.git" }
day-piece-lottie = { git = "https://github.com/daybrite/day-piece-lottie.git" }
serde = "1"
day-local = { path = "../elsewhere" }

[target.'cfg(target_os = "android")'.dependencies]
day-part-device = { git = "https://github.com/daybrite/day.git" }
"#,
        );
        // The manifest floor alone (no resolver: this fixture has no lockfile and no network).
        let deps = manifest_git_deps(&project.root).expect("deps");
        let day = "https://github.com/daybrite/day".to_string();
        assert_eq!(
            deps,
            [
                ("day-app".to_string(), day.clone()),
                ("day-part-device".to_string(), day.clone()),
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
    /// just what the manifest names is what let a stale `day-toolkit-material` build for months.
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
day = { package = "day-app", git = "https://github.com/daybrite/day.git" }
"#,
        );
        let deps = manifest_git_deps(&project.root).expect("deps");
        assert_eq!(deps.len(), 1, "the manifest names one crate");
        assert!(
            !deps.iter().any(|(d, _)| d == "day-toolkit-material"),
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
    /// here; verified against this very repository.
    #[test]
    fn the_checkout_map_is_read_from_the_workspace() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root");
        let crates = checkout_crates(here).expect("this repo is a day checkout");
        assert!(crates.contains_key(DAY_PACKAGE), "the facade crate");
        assert!(crates.contains_key("day-cli"), "this crate");
        assert!(
            crates[DAY_PACKAGE].join("Cargo.toml").is_file(),
            "the mapped path is a real crate dir"
        );
    }

    /// A crates.io app: the table lists what the sources carry, and a resolve's unused entries
    /// drop out without disturbing the rest.
    #[test]
    fn the_crates_io_table_drops_what_goes_unused() {
        let tmp = std::env::temp_dir().join(format!("day-patch-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let day = day_checkout(&tmp.join("day"), &["day-app", "day-core", "day-toolkit-qt"]);
        let sources = [checkout_source(DAY_GIT, &day)];
        let wanted = BTreeMap::from([(canon(DAY_GIT), Vec::new())]);
        let unused = BTreeSet::from(["day-toolkit-qt".to_string()]);
        let (table, count) = render(&sources, &wanted, "", &unused).expect("table");
        assert_eq!(count, 2, "{table}");
        assert!(table.starts_with("[patch.crates-io]\n"), "{table}");
        assert!(table.contains("day-core = { path ="), "{table}");
        assert!(!table.contains("day-toolkit-qt"), "{table}");

        // Every entry unused: the source contributes nothing, which is an error worth a sentence.
        let all = BTreeSet::from([
            "day-app".to_string(),
            "day-core".to_string(),
            "day-toolkit-qt".to_string(),
        ]);
        let err = render(&sources, &wanted, "", &all).expect_err("nothing to patch");
        assert!(err.contains("takes nothing from"), "{err}");
        // …unless `--main` found the source by itself: then it simply drops out.
        let found = [Source {
            named: false,
            ..checkout_source(DAY_GIT, &day)
        }];
        let (table, count) = render(&found, &wanted, "", &all).expect("dropped quietly");
        assert_eq!((table.as_str(), count), ("", 0));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// One crate, two sources: cargo would reject the duplicate key, so the table refuses first.
    #[test]
    fn a_crate_patched_from_two_sources_is_refused() {
        let tmp = std::env::temp_dir().join(format!("day-patch-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let a = piece_checkout(&tmp.join("a"), "day-piece-x", "https://example.invalid/a");
        let b = piece_checkout(&tmp.join("b"), "day-piece-x", "https://example.invalid/b");
        let sources = [
            checkout_source("https://example.invalid/a", &a),
            checkout_source("https://example.invalid/b", &b),
        ];
        let wanted = BTreeMap::new();
        let err = render(&sources, &wanted, "", &BTreeSet::new()).expect_err("twice");
        assert!(err.contains("day-piece-x is in both"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `--main` follows each `day-*` crate to its repository, once per repository, and only into
    /// Day's own organization; a followed repository's own manifests are read the same way.
    #[test]
    fn main_follows_each_day_repository_once() {
        let tmp = std::env::temp_dir().join(format!("day-patch-main-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let app = fixture(
            &tmp.join("app"),
            r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
day = { package = "day-app", version = "0.6" }
day-core = "0.6"
day-piece-lottie = "0.4"
day-weather = "1"
serde = "1"
local = { path = "../local" }
"#,
        );
        let day = day_checkout(&tmp.join("day"), &["day-app", "day-core"]);
        let lottie = tmp.join("lottie");
        std::fs::create_dir_all(&lottie).unwrap();
        std::fs::write(
            lottie.join("Cargo.toml"),
            "[package]\nname = \"day-piece-lottie\"\nversion = \"0.4.3\"\n\
             repository = \"https://github.com/daybrite/day-piece-lottie\"\n\
             [dependencies]\nday-piece-webview = \"0.4\"\n",
        )
        .unwrap();
        let webview = piece_checkout(
            &tmp.join("webview"),
            "day-piece-webview",
            "https://github.com/daybrite/day-piece-webview",
        );
        let mut asked = Vec::new();
        let mut lookup = |name: &str| -> Result<Option<String>, String> {
            asked.push(name.to_string());
            Ok(match name {
                "day-app" | "day-core" => Some("https://github.com/daybrite/day".into()),
                "day-piece-lottie" => Some("https://github.com/daybrite/day-piece-lottie".into()),
                "day-piece-webview" => Some("https://github.com/daybrite/day-piece-webview".into()),
                "day-weather" => Some("https://github.com/someone/day-weather".into()),
                _ => None,
            })
        };
        let mut fetch = |url: &str| -> Result<(PathBuf, String), String> {
            let dir = match url {
                "https://github.com/daybrite/day" => day.clone(),
                "https://github.com/daybrite/day-piece-lottie" => lottie.clone(),
                "https://github.com/daybrite/day-piece-webview" => webview.clone(),
                other => return Err(format!("unexpected fetch of {other}")),
            };
            Ok((dir, "0123456789abcdef0123456789abcdef01234567".into()))
        };
        let found = upstream_sources(&app.root, &[], &mut lookup, &mut fetch).expect("sources");
        let mut urls: Vec<&str> = found.iter().map(|s| s.url.as_str()).collect();
        urls.sort();
        assert_eq!(
            urls,
            [
                "https://github.com/daybrite/day",
                "https://github.com/daybrite/day-piece-lottie",
                "https://github.com/daybrite/day-piece-webview",
            ],
            "the third-party crate stays published, and the piece's own dependency is followed"
        );
        assert!(
            !(asked.contains(&"day-app".to_string()) && asked.contains(&"day-core".to_string())),
            "one lookup per repository: {asked:?}"
        );
        assert!(
            !asked.iter().any(|n| n == "serde" || n == "local"),
            "{asked:?}"
        );
        assert!(found.iter().all(|s| matches!(
            &s.target,
            Target::Fork { git_ref: Some(GitRef::Rev(r)), .. } if r.len() == 40
        )));

        // A checkout given with --local covers its crates: nothing to follow for those.
        let given = [checkout_source(DAY_GIT, &day)];
        let found = upstream_sources(
            &app.root,
            &given,
            &mut |name| {
                assert!(name != "day-app" && name != "day-core", "{name} is covered");
                Ok(None)
            },
            &mut |url| Err(format!("unexpected fetch of {url}")),
        )
        .expect("sources");
        assert!(found.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Which manifest entries ask crates.io for a Day crate: plain versions, renamed packages,
    /// every table; never paths, git, workspace inheritance or third-party names.
    #[test]
    fn registry_deps_are_read_from_every_table() {
        let tmp = std::env::temp_dir().join(format!("day-patch-regdeps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("Cargo.toml"),
            r#"
[package]
name = "app"
version = "0.1.0"

[workspace.dependencies]
day-model = "0.6"

[dependencies]
day = { package = "day-app", version = "0.6", features = ["model"] }
day-model.workspace = true
day-part-auth = { git = "https://github.com/daybrite/day-part-auth.git" }
day-local = { path = "local" }
serde = "1"

[target.'cfg(target_os = "android")'.dependencies]
day-part-device = { version = "0.6" }

[build-dependencies]
day-build = "0.6"
"#,
        )
        .unwrap();
        let deps: Vec<String> = registry_deps(&tmp).into_iter().collect();
        assert_eq!(
            deps,
            ["day-app", "day-build", "day-model", "day-part-device"],
            "{deps:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A piece's demo reaches the piece through `path = ".."` and its gallery through a sibling
    /// path, so the piece's own requirements (an optional web view behind a backend feature)
    /// count as the demo's.
    #[test]
    fn registry_deps_follow_path_dependencies() {
        let tmp = std::env::temp_dir().join(format!("day-patch-pathdeps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for dir in ["demo", "gallery"] {
            std::fs::create_dir_all(tmp.join(dir)).unwrap();
        }
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"day-piece-lottie\"\nversion = \"0.4.3\"\n\n[features]\n\
             gtk = [\"dep:day-piece-webview\"]\n\n[dependencies]\n\
             day-core = \"0.6\"\nday-piece-webview = { version = \"0.4\", optional = true }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("gallery/Cargo.toml"),
            "[package]\nname = \"gallery\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
             day-piece-lottie = { path = \"..\" }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("demo/Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
             day = { package = \"day-app\", version = \"0.6\" }\n\
             gallery = { path = \"../gallery\" }\n\n[workspace]\n",
        )
        .unwrap();
        let deps: Vec<String> = registry_deps(&tmp.join("demo")).into_iter().collect();
        assert_eq!(
            deps,
            ["day-app", "day-core", "day-piece-webview"],
            "{deps:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The table's path entries say which repository they patch through their checkout, so the
    /// check can tell a crate the checkout no longer carries from one nobody patched.
    #[test]
    fn path_entries_name_their_repository() {
        let tmp = std::env::temp_dir().join(format!("day-patch-repos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let day = day_checkout(&tmp.join("day"), &["day-app", "day-core"]);
        std::fs::create_dir_all(day.join(".git")).unwrap();
        let config = tmp.join("config.toml");
        std::fs::write(
            &config,
            format!(
                "[patch.crates-io]\nday-core = {{ path = {:?} }}\n\
                 day-piece-x = {{ git = \"https://github.com/daybrite/day-piece-x.git\", rev = \"abc\" }}\n",
                day.join("crates/day-core").display().to_string()
            ),
        )
        .unwrap();
        let repos = patched_repositories(&config);
        assert!(repos.contains(&canon(DAY_GIT)), "{repos:?}");
        assert!(
            repos.contains("https://github.com/daybrite/day-piece-x"),
            "{repos:?}"
        );
        assert!(git_keys(&config).is_empty(), "crates-io is not a git key");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn versions_compare_by_their_numeric_core() {
        assert!(newer("0.7.0", "0.6.9"));
        assert!(newer("0.6.10", "0.6.9"));
        assert!(!newer("0.6.1", "0.6.1"));
        assert!(!newer("0.6.1-dev", "0.6.1"));
        assert!(!newer("garbage", "0.1.0"));
    }
}
