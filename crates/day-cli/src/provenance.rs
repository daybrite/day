//! Build provenance: what an artifact was built from, and what it was built with (DESIGN.md §20.4).
//!
//! Two documents, deliberately separate, because they have opposite requirements.
//!
//! The **SBOM** describes the software: the app, its source repository and commit, and its
//! dependencies with SPDX license identifiers. Everything in it is a property of the source, so it
//! is identical on every machine that builds a given commit — which is what lets it be embedded in
//! the artifact without making that artifact environment-specific. It ships in both CycloneDX and
//! SPDX form, and lands where the app can read it at runtime (§18.3 resource staging), so an app
//! can show its own license notices.
//!
//! The **buildinfo** describes the machine: exact compiler, SDK, and packaging-tool versions. Those
//! differ between machines by design, so embedding them would make the artifact differ too, and the
//! reproducibility checks in §20.3 would never pass across environments. It is written next to the
//! artifact as a sidecar instead. Debian's `.buildinfo` separates the two for the same reason.
//!
//! `day rebuild` reads the SBOM for *what to build* and the buildinfo for *what to build it with*.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::meta::Project;
use crate::targets::Target;

/// The schema version of both documents. Bump it when a field's meaning changes; `day rebuild`
/// refuses a major it does not know rather than guessing.
pub const SCHEMA: &str = "1.0";

/// A tool the build used, and how a reader would obtain the same version.
#[derive(Debug, Clone)]
pub struct ToolRecord {
    /// Stable key used by `--force-tool` (`rust`, `xcode`, `gradle`, …).
    pub key: String,
    /// Human name for messages.
    pub name: String,
    /// Version exactly as the tool reported it.
    pub version: String,
    /// How to get this version. Never executed — printed for the reader to run.
    pub install_hint: String,
}

/// Everything `day rebuild` needs about the machine that produced an artifact.
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub schema: String,
    pub target: String,
    pub profile: String,
    pub host_os: String,
    pub host_arch: String,
    pub tools: Vec<ToolRecord>,
    /// sha256 of each artifact this pack produced, keyed by file name.
    pub artifacts: Vec<(String, String)>,
}

/// Everything `day rebuild` needs about the source, and everything a license screen needs.
#[derive(Debug, Clone)]
pub struct Sbom {
    pub schema: String,
    pub app_id: String,
    pub app_name: String,
    pub app_version: String,
    pub app_build: u64,
    /// Source repository URL, when the project is a git checkout with a remote.
    pub repository: Option<String>,
    /// Commit the build came from. `None` for a checkout with no commits.
    pub commit: Option<String>,
    /// True when the working tree had uncommitted changes — a rebuild cannot match this artifact.
    pub dirty: bool,
    pub components: Vec<Component>,
}

/// One dependency, with its license normalized to an SPDX expression where possible.
#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub version: String,
    /// SPDX license expression, or `None` when the package declares none.
    pub license: Option<String>,
    /// Package URL (purl), the cross-ecosystem component identifier both formats use.
    pub purl: String,
    /// Which dependency graph this came from: `cargo`, `swiftpm`, `gradle`, `ohpm`.
    pub ecosystem: String,
}

/// Run a tool and capture one line of version output. Returns `None` when the tool is absent —
/// provenance records what a machine had, and a missing tool is a fact rather than an error.
fn probe(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = if text.trim().is_empty() {
        String::from_utf8_lossy(&out.stderr).to_string()
    } else {
        text.to_string()
    };
    text.lines().next().map(|l| l.trim().to_string())
}

