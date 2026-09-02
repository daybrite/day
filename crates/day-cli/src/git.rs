// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day launch --git <url>[@<ref>]` — run an app straight out of a repository (DESIGN.md §16.5).
//!
//! Trying a Day app should not require deciding where to put it first. `--git` clones the
//! repository into a per-URL cache, finds the Day project inside it, and hands that directory to
//! the ordinary launch path: [`crate::meta::find_project`] and everything downstream see a normal
//! checkout and behave identically. Nothing in the launch body knows this happened.
//!
//! The cache is keyed by URL **and** ref (`<cache>/git/<host>/<owner>/<repo>/<ref>`), so the cargo
//! build tree under `build/day/` survives between runs and a second launch is an incremental
//! compile — the reason this is a cache directory rather than the system temp dir every other
//! scratch path in the CLI uses.
//!
//! It is a real working checkout, not an export. `Checkout` prints its path so it can be edited,
//! and a checkout carrying local edits is never updated over: someone who started working there
//! keeps their work, and the run builds what is on disk.
//!
//! Everything shells out to `git`, as `rebuild.rs` and `template.rs` already do; no git library is
//! linked in. Commands run through [`crate::ops::run_capture`], so their output is invisible by
//! default and forwarded live under `--verbose` — the CLI's one verbosity contract, rather than a
//! private choice about which git chatter matters.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::cli::CliError;
use crate::meta;
use crate::ops::{run_capture, status};
use crate::term::WARN;

/// A parsed `--git` argument.
#[derive(Debug, PartialEq, Eq)]
pub struct Spec {
    /// The repository to clone, with any `@ref`/`#ref` suffix removed.
    pub url: String,
    /// Branch, tag, or commit. `None` means the remote's default branch.
    pub git_ref: Option<String>,
}

/// Directory name standing in for "whatever the remote's default branch is". Git forbids a branch
/// or tag literally named `HEAD`, so this can never collide with a ref the user asked for.
const DEFAULT_REF_DIR: &str = "HEAD";

// --- Parsing -------------------------------------------------------------------------------------

/// Split `<url>[@<ref>]` (or `<url>#<ref>`) into its parts.
///
/// The `@` form is what the flag documents; `#` is accepted because `day new --template
/// <git-url>#ref` already spells a ref that way, and learning one syntax should not fail in the
/// other command.
///
/// Finding the `@` is the only subtle part. Two legitimate URLs carry one that is *not* a ref
/// separator — `git@github.com:owner/repo.git` and `https://user@host/owner/repo` — and both put
/// it inside the authority, before the path begins. So the separator is the last `@` at or after
/// [`path_start`], which also keeps a branch name containing `/` (`repo.git@feature/x`) intact.
pub fn parse_spec(arg: &str) -> Result<Spec, String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Err("--git needs a repository URL".into());
    }
    // `#` can't appear in a git URL at all, so the first one always starts the ref.
    let cut = arg.find('#').or_else(|| {
        let from = path_start(arg);
        arg[from..].rfind('@').map(|i| from + i)
    });
    let (url, git_ref) = match cut {
        Some(i) => (&arg[..i], &arg[i + 1..]),
        None => (arg, ""),
    };
    if url.is_empty() {
        return Err(format!("`{arg}` names a ref but no repository"));
    }
    if cut.is_some() && git_ref.is_empty() {
        return Err(format!(
            "`{arg}` ends with a ref separator but names no ref — drop it for the default branch"
        ));
    }
    Ok(Spec {
        url: url.to_string(),
        git_ref: (!git_ref.is_empty()).then(|| git_ref.to_string()),
    })
}

/// Byte offset where a git URL's *path* begins — past `scheme://authority`, past scp-like
/// `[user@]host:`, or 0 for a plain local path. A Windows drive letter (`C:\repos\x`) is a local
/// path, not an scp-like host.
fn path_start(url: &str) -> usize {
    if let Some(scheme) = url.find("://") {
        let authority = scheme + 3;
        return url[authority..]
            .find('/')
            .map_or(url.len(), |i| authority + i);
    }
    if let Some(colon) = url.find(':') {
        let drive_letter = colon == 1 && url.as_bytes()[0].is_ascii_alphabetic();
        if !drive_letter {
            return colon + 1;
        }
    }
    0
}

/// Whether a spec's URL names a REMOTE repository rather than a local path.
///
/// `--git` never has to ask — a repository URL is the only thing it takes. `--day-src` does: it
/// accepts a directory first, and a bare word that is neither a directory nor a URL should be
/// reported as a typo rather than handed to `git clone` to fail on minutes later.
pub fn looks_remote(url: &str) -> bool {
    // A scheme (`https://`, `ssh://`, `git://`) or scp-like `[user@]host:path`. `path_start`
    // already draws that line for the ref parser, and a local path leaves it at 0.
    url.contains("://") || path_start(url) > 0
}

// --- Where the checkout lives --------------------------------------------------------------------

/// The environment variable's value, if it is set and absolute. A relative cache root would put
/// the checkout somewhere that depends on the invoking directory, which is exactly what `--git`
/// exists to avoid, so it falls through to the per-OS default instead.
fn env_abs(key: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(key)?);
    dir.is_absolute().then_some(dir)
}

