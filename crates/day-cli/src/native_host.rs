// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Disposable Android and HarmonyOS project scaffolds. App-owned overrides always win.
use crate::meta::Project;
use std::path::{Path, PathBuf};

/// Only omit known boilerplate from templates; never discard extra Java/ArkTS app sources.
pub fn is_scaffold_file(path: &str) -> bool {
    (path.starts_with("platform/android/") || path.starts_with("platform/harmony/"))
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
    let dir = android_dir(project);
    if dir == project.root.join("build/android") {
        write_default(project, "android", &dir, false)?;
    }
    Ok(dir)
}

pub fn ensure_harmony_source(project: &Project) -> Result<PathBuf, String> {
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
        let dir = android_dir(project);
        if dir == project.root.join("platform/android") {
            return Ok(());
        }
        dir
    } else {
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
        .filter(|f| f.path.starts_with(&prefix) && !f.path.ends_with("_gitignore"))
        .collect();
    crate::template::render(&files, &context).map(|files| {
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
            .collect()
    })
}

fn write_default(project: &Project, platform: &str, dir: &Path, check: bool) -> Result<(), String> {
    for (relative, bytes) in rendered(project, platform)? {
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

/// A missing persistent manifest uses the current default, even before preparation (lint and
/// shortcut planning are read-only). Existing app-owned manifests are never replaced here.
pub fn android_manifest(project: &Project) -> Result<String, String> {
    let path = android_dir(project).join("app/src/main/AndroidManifest.xml");
    if android_dir(project) == project.root.join("platform/android") {
        return std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()));
    }
    let files = rendered(project, "android")?;
    let bytes = &files
        .iter()
        .find(|(p, _)| p == "app/src/main/AndroidManifest.xml")
        .unwrap()
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
    fn customized_projects_and_legacy_harmony_are_preserved() {
        let fixture = Fixture::new();
        let p = &fixture.0;
        for (platform, file) in [
            ("android", "settings.gradle"),
            ("ohos", "build-profile.json5"),
        ] {
            let dir = p.root.join("platform").join(platform);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(file), "synthetic customized project").unwrap();
        }
        assert_eq!(ensure_android(p).unwrap(), p.root.join("platform/android"));
        assert_eq!(
            ensure_harmony_source(p).unwrap(),
            p.root.join("platform/ohos")
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
            std::fs::read_to_string(p.root.join("platform/ohos/build-profile.json5")).unwrap(),
            "synthetic customized project"
        );
    }
}