/// Normalize a Cargo `license` field into an SPDX expression.
///
/// Cargo has accepted `/` as an OR separator since before SPDX expressions were standardized, so
/// older crates carry `MIT/Apache-2.0` and even `Apache-2.0 / MIT`. SPDX only understands `OR`, and
/// a scanner reading the raw field will either fail or silently treat it as one unknown license.
pub fn normalize_license(raw: &str) -> String {
    let joined = raw
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ");
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The git remote and commit for a project directory, when it is a checkout.
fn git_source(root: &Path) -> (Option<String>, Option<String>, bool) {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let remote = git(&["remote", "get-url", "origin"]).map(|u| {
        // Normalize scp-style git@host:owner/repo.git to a URL a reader can open.
        if let Some(rest) = u.strip_prefix("git@") {
            let rest = rest.replacen(':', "/", 1);
            format!("https://{}", rest.trim_end_matches(".git"))
        } else {
            u.trim_end_matches(".git").to_string()
        }
    });
    let commit = git(&["rev-parse", "HEAD"]);
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    (remote, commit, dirty)
}

/// Rust dependency components, read from `cargo metadata`.
///
/// Native dependency graphs (SwiftPM, Gradle, ohpm) are not collected yet; they are the next slice
/// of this feature, and their absence is recorded in the document rather than passed over silently.
fn cargo_components(root: &Path) -> Vec<Component> {
    let Ok(out) = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--format-version", "1"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    let mut comps: Vec<Component> = meta
        .get("packages")
        .and_then(|p| p.as_array())
        .map(|pkgs| {
            pkgs.iter()
                .filter_map(|p| {
                    let name = p.get("name")?.as_str()?.to_string();
                    let version = p.get("version")?.as_str()?.to_string();
                    let license = p
                        .get("license")
                        .and_then(|l| l.as_str())
                        .map(normalize_license);
                    Some(Component {
                        purl: format!("pkg:cargo/{name}@{version}"),
                        name,
                        version,
                        license,
                        ecosystem: "cargo".into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    comps.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    comps.dedup_by(|a, b| a.purl == b.purl);
    comps
}

/// Collect the SBOM for a project. Deterministic: every field is a property of the source.
pub fn collect_sbom(project: &Project) -> Sbom {
    let (repository, commit, dirty) = git_source(&project.root);
    Sbom {
        schema: SCHEMA.into(),
        app_id: project.manifest.app.id.clone(),
        app_name: project.manifest.app.name.clone(),
        app_version: project.manifest.app.version.clone(),
        app_build: project.manifest.app.build,
        repository,
        commit,
        dirty,
        components: cargo_components(&project.root),
    }
}

/// Collect the build environment for a target. Every value here is machine-specific.
pub fn collect_buildinfo(target: &Target, profile: &str) -> BuildInfo {
    let mut tools = Vec::new();
    let mut add = |key: &str, name: &str, version: Option<String>, hint: &str| {
        if let Some(v) = version {
            tools.push(ToolRecord {
                key: key.into(),
                name: name.into(),
                version: v,
                install_hint: hint.into(),
            });
        }
    };

    add(
        "rust",
        "rustc",
        probe("rustc", &["--version"]),
        "rustup toolchain install <version> && rustup override set <version>",
    );
    add(
        "cargo",
        "cargo",
        probe("cargo", &["--version"]),
        "ships with the matching rustc toolchain (rustup)",
    );
    add(
        "day",
        "day CLI",
        Some(env!("DAY_VERSION_LONG").to_string()),
        "cargo install day-cli --version <version>",
    );

    match target.toolkit {
        "appkit" | "uikit" => {
            add(
                "xcode",
                "Xcode",
                probe("xcodebuild", &["-version"]),
                "https://developer.apple.com/download/all/?q=Xcode — install, then \
                 sudo xcode-select -s /Applications/Xcode_<version>.app",
            );
            add(
                "clang",
                "Apple clang",
                probe("clang", &["--version"]),
                "ships with Xcode; selected by xcode-select",
            );
        }
        "mdc" => {
            add(
                "gradle",
                "Gradle",
                probe("gradle", &["--version"]).or_else(|| probe("gradle", &["-v"])),
                "https://gradle.org/releases/ (or the project's ./gradlew wrapper)",
            );
            add(
                "java",
                "JDK",
                probe("javac", &["-version"]),
                "brew install openjdk@<major> / apt install openjdk-<major>-jdk",
            );
            add(
                "ndk",
                "Android NDK",
                std::env::var("ANDROID_NDK_HOME")
                    .ok()
                    .and_then(|p| ndk_version(&p)),
                "sdkmanager 'ndk;<version>' — https://developer.android.com/ndk/downloads",
            );
        }
        "arkui" => {
            add(
                "ohos-sdk",
                "OpenHarmony SDK",
                std::env::var("OHOS_BASE_SDK_HOME").ok(),
                "https://gitee.com/openharmony/docs — see docs/harmonyos.md for the layout",
            );
            add(
                "hvigor",
                "hvigor",
                probe("hvigorw", &["--version"]),
                "ships with the OpenHarmony command-line-tools",
            );
        }
        "xaml" => {
            add(
                "msvc",
                "MSVC toolchain",
                probe("cl", &["/?"]),
                "https://visualstudio.microsoft.com/downloads/ (Desktop development with C++)",
            );
            add(
                "nsis",
                "NSIS",
                probe("makensis", &["/VERSION"]).or_else(|| probe("makensis", &["-VERSION"])),
                "choco install nsis / winget install NSIS.NSIS",
            );
        }
        "gtk" | "qt" => {
            add(
                "flatpak-builder",
                "flatpak-builder",
                probe("flatpak-builder", &["--version"]),
                "apt install flatpak-builder / dnf install flatpak-builder",
            );
            add(
                "cc",
                "C compiler",
                probe("cc", &["--version"]),
                "apt install build-essential",
            );
        }
        _ => {}
    }

    BuildInfo {
        schema: SCHEMA.into(),
        target: target.name.into(),
        profile: profile.into(),
        host_os: crate::targets::host_os().into(),
        host_arch: std::env::consts::ARCH.into(),
        tools,
        artifacts: Vec::new(),
    }
}

/// The NDK version from its `source.properties`, which is the only place it is recorded.
fn ndk_version(ndk_home: &str) -> Option<String> {
    let text = std::fs::read_to_string(Path::new(ndk_home).join("source.properties")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("Pkg.Revision"))
        .map(|v| v.trim_start_matches([' ', '=']).trim().to_string())
}

/// The SBOM as CycloneDX 1.5 JSON.
pub fn cyclonedx(sbom: &Sbom) -> serde_json::Value {
    let components: Vec<serde_json::Value> = sbom
        .components
        .iter()
        .map(|c| {
            let mut o = serde_json::json!({
                "type": "library",
                "name": c.name,
                "version": c.version,
                "purl": c.purl,
            });
            if let Some(l) = &c.license {
                // CycloneDX wants `expression` for anything that is not a single bare id.
                o["licenses"] = serde_json::json!([{ "expression": l }]);
            }
            // Which dependency graph this came from. Every component is `cargo` today; the field
            // is what will distinguish them once the native graphs are collected.
            o["properties"] =
                serde_json::json!([{ "name": "day:ecosystem", "value": c.ecosystem }]);
            o
        })
        .collect();
    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "bom-ref": sbom.app_id,
                "name": sbom.app_name,
                "version": sbom.app_version,
            },
            "properties": source_properties(sbom),
        },
        "components": components,
    })
}

/// The SBOM as SPDX 2.3 JSON.
pub fn spdx(sbom: &Sbom) -> serde_json::Value {
    // `sourceInfo` carries the same repository/commit CycloneDX puts in properties. Without it an
    // SPDX-only SBOM could not drive `day rebuild`, and `sbom = "sidecar spdx"` is a valid choice.
    let source_info = format!(
        "repository={} commit={} dirty={}",
        sbom.repository.as_deref().unwrap_or("NOASSERTION"),
        sbom.commit.as_deref().unwrap_or("NOASSERTION"),
        sbom.dirty
    );
    let mut packages = vec![serde_json::json!({
        "SPDXID": "SPDXRef-Application",
        "name": sbom.app_name,
        "versionInfo": sbom.app_version,
        "downloadLocation": sbom.repository.clone().unwrap_or_else(|| "NOASSERTION".into()),
        "sourceInfo": source_info,
        "filesAnalyzed": false,
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": "NOASSERTION",
    })];
    packages.extend(sbom.components.iter().enumerate().map(|(i, c)| {
        serde_json::json!({
            "SPDXID": format!("SPDXRef-Package-{i}"),
            "name": c.name,
            "versionInfo": c.version,
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": false,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": c.license.clone().unwrap_or_else(|| "NOASSERTION".into()),
            "externalRefs": [{
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": c.purl,
            }],
        })
    }));
    serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("{}-{}", sbom.app_name, sbom.app_version),
        "documentNamespace": format!("https://daybrite.dev/spdx/{}/{}", sbom.app_id, sbom.app_version),
        "creationInfo": {
            // No timestamp: a creation date would make the document differ per build, and this
            // document is embedded in the artifact (§20.3).
            "creators": [format!("Tool: day-cli-{}", env!("CARGO_PKG_VERSION"))],
            "created": "1970-01-01T00:00:00Z",
        },
        "packages": packages,
    })
}

/// Source facts shared by both formats, as CycloneDX properties.
fn source_properties(sbom: &Sbom) -> Vec<serde_json::Value> {
    let mut props = vec![
        serde_json::json!({"name": "day:schema", "value": sbom.schema}),
        serde_json::json!({"name": "day:app-id", "value": sbom.app_id}),
        serde_json::json!({"name": "day:app-build", "value": sbom.app_build.to_string()}),
        serde_json::json!({"name": "day:dirty", "value": sbom.dirty.to_string()}),
        // Named so a reader knows the gap is deliberate rather than an empty graph.
        serde_json::json!({"name": "day:native-deps", "value": "not-collected"}),
    ];
    if let Some(r) = &sbom.repository {
        props.push(serde_json::json!({"name": "day:repository", "value": r}));
    }
    if let Some(c) = &sbom.commit {
        props.push(serde_json::json!({"name": "day:commit", "value": c}));
    }
    props
}

/// The buildinfo sidecar, as JSON.
pub fn buildinfo_json(info: &BuildInfo) -> serde_json::Value {
    serde_json::json!({
        "schema": info.schema,
        "target": info.target,
        "profile": info.profile,
        "host": { "os": info.host_os, "arch": info.host_arch },
        "tools": info.tools.iter().map(|t| serde_json::json!({
            "key": t.key,
            "name": t.name,
            "version": t.version,
            "install": t.install_hint,
        })).collect::<Vec<_>>(),
        "artifacts": info.artifacts.iter().map(|(n, d)| serde_json::json!({
            "name": n, "sha256": d,
        })).collect::<Vec<_>>(),
    })
}

/// Write the configured SBOM documents into `dir`, returning the paths written.
pub fn write_sbom(
    dir: &Path,
    sbom: &Sbom,
    formats: &[crate::meta::SbomFormat],
) -> Result<Vec<PathBuf>, String> {
    use crate::meta::SbomFormat;
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for f in formats {
        let doc = match f {
            SbomFormat::Cyclonedx => cyclonedx(sbom),
            SbomFormat::Spdx => spdx(sbom),
        };
        let path = dir.join(f.file_name());
        let text =
            serde_json::to_string_pretty(&doc).map_err(|e| format!("serializing SBOM: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        out.push(path);
    }
    Ok(out)
}

/// Copy already-generated SBOM documents into a directory that becomes part of the artifact.
///
/// Called from each packer with the location that toolkit's runtime can read: the app bundle's
/// `Resources` on Apple, `assets/` in an `.apk`, `rawfile/` in a `.hap`, and the payload staged
/// for the Linux and Windows containers. Does nothing when no documents were generated.
pub fn embed_into(sbom_dir: &Path, dest: &Path) -> Result<(), String> {
    if !sbom_dir.is_dir() {
        return Ok(());
    }
    let dest = dest.join("sbom");
    std::fs::create_dir_all(&dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;
    for entry in std::fs::read_dir(sbom_dir)
        .map_err(|e| format!("reading {}: {e}", sbom_dir.display()))?
        .flatten()
    {
        let from = entry.path();
        if from.is_file() {
            let to = dest.join(entry.file_name());
            std::fs::copy(&from, &to)
                .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Write the buildinfo sidecar next to an artifact.
pub fn write_buildinfo(path: &Path, info: &BuildInfo) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&buildinfo_json(info))
        .map_err(|e| format!("serializing buildinfo: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_slash_separators_become_spdx_or() {
        assert_eq!(normalize_license("MIT/Apache-2.0"), "MIT OR Apache-2.0");
        assert_eq!(normalize_license("Apache-2.0 / MIT"), "Apache-2.0 OR MIT");
        // Already-valid expressions survive untouched.
        assert_eq!(normalize_license("MIT OR Apache-2.0"), "MIT OR Apache-2.0");
        assert_eq!(normalize_license("Unicode-3.0"), "Unicode-3.0");
        assert_eq!(
            normalize_license("Apache-2.0 WITH LLVM-exception"),
            "Apache-2.0 WITH LLVM-exception"
        );
    }

    #[test]
    fn the_spdx_document_carries_no_build_timestamp() {
        // A creation date would differ per build and this document is embedded in the artifact,
        // so it must be pinned (§20.3).
        let sbom = Sbom {
            schema: SCHEMA.into(),
            app_id: "dev.example.app".into(),
            app_name: "App".into(),
            app_version: "1.0.0".into(),
            app_build: 1,
            repository: None,
            commit: None,
            dirty: false,
            components: Vec::new(),
        };
        let doc = spdx(&sbom);
        assert_eq!(doc["creationInfo"]["created"], "1970-01-01T00:00:00Z");
    }
}

// --- Debian .buildinfo (deb822) ------------------------------------------------------------------
// The JSON sidecar above is Day's own, and `day rebuild` reads it on every platform. For the Linux
// targets Day additionally emits a file in Debian's deb822 `.buildinfo` format (deb-buildinfo(5)),
// because that is what Debian's reproducibility tooling and its maintainers already consume.
//
// It is a Debian-FORMAT file describing a build that is not a Debian source package: Day's Linux
// artifact is a `.flatpak`, so `Source`/`Binary`/`Version` carry the app's identity rather than a
// dpkg source package's. The fields are syntactically what deb-buildinfo(5) specifies; the values
// describe a Day app.

/// The Debian architecture name for the host, or `None` when it has no Debian equivalent.
fn debian_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        "arm" => Some("armhf"),
        "x86" => Some("i386"),
        "riscv64" => Some("riscv64"),
        "powerpc64" => Some("ppc64el"),
        "s390x" => Some("s390x"),
        _ => None,
    }
}

/// `Installed-Build-Depends`: every configured package on the build host, with its exact version.
///
/// Only meaningful on a dpkg system. Day builds on Fedora, Arch, and macOS too, and inventing this
/// list there would be worse than omitting it — a maintainer would trust versions that were never
/// installed. Returns `None` when `dpkg-query` is absent or fails.
fn dpkg_installed() -> Option<Vec<String>> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f=${binary:Package} (= ${Version})\n"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut list: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    list.sort();
    list.dedup();
    (!list.is_empty()).then_some(list)
}

/// Render a Debian `.buildinfo` (deb822, `Format: 1.0`) for a Linux target.
///
/// `runtime` is the Flathub runtime the app links against. Debian's schema has no field for it, so
/// it rides as an `X-Day-*` field: deb822 parsers ignore unknown fields, and omitting it would
/// leave the document describing only the build host while the app actually runs against the
/// runtime. A rebuild needs both.
pub fn debian_buildinfo(
    sbom: &Sbom,
    info: &BuildInfo,
    build_path: &Path,
    runtime: Option<(&str, String)>,
) -> String {
    let arch = debian_arch().unwrap_or("unknown");
    let mut out = String::new();
    let field = |out: &mut String, k: &str, v: &str| {
        out.push_str(&format!("{k}: {v}\n"));
    };
    field(&mut out, "Format", "1.0");
    field(&mut out, "Source", &sbom.app_name);
    field(&mut out, "Binary", &sbom.app_name);
    field(&mut out, "Architecture", arch);
    field(&mut out, "Version", &sbom.app_version);
    field(&mut out, "Build-Origin", "Day");
    field(&mut out, "Build-Architecture", arch);
    // Debian records the build path because Debian builds are path-sensitive. Day normalizes paths
    // (§20.3), so this is informational here rather than required to reproduce.
    field(&mut out, "Build-Path", &build_path.display().to_string());
    if let Some(k) = probe("uname", &["-sr"]) {
        field(&mut out, "Build-Kernel-Version", &k);
    }

    // Checksums-Sha256 is a folded field: one "<sha256> <size> <name>" line per file, indented.
    out.push_str("Checksums-Sha256:\n");
    for (name, digest) in &info.artifacts {
        let size = build_path
            .join("build/day/dist")
            .join(name)
            .metadata()
            .map(|m| m.len())
            .unwrap_or(0);
        out.push_str(&format!(" {digest} {size} {name}\n"));
    }

    out.push_str("Installed-Build-Depends:\n");
    match dpkg_installed() {
        Some(pkgs) => {
            for (i, p) in pkgs.iter().enumerate() {
                let sep = if i + 1 == pkgs.len() { "" } else { "," };
                out.push_str(&format!(" {p}{sep}\n"));
            }
        }
        // Named rather than left blank, so a reader knows the host was not dpkg-based instead of
        // concluding nothing was installed.
        None => out.push_str(" # not a dpkg host — see X-Day-Tools for the toolchain versions\n"),
    }

    if let Some((id, version)) = runtime {
        field(
            &mut out,
            "X-Day-Flatpak-Runtime",
            &format!("{id}//{version}"),
        );
    }
    if let Some(r) = &sbom.repository {
        field(&mut out, "X-Day-Repository", r);
    }
    if let Some(c) = &sbom.commit {
        field(&mut out, "X-Day-Commit", c);
    }
    field(&mut out, "X-Day-Dirty", &sbom.dirty.to_string());
    field(&mut out, "X-Day-Target", &info.target);
    field(&mut out, "X-Day-Profile", &info.profile);
    let tools = info
        .tools
        .iter()
        .map(|t| format!("{} (= {})", t.key, t.version))
        .collect::<Vec<_>>()
        .join(", ");
    field(&mut out, "X-Day-Tools", &tools);
    out
}

/// Debian's filename convention: `${source}_${version}_${arch}.buildinfo`.
pub fn debian_buildinfo_name(sbom: &Sbom) -> String {
    let source = sbom.app_name.replace(' ', "-").to_lowercase();
    format!(
        "{source}_{}_{}.buildinfo",
        sbom.app_version,
        debian_arch().unwrap_or("unknown")
    )
}

#[cfg(test)]
mod debian_tests {
    use super::*;

    fn fixture() -> (Sbom, BuildInfo) {
        let sbom = Sbom {
            schema: SCHEMA.into(),
            app_id: "dev.daybrite.showcase".into(),
            app_name: "showcase".into(),
            app_version: "0.1.2".into(),
            app_build: 1,
            repository: Some("https://github.com/daybrite/day".into()),
            commit: Some("3cf799cb9c9dc4bc045bc9f1457aed2838bf5e1d".into()),
            dirty: false,
            components: Vec::new(),
        };
        let info = BuildInfo {
            schema: SCHEMA.into(),
            target: "linux-gtk".into(),
            profile: "release".into(),
            host_os: "linux".into(),
            host_arch: "x86_64".into(),
            tools: vec![ToolRecord {
                key: "rust".into(),
                name: "rustc".into(),
                version: "1.97.0".into(),
                install_hint: "rustup".into(),
            }],
            artifacts: vec![("showcase-gtk-x86_64.flatpak".into(), "abc123".into())],
        };
        (sbom, info)
    }

    /// deb-buildinfo(5) names these as required; a file missing any of them is not a .buildinfo.
    #[test]
    fn carries_every_required_field() {
        let (sbom, info) = fixture();
        let text = debian_buildinfo(&sbom, &info, Path::new("/build"), None);
        for field in [
            "Format: 1.0",
            "Source:",
            "Binary:",
            "Architecture:",
            "Version:",
            "Build-Architecture:",
            "Checksums-Sha256:",
            "Installed-Build-Depends:",
        ] {
            assert!(
                text.contains(field),
                "missing required field {field}:\n{text}"
            );
        }
    }

    /// deb822 folded fields continue on lines that begin with a space; a continuation line that
    /// starts in column 0 would be parsed as a new field and silently corrupt the document.
    #[test]
    fn folded_fields_are_indented() {
        let (sbom, info) = fixture();
        let text = debian_buildinfo(&sbom, &info, Path::new("/build"), None);
        let mut in_folded = false;
        for line in text.lines() {
            if line.ends_with(':') && !line.starts_with(' ') {
                in_folded = true;
                continue;
            }
            if in_folded && !line.starts_with(' ') {
                in_folded = false;
            }
            if in_folded {
                assert!(line.starts_with(' '), "continuation not indented: {line:?}");
            }
        }
        // The checksum line carries digest, size, and name, in that order.
        assert!(
            text.contains(" abc123 0 showcase-gtk-x86_64.flatpak"),
            "{text}"
        );
    }

    /// The runtime a flatpak links against has no Debian field, so it rides as X-Day-*. Losing it
    /// would leave the document describing the build host but not what the app runs against.
    #[test]
    fn the_flatpak_runtime_is_recorded() {
        let (sbom, info) = fixture();
        let text = debian_buildinfo(
            &sbom,
            &info,
            Path::new("/build"),
            Some(("org.gnome.Platform", "48".into())),
        );
        assert!(
            text.contains("X-Day-Flatpak-Runtime: org.gnome.Platform//48"),
            "{text}"
        );
        assert!(text.contains("X-Day-Commit: 3cf799cb"), "{text}");
    }

    #[test]
    fn the_filename_follows_debian_convention() {
        let (sbom, _) = fixture();
        let name = debian_buildinfo_name(&sbom);
        assert!(name.starts_with("showcase_0.1.2_"), "{name}");
        assert!(name.ends_with(".buildinfo"), "{name}");
    }
}