/// The CLI's user-level cache root.
///
/// `XDG_CACHE_HOME` wins on **every** host, macOS and Windows included: setting it is a deliberate
/// statement about where caches go, and honoring it only on Linux would make the variable a
/// coin flip. Absent it, each OS gets its own convention.
///
/// This is the first user-level directory the CLI has owned — everything else it writes is
/// `<project>/build/day/` (`clean.rs`) or `std::env::temp_dir()`. Config and data roots resolve the
/// same `$XDG_*`-first-then-per-OS way when something needs them; neither has a caller yet.
pub fn cache_dir() -> Result<PathBuf, String> {
    cache_dir_from(
        env_abs("XDG_CACHE_HOME"),
        env_abs("LOCALAPPDATA"),
        env_abs("HOME").or_else(|| env_abs("USERPROFILE")),
    )
}

/// [`cache_dir`] over values already read, so the resolution order is testable without mutating
/// the process environment — racy under `cargo test`'s thread pool, and `set_var` is `unsafe` in
/// edition 2024.
fn cache_dir_from(
    xdg_cache_home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(dir) = xdg_cache_home {
        return Ok(dir.join("day"));
    }
    if cfg!(windows)
        && let Some(dir) = local_app_data
    {
        return Ok(dir.join("day").join("cache"));
    }
    let home = home.ok_or_else(|| {
        "no home directory to cache the checkout in — set XDG_CACHE_HOME, or pass --dir".to_string()
    })?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Caches/day")
    } else {
        home.join(".cache/day")
    })
}

/// Where a spec's checkout belongs under `cache`: `git/<host>/<owner>/<repo>/<ref>`.
///
/// The ref is a relative path of its own, so `feature/x` nests instead of being flattened into
/// `feature-x` — which would collide with a branch actually named that.
fn checkout_dir(cache: &Path, spec: &Spec) -> Result<PathBuf, String> {
    let mut dir = cache.join("git");
    for segment in repo_slug(&spec.url)? {
        dir.push(segment);
    }
    match &spec.git_ref {
        None => dir.push(DEFAULT_REF_DIR),
        Some(r) => {
            for segment in path_segments(r, "ref")? {
                dir.push(segment);
            }
        }
    }
    Ok(dir)
}

/// A URL as directory segments: host first, then its path, minus any `.git` suffix. SSH, HTTPS,
/// and local paths all reduce to the same shape, so the same repository reached two ways shares
/// one checkout instead of building twice.
fn repo_slug(url: &str) -> Result<Vec<String>, String> {
    let start = path_start(url);
    let (authority, path) = if let Some(scheme) = url.find("://") {
        (&url[scheme + 3..start], &url[start..])
    } else if start > 0 {
        // scp-like `[user@]host:path` — `start` is just past the colon.
        (&url[..start - 1], &url[start..])
    } else {
        // A local path or a bare name. It has no host, and `local` keeps it from ever colliding
        // with a real one (a git remote's host always contains a dot or is an alias, never this).
        ("local", url)
    };
    let host = authority
        .rsplit('@') // drop any `user:password@`
        .next()
        .unwrap_or(authority)
        .split(':') // drop any `:port`
        .next()
        .unwrap_or(authority)
        .to_ascii_lowercase();
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut out = Vec::new();
    if !host.is_empty() {
        out.push(sanitize(&host));
    }
    out.extend(path_segments(path, "repository URL")?);
    if out.len() < 2 {
        return Err(format!("`{url}` does not name a repository"));
    }
    Ok(out)
}

/// Split a URL path or a ref into directory segments, rejecting anything that could climb out of
/// the cache root. `..` is the whole reason this function exists: a crafted URL must not be able
/// to aim the checkout at `~/.ssh`.
fn path_segments(path: &str, what: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for raw in path.split(['/', '\\']) {
        if raw.is_empty() || raw == "." {
            continue;
        }
        if raw == ".." {
            return Err(format!(
                "`{path}` is not a usable {what} (it contains `..`)"
            ));
        }
        out.push(sanitize(raw));
    }
    Ok(out)
}

