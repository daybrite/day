// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Disposable Android and HarmonyOS project scaffolds. App-owned overrides always win.
use crate::meta::Project;
use std::path::{Path, PathBuf};

/// Only omit known boilerplate from templates; never discard extra Java/ArkTS app sources.
pub fn is_scaffold_file(path: &str) -> bool {
    (path.starts_with("platform/android/") || path.starts_with("platform/harmony/"))
        && path != "platform/android/AndroidManifest.xml"
        && crate::template::builtin_app()
            .iter()
            .any(|f| f.path == path)
}

pub fn android_dir(project: &Project) -> PathBuf {
    let source = project.root.join("platform/android");
    if [
        "settings.gradle.kts",
        "settings.gradle",
        "build.gradle.kts",
        "build.gradle",
        "app/src/main/AndroidManifest.xml",
    ]
    .iter()
    .any(|f| source.join(f).exists())
    {
        source
    } else {
        // Keep the same two-level depth as platform/android. Existing versions of Day's Gradle
        // plugin resolve ../../build/day and ../../resource, so no plugin upgrade is required.
        project.root.join("build/android")
    }
}

pub fn harmony_source(project: &Project) -> PathBuf {
    let source = crate::ohos::harmony_dir(project);
    if source.exists() {
        source
    } else {
        crate::ops::staged_root(project).join("harmony/template")
    }
}

pub fn ensure_android(project: &Project) -> Result<PathBuf, String> {
    warn_ignored_android_config(project);
    let dir = android_dir(project);
    if dir == project.root.join("build/android") {
        write_default(project, "android", &dir, false)?;
    }
    Ok(dir)
}

fn ignored_android_config(project: &Project) -> Option<String> {
    let config = &project.manifest.android;
    let configured = config.declared
        || config.cleartext_traffic
        || config.edge_to_edge
        || !config.cleartext_domains.is_empty()
        || !config.resource_overlays.is_empty();
    (configured && android_dir(project) == project.root.join("platform/android")).then(|| {
        format!(
            "{} has a persistent Gradle project: Day.toml [android] settings are not applied; \
             configure that native project directly or migrate to a generated host (docs/native-project-migration.md)",
            project.root.display()
        )
    })
}

fn warn_ignored_android_config(project: &Project) {
    if let Some(warning) = ignored_android_config(project) {
        eprintln!("day: warning: {warning}");
    }
}

fn check_harmony_layout(project: &Project) -> Result<(), String> {
    if project.root.join("platform/ohos").exists() {
        return Err("platform/ohos is unsupported; use platform/harmony for a customized host, or remove it to use the generated host".into());
    }
    Ok(())
}

pub fn ensure_harmony_source(project: &Project) -> Result<PathBuf, String> {
    check_harmony_layout(project)?;
    let source = harmony_source(project);
    if source == crate::ohos::harmony_dir(project) {
        if !source.join("build-profile.json5").is_file() {
            return Err(format!(
                "{} has no build-profile.json5; run `day project add-target harmony-arkui --persist-native-project` to complete this customized host",
                source.display()
            ));
        }
    } else {
        write_default(project, "harmony", &source, false)?;
    }
    Ok(source)
}

pub fn check(project: &Project, platform: &str) -> Result<(), String> {
    let dir = if platform == "android" {
        warn_ignored_android_config(project);
        let dir = android_dir(project);
        if dir == project.root.join("platform/android") {
            return Ok(());
        }
        dir
    } else {
        check_harmony_layout(project)?;
        let dir = harmony_source(project);
        if dir == crate::ohos::harmony_dir(project) {
            return Ok(());
        }
        dir
    };
    write_default(project, platform, &dir, true)?;
    if platform == "harmony" {
        let staged = crate::ohos::staged_harmony_dir(project);
        for (relative, _) in rendered(project, platform)? {
            let file = staged.join(relative);
            if !file.is_file() {
                return Err(format!("{} is missing — run `day prepare`", file.display()));
            }
        }
    }
    Ok(())
}

