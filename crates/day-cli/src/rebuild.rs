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
    /// Where the project sat inside the repository (`apps/example`, or empty at the root).
    /// Absent in artifacts packed before this was recorded — see `find_project_dir`.
    project: Option<String>,
    /// The app id the artifact declares, used to identify the project when `project` is absent.
    app_id: Option<String>,
    /// Present only when the buildinfo sidecar was found; without it tool gating is skipped.
    target: Option<String>,
    profile: Option<String>,
    tools: Vec<(String, String, String)>,
    /// Environment that shaped the original build, re-applied to the rebuild.
    inputs: Vec<(String, String)>,
    /// Staged payload digests, keyed by path relative to the payload root. The payload tier falls
    /// back to these when the container cannot be opened here.
    payload: Vec<(String, String)>,
}

/// `[{k: …, v: …}, …]` out of a buildinfo array, as pairs.
fn kv_list(doc: &serde_json::Value, array: &str, k: &str, v: &str) -> Vec<(String, String)> {
    doc.get(array)
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    Some((
                        e.get(k)?.as_str()?.to_string(),
                        e.get(v)?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
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
///
/// A sidecar is named after the artifact (`<artifact>.sbom-cdx.json`, §20.4), so it is looked up
/// exactly rather than by scanning the directory — a release directory holds every target's.
/// The bare `day-sbom.*.json` names are the EMBEDDED spelling, and are also what packs before
/// this naming existed wrote beside the artifact; both fall back to them.
fn locate_sbom(artifact: &Path, scratch: &Path) -> Result<serde_json::Value, String> {
    let dir = artifact.parent().unwrap_or(Path::new("."));
    let beside = [
        crate::pack::naming::sidecar(artifact, "sbom-cdx.json"),
        crate::pack::naming::sidecar(artifact, "sbom-spdx.json"),
        dir.join("day-sbom.cdx.json"),
        dir.join("day-sbom.spdx.json"),
    ];
    for path in &beside {
        if let Some(doc) = read_json(path) {
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
         A rebuild needs the source repository and commit. Either download the artifact's \
         {} sidecar into the same directory, or build the app with `sbom = \"embed cyclonedx\"` \
         in Day.toml so it ships inside.",
        artifact.display(),
        crate::pack::naming::sidecar(artifact, "sbom-cdx.json")
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
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

/// What the `.buildinfo.json` sidecar beside an artifact records, parsed into the fields a
/// rebuild uses. Everything is optional because the sidecar itself is.
struct Sidecar {
    target: Option<String>,
    profile: Option<String>,
    tools: Vec<(String, String, String)>,
    inputs: Vec<(String, String)>,
    payload: Vec<(String, String)>,
}

/// Read the buildinfo sidecar next to the artifact. It is always a sidecar, by design: embedding
/// tool versions would make the artifact differ per machine (§20.3). Without it a rebuild still
/// works, minus tool gating.
///
/// The name is `<artifact>.buildinfo.json` (§20.4), so it resolves exactly. The directory scan
/// behind it is the fallback for artifacts packed before that naming: it would pick the wrong
/// target's sidecar out of a release directory holding several, which is precisely why the
/// artifact's own name is now part of it.
fn read_sidecar(artifact: &Path) -> Sidecar {
    let dir = artifact.parent().unwrap_or(Path::new("."));
    let info = read_json(&crate::pack::naming::sidecar(artifact, "buildinfo.json")).or_else(|| {
        std::fs::read_dir(dir)
            .ok()
            .and_then(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .find(|p| p.to_string_lossy().ends_with(".buildinfo.json"))
            })
            .and_then(|p| read_json(&p))
    });

    let (inputs, payload) = match &info {
        Some(v) => (
            kv_list(v, "inputs", "name", "value"),
            kv_list(v, "payload", "path", "sha256"),
        ),
        None => (Vec::new(), Vec::new()),
    };

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

    Sidecar {
        target,
        profile,
        tools,
        inputs,
        payload,
    }
}

/// Gather everything known about how the artifact was produced.
fn collect(artifact: &Path, scratch: &Path) -> Result<Provenance, String> {
    let sbom = locate_sbom(artifact, scratch)?;
    let facts = source_facts(&sbom).ok_or_else(|| {
        "the SBOM has no day:repository / day:commit properties — it was written by a different \
         tool, or by a day-cli too old to record them"
            .to_string()
    })?;
    let side = read_sidecar(artifact);
    Ok(Provenance {
        repository: facts.repository,
        commit: facts.commit,
        dirty: facts.dirty,
        project: facts.project,
        app_id: facts.app_id,
        target: side.target,
        profile: side.profile,
        tools: side.tools,
        inputs: side.inputs,
        payload: side.payload,
    })
}

/// Provenance for a `--from-dir` rebuild. The caller names the source directly, so there is no
/// repository or commit to record and no SBOM is required; the buildinfo sidecar still gates the
/// tool versions when it sits beside the artifact.
fn collect_from_dir(artifact: &Path) -> Provenance {
    let side = read_sidecar(artifact);
    Provenance {
        repository: String::new(),
        commit: String::new(),
        dirty: false,
        project: None,
        app_id: None,
        target: side.target,
        profile: side.profile,
        tools: side.tools,
        inputs: side.inputs,
        payload: side.payload,
    }
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

/// Entry names never copied by [`copy_project`], at any depth. `.git` is history the rebuild has
/// no use for; `build`, `target`, and `node_modules` are build products, and a stale artifact or
/// object file carried into the copy would poison the rebuild it is meant to check.
const COPY_EXCLUDES: [&str; 4] = [".git", "build", "target", "node_modules"];

/// Copy a project directory into the scratch tree for a `--from-dir` rebuild.
///
/// Copying rather than building in place is deliberate: the copy sits at a different absolute
/// path, so a source path baked into the binary surfaces as a payload mismatch instead of
/// reproducing by accident (§20.3, the same reason `checkout` clones to scratch). Symlinks are
/// skipped, as `file_list` does.
fn copy_project(src: &Path, dest: &Path) -> Result<(), String> {
    let mut stack = vec![(src.to_path_buf(), dest.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        let entries = std::fs::read_dir(&from).map_err(|e| format!("{}: {e}", from.display()))?;
        for e in entries.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if COPY_EXCLUDES.contains(&name) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            let target = to.join(name);
            if meta.is_dir() {
                stack.push((p, target));
            } else {
                std::fs::copy(&p, target).map_err(|e| format!("copying {}: {e}", p.display()))?;
            }
        }
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
fn compare(
    original: &Path,
    rebuilt: &Path,
    scratch: &Path,
    recorded_payload: &[(String, String)],
    project_dir: &Path,
    target: &'static crate::targets::Target,
) -> (Verdict, Verdict) {
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
        // The container could not be opened here — a `.flatpak` is an OSTree bundle whose import
        // wants privileges the runner does not have, and a `.msix` needs a working unzip. The
        // payload tier is still decidable: the original recorded the digest of every staged
        // payload file, so hash what THIS build staged and compare that.
        return (
            payload_by_digest(recorded_payload, project_dir, target, original),
            container,
        );
    }
    match differing_members(&ua, &ub) {
        Ok((diffs, skipped)) if diffs.is_empty() => {
            if !skipped.is_empty() {
                status(
                    "Excluded",
                    &format!(
                        "{} file(s) no build controls: {}",
                        skipped.len(),
                        skipped.join(", ")
                    ),
                );
            }
            (Verdict::Identical, container)
        }
        Ok((diffs, _)) => {
            // Name the first actual difference. Without it a verdict of "these XML files differ"
            // sends the next person guessing at a package they cannot open on their own host.
            // Text members quote the line; the compiled binary — the member that actually fails
            // this check — names the Mach-O region instead, since a hex dump would say nothing.
            let hint = diffs
                .first()
                .and_then(|rel| {
                    let (pa, pb) = (ua.join(rel), ub.join(rel));
                    first_text_difference(&pa, &pb).or_else(|| first_binary_difference(&pa, &pb))
                })
                .map(|d| format!("\n        {}: {d}", diffs[0]))
                .unwrap_or_default();
            (
                Verdict::Differs(format!(
                    "{} file(s) differ after normalization: {}{hint}",
                    diffs.len(),
                    diffs
                        .iter()
                        .take(4)
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                container,
            )
        }
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
    /// Rebuild from this project directory instead of cloning the commit the SBOM records — for
    /// artifacts whose source is not in git, e.g. a freshly scaffolded project in CI. The SBOM
    /// and the dirty check are skipped (the caller vouches for the tree); tool gating still
    /// applies when a `.buildinfo.json` sits beside the artifact.
    pub from_dir: Option<std::path::PathBuf>,
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

    // `--from-dir` skips the SBOM and the dirty refusal: the caller names the source tree, so
    // there is no recorded commit to hold it against.
    let prov = match &opts.from_dir {
        Some(_) => collect_from_dir(&artifact),
        None => collect(&artifact, &scratch)?,
    };
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
    match &opts.from_dir {
        Some(dir) => {
            let dir = dir
                .canonicalize()
                .map_err(|e| format!("{}: {e}", dir.display()))?;
            if !dir.is_dir() {
                return Err(format!("{} is not a directory", dir.display()));
            }
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project");
            status("Copying", &format!("{} → {}", dir.display(), src.display()));
            copy_project(&dir, &src.join(name))?;
        }
        None => checkout(&prov.repository, &prov.commit, &src)?,
    }

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
    let mut pack = Command::new(&day);
    pack.current_dir(&project_dir)
        .args(["pack", "-p", target.name, "--profile", &profile])
        .arg("--no-version-in-name");
    // Re-apply the build inputs the artifact recorded. Their defaults are machine-dependent — with
    // no device attached, the ABI set collapses to one — so without this the rebuild packs a
    // structurally different artifact and the comparison reports a difference that is ours.
    for (k, v) in &prov.inputs {
        status("Input", &format!("{k}={v}"));
        pack.env(k, v);
    }
    let ok = pack
        .status()
        .map_err(|e| format!("running day pack: {e}"))?;
    if !ok.success() {
        return Err("the rebuild failed — see the pack output above".into());
    }

    let rebuilt_dir = project_dir.join("build/day/dist");
    let name = artifact.file_name().unwrap_or_default();
    let mut rebuilt = rebuilt_dir.join(name);
    // `day pack` names the artifact exactly as it ships, so the rebuild normally lands on the same
    // file name. The one exception is iOS: pack marks a build with no signing material
    // `…-unsigned.ipa`, and release CI strips that so the published asset keeps one name either
    // way. A verifying machine has no signing config, so it packs the marked name — accept it,
    // rather than report a rebuild that in fact ran.
    if !rebuilt.is_file()
        && let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".ipa"))
    {
        rebuilt = rebuilt_dir.join(format!("{stem}-unsigned.ipa"));
    }
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

    let (payload, container) = compare(
        &artifact,
        &rebuilt,
        &scratch,
        &prov.payload,
        &project_dir,
        target,
    );
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
/// `recorded` is the path the SBOM carries (`apps/example`, or empty for the repository root) and
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
///
/// This is where Day's reproducibility guarantee is defined, so state it plainly: a rebuild is NOT
/// promised to be byte-identical to the original artifact. It is promised to be identical after
/// normalization — once the parts that describe the machine and the moment, rather than the
/// compiled program, are removed. Toolchains that embed a signature, a build id, or a path to
/// their own scratch directory would otherwise make the check impossible to pass without pinning
/// the build directory, which is a worse guarantee than the one being made here.
///
/// What comes off, and why each is irrelevant to "is this the same code":
///
/// * the code signature — computed OVER the bytes below, so it cannot survive their normalization,
///   and identity/timestamp are the packager's, not the program's;
/// * the Mach-O `LC_UUID` — a per-link build id, deliberately unique per link;
/// * the debug map (`N_OSO` stabs) — absolute paths to the object files the linker consumed.
///
/// What deliberately does NOT come off is everything that decides what the program does: the text
/// and data, the symbol table proper, the load commands, the linked libraries. A change in any of
/// those still fails the check, which is the point.
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
    // Then the debug map. ld records an ABSOLUTE path to every object file it consumed in the
    // `N_OSO` stabs — into SYMROOT, into cargo's output, into the SwiftPM package's build dir —
    // so two builds of one commit from two directories carry different strings AND different
    // sizes, since the paths differ in length. That is a map of the machine that built the code,
    // not the code, so it comes off before the comparison: `-S` drops the debug symbols and
    // leaves everything that determines behaviour. `day build` also passes `-oso_prefix` to shrink
    // these to project-relative paths at link time (mobile.rs), but it cannot reach the objects a
    // SwiftPM package prelinks with `ld -r`, and this check must not depend on that.
    //
    // Its result is CHECKED, not discarded: `strip` refuses a binary whose signature the edit
    // would invalidate, and a silent refusal leaves the debug map in place, so the comparison
    // fails with "differs after normalization" and no hint that normalization is what broke.
    let stripped = Command::new("strip").arg("-S").arg(path).output();
    match stripped {
        Ok(o) if !o.status.success() => {
            return Err(format!(
                "`strip -S {}` failed ({}), so the debug map could not be removed and the \
                 comparison would report a difference that is not in the code: {}",
                path.display(),
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Err(e) => return Err(format!("running `strip` on {}: {e}", path.display())),
        Ok(_) => {}
    }
    let mut buf = std::fs::read(path).map_err(|e| e.to_string())?;
    if zero_macho_uuid(&mut buf) > 0 {
        std::fs::write(path, &buf).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Unpack a container so its members can be compared. `false` when the format cannot be opened here.
/// Compare the payload this rebuild staged against the digests the artifact recorded.
///
/// Not a weaker check than extraction, but a different one: it compares the compiled code as the
/// original build wrote it, before packaging, rather than as the container preserved it. It is what
/// Debian's `.buildinfo` has always done, and it is the only tier that can reach a verdict for a
/// container this host cannot open.
fn payload_by_digest(
    recorded: &[(String, String)],
    project_dir: &Path,
    target: &'static crate::targets::Target,
    original: &Path,
) -> Verdict {
    let fmt = original
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("this format");
    if recorded.is_empty() {
        return Verdict::Unchecked(format!(
            "no extractor for {fmt} on this host, and the artifact recorded no payload digests \
             (it was packed by a day-cli too old to write them)"
        ));
    }
    let Some(root) = crate::pack::payload_root(project_dir, target) else {
        return Verdict::Unchecked(format!(
            "no extractor for {fmt} on this host, and {} stages no payload to compare",
            target.name
        ));
    };
    let rebuilt = crate::pack::payload_digests(&root);
    if rebuilt.is_empty() {
        return Verdict::Unchecked(format!(
            "no extractor for {fmt} on this host, and the rebuild staged no payload under {}",
            root.display()
        ));
    }
    let want: std::collections::BTreeMap<&str, &str> = recorded
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_str()))
        .collect();
    let got: std::collections::BTreeMap<&str, &str> = rebuilt
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_str()))
        .collect();
    if want.keys().ne(got.keys()) {
        return Verdict::Differs(format!(
            "the two builds staged different payload files ({} vs {})",
            want.len(),
            got.len()
        ));
    }
    let diffs: Vec<&str> = want
        .iter()
        .filter(|(p, d)| got.get(*p) != Some(d))
        .map(|(p, _)| *p)
        .collect();
    if diffs.is_empty() {
        Verdict::Identical
    } else {
        Verdict::Differs(format!(
            "{} payload file(s) differ by digest: {}",
            diffs.len(),
            diffs.join(", ")
        ))
    }
}

/// A path `unzip` can actually open on Windows.
///
/// `canonicalize` returns an extended-length `\\?\D:\a\…` path there, and the MSYS `unzip` on the
/// runners treats every backslash as an escape: `D:\a\day\day\shipped\showcase.msix` reached it
/// as `\?D:adaydayshippedshowcase.msix` and it reported the file missing — which the payload tier
/// then reported as "no extractor for msix on this host". Forward slashes survive both layers, and
/// Windows accepts them everywhere. No-op off Windows.
fn portable(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) {
        s.trim_start_matches("\\\\?\\").replace('\\', "/")
    } else {
        s.into_owned()
    }
}

fn unpack(container: &Path, dest: &Path) -> bool {
    let _ = std::fs::create_dir_all(dest);
    match container.extension().and_then(|e| e.to_str()) {
        Some("ipa" | "apk" | "aab" | "hap" | "msix" | "zip") => Command::new("unzip")
            .args(["-q", "-o"])
            .arg(portable(container))
            .arg("-d")
            .arg(portable(dest))
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
/// A member that holds a SIGNATURE rather than build output.
///
/// These can never match and their difference says nothing about the build: CI signs Windows
/// packages with a self-signed certificate generated per run, so `AppxSignature.p7x` and the
/// `CodeIntegrity.cat` catalog derived from it differ on every pack even when every compiled byte
/// is identical. Counting them as payload differences would make the Windows leg permanently red
/// for the one reason §20.3 already classes as advisory. They are reported as excluded, never
/// silently dropped.
fn signature_member(rel: &Path) -> bool {
    // By component, not by prefix: inside an `.ipa` or a `.dmg` this material sits under
    // `Payload/<App>.app/Contents/`, so anchoring at the root would match none of it.
    let joined = rel.to_string_lossy().replace('\\', "/");
    let parts: Vec<&str> = joined.split('/').collect();
    let Some(name) = parts.last().copied() else {
        return false;
    };
    let in_dir = |d: &str| parts.iter().rev().skip(1).any(|c| *c == d);

    name == "AppxSignature.p7x"
        || (name == "CodeIntegrity.cat" && in_dir("AppxMetadata"))
        || in_dir("_CodeSignature")
        || name == "embedded.mobileprovision"
        || (in_dir("META-INF")
            && (name.ends_with(".SF")
                || name.ends_with(".RSA")
                || name.ends_with(".DSA")
                || name.ends_with(".EC")
                || name == "MANIFEST.MF"))
}

/// A member that indexes the CONTAINER rather than carrying build output.
///
/// `AppxBlockMap.xml` records each member's size, local-file-header size and per-block hashes;
/// `[Content_Types].xml` maps the extensions present to content types. Both are written by
/// `makeappx` from its own walk of the staging directory, so they describe the .msix ZIP, not the
/// app — the same tier §20.3 already classes as advisory when the container digest differs.
///
/// Excluding them hides nothing. Every file they describe is compared directly, and a member
/// appearing or disappearing is caught earlier as "different sets of files", so a payload change
/// cannot reach a verdict through here.
fn container_index_member(rel: &Path) -> bool {
    let joined = rel.to_string_lossy().replace('\\', "/");
    let name = joined.rsplit('/').next().unwrap_or_default();
    name == "AppxBlockMap.xml" || name == "[Content_Types].xml"
}

/// Why a member takes no part in the payload comparison, or `None` when it is build output.
fn excluded_member(rel: &Path) -> Option<&'static str> {
    if signature_member(rel) {
        Some("signature")
    } else if container_index_member(rel) {
        Some("container index")
    } else {
        None
    }
}

/// The first line that differs between two text files, for a report that would otherwise say only
/// that some XML differs. Binary or oversized members yield nothing rather than a wall of bytes.
fn first_text_difference(a: &Path, b: &Path) -> Option<String> {
    const MAX: usize = 256 * 1024;
    const WIDTH: usize = 160;
    let (ba, bb) = (std::fs::read(a).ok()?, std::fs::read(b).ok()?);
    if ba.len() > MAX || bb.len() > MAX {
        return None;
    }
    let (ta, tb) = (String::from_utf8(ba).ok()?, String::from_utf8(bb).ok()?);
    let clip = |l: &str| {
        let l = l.trim();
        if l.chars().count() > WIDTH {
            format!("{}…", l.chars().take(WIDTH).collect::<String>())
        } else {
            l.to_string()
        }
    };
    for (n, (la, lb)) in ta.lines().zip(tb.lines()).enumerate() {
        if la != lb {
            return Some(format!("line {}: {} | {}", n + 1, clip(la), clip(lb)));
        }
    }
    let (ca, cb) = (ta.lines().count(), tb.lines().count());
    (ca != cb).then(|| format!("{ca} line(s) vs {cb}"))
}

/// Where two binary members first differ, named in the Mach-O's own terms.
///
/// The sibling of `first_text_difference` for the member that actually fails this check on Apple
/// platforms: the compiled executable. "the executable differs" gives whoever reads a CI log
/// nothing to act on, while the region the bytes land in is a lead — `__TEXT,__text` says the
/// emitted code changed, `__LINKEDIT` says a table the linker built did, and unequal lengths say
/// the two links did not produce the same shape at all.
fn first_binary_difference(a: &Path, b: &Path) -> Option<String> {
    let (ba, bb) = (std::fs::read(a).ok()?, std::fs::read(b).ok()?);
    let sizes = (ba.len() != bb.len()).then(|| format!("{} vs {} bytes", ba.len(), bb.len()));
    let Some(at) = ba.iter().zip(&bb).position(|(x, y)| x != y) else {
        // Equal as far as the shorter one goes: there is no differing byte to point at.
        return sizes.map(|s| format!("one is a prefix of the other ({s})"));
    };
    let n = ba.iter().zip(&bb).filter(|(x, y)| x != y).count();
    let head = match sizes {
        Some(s) => format!("{s}, first difference at"),
        None => format!("{n} byte(s) differ, first at"),
    };
    Some(match macho_location(&ba, at) {
        Some(place) => format!("{head} 0x{at:x} ({place})"),
        None => format!("{head} 0x{at:x}"),
    })
}

/// The load command a Mach-O command word names, for the few that carry a file range.
fn lc_name(cmd: u32) -> &'static str {
    match cmd {
        0x01 => "LC_SEGMENT",
        0x02 => "LC_SYMTAB",
        0x0B => "LC_DYSYMTAB",
        0x0C => "LC_LOAD_DYLIB",
        0x0E => "LC_LOAD_DYLINKER",
        0x19 => "LC_SEGMENT_64",
        0x1B => "LC_UUID",
        0x1D => "LC_CODE_SIGNATURE",
        0x1C => "LC_RPATH",
        0x26 => "LC_FUNCTION_STARTS",
        0x29 => "LC_DATA_IN_CODE",
        0x2A => "LC_SOURCE_VERSION",
        0x32 => "LC_BUILD_VERSION",
        0x8000_0022 => "LC_DYLD_INFO_ONLY",
        0x8000_0028 => "LC_MAIN",
        0x8000_0033 => "LC_DYLD_EXPORTS_TRIE",
        0x8000_0034 => "LC_DYLD_CHAINED_FIXUPS",
        _ => "LC_?",
    }
}

/// Name the part of a Mach-O a file offset falls in — `__TEXT,__text`, a load command, or one of
/// the `__LINKEDIT` tables the header points at.
///
/// Best-effort and shape-tolerant by design: this runs on a file the check has already decided is
/// wrong, so a malformed or unexpected one must yield `None` rather than a guess or a panic. The
/// narrowest range wins, so a section is named ahead of the segment containing it.
fn macho_location(buf: &[u8], at: usize) -> Option<String> {
    fn u32_at(b: &[u8], at: usize, big: bool) -> Option<u32> {
        let v = b.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(v)
        } else {
            u32::from_le_bytes(v)
        })
    }
    fn u64_at(b: &[u8], at: usize, big: bool) -> Option<u64> {
        let v = b.get(at..at + 8)?.try_into().ok()?;
        Some(if big {
            u64::from_be_bytes(v)
        } else {
            u64::from_le_bytes(v)
        })
    }
    // A 16-byte fixed field, NUL-padded.
    fn name16(b: &[u8], at: usize) -> String {
        let raw = b.get(at..at + 16).unwrap_or_default();
        String::from_utf8_lossy(raw)
            .trim_end_matches('\0')
            .to_string()
    }
    fn thin(buf: &[u8], base: usize, at: usize) -> Option<String> {
        let (big, is64) = match u32_at(buf, base, false)? {
            0xFEED_FACF => (false, true),
            0xFEED_FACE => (false, false),
            0xCFFA_EDFE => (true, true),
            0xCEFA_EDFE => (true, false),
            _ => return None,
        };
        let ncmds = u32_at(buf, base + 16, big)?;
        let header = base + if is64 { 32 } else { 28 };
        let mut ranges: Vec<(usize, usize, String)> = vec![(base, header, "the header".into())];
        let mut lc = header;
        for i in 0..ncmds {
            let (cmd, size) = (u32_at(buf, lc, big)?, u32_at(buf, lc + 4, big)? as usize);
            if size < 8 || lc + size > buf.len() {
                break;
            }
            ranges.push((lc, lc + size, format!("load command {i}, {}", lc_name(cmd))));
            match cmd {
                // Segments: name the section rather than the segment wherever one covers the
                // offset. Zero-filled sections have no file range and are skipped.
                0x19 | 0x01 => {
                    let seg64 = cmd == 0x19;
                    let (first, stride) = if seg64 { (72, 80) } else { (56, 68) };
                    let nsects = u32_at(buf, lc + if seg64 { 64 } else { 48 }, big)?;
                    for s in 0..nsects as usize {
                        let so = lc + first + s * stride;
                        let (sect, seg) = (name16(buf, so), name16(buf, so + 16));
                        let (off, len) = if seg64 {
                            (
                                u32_at(buf, so + 48, big)? as usize,
                                u64_at(buf, so + 40, big)? as usize,
                            )
                        } else {
                            (
                                u32_at(buf, so + 40, big)? as usize,
                                u32_at(buf, so + 36, big)? as usize,
                            )
                        };
                        if off > 0 && len > 0 {
                            ranges.push((base + off, base + off + len, format!("{seg},{sect}")));
                        }
                    }
                }
                // The two tables LC_SYMTAB points at are separate ranges with separate meanings.
                0x02 => {
                    let symoff = u32_at(buf, lc + 8, big)? as usize;
                    let nsyms = u32_at(buf, lc + 12, big)? as usize;
                    let stroff = u32_at(buf, lc + 16, big)? as usize;
                    let strsize = u32_at(buf, lc + 20, big)? as usize;
                    let width = if is64 { 16 } else { 12 };
                    ranges.push((
                        base + symoff,
                        base + symoff + nsyms * width,
                        "__LINKEDIT symbol table".into(),
                    ));
                    ranges.push((
                        base + stroff,
                        base + stroff + strsize,
                        "__LINKEDIT string table".into(),
                    ));
                }
                // linkedit_data_command: cmd, cmdsize, dataoff, datasize.
                0x1D | 0x26 | 0x29 | 0x8000_0033 | 0x8000_0034 => {
                    let off = u32_at(buf, lc + 8, big)? as usize;
                    let len = u32_at(buf, lc + 12, big)? as usize;
                    if len > 0 {
                        ranges.push((
                            base + off,
                            base + off + len,
                            format!("__LINKEDIT {}", lc_name(cmd)),
                        ));
                    }
                }
                _ => {}
            }
            lc += size;
        }
        // Narrowest wins: a section beats the segment, and either beats the load-command table.
        ranges
            .into_iter()
            .filter(|(lo, hi, _)| at >= *lo && at < *hi)
            .min_by_key(|(lo, hi, _)| hi - lo)
            .map(|(_, _, what)| what)
    }
    // Fat: place the offset in its slice first, then inside that slice's Mach-O.
    if let Some(m @ (0xCAFE_BABE | 0xBEBA_FECA)) = u32_at(buf, 0, true) {
        let big = m == 0xCAFE_BABE;
        let n = u32_at(buf, 4, big)?;
        for i in 0..n as usize {
            let arch = 8 + i * 20;
            let off = u32_at(buf, arch + 8, big)? as usize;
            let size = u32_at(buf, arch + 12, big)? as usize;
            if at >= off && at < off + size {
                let inner = thin(buf, off, at).unwrap_or_else(|| "unplaced".into());
                return Some(format!("slice {i}, {inner}"));
            }
        }
        return Some("the fat header".into());
    }
    thin(buf, 0, at)
}

/// Differing members, and the members excluded from the comparison.
fn differing_members(a: &Path, b: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let keep = |l: Vec<PathBuf>| -> (Vec<PathBuf>, Vec<String>) {
        let (skip, rest): (Vec<PathBuf>, Vec<PathBuf>) =
            l.into_iter().partition(|p| excluded_member(p).is_some());
        let named = skip
            .iter()
            .map(|p| {
                let why = excluded_member(p).unwrap_or("excluded");
                format!("{} ({why})", p.display())
            })
            .collect();
        (rest, named)
    };
    let (la, siga) = keep(file_list(a));
    let (lb, _) = keep(file_list(b));
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
    Ok((diffs, siga))
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
            project_path: Some("apps/example".into()),
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
        assert_eq!(from_cdx.project.as_deref(), Some("apps/example"));
    }

    /// The bug this replaced: `find_project_dir` returned the first `Day.toml` a directory walk
    /// tripped over, so rebuilding day's own showcase packed a DIFFERENT app on one runner and
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
        mk("apps/other-app", Some("dev.example.other"));
        mk("apps/example", Some("dev.daybrite.showcase"));
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
            find_project_dir(&tmp, Some("apps/example"), None).expect("recorded"),
            tmp.join("apps/example"),
        );
        // An empty recorded path is a real answer — the project IS the repository root, which is
        // how a scaffolded single-app repo looks. It must not be read as "nothing recorded".
        mk("", Some("dev.example.root"));
        assert_eq!(find_project_dir(&tmp, Some(""), None).expect("root"), tmp);
        // Without it, the app id identifies the project among the several in the repository.
        assert_eq!(
            find_project_dir(&tmp, None, Some("dev.daybrite.showcase")).expect("by id"),
            tmp.join("apps/example"),
        );
        // Neither: refuse and name the candidates rather than pack an arbitrary one.
        let err = find_project_dir(&tmp, None, None).expect_err("ambiguous");
        assert!(
            err.contains("apps/other-app") && err.contains("apps/example"),
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

    /// The Windows extractor bug: a canonicalized path is verbatim (`\\?\D:\…`), and the MSYS
    /// `unzip` on the runners ate every backslash, so the `.msix` payload was never compared.
    #[test]
    fn windows_paths_reach_unzip_with_forward_slashes() {
        let p = Path::new("relative/dir/showcase.msix");
        assert_eq!(portable(p), "relative/dir/showcase.msix");
        // The transformation itself, spelled out for the platform that needs it.
        let win = r"\\?\D:\a\day\day\shipped\showcase.msix";
        let fixed = win.trim_start_matches(r"\\?\").replace('\\', "/");
        assert_eq!(fixed, "D:/a/day/day/shipped/showcase.msix");
    }

    /// The payload tier must reach a verdict for containers this host cannot open — and must say
    /// so plainly when the artifact predates the recorded digests.
    #[test]
    fn payload_by_digest_decides_when_there_is_no_extractor() {
        let tmp = std::env::temp_dir().join(format!("day-payload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("build/day/flatpak/linux-gtk/stage/bin");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("showcase-bin"), b"compiled code").expect("write");
        let target = crate::targets::find("linux-gtk").expect("target");
        let staged = crate::pack::payload_digests(&root);
        assert_eq!(staged.len(), 1, "one staged payload file");

        let art = Path::new("showcase-gtk-x86_64.flatpak");
        // Same digests → identical.
        assert_eq!(
            payload_by_digest(&staged, &tmp, target, art),
            Verdict::Identical
        );
        // A different digest for the same file → a named difference, not a shrug.
        let tampered = vec![(staged[0].0.clone(), "0".repeat(64))];
        match payload_by_digest(&tampered, &tmp, target, art) {
            Verdict::Differs(d) => assert!(d.contains("showcase-bin"), "{d}"),
            v => panic!("expected Differs, got {v:?}"),
        }
        // No recorded digests (an older artifact) → unchecked, and it says why.
        match payload_by_digest(&[], &tmp, target, art) {
            Verdict::Unchecked(d) => assert!(d.contains("too old"), "{d}"),
            v => panic!("expected Unchecked, got {v:?}"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// CI signs Windows packages with a per-run self-signed certificate, so the signature block
    /// differs on every pack. Reporting that as "the compiled code differs" made windows-xaml red
    /// for the one cause §20.3 classes as advisory.
    #[test]
    fn signature_members_are_excluded_from_the_payload_tier() {
        for p in [
            "AppxSignature.p7x",
            "AppxMetadata/CodeIntegrity.cat",
            "_CodeSignature/CodeResources",
            "embedded.mobileprovision",
            "META-INF/CERT.RSA",
            "META-INF/MANIFEST.MF",
            // Nested: where this material actually lives in an .ipa and a .dmg.
            "Payload/Showcase.app/_CodeSignature/CodeResources",
            "Payload/Showcase.app/embedded.mobileprovision",
            "content/Day Showcase.app/Contents/_CodeSignature/CodeResources",
        ] {
            assert!(signature_member(Path::new(p)), "{p} is signature material");
        }
        // Windows hands these over with backslashes.
        assert!(signature_member(Path::new(
            r"AppxMetadata\CodeIntegrity.cat"
        )));
        // Build output must never be excluded — that would hide a real difference.
        for p in [
            "AppxManifest.xml",
            "AppxBlockMap.xml",
            "showcase.exe",
            "lib/arm64-v8a/libshowcase.so",
            "META-INF/services/foo",
        ] {
            assert!(!signature_member(Path::new(p)), "{p} is build output");
        }
    }

    /// `makeappx` writes AppxBlockMap.xml and [Content_Types].xml from its own walk of the staging
    /// directory, so they index the ZIP rather than carry build output — and they differed between
    /// two packs of a byte-identical payload, which made windows-xaml red. They are excluded as
    /// container material, NOT as signature material: the distinction is what the report prints.
    #[test]
    fn the_container_index_is_excluded_but_still_named_by_category() {
        for p in ["AppxBlockMap.xml", "[Content_Types].xml"] {
            assert_eq!(
                excluded_member(Path::new(p)),
                Some("container index"),
                "{p}"
            );
            assert!(!signature_member(Path::new(p)), "{p} is not a signature");
        }
        assert_eq!(
            excluded_member(Path::new("AppxSignature.p7x")),
            Some("signature")
        );
        // Build output stays in the comparison, or an exclusion could hide a real difference.
        for p in ["AppxManifest.xml", "ci-sample.exe", "resources.pri"] {
            assert_eq!(excluded_member(Path::new(p)), None, "{p} is build output");
        }
    }

    /// Excluding the index must not let a payload change through: the files it describes are
    /// compared directly, so a changed .exe still fails even when the index matches.
    #[test]
    fn an_excluded_index_cannot_hide_a_changed_binary() {
        let tmp = std::env::temp_dir().join(format!("day-idx-{}", std::process::id()));
        let (a, b) = (tmp.join("a"), tmp.join("b"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).expect("mkdir");
            std::fs::write(d.join("AppxBlockMap.xml"), "<BlockMap/>").expect("map");
        }
        std::fs::write(a.join("app.exe"), b"MZ-one").expect("exe");
        std::fs::write(b.join("app.exe"), b"MZ-two").expect("exe");
        // Same name, different bytes: the index is excluded, the binary is not.
        std::fs::write(a.join("[Content_Types].xml"), "<Types a=\"1\"/>").expect("ct");
        std::fs::write(b.join("[Content_Types].xml"), "<Types a=\"2\"/>").expect("ct");
        let (diffs, skipped) = differing_members(&a, &b).expect("compared");
        assert_eq!(
            diffs,
            vec!["app.exe".to_string()],
            "the binary must still fail"
        );
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        assert!(
            skipped.iter().all(|s| s.contains("container index")),
            "the report names WHY: {skipped:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A verdict that says only "some XML differs" is unactionable on a package the reader cannot
    /// open, so the first differing line rides along.
    #[test]
    fn a_text_difference_is_quoted_in_the_report() {
        let tmp = std::env::temp_dir().join(format!("day-hint-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let (a, b) = (tmp.join("a.xml"), tmp.join("b.xml"));
        std::fs::write(&a, "<Root>\n  <File Name=\"one\"/>\n</Root>\n").expect("a");
        std::fs::write(&b, "<Root>\n  <File Name=\"two\"/>\n</Root>\n").expect("b");
        let d = first_text_difference(&a, &b).expect("a difference");
        assert!(d.starts_with("line 2:"), "{d}");
        assert!(d.contains("one") && d.contains("two"), "{d}");
        // Binary members yield nothing rather than a wall of bytes.
        std::fs::write(&a, [0xFF, 0xFE, 0x00]).expect("a");
        std::fs::write(&b, [0xFF, 0xFE, 0x01]).expect("b");
        assert!(first_text_difference(&a, &b).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The exclusion must survive a real comparison: identical payload + differing signature is a
    /// PASS, and a differing binary still fails.
    #[test]
    fn a_differing_signature_alone_is_not_a_payload_difference() {
        let tmp = std::env::temp_dir().join(format!("day-sigcmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (a, b) = (tmp.join("a"), tmp.join("b"));
        for (dir, sig) in [(&a, b"sig-one"), (&b, b"sig-two")] {
            std::fs::create_dir_all(dir.join("AppxMetadata")).expect("mkdir");
            std::fs::write(dir.join("showcase.exe"), b"identical code").expect("exe");
            std::fs::write(dir.join("AppxSignature.p7x"), sig).expect("sig");
            std::fs::write(dir.join("AppxMetadata/CodeIntegrity.cat"), sig).expect("cat");
        }
        let (diffs, skipped) = differing_members(&a, &b).expect("compared");
        assert!(
            diffs.is_empty(),
            "signature-only difference is not a payload difference: {diffs:?}"
        );
        assert_eq!(skipped.len(), 2, "and both exclusions are reported");

        // Now make the actual binary differ: that must still be caught.
        std::fs::write(b.join("showcase.exe"), b"different code").expect("exe");
        let (diffs, _) = differing_members(&a, &b).expect("compared");
        assert_eq!(diffs, vec!["showcase.exe".to_string()]);
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
            project_path: Some("apps/example".into()),
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

    /// A minimal 64-bit little-endian Mach-O: one `__TEXT` segment holding one `__text` section
    /// whose 16 bytes live at file offset 200.
    fn synthetic_macho() -> Vec<u8> {
        let mut buf = vec![0u8; 216];
        buf[0..4].copy_from_slice(&0xFEED_FACFu32.to_le_bytes());
        buf[16..20].copy_from_slice(&1u32.to_le_bytes()); // ncmds
        let lc = 32;
        buf[lc..lc + 4].copy_from_slice(&0x19u32.to_le_bytes()); // LC_SEGMENT_64
        buf[lc + 4..lc + 8].copy_from_slice(&152u32.to_le_bytes()); // cmdsize: 72 + one section
        buf[lc + 8..lc + 14].copy_from_slice(b"__TEXT");
        buf[lc + 64..lc + 68].copy_from_slice(&1u32.to_le_bytes()); // nsects
        let sect = lc + 72;
        buf[sect..sect + 6].copy_from_slice(b"__text");
        buf[sect + 16..sect + 22].copy_from_slice(b"__TEXT");
        buf[sect + 40..sect + 48].copy_from_slice(&16u64.to_le_bytes()); // size
        buf[sect + 48..sect + 52].copy_from_slice(&200u32.to_le_bytes()); // file offset
        buf
    }

    /// "the executable differs" is not a lead. The region has to ride along, and the narrowest
    /// range has to win so a section is named ahead of the segment and load command over it.
    #[test]
    fn a_binary_difference_names_the_macho_region() {
        let buf = synthetic_macho();
        assert_eq!(macho_location(&buf, 4).as_deref(), Some("the header"));
        assert_eq!(
            macho_location(&buf, 40).as_deref(),
            Some("load command 0, LC_SEGMENT_64")
        );
        assert_eq!(macho_location(&buf, 204).as_deref(), Some("__TEXT,__text"));

        let tmp = std::env::temp_dir().join(format!("day-bindiff-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let (a, b) = (tmp.join("a"), tmp.join("b"));
        std::fs::write(&a, &buf).expect("a");
        let mut other = buf.clone();
        other[204] = 0xFF;
        std::fs::write(&b, &other).expect("b");
        let d = first_binary_difference(&a, &b).expect("a difference");
        assert!(d.contains("__TEXT,__text"), "{d}");
        assert!(d.contains("0xcc"), "the offset is reported in hex: {d}");
        assert!(d.starts_with("1 byte(s) differ"), "{d}");

        // A length change is its own answer: the two links did not produce the same shape.
        std::fs::write(&b, &buf[..100]).expect("b");
        let d = first_binary_difference(&a, &b).expect("a difference");
        assert!(d.contains("216 vs 100 bytes"), "{d}");
        assert!(
            d.contains("prefix"),
            "and says the shorter is a prefix: {d}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
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

    /// The `--from-dir` copy must carry the source and NOT the build products: a stale artifact
    /// under `build/` (or an object file under `target/`) carried into the scratch copy would be
    /// compared against itself, and the verdict would say nothing.
    #[test]
    fn the_from_dir_copy_excludes_build_products_and_keeps_the_source() {
        let tmp = std::env::temp_dir().join(format!("day-fromdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("ci-sample");
        for dir in [
            "src",
            "build/day/dist",
            ".git/objects",
            "target/release",
            "website/node_modules/dep",
        ] {
            std::fs::create_dir_all(src.join(dir)).expect("mkdir");
        }
        std::fs::write(src.join("Day.toml"), "schema = 1\n").expect("Day.toml");
        std::fs::write(src.join("src/main.rs"), "fn main() {}\n").expect("main.rs");
        std::fs::write(src.join("build/day/dist/stale.dmg"), b"stale").expect("stale");
        std::fs::write(src.join(".git/HEAD"), "ref: refs/heads/main\n").expect("HEAD");
        std::fs::write(src.join("target/release/app"), b"old binary").expect("old bin");
        std::fs::write(src.join("website/site.toml"), "locales = []\n").expect("site.toml");
        std::fs::write(src.join("website/node_modules/dep/index.js"), "x").expect("dep");

        let dest = tmp.join("copy");
        copy_project(&src, &dest).expect("copy");

        assert!(dest.join("Day.toml").is_file(), "the manifest survives");
        assert!(dest.join("src/main.rs").is_file(), "the source survives");
        assert!(
            !dest.join("build").exists(),
            "stale build products would poison the rebuild"
        );
        assert!(!dest.join(".git").exists(), ".git is excluded");
        assert!(!dest.join("target").exists(), "target/ is excluded");
        // At any depth, not just the root — node_modules sits under website/ in a scaffold.
        assert!(!dest.join("website/node_modules").exists());
        assert!(
            dest.join("website/site.toml").is_file(),
            "…while the directory around it survives"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
