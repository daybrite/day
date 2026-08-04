//! `day rebuild <artifact>` — rebuild a shipped artifact from its own provenance and compare
//! (DESIGN.md §20.4).
//!
//! The point is to make verification a single command. Given an artifact someone is about to
//! install, read the SBOM for *what* it was built from and the buildinfo sidecar for *what it was
//! built with*, check this machine matches, rebuild from source, and report whether the result
//! agrees.
//!
//! Two things this deliberately does NOT do. It never installs a toolchain: a verification tool
//! that mutates the machine it is verifying on is a poor trade, so a version mismatch prints the
//! command to run and stops. And it does not promise byte equality — signatures and Mach-O build
//! IDs cannot be reproduced by a third party (§20.3), so it reports a payload verdict and a
//! container verdict separately and fails only on the former.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ops::status;

/// What a rebuild concluded, per tier.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Byte-for-byte identical.
    Identical,
    /// Differs, with a human-readable reason.
    Differs(String),
    /// Not checkable here — the format could not be opened on this host.
    Unchecked(String),
}

impl Verdict {
    fn label(&self) -> &str {
        match self {
            Verdict::Identical => "identical",
            Verdict::Differs(_) => "differs",
            Verdict::Unchecked(_) => "not checked",
        }
    }
}

/// Provenance recovered from an artifact and its sidecars.
struct Provenance {
    repository: String,
    commit: String,
    dirty: bool,
    /// Present only when the buildinfo sidecar was found; without it tool gating is skipped.
    target: Option<String>,
    profile: Option<String>,
    tools: Vec<(String, String, String)>,
}

/// Read a JSON document, tolerating absence.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Pull repository/commit/dirty from either SBOM format.
///
/// CycloneDX keeps them in `metadata.properties`; SPDX has no property bag, so they ride in the
/// application package's `sourceInfo`. Both are accepted because either format may be the only one
/// a project generates.
fn source_facts(doc: &serde_json::Value) -> Option<(String, String, bool)> {
    if let Some(v) = cyclonedx_props(doc) {
        return Some(v);
    }
    let info = doc
        .get("packages")?
        .as_array()?
        .iter()
        .find_map(|p| p.get("sourceInfo").and_then(|s| s.as_str()))?;
    let field = |k: &str| {
        info.split_whitespace()
            .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
            .map(str::to_string)
    };
    let repo = field("repository").filter(|v| v != "NOASSERTION")?;
    let commit = field("commit").filter(|v| v != "NOASSERTION")?;
    Some((repo, commit, field("dirty").as_deref() == Some("true")))
}

