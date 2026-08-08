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

use day_build::permissions::{OhosScene, PermissionSpec};

use crate::meta::Manifest;

/// One permission the app will declare, after merging Day.toml with library contributions.
#[derive(Debug)]
pub struct Resolved {
    pub spec: &'static PermissionSpec,
    /// The user-facing reason for the platform this plan was resolved for. `None` when the
    /// permission needs none (notifications).
    pub reason: Option<String>,
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
    pub raw_apple: BTreeMap<String, String>,
    pub raw_ohos: Vec<OhosEntry>,
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
    let mut by_name: BTreeMap<&'static str, Resolved> = BTreeMap::new();

    for (name, decl) in &manifest.permissions.declared {
        let Some(spec) = day_build::permissions::find(name) else {
            continue; // parse_manifest already rejected unknown names
        };
        if !decl.enabled() || !decl.covers(platform) {
            continue;
        }
        by_name.insert(
            spec.name,
            Resolved {
                spec,
                reason: decl.reason_for(platform).map(str::to_string),
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
                by_name.insert(
                    spec.name,
                    Resolved {
                        spec,
                        reason: None,
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
                 \n      [permissions]\n      {} = \"…why this app needs it…\"\n",
                r.spec.name
            ));
        }
    }

    let raw = &manifest.permissions.raw;
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
        raw_apple: match platform {
            "macos" => raw.macos.clone(),
            _ => raw.ios.clone(),
        },
        raw_ohos: raw
            .ohos
            .iter()
            .map(|p| OhosEntry {
                name: p.name.clone(),
                reason_key: p.reason.as_ref().map(|_| reason_key(&p.name)),
                when: match p.when.as_deref() {
                    Some("always") => "always",
                    _ => "inuse",
                },
            })
            .collect(),
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

/// The `$string:` resources those entries reference: resource name → text.
pub fn ohos_reason_strings(plan: &Plan, manifest: &Manifest) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in &plan.resolved {
        if r.spec.ohos.is_empty() {
            continue;
        }
        if let Some(reason) = &r.reason {
            out.insert(reason_key(r.spec.name), reason.clone());
        }
    }
    for p in &manifest.permissions.raw.ohos {
        if let Some(reason) = &p.reason {
            out.insert(reason_key(&p.name), reason.clone());
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
        let strings = ohos_reason_strings(&plan, &m);
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
