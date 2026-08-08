// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `[[shortcuts]]` conveyance — launcher shortcuts as saved deep links (docs/deep-links.md).
//!
//! Each Day.toml `[[shortcuts]]` entry is a route plus a Fluent label id. `day build` resolves
//! the label in every locale under `resource/locales/` and writes each platform's native
//! declaration:
//!
//! - **Android** — everything is staged, nothing committed: `build/day/android/res/xml/
//!   day_shortcuts.xml` + per-locale `values*/day_shortcuts.xml` string resources (the scaffold
//!   already registers `build/day/android/res` as a res srcDir), and the `<meta-data>` that
//!   points the launcher activity at them rides the day-pieces overlay manifest (pieces.rs).
//! - **iOS** — `UIApplicationShortcutItems` in the committed `Info.plist` (same editor as the
//!   permission keys), titled with the default-locale text; per-locale titles are
//!   `<loc>.lproj/InfoPlist.strings` files staged into the built bundle by the scaffold's
//!   `day xcode-backend stage-strings` script phase, keyed by that default text.
//! - **HarmonyOS** — `base/profile/shortcuts_config.json` + an `ohos.ability.shortcuts`
//!   metadata entry on the ability, labels as `$string:` references merged into each locale's
//!   `string.json` (ohos.rs owns those writers and calls in here).
//!
//! Activation needs no machinery of its own: every declaration emits a URL (or bare route) that
//! the platform's shipped deep-link intake already delivers — `day_spec::route_of_url` →
//! `request_route`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::meta::Project;

/// A shortcut with its label resolved in every app locale.
pub struct Resolved {
    /// The declared route (query params allowed).
    pub route: String,
    /// The per-platform identifier: `day_shortcut_<index>` in declaration order — the prefix is
    /// the ownership marker everywhere these land, exactly like `day_perm_reason_`.
    pub id: String,
    /// Label text per locale dir name (`en`, `fr`, `zh-CN`, …) — complete by construction:
    /// [`resolved`] errors on a missing translation rather than shipping a mixed-language menu.
    pub labels: BTreeMap<String, String>,
    /// The default-locale (`en`) label — the base value platforms fall back to.
    pub base: String,
}

/// The default locale: the one whose text goes into the base carrier (Info.plist title,
/// Android `values/`, HarmonyOS `base/`). Matches the scaffold's starter locale.
const DEFAULT_LOCALE: &str = "en";

/// Resolve every `[[shortcuts]]` entry against `resource/locales/`. Empty manifest → empty vec,
/// no filesystem touched.
pub fn resolved(project: &Project) -> Result<Vec<Resolved>, String> {
    let shortcuts = &project.manifest.shortcuts;
    if shortcuts.is_empty() {
        return Ok(Vec::new());
    }
    let root = project.root.join("resource/locales");
    let mut locales: Vec<String> = std::fs::read_dir(&root)
        .map_err(|e| format!("[[shortcuts]] needs {}: {e}", root.display()))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    locales.sort();
    if !locales.iter().any(|l| l == DEFAULT_LOCALE) {
        return Err(format!(
            "[[shortcuts]] labels resolve from resource/locales/, which has no {DEFAULT_LOCALE}/"
        ));
    }
    let mut out = Vec::new();
    for (i, s) in shortcuts.iter().enumerate() {
        let mut labels = BTreeMap::new();
        for loc in &locales {
            match ftl_value(&root.join(loc), &s.label)? {
                Some(v) => {
                    labels.insert(loc.clone(), v);
                }
                None => {
                    return Err(format!(
                        "[[shortcuts]] label `{}` is missing from resource/locales/{loc}/ — \
                         shortcut labels must be translated in every locale",
                        s.label
                    ));
                }
            }
        }
        let base = labels.get(DEFAULT_LOCALE).cloned().unwrap_or_default();
        out.push(Resolved {
            route: s.route.clone(),
            id: format!("day_shortcut_{i}"),
            labels,
            base,
        });
    }
    Ok(out)
}

