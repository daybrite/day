// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Resolve `[permissions]` (plus any library contributions) into what each platform must declare.
//!
//! One [`Plan`] feeds every writer — the Android manifest overlay, the Apple `Info.plist` keys, and
//! the HarmonyOS `module.json5` entries — so the three can never disagree about what the app asked
//! for. The table itself lives in `day_build::permissions`, shared with `day-part-permissions` so a
//! generated declaration cannot drift from the permission the app's code requests at runtime.
//!
//! Everything here is a pure function over the parsed manifest: no filesystem, no `cargo metadata`,
//! no platform tools. That is what makes the interesting parts (reason precedence, the union with
//! library contributions, the per-platform projections) testable on any host, which matters because
//! two of the three writers target platforms CI cannot run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use day_build::permissions::{OhosScene, PermissionSpec};

use crate::meta::Manifest;

/// The user-facing reasons, read from the app's Fluent catalogs (docs/permissions.md,
/// "Localized reasons"): one `permission_<id>` message per declaration, in every locale the
/// app has. The catalog is what makes a reason translatable; Day.toml's inline text remains
/// the single-locale shortcut and the fallback for the default locale.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// The app's default locale (`en` when present, else the first), the text every platform
    /// manifest carries directly.
    pub default_locale: String,
    /// locale → message id → text, `permission_*` messages only.
    pub messages: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for Catalog {
    /// No catalogs at all: the default locale is still `en`, so a plan resolved from a bare
    /// manifest files its inline reasons under a real locale.
    fn default() -> Self {
        Catalog {
            default_locale: "en".to_string(),
            messages: BTreeMap::new(),
        }
    }
}

impl Catalog {
    /// Read `resource/locales/<tag>/*.ftl` under `root`. A project without catalogs gets an
    /// empty one whose default locale is `en`.
    pub fn load(root: &Path) -> Catalog {
        let dir = root.join("resource/locales");
        let mut messages: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut locales = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if !e.path().is_dir() {
                    continue;
                }
                let tag = e.file_name().to_string_lossy().into_owned();
                // The pseudolocale is a development aid, never a shipped translation.
                if tag == "en-XA" {
                    continue;
                }
                let mut files: Vec<_> = std::fs::read_dir(e.path())
                    .map(|d| d.flatten().map(|f| f.path()).collect())
                    .unwrap_or_default();
                files.sort();
                let map = messages.entry(tag.clone()).or_default();
                for path in files {
                    if path.extension().is_some_and(|x| x == "ftl")
                        && let Ok(src) = std::fs::read_to_string(&path)
                    {
                        for (key, text) in day_build::message_texts(&src) {
                            if key.starts_with(MESSAGE_PREFIX) && !text.trim().is_empty() {
                                map.entry(key).or_insert(text);
                            }
                        }
                    }
                }
                locales.push(tag);
            }
        }
        locales.sort();
        Catalog {
            default_locale: crate::store::default_locale(&locales)
                .unwrap_or_else(|| "en".to_string()),
            messages,
        }
    }

    /// The message `id` in `locale`, when that catalog has it.
    pub fn text(&self, locale: &str, id: &str) -> Option<&str> {
        self.messages.get(locale)?.get(id).map(String::as_str)
    }

    /// `id` in every locale that has it: locale → text.
    fn texts(&self, id: &str) -> BTreeMap<String, String> {
        self.messages
            .iter()
            .filter_map(|(l, m)| m.get(id).map(|t| (l.clone(), t.clone())))
            .collect()
    }

    /// The locales the catalog covers.
    pub fn locales(&self) -> impl Iterator<Item = &String> {
        self.messages.keys()
    }
}

/// Every reason message starts with this, which is how lint knows a message is consumed by the
/// declaration pipeline rather than by app code.
pub const MESSAGE_PREFIX: &str = "permission_";

/// The catalog message id for a permission: `permission_camera`, `permission_location_when_in_use`,
/// and for a raw key the key itself, `permission_NSBluetoothAlwaysUsageDescription`.
pub fn message_id(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{MESSAGE_PREFIX}{slug}")
}

