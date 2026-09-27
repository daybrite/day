// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day metadata --version-bump` / `--version-set` / `--build-set`: move the app to its next
//! release number, and optionally commit, tag, and push it (docs/cli.md).
//!
//! The version lives in Cargo.toml (Day.toml derives it, never restates it), either the app's own
//! `[package] version` or, for an app inside a workspace, the `[workspace.package] version` it
//! inherits. The build number is Day.toml's `[app] build`, the monotonic versionCode /
//! CFBundleVersion the stores require to grow with every upload, so any new version takes the next
//! build number unless `--build-set` names one. Both files are edited with `toml_edit`, so their
//! comments and layout survive.
//!
//! The git steps run with the terminal attached, not captured: git asks for credentials, an SSH
//! passphrase or a signing-key PIN on its own (reading /dev/tty, masking what it should), and that
//! only works when its stdin, stdout and stderr are the user's terminal. Capturing them would turn
//! every prompt into an invisible hang, which is why `crate::git`'s clone disables prompts
//! instead; here the prompts are the point.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::CliError;
use crate::meta::Project;
use crate::ops::status;

/// Which part of `MAJOR.MINOR.PATCH` a bump advances.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Part {
    Patch,
    Minor,
    Major,
}

/// What `day metadata` was asked to change, and what to do in git afterwards.
pub struct Request {
    pub bump: Option<Part>,
    pub set_version: Option<String>,
    pub set_build: Option<u64>,
    pub commit: bool,
    pub message: Option<String>,
    pub tag: bool,
    pub push: bool,
}

