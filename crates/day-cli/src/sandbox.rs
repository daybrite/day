// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! One entitlement plan for Xcode builds and both ad-hoc/release packaging signatures.
use crate::{
    cli::Profile,
    meta::{Manifest, Project, UserSelectedFiles},
};
use plist::{Dictionary, Value};
use std::path::PathBuf;

fn merge(
    manifest: &Manifest,
    profile: Profile,
    mut values: Dictionary,
) -> Result<Dictionary, String> {
    let Some(config) = &manifest.sandbox.macos_appkit else {
        return Ok(values);
    };
    let mut put = |key: &str, enabled: bool| -> Result<(), String> {
        let key = format!("com.apple.security.{key}");
        if let Some(existing) = values.get(&key)
            && existing.as_boolean() != Some(enabled)
        {
            return Err(format!(
                "custom entitlements conflict with [sandbox.macos-appkit]: {key}"
            ));
        }
        if enabled {
            values.insert(key, Value::Boolean(true));
        } else {
            values.remove(&key);
        }
        Ok(())
    };
    if config.enabled && profile == Profile::Release {
        put("get-task-allow", false)?;
    }
    put("app-sandbox", config.enabled)?;
    if !config.enabled {
        return Ok(values);
    }
    put(
        "files.user-selected.read-only",
        config.user_selected_files == UserSelectedFiles::ReadOnly,
    )?;
    put(
        "files.user-selected.read-write",
        config.user_selected_files == UserSelectedFiles::ReadWrite,
    )?;
    put("files.bookmarks.app-scope", config.bookmarks)?;
    put("network.client", config.network_client)?;
    put(
        "network.server",
        config.network_server || (profile == Profile::Debug && config.development_network_server),
    )?;
    put("device.camera", config.camera)?;
    put("device.audio-input", config.microphone)?;
    put("personal-information.location", config.location)?;
    put("device.bluetooth", config.bluetooth)?;
    Ok(values)
}

/// The generated file is separate per profile: packing Release cannot reuse Debug grants.
/// Custom entitlements remain usable without the typed sandbox configuration.
pub fn entitlements(project: &Project, profile: Profile) -> Result<Option<PathBuf>, String> {
    let custom = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
        .and_then(|s| s.entitlements.as_ref());
    let values = if let Some(path) = custom {
        Value::from_file(project.root.join(path))
            .map_err(|e| format!("entitlements {path}: {e}"))?
            .into_dictionary()
            .ok_or("entitlements must be a plist dictionary")?
    } else {
        Dictionary::new()
    };
    let values = merge(&project.manifest, profile, values)?;
    if values.is_empty() {
        return Ok(None);
    }
    let dir = project.root.join("build/day/entitlements/macos-appkit");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.entitlements", profile.as_str()));
    let mut bytes = Vec::new();
    Value::Dictionary(values)
        .to_writer_xml(&mut bytes)
        .map_err(|e| e.to_string())?;
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(Some(path))
}

/// Check the actual signed product, not just its input plist. A hand-edited Xcode target
/// must not silently turn a requested sandbox off.
pub fn verify(project: &Project, app: &std::path::Path, profile: Profile) -> Result<(), String> {
    let expected = entitlements(project, profile)?;
    if expected.is_none() && project.manifest.sandbox.macos_appkit.is_none() {
        return Ok(());
    }
    crate::ops::run_capture(
        std::process::Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(app),
        "codesign verify",
    )
    .and_then(|out| {
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    })?;
    let mut output = crate::ops::run_capture(
        std::process::Command::new("codesign")
            .args(["--display", "--entitlements", "-", "--xml"])
            .arg(app),
        "codesign entitlements",
    )?;
    // Older codesign versions emit XML directly and do not recognize --xml.
    if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("unrecognized option")
    {
        output = crate::ops::run_capture(
            std::process::Command::new("codesign")
                .args(["--display", "--entitlements", ":-"])
                .arg(app),
            "codesign entitlements",
        )?;
    }
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let actual = if output.stdout.is_empty() {
        Value::Dictionary(Dictionary::new())
    } else {
        Value::from_reader(std::io::Cursor::new(output.stdout))
            .map_err(|e| format!("signed entitlements: {e}"))?
    };
    let actual = actual
        .as_dictionary()
        .ok_or("signed entitlements are not a dictionary")?;
    // Reject unexpected grants for keys controlled by the typed policy, too.
    merge(&project.manifest, profile, actual.clone())
        .map_err(|e| format!("signed app violates sandbox policy: {e}"))?;
    let Some(expected) = expected else {
        return Ok(());
    };
    let wanted = Value::from_file(expected).map_err(|e| e.to_string())?;
    for (key, value) in wanted
        .as_dictionary()
        .ok_or("invalid generated entitlements")?
    {
        if actual.get(key) != Some(value) {
            return Err(format!(
                "signed app is missing or overrides requested entitlement {key}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(config: &str) -> Manifest {
        crate::meta::parse_manifest(
            &format!("schema=1\n[app]\nid='dev.test.sandbox'\n{config}"),
            "[package]\nname='sandbox-test'\nversion='0.1.0'",
            None,
        )
        .unwrap()
    }
    #[test]
    fn sandbox_profile_policy_and_explicit_off() {
        let m = manifest("[sandbox.macos-appkit]\nenabled=true\nnetwork-client=true");
        let debug = merge(&m, Profile::Debug, Dictionary::new()).unwrap();
        let release = merge(&m, Profile::Release, Dictionary::new()).unwrap();
        for key in [
            "app-sandbox",
            "network.client",
            "files.user-selected.read-write",
            "files.bookmarks.app-scope",
        ] {
            assert_eq!(
                release.get(&format!("com.apple.security.{key}")),
                Some(&Value::Boolean(true))
            );
        }
        assert_eq!(
            debug.get("com.apple.security.network.server"),
            Some(&Value::Boolean(true))
        );
        assert!(!release.contains_key("com.apple.security.network.server"));
        assert!(!release.contains_key("com.apple.security.device.camera"));
        let off = manifest("[sandbox.macos-appkit]\nenabled=false");
        assert!(
            merge(&off, Profile::Debug, Dictionary::new())
                .unwrap()
                .is_empty()
        );
        assert!(
            merge(&manifest(""), Profile::Debug, Dictionary::new())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn custom_entitlements_are_preserved_but_cannot_silently_override_policy() {
        let mut custom = Dictionary::new();
        custom.insert(
            "com.example.custom".into(),
            Value::Array(vec![Value::String("example".into())]),
        );
        let m = manifest("[sandbox.macos-appkit]\nenabled=true\nuser-selected-files='read-only'");
        let merged = merge(&m, Profile::Release, custom.clone()).unwrap();
        assert!(merged.contains_key("com.apple.security.files.user-selected.read-only"));
        assert!(!merged.contains_key("com.apple.security.files.user-selected.read-write"));
        assert_eq!(
            merged.get("com.example.custom"),
            custom.get("com.example.custom")
        );
        let mut debug_grant = custom.clone();
        debug_grant.insert(
            "com.apple.security.get-task-allow".into(),
            Value::Boolean(true),
        );
        assert!(merge(&m, Profile::Release, debug_grant).is_err());
        custom.insert(
            "com.apple.security.app-sandbox".into(),
            Value::Boolean(false),
        );
        assert!(merge(&m, Profile::Release, custom).is_err());
    }
}