/// Render only the platform boilerplate from the built-in template, with JSON-escaped identity.
/// App-specific manifest permissions, resources, and pieces are staged by the existing builders.
fn rendered(project: &Project, platform: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let target = if platform == "android" {
        "android-mdc"
    } else {
        "harmony-arkui"
    };
    let app = project.manifest.resolve(target);
    let escape = |s: &str| {
        let json = serde_json::to_string(s).expect("string is JSON serializable");
        json[1..json.len() - 1].to_owned()
    };
    let context = serde_json::json!({"id": escape(&app.id), "title": escape(&app.title), "scheme": escape(&app.scheme())});
    let prefix = format!("platform/{platform}/");
    let files: Vec<_> = crate::template::builtin_app()
        .into_iter()
        .filter(|f| {
            f.path.starts_with(&prefix)
                && is_scaffold_file(&f.path)
                && !f.path.ends_with("_gitignore")
        })
        .collect();
    let files = crate::template::render(&files, &context).map(|files| {
        files
            .into_iter()
            .map(|(path, bytes)| {
                let bytes = if platform == "harmony"
                    && (path.ends_with("/app.json5") || path.ends_with("/module.json5"))
                {
                    let text = String::from_utf8(bytes).expect("built-in manifest is UTF-8");
                    crate::icon::rewrite_icon_refs(&text)
                        .unwrap_or(text)
                        .into_bytes()
                } else {
                    bytes
                };
                (path.strip_prefix(&prefix).unwrap().to_owned(), bytes)
            })
            .collect::<Vec<_>>()
    })?;
    if platform == "android" {
        customize_android(project, files)
    } else {
        Ok(files)
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn read_resources(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "{}: Android resource overlays must contain regular files, not symlinks",
                path.display()
            ));
        }
        if kind.is_dir() {
            read_resources(root, &path, files)?;
        } else if kind.is_file() {
            files.push((
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            ));
        }
    }
    Ok(())
}

fn customize_android(
    project: &Project,
    files: Vec<(String, Vec<u8>)>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    use std::collections::BTreeMap;
    let config = &project.manifest.android;
    let mut files: BTreeMap<_, _> = files.into_iter().collect();
    for overlay in &config.resource_overlays {
        let path = Path::new(overlay);
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "android.resource-overlays must be relative directories within the app: {overlay:?}"
            ));
        }
        let root = project.root.join(path);
        let canonical = root
            .canonicalize()
            .map_err(|e| format!("{}: {e}", root.display()))?;
        if !canonical.starts_with(project.root.canonicalize().map_err(|e| e.to_string())?) {
            return Err(format!(
                "Android resource overlay escapes the app: {overlay:?}"
            ));
        }
        let mut resources = Vec::new();
        read_resources(&root, &root, &mut resources)?;
        for (path, bytes) in resources {
            files.insert(format!("app/src/main/res/{path}"), bytes);
        }
    }
    let manifest = files
        .get_mut("app/src/main/AndroidManifest.xml")
        .ok_or("Android template is missing app/src/main/AndroidManifest.xml")?;
    let mut text = String::from_utf8(manifest.clone()).map_err(|e| e.to_string())?;
    let mut attributes = String::new();
    if config.cleartext_traffic {
        attributes.push_str(" android:usesCleartextTraffic=\"true\"");
    }
    if !config.cleartext_domains.is_empty() {
        attributes.push_str(" android:networkSecurityConfig=\"@xml/day_network_security\"");
        let mut network = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<network-security-config>\n  <base-config cleartextTrafficPermitted=\"{}\" />\n  <domain-config cleartextTrafficPermitted=\"true\">\n",
            config.cleartext_traffic
        );
        for domain in &config.cleartext_domains {
            if domain.is_empty()
                || domain
                    .bytes()
                    .any(|b| !(b.is_ascii_alphanumeric() || b".-:".contains(&b)))
            {
                return Err(format!(
                    "android.cleartext-domains expects hostnames or IP addresses, not URLs: {domain:?}"
                ));
            }
            network.push_str(&format!(
                "    <domain includeSubdomains=\"false\">{}</domain>\n",
                xml(domain)
            ));
        }
        network.push_str("  </domain-config>\n</network-security-config>\n");
        files.insert(
            "app/src/main/res/xml/day_network_security.xml".into(),
            network.into_bytes(),
        );
    }
    text = text.replacen("<application", &format!("<application{attributes}"), 1);
    if config.edge_to_edge {
        text = text.replace("</application>", "    <meta-data android:name=\"dev.daybrite.day.EDGE_TO_EDGE\" android:value=\"true\" />\n    </application>");
    }
    files.insert("app/src/main/AndroidManifest.xml".into(), text.into_bytes());
    Ok(files.into_iter().collect())
}