/// A release number: `MAJOR.MINOR.PATCH`, with any pre-release or build suffix kept only for
/// reading (a bump from `1.4.0-beta.2` releases `1.4.0`'s successor, not another beta).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parse `x.y.z`, tolerating a `-pre` / `+build` suffix when `strict` is false (the version
    /// already in Cargo.toml), and refusing one when it is true (a version the user sets, where a
    /// suffix would be a typo more often than an intent).
    pub fn parse(text: &str, strict: bool) -> Result<Version, String> {
        let core = if strict {
            text
        } else {
            text.split(['-', '+']).next().unwrap_or(text)
        };
        let parts: Vec<&str> = core.split('.').collect();
        let number = |s: &str| -> Option<u64> {
            // No leading zeros, as semver requires ("01" is not a version component).
            if s.is_empty() || (s.len() > 1 && s.starts_with('0')) {
                return None;
            }
            s.parse().ok()
        };
        match parts.as_slice() {
            [a, b, c] => match (number(a), number(b), number(c)) {
                (Some(major), Some(minor), Some(patch)) => Ok(Version {
                    major,
                    minor,
                    patch,
                }),
                _ => Err(format!("{text:?} is not a MAJOR.MINOR.PATCH version")),
            },
            _ => Err(format!("{text:?} is not a MAJOR.MINOR.PATCH version")),
        }
    }

    pub fn bumped(self, part: Part) -> Version {
        match part {
            Part::Patch => Version {
                patch: self.patch + 1,
                ..self
            },
            Part::Minor => Version {
                minor: self.minor + 1,
                patch: 0,
                ..self
            },
            Part::Major => Version {
                major: self.major + 1,
                minor: 0,
                patch: 0,
            },
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Where the version is written: the Cargo.toml that holds it, and the path within it.
struct VersionSource {
    file: PathBuf,
    /// `package` for the app's own version, `workspace.package` when it is inherited.
    table: &'static [&'static str],
}

pub fn run(project: &Project, req: Request) -> Result<i32, CliError> {
    let commit = req.commit || req.message.is_some() || req.tag || req.push;
    let tag = req.tag || req.push;
    if req.bump.is_none() && req.set_version.is_none() && req.set_build.is_none() {
        return Err(CliError::usage(
            "nothing to change: pass --version-bump, --version-set or --build-set",
        ));
    }
    let set_version = req
        .set_version
        .as_deref()
        .map(|v| Version::parse(v, true))
        .transpose()
        .map_err(CliError::usage)?;

    let source = version_source(&project.root).map_err(CliError::failure)?;
    let day_toml = project.root.join("Day.toml");
    let current_text = read_version(&source).map_err(CliError::failure)?;
    let current = Version::parse(&current_text, false)
        .map_err(|e| CliError::failure(format!("{}: {e}", display(&source.file, &project.root))))?;
    let current_build = read_build(&day_toml).map_err(CliError::failure)?;

    let version = match (req.bump, set_version) {
        (Some(part), _) => current.bumped(part),
        (None, Some(v)) => v,
        (None, None) => current,
    };
    let version_changes = version.to_string() != current_text;
    // A new version is a new upload, and the stores refuse an upload whose build number did not
    // grow, so it takes the next one unless the caller chose it.
    let build = req.set_build.unwrap_or(if version_changes {
        current_build + 1
    } else {
        current_build
    });
    if !version_changes && build == current_build {
        return Err(CliError::usage(format!(
            "version {current_text} and build {current_build} are already set; nothing to change"
        )));
    }
    let tag_name = format!("v{version}");

    // Everything that can refuse, refused before a file is touched: a half-applied bump on a
    // tree that was not clean is exactly the mess this check exists to prevent.
    let repo = if commit {
        let repo = repo_root(&project.root)?;
        let dirty = git_capture(&repo, &["status", "--porcelain", "--untracked-files=no"])?;
        if !dirty.trim().is_empty() {
            return Err(CliError::failure(
                "Must be run on a clean git worktree; commit (or stash) first",
            ));
        }
        if tag
            && git_ok(
                &repo,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/tags/{tag_name}"),
                ],
            )
        {
            return Err(CliError::failure(format!(
                "tag {tag_name} already exists; choose another version with --version-set"
            )));
        }
        Some(repo)
    } else {
        None
    };

    if version_changes {
        write_version(&source, &version.to_string()).map_err(CliError::failure)?;
        refresh_lockfile(&source.file);
    }
    if build != current_build {
        write_build(&day_toml, build).map_err(CliError::failure)?;
    }
    status(
        "Version",
        &format!("{current_text} → {version} (build {current_build} → {build})"),
    );

    let Some(repo) = repo else {
        return Ok(0);
    };
    // The tree was clean, so every modified tracked file is this command's.
    let changed: Vec<String> = git_capture(&repo, &["diff", "--name-only"])?
        .lines()
        .map(str::to_string)
        .collect();
    let mut add = vec!["add", "--"];
    add.extend(changed.iter().map(String::as_str));
    // Each failure says where it left things, since the steps before it have already happened.
    let files = changed.join(", ");
    let message = req.message.clone().unwrap_or_else(|| tag_name.clone());
    git_interactive(&repo, &add)
        .and_then(|()| git_interactive(&repo, &["commit", "-m", &message]))
        .map_err(|e| {
            CliError::failure(format!(
                "{e}; {files} are updated and staged but not committed. Commit them yourself, or \
                 undo with `git reset --hard`"
            ))
        })?;
    status("Committed", &message);
    if tag {
        git_interactive(&repo, &["tag", "-a", &tag_name, "-m", &tag_name]).map_err(|e| {
            CliError::failure(format!("{e}; the release is committed but not tagged"))
        })?;
        status("Tagged", &tag_name);
    }
    if req.push {
        let (remote, dest) = push_destination(&repo)?;
        let refs = [format!("HEAD:{dest}"), format!("refs/tags/{tag_name}")];
        // One atomic push: the release commit and its tag land together or not at all, so a
        // rejected branch never leaves a tag on the remote pointing at a commit it lacks.
        git_interactive(&repo, &["push", "--atomic", &remote, &refs[0], &refs[1]]).map_err(
            |e| {
                CliError::failure(format!(
                    "{e}; the commit and tag {tag_name} are local only. Push them with \
                 `git push --atomic {remote} {} {}`",
                    refs[0], refs[1]
                ))
            },
        )?;
        status("Pushed", &format!("{dest} and {tag_name} to {remote}"));
    }
    Ok(0)
}