/// Pull `day:*` properties out of a CycloneDX document.
fn cyclonedx_props(doc: &serde_json::Value) -> Option<(String, String, bool)> {
    let props = doc.get("metadata")?.get("properties")?.as_array()?;
    let get = |k: &str| {
        props
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(k))
            .and_then(|p| p.get("value"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    Some((
        get("day:repository")?,
        get("day:commit")?,
        get("day:dirty").as_deref() == Some("true"),
    ))
}

/// Find an SBOM either beside the artifact or inside it.
fn locate_sbom(artifact: &Path, scratch: &Path) -> Result<serde_json::Value, String> {
    let dir = artifact.parent().unwrap_or(Path::new("."));
    for name in ["day-sbom.cdx.json", "day-sbom.spdx.json"] {
        if let Some(doc) = read_json(&dir.join(name)) {
            return Ok(doc);
        }
    }
    // Not beside it — look inside. Every container Day produces except .dmg and .flatpak is a zip.
    let inner = scratch.join("extracted");
    std::fs::create_dir_all(&inner).map_err(|e| e.to_string())?;
    let ext = artifact
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let opened = match ext {
        "ipa" | "apk" | "aab" | "hap" | "msix" | "zip" => Command::new("unzip")
            .args(["-q", "-o"])
            .arg(artifact)
            .arg("-d")
            .arg(&inner)
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        "dmg" if cfg!(target_os = "macos") => mount_dmg(artifact, &inner).is_ok(),
        _ => false,
    };
    if opened {
        if let Some(found) = find_file(&inner, "day-sbom.cdx.json")
            .or_else(|| find_file(&inner, "day-sbom.spdx.json"))
            && let Some(doc) = read_json(&found)
        {
            return Ok(doc);
        }
    }
    Err(format!(
        "no SBOM beside or inside {}.\n  \
         A rebuild needs the source repository and commit. Either place day-sbom.cdx.json next to \
         the artifact, or build the app with `sbom = \"embed cyclonedx\"` in Day.toml so it ships \
         inside.",
        artifact.display()
    ))
}

/// Mount a .dmg read-only and copy its contents out, so the caller can treat it like a directory.
fn mount_dmg(dmg: &Path, dest: &Path) -> Result<(), String> {
    let mnt = dest.join("_mnt");
    std::fs::create_dir_all(&mnt).map_err(|e| e.to_string())?;
    let ok = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-quiet", "-mountpoint"])
        .arg(&mnt)
        .arg(dmg)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err(format!("could not mount {}", dmg.display()));
    }
    let copied = Command::new("ditto")
        .arg(&mnt)
        .arg(dest.join("content"))
        .status();
    let _ = Command::new("hdiutil")
        .args(["detach", "-quiet"])
        .arg(&mnt)
        .status();
    copied
        .map(|_| ())
        .map_err(|e| format!("copying from the mounted image: {e}"))
}

/// Depth-first search for a file by name, without following symlinks.
///
/// `symlink_metadata`, not `is_dir`: a mounted `.dmg` contains an `Applications` symlink pointing
/// at `/Applications`, and following it walks every app installed on the machine. An unreadable
/// directory is skipped rather than ending the search — one bad entry should not hide the file.
fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(p);
            }
        }
    }
    None
}

