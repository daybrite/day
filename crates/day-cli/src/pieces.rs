//! Standalone-piece backend discovery (docs/extending.md). External piece crates (e.g.
//! `day-piece-picker`) declare their per-toolkit backend contributions in `Cargo.toml` under
//! `[package.metadata.day.<toolkit>]`; the day CLI reads them from `cargo metadata` and folds them
//! into the native build — so a piece carries BOTH its front-end (Rust) and its backend (Java /
//! Gradle deps / …) with ZERO edits to the core day crates.
//!
//! Android contract (`[package.metadata.day.android]`):
//! ```toml
//! java = ["android/java"]                 # dirs (rel. to the crate) → Gradle java srcDirs
//! gradle-dependencies = ["g:a:v", …]      # → the app module's dependencies { }
//! gradle-repositories = ["https://…", …]  # → extra Maven repos
//! permissions = ["android.permission.INTERNET", …]  # → <uses-permission>s merged into the manifest
//! ```
//! The resolved contributions are written to `build/day/android/day-pieces.json`, which the app's
//! `build.gradle.kts` reads generically (loops over the lists — no per-piece Gradle edits, ever).
//! Permissions additionally go into a generated manifest overlay (`day-pieces-manifest.xml`) that the
//! scaffold points its debug+release source-set manifests at, so AGP merges them into the app manifest.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::meta::Project;

/// The build-side contribution list handed to Gradle (serialized to day-pieces.json).
#[derive(Debug, Default, Serialize)]
pub struct AndroidPieces {
    /// Absolute Java/Kotlin source dirs to add as Gradle `java.srcDir`s.
    #[serde(rename = "javaSrcDirs")]
    pub java_src_dirs: Vec<String>,
    /// Gradle dependency coordinates (`group:artifact:version`).
    pub dependencies: Vec<String>,
    /// Extra Maven repository URLs.
    pub repositories: Vec<String>,
    /// Android `<uses-permission>` names to merge into the app manifest.
    pub permissions: Vec<String>,
}

// --- `cargo metadata` JSON (only the fields we need) ---

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Option<Resolve>,
}
#[derive(Deserialize)]
struct Package {
    id: String,
    manifest_path: String,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct Resolve {
    root: Option<String>,
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    id: String,
    #[serde(default)]
    deps: Vec<Dep>,
}
#[derive(Deserialize)]
struct Dep {
    pkg: String,
}

/// The `[package.metadata.day.android]` table, as declared by a piece crate.
#[derive(Deserialize, Default)]
struct AndroidMeta {
    #[serde(default)]
    java: StringOrVec,
    #[serde(default, rename = "gradle-dependencies")]
    gradle_dependencies: Vec<String>,
    #[serde(default, rename = "gradle-repositories")]
    gradle_repositories: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
}

/// Accept `java = "android/java"` or `java = ["a", "b"]`.
#[derive(Default)]
struct StringOrVec(Vec<String>);
impl<'de> Deserialize<'de> for StringOrVec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum V {
            One(String),
            Many(Vec<String>),
        }
        Ok(StringOrVec(match V::deserialize(d)? {
            V::One(s) => vec![s],
            V::Many(v) => v,
        }))
    }
}