/// Find the Cargo.toml that holds the app's version: its own `[package] version`, or the
/// `[workspace.package] version` of the nearest ancestor workspace when it inherits one.
fn version_source(root: &Path) -> Result<VersionSource, String> {
    let own = root.join("Cargo.toml");
    let doc = parse(&own)?;
    let version = doc
        .get("package")
        .and_then(|p| p.get("version"))
        .ok_or_else(|| format!("{}: no [package] version", own.display()))?;
    if version.is_str() {
        return Ok(VersionSource {
            file: own,
            table: &["package"],
        });
    }
    let inherited = version
        .get("workspace")
        .and_then(|w| w.as_bool())
        .unwrap_or(false);
    if !inherited {
        return Err(format!(
            "{}: [package] version is neither a string nor inherited from the workspace",
            own.display()
        ));
    }
    for dir in root.ancestors() {
        let candidate = dir.join("Cargo.toml");
        let Ok(doc) = parse(&candidate) else {
            continue;
        };
        if doc.get("workspace").is_some() {
            return Ok(VersionSource {
                file: candidate,
                table: &["workspace", "package"],
            });
        }
    }
    Err("Cargo.toml: version.workspace = true, but no ancestor declares a [workspace]".into())
}

fn parse(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn table<'a>(doc: &'a toml_edit::DocumentMut, path: &[&str]) -> Option<&'a toml_edit::Item> {
    path.iter()
        .try_fold(doc.as_item(), |item, key| item.get(key))
}

fn read_version(source: &VersionSource) -> Result<String, String> {
    let doc = parse(&source.file)?;
    table(&doc, source.table)
        .and_then(|t| t.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "{}: no [{}] version",
                source.file.display(),
                source.table.join(".")
            )
        })
}

fn write_version(source: &VersionSource, version: &str) -> Result<(), String> {
    let mut doc = parse(&source.file)?;
    let mut item = doc.as_item_mut();
    for key in source.table {
        item = &mut item[key];
    }
    // Replacing the value's contents, not the item, keeps any comment trailing it.
    let old = item["version"]
        .as_value_mut()
        .ok_or_else(|| format!("{}: version is not a value", source.file.display()))?;
    let decor = old.decor().clone();
    *old = toml_edit::Value::from(version);
    *old.decor_mut() = decor;
    std::fs::write(&source.file, doc.to_string()).map_err(|e| e.to_string())
}

/// Day.toml's `[app] build`, or the default it would take when absent.
fn read_build(day_toml: &Path) -> Result<u64, String> {
    let doc = parse(day_toml)?;
    match doc.get("app").and_then(|a| a.get("build")) {
        None => Ok(1),
        Some(b) => b
            .as_integer()
            .filter(|n| *n >= 0)
            .map(|n| n as u64)
            .ok_or_else(|| "Day.toml: [app] build is not a non-negative integer".to_string()),
    }
}

fn write_build(day_toml: &Path, build: u64) -> Result<(), String> {
    let mut doc = parse(day_toml)?;
    let app = doc
        .get_mut("app")
        .and_then(|a| a.as_table_like_mut())
        .ok_or("Day.toml: no [app] table")?;
    let n = i64::try_from(build).map_err(|_| format!("build {build} is too large"))?;
    match app.get_mut("build").and_then(|b| b.as_value_mut()) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = toml_edit::Value::from(n);
            *old.decor_mut() = decor;
        }
        None => {
            app.insert("build", toml_edit::value(n));
        }
    }
    std::fs::write(day_toml, doc.to_string()).map_err(|e| e.to_string())
}

/// Carry the new version into Cargo.lock, when there is one, so the next build does not rewrite it
/// and leave the release commit's tree dirty. `--workspace` touches only the workspace's own
/// entries; `--offline` keeps a version bump from reaching the network. Best-effort: a failure is
/// reported, and the next cargo invocation brings the lockfile up to date.
fn refresh_lockfile(cargo_toml: &Path) {
    let dir = cargo_toml.parent().unwrap_or(Path::new("."));
    if !dir.join("Cargo.lock").exists() {
        return;
    }
    let ok = Command::new("cargo")
        .current_dir(dir)
        .args(["update", "--workspace", "--offline", "--quiet"])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        status(
            "Warning",
            "Cargo.lock could not be refreshed offline; the next build will update it",
        );
    }
}