/// One path segment, reduced to characters every filesystem accepts. Windows rejects `:` and `?`
/// outright; a colon in a segment also reads as a drive separator.
fn sanitize(segment: &str) -> String {
    segment
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// --- Clone, update, and locate ---------------------------------------------------------------

/// Put the repository on disk and return its ROOT.
///
/// `dir` is `--dir` (clone here instead of the cache). Separate from [`prepare`] because not every
/// repository day clones holds an app: `--day-src` clones the FRAMEWORK, which has no `Day.toml`
/// anywhere in it and would fail the project lookup [`prepare`] adds on top.
pub fn checkout(spec: &Spec, dir: Option<&Path>) -> Result<PathBuf, CliError> {
    require_git()?;
    let dest = match dir {
        Some(d) if d.is_absolute() => d.to_path_buf(),
        Some(d) => std::env::current_dir()
            .map_err(|e| CliError::failure(format!("current directory: {e}")))?
            .join(d),
        None => {
            let cache = cache_dir().map_err(CliError::failure)?;
            checkout_dir(&cache, spec).map_err(CliError::usage)?
        }
    };

    // The cache is day's to manage — it created every directory in it and may delete one. A
    // `--dir` is the caller's directory, and nothing here removes it: the two places that would
    // (a wrong origin, a failed clone) report instead.
    let ours = dir.is_none();
    if dest.join(".git").is_dir() {
        update(&dest, spec, ours)?;
    } else {
        clone(&dest, spec)?;
    }
    Ok(dest)
}

/// Put the repository on disk and return the Day project directory inside it — what the caller
/// hands to [`crate::meta::find_project`] as if the user had `cd`'d there.
///
/// `project` is the global `--project`, which under `--git` selects a project *within* the
/// checkout by repo-relative path.
pub fn prepare(
    spec: &Spec,
    dir: Option<&Path>,
    project: Option<&Path>,
) -> Result<PathBuf, CliError> {
    let dest = checkout(spec, dir)?;
    status("Checkout", &display_path(&dest));
    project_in(&dest, &spec.url, project)
}

/// `git` is not a build prerequisite for a normal project, so it is only demanded here — and the
/// miss is an environment failure (exit 3), the same class `day doctor` reports.
fn require_git() -> Result<(), CliError> {
    let found = Command::new("git")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if found {
        Ok(())
    } else {
        Err(CliError::env(
            "--git needs the `git` command, which is not on PATH (`day doctor` reports it too)",
        ))
    }
}

fn clone(dest: &Path, spec: &Spec) -> Result<(), CliError> {
    status("Cloning", &describe(spec));
    // Whether the cleanup below is allowed to delete `dest`. A `--dir` that already exists is
    // someone's directory — git will refuse to clone into it, and that refusal must not be
    // followed by day removing it.
    let ours = !dest.exists();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliError::failure(format!("{}: {e}", parent.display())))?;
    }
    let cloned = run_capture(
        Command::new("git")
            // Day captures git's output, so a credential prompt would be an invisible hang — the
            // terminal sits there with nothing on it and no way to know what it wants. Fail fast
            // instead and say what to do about it.
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(["clone", &spec.url])
            .arg(dest),
        "git clone",
    )
    .map_err(CliError::failure)?;
    if !cloned.status.success() {
        // A half-written tree would be mistaken for a checkout by the next run, which would then
        // try to fetch inside it and report something unrelated to the real failure.
        if ours {
            let _ = std::fs::remove_dir_all(dest);
        }
        return Err(CliError::failure(format!(
            "could not clone {}{}\n  A private repository needs credentials `git` can find \
             without asking — an ssh key in your agent, or a configured credential helper.",
            spec.url,
            detail(&cloned)
        )));
    }
    if let Some(r) = &spec.git_ref
        && let Err(e) = switch_to(dest, r, &spec.url)
    {
        // The directory is named after a ref this checkout never reached. Leaving it would put a
        // clone of the default branch under `…/no-such-branch/`, which the next run would treat
        // as that ref's checkout.
        if ours {
            let _ = std::fs::remove_dir_all(dest);
        }
        return Err(e);
    }
    Ok(())
}

/// Bring an existing checkout to the requested ref: fetch, then fast-forward. Nothing here ever
/// resets or forces — the two cases that would lose work (local edits, a diverged branch) stop and
/// say so instead.
fn update(dest: &Path, spec: &Spec, ours: bool) -> Result<(), CliError> {
    // A directory keyed by URL should hold that URL. When it doesn't — a moved repo, a
    // hand-edited cache, a slug collision nobody predicted — start over rather than build
    // somebody else's code under this URL's name. Inside the cache that means re-cloning; a
    // `--dir` is the caller's, so it is reported and left alone.
    let origin = git(dest, &["remote", "get-url", "origin"])?;
    let found = origin.status.success().then(|| stdout(&origin));
    let same = match (&found, repo_slug(&spec.url)) {
        (Some(url), Ok(want)) => repo_slug(url).is_ok_and(|have| have == want),
        _ => false,
    };
    if !same {
        if !ours {
            return Err(CliError::usage(format!(
                "{} is a checkout of {}, not {} — point --dir somewhere else",
                dest.display(),
                found.as_deref().unwrap_or("no remote"),
                spec.url,
            )));
        }
        std::fs::remove_dir_all(dest)
            .map_err(|e| CliError::failure(format!("{}: {e}", dest.display())))?;
        return clone(dest, spec);
    }

    // Local edits win. This checkout is somewhere a person may have started working, and a
    // `--git` launch that quietly discarded their changes would be the last time they trusted it.
    //
    // `Cargo.lock` is the exception, and it has to be: building here is what `--git` DOES, and
    // cargo rewrites the lock to record what it resolved. Counting day's own output as the user's
    // work in progress is how a checkout stops updating after its first build and then warns about
    // it on every run afterwards.
    let dirty = git(dest, &["status", "--porcelain"])?;
    let edits = edited_paths(&dirty.stdout);
    if !edits.is_empty() {
        warn(&format!(
            "the checkout has local edits — building it as it stands, at {}",
            short_head(dest)
        ));
        return Ok(());
    }
    let lock_dirty = dirty.status.success() && !dirty.stdout.is_empty();

    let fetched = git(dest, &["fetch", "--tags", "origin"])?;
    if !fetched.status.success() {
        // Offline is a normal way to run a second time. Say so and build what is already here.
        warn("could not reach the remote — building the checkout as it stands");
    }
    if let Some(r) = &spec.git_ref {
        // Only when it would actually move: a checkout already sitting on the requested ref is
        // left untouched, which is what keeps the lock cargo just wrote (and the resolution work
        // behind it) instead of discarding it on every run.
        if moves_head(dest, r) {
            discard_lock(dest, lock_dirty);
            switch_to(dest, r, &spec.url)?;
        }
    }
    fast_forward(dest, lock_dirty)
}