/// One permission the app will declare, after merging Day.toml with library contributions.
#[derive(Debug)]
pub struct Resolved {
    pub spec: &'static PermissionSpec,
    /// The user-facing reason for the platform this plan was resolved for, in the default
    /// locale. `None` when the permission needs none (notifications).
    pub reason: Option<String>,
    /// The same reason in every locale the catalog has it for (locale → text), the default
    /// locale included. Empty for a permission that needs none.
    pub reasons: BTreeMap<String, String>,
    /// Who asked for it — `"Day.toml"` and/or the contributing crate names, for diagnostics.
    pub sources: Vec<String>,
}

/// Everything one platform must declare.
#[derive(Debug, Default)]
pub struct Plan {
    /// Portable permissions, sorted by name so every generated file is byte-stable across builds.
    pub resolved: Vec<Resolved>,
    /// `[permissions.raw]` for this platform, passed through untouched.
    pub raw_android: Vec<AndroidRaw>,
    /// Raw Apple keys → their default-locale text.
    pub raw_apple: BTreeMap<String, String>,
    /// Raw Apple keys → locale → text, the catalog's translations.
    pub raw_apple_reasons: BTreeMap<String, BTreeMap<String, String>>,
    pub raw_ohos: Vec<OhosEntry>,
    /// Raw HarmonyOS permission names → locale → reason text.
    pub raw_ohos_reasons: BTreeMap<String, BTreeMap<String, String>>,
    /// The locale the platform manifests carry directly; every other locale is a translation
    /// (`InfoPlist.xcstrings`, the per-locale `string.json`).
    pub default_locale: String,
}

/// An Android `<uses-permission>` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidRaw {
    pub name: String,
    pub max_sdk: Option<u32>,
}

/// A HarmonyOS `requestPermissions` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OhosEntry {
    pub name: String,
    /// The `$string:` resource NAME (not the text) — HarmonyOS requires a resource reference.
    pub reason_key: Option<String>,
    pub when: &'static str,
}

/// Resolve for `platform` — one of `"android"`, `"ios"`, `"macos"`, `"ohos"`.
///
/// `contributed` is `(crate_name, permission_name)` from dependencies' own
/// `[package.metadata.day.permissions]`. A contribution the app has not given a reason for is a
/// hard ERROR on the platforms that need one: the alternative is an app that builds fine and then
/// terminates the first time it touches the API on a device.
pub fn resolve(
    manifest: &Manifest,
    platform: &str,
    contributed: &[(String, String)],
) -> Result<Plan, String> {
    resolve_with(manifest, platform, contributed, &Catalog::default())
}

/// [`resolve`] for a project on disk: the manifest plus the catalogs under `resource/locales/`,
/// so reasons come out localized. What every writer uses.
pub fn resolve_project(
    project: &crate::meta::Project,
    platform: &str,
    contributed: &[(String, String)],
) -> Result<Plan, String> {
    resolve_with(
        &project.manifest,
        platform,
        contributed,
        &Catalog::load(&project.root),
    )
}

/// The reason texts for one declaration: the catalog's `permission_<id>` message in every
/// locale, with Day.toml's inline text filling the default locale when the catalog lacks it.
fn reason_texts(catalog: &Catalog, id: &str, inline: Option<&str>) -> BTreeMap<String, String> {
    let mut out = catalog.texts(id);
    if let Some(text) = inline
        && !out.contains_key(&catalog.default_locale)
    {
        out.insert(catalog.default_locale.clone(), text.to_string());
    }
    out
}

