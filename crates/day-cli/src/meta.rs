//! Day.toml — the project manifest (DESIGN.md §17.3).
//!
//! Follows the Tauri / Dioxus model: a dedicated manifest file that doubles as the project
//! marker (`find_project` walks up to the nearest `Day.toml`). Two rules keep it honest:
//!
//! * **Derive, don't restate**: `name` and `version` come from the sibling `Cargo.toml`'s
//!   `[package]` — they are never written in Day.toml, so app identity can't drift from the
//!   crate's.
//! * **Base + overrides**: `[app]` holds the base properties; any of them can be overridden
//!   per platform (`[app.ios]`), per toolkit (`[app.qt]`), or per full target
//!   (`[app.macos-appkit]`) — most specific wins (see [`Manifest::resolve`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest schema version (currently 1).
    pub schema: u32,
    pub app: App,
    #[serde(default)]
    pub window: Window,
    /// Code-signing / notarization configuration (§16.5, §17.3). Values may reference environment
    /// variables as `${VAR}` — resolved at use time (see `pack::settings::interpolate`), never at
    /// parse time, so `day sign --check` can report missing variables without failing the parse.
    #[serde(default)]
    pub signing: Option<Signing>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signing {
    #[serde(default)]
    pub macos: Option<MacosSigning>,
    #[serde(default)]
    pub ios: Option<IosSigning>,
    #[serde(default)]
    pub android: Option<AndroidSigning>,
    #[serde(default)]
    pub windows: Option<WindowsSigning>,
    #[serde(default)]
    pub ohos: Option<OhosSigning>,
}

/// macOS Developer-ID signing + notarization (§16.5: codesign + notarytool + stapler).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct MacosSigning {
    /// Signing identity ("Developer ID Application: …"); "-" or absent = ad-hoc (dev tier).
    #[serde(default)]
    pub identity: Option<String>,
    /// Entitlements plist path, relative to the project root.
    #[serde(default)]
    pub entitlements: Option<String>,
    #[serde(default)]
    pub notarize: Option<Notarize>,
}

/// notarytool App Store Connect API-key auth (never interactive Apple-ID — §16.5).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Notarize {
    pub key_id: String,
    pub issuer: String,
    /// Path to the AuthKey_<id>.p8 file.
    pub key_path: String,
}

/// iOS App Store export signing: xcodebuild automatic signing with an ASC API key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct IosSigning {
    /// Apple Developer team id (DEVELOPMENT_TEAM).
    pub team: String,
    /// ExportOptions method; default "app-store-connect".
    #[serde(default)]
    pub export_method: Option<String>,
    /// ASC API key for `-allowProvisioningUpdates` in CI (optional locally, where the
    /// Xcode-account session signs). All three fields travel together.
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
}

/// Android release keystore (Gradle signingConfig; .aab is jar-signed by Gradle — §16.5).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct AndroidSigning {
    pub keystore: String,
    pub key_alias: String,
    pub store_pass: String,
    pub key_pass: String,
}

/// Windows Authenticode: certs are HSM/service-held since 2023 — a provider enum, not a .pfx path.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct WindowsSigning {
    /// "self-signed-dev" | "signtool-cert-store" | "azure-artifact-signing"
    pub provider: String,
    /// Cert subject for the MSIX Identity Publisher (must byte-match the signing cert subject).
    #[serde(default)]
    pub publisher: Option<String>,
    /// signtool-cert-store: SHA-1 thumbprint of the installed certificate.
    #[serde(default)]
    pub thumbprint: Option<String>,
    /// azure-artifact-signing: endpoint / account / certificate-profile (+ dlib path).
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    /// Path to Azure.CodeSigning.Dlib.dll (azure-artifact-signing).
    #[serde(default)]
    pub dlib: Option<String>,
    /// RFC-3161 timestamp URL; defaults per provider.
    #[serde(default)]
    pub timestamp_url: Option<String>,
}

/// OpenHarmony release signing material (hap-sign-tool).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OhosSigning {
    /// .p12 keystore path.
    pub keystore: String,
    pub key_alias: String,
    pub store_pass: String,
    pub key_pass: String,
    /// Release certificate (.cer) path.
    pub cert: String,
    /// Provisioning profile (.p7b) path.
    pub profile: String,
}

/// `[app]`: the Day-specific app identity. `name`/`version` are FILLED FROM Cargo.toml after
/// parsing (never written in Day.toml). Every other property can be overridden per platform /
/// toolkit / target via `[app.<key>]` tables collected in `overrides`.
#[derive(Debug, Deserialize)]
pub struct App {
    /// The crate name, from Cargo.toml `[package] name`.
    #[serde(skip)]
    pub name: String,
    /// The crate version, from Cargo.toml `[package] version`.
    #[serde(skip)]
    pub version: String,
    /// Application id / bundle id (reverse-DNS).
    pub id: String,
    /// Display title (window / app store); default: the crate name.
    #[serde(default)]
    pub title: Option<String>,
    /// Monotonic build number (versionCode / CFBundleVersion).
    #[serde(default = "default_build")]
    pub build: u64,
    /// The platform-toolkit combos this app ships on (`day app add-toolkit` appends here).
    #[serde(default)]
    pub targets: Vec<String>,
    /// `[app.<platform|toolkit|target>]` override tables — validated by `day lint`.
    /// (serde note: this flatten map is why App has no deny_unknown_fields — a typo'd scalar
    /// key still errors because it can't parse as an override TABLE.)
    #[serde(flatten)]
    pub overrides: BTreeMap<String, AppOverride>,
}

