// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Build flavors (DESIGN.md §16.6): one source tree, several shipped apps.
//!
//! A flavor is a layer over `Day.toml`, declared in `Day-<name>.toml` beside it and activated with
//! `--flavor <name>` (or `DAY_FLAVOR`, which is how CI passes it). It may change the app's
//! identity, its targets, the cargo features it compiles with, the environment the build sees, and
//! which `resource/` and `store/` trees it ships — everything that makes a paid build, a
//! white-label build, or a store build a different *app* rather than a different project.
//!
//! The flag is global and process-wide, like `--day-src` and `--verbose`: every command reads the
//! active flavor back from here rather than threading it through its own signature, so a flavor
//! reaches the whole tool without 60 builders growing a parameter they would only pass along.
//!
//! What a flavor may NOT change: the crate name, the schema version, or the Day dependency. Those
//! make it a different project, and a project is what `day new` is for.
//!
//! Flutter takes the other road: its `--flavor` is
//! documented as "a custom Android product flavor or an Xcode scheme" — a pointer into the native
//! build systems rather than a model the tool owns. The cost shows up as parsing an Xcode
//! configuration name back into a flavor, and as helpers that guess the filenames Gradle will
//! emit. Day generates the host projects itself (the bundle id reaches Xcode through a generated
//! xcconfig, the applicationId through generated Gradle properties), so a flavor here changes
//! files Day already writes, and no native build system has to be taught the concept.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::meta::{App, AppOverride, Manifest};

/// The active flavor, set once from the global flag (or `DAY_FLAVOR`) before any command runs.
static ACTIVE: OnceLock<Option<String>> = OnceLock::new();
/// The active flavor's build inputs, filled when its manifest is merged: read by the builders.
static INPUTS: OnceLock<FlavorInputs> = OnceLock::new();

/// What a flavor adds to a build, beyond the app identity the manifest already carries.
#[derive(Debug, Default, Clone)]
pub struct FlavorInputs {
    /// Extra cargo features, on top of the backend feature and the pieces' union.
    pub features: Vec<String>,
    /// Environment for every build tool this run spawns: what `env!()` and build.rs read, the
    /// Rust answer to Flutter's `--dart-define`.
    pub env: BTreeMap<String, String>,
    /// The `resource/`-shaped tree overlaid on the app's own, relative to the project root.
    pub resources: Option<String>,
    /// The `store/`-shaped listing this flavor publishes with, relative to the project root.
    pub store: Option<String>,
    /// Whether the flavor named the artifact stem itself, anywhere in its `[app]` tables. When it
    /// did, [`crate::pack::naming::stem`] leaves the name alone instead of appending the flavor.
    pub names_artifact: bool,
}

/// Record the flavor this run builds. Called once, before the project is loaded.
///
/// `DAY_FLAVOR` is the fallback so a CI job can set it once for a whole matrix leg instead of
/// threading `--flavor` through every command in a script.
pub fn set(name: Option<String>) {
    let resolved = name
        .or_else(|| std::env::var("DAY_FLAVOR").ok())
        .filter(|s| !s.trim().is_empty());
    let _ = ACTIVE.set(resolved);
}

/// The flavor this run builds, if any.
pub fn active() -> Option<&'static str> {
    ACTIVE.get().and_then(|o| o.as_deref())
}

/// The build inputs the active flavor declared (empty when no flavor is active).
pub fn inputs() -> &'static FlavorInputs {
    static EMPTY: FlavorInputs = FlavorInputs {
        features: Vec::new(),
        env: BTreeMap::new(),
        resources: None,
        store: None,
        names_artifact: false,
    };
    INPUTS.get().unwrap_or(&EMPTY)
}

/// The manifest file a flavor is declared in: `Day-paid.toml` for `--flavor paid`.
pub fn manifest_name(name: &str) -> String {
    format!("Day-{name}.toml")
}

/// Every flavor a project declares, sorted: the `Day-<name>.toml` files beside its `Day.toml`.
/// `day lint` checks them all, and the error for an unknown flavor lists them.
pub fn declared(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_prefix("Day-")?.strip_suffix(".toml")?;
            (!stem.is_empty()).then(|| stem.to_string())
        })
        .collect();
    names.sort();
    names
}

