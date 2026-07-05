//! day.yaml — the project manifest (DESIGN.md §17.3), v0 subset.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub day: u32,
    pub app: App,
    // Parsed for schema validation (deny_unknown_fields); the app scaffold consumes these,
    // the CLI does not yet (§17.3).
    #[serde(default)]
    #[allow(dead_code)]
    pub targets: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub window: Window,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_build")]
    pub build: u64,
}

fn default_version() -> String {
    "0.1.0".into()
}
fn default_build() -> u64 {
    1
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // see Manifest::window
pub struct Window {
    #[serde(default = "default_w")]
    pub width: f64,
    #[serde(default = "default_h")]
    pub height: f64,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            width: default_w(),
            height: default_h(),
        }
    }
}

fn default_w() -> f64 {
    480.0
}
fn default_h() -> f64 {
    640.0
}

pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

/// On Windows `std::fs::canonicalize` returns an extended-length `\\?\` (verbatim) path. That prefix
/// flows into `CARGO_TARGET_DIR` (ops.rs), and the windows-gnu toolchain's MinGW linker
/// (`ld`/`collect2`) can't parse `\\?\` object-file arguments — it drops the prefix and reports
/// `cannot find \\symbols.o`, failing the link (hit on windows-gtk / windows-qt; MSVC's link.exe
/// tolerates it, so winui was unaffected). De-verbatim the path so every subtool gets a plain
/// absolute path — still absolute, so the xcodebuild-SYMROOT need in `find_project` holds. No-op off
/// Windows, where canonicalize never adds a verbatim prefix.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(s) = p.to_str() {
        // `\\?\UNC\server\share` → `\\server\share`; `\\?\D:\path` → `D:\path`.
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p
}

/// Find the nearest ancestor directory containing day.yaml (from `start` or cwd).
pub fn find_project(start: Option<&Path>) -> Result<Project, String> {
    let mut dir = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    loop {
        let candidate = dir.join("day.yaml");
        if candidate.exists() {
            let text = std::fs::read_to_string(&candidate).map_err(|e| e.to_string())?;
            let manifest: Manifest =
                serde_norway::from_str(&text).map_err(|e| format!("day.yaml: {e}"))?;
            if manifest.day != 1 {
                return Err(format!(
                    "day.yaml: unsupported schema version {}",
                    manifest.day
                ));
            }
            // Always hand back an ABSOLUTE root. A relative `--project` (e.g. `apps/showcase`) would
            // otherwise flow into build-tool arguments like xcodebuild's `SYMROOT` as a relative path;
            // xcodebuild resolves relative build paths against each target's own working directory, so
            // the app target and a SwiftPM package dependency scatter their products into different
            // trees (a missing `*_*.bundle` copy failure). Absolute paths resolve identically everywhere.
            let root = std::fs::canonicalize(&dir).unwrap_or_else(|_| {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&dir))
                    .unwrap_or_else(|_| dir.clone())
            });
            return Ok(Project {
                root: strip_verbatim(root),
                manifest,
            });
        }
        if !dir.pop() {
            return Err("no day.yaml found in this directory or any ancestor".into());
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn strip_verbatim_deverbatims_windows_paths() {
        // Drive + UNC verbatim prefixes are removed so the MinGW linker can read the paths.
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\D:\a\day\day\apps\showcase")),
            PathBuf::from(r"D:\a\day\day\apps\showcase")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\proj")),
            PathBuf::from(r"\\server\share\proj")
        );
        // A plain absolute path is already fine — leave it untouched.
        assert_eq!(
            strip_verbatim(PathBuf::from(r"D:\a\proj")),
            PathBuf::from(r"D:\a\proj")
        );
        // canonicalize() really does hand back a verbatim path here; the result must not.
        let canon = std::fs::canonicalize(".").unwrap();
        assert!(!strip_verbatim(canon).to_string_lossy().starts_with(r"\\?\"));
    }
}