/// One `[app.<key>]` override table: any subset of the overridable `[app]` properties.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppOverride {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub build: Option<u64>,
}

/// The app identity a specific target builds with, after applying `[app.<key>]` overrides.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedApp {
    pub name: String,
    pub version: String,
    pub id: String,
    pub title: String,
    pub build: u64,
}

impl Manifest {
    /// Resolve the app identity for `target` (e.g. `macos-appkit`). Override precedence, most
    /// specific wins: `[app.<target>]` > `[app.<platform>]` > `[app.<toolkit>]` > `[app]`.
    pub fn resolve(&self, target: &str) -> ResolvedApp {
        let mut out = ResolvedApp {
            name: self.app.name.clone(),
            version: self.app.version.clone(),
            id: self.app.id.clone(),
            title: self
                .app
                .title
                .clone()
                .unwrap_or_else(|| self.app.name.clone()),
            build: self.app.build,
        };
        let platform = target.split('-').next().unwrap_or_default();
        let toolkit = target.split_once('-').map(|(_, t)| t).unwrap_or_default();
        // Increasing precedence: toolkit, then platform, then the exact target.
        for key in [toolkit, platform, target] {
            if let Some(o) = self.app.overrides.get(key) {
                if let Some(id) = &o.id {
                    out.id = id.clone();
                }
                if let Some(title) = &o.title {
                    out.title = title.clone();
                }
                if let Some(build) = o.build {
                    out.build = build;
                }
            }
        }
        out
    }
}

fn default_build() -> u64 {
    1
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
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

/// Parse Day.toml text + the sibling Cargo.toml's `[package]` into a Manifest.
pub fn parse_manifest(day_toml: &str, cargo_toml: &str) -> Result<Manifest, String> {
    let mut manifest: Manifest = toml::from_str(day_toml).map_err(|e| format!("Day.toml: {e}"))?;
    if manifest.schema != 1 {
        return Err(format!(
            "Day.toml: unsupported schema version {}",
            manifest.schema
        ));
    }
    // `name`/`version` are derived, never restated (a permissive parse: version may be
    // workspace-inherited in exotic layouts — fall back rather than fail).
    let cargo: toml::Value = toml::from_str(cargo_toml).map_err(|e| format!("Cargo.toml: {e}"))?;
    let package = cargo
        .get("package")
        .ok_or("Cargo.toml: no [package] table")?;
    manifest.app.name = package
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Cargo.toml: no package.name")?
        .to_string();
    manifest.app.version = package
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.1.0")
        .to_string();
    Ok(manifest)
}

/// Find the nearest ancestor directory containing Day.toml (from `start` or cwd).
pub fn find_project(start: Option<&Path>) -> Result<Project, String> {
    let mut dir = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    loop {
        let candidate = dir.join("Day.toml");
        if candidate.exists() {
            let day_toml = std::fs::read_to_string(&candidate).map_err(|e| e.to_string())?;
            let cargo_path = dir.join("Cargo.toml");
            let cargo_toml = std::fs::read_to_string(&cargo_path).map_err(|e| {
                format!(
                    "{}: {e} (Day.toml marks a Day project, which is also a cargo package)",
                    cargo_path.display()
                )
            })?;
            let manifest = parse_manifest(&day_toml, &cargo_toml)?;
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
            return Err("no Day.toml found in this directory or any ancestor".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO: &str = "[package]\nname = \"demo-app\"\nversion = \"1.2.3\"\n";

    #[test]
    fn identity_derives_from_cargo_toml() {
        let m = parse_manifest("schema = 1\n[app]\nid = \"dev.example.demo\"\n", CARGO).unwrap();
        assert_eq!(m.app.name, "demo-app");
        assert_eq!(m.app.version, "1.2.3");
        let r = m.resolve("macos-appkit");
        assert_eq!(r.title, "demo-app"); // no title ⇒ crate name
        assert_eq!(r.build, 1);
    }

    #[test]
    fn overrides_resolve_most_specific_wins() {
        let m = parse_manifest(
            r#"
schema = 1

[app]
id = "dev.example.demo"
title = "Demo"
targets = ["ios-uikit", "macos-appkit", "macos-qt"]

# toolkit-wide override
[app.qt]
title = "Demo (Qt)"

# platform override beats toolkit
[app.macos]
id = "dev.example.demo.mac"

# exact target beats both
[app.macos-qt]
title = "Demo for macOS Qt"
build = 7
"#,
            CARGO,
        )
        .unwrap();
        assert_eq!(m.resolve("ios-uikit").title, "Demo");
        assert_eq!(m.resolve("macos-appkit").id, "dev.example.demo.mac");
        assert_eq!(m.resolve("macos-appkit").title, "Demo");
        let mq = m.resolve("macos-qt");
        assert_eq!(mq.id, "dev.example.demo.mac"); // platform
        assert_eq!(mq.title, "Demo for macOS Qt"); // exact target beats [app.qt]
        assert_eq!(mq.build, 7);
        assert_eq!(m.resolve("linux-qt").title, "Demo (Qt)"); // toolkit layer
    }

    #[test]
    fn schema_and_shape_are_validated() {
        assert!(parse_manifest("schema = 2\n[app]\nid = \"x\"\n", CARGO).is_err());
        assert!(parse_manifest("schema = 1\n", CARGO).is_err()); // no [app]
        // A typo'd scalar under [app] can't parse as an override table.
        assert!(parse_manifest("schema = 1\n[app]\nid = \"x\"\ntitel = \"y\"\n", CARGO).is_err());
    }

    #[cfg(windows)]
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
