// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A writable hvigor project, separate from the app's native sources. Copy symlink targets
//! (not links or hard links), so merging generated resources cannot write back into the app.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Preserve native tool caches between builds, but never import them from the source project.
fn cache(path: &Path) -> bool {
    path == Path::new("entry/libs")
        || (path.components().count() <= 2
            && matches!(
                path.file_name().and_then(|s| s.to_str()),
                Some("build" | ".hvigor" | "oh_modules" | "node_modules" | ".preview" | ".cxx")
            ))
}

/// Older CLIs staged these inside platform/harmony. They must be regenerated from today's
/// inputs, not copied back into a fresh build (including assets/locales removed since then).
fn generated(path: &Path) -> bool {
    [
        "entry/src/main/resources/rawfile/day",
        "entry/src/main/ets/daypieces",
        "entry/src/main/ets/daybridge",
        "entry/src/main/ets/day",
        "entry/src/main/cpp/types/libentry",
    ]
    .iter()
    .any(|p| path == Path::new(p))
}

pub(super) fn sync(source: &Path, dest: &Path) -> Result<(), String> {
    copy_tree(source, dest, Path::new(""), &mut BTreeSet::new())
}

fn remove(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let result = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| format!("{}: {e}", path.display()))
}

fn copy_tree(
    source: &Path,
    dest: &Path,
    relative: &Path,
    ancestors: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    let real = source
        .canonicalize()
        .map_err(|e| format!("{}: {e}", source.display()))?;
    if !ancestors.insert(real.clone()) {
        return Err(format!(
            "cyclic HarmonyOS source symlink: {}",
            source.display()
        ));
    }
    let is_dir = real.is_dir();
    if let Ok(meta) = fs::symlink_metadata(dest)
        && (meta.is_symlink() || meta.is_dir() != is_dir)
    {
        remove(dest)?;
    }
    if is_dir {
        fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        let mut expected = BTreeSet::new();
        for entry in fs::read_dir(source).map_err(|e| format!("{}: {e}", source.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let rel = relative.join(&name);
            if cache(&rel) || generated(&rel) || name == ".git" {
                continue;
            }
            copy_tree(&entry.path(), &dest.join(&name), &rel, ancestors)?;
            expected.insert(name);
        }
        for entry in fs::read_dir(dest).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !expected.contains(&entry.file_name()) && !cache(&relative.join(entry.file_name())) {
                remove(&entry.path())?;
            }
        }
    } else {
        let bytes = fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
        if fs::read(dest).ok().as_deref() != Some(bytes.as_slice()) {
            fs::copy(source, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        }
    }
    ancestors.remove(&real);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Project;
    use crate::ohos::{harmony_dir, stage_host, staged_harmony_dir};
    use std::process::Command;

    struct Fixture(Project);

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("day-ohos-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let manifest = crate::meta::parse_manifest(
                r#"schema = 1
[app]
id = "dev.test.staged"
title = "Staged"
targets = ["harmony-arkui"]
[permissions.raw]
ohos = [{ name = "ohos.permission.ACCELEROMETER", when = "inuse" }]
[[shortcuts]]
route = "play"
label = "play_label"
"#,
                "[package]\nname = 'staged'\nversion = '0.1.0'\n",
                None,
            )
            .unwrap();
            let fixture = Self(Project { root, manifest });
            fixture.write(".gitignore", "/build/\n/target/\nCargo.lock\n");
            fixture.write(
                "Cargo.toml",
                "[package]\nname = 'staged'\nversion = '0.1.0'\n[features]\narkui = []\n",
            );
            fixture.write("src/lib.rs", "");
            fixture.write("platform/harmony/build-profile.json5", "{}");
            fixture.write(
                "platform/harmony/AppScope/app.json5",
                "{app:{bundleName:'dev.test.old'}}",
            );
            fixture.write("platform/harmony/entry/src/main/module.json5", "{module:{name:'entry',requestPermissions:[],abilities:[{name:'EntryAbility',skills:[{uris:[{scheme:'old'}]}]}]}}");
            // A hand-written host needs no day-arkui dependency, keeping the fixture offline.
            fixture.write(
                "platform/harmony/entry/src/main/ets/pages/Index.ets",
                "// Custom host\n",
            );
            fixture.write(
                "platform/harmony/entry/src/main/resources/base/element/string.json",
                r#"{"string":[{"name":"custom","value":"Keep me"}]}"#,
            );
            fixture.write(
                "resource/locales/en/app.ftl",
                "permission_ohos_permission_ACCELEROMETER = Tilt to score.\nplay_label = Play\n",
            );
            fixture.write("resource/locales/fr/app.ftl", "permission_ohos_permission_ACCELEROMETER = Inclinez pour marquer.\nplay_label = Jouer\n");
            fixture.write("resource/locales/pt-BR/app.ftl", "permission_ohos_permission_ACCELEROMETER = Incline para marcar.\nplay_label = Jogar\n");
            fixture.write("resource/assets/rules.txt", "Rules");
            fixture
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.0.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn git(&self, args: &[&str]) -> Vec<u8> {
            let result = Command::new("git")
                .args(args)
                .current_dir(&self.0.root)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            result.stdout
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }

    fn strings(project: &Project, locale: &str) -> serde_json::Value {
        let path = staged_harmony_dir(project).join(format!(
            "entry/src/main/resources/{locale}/element/string.json"
        ));
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn native_preparation_keeps_git_pristine_and_packages_localized_resources() {
        let fixture = Fixture::new("pristine");
        fixture.git(&["init", "-q"]);
        fixture.git(&["add", "."]);
        let before = fixture.git(&["status", "--porcelain", "--untracked-files=all"]);
        stage_host(&fixture.0).unwrap();
        let staged = staged_harmony_dir(&fixture.0);
        let module = fs::read_to_string(staged.join("entry/src/main/module.json5")).unwrap();
        assert!(module.contains("$string:day_perm_reason_ohos_permission_accelerometer"));
        assert!(module.contains("ohos.ability.shortcuts"));
        assert!(module.contains("staged"));
        for (locale, reason, label) in [
            ("base", "Tilt to score.", "Play"),
            ("fr", "Inclinez pour marquer.", "Jouer"),
            ("pt_BR", "Incline para marcar.", "Jogar"),
        ] {
            let json = strings(&fixture.0, locale);
            let entries = json["string"].as_array().unwrap();
            assert!(entries.iter().any(|s| s["name"]
                == "day_perm_reason_ohos_permission_accelerometer"
                && s["value"] == reason));
            assert!(
                entries
                    .iter()
                    .any(|s| s["name"] == "day_shortcut_0" && s["value"] == label)
            );
        }
        assert!(
            strings(&fixture.0, "base")["string"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == "custom" && s["value"] == "Keep me")
        );
        assert_eq!(
            fs::read_to_string(staged.join("entry/src/main/resources/rawfile/day/rules.txt"))
                .unwrap(),
            "Rules"
        );
        let first = strings(&fixture.0, "fr");
        stage_host(&fixture.0).unwrap();
        assert_eq!(strings(&fixture.0, "fr"), first);
        assert_eq!(
            fixture.git(&["status", "--porcelain", "--untracked-files=all"]),
            before
        );
        assert!(fixture.git(&["diff", "--exit-code"]).is_empty());
        assert!(
            !harmony_dir(&fixture.0)
                .join("entry/src/main/resources/fr")
                .exists()
        );
    }

    #[test]
    fn restaging_drops_removed_inputs_and_old_translations_but_keeps_native_caches() {
        let mut fixture = Fixture::new("removed");
        fixture.write("platform/harmony/entry/src/main/resources/de/element/string.json", r#"{"string":[{"name":"custom","value":"Behalten"},{"name":"day_perm_reason_old","value":"Stale"},{"name":"day_shortcut_8","value":"Stale"}]}"#);
        fixture.write(
            "platform/harmony/entry/src/main/resources/rawfile/custom/build/notes.txt",
            "Source asset",
        );
        fixture.write(
            "platform/harmony/entry/src/main/resources/rawfile/day/old.txt",
            "Old generated asset",
        );
        fixture.write(
            "platform/harmony/entry/src/main/ets/Removed.ets",
            "// Source\n",
        );
        fixture.write(
            "platform/harmony/entry/build/old.hap",
            "Old source-tree output",
        );
        stage_host(&fixture.0).unwrap();
        let staged = staged_harmony_dir(&fixture.0);
        assert!(!staged.join("entry/build/old.hap").exists());
        assert!(
            !staged
                .join("entry/src/main/resources/rawfile/day/old.txt")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(
                staged.join("entry/src/main/resources/rawfile/custom/build/notes.txt")
            )
            .unwrap(),
            "Source asset"
        );
        fs::create_dir_all(staged.join("entry/build")).unwrap();
        fs::write(staged.join("entry/build/cache"), "Keep").unwrap();
        fs::remove_dir_all(fixture.0.root.join("resource/locales/fr")).unwrap();
        fs::remove_file(fixture.0.root.join("resource/assets/rules.txt")).unwrap();
        fs::remove_file(harmony_dir(&fixture.0).join("entry/src/main/ets/Removed.ets")).unwrap();
        fixture.0.manifest.permissions = Default::default();
        fixture.0.manifest.shortcuts.clear();
        stage_host(&fixture.0).unwrap();
        assert!(!staged.join("entry/src/main/resources/fr").exists());
        assert!(!staged.join("entry/src/main/ets/Removed.ets").exists());
        assert!(
            !staged
                .join("entry/src/main/resources/rawfile/day/rules.txt")
                .exists()
        );
        assert_eq!(
            strings(&fixture.0, "de"),
            serde_json::json!({"string":[{"name":"custom","value":"Behalten"}]})
        );
        assert_eq!(
            fs::read_to_string(staged.join("entry/build/cache")).unwrap(),
            "Keep"
        );
        assert!(
            !fs::read_to_string(staged.join("entry/src/main/module.json5"))
                .unwrap()
                .contains("ohos.permission.ACCELEROMETER")
        );
    }

    #[test]
    fn legacy_host_is_staged_and_release_signing_finds_the_staged_hap() {
        let fixture = Fixture::new("legacy");
        fs::rename(
            fixture.0.root.join("platform/harmony"),
            fixture.0.root.join("platform/ohos"),
        )
        .unwrap();
        stage_host(&fixture.0).unwrap();
        let unsigned =
            staged_harmony_dir(&fixture.0).join("entry/build/entry-default-unsigned.hap");
        fs::create_dir_all(unsigned.parent().unwrap()).unwrap();
        fs::write(&unsigned, "staged hap").unwrap();
        assert_eq!(crate::ohos::find_unsigned_hap(&fixture.0), Some(unsigned));
        assert!(!fixture.0.root.join("platform/harmony").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_resources_are_copied_without_writing_through_and_cycles_fail() {
        let fixture = Fixture::new("links");
        fixture.write(
            "custom-strings/string.json",
            r#"{"string":[{"name":"custom","value":"Keep"}]}"#,
        );
        let source = harmony_dir(&fixture.0);
        let element = source.join("entry/src/main/resources/base/element");
        fs::remove_dir_all(&element).unwrap();
        std::os::unix::fs::symlink(fixture.0.root.join("custom-strings"), &element).unwrap();
        let before = fs::read(element.join("string.json")).unwrap();
        stage_host(&fixture.0).unwrap();
        assert_eq!(fs::read(element.join("string.json")).unwrap(), before);
        assert!(
            !fs::symlink_metadata(
                staged_harmony_dir(&fixture.0).join("entry/src/main/resources/base/element")
            )
            .unwrap()
            .is_symlink()
        );
        std::os::unix::fs::symlink(&source, source.join("cycle")).unwrap();
        assert!(
            stage_host(&fixture.0)
                .unwrap_err()
                .contains("cyclic HarmonyOS source symlink")
        );
    }
}
