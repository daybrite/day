//! Standalone-piece backend discovery (docs/extending.md). External piece crates (e.g.
//! `day-piece-picker`) declare their per-toolkit backend contributions in `Cargo.toml` under
//! `[package.metadata.day.<toolkit>]`; the Day CLI reads them from `cargo metadata` and folds them
//! into the native build — so a piece carries BOTH its front-end (Rust) and its backend (Java /
//! Gradle deps / …) without touching the core Day crates.
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
//!
//! iOS contract (`[package.metadata.day.ios]`):
//! ```toml
//! swift = ["ios/swift"]                 # dirs (rel. to the crate) of Swift shim sources
//! swift-packages = [                    # SwiftPM package dependencies to link
//!   { url = "https://…", from = "1.0.0", products = ["Foo"] },
//! ]
//! ```
//! Xcode is not script-driven like Gradle, so instead the CLI generates a LOCAL SwiftPM package at
//! `build/day/ios/DayPieces` — its `Package.swift` lists every piece's `swift-packages` as
//! dependencies and compiles every piece's staged Swift shims. The app's checked-in `.xcodeproj`
//! depends on that one local package (the iOS analog of the Gradle scaffold), so adding an iOS piece
//! is pure `Cargo.toml` data — no `.xcodeproj` edits, ever.

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
    name: String,
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

/// The `[package.metadata.day.piece]` marker a standalone piece declares to name the backends it
/// carries a native-renderer *feature* for. The Day CLI unions `<pkg>/<backend>` into the app build
/// (see [`feature_union`]) so the app need only depend on the piece — never re-list its per-backend
/// features. COMPOSE pieces (built from core pieces, no per-backend feature) omit this table and so
/// contribute nothing.
#[derive(Deserialize, Default)]
struct PieceMeta {
    /// Backend toolkit names (`appkit`, `gtk`, `qt`, `uikit`, `widget`, `winui`, `mock`) this piece
    /// declares a `[features]` entry for. Only these get `<pkg>/<backend>` unioned in.
    #[serde(default)]
    backends: Vec<String>,
}

/// Compute the extra `--features` entries that wire each standalone piece's per-backend renderer into
/// a build whose toolkit is `backend`. Scans the app's dependency closure for pieces declaring
/// `[package.metadata.day.piece].backends` that INCLUDE `backend` and returns one `<pkg>/<backend>`
/// per match (deduped, sorted). This lets the app depend on a piece with a plain `{ workspace = true }`
/// and no per-backend feature fan-out — the CLI derives them here.
///
/// Robustness: only pieces that ACTUALLY declare `backend` contribute (so `cargo`'s "feature does not
/// exist" / "not a direct dependency" errors can't fire), and a metadata failure degrades to an empty
/// list (warn, don't fail) so the app still builds with whatever features it lists itself. Because the
/// union is additive, an app that still lists the per-piece features stays correct (dupes are fine).
pub fn feature_union(project: &Project, backend: &str) -> Vec<String> {
    let meta = match cargo_metadata(project, &[backend]) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "day: piece feature discovery failed ({e}); \
                 building with the app's declared features only"
            );
            return Vec::new();
        }
    };
    let in_closure = closure(&meta);
    let mut feats = Vec::new();
    for pkg in &meta.packages {
        if !in_closure.contains(&pkg.id) {
            continue;
        }
        let Some(piece) = piece_meta::<PieceMeta>(pkg, "piece") else {
            continue;
        };
        if piece.backends.iter().any(|b| b == backend) {
            feats.push(format!("{}/{backend}", pkg.name));
        }
    }
    feats.sort();
    feats.dedup();
    feats
}

/// Run `cargo metadata` for the app with a specific feature selection (no default features), so only
/// pieces actually pulled in by that backend's features are considered.
fn cargo_metadata(project: &Project, features: &[&str]) -> Result<Metadata, String> {
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
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata parse: {e}"))
}

/// Deserialize a piece's `[package.metadata.day.<toolkit>]` table, warning (not failing) on a
/// malformed one. Returns `None` when the piece declares no such table.
fn piece_meta<T: serde::de::DeserializeOwned>(pkg: &Package, toolkit: &str) -> Option<T> {
    let table = pkg
        .metadata
        .as_ref()
        .and_then(|m| m.get("day")) // Cargo.toml `[package.metadata.day.*]` — lowercase key
        .and_then(|d| d.get(toolkit))?;
    match serde_json::from_value(table.clone()) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!(
                "day: {} has malformed [package.metadata.day.{toolkit}]: {e}",
                pkg.manifest_path
            );
            None
        }
    }
}