/// The file `cargo` rewrites inside a checkout day builds in.
const GENERATED_LOCK: &str = "Cargo.lock";

/// The paths `git status --porcelain` reports, minus the ones day's own builds write.
///
/// Porcelain lines are `XY <path>`, and a rename is `R  <old> -> <new>`; only the destination
/// matters here. Kept separate from the git call so the rule is testable without a repository.
fn edited_paths(porcelain: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(porcelain)
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| match path.split_once(" -> ") {
            Some((_, to)) => to,
            None => path,
        })
        .map(|p| p.trim().trim_matches('"').to_string())
        .filter(|p| !p.is_empty() && p != GENERATED_LOCK)
        .collect()
}

/// Whether checking `git_ref` out would move HEAD — false when the checkout already sits on it.
fn moves_head(dest: &Path, git_ref: &str) -> bool {
    let Ok(want) = git(dest, &["rev-parse", "--verify", "--quiet", git_ref]) else {
        return true;
    };
    if !want.status.success() {
        // Unresolvable here; let `switch_to` run and report it properly.
        return true;
    }
    stdout(&want) != head_sha(dest)
}

/// Throw away a `Cargo.lock` cargo rewrote, so a checkout or a merge that carries a new one can
/// land. Only ever called when the lock is the sole modified file and HEAD is about to move: git
/// refuses to overwrite it, and the alternative is a checkout that never updates again.
fn discard_lock(dest: &Path, lock_dirty: bool) {
    if lock_dirty {
        let _ = git(dest, &["checkout", "--", GENERATED_LOCK]);
    }
}

/// Check out a named ref. Detaching at a tag or commit is normal here, so git's detached-HEAD
/// advice is suppressed rather than printed at someone who asked for exactly that.
fn switch_to(dest: &Path, git_ref: &str, url: &str) -> Result<(), CliError> {
    let out = git(
        dest,
        &["-c", "advice.detachedHead=false", "checkout", git_ref],
    )?;
    if !out.status.success() {
        return Err(CliError::failure(format!(
            "{url} has no branch, tag, or commit `{git_ref}`{}",
            detail(&out)
        )));
    }
    Ok(())
}

/// Fast-forward the current branch onto its upstream. A ref naming a branch has one; a tag or
/// commit is detached and already exactly where it should be, so this is a no-op there.
fn fast_forward(dest: &Path, lock_dirty: bool) -> Result<(), CliError> {
    let upstream = git(dest, &["rev-parse", "--verify", "--quiet", "@{u}"])?;
    if !upstream.status.success() {
        return Ok(());
    }
    // Already there: nothing to merge, and nothing to discard. The common case on every run after
    // the first, so it must not cost the lock cargo wrote.
    if stdout(&upstream) == head_sha(dest) {
        return Ok(());
    }
    let before = short_head(dest);
    // A merge refuses to overwrite a modified file, so a lock cargo rewrote would otherwise turn
    // every upstream commit that touches it into "the checkout has commits the remote doesn't".
    discard_lock(dest, lock_dirty);
    let merged = git(dest, &["merge", "--ff-only", "@{u}"])?;
    if !merged.status.success() {
        warn("the checkout has commits the remote doesn't — building it as it stands");
        return Ok(());
    }
    let after = short_head(dest);
    if before != after {
        status("Updated", &format!("{before} → {after}"));
    }
    Ok(())
}

/// Where to read a `--script` from when the app came out of a repository.
///
/// The flag is CWD-relative as always, and a file that IS there wins — that is what lets someone
/// drive a cloned app with a dayscript of their own. But the usual intent under `--git` is the
/// repository's own script (`--script dayscript/demo.yaml`), and the repository is not where the
/// caller is standing, so a relative path that resolves nowhere else is looked up in the checkout.
pub fn script_path(path: &Path, project_root: &Path) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }
    let in_repo = project_root.join(path);
    if in_repo.exists() {
        in_repo
    } else {
        path.to_path_buf()
    }
}

/// Locate the Day project inside the checkout.
fn project_in(root: &Path, url: &str, project: Option<&Path>) -> Result<PathBuf, CliError> {
    if let Some(rel) = project {
        if rel.is_absolute() {
            return Err(CliError::usage(format!(
                "with --git, --project selects a directory INSIDE the checkout, so it must be \
                 repo-relative — `{}` is absolute",
                rel.display()
            )));
        }
        let dir = root.join(rel);
        if !dir.join("Day.toml").is_file() {
            return Err(CliError::usage(format!(
                "{url} has no Day.toml in `{}`",
                rel.display()
            )));
        }
        return Ok(dir);
    }
    match meta::day_projects(root).as_slice() {
        [] => Err(CliError::usage(format!(
            "{url} holds no Day project — no directory in it has both a Day.toml and a Cargo.toml"
        ))),
        [only] => Ok(only.clone()),
        many => Err(CliError::usage(format!(
            "{url} holds {} Day projects — name one with --project: {}",
            many.len(),
            many.iter()
                .map(|p| {
                    // Repo-relative with forward slashes on every host: the value the caller
                    // types back, not the OS separator (Windows would print `apps\example`).
                    p.strip_prefix(root)
                        .unwrap_or(p)
                        .display()
                        .to_string()
                        .replace('\\', "/")
                })
                .collect::<Vec<_>>()
                .join(", "),
        ))),
    }
}