/// Resolve every piece in the app's Android dependency closure and collect its contributions.
/// The `features` are the ones the Android build compiles with (so only pieces actually pulled in
/// by that feature set contribute) — currently `["widget"]`, no default features.
pub fn resolve_android(project: &Project, features: &[&str]) -> Result<AndroidPieces, String> {
    let manifest = project.root.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--format-version", "1", "--no-default-features"])
        .arg("--manifest-path")
        .arg(&manifest);
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    let out = cmd.output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next_back()
                .unwrap_or("")
        ));
    }
    let meta: Metadata =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata parse: {e}"))?;

    // Transitive closure of package ids reachable from the resolve root (the app).
    let in_closure = closure(&meta);

    let mut pieces = AndroidPieces::default();
    let mut seen_java = HashSet::new();
    for pkg in &meta.packages {
        if !in_closure.contains(&pkg.id) {
            continue;
        }
        let Some(android) = pkg
            .metadata
            .as_ref()
            .and_then(|m| m.get("day"))
            .and_then(|d| d.get("android"))
        else {
            continue;
        };
        let android: AndroidMeta = match serde_json::from_value(android.clone()) {
            Ok(a) => a,
            Err(e) => {
                eprintln!(
                    "day: {} has malformed [package.metadata.day.android]: {e}",
                    pkg.manifest_path
                );
                continue;
            }
        };
        let crate_dir = Path::new(&pkg.manifest_path)
            .parent()
            .unwrap_or(Path::new("."));
        for rel in &android.java.0 {
            let dir = crate_dir.join(rel);
            if !dir.is_dir() {
                eprintln!("day: {} java dir {:?} not found — skipping", pkg.id, dir);
                continue;
            }
            let abs = dir.to_string_lossy().into_owned();
            if seen_java.insert(abs.clone()) {
                pieces.java_src_dirs.push(abs);
            }
        }
        for dep in android.gradle_dependencies {
            if !pieces.dependencies.contains(&dep) {
                pieces.dependencies.push(dep);
            }
        }
        for repo in android.gradle_repositories {
            if !pieces.repositories.contains(&repo) {
                pieces.repositories.push(repo);
            }
        }
        for perm in android.permissions {
            if !pieces.permissions.contains(&perm) {
                pieces.permissions.push(perm);
            }
        }
    }
    Ok(pieces)
}

/// Package ids transitively reachable from the resolve root (falls back to "all resolved" if the
/// root is a virtual workspace with no single root).
fn closure(meta: &Metadata) -> HashSet<String> {
    let Some(resolve) = &meta.resolve else {
        return meta.packages.iter().map(|p| p.id.clone()).collect();
    };
    let by_id: std::collections::HashMap<&str, &Node> =
        resolve.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let Some(root) = &resolve.root else {
        return resolve.nodes.iter().map(|n| n.id.clone()).collect();
    };
    let mut seen = HashSet::new();
    let mut stack = vec![root.clone()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(node) = by_id.get(id.as_str()) {
            for d in &node.deps {
                stack.push(d.pkg.clone());
            }
        }
    }
    seen
}

/// Write the resolved contributions to `build/day/android/day-pieces.json` for Gradle to read (and,
/// when pieces contribute Android permissions, a `day-pieces-manifest.xml` overlay the scaffold
/// merges). Always writes (an empty manifest when there are no pieces) so a stale file never lingers.
pub fn write_android_manifest(project: &Project) -> Result<(), String> {
    let pieces = resolve_android(project, &["widget"]).unwrap_or_else(|e| {
        eprintln!("day: piece discovery failed ({e}); building with framework pieces only");
        AndroidPieces::default()
    });
    let dir = project.root.join("build/day/android");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&pieces).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("day-pieces.json"), json).map_err(|e| e.to_string())?;

    // Permissions → a manifest overlay AGP merges into the app manifest (the scaffold points its
    // debug+release source-set manifests here). Remove any stale overlay when there are none.
    let overlay = dir.join("day-pieces-manifest.xml");
    if pieces.permissions.is_empty() {
        let _ = std::fs::remove_file(&overlay);
    } else {
        std::fs::write(&overlay, permissions_manifest(&pieces.permissions))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A minimal manifest carrying only the pieces' `<uses-permission>`s — merged into the app manifest
/// by AGP's manifest merger (which also dedups against any the app already declares).
fn permissions_manifest(permissions: &[String]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!-- Generated by `day build` from standalone-piece [package.metadata.day.android] \
         permissions. Do not edit. -->\n\
         <manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n",
    );
    for perm in permissions {
        s.push_str(&format!(
            "    <uses-permission android:name=\"{perm}\" />\n"
        ));
    }
    s.push_str("</manifest>\n");
    s
}