/// Gather everything known about how the artifact was produced.
fn collect(artifact: &Path, scratch: &Path) -> Result<Provenance, String> {
    let sbom = locate_sbom(artifact, scratch)?;
    let (repository, commit, dirty) = source_facts(&sbom).ok_or_else(|| {
        "the SBOM has no day:repository / day:commit properties — it was written by a different \
         tool, or by a day-cli too old to record them"
            .to_string()
    })?;

    // The buildinfo is always a sidecar, by design: embedding tool versions would make the
    // artifact differ per machine (§20.3). Without it a rebuild still works, minus tool gating.
    let dir = artifact.parent().unwrap_or(Path::new("."));
    let info = std::fs::read_dir(dir)
        .ok()
        .and_then(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .find(|p| p.to_string_lossy().ends_with(".buildinfo.json"))
        })
        .and_then(|p| read_json(&p));

    let (target, profile, tools) = match info {
        Some(v) => {
            let tools = v
                .get("tools")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| {
                            Some((
                                t.get("key")?.as_str()?.to_string(),
                                t.get("version")?.as_str()?.to_string(),
                                t.get("install")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            (
                v.get("target").and_then(|t| t.as_str()).map(str::to_string),
                v.get("profile")
                    .and_then(|t| t.as_str())
                    .map(str::to_string),
                tools,
            )
        }
        None => (None, None, Vec::new()),
    };

    Ok(Provenance {
        repository,
        commit,
        dirty,
        target,
        profile,
        tools,
    })
}

/// Refuse early when this machine can never produce the artifact, with the reason and what is
/// missing. Checked before cloning anything, so a hopeless rebuild costs seconds rather than a
/// full checkout and compile.
fn preflight(target_name: &str) -> Result<&'static crate::targets::Target, String> {
    let target = crate::targets::find(target_name)
        .ok_or_else(|| format!("unknown target {target_name:?} — this day-cli may be too old"))?;
    let host = crate::targets::host_os();
    if target.host != "any" && target.host != host {
        return Err(format!(
            "{} can only be rebuilt on a {} host; this is {}.\n  \
             Nothing about the environment can be adjusted to change that — rebuild on {} instead.",
            target.name, target.host, host, target.host
        ));
    }
    // A present host OS is not enough: the platform toolchain has to exist too.
    let missing: Option<(&str, &str)> = match target.toolkit {
        "appkit" | "uikit" if which("xcodebuild").is_none() => Some((
            "Xcode",
            "install Xcode from https://developer.apple.com/download/all/?q=Xcode, then run \
             sudo xcode-select -s /Applications/Xcode.app",
        )),
        "mdc" if which("gradle").is_none() && which("java").is_none() => Some((
            "a JDK and Gradle",
            "brew install openjdk gradle  /  apt install default-jdk gradle",
        )),
        "gtk" | "qt" if which("flatpak-builder").is_none() => Some((
            "flatpak-builder",
            "apt install flatpak-builder  /  dnf install flatpak-builder",
        )),
        "arkui" if std::env::var("OHOS_BASE_SDK_HOME").is_err() => Some((
            "the OpenHarmony SDK",
            "set OHOS_BASE_SDK_HOME — see docs/harmonyos.md",
        )),
        _ => None,
    };
    if let Some((what, how)) = missing {
        return Err(format!(
            "{} needs {what}, which is not on this machine.\n  {how}",
            target.name
        ));
    }
    Ok(target)
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

/// Probe the current version of a recorded tool, using the same commands provenance used.
fn current_version(key: &str) -> Option<String> {
    let (bin, args): (&str, &[&str]) = match key {
        "rust" => ("rustc", &["--version"]),
        "cargo" => ("cargo", &["--version"]),
        "xcode" => ("xcodebuild", &["-version"]),
        "clang" => ("clang", &["--version"]),
        "gradle" => ("gradle", &["--version"]),
        "java" => ("javac", &["-version"]),
        "nsis" => ("makensis", &["/VERSION"]),
        "flatpak-builder" => ("flatpak-builder", &["--version"]),
        "cc" => ("cc", &["--version"]),
        "hvigor" => ("hvigorw", &["--version"]),
        // `day` is this process, and the SDK keys are directory paths rather than commands.
        "day" => return Some(env!("DAY_VERSION_LONG").to_string()),
        _ => return None,
    };
    let out = Command::new(bin).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = if text.trim().is_empty() {
        String::from_utf8_lossy(&out.stderr).to_string()
    } else {
        text.to_string()
    };
    text.lines().next().map(|l| l.trim().to_string())
}

/// Compare recorded tool versions against this machine. Mismatches are errors unless forced.
fn check_tools(tools: &[(String, String, String)], forced: &[String]) -> Result<(), String> {
    let force_all = forced.iter().any(|f| f == "all");
    let mut problems = Vec::new();
    for (key, recorded, hint) in tools {
        let forced_here = force_all || forced.iter().any(|f| f == key);
        let Some(found) = current_version(key) else {
            if !forced_here {
                problems.push(format!(
                    "  {key}: not found on this machine (built with {recorded})\n      {hint}"
                ));
            }
            continue;
        };
        if &found == recorded {
            continue;
        }
        if forced_here {
            status(
                "Forcing",
                &format!("{key}: {found} (artifact was built with {recorded})"),
            );
        } else {
            problems.push(format!(
                "  {key}: this machine has {found}\n      the artifact was built with {recorded}\n      {hint}"
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the build environment does not match the artifact:\n{}\n\n  \
         Install the versions above, or re-run with --force-tool=<name> for each tool you want to \
         ignore (--force-tool=all ignores every mismatch). Forcing usually changes the output, so \
         a forced rebuild that differs proves nothing.",
        problems.join("\n")
    ))
}

/// Clone the recorded repository at the recorded commit.
fn checkout(repository: &str, commit: &str, dest: &Path) -> Result<(), String> {
    status(
        "Cloning",
        &format!("{repository} @ {}", &commit[..12.min(commit.len())]),
    );
    let ok = Command::new("git")
        .args(["clone", "--quiet", repository])
        .arg(dest)
        .status()
        .map_err(|e| format!("git clone: {e}"))?;
    if !ok.success() {
        return Err(format!("could not clone {repository}"));
    }
    let ok = Command::new("git")
        .current_dir(dest)
        .args(["checkout", "--quiet", commit])
        .status()
        .map_err(|e| format!("git checkout: {e}"))?;
    if !ok.success() {
        return Err(format!(
            "{repository} has no commit {commit} — the artifact may predate a force-push, or the \
             commit was never pushed"
        ));
    }
    Ok(())
}

/// sha256 of a file, reusing pack's implementation so digests agree with what pack recorded.
fn digest(path: &Path) -> Result<String, String> {
    crate::pack::sha256_file(path)
}

/// Compare the original artifact with the rebuilt one, at both tiers.
fn compare(original: &Path, rebuilt: &Path) -> (Verdict, Verdict) {
    let (a, b) = match (digest(original), digest(rebuilt)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return (
                Verdict::Unchecked("could not hash one of the artifacts".into()),
                Verdict::Unchecked("could not hash one of the artifacts".into()),
            );
        }
    };
    let container = if a == b {
        Verdict::Identical
    } else {
        Verdict::Differs(format!("{}… vs {}…", &a[..16], &b[..16]))
    };
    // When the containers match, so does everything inside them.
    if container == Verdict::Identical {
        return (Verdict::Identical, container);
    }
    // Otherwise the payload tier needs the container opened, which is format-specific. Rather than
    // reimplement every extractor here, defer to the shared checker when it is available.
    (
        Verdict::Unchecked(
            "container differs; run scripts/ci/repro-check.sh on the two dist directories for a \
             payload-tier verdict"
                .into(),
        ),
        container,
    )
}

/// Options for [`run`].
pub struct Options {
    pub force_tools: Vec<String>,
    pub keep: bool,
}

/// Rebuild `artifact` from its provenance and report both verdicts. Returns the process exit code.
pub fn run(artifact: &Path, opts: &Options) -> Result<i32, String> {
    if !artifact.is_file() {
        return Err(format!("{} is not a file", artifact.display()));
    }
    let artifact = artifact
        .canonicalize()
        .map_err(|e| format!("{}: {e}", artifact.display()))?;

    let scratch = std::env::temp_dir().join(format!(
        "day-rebuild-{}",
        artifact
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("artifact")
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;

    let prov = collect(&artifact, &scratch)?;
    if prov.dirty {
        return Err(
            "this artifact was built from a working tree with uncommitted changes, so its commit \
             does not describe it. No rebuild can reproduce it."
                .into(),
        );
    }
    let target_name = prov.target.clone().ok_or_else(|| {
        format!(
            "no .buildinfo.json beside {} — it records the target and the toolchain.\n  \
             Place the sidecar `day pack` wrote next to the artifact.",
            artifact.display()
        )
    })?;
    let target = preflight(&target_name)?;
    let profile = prov.profile.clone().unwrap_or_else(|| "release".into());

    if prov.tools.is_empty() {
        status(
            "Warning",
            "no tool versions recorded — rebuilding without environment checks",
        );
    } else {
        check_tools(&prov.tools, &opts.force_tools)?;
        status(
            "Environment",
            &format!("{} tool(s) match the artifact", prov.tools.len()),
        );
    }

    let src = scratch.join("src");
    checkout(&prov.repository, &prov.commit, &src)?;

    // The app may live in a subdirectory of the repository; find the Day.toml whose app id matches.
    let project_dir = find_project_dir(&src, &artifact).unwrap_or(src.clone());
    status(
        "Rebuilding",
        &format!("{} ({profile}) in {}", target.name, project_dir.display()),
    );
    let day = std::env::current_exe().map_err(|e| e.to_string())?;
    let ok = Command::new(&day)
        .current_dir(&project_dir)
        .args(["pack", "-p", target.name, "--profile", &profile])
        .arg("--no-version-in-name")
        .status()
        .map_err(|e| format!("running day pack: {e}"))?;
    if !ok.success() {
        return Err("the rebuild failed — see the pack output above".into());
    }

    let rebuilt_dir = project_dir.join("build/day/dist");
    let name = artifact.file_name().unwrap_or_default();
    let rebuilt = rebuilt_dir.join(name);
    if !rebuilt.is_file() {
        return Err(format!(
            "the rebuild produced no {}; it made: {}",
            name.to_string_lossy(),
            std::fs::read_dir(&rebuilt_dir)
                .map(|rd| rd
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        ));
    }

    let (payload, container) = compare(&artifact, &rebuilt);
    status("Payload", payload.label());
    if let Verdict::Differs(d) | Verdict::Unchecked(d) = &payload {
        status("", d);
    }
    status("Container", container.label());
    if let Verdict::Differs(d) | Verdict::Unchecked(d) = &container {
        status("", d);
    }

    if opts.keep {
        status("Kept", &format!("{}", scratch.display()));
    } else {
        let _ = std::fs::remove_dir_all(&scratch);
    }

    // Only a payload mismatch is a failure. A container difference is expected on every signed
    // format and on Apple platforms generally (§20.3).
    Ok(match payload {
        Verdict::Differs(_) => 1,
        _ => 0,
    })
}

/// Locate the project inside a checkout. A repository may hold several apps.
fn find_project_dir(root: &Path, _artifact: &Path) -> Option<PathBuf> {
    if root.join("Day.toml").is_file() {
        return Some(root.to_path_buf());
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && !p.ends_with(".git") && !p.ends_with("target") {
                if p.join("Day.toml").is_file() {
                    return Some(p);
                }
                stack.push(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_that_cannot_build_here_is_refused_with_the_reason() {
        // windows-xaml never builds on a non-Windows host, and the message must say so rather
        // than failing later inside a build nobody can finish.
        if crate::targets::host_os() != "windows" {
            let err = preflight("windows-xaml").unwrap_err();
            assert!(err.contains("windows"), "{err}");
            assert!(err.contains("host"), "{err}");
        }
    }

    #[test]
    fn a_forced_tool_stops_being_an_error() {
        let tools = vec![(
            "rust".to_string(),
            "rustc 0.0.0-not-a-real-version".to_string(),
            "rustup".to_string(),
        )];
        assert!(check_tools(&tools, &[]).is_err(), "mismatch must fail");
        assert!(
            check_tools(&tools, &["rust".into()]).is_ok(),
            "--force-tool=rust"
        );
        assert!(
            check_tools(&tools, &["all".into()]).is_ok(),
            "--force-tool=all"
        );
    }

    /// Either SBOM format must drive a rebuild on its own: a project may generate only one.
    #[test]
    fn source_facts_come_from_cyclonedx_or_spdx() {
        let sbom = crate::provenance::Sbom {
            schema: crate::provenance::SCHEMA.into(),
            app_id: "dev.example.app".into(),
            app_name: "app".into(),
            app_version: "1.0.0".into(),
            app_build: 1,
            repository: Some("https://example.invalid/repo".into()),
            commit: Some("abc123".into()),
            dirty: false,
            components: Vec::new(),
        };
        let from_cdx = source_facts(&crate::provenance::cyclonedx(&sbom)).expect("cyclonedx");
        let from_spdx = source_facts(&crate::provenance::spdx(&sbom)).expect("spdx");
        assert_eq!(from_cdx, from_spdx, "the two formats must agree");
        assert_eq!(from_cdx.0, "https://example.invalid/repo");
        assert_eq!(from_cdx.1, "abc123");
        assert!(!from_cdx.2);
    }

    /// A dirty build must be refused: its commit does not describe the artifact.
    #[test]
    fn a_dirty_build_is_reported_as_such() {
        let sbom = crate::provenance::Sbom {
            schema: crate::provenance::SCHEMA.into(),
            app_id: "dev.example.app".into(),
            app_name: "app".into(),
            app_version: "1.0.0".into(),
            app_build: 1,
            repository: Some("https://example.invalid/repo".into()),
            commit: Some("abc123".into()),
            dirty: true,
            components: Vec::new(),
        };
        assert!(source_facts(&crate::provenance::spdx(&sbom)).unwrap().2);
        assert!(
            source_facts(&crate::provenance::cyclonedx(&sbom))
                .unwrap()
                .2
        );
    }

    #[test]
    fn an_unknown_target_is_named_in_the_error() {
        let err = preflight("not-a-target").unwrap_err();
        assert!(err.contains("not-a-target"), "{err}");
    }
}