/// Find a Fluent message's value in a locale dir's `*.ftl` files. Only simple single-line
/// static messages qualify — a placeable or a continuation line is an error, not a skip,
/// because the native carriers hold plain strings the OS renders with no formatter behind them.
fn ftl_value(dir: &Path, key: &str) -> Result<Option<String>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ftl"))
        .collect();
    files.sort();
    for file in files {
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            let Some(rest) = line.strip_prefix(key) else {
                continue;
            };
            let Some(value) = rest.trim_start().strip_prefix('=') else {
                continue;
            };
            let value = value.trim();
            let continued = lines
                .peek()
                .is_some_and(|n| !n.trim().is_empty() && n.starts_with([' ', '\t']));
            if value.is_empty() || continued {
                return Err(format!(
                    "{}: `{key}` is a multi-line message — shortcut labels must be single-line",
                    file.display()
                ));
            }
            if value.contains('{') {
                return Err(format!(
                    "{}: `{key}` uses a placeable — shortcut labels must be static text",
                    file.display()
                ));
            }
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ===========================================================================
// Android — staged res files + the overlay-manifest fragment
// ===========================================================================

/// The `resource/locales/` dir name → Android `values*` resource qualifier.
fn android_values_dir(locale: &str) -> String {
    if locale == DEFAULT_LOCALE {
        return "values".to_string();
    }
    let parts: Vec<&str> = locale.split('-').collect();
    match parts.as_slice() {
        [lang, region] if region.len() == 2 => format!("values-{lang}-r{region}"),
        [lang] => format!("values-{lang}"),
        // Script subtags and anything longer take the unambiguous BCP-47 form.
        parts => format!("values-b+{}", parts.join("+")),
    }
}

/// Stage `xml/day_shortcuts.xml` + per-locale string resources into
/// `build/day/android/res`. Runs AFTER `resources::android::stage`, which wipes that tree on
/// builds that have resources to stage — and clears its own files first because builds with
/// no resources don't.
pub fn sync_android(project: &Project) -> Result<(), String> {
    let manifest = project
        .root
        .join("platform/android/app/src/main/AndroidManifest.xml");
    if !manifest.exists() {
        return Ok(());
    }
    let res = project.root.join("build/day/android/res");
    // Remove every file this sync owns, then regenerate — a shortcut removed from Day.toml
    // must not linger in a res tree the image stage had no reason to wipe.
    if let Ok(rd) = std::fs::read_dir(&res) {
        for dir in rd.flatten() {
            let stale = dir.path().join("day_shortcuts.xml");
            let _ = std::fs::remove_file(stale);
        }
    }
    let shortcuts = resolved(project)?;
    if shortcuts.is_empty() {
        return Ok(());
    }
    let text =
        std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let scheme = android_scheme(&text).ok_or_else(|| {
        format!(
            "[[shortcuts]] needs a deep-link scheme, and {} has no android:scheme in an \
             intent-filter (docs/deep-links.md)",
            manifest.display()
        )
    })?;
    let activity = android_launcher_activity(&text)
        .ok_or_else(|| format!("{}: no LAUNCHER activity found", manifest.display()))?;
    let app_id = project.manifest.resolve("android-mdc").id;

    let xml_dir = res.join("xml");
    std::fs::create_dir_all(&xml_dir).map_err(|e| format!("mkdir {}: {e}", xml_dir.display()))?;
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!-- Generated by `day build` from Day.toml [[shortcuts]]. Do not edit. -->\n\
         <shortcuts xmlns:android=\"http://schemas.android.com/apk/res/android\">\n",
    );
    for sc in &shortcuts {
        s.push_str(&format!(
            "    <shortcut\n        android:shortcutId=\"{id}\"\n        \
             android:enabled=\"true\"\n        \
             android:shortcutShortLabel=\"@string/{id}\">\n        \
             <intent\n            android:action=\"android.intent.action.VIEW\"\n            \
             android:data=\"{data}\"\n            \
             android:targetPackage=\"{app_id}\"\n            \
             android:targetClass=\"{activity}\" />\n    </shortcut>\n",
            id = sc.id,
            data = xml_escape(&format!("{scheme}://{}", sc.route)),
        ));
    }
    s.push_str("</shortcuts>\n");
    std::fs::write(xml_dir.join("day_shortcuts.xml"), s).map_err(|e| e.to_string())?;

    // One values*/day_shortcuts.xml per locale; the file name is the ownership marker.
    let locales: Vec<String> = shortcuts[0].labels.keys().cloned().collect();
    for loc in &locales {
        let dir = res.join(android_values_dir(loc));
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n");
        for sc in &shortcuts {
            let label = sc.labels.get(loc).unwrap_or(&sc.base);
            s.push_str(&format!(
                "    <string name=\"{}\">{}</string>\n",
                sc.id,
                xml_escape(label)
            ));
        }
        s.push_str("</resources>\n");
        std::fs::write(dir.join("day_shortcuts.xml"), s).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The `<activity>` fragment the day-pieces overlay manifest carries when shortcuts are
/// declared: AGP's manifest merger matches the activity by name and merges the `<meta-data>`
/// into the app manifest's own element. `None` when the app declares no shortcuts or ships no
/// Android platform dir.
pub fn android_manifest_fragment(project: &Project) -> Option<String> {
    if project.manifest.shortcuts.is_empty() {
        return None;
    }
    let manifest = project
        .root
        .join("platform/android/app/src/main/AndroidManifest.xml");
    let text = std::fs::read_to_string(manifest).ok()?;
    let activity = android_launcher_activity(&text)?;
    Some(format!(
        "<!-- Day.toml [[shortcuts]] — merged into the launcher activity by name. -->\n\
         <activity android:name=\"{activity}\">\n    \
         <meta-data android:name=\"android.app.shortcuts\" \
         android:resource=\"@xml/day_shortcuts\" />\n</activity>"
    ))
}

/// The first `android:scheme` in the manifest — the scaffold's deep-link intent-filter.
fn android_scheme(manifest: &str) -> Option<String> {
    let at = manifest.find("android:scheme=\"")?;
    let rest = &manifest[at + "android:scheme=\"".len()..];
    rest.split('"').next().map(String::from)
}

/// The `android:name` of the activity whose intent-filter carries `LAUNCHER`.
fn android_launcher_activity(manifest: &str) -> Option<String> {
    for chunk in manifest.split("<activity").skip(1) {
        let body = chunk.split("</activity>").next().unwrap_or(chunk);
        if !body.contains("android.intent.category.LAUNCHER") {
            continue;
        }
        let at = body.find("android:name=\"")?;
        let rest = &body[at + "android:name=\"".len()..];
        return rest.split('"').next().map(String::from);
    }
    None
}

// ===========================================================================
// iOS — committed Info.plist items + staged InfoPlist.strings
// ===========================================================================

/// The `resource/locales/` dir name → `.lproj` name. Apple bundles spell Chinese by script,
/// not region; everything else passes through.
fn lproj_name(locale: &str) -> &str {
    match locale {
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        other => other,
    }
}

/// The scheme the iOS scaffold registered (`CFBundleURLSchemes`), read from the committed
/// Info.plist so conveyance can't drift from registration.
fn ios_scheme(plist: &str) -> Option<String> {
    let at = plist.find("CFBundleURLSchemes")?;
    let rest = &plist[at..];
    let s = rest.find("<string>")?;
    let rest = &rest[s + "<string>".len()..];
    rest.split('<').next().map(str::trim).map(String::from)
}

/// Write `UIApplicationShortcutItems` into the committed Info.plist — same editor, same
/// idempotence story as the permission keys. The item title is the DEFAULT-locale text: that
/// string doubles as the lookup key in the staged `InfoPlist.strings`, so an unlocalized
/// device falls back to readable text instead of a raw key.
pub fn sync_ios(project: &Project, plist_path: &Path) -> Result<(), String> {
    if !plist_path.exists() {
        return Ok(());
    }
    let shortcuts = resolved(project)?;
    let before = std::fs::read_to_string(plist_path)
        .map_err(|e| format!("{}: {e}", plist_path.display()))?;
    let items: Vec<Vec<(String, String)>> = shortcuts
        .iter()
        .map(|sc| {
            // The type string is the saved deep link itself; the intake maps it through
            // `route_of_url`, which passes a bare route through when no scheme is registered.
            let link = match ios_scheme(&before) {
                Some(scheme) => format!("{scheme}://{}", sc.route),
                None => sc.route.clone(),
            };
            vec![
                ("UIApplicationShortcutItemType".to_string(), link),
                (
                    "UIApplicationShortcutItemTitle".to_string(),
                    sc.base.clone(),
                ),
            ]
        })
        .collect();
    let value = (!items.is_empty()).then_some(items.as_slice());
    let after = crate::plist::apply_dict_array_key(&before, "UIApplicationShortcutItems", value)?;
    if after != before {
        std::fs::write(plist_path, after).map_err(|e| format!("{}: {e}", plist_path.display()))?;
    }
    Ok(())
}

/// Stage per-locale `<loc>.lproj/InfoPlist.strings` into the built bundle — called by
/// `day xcode-backend stage-strings` from the scaffold's script phase, which runs before
/// code signing so the files are sealed with everything else. Clears its own files first so
/// a removed shortcut (or locale) doesn't linger across incremental builds.
pub fn stage_ios_strings(project: &Project, bundle: &Path) -> Result<(), String> {
    if let Ok(rd) = std::fs::read_dir(bundle) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|x| x == "lproj") {
                let _ = std::fs::remove_file(p.join("InfoPlist.strings"));
                // Leave the dir: other tooling may own siblings in it.
                let _ = std::fs::remove_dir(&p);
            }
        }
    }
    let shortcuts = resolved(project)?;
    if shortcuts.is_empty() {
        return Ok(());
    }
    let locales: Vec<String> = shortcuts[0].labels.keys().cloned().collect();
    for loc in &locales {
        if loc == DEFAULT_LOCALE {
            continue; // the Info.plist value IS the default-locale text
        }
        let dir = bundle.join(format!("{}.lproj", lproj_name(loc)));
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let mut s = String::from(
            "/* Generated by `day build` from Day.toml [[shortcuts]]. Do not edit. */\n",
        );
        for sc in &shortcuts {
            let label = sc.labels.get(loc).unwrap_or(&sc.base);
            s.push_str(&format!(
                "\"{}\" = \"{}\";\n",
                strings_escape(&sc.base),
                strings_escape(label)
            ));
        }
        std::fs::write(dir.join("InfoPlist.strings"), s).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn strings_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Ensure the iOS scaffold's pbxproj carries the `stage-strings` script phase — scaffolds
/// generated before it existed get it injected once, anchored on the template's deterministic
/// object ids. A hand-restructured project that lost the anchors gets instructions instead of
/// a half-edit.
pub fn ensure_ios_strings_phase(project: &Project) -> Result<(), String> {
    if project.manifest.shortcuts.is_empty() {
        return Ok(());
    }
    let pbxproj = project
        .root
        .join("platform/ios/DayApp.xcodeproj/project.pbxproj");
    if !pbxproj.exists() {
        return Ok(());
    }
    let before =
        std::fs::read_to_string(&pbxproj).map_err(|e| format!("{}: {e}", pbxproj.display()))?;
    if before.contains("xcode-backend stage-strings") {
        return Ok(());
    }
    let phase_ref = "\t\t\t\tDA0000000000000000000043 /* Resources */,";
    let section_end = "/* End PBXShellScriptBuildPhase section */";
    if !before.contains(phase_ref)
        || !before.contains(section_end)
        || before.contains("DA0000000000000000000044")
    {
        return Err(format!(
            "{}: can't add the `Stage Day Strings` build phase automatically — add a Run \
             Script phase running `\"${{DAY_BIN:-day}}\" xcode-backend stage-strings` \
             (docs/deep-links.md)",
            pbxproj.display()
        ));
    }
    let block = "\t\tDA0000000000000000000044 /* Stage Day Strings */ = {\n\
         \t\t\tisa = PBXShellScriptBuildPhase;\n\
         \t\t\talwaysOutOfDate = 1;\n\
         \t\t\tbuildActionMask = 2147483647;\n\
         \t\t\tfiles = (\n\t\t\t);\n\
         \t\t\tinputPaths = (\n\t\t\t);\n\
         \t\t\tname = \"Stage Day Strings\";\n\
         \t\t\toutputPaths = (\n\t\t\t);\n\
         \t\t\trunOnlyForDeploymentPostprocessing = 0;\n\
         \t\t\tshellPath = /bin/sh;\n\
         \t\t\tshellScript = \"\\\"${DAY_BIN:-day}\\\" xcode-backend stage-strings\\n\";\n\
         \t\t};\n";
    let after = before
        .replace(
            phase_ref,
            &format!("{phase_ref}\n\t\t\t\tDA0000000000000000000044 /* Stage Day Strings */,"),
        )
        .replace(section_end, &format!("{block}{section_end}"));
    std::fs::write(&pbxproj, after).map_err(|e| format!("{}: {e}", pbxproj.display()))?;
    crate::ops::status(
        "Adding",
        "iOS `Stage Day Strings` build phase (shortcut labels)",
    );
    Ok(())
}

// ===========================================================================
// HarmonyOS — profile JSON + $string entries (module.json5 edits live in ohos.rs)
// ===========================================================================

/// The `resource/locales/` dir name → HarmonyOS resource qualifier dir (`base` for the
/// default locale, dashes to underscores otherwise: `zh-CN` → `zh_CN`).
pub fn harmony_resource_dir(locale: &str) -> String {
    if locale == DEFAULT_LOCALE {
        "base".to_string()
    } else {
        locale.replace('-', "_")
    }
}

/// The `shortcuts_config.json` profile content. The want carries the deep link in
/// `parameters["day.uri"]` — the ability forwards it through the same `deepLink` shim call a
/// `uris`-skill launch uses, so activation is one rail regardless of temperature.
pub fn harmony_shortcuts_config(
    shortcuts: &[Resolved],
    scheme: Option<&str>,
    bundle: &str,
    module: &str,
    ability: &str,
) -> String {
    let mut items = Vec::new();
    for sc in shortcuts {
        let link = match scheme {
            Some(s) => format!("{s}://{}", sc.route),
            None => sc.route.clone(),
        };
        items.push(serde_json::json!({
            "shortcutId": sc.id,
            "label": format!("$string:{}", sc.id),
            "wants": [{
                "bundleName": bundle,
                "moduleName": module,
                "abilityName": ability,
                "parameters": { "day.uri": link },
            }],
        }));
    }
    let doc = serde_json::json!({ "shortcuts": items });
    // hvigor accepts any valid JSON; pretty so a human can read the checked-in file.
    let mut out = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_values_dirs() {
        assert_eq!(android_values_dir("en"), "values");
        assert_eq!(android_values_dir("fr"), "values-fr");
        assert_eq!(android_values_dir("zh-CN"), "values-zh-rCN");
        assert_eq!(android_values_dir("zh-Hans-CN"), "values-b+zh+Hans+CN");
    }

    #[test]
    fn lproj_names() {
        assert_eq!(lproj_name("fr"), "fr");
        assert_eq!(lproj_name("zh-CN"), "zh-Hans");
        assert_eq!(lproj_name("pt-BR"), "pt-BR");
    }

    #[test]
    fn manifest_scrapes() {
        let manifest = r#"
            <activity android:name="dev.daybrite.day.bridge.DayActivity" android:exported="true">
                <intent-filter>
                    <action android:name="android.intent.action.MAIN" />
                    <category android:name="android.intent.category.LAUNCHER" />
                </intent-filter>
                <intent-filter>
                    <data android:scheme="dayshowcase" />
                </intent-filter>
            </activity>"#;
        assert_eq!(
            android_launcher_activity(manifest).as_deref(),
            Some("dev.daybrite.day.bridge.DayActivity")
        );
        assert_eq!(android_scheme(manifest).as_deref(), Some("dayshowcase"));
    }

    #[test]
    fn ftl_lookup_rules() {
        let dir = std::env::temp_dir().join(format!("day-ftl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("app.ftl"),
            "plain = Menus & dialogs\nmulti =\n    one\n    two\nvar = Hello { $name }\n",
        )
        .unwrap();
        assert_eq!(
            ftl_value(&dir, "plain").unwrap().as_deref(),
            Some("Menus & dialogs")
        );
        assert!(ftl_value(&dir, "multi").is_err());
        assert!(ftl_value(&dir, "var").is_err());
        assert_eq!(ftl_value(&dir, "absent").unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn harmony_config_shape() {
        let sc = Resolved {
            route: "menus".into(),
            id: "day_shortcut_0".into(),
            labels: BTreeMap::new(),
            base: "Menus".into(),
        };
        let json =
            harmony_shortcuts_config(&[sc], Some("dayshowcase"), "dev.b", "entry", "EntryAbility");
        let doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        let want = &doc["shortcuts"][0]["wants"][0];
        assert_eq!(want["parameters"]["day.uri"], "dayshowcase://menus");
        assert_eq!(doc["shortcuts"][0]["label"], "$string:day_shortcut_0");
    }
}