/// One `Day-<name>.toml`. Strict, like the base manifest: a key this does not know is a typo, and
/// a typo that parses would silently build the base app instead of the flavor.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct FlavorManifest {
    #[serde(default)]
    app: Option<FlavorApp>,
    #[serde(default)]
    cargo: Option<FlavorCargo>,
    /// Environment for the build tools, e.g. a tier the app compiles against.
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// A `resource/`-shaped directory overlaid on the app's own (icons, locales, assets).
    #[serde(default)]
    resources: Option<String>,
    /// A `store/`-shaped listing directory, because a flavor is usually its own store record.
    #[serde(default)]
    store: Option<String>,
    /// `[signing.*]`, per platform, for a flavor that ships under someone else's account: a
    /// white-label build signed by the customer, or an app continuing a listing that belongs to
    /// another team. Each platform table the flavor states replaces the base app's for that
    /// platform; the ones it leaves out are inherited, values and `${VAR}` references alike.
    #[serde(default)]
    signing: Option<crate::meta::Signing>,
}

/// `[app]` in a flavor: the overridable identity, the target list, and the same
/// `[app.<platform|toolkit|target>]` tables the base manifest takes.
#[derive(Debug, Default, Deserialize)]
struct FlavorApp {
    #[serde(default)]
    id: Option<String>,
    /// The marketing version this flavor ships, in place of the crate's.
    ///
    /// The one `[app]` value a plain `Day.toml` cannot state, because a project has one version
    /// and Cargo owns it. A flavor is where it earns a second: a flavor that continues another
    /// app's store record inherits that record's release history, and both stores refuse a build
    /// whose version does not climb past what is already published there — a number the crate
    /// knows nothing about.
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    scheme: Option<String>,
    #[serde(default)]
    build: Option<u64>,
    /// Replaces the base list when present: a flavor may ship on fewer targets (a demo that is
    /// web only) or on more.
    #[serde(default)]
    targets: Option<Vec<String>>,
    #[serde(flatten)]
    overrides: BTreeMap<String, AppOverride>,
}

/// `[cargo]`: what a flavor compiles with. Only features — `day build` always passes
/// `--no-default-features` (the scaffold's default feature is `mock`, which no real build wants),
/// so a knob for it here would be a no-op wearing a name.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct FlavorCargo {
    #[serde(default)]
    features: Vec<String>,
}

/// Parse one flavor manifest, with the one check serde cannot express.
///
/// `resources` and `store` are top-level keys, and TOML gives a bare key to the last table it saw
/// — so writing them below `[env]` makes them environment variables called `resources` and
/// `store`, and the flavor quietly ships the base app's assets instead of its own. No build
/// environment has a lower-case variable by either name, so reading it as the mistake it is costs
/// nothing and saves a build that looks right everywhere except the thing it was meant to change.
fn parse(text: &str) -> Result<FlavorManifest, String> {
    let flavor: FlavorManifest = toml::from_str(text).map_err(|e| e.to_string())?;
    if flavor
        .app
        .as_ref()
        .and_then(|a| a.version.as_deref())
        .is_some_and(|v| v.trim().is_empty())
    {
        return Err("[app] version is empty — state a version or leave the key out".to_string());
    }
    for key in ["app", "cargo", "env", "resources", "store"] {
        if flavor.env.contains_key(key) {
            return Err(format!(
                "[env] sets {key:?}, which is a top-level key of the flavor, not a variable: \
                 move it above the first table (TOML reads a bare key after `[env]` as part of it)"
            ));
        }
    }
    Ok(flavor)
}

/// Merge the active flavor into a freshly parsed manifest, and record its build inputs.
///
/// Precedence, stated once because everything else follows from it: the flavor is a layer above
/// the whole base manifest, and inside each layer the existing specificity applies —
///
/// `flavor[app.<target>]` → `flavor[app.<platform>]` → `flavor[app]` →
/// `base[app.<target>]` → `base[app.<platform>]` → `base[app]`
///
/// which falls out of merging the flavor's scalars over the base's and its override tables
/// field-by-field over the base's: `Manifest::resolve` then reads one manifest and cannot tell a
/// flavored one from a plain one.
pub fn apply(root: &Path, manifest: &mut Manifest) -> Result<(), String> {
    let Some(name) = active() else {
        return Ok(());
    };
    let path = root.join(manifest_name(name));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        let known = declared(root);
        let known = if known.is_empty() {
            "this project declares none".to_string()
        } else {
            format!("this project declares: {}", known.join(", "))
        };
        format!(
            "no flavor {name:?}: {} not found ({known})",
            path.file_name().unwrap_or_default().to_string_lossy()
        )
    })?;
    let flavor = parse(&text).map_err(|e| format!("{}: {e}", manifest_name(name)))?;
    let names_artifact = flavor.app.as_ref().is_some_and(|app| {
        app.artifact.is_some() || app.overrides.values().any(|over| over.artifact.is_some())
    });
    merge_app(&mut manifest.app, flavor.app);
    merge_signing(&mut manifest.signing, flavor.signing);
    let cargo = flavor.cargo.unwrap_or_default();
    let _ = INPUTS.set(FlavorInputs {
        features: cargo.features,
        env: flavor.env,
        resources: flavor.resources,
        store: flavor.store,
        names_artifact,
    });
    Ok(())
}