/// [`resolve`] against an explicit catalog.
pub fn resolve_with(
    manifest: &Manifest,
    platform: &str,
    contributed: &[(String, String)],
    catalog: &Catalog,
) -> Result<Plan, String> {
    let mut by_name: BTreeMap<&'static str, Resolved> = BTreeMap::new();
    let default = catalog.default_locale.as_str();

    for (name, decl) in &manifest.permissions.declared {
        let Some(spec) = day_build::permissions::find(name) else {
            continue; // parse_manifest already rejected unknown names
        };
        if !decl.enabled() || !decl.covers(platform) {
            continue;
        }
        let reasons = if spec.needs_reason {
            reason_texts(catalog, &message_id(spec.name), decl.reason_for(platform))
        } else {
            BTreeMap::new()
        };
        by_name.insert(
            spec.name,
            Resolved {
                spec,
                reason: reasons.get(default).cloned(),
                reasons,
                sources: vec!["Day.toml".to_string()],
            },
        );
    }

    for (crate_name, perm) in contributed {
        let Some(spec) = day_build::permissions::find(perm) else {
            return Err(format!(
                "{crate_name} declares [package.metadata.day.permissions] uses = [{perm:?}], which \
                 is not a known permission (valid: {})",
                day_build::permissions::names().join(", ")
            ));
        };
        match by_name.get_mut(spec.name) {
            Some(existing) => existing.sources.push(crate_name.clone()),
            None => {
                // A contribution's reason is the app's to give, so it still comes from the
                // catalog when the app wrote one there without a Day.toml line of its own.
                let reasons = if spec.needs_reason {
                    reason_texts(catalog, &message_id(spec.name), None)
                } else {
                    BTreeMap::new()
                };
                by_name.insert(
                    spec.name,
                    Resolved {
                        spec,
                        reason: reasons.get(default).cloned(),
                        reasons,
                        sources: vec![crate_name.clone()],
                    },
                );
            }
        }
    }

    // A reason is only consumable where the platform has somewhere to put it.
    let needs_reason_here = matches!(platform, "ios" | "macos" | "ohos");
    for r in by_name.values() {
        let consumes = match platform {
            "ios" => !r.spec.ios.is_empty(),
            "macos" => !r.spec.macos.is_empty(),
            "ohos" => !r.spec.ohos.is_empty(),
            _ => false,
        };
        if needs_reason_here && consumes && r.spec.needs_reason && r.reason.is_none() {
            let who = r
                .sources
                .iter()
                .filter(|s| *s != "Day.toml")
                .cloned()
                .collect::<Vec<_>>();
            let blame = if who.is_empty() {
                format!("[permissions] {} needs a reason", r.spec.name)
            } else {
                format!(
                    "{} declares [package.metadata.day.permissions] uses = [{:?}], but Day.toml \
                     gives no reason for it",
                    who.join(", "),
                    r.spec.name
                )
            };
            return Err(format!(
                "{blame}.\n  {platform} shows this text to the user when it prompts, and an app \
                 that touches the API without it is terminated by the OS.\n  Add to Day.toml:\n\
                 \n      [permissions]\n      {name} = \"…why this app needs it…\"\n\n  or, \
                 translatable, to resource/locales/{default}/app.ftl:\n\n      {id} = …why this \
                 app needs it…\n",
                name = r.spec.name,
                id = message_id(r.spec.name),
            ));
        }
    }

    let raw = &manifest.permissions.raw;
    let raw_apple_table = match platform {
        "macos" => &raw.macos,
        _ => &raw.ios,
    };
    let mut raw_apple = BTreeMap::new();
    let mut raw_apple_reasons = BTreeMap::new();
    for (key, value) in raw_apple_table {
        if !value.enabled() {
            continue;
        }
        let reasons = reason_texts(catalog, &message_id(key), value.literal());
        let Some(text) = reasons.get(default) else {
            return Err(format!(
                "[permissions.raw] {key} = true takes its text from the catalog, but \
                 resource/locales/{default}/app.ftl has no `{id}` message",
                id = message_id(key)
            ));
        };
        raw_apple.insert(key.clone(), text.clone());
        raw_apple_reasons.insert(key.clone(), reasons);
    }
    let mut raw_ohos = Vec::new();
    let mut raw_ohos_reasons = BTreeMap::new();
    for p in &raw.ohos {
        let reasons = reason_texts(catalog, &message_id(&p.name), p.reason.as_deref());
        raw_ohos.push(OhosEntry {
            name: p.name.clone(),
            reason_key: reasons.contains_key(default).then(|| reason_key(&p.name)),
            when: match p.when.as_deref() {
                Some("always") => "always",
                _ => "inuse",
            },
        });
        if !reasons.is_empty() {
            raw_ohos_reasons.insert(p.name.clone(), reasons);
        }
    }
    Ok(Plan {
        resolved: by_name.into_values().collect(),
        raw_android: raw
            .android
            .iter()
            .map(|name| AndroidRaw {
                name: name.clone(),
                max_sdk: None,
            })
            .collect(),
        raw_apple,
        raw_apple_reasons,
        raw_ohos,
        raw_ohos_reasons,
        default_locale: default.to_string(),
    })
}