fn repo_root(dir: &Path) -> Result<PathBuf, CliError> {
    let top = git_capture(dir, &["rev-parse", "--show-toplevel"])
        .map_err(|_| CliError::failure(format!("{} is not in a git repository", dir.display())))?;
    Ok(PathBuf::from(top.trim()))
}

/// Where a push goes: the current branch's upstream remote and branch, else `origin` and the
/// branch's own name.
fn push_destination(repo: &Path) -> Result<(String, String), CliError> {
    let branch = git_capture(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| CliError::failure("HEAD is detached; check out a branch to push from"))?;
    let branch = branch.trim().to_string();
    let config = |key: &str| {
        git_capture(
            repo,
            &["config", "--get", &format!("branch.{branch}.{key}")],
        )
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    };
    let remote = config("remote").unwrap_or_else(|| "origin".into());
    let dest = config("merge").unwrap_or_else(|| format!("refs/heads/{branch}"));
    Ok((remote, dest))
}

/// A read-only git query, captured. None of these ever prompts, so there is nothing to forward.
fn git_capture(repo: &Path, args: &[&str]) -> Result<String, CliError> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| CliError::env(format!("git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(CliError::failure(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

fn git_ok(repo: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A git step that may talk to the user: stdin, stdout and stderr stay the terminal's (the
/// `Command` default), so a credential prompt, an SSH passphrase or a commit hook's question
/// reaches them as it would outside Day, with the masking git and ssh apply themselves.
fn git_interactive(repo: &Path, args: &[&str]) -> Result<(), CliError> {
    let ok = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .map_err(|e| CliError::env(format!("git: {e}")))?
        .success();
    if ok {
        Ok(())
    } else {
        Err(CliError::failure(format!("git {} failed", args[0])))
    }
}

fn display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_bump() {
        let v = Version::parse("2.1.13", true).unwrap();
        assert_eq!(v.bumped(Part::Patch).to_string(), "2.1.14");
        assert_eq!(v.bumped(Part::Minor).to_string(), "2.2.0");
        assert_eq!(v.bumped(Part::Major).to_string(), "3.0.0");
        // A pre-release in Cargo.toml bumps from its core; a set version must be plain.
        assert_eq!(
            Version::parse("1.4.0-beta.2", false)
                .unwrap()
                .bumped(Part::Patch)
                .to_string(),
            "1.4.1"
        );
        assert!(Version::parse("1.4.0-beta.2", true).is_err());
        for bad in ["1.2", "1.2.3.4", "1.02.3", "a.b.c", "", "1..3"] {
            assert!(Version::parse(bad, true).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn edits_keep_comments_and_layout() {
        let dir = std::env::temp_dir().join(format!("day-bump-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cargo = dir.join("Cargo.toml");
        std::fs::write(
            &cargo,
            "[workspace]\nmembers = []\n\n[workspace.package]\n# the release\nversion = \"2.1.13\" # bump me\n\n[package]\nname = \"app\"\nversion.workspace = true\n",
        )
        .unwrap();
        let day = dir.join("Day.toml");
        std::fs::write(
            &day,
            "schema = 1\n\n[app]\nid = \"a.b\"\nbuild = 41 # store build\n",
        )
        .unwrap();

        let source = version_source(&dir).unwrap();
        assert_eq!(source.table, &["workspace", "package"]);
        assert_eq!(read_version(&source).unwrap(), "2.1.13");
        write_version(&source, "2.2.0").unwrap();
        let text = std::fs::read_to_string(&cargo).unwrap();
        assert!(
            text.contains("# the release\nversion = \"2.2.0\" # bump me"),
            "{text}"
        );
        assert!(text.contains("version.workspace = true"), "{text}");

        assert_eq!(read_build(&day).unwrap(), 41);
        write_build(&day, 42).unwrap();
        assert_eq!(
            std::fs::read_to_string(&day).unwrap(),
            "schema = 1\n\n[app]\nid = \"a.b\"\nbuild = 42 # store build\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