/// Overlay a flavor's `[app]` onto the base one: a value it states wins, a value it omits is
/// inherited, and its override tables merge field-by-field so a flavor can retitle one platform
/// without restating the rest.
fn merge_app(base: &mut App, flavor: Option<FlavorApp>) {
    let Some(f) = flavor else { return };
    if let Some(v) = f.id {
        base.id = v;
    }
    if let Some(v) = f.version {
        base.version = v;
    }
    if f.title.is_some() {
        base.title = f.title;
    }
    if f.artifact.is_some() {
        base.artifact = f.artifact;
    }
    if f.scheme.is_some() {
        base.scheme = f.scheme;
    }
    if let Some(v) = f.build {
        base.build = v;
    }
    if let Some(v) = f.targets {
        base.targets = v;
    }
    for (key, over) in f.overrides {
        let slot = base.overrides.entry(key).or_default();
        if over.id.is_some() {
            slot.id = over.id;
        }
        if over.title.is_some() {
            slot.title = over.title;
        }
        if over.artifact.is_some() {
            slot.artifact = over.artifact;
        }
        if over.scheme.is_some() {
            slot.scheme = over.scheme;
        }
        if over.build.is_some() {
            slot.build = over.build;
        }
    }
}

/// Overlay a flavor's `[signing]` on the base app's, platform by platform. A platform the flavor
/// states is the flavor's, whole — an identity and a key belong together, and half of each would
/// sign nothing.
fn merge_signing(base: &mut Option<crate::meta::Signing>, flavor: Option<crate::meta::Signing>) {
    let Some(f) = flavor else { return };
    let base = base.get_or_insert_with(Default::default);
    if f.macos.is_some() {
        base.macos = f.macos;
    }
    if f.ios.is_some() {
        base.ios = f.ios;
    }
    if f.android.is_some() {
        base.android = f.android;
    }
    if f.windows.is_some() {
        base.windows = f.windows;
    }
    if f.ohos.is_some() {
        base.ohos = f.ohos;
    }
}

/// What `day lint` reads out of a flavor without activating it: enough to check it parses and to
/// compare flavors against each other.
pub struct FlavorPreview {
    /// The app id this flavor builds, or `None` when it inherits the base app's.
    pub id: Option<String>,
    /// The cargo features it enables.
    pub features: Vec<String>,
}

/// Parse one `Day-<name>.toml` for inspection. The same strict schema the merge uses, so a lint
/// pass and a build agree about what is valid.
pub fn preview(text: &str) -> Result<FlavorPreview, String> {
    let flavor = parse(text)?;
    Ok(FlavorPreview {
        id: flavor.app.and_then(|a| a.id),
        features: flavor.cargo.map(|c| c.features).unwrap_or_default(),
    })
}

/// Whether `Cargo.toml` declares `feature`, so a flavor naming one that does not exist is caught
/// by `day lint` rather than by a build that fails only when that flavor is asked for.
pub fn cargo_declares_feature(project: &crate::meta::Project, feature: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(project.root.join("Cargo.toml")) else {
        return true; // Unreadable: not this rule's business to complain about.
    };
    let Ok(doc) = toml::from_str::<toml::Value>(&text) else {
        return true;
    };
    // A `<pkg>/<feature>` reference enables a dependency's feature and needs no declaration here.
    if feature.contains('/') {
        return true;
    }
    doc.get("features")
        .and_then(|f| f.as_table())
        .is_some_and(|t| t.contains_key(feature))
}

/// The environment variable a flavor crosses a process boundary in.
pub const ENV: &str = "DAY_FLAVOR";

/// `DAY_FLAVOR=<name>` for a build tool that calls `day` back: an xcodebuild build setting (Xcode
/// exports those to its script phases), or the environment gradle hands its own callbacks.
///
/// Without it the callback process loads `Day.toml` alone and writes the base app's identity into
/// a bundle the flavor is assembling — which the xcode backend catches as "app metadata changed
/// since Xcode read it", one build after the values were already wrong.
pub fn setting() -> Option<String> {
    active().map(|name| format!("{ENV}={name}"))
}