// --- Small helpers -------------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> Result<Output, CliError> {
    run_capture(Command::new("git").current_dir(dir).args(args), "git").map_err(CliError::failure)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The full commit HEAD points at, for comparing against a resolved ref. Empty when git won't
/// say, which compares unequal to any real sha and so errs toward doing the update.
fn head_sha(dir: &Path) -> String {
    match git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
        Ok(out) if out.status.success() => stdout(&out),
        _ => String::new(),
    }
}

/// The abbreviated commit HEAD points at, or `?` when git won't say (a checkout mid-clone, a
/// repository with no commits). Only ever used in a message, so it never fails a run.
fn short_head(dir: &Path) -> String {
    match git(dir, &["rev-parse", "--short", "HEAD"]) {
        Ok(out) if out.status.success() => stdout(&out),
        _ => "?".to_string(),
    }
}

/// Git's own last words, indented under the error day reports. Trimmed to the tail because a
/// failed clone can print a paragraph of transport detail and the cause is at the end.
fn detail(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(3)
        .collect();
    if tail.is_empty() {
        return String::new();
    }
    let mut msg = String::from("\n");
    for line in tail.into_iter().rev() {
        msg.push_str("  ");
        msg.push_str(line.trim_end());
        msg.push('\n');
    }
    msg.pop();
    msg
}

/// `owner/repo @ ref` for the `Cloning` line — the whole URL is noise once it is on screen twice.
fn describe(spec: &Spec) -> String {
    let name = match repo_slug(&spec.url) {
        Ok(segments) if segments.len() >= 3 => segments[segments.len() - 2..].join("/"),
        _ => spec.url.clone(),
    };
    match &spec.git_ref {
        Some(r) => format!("{name} @ {r}"),
        None => name,
    }
}