/// The `$string:` resource name for a permission's reason. Namespaced by the `day_perm_reason_`
/// prefix, which is how the resource writer knows which entries it owns.
pub fn reason_key(permission: &str) -> String {
    let slug: String = permission
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("day_perm_reason_{}", slug.to_ascii_lowercase())
}

// ---------------------------------------------------------------------------
// Projections — pure functions over a Plan, one per writer.
// ---------------------------------------------------------------------------

/// `<uses-permission>` entries, deduped and sorted (a stable file keeps AGP's up-to-date checks warm).
pub fn android_entries(plan: &Plan) -> Vec<AndroidRaw> {
    let mut out: BTreeMap<String, Option<u32>> = BTreeMap::new();
    for r in &plan.resolved {
        for p in r.spec.android {
            // A permission contributed twice keeps the TIGHTER cap: dropping a maxSdkVersion would
            // silently widen what the app asks for.
            let slot = out.entry(p.name.to_string()).or_insert(p.max_sdk);
            *slot = match (*slot, p.max_sdk) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            };
        }
    }
    for p in &plan.raw_android {
        out.entry(p.name.clone()).or_insert(p.max_sdk);
    }
    out.into_iter()
        .map(|(name, max_sdk)| AndroidRaw { name, max_sdk })
        .collect()
}

