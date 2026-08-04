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
    /// Where the project sat inside the repository (`apps/showcase`, or empty at the root).
    /// Absent in artifacts packed before this was recorded — see `find_project_dir`.
    project: Option<String>,
    /// The app id the artifact declares, used to identify the project when `project` is absent.
    app_id: Option<String>,
    /// Present only when the buildinfo sidecar was found; without it tool gating is skipped.
    target: Option<String>,
    profile: Option<String>,
    tools: Vec<(String, String, String)>,
}

/// Read a JSON document, tolerating absence.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// What the SBOM says about the source: repository, commit, dirty flag, project path, app id.
/// The last two are optional — older artifacts predate them.
#[derive(Debug, PartialEq)]
struct SourceFacts {
    repository: String,
    commit: String,
    dirty: bool,
    project: Option<String>,
    app_id: Option<String>,
}

/// Pull the source facts from either SBOM format.
///
/// CycloneDX keeps them in `metadata.properties`; SPDX has no property bag, so they ride in the
/// application package's `sourceInfo`. Both are accepted because either format may be the only one
/// a project generates.
fn source_facts(doc: &serde_json::Value) -> Option<SourceFacts> {
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
            .filter(|v| v != "NOASSERTION")
    };
    Some(SourceFacts {
        repository: field("repository")?,
        commit: field("commit")?,
        dirty: info.contains("dirty=true"),
        // An empty prefix is the repository root, which is a real answer — keep it as Some("").
        project: info
            .split_whitespace()
            .find_map(|kv| kv.strip_prefix("project="))
            .filter(|v| *v != "NOASSERTION")
            .map(str::to_string),
        app_id: field("app-id"),
    })
}

