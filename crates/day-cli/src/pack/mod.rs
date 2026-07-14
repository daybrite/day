//! `day pack` (DESIGN.md §16.5): build → sign → installable artifact, per target, with the
//! hoppack-lineage stage order (build → assemble → sign → package → notarize → verify). Every
//! artifact lands in `build/day/dist/` with a sha256 and a signing tier; the tier degrades
//! LOUDLY (never silently) when release signing material is absent (§20).
//!
//! Per-target default formats:
//!   macos-appkit → dmg · ios-uikit → ipa (sim-app without ASC creds) · android-widget → apk+aab
//!   linux-gtk/linux-qt → flatpak · windows-winui → msix+nsis · ohos-arkui → hap
//! GTK/Qt on macOS/Windows is DP-7 (deferred) and refuses with a pointer.

pub(crate) mod android;
mod flatpak;
mod ios;
mod macos;
mod msix;
mod nsis;
mod ohos;
pub mod settings;

use std::path::{Path, PathBuf};

use crate::meta::Project;
use crate::ops::status;
use crate::targets::Target;
pub use settings::PackOptions;

/// How an artifact ended up signed. `DevSigned` covers ad-hoc codesign, debug/CI-generated
/// keystores and self-signed certs — installable for development, not distributable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignTier {
    Unsigned,
    DevSigned,
    Release,
}

impl SignTier {
    pub fn as_str(self) -> &'static str {
        match self {
            SignTier::Unsigned => "unsigned",
            SignTier::DevSigned => "dev-signed",
            SignTier::Release => "release",
        }
    }
}

pub struct Artifact {
    pub path: PathBuf,
    /// Format tag: "dmg" | "ipa" | "sim-app" | "apk" | "aab" | "flatpak" | "msix" | "nsis" | "hap"
    pub kind: &'static str,
    pub sha256: String,
    pub tier: SignTier,
}

pub struct PackOutcome {
    pub target: &'static str,
    pub artifacts: Vec<Artifact>,
    pub seconds: f64,
}

/// Signing failures exit with code 6 (§16.3); everything else is a build failure (4).
pub enum PackError {
    Sign(String),
    Other(String),
}

impl From<String> for PackError {
    fn from(s: String) -> Self {
        PackError::Other(s)
    }
}

impl PackError {
    pub fn message(&self) -> &str {
        match self {
            PackError::Sign(m) | PackError::Other(m) => m,
        }
    }
    pub fn exit_code(&self) -> i32 {
        match self {
            PackError::Sign(_) => 6,
            PackError::Other(_) => 4,
        }
    }
}

fn default_formats(target: &Target) -> Result<Vec<&'static str>, String> {
    Ok(match target.name {
        "macos-appkit" => vec!["dmg"],
        "ios-uikit" => vec!["ipa"], // falls back to sim-app without ASC signing config
        "android-widget" => vec!["apk", "aab"],
        "linux-gtk" | "linux-qt" => vec!["flatpak"],
        "windows-winui" => vec!["msix", "nsis"],
        "ohos-arkui" => vec!["hap"],
        "macos-gtk" | "macos-qt" | "windows-gtk" | "windows-qt" => {
            return Err(format!(
                "pack for {} means bundling the toolkit into the package — deferred (DESIGN.md \
                 DP-7). Pack the platform-native target instead, or `day launch -p {}` for development.",
                target.name, target.name
            ));
        }
        other => return Err(format!("pack does not support {other}")),
    })
}

pub fn run(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
) -> Result<PackOutcome, PackError> {
    let start = std::time::Instant::now();
    let defaults = default_formats(target)?;
    let formats: Vec<String> = match &opts.formats {
        Some(list) => {
            for f in list {
                if !defaults.contains(&f.as_str()) {
                    return Err(PackError::Other(format!(
                        "format {f:?} is not available for {} (available: {})",
                        target.name,
                        defaults.join(", ")
                    )));
                }
            }
            list.clone()
        }
        None => defaults.iter().map(|s| s.to_string()).collect(),
    };

    let dist = project.root.join("build/day/dist");
    std::fs::create_dir_all(&dist).map_err(|e| PackError::Other(e.to_string()))?;

    let mut artifacts: Vec<Artifact> = Vec::new();
    match target.name {
        "macos-appkit" => {
            // dmg is the only macOS format today; the .app assembly is its input.
            artifacts.push(macos::pack(project, target, opts, &dist)?);
        }
        "ios-uikit" => {
            artifacts.push(ios::pack(project, target, opts, &dist)?);
        }
        "android-widget" => {
            artifacts.extend(android::pack(project, target, opts, &dist, &formats)?);
        }
        "linux-gtk" | "linux-qt" => {
            artifacts.push(flatpak::pack(project, target, opts, &dist)?);
        }
        "windows-winui" => {
            let staged = msix::stage_payload(project, target, opts)?;
            if formats.iter().any(|f| f == "msix") {
                artifacts.push(msix::pack(project, opts, &staged, &dist)?);
            }
            if formats.iter().any(|f| f == "nsis") {
                artifacts.push(nsis::pack(project, opts, &staged, &dist)?);
            }
        }
        "ohos-arkui" => {
            artifacts.push(ohos::pack(project, target, opts, &dist)?);
        }
        other => return Err(PackError::Other(format!("pack does not support {other}"))),
    }

    // Checksums + the loud per-artifact summary (§16.3 result contract).
    for a in &mut artifacts {
        a.sha256 = sha256_file(&a.path).map_err(PackError::Other)?;
        status(
            "Packed",
            &format!(
                "{} ({}, {}) sha256:{}…",
                a.path.display(),
                a.kind,
                a.tier.as_str(),
                &a.sha256[..12]
            ),
        );
        if a.tier != SignTier::Release {
            status(
                "Warning",
                &format!(
                    "{} is {} — NOT distributable (configure Day.toml `signing:` for release signing)",
                    a.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("artifact"),
                    a.tier.as_str()
                ),
            );
        }
    }

    Ok(PackOutcome {
        target: target.name,
        artifacts,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// Validate that the windows signing config resolves (shared with `day sign --check`).
pub(crate) fn msix_check(project: &Project) -> Result<(), String> {
    msix::resolve_signing(project)
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}

/// Doctor probe: locate a Windows-Kits tool (None off-Windows or when the SDK is absent).
pub(crate) fn windows_kit_tool_probe(tool: &str) -> Option<String> {
    msix::windows_kit_tool(tool).map(|p| p.display().to_string())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("checksum {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    // Stream the file through the hasher (sha2 0.11 dropped the `io::Write` hasher impl that let
    // `io::copy` write into it directly).
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("checksum {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    // sha2 0.11's digest output no longer implements `LowerHex` — hex-encode by hand.
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Run a command, returning a readable error with the failing tool's tail output.
pub(crate) fn run_tool(cmd: &mut std::process::Command, what: &str) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("{what}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let tail: Vec<&str> = text.lines().rev().take(25).collect();
    Err(format!(
        "{what} failed:\n{}",
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    ))
}

/// Copy a directory tree (used for staging payloads).
pub(crate) fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    for e in entries.flatten() {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| format!("copy {} → {}: {e}", from.display(), to.display()))?;
        }
    }
    Ok(())
}