/// The same, for a tool that takes an environment rather than an argument.
pub fn apply_env(cmd: &mut std::process::Command) {
    if let Some(name) = active() {
        cmd.env(ENV, name);
    }
}

/// The build environment a flavor contributes, in the shape the tool runners already take.
pub fn build_env() -> BTreeMap<String, OsString> {
    inputs()
        .env
        .iter()
        .map(|(k, v)| (k.clone(), OsString::from(v)))
        .collect()
}

/// The `resource/` tree this run stages from.
///
/// With no overlay declared it is the app's own, unchanged. With one, the two are merged into a
/// tree under the flavor's build root — the app's files, then the overlay's on top, by relative
/// path — so a flavor that ships one different icon states one icon rather than a copy of every
/// asset the app has. Merged once per process, because every stager asks for it.
pub fn merged_resources(project: &crate::meta::Project) -> Option<PathBuf> {
    static MERGED: OnceLock<Option<PathBuf>> = OnceLock::new();
    let overlay = project.root.join(inputs().resources.as_ref()?);
    MERGED
        .get_or_init(|| {
            let base = project.root.join("resource");
            let dest = crate::ops::staged_root(project).join("resource");
            let stamp_file = dest.with_extension("stamp");
            let stamp = tree_stamp(&[&base, &overlay]);
            // Skip a merge nothing changed under. Beyond the copying itself, the app's build
            // script reaches the strings through an `include_str!` of the merged file, so a tree
            // rewritten on every run would recompile the app crate on every run wherever the
            // copy does not carry the source timestamps over (it does on macOS, not on Linux).
            if dest.is_dir()
                && std::fs::read_to_string(&stamp_file).ok().as_deref() == Some(stamp.as_str())
            {
                return Some(dest);
            }
            let _ = std::fs::remove_dir_all(&dest);
            if let Err(e) = std::fs::create_dir_all(&dest) {
                crate::ops::status("Warning", &format!("flavor resources: {e}"));
                return None;
            }
            for src in [&base, &overlay] {
                if src.is_dir()
                    && let Err(e) = crate::pack::copy_tree(src, &dest)
                {
                    crate::ops::status("Warning", &format!("flavor resources: {e}"));
                    return None;
                }
            }
            let _ = std::fs::write(&stamp_file, &stamp);
            crate::ops::status(
                "Resources",
                &format!(
                    "{} over resource/ → {}",
                    overlay.file_name().unwrap_or_default().to_string_lossy(),
                    dest.display()
                ),
            );
            Some(dest)
        })
        .clone()
}

/// A cheap fingerprint of the trees a merge reads: every file's relative path, length and
/// modification time, hashed. It answers one question — has anything changed since the merge on
/// disk was made — and a file edited within a timestamp's resolution but kept at the same length
/// is the known cost of asking it this way rather than by reading every byte.
fn tree_stamp(roots: &[&Path]) -> String {
    use std::hash::{Hash, Hasher};
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    for root in roots {
        collect_stamp(root, root, &mut entries);
    }
    entries.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn collect_stamp(root: &Path, dir: &Path, out: &mut Vec<(String, u64, u128)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_stamp(root, &path, out);
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        out.push((rel, meta.len(), modified));
    }
}

/// The overlay directory [`merged_resources`] merged on top, if this flavor declares one.
pub fn resource_overlay(project: &crate::meta::Project) -> Option<PathBuf> {
    inputs()
        .resources
        .as_ref()
        .map(|dir| project.root.join(dir))
}