/// `Info.plist` usage-description keys → text.
pub fn apple_keys(plan: &Plan, macos: bool) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in &plan.resolved {
        let keys = if macos { r.spec.macos } else { r.spec.ios };
        let Some(reason) = r.reason.as_ref() else {
            continue; // notifications: declared, but nothing to write
        };
        for key in keys {
            out.insert((*key).to_string(), reason.clone());
        }
    }
    for (k, v) in &plan.raw_apple {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// `Info.plist` usage-description keys → locale → text, every locale the catalog translates:
/// what `InfoPlist.xcstrings` carries so the prompt speaks the user's language.
pub fn apple_keys_localized(
    plan: &Plan,
    macos: bool,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for r in &plan.resolved {
        if r.reasons.is_empty() {
            continue;
        }
        for key in if macos { r.spec.macos } else { r.spec.ios } {
            out.insert((*key).to_string(), r.reasons.clone());
        }
    }
    for (k, v) in &plan.raw_apple_reasons {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// True when any key has text in a locale other than the default — the case that needs an
/// `InfoPlist.xcstrings` at all.
pub fn has_translations(keys: &BTreeMap<String, BTreeMap<String, String>>, default: &str) -> bool {
    keys.values().any(|m| m.keys().any(|l| l != default))
}

/// The `InfoPlist.xcstrings` string catalog (Xcode 15+) for `keys`: one entry per usage
/// description, one `stringUnit` per locale, spelled the way Xcode spells locales
/// (`zh-Hans`, not `zh-CN`). Sorted maps in, byte-stable JSON out — two builds produce the
/// same file, so the tracked catalog stays clean.
pub fn xcstrings_json(default: &str, keys: &BTreeMap<String, BTreeMap<String, String>>) -> String {
    let mut strings = serde_json::Map::new();
    for (key, by_locale) in keys {
        let mut localizations = serde_json::Map::new();
        for (locale, text) in by_locale {
            localizations.insert(
                crate::localize::xcode_region(locale),
                serde_json::json!({ "stringUnit": { "state": "translated", "value": text } }),
            );
        }
        strings.insert(
            key.clone(),
            serde_json::json!({ "extractionState": "manual", "localizations": localizations }),
        );
    }
    let doc = serde_json::json!({
        "sourceLanguage": crate::localize::xcode_region(default),
        "strings": strings,
        "version": "1.0",
    });
    let mut out = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

/// Every `Info.plist` key Day manages on this platform — the set it may write OR remove. Derived
/// from the table, so a fresh clone needs no state file to clean up after a removed declaration.
pub fn apple_managed_keys(macos: bool) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for spec in day_build::permissions::ALL {
        for key in if macos { spec.macos } else { spec.ios } {
            out.insert((*key).to_string());
        }
    }
    out
}

/// HarmonyOS `requestPermissions` entries.
pub fn ohos_entries(plan: &Plan) -> Vec<OhosEntry> {
    let mut out: Vec<OhosEntry> = Vec::new();
    for r in &plan.resolved {
        for p in r.spec.ohos {
            out.push(OhosEntry {
                name: p.name.to_string(),
                reason_key: r.reason.as_ref().map(|_| reason_key(r.spec.name)),
                when: match p.when {
                    OhosScene::Always => "always",
                    OhosScene::InUse => "inuse",
                },
            });
        }
    }
    out.extend(plan.raw_ohos.iter().cloned());
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// The `$string:` resources those entries reference, in the default locale: resource name →
/// text (the base `string.json`).
pub fn ohos_reason_strings(plan: &Plan) -> BTreeMap<String, String> {
    ohos_reason_strings_localized(plan)
        .remove(&plan.default_locale)
        .unwrap_or_default()
}

/// The same for every locale: locale → resource name → text (the per-locale `string.json`s).
pub fn ohos_reason_strings_localized(plan: &Plan) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for r in &plan.resolved {
        if r.spec.ohos.is_empty() {
            continue;
        }
        for (locale, text) in &r.reasons {
            out.entry(locale.clone())
                .or_default()
                .insert(reason_key(r.spec.name), text.clone());
        }
    }
    for (name, reasons) in &plan.raw_ohos_reasons {
        for (locale, text) in reasons {
            out.entry(locale.clone())
                .or_default()
                .insert(reason_key(name), text.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::parse_manifest;

    const CARGO: &str = "[package]\nname = \"demo-app\"\nversion = \"1.0.0\"\n";

    fn manifest(perms: &str) -> Manifest {
        parse_manifest(
            &format!("schema = 1\n[app]\nid = \"dev.x.demo\"\n{perms}"),
            CARGO,
            None,
        )
        .expect("parse")
    }

    #[test]
    fn day_toml_only() {
        let m = manifest("[permissions]\ncamera = \"Scan a document.\"\n");
        let plan = resolve(&m, "ios", &[]).expect("resolve");
        assert_eq!(plan.resolved.len(), 1);
        assert_eq!(plan.resolved[0].reason.as_deref(), Some("Scan a document."));
        assert_eq!(plan.resolved[0].sources, ["Day.toml"]);
        assert_eq!(
            apple_keys(&plan, false)
                .get("NSCameraUsageDescription")
                .map(String::as_str),
            Some("Scan a document.")
        );
    }

    /// A library that needs the camera but no app reason: fine on Android (no reason exists there),
    /// a hard error on iOS, where it would otherwise be a crash on a device.
    #[test]
    fn library_contribution_without_a_reason() {
        let m = manifest("");
        let contributed = [("day-piece-media".to_string(), "camera".to_string())];

        let android = resolve(&m, "android", &contributed).expect("android is permissive");
        assert_eq!(android.resolved.len(), 1);

        let err = resolve(&m, "ios", &contributed).expect_err("ios must refuse");
        assert!(err.contains("day-piece-media"), "{err}");
        assert!(
            err.contains("camera = "),
            "must show the lines to paste: {err}"
        );
    }

    #[test]
    fn day_toml_reason_satisfies_a_contribution() {
        let m = manifest("[permissions]\ncamera = \"Attach a photo.\"\n");
        let plan = resolve(
            &m,
            "ios",
            &[("day-piece-media".to_string(), "camera".to_string())],
        )
        .expect("resolve");
        assert_eq!(plan.resolved.len(), 1);
        assert_eq!(plan.resolved[0].sources, ["Day.toml", "day-piece-media"]);
    }

    /// Notifications needs no reason anywhere, so it must not trip the reason check — and it writes
    /// no Apple key at all.
    #[test]
    fn notifications_needs_no_reason() {
        let m = manifest("[permissions]\nnotifications = true\n");
        let plan = resolve(&m, "ios", &[]).expect("resolve");
        assert!(apple_keys(&plan, false).is_empty());
        assert_eq!(
            android_entries(&resolve(&m, "android", &[]).unwrap())
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>(),
            ["android.permission.POST_NOTIFICATIONS"]
        );
    }

    /// motion has no macOS key, so a macOS build must not demand a reason it cannot use.
    #[test]
    fn a_platform_without_the_key_needs_no_reason() {
        let m = manifest("[permissions.motion]\nios-reason = \"Count your steps.\"\n");
        resolve(&m, "macos", &[]).expect("macos has no motion key, so no reason is needed");
        let ios = resolve(&m, "ios", &[]).expect("ios reason supplied");
        assert_eq!(
            apple_keys(&ios, false)
                .get("NSMotionUsageDescription")
                .map(String::as_str),
            Some("Count your steps.")
        );
    }

    #[test]
    fn photos_caps_legacy_storage_and_keeps_the_cap_on_merge() {
        let m = manifest("[permissions]\nphotos = \"Attach a picture.\"\n");
        let plan = resolve(&m, "android", &[]).expect("resolve");
        let entries = android_entries(&plan);
        let legacy = entries
            .iter()
            .find(|p| p.name.ends_with("READ_EXTERNAL_STORAGE"))
            .expect("legacy storage");
        assert_eq!(legacy.max_sdk, Some(32));
        // Sorted and deduped, so the generated overlay is byte-stable.
        let names: Vec<_> = entries.iter().map(|p| p.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    /// `platforms = [...]` restricts where a declaration applies.
    #[test]
    fn platform_subset_is_honored() {
        let m = manifest("[permissions.camera]\nreason = \"Scan.\"\nplatforms = [\"ios\"]\n");
        assert_eq!(resolve(&m, "ios", &[]).unwrap().resolved.len(), 1);
        assert_eq!(resolve(&m, "android", &[]).unwrap().resolved.len(), 0);
    }

    #[test]
    fn ohos_entries_reference_reason_resources() {
        let m = manifest("[permissions]\ncamera = \"Scan a document.\"\n");
        let plan = resolve(&m, "ohos", &[]).expect("resolve");
        let entries = ohos_entries(&plan);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "ohos.permission.CAMERA");
        assert_eq!(
            entries[0].reason_key.as_deref(),
            Some("day_perm_reason_camera")
        );
        assert_eq!(entries[0].when, "inuse");
        let strings = ohos_reason_strings(&plan);
        assert_eq!(
            strings.get("day_perm_reason_camera").map(String::as_str),
            Some("Scan a document.")
        );
    }

    /// Background location is the one entry whose scene differs.
    #[test]
    fn ohos_background_location_uses_the_always_scene() {
        let m = manifest("[permissions]\nlocation-always = \"Track your route.\"\n");
        let plan = resolve(&m, "ohos", &[]).expect("resolve");
        let entries = ohos_entries(&plan);
        let bg = entries
            .iter()
            .find(|e| e.name.ends_with("LOCATION_IN_BACKGROUND"))
            .expect("background entry");
        assert_eq!(bg.when, "always");
    }

    /// Apple requires the when-in-use key alongside the always key, or the prompt never appears.
    #[test]
    fn location_always_writes_both_apple_keys() {
        let m = manifest("[permissions]\nlocation-always = \"Track your route.\"\n");
        let keys = apple_keys(&resolve(&m, "ios", &[]).unwrap(), false);
        assert!(keys.contains_key("NSLocationAlwaysAndWhenInUseUsageDescription"));
        assert!(keys.contains_key("NSLocationWhenInUseUsageDescription"));
    }

    #[test]
    fn raw_escape_hatches_pass_through() {
        let m = manifest(
            "[permissions.raw]\nandroid = [\"android.permission.READ_CONTACTS\"]\n\
             ios = { NSContactsUsageDescription = \"Find friends.\" }\n",
        );
        let android = android_entries(&resolve(&m, "android", &[]).unwrap());
        assert!(android.iter().any(|p| p.name.ends_with("READ_CONTACTS")));
        let ios = apple_keys(&resolve(&m, "ios", &[]).unwrap(), false);
        assert_eq!(
            ios.get("NSContactsUsageDescription").map(String::as_str),
            Some("Find friends.")
        );
    }

    /// The managed set is what makes the plist writer safe: it is derived from the table, so a
    /// removed declaration is cleaned up even on a fresh clone with no state file.
    #[test]
    fn managed_keys_cover_the_table() {
        let ios = apple_managed_keys(false);
        assert!(ios.contains("NSCameraUsageDescription"));
        assert!(ios.contains("NSMotionUsageDescription"));
        // A key Day never writes must not be in the managed set, or it would be removed from a
        // user's hand-edited plist.
        assert!(!ios.contains("NSContactsUsageDescription"));
        assert!(!apple_managed_keys(true).contains("NSMotionUsageDescription"));
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::meta::parse_manifest;

    const CARGO: &str = "[package]\nname = \"demo-app\"\nversion = \"1.0.0\"\n";

    fn manifest(perms: &str) -> Manifest {
        parse_manifest(
            &format!("schema = 1\n[app]\nid = \"dev.x.demo\"\n{perms}"),
            CARGO,
            None,
        )
        .expect("parse")
    }

    fn catalog(entries: &[(&str, &[(&str, &str)])]) -> Catalog {
        let mut messages: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (locale, msgs) in entries {
            let m = messages.entry((*locale).to_string()).or_default();
            for (k, v) in *msgs {
                m.insert((*k).to_string(), (*v).to_string());
            }
        }
        Catalog {
            default_locale: "en".into(),
            messages,
        }
    }

    #[test]
    fn message_ids_keep_case_and_slug_the_rest() {
        assert_eq!(message_id("camera"), "permission_camera");
        assert_eq!(
            message_id("location-when-in-use"),
            "permission_location_when_in_use"
        );
        assert_eq!(
            message_id("NSBluetoothAlwaysUsageDescription"),
            "permission_NSBluetoothAlwaysUsageDescription"
        );
        assert_eq!(
            message_id("ohos.permission.READ_CONTACTS"),
            "permission_ohos_permission_READ_CONTACTS"
        );
    }

    #[test]
    fn the_catalog_message_wins_and_every_locale_rides_along() {
        let m = manifest("[permissions]\ncamera = \"Inline.\"\n");
        let c = catalog(&[
            ("en", &[("permission_camera", "Scan.")]),
            ("fr", &[("permission_camera", "Scanner.")]),
        ]);
        let plan = resolve_with(&m, "ios", &[], &c).expect("plan");
        let cam = &plan.resolved[0];
        assert_eq!(cam.reason.as_deref(), Some("Scan."));
        assert_eq!(cam.reasons.get("fr").map(String::as_str), Some("Scanner."));
        let localized = apple_keys_localized(&plan, false);
        assert_eq!(localized["NSCameraUsageDescription"]["fr"], "Scanner.");
        assert!(has_translations(&localized, "en"));
    }

    #[test]
    fn the_inline_text_fills_the_default_locale_when_the_catalog_lacks_it() {
        let m = manifest("[permissions]\ncamera = \"Inline.\"\n");
        let c = catalog(&[("fr", &[("permission_camera", "Scanner.")])]);
        let plan = resolve_with(&m, "ios", &[], &c).expect("plan");
        assert_eq!(plan.resolved[0].reason.as_deref(), Some("Inline."));
        assert_eq!(plan.resolved[0].reasons.len(), 2);
        assert!(!has_translations(
            &apple_keys_localized(&resolve(&m, "ios", &[]).expect("plan"), false),
            "en"
        ));
    }

    #[test]
    fn a_declaration_with_only_a_catalog_reason_resolves() {
        let m = manifest("[permissions]\ncamera = true\n");
        assert!(resolve(&m, "ios", &[]).is_err(), "no reason anywhere");
        let c = catalog(&[("en", &[("permission_camera", "Scan.")])]);
        let plan = resolve_with(&m, "ios", &[], &c).expect("plan");
        assert_eq!(
            apple_keys(&plan, false)["NSCameraUsageDescription"],
            "Scan."
        );
    }

    #[test]
    fn raw_apple_keys_take_the_catalog_or_the_literal() {
        let m = manifest(
            "[permissions.raw]\nios = { NSContactsUsageDescription = true, NSFaceIDUsageDescription = \"Unlock.\" }\n",
        );
        assert!(resolve(&m, "ios", &[]).is_err(), "true without a message");
        let c = catalog(&[
            (
                "en",
                &[("permission_NSContactsUsageDescription", "Friends.")],
            ),
            (
                "de",
                &[
                    ("permission_NSContactsUsageDescription", "Freunde."),
                    ("permission_NSFaceIDUsageDescription", "Entsperren."),
                ],
            ),
        ]);
        let plan = resolve_with(&m, "ios", &[], &c).expect("plan");
        assert_eq!(plan.raw_apple["NSContactsUsageDescription"], "Friends.");
        assert_eq!(plan.raw_apple["NSFaceIDUsageDescription"], "Unlock.");
        let localized = apple_keys_localized(&plan, false);
        assert_eq!(localized["NSFaceIDUsageDescription"]["de"], "Entsperren.");
        assert_eq!(localized["NSFaceIDUsageDescription"]["en"], "Unlock.");
    }

    #[test]
    fn ohos_strings_come_out_per_locale() {
        let m = manifest(
            "[permissions]\ncamera = \"Scan.\"\n[permissions.raw]\nohos = [{ name = \"ohos.permission.READ_CONTACTS\", reason = \"Friends.\" }]\n",
        );
        let c = catalog(&[(
            "zh-CN",
            &[
                ("permission_camera", "扫描。"),
                ("permission_ohos_permission_READ_CONTACTS", "朋友。"),
            ],
        )]);
        let plan = resolve_with(&m, "ohos", &[], &c).expect("plan");
        let all = ohos_reason_strings_localized(&plan);
        assert_eq!(all["en"]["day_perm_reason_camera"], "Scan.");
        assert_eq!(all["zh-CN"]["day_perm_reason_camera"], "扫描。");
        assert_eq!(
            all["zh-CN"]["day_perm_reason_ohos_permission_read_contacts"],
            "朋友。"
        );
        assert_eq!(
            ohos_reason_strings(&plan)["day_perm_reason_ohos_permission_read_contacts"],
            "Friends."
        );
    }

    #[test]
    fn xcstrings_spell_locales_the_xcode_way_and_stay_stable() {
        let mut keys: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        keys.entry("NSCameraUsageDescription".into())
            .or_default()
            .insert("en".into(), "Scan.".into());
        keys.entry("NSCameraUsageDescription".into())
            .or_default()
            .insert("zh-CN".into(), "扫描。".into());
        let a = xcstrings_json("en", &keys);
        let v: serde_json::Value = serde_json::from_str(&a).expect("json");
        assert_eq!(v["sourceLanguage"], "en");
        assert_eq!(v["version"], "1.0");
        assert_eq!(
            v["strings"]["NSCameraUsageDescription"]["localizations"]["zh-Hans"]["stringUnit"]["value"],
            "扫描。"
        );
        assert_eq!(
            v["strings"]["NSCameraUsageDescription"]["extractionState"],
            "manual"
        );
        assert_eq!(a, xcstrings_json("en", &keys));
    }
}