/// A path with the home directory folded back to `~`, so the `Checkout` line stays short enough
/// to read and short enough to paste.
pub fn display_path(path: &Path) -> String {
    let shown = env_abs("HOME")
        .or_else(|| env_abs("USERPROFILE"))
        .and_then(|home| {
            path.strip_prefix(home)
                .ok()
                .map(|rest| Path::new("~").join(rest))
        });
    shown
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

fn warn(msg: &str) {
    anstream::eprintln!("{WARN}warning{WARN:#} {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(url: &str, git_ref: Option<&str>) -> Spec {
        Spec {
            url: url.to_string(),
            git_ref: git_ref.map(str::to_string),
        }
    }

    #[test]
    fn at_separates_the_ref() {
        assert_eq!(
            parse_spec("https://github.com/daybrite/Day-Rise.git@main").unwrap(),
            spec("https://github.com/daybrite/Day-Rise.git", Some("main"))
        );
    }

    #[test]
    fn hash_separates_the_ref_too() {
        // The spelling `day new --template <git-url>#ref` already teaches.
        assert_eq!(
            parse_spec("https://github.com/daybrite/Day-Rise.git#v1.2.0").unwrap(),
            spec("https://github.com/daybrite/Day-Rise.git", Some("v1.2.0"))
        );
    }

    #[test]
    fn no_ref_means_the_default_branch() {
        assert_eq!(
            parse_spec("https://github.com/daybrite/Day-Rise.git").unwrap(),
            spec("https://github.com/daybrite/Day-Rise.git", None)
        );
    }

    /// The `@` in an ssh URL is a username, not a ref. Reading it as one would leave a repository
    /// named `git` at ref `github.com:daybrite/Day-Rise.git`.
    #[test]
    fn ssh_userinfo_is_not_a_ref() {
        assert_eq!(
            parse_spec("git@github.com:daybrite/Day-Rise.git").unwrap(),
            spec("git@github.com:daybrite/Day-Rise.git", None)
        );
        assert_eq!(
            parse_spec("git@github.com:daybrite/Day-Rise.git@v1").unwrap(),
            spec("git@github.com:daybrite/Day-Rise.git", Some("v1"))
        );
    }

    #[test]
    fn https_userinfo_is_not_a_ref() {
        assert_eq!(
            parse_spec("https://user@example.com/owner/repo").unwrap(),
            spec("https://user@example.com/owner/repo", None)
        );
    }

    /// A branch name may contain `/`, so the separator can't be found by looking after the last
    /// slash in the whole string.
    #[test]
    fn a_slash_in_the_ref_survives() {
        assert_eq!(
            parse_spec("https://example.com/o/r.git@feature/x").unwrap(),
            spec("https://example.com/o/r.git", Some("feature/x"))
        );
    }

    #[test]
    fn an_empty_or_dangling_spec_is_an_error() {
        assert!(parse_spec("   ").is_err());
        assert!(parse_spec("https://example.com/o/r.git@").is_err());
        assert!(parse_spec("@main").is_err());
    }

    #[test]
    fn xdg_cache_home_wins_on_every_host() {
        let got = cache_dir_from(
            Some(PathBuf::from("/xdg/cache")),
            Some(PathBuf::from("/local/appdata")),
            Some(PathBuf::from("/home/dev")),
        )
        .unwrap();
        assert_eq!(got, PathBuf::from("/xdg/cache/day"));
    }

    /// A relative XDG_CACHE_HOME is ignored by `env_abs` before it ever reaches the resolver, so
    /// the fallback still lands somewhere absolute.
    #[test]
    fn the_fallback_is_under_the_home_directory() {
        let home = PathBuf::from("/home/dev");
        let got = cache_dir_from(None, None, Some(home.clone())).unwrap();
        assert!(got.starts_with(&home), "{got:?} is not under {home:?}");
        assert!(got.ends_with("day"), "{got:?} does not end in `day`");
    }

    #[test]
    fn no_home_and_no_xdg_is_an_error() {
        assert!(cache_dir_from(None, None, None).is_err());
    }

    #[test]
    fn https_maps_to_host_owner_repo_ref() {
        let cache = Path::new("/cache/day");
        let s = parse_spec("https://github.com/daybrite/Day-Rise.git@main").unwrap();
        assert_eq!(
            checkout_dir(cache, &s).unwrap(),
            PathBuf::from("/cache/day/git/github.com/daybrite/Day-Rise/main")
        );
    }

    /// The same repository reached three ways shares one checkout, so it is cloned and built once.
    #[test]
    fn ssh_and_https_and_bare_share_a_directory() {
        let cache = Path::new("/cache/day");
        let a = checkout_dir(
            cache,
            &parse_spec("https://GitHub.com/daybrite/Day-Rise.git").unwrap(),
        );
        let b = checkout_dir(
            cache,
            &parse_spec("git@github.com:daybrite/Day-Rise.git").unwrap(),
        );
        let c = checkout_dir(
            cache,
            &parse_spec("https://github.com/daybrite/Day-Rise").unwrap(),
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(
            c.unwrap(),
            PathBuf::from("/cache/day/git/github.com/daybrite/Day-Rise/HEAD")
        );
    }

    #[test]
    fn a_ref_with_a_slash_nests() {
        let cache = Path::new("/cache/day");
        let s = parse_spec("https://example.com/o/r.git@feature/x").unwrap();
        assert_eq!(
            checkout_dir(cache, &s).unwrap(),
            PathBuf::from("/cache/day/git/example.com/o/r/feature/x")
        );
    }

    /// The checkout path is built from a URL, so it must not be able to climb out of the cache.
    #[test]
    fn dot_dot_cannot_escape_the_cache() {
        let cache = Path::new("/cache/day");
        assert!(checkout_dir(cache, &spec("https://example.com/../../etc", None)).is_err());
        assert!(
            checkout_dir(
                cache,
                &spec("https://example.com/o/r", Some("../../../.ssh"))
            )
            .is_err()
        );
    }

    #[test]
    fn a_url_without_a_repository_is_rejected() {
        assert!(repo_slug("https://example.com").is_err());
        assert!(repo_slug("https://example.com/").is_err());
    }

    #[test]
    fn the_cloning_line_names_owner_and_repo() {
        let s = parse_spec("https://github.com/daybrite/Day-Rise.git@main").unwrap();
        assert_eq!(describe(&s), "daybrite/Day-Rise @ main");
        let s = parse_spec("https://github.com/daybrite/Day-Rise.git").unwrap();
        assert_eq!(describe(&s), "daybrite/Day-Rise");
    }

    // --- Against a real repository ---------------------------------------------------------
    //
    // Cloning is the half that can't be reasoned about from the strings alone, so these drive
    // `git` itself — against a repository built in the temp dir, so they need no network and run
    // wherever CI does. Tag + pid keeps concurrent test threads off each other's directories
    // without a `tempfile` dependency, the same fixture shape `tests/mcp_stdio.rs` uses.

    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("day-git-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Scratch { dir }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Run `git` in `dir`, with an identity and defaults of our own so the result never depends on
    /// the machine's git config (or on a signing key it can't reach in CI).
    fn run_git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=day tests",
                "-c",
                "user.email=tests@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run git");
        assert!(ok.success(), "git {args:?} failed in {}", dir.display());
    }

    /// A repository holding one Day project, plus the two manifests `meta::day_projects` requires.
    fn origin_repo(scratch: &Scratch, sub: &str) -> PathBuf {
        let dir = scratch.dir.join("origin");
        let project = if sub.is_empty() {
            dir.clone()
        } else {
            dir.join(sub)
        };
        std::fs::create_dir_all(&project).expect("project dir");
        std::fs::write(
            project.join("Day.toml"),
            "schema = 1\n[app]\nid = \"dev.example.gitfixture\"\n",
        )
        .expect("Day.toml");
        std::fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("Cargo.toml");
        if !dir.join(".git").is_dir() {
            run_git(&dir, &["init"]);
        }
        run_git(&dir, &["add", "-A"]);
        run_git(&dir, &["commit", "-m", "fixture"]);
        dir
    }

    /// Skip the git-driven tests on a host without git rather than fail them — the same call the
    /// CLI makes, so a skip here means `--git` could not have run either.
    fn git_available() -> bool {
        if require_git().is_ok() {
            return true;
        }
        eprintln!("skipped: no `git` on PATH");
        false
    }

    fn head(dir: &Path) -> String {
        short_head(dir)
    }

    #[test]
    fn clones_and_returns_the_project_inside() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("clone");
        let origin = origin_repo(&scratch, "");
        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();

        let project = prepare(&spec, Some(&dest), None).expect("prepare");
        assert_eq!(project, dest);
        assert!(dest.join("Day.toml").is_file());
    }

    #[test]
    fn a_second_run_fast_forwards() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("ff");
        let origin = origin_repo(&scratch, "");
        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();
        prepare(&spec, Some(&dest), None).expect("first run");
        let first = head(&dest);

        std::fs::write(origin.join("NEW.md"), "later\n").expect("write");
        run_git(&origin, &["add", "-A"]);
        run_git(&origin, &["commit", "-m", "later"]);

        prepare(&spec, Some(&dest), None).expect("second run");
        assert_ne!(head(&dest), first, "HEAD did not advance");
        assert!(dest.join("NEW.md").is_file());
    }

    /// `Cargo.lock` is written by the build `--git` just ran, so it is not a local edit. Reading
    /// it as one stops the checkout updating after its first build and warns on every run after.
    #[test]
    fn a_rewritten_lock_is_not_a_local_edit() {
        assert!(edited_paths(b" M Cargo.lock\n").is_empty());
        assert!(edited_paths(b"").is_empty());
        // Anything else still counts, alongside the lock or on its own.
        assert_eq!(edited_paths(b" M src/lib.rs\n"), ["src/lib.rs"]);
        assert_eq!(
            edited_paths(b" M Cargo.lock\n M src/lib.rs\n"),
            ["src/lib.rs"]
        );
        // Untracked files are someone's work too.
        assert_eq!(edited_paths(b"?? notes.md\n"), ["notes.md"]);
        // A rename reports both sides; the destination is the path that exists.
        assert_eq!(edited_paths(b"R  old.rs -> new.rs\n"), ["new.rs"]);
        // A lock somewhere else in the tree is not the one day's build writes.
        assert_eq!(edited_paths(b" M sub/Cargo.lock\n"), ["sub/Cargo.lock"]);
    }

    /// End to end: build in the checkout (which is what rewrites the lock), then update. The
    /// commit has to land, with no warning and no lost work.
    #[test]
    fn a_lock_rewritten_by_a_build_does_not_block_the_update() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("lockff");
        let origin = origin_repo(&scratch, "");
        std::fs::write(origin.join("Cargo.lock"), "# resolved\n").expect("write");
        run_git(&origin, &["add", "-A"]);
        run_git(&origin, &["commit", "-m", "lock"]);

        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();
        prepare(&spec, Some(&dest), None).expect("first run");
        let first = head(&dest);

        // What cargo does during the build day just ran.
        std::fs::write(dest.join("Cargo.lock"), "# rewritten by cargo\n").expect("write");

        // Upstream moves, and touches the same file — the case a dirty lock would block.
        std::fs::write(origin.join("Cargo.lock"), "# resolved, later\n").expect("write");
        std::fs::write(origin.join("NEW.md"), "later\n").expect("write");
        run_git(&origin, &["add", "-A"]);
        run_git(&origin, &["commit", "-m", "later"]);

        prepare(&spec, Some(&dest), None).expect("second run");
        assert_ne!(head(&dest), first, "the update was blocked by the lockfile");
        assert!(dest.join("NEW.md").is_file());
    }

    /// With nothing new upstream — every run after the first — the lock cargo wrote is left
    /// alone. Discarding it anyway would throw away the resolution behind it and make the next
    /// build redo the work, on every single run.
    #[test]
    fn an_up_to_date_checkout_keeps_the_lock_cargo_wrote() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("lockkeep");
        let origin = origin_repo(&scratch, "");
        std::fs::write(origin.join("Cargo.lock"), "# resolved\n").expect("write");
        run_git(&origin, &["add", "-A"]);
        run_git(&origin, &["commit", "-m", "lock"]);

        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();
        prepare(&spec, Some(&dest), None).expect("first run");
        std::fs::write(dest.join("Cargo.lock"), "# rewritten by cargo\n").expect("write");

        prepare(&spec, Some(&dest), None).expect("second run");
        assert_eq!(
            std::fs::read_to_string(dest.join("Cargo.lock")).expect("read"),
            "# rewritten by cargo\n",
            "an up-to-date checkout had its lockfile reset for no reason"
        );
    }

    /// The checkout is a place someone may have started working. An update must never be what
    /// takes their edits away.
    #[test]
    fn local_edits_survive_an_update() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("dirty");
        let origin = origin_repo(&scratch, "");
        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();
        prepare(&spec, Some(&dest), None).expect("first run");
        let pinned = head(&dest);
        std::fs::write(dest.join("Day.toml"), "schema = 1\n[app]\nid = \"mine\"\n")
            .expect("local edit");

        std::fs::write(origin.join("NEW.md"), "later\n").expect("write");
        run_git(&origin, &["add", "-A"]);
        run_git(&origin, &["commit", "-m", "later"]);

        prepare(&spec, Some(&dest), None).expect("second run");
        assert_eq!(head(&dest), pinned, "an edited checkout was moved anyway");
        let kept = std::fs::read_to_string(dest.join("Day.toml")).expect("read back");
        assert!(kept.contains("mine"), "the local edit was overwritten");
    }

    #[test]
    fn a_ref_the_repository_lacks_fails() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("noref");
        let origin = origin_repo(&scratch, "");
        let dest = scratch.dir.join("checkout");
        let arg = format!(
            "{}@no-such-branch",
            origin.to_str().expect("utf-8 temp path")
        );
        let spec = parse_spec(&arg).unwrap();
        assert!(prepare(&spec, Some(&dest), None).is_err());
    }

    /// `--dir` names somebody's directory. A wrong-repo checkout there is reported, never
    /// deleted — only the cache is day's to re-clone.
    #[test]
    fn a_dir_holding_another_repository_is_reported_not_deleted() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("wrongrepo");
        let origin = origin_repo(&scratch, "");
        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();
        prepare(&spec, Some(&dest), None).expect("first run");

        let other = Spec {
            url: "https://example.com/someone/else.git".into(),
            git_ref: None,
        };
        let err = prepare(&other, Some(&dest), None).expect_err("a different repo is refused");
        assert_eq!(err.exit_code(), 2);
        assert!(dest.join(".git").is_dir(), "the caller's --dir was deleted");
        assert!(dest.join("Day.toml").is_file());
    }

    /// A clone into a directory that already has something in it fails. What must not follow is
    /// day removing that directory on the way out.
    #[test]
    fn a_failed_clone_leaves_a_pre_existing_dir_alone() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("occupied");
        let dest = scratch.dir.join("mine");
        std::fs::create_dir_all(&dest).expect("dir");
        std::fs::write(dest.join("notes.txt"), "keep me\n").expect("write");
        let missing = scratch.dir.join("no-such-repo");
        let spec = parse_spec(missing.to_str().expect("utf-8 temp path")).unwrap();

        assert!(prepare(&spec, Some(&dest), None).is_err());
        assert!(
            dest.join("notes.txt").is_file(),
            "the directory was deleted"
        );
    }

    #[test]
    fn a_project_in_a_subdirectory_is_found() {
        if !git_available() {
            return;
        }
        let scratch = Scratch::new("sub");
        let origin = origin_repo(&scratch, "apps/example");
        let dest = scratch.dir.join("checkout");
        let spec = parse_spec(origin.to_str().expect("utf-8 temp path")).unwrap();

        let project = prepare(&spec, Some(&dest), None).expect("prepare");
        assert_eq!(project, dest.join("apps").join("example"));
    }

    #[test]
    fn several_projects_ask_for_a_project_flag() {
        let scratch = Scratch::new("many");
        let root = scratch.dir.join("repo");
        for name in ["one", "two"] {
            let dir = root.join("apps").join(name);
            std::fs::create_dir_all(&dir).expect("dir");
            std::fs::write(dir.join("Day.toml"), "schema = 1\n").expect("Day.toml");
            std::fs::write(dir.join("Cargo.toml"), "[package]\n").expect("Cargo.toml");
        }
        let err = project_in(&root, "https://example.com/o/r", None)
            .expect_err("two projects is ambiguous");
        let msg = err.to_string();
        assert!(msg.contains("apps/one"), "{msg}");
        assert!(msg.contains("apps/two"), "{msg}");
        assert!(msg.contains("--project"), "{msg}");

        let picked = project_in(
            &root,
            "https://example.com/o/r",
            Some(Path::new("apps/two")),
        )
        .expect("--project picks one");
        assert_eq!(picked, root.join("apps").join("two"));
    }

    /// A repository's own dayscript is named as the repository names it, from wherever you ran
    /// the command — but a file of your own in the current directory still wins.
    #[test]
    fn a_script_falls_back_to_the_checkout() {
        let scratch = Scratch::new("script");
        let repo = scratch.dir.join("repo");
        std::fs::create_dir_all(repo.join("dayscript")).expect("dir");
        std::fs::write(repo.join("dayscript/demo.yaml"), "steps: []\n").expect("write");

        let rel = Path::new("dayscript/demo.yaml");
        assert_eq!(script_path(rel, &repo), repo.join(rel));

        // One that exists nowhere is left alone, so the error names what the caller typed.
        let missing = Path::new("dayscript/absent.yaml");
        assert_eq!(script_path(missing, &repo), missing.to_path_buf());

        // An absolute path is never rewritten.
        let abs = scratch.dir.join("elsewhere.yaml");
        assert_eq!(script_path(&abs, &repo), abs);
    }

    #[test]
    fn an_absolute_project_with_git_is_a_usage_error() {
        let scratch = Scratch::new("abs");
        let err = project_in(&scratch.dir, "https://example.com/o/r", Some(&scratch.dir))
            .expect_err("absolute --project is rejected");
        assert_eq!(err.exit_code(), 2);
    }
}