/// The `store/` listing this run publishes with: the flavor's when it declares one.
pub fn store_overlay(root: &Path) -> Option<PathBuf> {
    inputs().store.as_ref().map(|s| root.join(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App {
            name: "notes".into(),
            version: "1.0.0".into(),
            id: "dev.example.notes".into(),
            title: Some("Notes".into()),
            artifact: None,
            scheme: None,
            build: 1,
            targets: vec!["ios-uikit".into()],
            overrides: BTreeMap::new(),
        }
    }

    fn flavor(toml: &str) -> FlavorManifest {
        toml::from_str(toml).expect("flavor parses")
    }

    #[test]
    fn a_flavor_states_what_it_changes_and_inherits_the_rest() {
        let mut base = app();
        base.overrides.insert(
            "android".into(),
            AppOverride {
                id: Some("dev.example.notes.droid".into()),
                title: Some("Notes for Android".into()),
                ..Default::default()
            },
        );
        let f = flavor(
            r#"
            [app]
            id = "dev.example.notes.paid"
            build = 7
            targets = ["ios-uikit", "web-dom"]
            [app.android]
            id = "dev.example.notes_paid"
            "#,
        );
        merge_app(&mut base, f.app);
        assert_eq!(base.id, "dev.example.notes.paid");
        assert_eq!(base.build, 7);
        // Untouched by the flavor, so the base value stands.
        assert_eq!(base.title.as_deref(), Some("Notes"));
        assert_eq!(base.targets, ["ios-uikit", "web-dom"]);
        // The flavor restated one field of the android table; the other is still the base's.
        let android = &base.overrides["android"];
        assert_eq!(android.id.as_deref(), Some("dev.example.notes_paid"));
        assert_eq!(android.title.as_deref(), Some("Notes for Android"));
    }

    #[test]
    fn an_unknown_key_is_a_typo_rather_than_a_silent_base_build() {
        // A misspelled scalar under [app] is caught the way the base manifest catches one: it
        // lands in the flattened override map and fails to parse as an override table.
        let err = toml::from_str::<FlavorManifest>("[app]\nidd = \"x\"\n").unwrap_err();
        assert!(
            err.to_string().contains("expected struct AppOverride"),
            "{err}"
        );
        // A misspelled top-level key is named outright.
        let err = toml::from_str::<FlavorManifest>("resource = \"r\"\n").unwrap_err();
        assert!(err.to_string().contains("resource"), "{err}");
    }

    #[test]
    fn a_flavor_may_carry_its_own_release_train() {
        let mut base = app();
        let f = flavor("[app]\nversion = \"1.9.0\"\nbuild = 36\n");
        merge_app(&mut base, f.app);
        assert_eq!(base.version, "1.9.0");
        assert_eq!(base.build, 36);
        // The crate keeps its version when the flavor states none.
        let mut base = app();
        merge_app(&mut base, flavor("[app]\nbuild = 2\n").app);
        assert_eq!(base.version, "1.0.0");
        // An empty version would name an artifact `fair-games--android-mdc.aab`.
        let err = parse("[app]\nversion = \"\"\n").expect_err("caught");
        assert!(err.contains("version is empty"), "{err}");
    }

    #[test]
    fn signing_is_overlaid_one_platform_at_a_time() {
        let base: Option<crate::meta::Signing> = Some(
            toml::from_str(
                "[macos]\nidentity = \"Developer ID Application: Base\"\n\
                 [android]\nkeystore = \"base.keystore\"\nkey-alias = \"base\"\n\
                 store-pass = \"x\"\nkey-pass = \"x\"\n",
            )
            .expect("base signing"),
        );
        let mut merged = base;
        let flavor: crate::meta::Signing = toml::from_str(
            "[android]\nkeystore = \"${FLAVOR_KEYSTORE}\"\nkey-alias = \"${FLAVOR_ALIAS}\"\n\
                 store-pass = \"${FLAVOR_STORE_PASS}\"\nkey-pass = \"${FLAVOR_KEY_PASS}\"\n",
        )
        .expect("flavor signing");
        merge_signing(&mut merged, Some(flavor));
        let merged = merged.expect("merged");
        // The platform the flavor states is the flavor\'s...
        assert_eq!(
            merged.android.map(|a| a.keystore).as_deref(),
            Some("${FLAVOR_KEYSTORE}")
        );
        // ...and the one it says nothing about is still the base app\'s.
        assert_eq!(
            merged.macos.and_then(|m| m.identity).as_deref(),
            Some("Developer ID Application: Base")
        );
    }

    #[test]
    fn an_overlay_key_below_env_is_the_toml_trap_and_is_named() {
        // Written after [env], `resources` is an environment variable, not the overlay — and the
        // flavor would build with the base app's assets and no complaint from anyone.
        let err =
            parse("[env]\nTIER = \"paid\"\nresources = \"resource-paid\"\n").expect_err("caught");
        assert!(err.contains("top-level key"), "{err}");
        // Above the table, the same two lines mean what they say.
        let ok = parse("resources = \"resource-paid\"\n[env]\nTIER = \"paid\"\n").expect("parses");
        assert_eq!(ok.resources.as_deref(), Some("resource-paid"));
        assert_eq!(ok.env["TIER"], "paid");
    }

    #[test]
    fn declared_flavors_are_listed_for_the_unknown_flavor_error() {
        let dir = std::env::temp_dir().join(format!("day-flavor-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for f in ["Day.toml", "Day-paid.toml", "Day-demo.toml", "Days.toml"] {
            std::fs::write(dir.join(f), "").expect("fixture");
        }
        assert_eq!(declared(&dir), ["demo", "paid"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
