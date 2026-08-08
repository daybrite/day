// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Standalone-piece backend discovery (docs/extending.md). External piece crates (e.g.
//! `day-piece-searchfield`) declare their per-toolkit backend contributions in `Cargo.toml` under
//! `[package.metadata.day.<toolkit>]`; the Day CLI reads them from `cargo metadata` and folds them
//! into the native build — so a piece carries BOTH its front-end (Rust) and its backend (Java /
//! Gradle deps / …) without touching the core Day crates.
//!
//! Android contract (`[package.metadata.day.android]`):
//! ```toml
//! java = ["android/java"]                 # dirs (rel. to the crate) → Gradle java srcDirs
//! res = ["android/res"]                   # dirs (rel. to the crate) → Gradle res srcDirs
//! gradle-dependencies = ["g:a:v", …]      # → the app module's dependencies { }
//! gradle-repositories = ["https://…", …]  # → extra Maven repos
//! permissions = ["android.permission.INTERNET", …]  # → <uses-permission>s merged into the manifest
//! proguard = ["android/proguard-rules.pro"]  # → R8 keep rules for classes native code reaches by name
//! manifest-components = ["android/components.xml"]  # → <receiver>/<service>/… merged into <application>
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
//!
//! HarmonyOS contract (`[package.metadata.day.ohos]`):
//! ```toml
//! ets = ["ohos/ets"]                    # dirs (rel. to the crate) of ArkTS sources
//! ```
//! For components that exist ONLY in ArkTS — the ArkUI C node API cannot construct a `Web` at all.
//! Hvigor compiles ArkTS only from inside the module, so these stage into the project itself
//! (`entry/src/main/ets/daypieces/<crate>/`, gitignored) beside a generated `DayPieces.ets` whose
//! `registerDayPieces(uiContext)` the checked-in host page calls once — so adding an ArkTS piece is
//! pure `Cargo.toml` data too. Each declared dir needs an `Index.ets` exporting a `DayPieceModule`.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::meta::Project;

/// `[package.metadata.day.permissions]` — a library declaring which PORTABLE permissions it needs
/// (docs/permissions.md). Machine-facing only: a library cannot write the user-facing reason, which
/// is why that lives in the app's Day.toml and why a contribution without one is a build error on
/// the platforms that show it.
#[derive(Debug, Default, Deserialize)]
struct PermissionsMeta {
    #[serde(default)]
    uses: Vec<String>,
}

/// The build-side contribution list handed to Gradle (serialized to day-pieces.json).
#[derive(Debug, Default, Serialize)]
pub struct AndroidPieces {
    /// The day-android framework Java shim (DayActivity, DayBridge, …), resolved from the
    /// `day-android` crate the app depends on — wherever cargo has it (workspace path, git
    /// checkout, or registry source). Without this dir in the dex, the APK installs and then
    /// crashes with ClassNotFoundException at launch; the Gradle scaffold hard-fails instead.
    #[serde(rename = "dayJavaSrcDir")]
    pub day_java_src_dir: Option<String>,
    /// The day-android framework's own R8/ProGuard keep rules (bridge classes + native methods),
    /// resolved from the day-android crate. Applied to every release build so minification never
    /// renames the JNI-reached bridge (docs/extending.md).
    #[serde(rename = "dayProguardFile")]
    pub day_proguard_file: Option<String>,
    /// Absolute Java/Kotlin source dirs to add as Gradle `java.srcDir`s.
    #[serde(rename = "javaSrcDirs")]
    pub java_src_dirs: Vec<String>,
    /// Absolute Android resource dirs to add as Gradle `res.srcDir`s — a piece can ship its own
    /// styles/drawables (e.g. a theme overlay its dialog needs) without touching the scaffold.
    #[serde(rename = "resSrcDirs")]
    pub res_src_dirs: Vec<String>,
    /// Gradle dependency coordinates (`group:artifact:version`).
    pub dependencies: Vec<String>,
    /// Extra Maven repository URLs.
    pub repositories: Vec<String>,
    /// Android `<uses-permission>` names to merge into the app manifest.
    pub permissions: Vec<String>,
    /// Absolute R8/ProGuard rule files contributed by the app and its pieces/parts — every
    /// component that hands Java classes to native code by name (JNI FindClass, `dcall_static`,
    /// reflection) ships one and declares it in `[package.metadata.day.android].proguard`. Folded
    /// into the release build's proguard configuration so those names survive minification.
    #[serde(rename = "proguardFiles")]
    pub proguard_files: Vec<String>,
    /// Absolute manifest-fragment files contributed by pieces/parts that need a `<receiver>`,
    /// `<service>`, or `<activity>` of their own — a scheduled-notification part cannot work
    /// without one (docs/notify.md). Their contents are inlined into the `<application>` of the
    /// generated overlay; the paths ride in day-pieces.json so Gradle can gate on them and so a
    /// build log shows which crate contributed what.
    #[serde(rename = "manifestComponents")]
    pub manifest_components: Vec<String>,
}

// --- `cargo metadata` JSON (only the fields we need) ---