fn write_default(project: &Project, platform: &str, dir: &Path, check: bool) -> Result<(), String> {
    let files = rendered(project, platform)?;
    if platform == "android" {
        let resources = dir.join("app/src/main/res");
        if resources.is_dir() {
            let mut existing = Vec::new();
            read_resources(&resources, &resources, &mut existing)?;
            for (relative, _) in existing {
                let relative = format!("app/src/main/res/{relative}");
                if !files.iter().any(|(path, _)| path == &relative) {
                    if check {
                        return Err(format!(
                            "{} is stale — run `day prepare`",
                            dir.join(&relative).display()
                        ));
                    }
                    std::fs::remove_file(dir.join(&relative)).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    for (relative, bytes) in files {
        let file = dir.join(relative);
        if std::fs::read(&file).ok().as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        if check {
            return Err(format!(
                "{} is missing or stale — run `day prepare`",
                file.display()
            ));
        }
        std::fs::create_dir_all(file.parent().unwrap())
            .map_err(|e| format!("{}: {e}", file.display()))?;
        std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

/// Read the persistent manifest or render the generated host's template without writing files.
/// Existing app-owned manifests are never replaced here.
pub fn android_manifest(project: &Project) -> Result<String, String> {
    let path = android_dir(project).join("app/src/main/AndroidManifest.xml");
    if android_dir(project) == project.root.join("platform/android") {
        return std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()));
    }
    let files = rendered(project, "android")?;
    let bytes = &files
        .iter()
        .find(|(p, _)| p == "app/src/main/AndroidManifest.xml")
        .ok_or("Android template is missing app/src/main/AndroidManifest.xml")?
        .1;
    String::from_utf8(bytes.clone()).map_err(|e| e.to_string())
}

/// Android Studio needs identity, sources, resources and the plugin before Gradle sync.
pub fn prepare_android(project: &Project) -> Result<PathBuf, String> {
    let dir = ensure_android(project)?;
    crate::pack::android::write_app_properties(project)?;
    let target = crate::external::find_target(project, "android-mdc")?;
    crate::resources::stage(project, target)?;
    crate::shortcuts::sync_android(project)?;
    crate::pieces::write_android_manifest(project)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(Project);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "day-native-host-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(Project { root, manifest: toml::from_str(
                "schema = 1\n[app]\nid = 'dev.example.fixture'\ntitle = 'A \"quoted\" title'\ntargets = ['android-mdc', 'harmony-arkui']\n"
            ).unwrap() })
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.root);
        }
    }

    #[test]
    fn persistent_android_warns_even_for_explicit_default_policy() {
        let mut fixture = Fixture::new();
        let p = &mut fixture.0;
        let dir = p.root.join("platform/android");
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.gradle");
        std::fs::write(&settings, "synthetic persistent settings").unwrap();
        assert!(ignored_android_config(p).is_none());
        p.manifest = toml::from_str(
            "schema = 1\n[app]\nid = 'dev.example.fixture'\n[android]\ncleartext-traffic = false\n",
        )
        .unwrap();
        let warning = ignored_android_config(p).unwrap();
        assert!(warning.contains("[android] settings are not applied"));
        assert_eq!(ensure_android(p).unwrap(), dir);
        check(p, "android").unwrap();
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            "synthetic persistent settings"
        );
        std::fs::remove_file(settings).unwrap();
        assert!(ignored_android_config(p).is_none());
    }

    #[test]
    fn missing_android_template_manifest_returns_an_error() {
        let fixture = Fixture::new();
        let error = customize_android(&fixture.0, Vec::new()).unwrap_err();
        assert!(error.contains("Android template is missing"));
    }

    #[test]
    fn generated_projects_are_disposable_idempotent_and_checkable() {
        let fixture = Fixture::new();
        let p = &fixture.0;
        for platform in ["android", "harmony"] {
            assert!(check(p, platform).unwrap_err().contains("day prepare"));
        }
        assert!(!p.root.join("build").exists(), "check must never write");
        let android = ensure_android(p).unwrap();
        let harmony = ensure_harmony_source(p).unwrap();
        assert_eq!(android, p.root.join("build/android"));
        assert_eq!(harmony, p.root.join("build/day/harmony/template"));
        assert!(!p.root.join("platform").exists());
        let settings = android.join("settings.gradle.kts");
        let before = std::fs::metadata(&settings).unwrap().modified().unwrap();
        let cache = android.join(".gradle/cache");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, "synthetic cache").unwrap();
        ensure_android(p).unwrap();
        assert_eq!(
            before,
            std::fs::metadata(&settings).unwrap().modified().unwrap()
        );
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), "synthetic cache");
        assert!(
            check(p, "harmony").is_err(),
            "the actual staged project is required too"
        );
        write_default(p, "harmony", &crate::ohos::staged_harmony_dir(p), false).unwrap();
        for platform in ["android", "harmony"] {
            check(p, platform).unwrap();
        }
        std::fs::write(&settings, "synthetic stale settings").unwrap();
        assert!(check(p, "android").is_err());
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            "synthetic stale settings"
        );
        ensure_android(p).unwrap();
        check(p, "android").unwrap();
        let strings: serde_json::Value = serde_json::from_slice(
            &std::fs::read(harmony.join("AppScope/resources/base/element/string.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(strings["string"][0]["value"], "A \"quoted\" title");
        std::fs::remove_dir_all(p.root.join("build")).unwrap();
        ensure_android(p).unwrap();
        ensure_harmony_source(p).unwrap();
        assert!(!p.root.join("platform").exists());
    }

    #[test]
    fn android_manifest_overlay_stays_app_owned_without_opting_into_a_native_project() {
        let fixture = Fixture::new();
        let p = &fixture.0;
        let overlay = p.root.join("platform/android/AndroidManifest.xml");
        std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
        let custom = "<manifest><!-- synthetic app overlay --><application /></manifest>";
        std::fs::write(&overlay, custom).unwrap();
        assert!(!is_scaffold_file("platform/android/AndroidManifest.xml"));
        let dir = ensure_android(p).unwrap();
        assert_eq!(dir, p.root.join("build/android"));
        assert!(!dir.join("AndroidManifest.xml").exists());
        check(p, "android").unwrap();
        std::fs::remove_dir_all(p.root.join("build")).unwrap();
        ensure_android(p).unwrap();
        assert_eq!(std::fs::read_to_string(&overlay).unwrap(), custom);
        std::fs::remove_file(&overlay).unwrap();
        ensure_android(p).unwrap();
        assert!(
            !overlay.exists(),
            "preparation must not recreate an optional app overlay"
        );
    }

    #[test]
    fn obsolete_harmony_directory_is_rejected() {
        let fixture = Fixture::new();
        let p = &fixture.0;
        std::fs::create_dir_all(p.root.join("platform/ohos")).unwrap();
        for result in [ensure_harmony_source(p).map(|_| ()), check(p, "harmony")] {
            assert!(result.unwrap_err().contains("platform/harmony"));
        }
        assert!(!p.root.join("build").exists());
    }

    #[test]
    fn customized_projects_are_preserved() {
        let fixture = Fixture::new();
        let p = &fixture.0;
        for (platform, file) in [
            ("android", "settings.gradle"),
            ("harmony", "build-profile.json5"),
        ] {
            let dir = p.root.join("platform").join(platform);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(file), "synthetic customized project").unwrap();
        }
        assert_eq!(ensure_android(p).unwrap(), p.root.join("platform/android"));
        assert_eq!(
            ensure_harmony_source(p).unwrap(),
            p.root.join("platform/harmony")
        );
        for platform in ["android", "harmony"] {
            check(p, platform).unwrap();
        }
        assert!(!p.root.join("build").exists());
        assert_eq!(
            std::fs::read_to_string(p.root.join("platform/android/settings.gradle")).unwrap(),
            "synthetic customized project"
        );
        assert_eq!(
            std::fs::read_to_string(p.root.join("platform/harmony/build-profile.json5")).unwrap(),
            "synthetic customized project"
        );
    }
    #[test]
    fn android_policy_and_overlays_survive_generation_and_removed_inputs_are_pruned() {
        let mut fixture = Fixture::new();
        let p = &mut fixture.0;
        p.manifest.android = toml::from_str(
            r#"
cleartext-domains = ["127.0.0.1"]
edge-to-edge = true
resource-overlays = ["platform/android/res"]
"#,
        )
        .unwrap();
        let resources = p.root.join("platform/android/res/values");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(
            resources.join("styles.xml"),
            "<resources><!-- custom theme fixture --></resources>",
        )
        .unwrap();
        std::fs::write(resources.join("colors.xml"), "<resources/>").unwrap();
        let dir = ensure_android(p).unwrap();
        assert_eq!(dir, p.root.join("build/android"));
        let manifest = android_manifest(p).unwrap();
        assert!(manifest.contains("dev.daybrite.day.EDGE_TO_EDGE"));
        assert!(manifest.contains("@xml/day_network_security"));
        let policy =
            std::fs::read_to_string(dir.join("app/src/main/res/xml/day_network_security.xml"))
                .unwrap();
        assert!(policy.contains("<base-config cleartextTrafficPermitted=\"false\""));
        assert!(policy.contains("includeSubdomains=\"false\">127.0.0.1</domain>"));
        let output = dir.join("app/src/main/res/values/styles.xml");
        assert_eq!(
            std::fs::read(output).unwrap(),
            std::fs::read(resources.join("styles.xml")).unwrap()
        );
        std::fs::remove_file(resources.join("colors.xml")).unwrap();
        assert!(check(p, "android").unwrap_err().contains("stale"));
        assert!(
            dir.join("app/src/main/res/values/colors.xml").exists(),
            "check is read-only"
        );
        p.manifest.android.cleartext_domains.clear();
        p.manifest.android.cleartext_traffic = true;
        ensure_android(p).unwrap();
        assert!(
            !dir.join("app/src/main/res/xml/day_network_security.xml")
                .exists()
        );
        assert!(!dir.join("app/src/main/res/values/colors.xml").exists());
        assert!(
            android_manifest(p)
                .unwrap()
                .contains("android:usesCleartextTraffic=\"true\"")
        );
        check(p, "android").unwrap();
        p.manifest.android.resource_overlays = vec!["../outside".into()];
        assert!(
            ensure_android(p)
                .unwrap_err()
                .contains("relative directories")
        );
    }
}