/// Resolve every piece in the app's Android dependency closure and collect its contributions.
/// The `features` are the ones the Android build compiles with (so only pieces actually pulled in
/// by that feature set contribute) — currently `["widget"]`, no default features.
pub fn resolve_android(project: &Project, features: &[&str]) -> Result<AndroidPieces, String> {
    let meta = cargo_metadata(project, features)?;

    // Transitive closure of package ids reachable from the resolve root (the app).
    let in_closure = closure(&meta);

    let mut pieces = AndroidPieces::default();
    let mut seen_java = HashSet::new();
    for pkg in &meta.packages {
        if !in_closure.contains(&pkg.id) {
            continue;
        }
        let Some(android) = piece_meta::<AndroidMeta>(pkg, "android") else {
            continue;
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

// ===========================================================================
// iOS — a piece's Swift shims + SwiftPM package dependencies
// ===========================================================================

/// A SwiftPM package dependency declared by a piece (`[package.metadata.day.ios].swift-packages`).
#[derive(Debug, Clone, Deserialize)]
struct SwiftPackage {
    url: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    exact: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    products: Vec<String>,
}

impl SwiftPackage {
    /// SwiftPM derives a package's identity from the last path component of its URL (sans `.git`).
    fn identity(&self) -> String {
        self.url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&self.url)
            .trim_end_matches(".git")
            .to_string()
    }
    /// The version requirement clause for `.package(url:, …)`.
    fn requirement(&self) -> String {
        if let Some(v) = &self.exact {
            format!("exact: \"{v}\"")
        } else if let Some(b) = &self.branch {
            format!("branch: \"{b}\"")
        } else if let Some(r) = &self.revision {
            format!("revision: \"{r}\"")
        } else {
            // Default to `from:` (allows compatible newer versions); fall back to any version.
            format!("from: \"{}\"", self.from.as_deref().unwrap_or("0.0.0"))
        }
    }
}

/// The `[package.metadata.day.ios]` table, as declared by a piece crate.
#[derive(Deserialize, Default)]
struct IosMeta {
    #[serde(default)]
    swift: StringOrVec,
    #[serde(default, rename = "swift-packages")]
    swift_packages: Vec<SwiftPackage>,
    /// System frameworks to link (e.g. `["WebKit"]`) — so a piece needn't `dlopen` or hand-`#[link]`.
    #[serde(default)]
    frameworks: Vec<String>,
}

/// The resolved iOS contributions across all pieces in the app's dependency closure.
#[derive(Default)]
struct IosPieces {
    /// `(namespace, absolute dir)` Swift source dirs to compile — the namespace (the piece's crate
    /// name) subfolders the staged shims so two pieces' files can't collide.
    swift_dirs: Vec<(String, String)>,
    /// SwiftPM package dependencies (deduped by identity).
    packages: Vec<SwiftPackage>,
    /// System frameworks the app links (deduped).
    frameworks: Vec<String>,
}

/// Resolve every piece in the app's iOS dependency closure (features = `["uikit"]`) and collect its
/// Swift shim dirs + SwiftPM package dependencies.
fn resolve_ios(project: &Project, features: &[&str]) -> Result<IosPieces, String> {
    let meta = cargo_metadata(project, features)?;
    let in_closure = closure(&meta);

    let mut pieces = IosPieces::default();
    let mut seen_dirs = HashSet::new();
    let mut seen_pkgs = HashSet::new();
    for pkg in &meta.packages {
        if !in_closure.contains(&pkg.id) {
            continue;
        }
        let Some(ios) = piece_meta::<IosMeta>(pkg, "ios") else {
            continue;
        };
        let crate_dir = Path::new(&pkg.manifest_path)
            .parent()
            .unwrap_or(Path::new("."));
        let namespace = crate_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "piece".into());
        for rel in &ios.swift.0 {
            let dir = crate_dir.join(rel);
            if !dir.is_dir() {
                eprintln!("day: {} swift dir {:?} not found — skipping", pkg.id, dir);
                continue;
            }
            let abs = dir.to_string_lossy().into_owned();
            if seen_dirs.insert(abs.clone()) {
                pieces.swift_dirs.push((namespace.clone(), abs));
            }
        }
        for spkg in ios.swift_packages {
            if seen_pkgs.insert(spkg.identity()) {
                pieces.packages.push(spkg);
            }
        }
        for fw in ios.frameworks {
            if !pieces.frameworks.contains(&fw) {
                pieces.frameworks.push(fw);
            }
        }
    }
    Ok(pieces)
}

/// Generate the local `DayPieces` SwiftPM package (Package.swift + staged Swift shims) under
/// `build/day/ios/DayPieces`, from every piece's `[package.metadata.day.ios]`. The app's `.xcodeproj`
/// depends on this local package, so `day build` (ios) calls this before `xcodebuild`. Always writes
/// a VALID package (an empty target with a placeholder source when no pieces contribute), so the
/// project's local-package reference always resolves.
pub fn write_ios_pieces(project: &Project) -> Result<(), String> {
    let pieces = resolve_ios(project, &["uikit"]).unwrap_or_else(|e| {
        eprintln!("day: iOS piece discovery failed ({e}); building with framework pieces only");
        IosPieces::default()
    });

    let pkg_dir = project.root.join("build/day/ios/DayPieces");
    let sources = pkg_dir.join("Sources/DayPieces");
    // Regenerate the staged sources fresh so a removed piece never leaves a stale shim behind.
    let _ = std::fs::remove_dir_all(&sources);
    std::fs::create_dir_all(&sources).map_err(|e| e.to_string())?;

    // A placeholder keeps the target valid (≥1 source) even with no piece shims.
    std::fs::write(
        sources.join("_DayPieces.swift"),
        "// Generated by `day build`. The DayPieces local package aggregates every standalone piece's\n\
         // iOS Swift shims and SwiftPM package dependencies (docs/extending.md). Do not edit.\n\
         enum _DayPieces {}\n",
    )
    .map_err(|e| e.to_string())?;

    // Stage every piece's Swift shim files under a per-crate subdir so they can't collide.
    for (namespace, dir) in &pieces.swift_dirs {
        stage_swift_dir(Path::new(dir), &sources.join(namespace))?;
    }

    // Processed images (§18.3): generate a Media.xcassets from the project's images/ into the target
    // so SwiftPM `.process` compiles it (actool) into the package's Assets.car.
    let images = crate::resources::ResourceSet::scan(project).images;
    let has_resources = crate::resources::apple::write_media_xcassets(&sources, &images)?;

    std::fs::write(
        pkg_dir.join("Package.swift"),
        package_swift(&pieces, has_resources),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Copy every `.swift` file under `src` into `dest` (recursively), so a piece's shims join the
/// DayPieces target's sources.
fn stage_swift_dir(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            stage_swift_dir(&path, &dest.join(entry.file_name()))?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("swift") {
            std::fs::copy(&path, dest.join(entry.file_name())).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Render the generated `DayPieces/Package.swift`. When `has_resources`, the target processes the
/// generated `Media.xcassets` (§18.3) — SwiftPM runs `actool` → an optimized `Assets.car` in the
/// package's resource bundle, which `day-uikit` loads images from by name.
fn package_swift(pieces: &IosPieces, has_resources: bool) -> String {
    let deps: String = pieces
        .packages
        .iter()
        .map(|p| {
            format!(
                "        .package(url: \"{}\", {}),\n",
                p.url,
                p.requirement()
            )
        })
        .collect();
    let products: String = pieces
        .packages
        .iter()
        .flat_map(|p| {
            let id = p.identity();
            p.products.iter().map(move |prod| {
                format!("            .product(name: \"{prod}\", package: \"{id}\"),\n")
            })
        })
        .collect();
    // System frameworks link on the target (`.linkedFramework`), so a piece can declare `frameworks =
    // ["WebKit"]` instead of `dlopen`ing or hand-`#[link]`ing them; they reach the app via DayPieces.
    let linker: String = if pieces.frameworks.is_empty() {
        String::new()
    } else {
        let fws: String = pieces
            .frameworks
            .iter()
            .map(|f| format!(".linkedFramework(\"{f}\"), "))
            .collect();
        format!(", linkerSettings: [{fws}]")
    };
    // App images (§18.3) staged as a `.process`ed asset catalog next to the shims.
    let resources = if has_resources {
        ", resources: [.process(\"Media.xcassets\")]"
    } else {
        ""
    };
    format!(
        "// swift-tools-version:5.9\n\
         // Generated by `day build` from standalone pieces' [package.metadata.day.ios]. Do not edit.\n\
         import PackageDescription\n\n\
         let package = Package(\n\
         \x20   name: \"DayPieces\",\n\
         \x20   platforms: [.iOS(.v15)],\n\
         \x20   products: [.library(name: \"DayPieces\", targets: [\"DayPieces\"])],\n\
         \x20   dependencies: [\n{deps}    ],\n\
         \x20   targets: [\n\
         \x20       .target(name: \"DayPieces\", dependencies: [\n{products}        ], path: \"Sources/DayPieces\"{resources}{linker}),\n\
         \x20   ]\n\
         )\n"
    )
}