/// Pull `day:*` properties out of a CycloneDX document.
fn cyclonedx_props(doc: &serde_json::Value) -> Option<SourceFacts> {
    let props = doc.get("metadata")?.get("properties")?.as_array()?;
    let get = |k: &str| {
        props
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(k))
            .and_then(|p| p.get("value"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    Some(SourceFacts {
        repository: get("day:repository")?,
        commit: get("day:commit")?,
        dirty: get("day:dirty").as_deref() == Some("true"),
        project: get("day:project"),
        app_id: get("day:app-id"),
    })
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
    if opened
        && let Some(found) = find_file(&inner, "day-sbom.cdx.json")
            .or_else(|| find_file(&inner, "day-sbom.spdx.json"))
        && let Some(doc) = read_json(&found)
    {
        return Ok(doc);
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
    let facts = source_facts(&sbom).ok_or_else(|| {
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
        repository: facts.repository,
        commit: facts.commit,
        dirty: facts.dirty,
        project: facts.project,
        app_id: facts.app_id,
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

/// This machine's version of a tool, by the key the buildinfo records it under. Delegates to the
/// same probe `day pack` used, so "matches" means the two ran identical code rather than two
/// implementations that agree by luck.
fn current_version(key: &str) -> Option<String> {
    crate::provenance::tool_version(key)
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

/// Compare the original artifact with the rebuilt one, at both tiers (§20.3).
///
/// The container verdict is a plain byte comparison. The payload verdict opens both containers and
/// compares their members after normalization, which is the only tier that can pass on a platform
/// whose artifacts carry a signature or a linker build ID.
fn compare(original: &Path, rebuilt: &Path, scratch: &Path) -> (Verdict, Verdict) {
    let (a, b) = match (digest(original), digest(rebuilt)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            let why = Verdict::Unchecked("could not hash one of the artifacts".into());
            return (
                Verdict::Unchecked("could not hash one of the artifacts".into()),
                why,
            );
        }
    };
    let container = if a == b {
        Verdict::Identical
    } else {
        Verdict::Differs(format!("{}… vs {}…", &a[..16], &b[..16]))
    };
    // Identical containers imply identical contents; no need to open them.
    if container == Verdict::Identical {
        return (Verdict::Identical, container);
    }

    let (ua, ub) = (scratch.join("cmp/a"), scratch.join("cmp/b"));
    let _ = std::fs::remove_dir_all(scratch.join("cmp"));
    if !unpack(original, &ua) || !unpack(rebuilt, &ub) {
        return (
            Verdict::Unchecked(format!(
                "no extractor for {} on this host — the compiled code was not compared",
                original
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("this format")
            )),
            container,
        );
    }
    match differing_members(&ua, &ub) {
        Ok(diffs) if diffs.is_empty() => (Verdict::Identical, container),
        Ok(diffs) => (
            Verdict::Differs(format!(
                "{} file(s) differ after normalization: {}",
                diffs.len(),
                diffs
                    .iter()
                    .take(4)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            container,
        ),
        Err(e) => (Verdict::Differs(e), container),
    }
}

/// Options for [`run`].
pub struct Options {
    pub force_tools: Vec<String>,
    pub keep: bool,
    /// Treat an unverifiable payload as a failure. CI sets this: a format this host cannot open
    /// means the compiled code went uncompared, and reporting that as success is a false pass.
    pub strict: bool,
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

    // The app may live in a subdirectory of the repository, and a repository may hold SEVERAL
    // (day's own holds three apps plus the scaffold templates, which carry a Day.toml but no
    // Cargo.toml). Packing the wrong one produces a confusing failure deep inside cargo, so this
    // is resolved explicitly rather than by first-hit search.
    let project_dir = find_project_dir(&src, prov.project.as_deref(), prov.app_id.as_deref())?;
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

    let (payload, container) = compare(&artifact, &rebuilt, &scratch);
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
    // format and on Apple platforms generally (§20.3). Under --strict an unverifiable payload
    // fails too: "could not check" is not "checked and matched".
    Ok(match payload {
        Verdict::Differs(_) => 1,
        Verdict::Unchecked(_) if opts.strict => 1,
        _ => 0,
    })
}

/// The app id a `Day.toml` declares, or `None` if it is unreadable or declares none.
fn day_toml_app_id(day_toml: &Path) -> Option<String> {
    let text = std::fs::read_to_string(day_toml).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;
    doc.get("app")?.get("id")?.as_str().map(str::to_string)
}

/// Every Day project in a checkout, deepest-last and sorted, so the answer never depends on
/// filesystem order. A directory qualifies only with BOTH manifests: `crates/day-cli/templates/app`
/// has a `Day.toml` and no `Cargo.toml`, and packing it fails with a missing-manifest error that
/// says nothing about the real problem.
fn day_projects(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.join("Day.toml").is_file() && dir.join("Cargo.toml").is_file() {
            found.push(dir.clone());
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut kids: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.ends_with(".git") && !p.ends_with("target"))
            .collect();
        kids.sort();
        stack.extend(kids);
    }
    found.sort();
    found
}

/// Locate the project to rebuild inside a checkout.
///
/// `recorded` is the path the SBOM carries (`apps/showcase`, or empty for the repository root) and
/// is authoritative: the packing run knew exactly where it stood. `app_id` identifies the project
/// in artifacts packed before that was recorded — a search, but one that checks the id it finds
/// rather than taking the first `Day.toml` it trips over.
fn find_project_dir(
    root: &Path,
    recorded: Option<&str>,
    app_id: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(rel) = recorded {
        let dir = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        if dir.join("Day.toml").is_file() {
            return Ok(dir);
        }
        return Err(format!(
            "the artifact was packed in `{}` of {}, which does not exist at this commit",
            if rel.is_empty() { "." } else { rel },
            root.display(),
        ));
    }

    let projects = day_projects(root);
    if let Some(want) = app_id
        && let Some(hit) = projects
            .iter()
            .find(|p| day_toml_app_id(&p.join("Day.toml")).as_deref() == Some(want))
    {
        return Ok(hit.clone());
    }
    match projects.len() {
        0 => Err(format!("no Day project in {}", root.display())),
        1 => Ok(projects[0].clone()),
        _ => Err(format!(
            "this SBOM records neither the project path nor an app id, and the repository holds \
             {} Day projects ({}). Re-pack with a current day-cli, which records the path.",
            projects.len(),
            projects
                .iter()
                .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

// --- Two-tier comparison (§20.3) -----------------------------------------------------------------
// Ported from the retired scripts/ci/repro-check.sh (last present at 07dc6ac) so `day rebuild` is
// self-contained: a verification command
// that needs a checkout of Day's own CI scripts to reach a verdict is not much use to someone
// holding only an artifact.

const LC_UUID: u32 = 0x1B;

/// Zero the `LC_UUID` load command in a Mach-O, in place, for thin and fat binaries.
///
/// Apple's linker derives the UUID from its inputs including object-file paths, so the same
/// sources built in two directories differ by these 16 bytes and nothing else. TN3178 documents
/// the field; zeroing it lets the comparison see the code rather than the build location.
fn zero_macho_uuid(buf: &mut [u8]) -> usize {
    fn u32_at(b: &[u8], at: usize, big: bool) -> Option<u32> {
        let v = b.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(v)
        } else {
            u32::from_le_bytes(v)
        })
    }
    fn thin(buf: &mut [u8], off: usize) -> usize {
        // The first word read little-endian identifies both the byte order and the width.
        let Some(magic) = u32_at(buf, off, false) else {
            return 0;
        };
        let (big, is64) = match magic {
            0xFEED_FACF => (false, true),
            0xFEED_FACE => (false, false),
            0xCFFA_EDFE => (true, true),
            0xCEFA_EDFE => (true, false),
            _ => return 0,
        };
        let Some(ncmds) = u32_at(buf, off + 16, big) else {
            return 0;
        };
        let mut lc = off + if is64 { 32 } else { 28 };
        let mut zeroed = 0;
        for _ in 0..ncmds {
            let (Some(cmd), Some(size)) = (u32_at(buf, lc, big), u32_at(buf, lc + 4, big)) else {
                break;
            };
            if size < 8 || lc + size as usize > buf.len() {
                break;
            }
            if cmd == LC_UUID && lc + 24 <= buf.len() {
                buf[lc + 8..lc + 24].fill(0);
                zeroed += 1;
            }
            lc += size as usize;
        }
        zeroed
    }
    match u32_at(buf, 0, true) {
        // Fat header: the arch table gives each slice's offset.
        Some(m @ (0xCAFE_BABE | 0xBEBA_FECA)) => {
            let big = m == 0xCAFE_BABE;
            let Some(n) = u32_at(buf, 4, big) else {
                return 0;
            };
            let mut zeroed = 0;
            for i in 0..n as usize {
                let arch = 8 + i * 20;
                let Some(slice_off) = u32_at(buf, arch + 8, big) else {
                    break;
                };
                zeroed += thin(buf, slice_off as usize);
            }
            zeroed
        }
        _ => thin(buf, 0),
    }
}

/// Strip the build-path-derived metadata from a file so the comparison sees code.
fn normalize(path: &Path) -> Result<(), String> {
    let Ok(head) = std::fs::read(path) else {
        return Ok(());
    };
    let looks_macho = head.len() > 4
        && matches!(
            u32::from_le_bytes(head[0..4].try_into().unwrap_or_default()),
            0xFEED_FACF | 0xFEED_FACE | 0xCFFA_EDFE | 0xCEFA_EDFE
        )
        || head.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE]);
    if !looks_macho {
        return Ok(());
    }
    // The ad-hoc signature covers the UUID, so it has to go first or it will not match either.
    let _ = Command::new("codesign")
        .args(["--remove-signature"])
        .arg(path)
        .output();
    let mut buf = std::fs::read(path).map_err(|e| e.to_string())?;
    if zero_macho_uuid(&mut buf) > 0 {
        std::fs::write(path, &buf).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Unpack a container so its members can be compared. `false` when the format cannot be opened here.
fn unpack(container: &Path, dest: &Path) -> bool {
    let _ = std::fs::create_dir_all(dest);
    match container.extension().and_then(|e| e.to_str()) {
        Some("ipa" | "apk" | "aab" | "hap" | "msix" | "zip") => Command::new("unzip")
            .args(["-q", "-o"])
            .arg(container)
            .arg("-d")
            .arg(dest)
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        Some("dmg") if cfg!(target_os = "macos") => mount_dmg(container, dest).is_ok(),
        // A .flatpak is an OSTree static delta, not an archive: import it into a scratch repo and
        // check the ref out. Needs `flatpak` and `ostree`, which any host that can build one has.
        Some("flatpak") => unpack_flatpak(container, dest),
        _ => false,
    }
}

/// Import a single-file flatpak bundle into a scratch repo and check its ref out to `dest`.
fn unpack_flatpak(bundle: &Path, dest: &Path) -> bool {
    let repo = dest.with_extension("ostree-repo");
    let _ = std::fs::remove_dir_all(&repo);
    let ok = Command::new("ostree")
        .arg("init")
        .arg("--mode=archive")
        .arg(format!("--repo={}", repo.display()))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return false;
    }
    let imported = Command::new("flatpak")
        .args(["build-import-bundle", "--no-update-summary"])
        .arg(&repo)
        .arg(bundle)
        .output();
    let Ok(out) = imported else { return false };
    if !out.status.success() {
        return false;
    }
    // build-import-bundle prints the ref it created; fall back to asking the repo.
    let refs = Command::new("ostree")
        .arg(format!("--repo={}", repo.display()))
        .arg("refs")
        .output();
    let Ok(refs) = refs else { return false };
    let Some(r) = String::from_utf8_lossy(&refs.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
    else {
        return false;
    };
    Command::new("ostree")
        .arg(format!("--repo={}", repo.display()))
        .args(["checkout", "--union", &r])
        .arg(dest)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Every regular file under `root`, relative to it, sorted — the comparison unit.
fn file_list(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
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
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
    out.sort();
    out
}

/// Compare two unpacked trees after normalizing each file. Returns the members that still differ.
fn differing_members(a: &Path, b: &Path) -> Result<Vec<String>, String> {
    let (la, lb) = (file_list(a), file_list(b));
    if la != lb {
        return Err("the two builds produced different sets of files".into());
    }
    let mut diffs = Vec::new();
    for rel in la {
        let (pa, pb) = (a.join(&rel), b.join(&rel));
        normalize(&pa)?;
        normalize(&pb)?;
        if std::fs::read(&pa).ok() != std::fs::read(&pb).ok() {
            diffs.push(rel.display().to_string());
        }
    }
    Ok(diffs)
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
            project_path: Some("apps/showcase".into()),
            components: Vec::new(),
        };
        let from_cdx = source_facts(&crate::provenance::cyclonedx(&sbom)).expect("cyclonedx");
        let from_spdx = source_facts(&crate::provenance::spdx(&sbom)).expect("spdx");
        assert_eq!(from_cdx, from_spdx, "the two formats must agree");
        assert_eq!(from_cdx.repository, "https://example.invalid/repo");
        assert_eq!(from_cdx.commit, "abc123");
        assert!(!from_cdx.dirty);
        // The project path decides WHICH app in the repository gets rebuilt, so it has to survive
        // both formats — SPDX has no property bag and carries it inside `sourceInfo`.
        assert_eq!(from_cdx.project.as_deref(), Some("apps/showcase"));
    }

    /// The bug this replaced: `find_project_dir` returned the first `Day.toml` a directory walk
    /// tripped over, so rebuilding day's own showcase packed `apps/daylite` on one runner and
    /// `crates/day-cli/templates/app` (a template with no Cargo.toml) on another.
    #[test]
    fn the_project_is_chosen_by_record_then_app_id_never_by_walk_order() {
        let tmp = std::env::temp_dir().join(format!("day-rebuild-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mk = |rel: &str, id: Option<&str>| {
            let dir = tmp.join(rel);
            std::fs::create_dir_all(&dir).expect("mkdir");
            if let Some(id) = id {
                std::fs::write(
                    dir.join("Day.toml"),
                    format!("schema = 1\n[app]\nid = \"{id}\"\n"),
                )
                .expect("Day.toml");
                std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n")
                    .expect("Cargo.toml");
            }
        };
        // Sorts BEFORE the real app, which is what made first-hit search pick it.
        mk("apps/daylite", Some("dev.daybrite.daylite"));
        mk("apps/showcase", Some("dev.daybrite.showcase"));
        // A scaffold template: Day.toml, no Cargo.toml. Never a rebuild candidate.
        let tpl = tmp.join("crates/day-cli/templates/app");
        std::fs::create_dir_all(&tpl).expect("mkdir");
        std::fs::write(
            tpl.join("Day.toml"),
            "schema = 1\n[app]\nid = \"{{app_id}}\"\n",
        )
        .expect("template");

        // Recorded path wins outright.
        assert_eq!(
            find_project_dir(&tmp, Some("apps/showcase"), None).expect("recorded"),
            tmp.join("apps/showcase"),
        );
        // An empty recorded path is a real answer — the project IS the repository root, which is
        // how a scaffolded single-app repo looks. It must not be read as "nothing recorded".
        mk("", Some("dev.example.root"));
        assert_eq!(find_project_dir(&tmp, Some(""), None).expect("root"), tmp);
        // Without it, the app id identifies the project among the several in the repository.
        assert_eq!(
            find_project_dir(&tmp, None, Some("dev.daybrite.showcase")).expect("by id"),
            tmp.join("apps/showcase"),
        );
        // Neither: refuse and name the candidates rather than pack an arbitrary one.
        let err = find_project_dir(&tmp, None, None).expect_err("ambiguous");
        assert!(
            err.contains("apps/daylite") && err.contains("apps/showcase"),
            "{err}"
        );
        assert!(
            !err.contains("templates"),
            "a template is not a candidate: {err}"
        );
        // A recorded path that is not in this commit is an error, not a silent fallback.
        assert!(find_project_dir(&tmp, Some("apps/gone"), None).is_err());

        let _ = std::fs::remove_dir_all(&tmp);
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
            project_path: Some("apps/showcase".into()),
            components: Vec::new(),
        };
        assert!(source_facts(&crate::provenance::spdx(&sbom)).unwrap().dirty);
        assert!(
            source_facts(&crate::provenance::cyclonedx(&sbom))
                .unwrap()
                .dirty
        );
    }

    /// The Rust port must agree with the retired scripts/ci/macho-normalize.py (last present at
    /// 07dc6ac), which was validated against
    /// two real showcase binaries that differed only in LC_UUID.
    #[test]
    fn zeroing_the_uuid_matches_the_python_normalizer() {
        // A minimal 64-bit little-endian Mach-O with one LC_UUID load command.
        let mut buf = vec![0u8; 64];
        buf[0..4].copy_from_slice(&0xFEED_FACFu32.to_le_bytes()); // magic, LE 64-bit
        buf[16..20].copy_from_slice(&1u32.to_le_bytes()); // ncmds
        let lc = 32;
        buf[lc..lc + 4].copy_from_slice(&LC_UUID.to_le_bytes());
        buf[lc + 4..lc + 8].copy_from_slice(&24u32.to_le_bytes()); // cmdsize
        buf[lc + 8..lc + 24].copy_from_slice(&[0xAB; 16]); // the uuid
        assert_eq!(zero_macho_uuid(&mut buf), 1, "one uuid zeroed");
        assert_eq!(&buf[lc + 8..lc + 24], &[0u8; 16], "uuid cleared");
        // The magic and load-command header must survive untouched.
        assert_eq!(&buf[0..4], &0xFEED_FACFu32.to_le_bytes());
        assert_eq!(&buf[lc..lc + 4], &LC_UUID.to_le_bytes());
    }

    /// Endianness detection was the bug in the first Python attempt: reading the magic big-endian
    /// and mapping it backwards corrupted the load-command walk.
    #[test]
    fn a_non_macho_file_is_left_alone() {
        let mut buf = b"#!/bin/sh\necho hello\n".to_vec();
        let before = buf.clone();
        assert_eq!(zero_macho_uuid(&mut buf), 0);
        assert_eq!(buf, before);
    }

    #[test]
    fn an_unknown_target_is_named_in_the_error() {
        let err = preflight("not-a-target").unwrap_err();
        assert!(err.contains("not-a-target"), "{err}");
    }
}