#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub(crate) packages: Vec<Package>,
    resolve: Option<Resolve>,
}
#[derive(Deserialize)]
pub(crate) struct Package {
    id: String,
    pub(crate) name: String,
    manifest_path: String,
    #[serde(default)]
    pub(crate) metadata: Option<serde_json::Value>,
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
    #[serde(default)]
    res: StringOrVec,
    #[serde(default, rename = "gradle-dependencies")]
    gradle_dependencies: Vec<String>,
    #[serde(default, rename = "gradle-repositories")]
    gradle_repositories: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
    /// R8/ProGuard rule files (relative to the crate) — one per component that needs its Java
    /// classes kept by name under release minification.
    #[serde(default)]
    proguard: StringOrVec,
    /// Manifest fragments (relative to the crate) holding the `<receiver>`/`<service>`/`<activity>`
    /// elements the crate's own Java classes need declared. Each file holds ONLY the elements —
    /// no `<manifest>` or `<application>` wrapper, which the CLI adds — and must name its classes
    /// fully-qualified, since the overlay merges into an app whose package it cannot know.
    #[serde(default, rename = "manifest-components")]
    manifest_components: StringOrVec,
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
    /// Backend toolkit names (`appkit`, `gtk`, `qt`, `uikit`, `mdc`, `xaml`, `mock`) this piece
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
pub(crate) fn cargo_metadata(project: &Project, features: &[&str]) -> Result<Metadata, String> {
    cargo_metadata_inner(project, features, false)
}

/// `cargo metadata --all-features` — the form external-toolkit discovery needs: a toolkit crate
/// is an OPTIONAL dependency (behind the very feature its declaration names), and cargo omits
/// unactivated optional deps from `packages` under any narrower flag set (verified empirically:
/// only `--all-features` lists them). Feature-closure consumers keep the precise form above.
pub(crate) fn cargo_metadata_all_features(project: &Project) -> Result<Metadata, String> {
    cargo_metadata_inner(project, &[], true)
}

fn cargo_metadata_inner(
    project: &Project,
    features: &[&str],
    all_features: bool,
) -> Result<Metadata, String> {
    let manifest = project.root.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    // Run from the project root, like every build command: cargo discovers `.cargo/config.toml`
    // (the `day patch` table) from the CURRENT DIRECTORY, not the manifest path. Without this,
    // `day` invoked from outside the project resolves the graph without the patch table — a crate
    // that exists only in the local checkout fails resolution, and every metadata consumer
    // (feature union, piece staging) silently degrades to "no contributions".
    cmd.current_dir(&project.root)
        .args(["metadata", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(&manifest);
    if all_features {
        cmd.arg("--all-features");
    } else {
        cmd.arg("--no-default-features");
    }
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
pub(crate) fn piece_meta<T: serde::de::DeserializeOwned>(
    pkg: &Package,
    toolkit: &str,
) -> Option<T> {
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
/// by that feature set contribute) — currently `["mdc"]`, no default features.
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
        // The framework's own Java shim rides with the day-android crate (§17.1) — resolve it
        // from wherever cargo checked the crate out instead of assuming a day repo layout.
        if pkg.name == "day-android" {
            let java = Path::new(&pkg.manifest_path)
                .parent()
                .unwrap_or(Path::new("."))
                .join("java");
            if !java.is_dir() {
                return Err(format!(
                    "day-android crate at {:?} has no java/ dir — the Android Java shim is \
                     missing from this day checkout",
                    pkg.manifest_path
                ));
            }
            pieces.day_java_src_dir = Some(java.to_string_lossy().into_owned());
            // The framework's own R8 keep rules ride alongside the Java shim (optional — an older
            // day-android checkout without the file simply contributes none).
            let rules = Path::new(&pkg.manifest_path)
                .parent()
                .unwrap_or(Path::new("."))
                .join("proguard-rules.pro");
            if rules.is_file() {
                pieces.day_proguard_file = Some(rules.to_string_lossy().into_owned());
            }
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
        for rel in &android.res.0 {
            let dir = crate_dir.join(rel);
            if !dir.is_dir() {
                eprintln!("day: {} res dir {:?} not found — skipping", pkg.id, dir);
                continue;
            }
            let abs = dir.to_string_lossy().into_owned();
            if !pieces.res_src_dirs.contains(&abs) {
                pieces.res_src_dirs.push(abs);
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
        for rel in &android.proguard.0 {
            let file = crate_dir.join(rel);
            if !file.is_file() {
                eprintln!(
                    "day: {} proguard file {:?} not found — skipping",
                    pkg.id, file
                );
                continue;
            }
            let abs = file.to_string_lossy().into_owned();
            if !pieces.proguard_files.contains(&abs) {
                pieces.proguard_files.push(abs);
            }
        }
        // A missing fragment is a HARD error, unlike the skip-and-warn above: the others degrade to
        // a smaller build, but a dropped `<receiver>` yields an APK that installs, runs, and then
        // silently never delivers — the failure mode this key exists to prevent.
        for rel in &android.manifest_components.0 {
            let file = crate_dir.join(rel);
            if !file.is_file() {
                return Err(format!(
                    "{}: manifest-components file {:?} not found",
                    pkg.id, file
                ));
            }
            let abs = file.to_string_lossy().into_owned();
            if !pieces.manifest_components.contains(&abs) {
                pieces.manifest_components.push(abs);
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

/// Every `[package.metadata.day.permissions].uses` in the app's dependency closure, as
/// `(crate_name, permission)`. The app package itself participates (the closure starts at the
/// resolve root), so an app may use the same key instead of Day.toml when it has no reason to give.
pub fn contributed_permissions(project: &Project, backends: &[&str]) -> Vec<(String, String)> {
    let Ok(meta) = cargo_metadata(project, backends) else {
        return Vec::new();
    };
    let reachable = closure(&meta);
    let mut out = Vec::new();
    for pkg in meta.packages.iter().filter(|p| reachable.contains(&p.id)) {
        if let Some(m) = piece_meta::<PermissionsMeta>(pkg, "permissions") {
            for perm in m.uses {
                out.push((pkg.name.clone(), perm));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Write the resolved contributions to `build/day/android/day-pieces.json` for Gradle to read (and,
/// when pieces contribute Android permissions, a `day-pieces-manifest.xml` overlay the scaffold
/// merges). Always writes (an empty manifest when there are no pieces) so a stale file never lingers.
pub fn write_android_manifest(project: &Project) -> Result<(), String> {
    let mut pieces = resolve_android(project, &["mdc"]).unwrap_or_else(|e| {
        eprintln!("day: piece discovery failed ({e}); building with framework pieces only");
        AndroidPieces::default()
    });
    let dir = project.root.join("build/day/android");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Day.toml's [permissions] joins the pieces' raw contributions here, so BOTH reach the overlay
    // through one path. `pieces.permissions` must carry every name: the scaffold's build.gradle.kts
    // gates the overlay on that list being non-empty.
    let contributed = contributed_permissions(project, &["mdc"]);
    let declared = crate::permissions::resolve(&project.manifest, "android", &contributed)
        .map_err(|e| format!("Day.toml: {e}"))?;
    let mut entries = crate::permissions::android_entries(&declared);
    for name in &pieces.permissions {
        if !entries.iter().any(|e| &e.name == name) {
            entries.push(crate::permissions::AndroidRaw {
                name: name.clone(),
                max_sdk: None,
            });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    pieces.permissions = entries.iter().map(|e| e.name.clone()).collect();

    // day-pieces.json is written AFTER the merge so Gradle sees the full list.
    let json = serde_json::to_string_pretty(&pieces).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("day-pieces.json"), json).map_err(|e| e.to_string())?;

    // Permissions → a manifest overlay AGP merges into the app manifest (the scaffold points its
    // debug+release source-set manifests here). Remove any stale overlay when there are none.
    //
    // The FILENAME is a compatibility surface: it is baked into every scaffold `day new` has ever
    // generated, and a source set has exactly one manifest slot (debug and release are both already
    // claimed). Widen what this file contains; never move or split it, or permission merging breaks
    // silently in every checked-out app.
    let overlay = dir.join("day-pieces-manifest.xml");
    let mut components = read_manifest_components(&pieces.manifest_components)?;
    // Day.toml [[shortcuts]] rides the same overlay: an <activity> fragment the manifest
    // merger folds into the launcher activity by name (docs/deep-links.md, "Shortcuts are
    // saved deep links").
    if let Some(frag) = crate::shortcuts::android_manifest_fragment(project) {
        components.push(frag);
    }
    if entries.is_empty() && components.is_empty() {
        let _ = std::fs::remove_file(&overlay);
    } else {
        // A scaffold generated before manifest-components existed gates the overlay on the
        // permission list being non-empty, so a crate contributing ONLY components would have its
        // receivers silently dropped. Say so, with the one-line fix, rather than shipping an APK
        // that installs and never delivers.
        if entries.is_empty() {
            eprintln!(
                "day: a dependency or Day.toml [[shortcuts]] contributes Android manifest \
                 components but no permissions. \
                 If this app's platform/android/app/build.gradle.kts still reads \
                 `if (piecePermissions.isNotEmpty() && pieceManifest.exists())`, change it to \
                 `if (pieceManifest.exists())` or the components will not be merged."
            );
        }
        std::fs::write(&overlay, pieces_manifest(&entries, &components))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Everything outside `<!-- … -->`. Used for validation only — the comments are kept in the
/// generated overlay, where they explain to a reader which crate contributed what.
fn strip_xml_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            // An unterminated comment swallows the remainder, which is what a parser would do.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Read and validate each contributed manifest fragment. The fragment holds only the elements that
/// belong inside `<application>`; rejecting a wrapper here turns a confusing AGP merge failure into
/// a build error naming the file.
fn read_manifest_components(paths: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for path in paths {
        let body = std::fs::read_to_string(path)
            .map_err(|e| format!("manifest-components {path}: {e}"))?;
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return Err(format!("manifest-components {path}: file is empty"));
        }
        // Look for a wrapper only OUTSIDE comments: a fragment's header comment routinely
        // mentions `<application>` while explaining that it must not contain one, and matching
        // that would reject a correct file (it rejected this crate's own reference fragment).
        let code = strip_xml_comments(trimmed);
        if code.contains("<manifest") || code.contains("<application") {
            return Err(format!(
                "manifest-components {path}: contains a <manifest>/<application> wrapper — the \
                 file must hold ONLY the elements that go inside <application> (the CLI adds the \
                 wrapper)"
            ));
        }
        out.push(trimmed.to_string());
    }
    Ok(out)
}

/// The overlay: the `<uses-permission>`s, plus any `<application>` components pieces/parts declared
/// — merged into the app manifest by AGP's manifest merger (which also dedups against any the app
/// already declares).
fn pieces_manifest(
    permissions: &[crate::permissions::AndroidRaw],
    components: &[String],
) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!-- Generated by `day build` from Day.toml [permissions] and \
         [package.metadata.day.*] contributions. Do not edit. -->\n\
         <manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n",
    );
    for perm in permissions {
        match perm.max_sdk {
            // The cap matters: an uncapped legacy storage permission makes stores flag the app for
            // requesting broad access that API 33+ replaced with the granular READ_MEDIA_* set.
            Some(max) => s.push_str(&format!(
                "    <uses-permission android:name=\"{}\" android:maxSdkVersion=\"{max}\" />\n",
                perm.name
            )),
            None => s.push_str(&format!(
                "    <uses-permission android:name=\"{}\" />\n",
                perm.name
            )),
        }
    }
    if !components.is_empty() {
        s.push_str("    <application>\n");
        for frag in components {
            for line in frag.lines() {
                if line.trim().is_empty() {
                    s.push('\n');
                } else {
                    s.push_str("        ");
                    s.push_str(line.trim_end());
                    s.push('\n');
                }
            }
        }
        s.push_str("    </application>\n");
    }
    s.push_str("</manifest>\n");
    s
}

// ===========================================================================
// iOS — a piece's Swift shims + SwiftPM package dependencies
// ===========================================================================

/// A SwiftPM package dependency declared by a piece (`[package.metadata.day.ios/macos]
/// .swift-packages`): **remote** (`url` + a version requirement) or **local** (`path`, relative to
/// the declaring crate; absolutized at discovery). Local packages are additionally scanned for
/// exportable public SwiftUI views (docs/swiftui.md) — `day build` generates their provider glue
/// and day-build generates the app's typed `crate::swiftui::*` bindings from the same scan.
#[derive(Debug, Clone, Deserialize)]
struct SwiftPackage {
    #[serde(default)]
    url: Option<String>,
    /// A local package directory, relative to the declaring crate.
    #[serde(default)]
    path: Option<String>,
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
    /// SwiftPM derives a package's identity from the last path component of its URL (sans `.git`)
    /// — or of its directory, for a local package.
    fn identity(&self) -> String {
        let base = self.path.as_deref().or(self.url.as_deref()).unwrap_or("");
        base.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(base)
            .trim_end_matches(".git")
            .to_string()
    }
    /// The library products to link — the declared list, or the package identity by convention
    /// (the scaffold layout `swift package init` produces).
    fn product_names(&self) -> Vec<String> {
        if self.products.is_empty() {
            vec![self.identity()]
        } else {
            self.products.clone()
        }
    }
    /// The `.package(…)` dependency clause.
    fn dependency_clause(&self) -> String {
        match &self.path {
            Some(p) => format!(".package(path: \"{p}\")"),
            None => format!(
                ".package(url: \"{}\", {})",
                self.url.as_deref().unwrap_or(""),
                self.requirement()
            ),
        }
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

/// The `[package.metadata.day.ios]` / `[package.metadata.day.macos]` table, as declared by a piece
/// crate — or by the app itself (the app crate is in its own dependency closure, which is how an
/// app contributes its own Swift sources and packages).
#[derive(Deserialize, Default)]
struct AppleMeta {
    #[serde(default)]
    swift: StringOrVec,
    #[serde(default, rename = "swift-packages")]
    swift_packages: Vec<SwiftPackage>,
    /// System frameworks to link (e.g. `["WebKit"]`) — so a piece needn't `dlopen` or hand-`#[link]`.
    #[serde(default)]
    frameworks: Vec<String>,
    /// The minimum platform version this contribution needs (e.g. `"16.0"`). The generated
    /// package's floor — and the leg's deployment target — is the max across contributions.
    #[serde(default)]
    platform: Option<String>,
}

/// The resolved Apple-leg contributions across all pieces in the app's dependency closure.
#[derive(Default)]
struct ApplePieces {
    /// `(namespace, absolute dir)` Swift source dirs to compile — the namespace (the piece's crate
    /// name) subfolders the staged shims so two pieces' files can't collide.
    swift_dirs: Vec<(String, String)>,
    /// SwiftPM package dependencies (deduped by identity).
    packages: Vec<SwiftPackage>,
    /// System frameworks the app links (deduped).
    frameworks: Vec<String>,
    /// The max `platform` floor across contributions (`None` = every contribution is fine with
    /// the leg's default).
    platform: Option<String>,
    /// Local packages to scan for exportable SwiftUI views: `(identity, absolute dir)`.
    scan_roots: Vec<(String, String)>,
    /// Whether `day-piece-swiftui` is in the closure — the generated provider glue subclasses the
    /// base class its shim stages, so without it the view export is skipped (with a warning).
    has_swiftui_piece: bool,
}

/// The higher of two dotted platform versions ("16.0" vs "9.4"), numerically per component.
fn max_platform(a: &str, b: &str) -> String {
    fn key(v: &str) -> Vec<u32> {
        v.split('.')
            .map(|c| c.trim().parse().unwrap_or(0))
            .collect()
    }
    if key(b) > key(a) {
        b.to_string()
    } else {
        a.to_string()
    }
}

/// Resolve every piece in the app's dependency closure for one Apple leg — `("ios", ["uikit"])` or
/// `("macos", ["appkit"])` — and collect its Swift dirs, SwiftPM packages, frameworks, and floor.
fn resolve_apple(
    project: &Project,
    features: &[&str],
    platform_key: &str,
) -> Result<ApplePieces, String> {
    let meta = cargo_metadata(project, features)?;
    let in_closure = closure(&meta);

    let mut pieces = ApplePieces::default();
    let mut seen_dirs = HashSet::new();
    let mut seen_pkgs = HashSet::new();
    for pkg in &meta.packages {
        if !in_closure.contains(&pkg.id) {
            continue;
        }
        if pkg.name == "day-piece-swiftui" {
            pieces.has_swiftui_piece = true;
        }
        let Some(apple) = piece_meta::<AppleMeta>(pkg, platform_key) else {
            continue;
        };
        let crate_dir = Path::new(&pkg.manifest_path)
            .parent()
            .unwrap_or(Path::new("."));
        let namespace = crate_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "piece".into());
        for rel in &apple.swift.0 {
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
        for mut spkg in apple.swift_packages {
            if let Some(rel) = &spkg.path {
                // Local package: absolutize against the declaring crate, validate, and record it
                // as a scan root for the SwiftUI view export.
                let dir = crate_dir.join(rel);
                if !dir.join("Package.swift").is_file() {
                    eprintln!(
                        "day: {} swift package {:?} not found (no Package.swift) — skipping",
                        pkg.id, dir
                    );
                    continue;
                }
                let abs = dir.to_string_lossy().into_owned();
                spkg.path = Some(abs.clone());
                if seen_pkgs.insert(format!("path:{abs}")) {
                    pieces.scan_roots.push((spkg.identity(), abs));
                    pieces.packages.push(spkg);
                }
            } else if spkg.url.is_some() {
                if seen_pkgs.insert(spkg.identity()) {
                    pieces.packages.push(spkg);
                }
            } else {
                eprintln!(
                    "day: {} has a swift-packages entry with neither `url` nor `path` — skipping",
                    pkg.id
                );
            }
        }
        for fw in apple.frameworks {
            if !pieces.frameworks.contains(&fw) {
                pieces.frameworks.push(fw);
            }
        }
        if let Some(p) = apple.platform {
            pieces.platform = Some(match pieces.platform.take() {
                Some(cur) => max_platform(&cur, &p),
                None => p,
            });
        }
    }
    Ok(pieces)
}

/// Generate the local `DayPieces` SwiftPM package (Package.swift + staged Swift shims + generated
/// SwiftUI provider glue) under `build/day/ios/DayPieces`, from every piece's
/// `[package.metadata.day.ios]`. The app's `.xcodeproj` depends on this local package, so `day
/// build` (ios) calls this before `xcodebuild`. Always writes a VALID package (an empty target
/// with a placeholder source when no pieces contribute), so the project's local-package reference
/// always resolves.
///
/// Returns the deployment-target override to pass to xcodebuild: `Some(floor)` when a
/// contribution's `platform` exceeds the scaffold pbxproj's checked-in value (a command-line
/// setting raises the app AND the SwiftPM package targets; the pbxproj itself is never edited —
/// §15.2 "aggregation never mutates the scaffolds").
pub fn write_ios_pieces(project: &Project) -> Result<Option<String>, String> {
    let pieces = resolve_apple(project, &["uikit"], "ios").unwrap_or_else(|e| {
        eprintln!("day: iOS piece discovery failed ({e}); building with framework pieces only");
        ApplePieces::default()
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

    // Exported SwiftUI views (docs/swiftui.md): scan local packages, generate provider glue.
    if let Some(glue) = render_view_glue(&pieces) {
        std::fs::write(sources.join("_DayViews.swift"), glue).map_err(|e| e.to_string())?;
    }

    // Processed images (§18.3): generate a Media.xcassets from the project's images/ into the target
    // so SwiftPM `.process` compiles it (actool) into the package's Assets.car.
    // uikit renders the imageset SVGs (`preserves-vector-representation`), so the only rasters
    // this catalog picks up are the fallbacks for art that has no SVG form (docs/vectors.md).
    let images = crate::resources::ResourceSet::scan(project, "uikit").images;
    let vectors: Vec<(String, std::path::PathBuf)> =
        std::fs::read_dir(crate::resources::vector_svg_dir(project))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("svg"))
            .filter_map(|p| {
                let stem = p.file_stem()?.to_str()?.to_string();
                Some((stem, p))
            })
            .collect();
    let has_resources = crate::resources::apple::write_media_xcassets(&sources, &images, &vectors)?;

    // Bundled fonts (§18.4): copied VERBATIM into the target so SwiftPM `.copy`s the directory
    // into the DayPieces bundle (`DayPieces_DayPieces.bundle/fonts/…` — fonts must not be
    // `.process`ed). day-uikit registers every file in there with CoreText at launch, and
    // build_ios lists the same paths in the app Info.plist's UIAppFonts.
    let fonts = crate::resources::scan_fonts(project)?;
    if !fonts.is_empty() {
        let fdir = sources.join("fonts");
        std::fs::create_dir_all(&fdir).map_err(|e| e.to_string())?;
        for f in &fonts {
            let name = f.path.file_name().ok_or("font file name")?;
            std::fs::copy(&f.path, fdir.join(name)).map_err(|e| e.to_string())?;
        }
    }

    // The package floor: the shipped default unless a contribution needs more. The package floor
    // may never exceed the effective app target (xcodebuild errors), which is what the returned
    // override guarantees.
    let floor = max_platform("15.0", pieces.platform.as_deref().unwrap_or("15.0"));
    std::fs::write(
        pkg_dir.join("Package.swift"),
        package_swift(
            &pieces,
            "ios",
            &format!(".iOS(\"{floor}\")"),
            false,
            has_resources,
            !fonts.is_empty(),
        ),
    )
    .map_err(|e| e.to_string())?;

    // Override only when the contributions exceed the scaffold's checked-in target — and never
    // lower a value the user raised by hand.
    let pbx = pbxproj_ios_target(project).unwrap_or_else(|| "15.0".into());
    Ok((max_platform(&pbx, &floor) != pbx).then_some(floor))
}

/// The scaffold's checked-in `IPHONEOS_DEPLOYMENT_TARGET` (the max across every place it can
/// be set), parsed tolerantly — `None` when nothing declares it. Since the xcconfig split
/// (§17.4) the setting normally lives in `DayApp.xcconfig`; the pbxproj is still read for
/// pre-split projects and hand-added per-config overrides. The same line parser serves both
/// formats (the xcconfig just has no trailing `;`, which the parser already trims).
fn pbxproj_ios_target(project: &Project) -> Option<String> {
    [
        "platform/ios/DayApp.xcodeproj/project.pbxproj",
        "platform/ios/DayApp.xcconfig",
    ]
    .iter()
    .filter_map(|rel| std::fs::read_to_string(project.root.join(rel)).ok())
    .filter_map(|text| ios_target_from_pbxproj(&text))
    .reduce(|a, b| max_platform(&a, &b))
}

fn ios_target_from_pbxproj(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| {
            let (key, value) = l.trim().split_once('=')?;
            (key.trim() == "IPHONEOS_DEPLOYMENT_TARGET")
                .then(|| value.trim().trim_end_matches(';').trim().to_string())
        })
        .reduce(|a, b| max_platform(&a, &b))
}

/// Render the generated SwiftUI provider glue for every scan root (docs/swiftui.md), or `None`
/// when there is nothing to export. Skipped views are reported so a missing binding is never a
/// silent mystery; a scan failure degrades to a warning (the app still builds without the export).
fn render_view_glue(pieces: &ApplePieces) -> Option<String> {
    if pieces.scan_roots.is_empty() {
        return None;
    }
    if !pieces.has_swiftui_piece {
        eprintln!(
            "day: local Swift packages are declared but day-piece-swiftui is not a dependency — \
             skipping the SwiftUI view export (docs/swiftui.md)"
        );
        return None;
    }
    let mut scans = Vec::new();
    for (name, dir) in &pieces.scan_roots {
        match day_build::swiftui::scan_package(Path::new(dir)) {
            Ok(scan) => {
                for (view, reason) in &scan.skipped {
                    eprintln!("day: swiftui: {view} not exported — {reason}");
                }
                scans.push((name.clone(), scan));
            }
            Err(e) => eprintln!("day: swiftui view scan failed for {name}: {e}"),
        }
    }
    scans
        .iter()
        .any(|(_, s)| !s.views.is_empty())
        .then(|| day_build::swiftui::render_glue(&scans))
}

/// Copy every `.swift` file under `src` into `dest` (recursively), so a piece's shims join the
/// DayPieces target's sources.
/// The `[package.metadata.day.ohos]` table, as declared by a piece crate.
#[derive(Deserialize, Default)]
struct OhosMeta {
    /// Dirs (relative to the crate) of ArkTS sources staged into the app's hvigor project. Each
    /// dir must carry an `Index.ets` exporting `dayPiece: DayPieceModule` (docs/extending.md).
    #[serde(default)]
    ets: StringOrVec,
}

/// The resolved HarmonyOS contributions across all pieces in the app's dependency closure.
#[derive(Default)]
struct OhosPieces {
    /// `(namespace, absolute dir)` ArkTS dirs to stage — the namespace (the piece's crate name)
    /// subfolders them so two pieces' files can't collide, as on iOS.
    ets_dirs: Vec<(String, String)>,
}

/// Resolve every piece in the app's HarmonyOS dependency closure (features = `["arkui"]`) and
/// collect its ArkTS dirs.
fn resolve_ohos(project: &Project, features: &[&str]) -> Result<OhosPieces, String> {
    let meta = cargo_metadata(project, features)?;
    let in_closure = closure(&meta);

    let mut pieces = OhosPieces::default();
    let mut seen = HashSet::new();
    for pkg in &meta.packages {
        if !in_closure.contains(&pkg.id) {
            continue;
        }
        let Some(ohos) = piece_meta::<OhosMeta>(pkg, "ohos") else {
            continue;
        };
        let crate_dir = Path::new(&pkg.manifest_path)
            .parent()
            .unwrap_or(Path::new("."));
        let namespace = crate_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "piece".into());
        for rel in &ohos.ets.0 {
            let dir = crate_dir.join(rel);
            if !dir.is_dir() {
                eprintln!("day: {} ets dir {:?} not found — skipping", pkg.id, dir);
                continue;
            }
            if !dir.join("Index.ets").is_file() {
                eprintln!(
                    "day: {} ets dir {:?} has no Index.ets (the `dayPiece` entry module) — skipping",
                    pkg.id, dir
                );
                continue;
            }
            let abs = dir.to_string_lossy().into_owned();
            if seen.insert(abs.clone()) {
                pieces.ets_dirs.push((namespace.clone(), abs));
            }
        }
    }
    pieces.ets_dirs.sort();
    Ok(pieces)
}

/// Stage every piece's ArkTS into the hvigor project's `entry/src/main/ets/daypieces/` and generate
/// the two files the host page leans on: `DayPiece.ets` (the `DayPieceModule` interface both sides
/// implement) and `DayPieces.ets` (the aggregator whose `registerDayPieces(uiContext)` hands the
/// native shim one factory + command sink + disposer for ALL pieces). Hvigor compiles ArkTS only
/// from inside the module, so unlike the android/iOS legs these land in the project — the scaffold
/// gitignores the directory. Always writes both generated files, even with no contributing piece,
/// because the host page imports them unconditionally.
pub fn write_ohos_pieces(project: &Project, harmony: &Path) -> Result<(), String> {
    let pieces = resolve_ohos(project, &["arkui"]).unwrap_or_else(|e| {
        eprintln!(
            "day: HarmonyOS piece discovery failed ({e}); building with framework pieces only"
        );
        OhosPieces::default()
    });

    let dir = harmony.join("entry/src/main/ets/daypieces");
    // Regenerate fresh so a removed piece never leaves a stale module the aggregator won't import
    // but hvigor would still compile.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    std::fs::write(dir.join("DayPiece.ets"), DAY_PIECE_ETS).map_err(|e| e.to_string())?;
    for (namespace, src) in &pieces.ets_dirs {
        stage_ets_dir(Path::new(src), &dir.join(namespace))?;
    }
    std::fs::write(dir.join("DayPieces.ets"), day_pieces_ets(&pieces))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The generated `DayPieceModule` contract — the seam between a piece's ArkTS and the aggregator.
/// Written verbatim every build so the two generated files always agree.
const DAY_PIECE_ETS: &str = r#"// Generated by `day build`. The contract between a standalone piece's ArkTS and the generated
// aggregator (docs/extending.md). Do not edit.
import { FrameNode, UIContext } from '@kit.ArkUI';

export interface DayPieceModule {
  // The piece kind this module renders, matching the Rust `KIND` (e.g. 'day.piece.webview').
  kind: string;
  // Build the component and return its FrameNode; undefined declines the node (Day then renders
  // its placeholder leaf). `props` is whatever the piece's Rust renderer encoded.
  make: (ui: UIContext, id: number, props: string) => FrameNode | undefined;
  // A command from the piece's Rust renderer. `cmd`/`arg` are the piece's own vocabulary.
  update: (id: number, cmd: string, arg: string) => void;
  // Release everything held for `id` — Day disposed the node.
  dispose: (id: number) => void;
}
"#;

/// Render the aggregator for the resolved pieces.
fn day_pieces_ets(pieces: &OhosPieces) -> String {
    let mut imports = String::new();
    let mut entries = String::new();
    for (i, (namespace, _)) in pieces.ets_dirs.iter().enumerate() {
        imports.push_str(&format!(
            "import {{ dayPiece as dayPiece{i} }} from './{namespace}/Index';\n"
        ));
        entries.push_str(&format!("  dayPiece{i},\n"));
    }
    format!(
        r#"// Generated by `day build`. Registers every standalone piece's ArkTS component with the native
// shim (docs/extending.md): one factory, one command sink, one disposer for all of them. Do not edit.
import nativeEntry from 'libentry.so';
import {{ FrameNode, UIContext }} from '@kit.ArkUI';
import {{ DayPieceModule }} from './DayPiece';
{imports}
const dayPieces: DayPieceModule[] = [
{entries}];

// Which module owns a live node, so commands and disposal reach the right piece.
const dayPieceOwners: Map<number, DayPieceModule> = new Map();

// Call once, before `start()`: a piece node can be realized during the first tree build.
export function registerDayPieces(ui: UIContext): void {{
  nativeEntry.registerPiece(
    (kind: string, id: number, props: string): FrameNode | undefined => {{
      for (const m of dayPieces) {{
        if (m.kind === kind) {{
          const node: FrameNode | undefined = m.make(ui, id, props);
          if (node !== undefined) {{
            dayPieceOwners.set(id, m);
          }}
          return node;
        }}
      }}
      return undefined;
    }},
    (id: number, cmd: string, arg: string): void => {{
      dayPieceOwners.get(id)?.update(id, cmd, arg);
    }},
    (id: number): void => {{
      const m: DayPieceModule | undefined = dayPieceOwners.get(id);
      if (m !== undefined) {{
        dayPieceOwners.delete(id);
        m.dispose(id);
      }}
    }}
  );
}}
"#
    )
}

/// Copy a piece's ArkTS sources (`.ets`) into the project, recursively — the HarmonyOS counterpart
/// of [`stage_swift_dir`].
fn stage_ets_dir(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            stage_ets_dir(&path, &dest.join(entry.file_name()))?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("ets") {
            std::fs::copy(&path, dest.join(entry.file_name())).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

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

/// Render the generated `DayPieces/Package.swift` for one Apple leg. When `has_resources`, the
/// target processes the generated `Media.xcassets` (§18.3) — SwiftPM runs `actool` → an optimized
/// `Assets.car` in the package's resource bundle, which `day-uikit` loads images from by name.
/// When `has_fonts`, the staged `fonts/` directory is `.copy`d verbatim into the same bundle
/// (§18.4). The macOS leg passes `static_product` (its archive is linked into the cargo binary)
/// and neither resource kind (`pack/macos.rs` ships those via `Contents/Resources`).
fn package_swift(
    pieces: &ApplePieces,
    meta_key: &str,
    platform_clause: &str,
    static_product: bool,
    has_resources: bool,
    has_fonts: bool,
) -> String {
    let deps: String = pieces
        .packages
        .iter()
        .map(|p| format!("        {},\n", p.dependency_clause()))
        .collect();
    let products: String = pieces
        .packages
        .iter()
        .flat_map(|p| {
            let id = p.identity();
            p.product_names().into_iter().map(move |prod| {
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
    // App images (§18.3) staged as a `.process`ed asset catalog next to the shims; app fonts
    // (§18.4) as a `.copy`d directory (font files must reach the bundle byte-identical).
    let mut entries: Vec<&str> = Vec::new();
    if has_resources {
        entries.push(".process(\"Media.xcassets\")");
    }
    if has_fonts {
        entries.push(".copy(\"fonts\")");
    }
    let resources = if entries.is_empty() {
        String::new()
    } else {
        format!(", resources: [{}]", entries.join(", "))
    };
    let product_type = if static_product {
        "type: .static, "
    } else {
        ""
    };
    format!(
        "// swift-tools-version:5.9\n\
         // Generated by `day build` from standalone pieces' [package.metadata.day.{meta_key}]. Do not edit.\n\
         import PackageDescription\n\n\
         let package = Package(\n\
         \x20   name: \"DayPieces\",\n\
         \x20   platforms: [{platform_clause}],\n\
         \x20   products: [.library(name: \"DayPieces\", {product_type}targets: [\"DayPieces\"])],\n\
         \x20   dependencies: [\n{deps}    ],\n\
         \x20   targets: [\n\
         \x20       .target(name: \"DayPieces\", dependencies: [\n{products}        ], path: \"Sources/DayPieces\"{resources}{linker}),\n\
         \x20   ]\n\
         )\n"
    )
}

// ===========================================================================
// macOS — the appkit leg's Swift contributions (docs/swiftui.md)
// ===========================================================================

/// The macOS Swift contributions `day build -p macos-appkit` folds into the cargo binary.
pub struct MacosSwift {
    /// The generated SwiftPM package to `swift build` and statically link.
    pub package: std::path::PathBuf,
    /// System frameworks to link alongside it (from `[package.metadata.day.macos].frameworks`).
    pub frameworks: Vec<String>,
    /// The package's platform floor — also the binary's `MACOSX_DEPLOYMENT_TARGET`, so the cargo
    /// link and the Swift objects agree on the minimum OS.
    pub platform: String,
}

/// Generate the local `DayPieces` SwiftPM package under `build/day/macos/DayPieces` from every
/// piece's `[package.metadata.day.macos]` — the macOS analog of [`write_ios_pieces`], minus the
/// resource legs (`pack/macos.rs` owns those). Returns `None` when nothing contributes Swift: the
/// package dir is removed and the cargo build stays byte-identical to today's, with no Swift
/// toolchain requirement (the zero-cost path).
///
/// Unlike the iOS leg this stages **only files whose bytes changed** (and prunes the rest): the
/// package is rebuilt by `swift build` on every `day build`, and churned mtimes would make that
/// incremental build recompile from scratch each time (§17.5's touch-only-when-changed rule).
/// `keep_empty`: the cargo-driven build passes `false` — no Swift contributions means no
/// package and no `swift build` prepass at all. The Xcode host project (platform/macos/)
/// passes `true`: its pbxproj references the package unconditionally, so an empty one must
/// still exist for xcodebuild to resolve.
pub fn write_macos_pieces(
    project: &Project,
    keep_empty: bool,
) -> Result<Option<MacosSwift>, String> {
    let pieces = resolve_apple(project, &["appkit"], "macos").unwrap_or_else(|e| {
        eprintln!("day: macOS piece discovery failed ({e}); building without Swift contributions");
        ApplePieces::default()
    });

    let pkg_dir = project.root.join("build/day/macos/DayPieces");
    if pieces.swift_dirs.is_empty() && pieces.packages.is_empty() && !keep_empty {
        let _ = std::fs::remove_dir_all(&pkg_dir);
        return Ok(None);
    }

    let sources = pkg_dir.join("Sources/DayPieces");
    std::fs::create_dir_all(&sources).map_err(|e| e.to_string())?;
    let mut expected: Vec<std::path::PathBuf> = Vec::new();

    // A placeholder keeps the target valid (≥1 source) even with package-only contributions.
    let placeholder = sources.join("_DayPieces.swift");
    write_if_changed(
        &placeholder,
        "// Generated by `day build`. The DayPieces local package aggregates every standalone piece's\n\
         // macOS Swift shims and SwiftPM package dependencies (docs/extending.md). Do not edit.\n\
         enum _DayPieces {}\n",
    )?;
    expected.push(placeholder);

    for (namespace, dir) in &pieces.swift_dirs {
        sync_swift_dir(Path::new(dir), &sources.join(namespace), &mut expected)?;
    }

    if let Some(glue) = render_view_glue(&pieces) {
        let path = sources.join("_DayViews.swift");
        write_if_changed(&path, &glue)?;
        expected.push(path);
    }

    prune_except(&sources, &expected.into_iter().collect());

    // SwiftUI needs a meaningful baseline; 13.0 is the floor of the APIs the docs promise
    // (Grid, NavigationSplitView-era layout) and of every Mac Day supports.
    let platform = max_platform("13.0", pieces.platform.as_deref().unwrap_or("13.0"));
    write_if_changed(
        &pkg_dir.join("Package.swift"),
        &package_swift(
            &pieces,
            "macos",
            &format!(".macOS(\"{platform}\")"),
            true,
            false,
            false,
        ),
    )?;

    Ok(Some(MacosSwift {
        package: pkg_dir,
        frameworks: pieces.frameworks,
        platform,
    }))
}

/// Write `content` only when the file's bytes differ — generated trees must not churn mtimes, or
/// the incremental Swift build behind them recompiles on every `day build`.
fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|cur| cur == content.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))
}

/// Copy every `.swift` file under `src` into `dest` (recursively) via [`write_if_changed`],
/// recording each destination in `expected` — the mtime-stable counterpart of [`stage_swift_dir`].
fn sync_swift_dir(
    src: &Path,
    dest: &Path,
    expected: &mut Vec<std::path::PathBuf>,
) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sync_swift_dir(&path, &dest.join(entry.file_name()), expected)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("swift") {
            let content =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let target = dest.join(entry.file_name());
            write_if_changed(&target, &content)?;
            expected.push(target);
        }
    }
    Ok(())
}

/// Remove every file under `root` not in `expected` (and any directory left empty), so a removed
/// piece never leaves a stale shim behind — the pruning half of the mtime-stable staging.
fn prune_except(root: &Path, expected: &HashSet<std::path::PathBuf>) {
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune_except(&path, expected);
            if std::fs::read_dir(&path).is_ok_and(|mut d| d.next().is_none()) {
                let _ = std::fs::remove_dir(&path);
            }
        } else if !expected.contains(&path) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::AndroidRaw;

    fn raw(name: &str) -> AndroidRaw {
        AndroidRaw {
            name: name.to_string(),
            max_sdk: None,
        }
    }

    #[test]
    fn overlay_without_components_is_unchanged() {
        // The pre-existing shape: permissions only, no <application> element at all. Guards the
        // compatibility surface — every checked-out app's scaffold already merges this file.
        let xml = pieces_manifest(&[raw("android.permission.INTERNET")], &[]);
        assert!(xml.contains("<uses-permission android:name=\"android.permission.INTERNET\" />"));
        assert!(!xml.contains("<application"));
        assert!(xml.trim_end().ends_with("</manifest>"));
    }

    #[test]
    fn components_are_wrapped_in_application_and_indented() {
        let frag = "<receiver android:name=\"dev.daybrite.day.notify.DayNotifyAlarmReceiver\"\n    android:exported=\"false\" />";
        let xml = pieces_manifest(
            &[raw("android.permission.RECEIVE_BOOT_COMPLETED")],
            &[frag.to_string()],
        );
        assert!(xml.contains("    <application>\n"));
        assert!(xml.contains("    </application>\n"));
        assert!(
            xml.contains(
                "        <receiver android:name=\"dev.daybrite.day.notify.DayNotifyAlarmReceiver\""
            ),
            "fragment should be indented inside <application>:\n{xml}"
        );
        // The permission still rides in the same file, above the application block.
        let perm_at = xml.find("<uses-permission").expect("permission present");
        let app_at = xml.find("<application").expect("application present");
        assert!(perm_at < app_at);
    }

    #[test]
    fn components_alone_still_produce_an_overlay() {
        let xml = pieces_manifest(&[], &["<service android:name=\"a.B\" />".to_string()]);
        assert!(xml.contains("<service android:name=\"a.B\" />"));
        assert!(!xml.contains("<uses-permission"));
    }

    #[test]
    fn wrapper_in_a_fragment_is_rejected() {
        let dir = std::env::temp_dir().join("day-pieces-frag-wrapper");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("components.xml");
        std::fs::write(
            &path,
            "<manifest><application><receiver android:name=\"a.B\" /></application></manifest>",
        )
        .unwrap();
        let err = read_manifest_components(&[path.to_string_lossy().into_owned()])
            .expect_err("a wrapped fragment must be rejected");
        assert!(err.contains("wrapper"), "unhelpful error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The wrapper check must not fire on a header comment. This is not hypothetical: this crate's
    /// own reference fragment explains "only the elements that go inside <application>", and the
    /// naive substring check rejected it on the first real Android build.
    #[test]
    fn wrapper_named_only_inside_a_comment_is_accepted() {
        let dir = std::env::temp_dir().join("day-pieces-frag-comment");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("components.xml");
        std::fs::write(
            &path,
            "<!-- Only the elements that go inside <application>; no <manifest> wrapper. -->\n\
             <receiver android:name=\"a.B\" android:exported=\"false\" />",
        )
        .unwrap();
        let out = read_manifest_components(&[path.to_string_lossy().into_owned()])
            .expect("a comment mentioning the wrapper must not be rejected");
        assert!(out[0].contains("<receiver"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unterminated_comment_does_not_hide_a_real_wrapper() {
        // A truncated comment must not become a way to smuggle an <application> past the check.
        assert!(!strip_xml_comments("<!-- oops <application>").contains("<application"));
        assert!(strip_xml_comments("<!-- c --><application>").contains("<application"));
    }

    #[test]
    fn empty_fragment_is_rejected() {
        let dir = std::env::temp_dir().join("day-pieces-frag-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("components.xml");
        std::fs::write(&path, "   \n\t\n").unwrap();
        let err = read_manifest_components(&[path.to_string_lossy().into_owned()])
            .expect_err("an empty fragment must be rejected");
        assert!(err.contains("empty"), "unhelpful error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn android_meta_parses_the_new_key() {
        let meta: AndroidMeta = toml::from_str(
            "java = \"android/java\"\nmanifest-components = [\"android/components.xml\"]\n",
        )
        .expect("parses");
        assert_eq!(meta.java.0, vec!["android/java".to_string()]);
        assert_eq!(
            meta.manifest_components.0,
            vec!["android/components.xml".to_string()]
        );
    }

    #[test]
    fn android_meta_without_the_key_still_parses() {
        // Every existing part's manifest must keep parsing — the field is additive.
        let meta: AndroidMeta =
            toml::from_str("java = [\"android/java\"]\npermissions = []\n").expect("parses");
        assert!(meta.manifest_components.0.is_empty());
    }

    #[test]
    fn max_platform_compares_numerically() {
        assert_eq!(max_platform("15.0", "16.0"), "16.0");
        assert_eq!(max_platform("16.0", "15.4"), "16.0");
        // Numeric, not lexicographic: "9.9" < "10.0".
        assert_eq!(max_platform("9.9", "10.0"), "10.0");
        assert_eq!(max_platform("13.0", "13.0"), "13.0");
        assert_eq!(max_platform("13.0.1", "13.0"), "13.0.1");
    }

    #[test]
    fn apple_meta_parses_with_and_without_the_new_keys() {
        // Every existing piece's manifest must keep parsing — the fields are additive.
        let old: AppleMeta = toml::from_str(
            "swift = [\"ios/swift\"]\n\
             swift-packages = [{ url = \"https://github.com/airbnb/lottie-ios\", from = \"4.5.0\", products = [\"Lottie\"] }]\n",
        )
        .expect("parses");
        assert!(old.platform.is_none());
        assert_eq!(old.swift_packages[0].identity(), "lottie-ios");
        assert_eq!(old.swift_packages[0].product_names(), vec!["Lottie"]);

        let new: AppleMeta =
            toml::from_str("platform = \"16.0\"\nswift-packages = [{ path = \"swiftui\" }]\n")
                .expect("parses");
        assert_eq!(new.platform.as_deref(), Some("16.0"));
        let pkg = &new.swift_packages[0];
        assert_eq!(pkg.identity(), "swiftui");
        // A local package's products default to its identity (the `swift package init` layout).
        assert_eq!(pkg.product_names(), vec!["swiftui"]);
        assert_eq!(pkg.dependency_clause(), ".package(path: \"swiftui\")");
    }

    #[test]
    fn ios_target_parses_from_the_scaffold_pbxproj() {
        // Since the xcconfig split (§17.4) the scaffold declares the floor in
        // DayApp.xcconfig — the floor override maxes against that value — and the pbxproj
        // must NOT redeclare it (a buildSettings value would override the xcconfig, leaving
        // the committed setting dead).
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/app/platform/ios");
        let pbx = std::fs::read_to_string(format!("{root}/DayApp.xcodeproj/project.pbxproj"))
            .expect("template pbxproj");
        assert_eq!(ios_target_from_pbxproj(&pbx), None);
        let xcc =
            std::fs::read_to_string(format!("{root}/DayApp.xcconfig")).expect("template xcconfig");
        assert_eq!(ios_target_from_pbxproj(&xcc).as_deref(), Some("15.0"));
        // Tolerant of spacing, takes the max across configurations.
        let raw = "  IPHONEOS_DEPLOYMENT_TARGET = 15.0;\n\tIPHONEOS_DEPLOYMENT_TARGET=16.0 ;\n";
        assert_eq!(ios_target_from_pbxproj(raw).as_deref(), Some("16.0"));
        assert_eq!(ios_target_from_pbxproj("nothing here"), None);
    }

    #[test]
    fn macos_package_swift_is_static_with_local_deps() {
        let pieces = ApplePieces {
            packages: vec![SwiftPackage {
                url: None,
                path: Some("/abs/swiftui".into()),
                from: None,
                exact: None,
                branch: None,
                revision: None,
                products: vec![],
            }],
            frameworks: vec!["UserNotifications".into()],
            ..Default::default()
        };
        let text = package_swift(&pieces, "macos", ".macOS(\"13.0\")", true, false, false);
        assert!(text.contains("platforms: [.macOS(\"13.0\")]"));
        assert!(
            text.contains(".library(name: \"DayPieces\", type: .static, targets: [\"DayPieces\"])")
        );
        assert!(text.contains(".package(path: \"/abs/swiftui\"),"));
        assert!(text.contains(".product(name: \"swiftui\", package: \"swiftui\"),"));
        assert!(text.contains(".linkedFramework(\"UserNotifications\")"));
        assert!(text.contains("[package.metadata.day.macos]"));
    }

    #[test]
    fn ios_package_swift_keeps_its_shipped_shape() {
        // The iOS output with no new metadata must stay byte-compatible with what shipped:
        // url deps + explicit products, non-static product, the .v15-equivalent floor.
        let pieces = ApplePieces {
            packages: vec![SwiftPackage {
                url: Some("https://github.com/airbnb/lottie-ios".into()),
                path: None,
                from: Some("4.5.0".into()),
                exact: None,
                branch: None,
                revision: None,
                products: vec!["Lottie".into()],
            }],
            ..Default::default()
        };
        let text = package_swift(&pieces, "ios", ".iOS(\"15.0\")", false, true, false);
        assert!(text.contains("platforms: [.iOS(\"15.0\")]"));
        assert!(text.contains(".library(name: \"DayPieces\", targets: [\"DayPieces\"])"));
        assert!(
            text.contains(
                ".package(url: \"https://github.com/airbnb/lottie-ios\", from: \"4.5.0\"),"
            )
        );
        assert!(text.contains(".product(name: \"Lottie\", package: \"lottie-ios\"),"));
        assert!(text.contains(".process(\"Media.xcassets\")"));
        assert!(text.contains("[package.metadata.day.ios]"));
    }
}
