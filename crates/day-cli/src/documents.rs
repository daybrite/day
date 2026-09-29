// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! File associations declared in Day.toml. Registration never changes the user's default app.
use crate::meta::Project;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileType {
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
    #[serde(default)]
    pub apple_uti: Option<String>,
}
pub fn validate(types: &[FileType]) -> Result<(), String> {
    for t in types {
        if t.extensions.is_empty() || t.mime_types.is_empty() {
            return Err("[[file_types]] requires extensions and mime_types".into());
        }
        if t.extensions.iter().any(|e| {
            e.is_empty()
                || !e
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }) {
            return Err("[[file_types]] extensions must be lowercase letters, digits or hyphens, without a leading dot".into());
        }
        if t.mime_types.iter().any(|m| {
            m.split('/').count() != 2
                || m.split('/').any(str::is_empty)
                || !m
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/!#$&^_.+-".contains(&b))
        }) {
            return Err(
                "[[file_types]] mime_types must be explicit MIME types, without wildcards".into(),
            );
        }
        if t.apple_uti.as_ref().is_some_and(|u| {
            u.is_empty()
                || !u
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        }) {
            return Err("invalid [[file_types]] apple_uti".into());
        }
    }
    Ok(())
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn android(project: &Project) -> Option<String> {
    if project.manifest.file_types.is_empty() {
        return None;
    }
    let mut s = String::from(
        "<activity android:name=\"dev.daybrite.day.bridge.DayActivity\" android:exported=\"true\">\n",
    );
    for t in &project.manifest.file_types {
        for mime in &t.mime_types {
            s.push_str(&format!("<intent-filter><action android:name=\"android.intent.action.VIEW\"/><category android:name=\"android.intent.category.DEFAULT\"/><data android:mimeType=\"{}\"/></intent-filter>\n",xml(mime)));
        }
    }
    s.push_str("</activity>");
    Some(s)
}
pub fn sync_apple(project: &Project, path: &Path, ios: bool) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let mut p = plist::Value::from_file(path).map_err(|e| e.to_string())?;
    let d = p
        .as_dictionary_mut()
        .ok_or("Info.plist is not a dictionary")?;
    if project.manifest.file_types.is_empty() && !d.contains_key("DayManagedDocumentTypes") {
        return Ok(());
    }
    let previous: Vec<String> = d
        .get("DayManagedDocumentTypes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_string().map(str::to_owned))
                .collect()
        })
        .unwrap_or_else(|| {
            d.get("CFBundleDocumentTypes")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter(|v| {
                    v.as_dictionary()
                        .and_then(|d| d.get("CFBundleTypeName"))
                        .and_then(|v| v.as_string())
                        .is_some_and(|s| s.starts_with("day.document."))
                })
                .flat_map(|v| {
                    v.as_dictionary()
                        .and_then(|d| d.get("LSItemContentTypes"))
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                })
                .filter_map(|v| v.as_string().map(str::to_owned))
                .collect()
        });
    let mut docs: Vec<_> = d
        .get("CFBundleDocumentTypes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|v| {
            !v.as_dictionary()
                .and_then(|d| d.get("CFBundleTypeName"))
                .and_then(|v| v.as_string())
                .is_some_and(|s| s.starts_with("day.document."))
        })
        .collect();
    let mut imports: Vec<_> = d
        .get("UTImportedTypeDeclarations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|v| {
            !v.as_dictionary()
                .and_then(|d| d.get("UTTypeIdentifier"))
                .and_then(|v| v.as_string())
                .is_some_and(|s| previous.iter().any(|p| p == s))
        })
        .collect();
    let mut managed_imports = Vec::new();
    for (i, t) in project.manifest.file_types.iter().enumerate() {
        let uti = t
            .apple_uti
            .clone()
            .unwrap_or_else(|| format!("{}.document.{i}", project.manifest.app.id));
        let mut item = plist::Dictionary::new();
        item.insert(
            "CFBundleTypeName".into(),
            format!("day.document.{i}").into(),
        );
        item.insert("CFBundleTypeRole".into(), "Viewer".into());
        item.insert("LSHandlerRank".into(), "Alternate".into());
        item.insert(
            "LSItemContentTypes".into(),
            plist::Value::Array(vec![uti.clone().into()]),
        );
        docs.push(plist::Value::Dictionary(item));
        // Known UTIs are imported, never redefined as owned/exported types.
        let mut tags = plist::Dictionary::new();
        tags.insert(
            "public.filename-extension".into(),
            plist::Value::Array(t.extensions.iter().cloned().map(Into::into).collect()),
        );
        tags.insert(
            "public.mime-type".into(),
            plist::Value::Array(t.mime_types.iter().cloned().map(Into::into).collect()),
        );
        let mut imported = plist::Dictionary::new();
        imported.insert("UTTypeIdentifier".into(), uti.into());
        imported.insert(
            "UTTypeConformsTo".into(),
            plist::Value::Array(vec!["public.data".into()]),
        );
        imported.insert(
            "UTTypeTagSpecification".into(),
            plist::Value::Dictionary(tags),
        );
        if !imports.iter().any(|v| {
            v.as_dictionary().and_then(|d| d.get("UTTypeIdentifier"))
                == imported.get("UTTypeIdentifier")
        }) {
            managed_imports.push(imported["UTTypeIdentifier"].clone());
            imports.push(plist::Value::Dictionary(imported));
        }
    }
    for key in [
        "CFBundleDocumentTypes",
        "UTImportedTypeDeclarations",
        "DayManagedDocumentTypes",
    ] {
        d.remove(key);
    }
    if !docs.is_empty() {
        d.insert("CFBundleDocumentTypes".into(), plist::Value::Array(docs));
    }
    d.insert(
        "UTImportedTypeDeclarations".into(),
        plist::Value::Array(imports),
    );
    if !project.manifest.file_types.is_empty() {
        d.insert(
            "DayManagedDocumentTypes".into(),
            plist::Value::Array(managed_imports),
        );
        if ios {
            d.insert("LSSupportsOpeningDocumentsInPlace".into(), false.into());
        }
    }
    let mut bytes = Vec::new();
    p.to_writer_xml(&mut bytes).map_err(|e| e.to_string())?;
    if std::fs::read(path).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn web_manifest(mut manifest: Value, types: &[FileType]) -> Value {
    if !types.is_empty() {
        let mut accept = serde_json::Map::new();
        for t in types {
            for mime in &t.mime_types {
                let values = accept
                    .entry(mime.clone())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .unwrap();
                for ext in &t.extensions {
                    let value = json!(format!(".{ext}"));
                    if !values.contains(&value) {
                        values.push(value);
                    }
                }
            }
        }
        manifest["file_handlers"] = json!([{"action":"./", "accept":accept}]);
    }
    manifest
}
pub fn linux_entry(entry: String, types: &[FileType]) -> String {
    if types.is_empty() {
        return entry;
    }
    let mut s = entry
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("{line} --day-open-files %U")
            } else {
                line.into()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.push_str("\nMimeType=");
    for t in types {
        for mime in &t.mime_types {
            s.push_str(mime);
            s.push(';')
        }
    }
    s.push('\n');
    s
}
pub fn windows_manifest(manifest: String, types: &[FileType]) -> String {
    if types.is_empty() {
        return manifest;
    }
    let mut s = String::from("<Extensions>");
    for (i, t) in types.iter().enumerate() {
        s.push_str(&format!("<uap:Extension Category=\"windows.fileTypeAssociation\"><uap3:FileTypeAssociation Name=\"daydocument{i}\"><uap:SupportedFileTypes>"));
        for e in &t.extensions {
            s.push_str(&format!("<uap:FileType>.{}</uap:FileType>", xml(e)))
        }
        s.push_str("</uap:SupportedFileTypes><uap2:SupportedVerbs><uap3:Verb Id=\"open\" Parameters=\"--day-open-file &quot;%1&quot;\">Open</uap3:Verb></uap2:SupportedVerbs></uap3:FileTypeAssociation></uap:Extension>");
    }
    s.push_str("</Extensions>");
    let manifest = manifest.replace("<Package ", "<Package xmlns:uap2=\"http://schemas.microsoft.com/appx/manifest/uap/windows10/2\" xmlns:uap3=\"http://schemas.microsoft.com/appx/manifest/uap/windows10/3\" IgnorableNamespaces=\"uap uap2 uap3 rescap\" ");
    manifest.replace("    </Application>", &format!("{s}\n    </Application>"))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn epub() -> Vec<FileType> {
        vec![FileType {
            extensions: vec!["epub".into()],
            mime_types: vec!["application/epub+zip".into()],
            apple_uti: Some("org.idpf.epub-container".into()),
        }]
    }
    #[test]
    fn associations_use_explicit_file_delivery() {
        let t = epub();
        validate(&t).unwrap();
        assert!(linux_entry("Exec=reader\n".into(), &t).contains("--day-open-files %U"));
        assert_eq!(
            web_manifest(json!({}), &t)["file_handlers"][0]["accept"]["application/epub+zip"][0],
            ".epub"
        );
        assert!(
            windows_manifest("    </Application>".into(), &t)
                .contains("<uap:FileType>.epub</uap:FileType>")
        );
    }
    #[test]
    fn generated_registrations_preserve_other_handlers_and_escape_paths() {
        let types = epub();
        let appx = windows_manifest("<Package >    </Application>".into(), &types);
        assert!(appx.contains("xmlns:uap3="));
        assert!(appx.contains("<uap2:SupportedVerbs>"));
        assert!(appx.contains("&quot;%1&quot;"));
        assert!(!appx.contains("&amp;quot;"));
        let script = nsis(
            "Section \"Install\"\nSectionEnd\nSection \"Uninstall\"\nSectionEnd".into(),
            &types,
            "org.test.reader",
            "reader",
        );
        assert!(script.contains("OpenWithProgids"));
        assert!(script.contains("DeleteRegValue"));
        assert!(!script.contains("DeleteRegKey HKCU \"Software\\Classes\\.epub\""));
        let module=harmony_module(r#"{module:{abilities:[{name:'EntryAbility',skills:[{actions:['existing.action']}]}]}}"#,&types).unwrap();
        assert!(module.contains("existing.action"));
        assert!(module.contains("FileOpen"));
        assert!(module.contains("application/epub+zip"));
        let mut duplicate = types.clone();
        duplicate[0].extensions.push("ebook".into());
        let mut combined = types;
        combined.extend(duplicate);
        assert_eq!(
            web_manifest(json!({}), &combined)["file_handlers"][0]["accept"]["application/epub+zip"],
            json!([".epub", ".ebook"])
        );
    }
    #[test]
    fn apple_registration_updates_and_removes_only_managed_entries() {
        let dir = std::env::temp_dir().join(format!("day-document-plist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname='document-fixture'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(dir.join("Day.toml"),"schema=1\n[app]\nid='org.test.reader'\n[[file_types]]\nextensions=['epub']\nmime_types=['application/epub+zip']\napple_uti='org.idpf.epub-container'\n").unwrap();
        let mut project = crate::meta::find_project(Some(&dir)).unwrap();
        let path = dir.join("Info.plist");
        let mut doc = plist::Dictionary::new();
        let mut custom = plist::Dictionary::new();
        custom.insert("CFBundleTypeName".into(), "app-authored".into());
        doc.insert(
            "CFBundleDocumentTypes".into(),
            plist::Value::Array(vec![plist::Value::Dictionary(custom)]),
        );
        plist::Value::Dictionary(doc).to_file_xml(&path).unwrap();
        sync_apple(&project, &path, true).unwrap();
        sync_apple(&project, &path, true).unwrap();
        let value = plist::Value::from_file(&path).unwrap();
        let d = value.as_dictionary().unwrap();
        assert_eq!(d["CFBundleDocumentTypes"].as_array().unwrap().len(), 2);
        assert_eq!(
            d["LSSupportsOpeningDocumentsInPlace"].as_boolean(),
            Some(false)
        );
        project.manifest.file_types.clear();
        sync_apple(&project, &path, true).unwrap();
        let value = plist::Value::from_file(&path).unwrap();
        let d = value.as_dictionary().unwrap();
        assert_eq!(d["CFBundleDocumentTypes"].as_array().unwrap().len(), 1);
        assert!(
            d["UTImportedTypeDeclarations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_wildcards_and_manifest_injection() {
        let mut t = epub();
        t[0].mime_types = vec!["*/*".into()];
        assert!(validate(&t).is_err());
        t = epub();
        t[0].extensions = vec!["epub\"/><evil>".into()];
        assert!(validate(&t).is_err());
    }
}

/// Harmony's staged host is regenerated from the source before this merge, so removing a
/// declaration also removes its generated skill without touching app-authored skills.
pub fn harmony_module(text: &str, types: &[FileType]) -> Result<String, String> {
    use crate::json5::{self, Value, get, get_mut, string};
    if types.is_empty() {
        return Ok(text.into());
    }
    let mut doc = json5::parse(text)?;
    let module = get_mut(&mut doc.value, "module").ok_or("no Harmony module")?;
    let Value::JSONArray { values, .. } = get_mut(module, "abilities").ok_or("no abilities")?
    else {
        return Err("invalid abilities".into());
    };
    let ability = &mut values
        .iter_mut()
        .find(|v| get(&v.value, "name").and_then(string).as_deref() == Some("EntryAbility"))
        .ok_or("no EntryAbility for file types")?
        .value;
    if get(ability, "skills").is_none() {
        json5::insert(ability, "skills", json5::parse("[]")?.value)?;
    }
    let uris: Vec<_> = types
        .iter()
        .flat_map(|t| t.mime_types.iter())
        .map(|mime| json!({"scheme":"file","type":mime,"linkFeature":"FileOpen"}))
        .collect();
    let skill = json!({"actions":["ohos.want.action.viewData"],"uris":uris});
    json5::push(
        get_mut(ability, "skills").unwrap(),
        json5::parse(&skill.to_string())?.value,
    )?;
    Ok(doc.to_string())
}

/// Per-user Open With registration. Never overwrite the extension's default association.
pub fn nsis(script: String, types: &[FileType], id: &str, exe: &str) -> String {
    if types.is_empty() {
        return script;
    }
    fn quote(s: &str) -> String {
        s.replace('$', "$$").replace('"', "$\\\"")
    }
    let id = quote(id);
    let exe = quote(exe);
    let mut install = String::new();
    let mut uninstall = String::new();
    for (i, t) in types.iter().enumerate() {
        let prog = format!("{id}.document.{i}");
        install.push_str(&format!(r#"  WriteRegStr HKCU "Software\Classes\{prog}\shell\open\command" "" '$\"$INSTDIR\{exe}.exe$\" --day-open-file $\"%1$\"'
"#));
        for ext in &t.extensions {
            install.push_str(&format!("  WriteRegStr HKCU \"Software\\Classes\\.{ext}\\OpenWithProgids\" \"{prog}\" \"\"\n"));
            uninstall.push_str(&format!(
                "  DeleteRegValue HKCU \"Software\\Classes\\.{ext}\\OpenWithProgids\" \"{prog}\"\n"
            ));
        }
        uninstall.push_str(&format!(
            "  DeleteRegKey HKCU \"Software\\Classes\\{prog}\"\n"
        ));
    }
    for out in [&mut install, &mut uninstall] {
        out.push_str("  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'\n");
    }
    script
        .replace(
            "Section \"Install\"",
            &format!("Section \"Install\"\n{install}"),
        )
        .replace(
            "Section \"Uninstall\"",
            &format!("Section \"Uninstall\"\n{uninstall}"),
        )
}

/// Development bundles let Finder/Dock deliver file-open events to GTK and Qt too. These
/// reference the build's resource roots and installed toolkit; they are not redistributable.
pub fn desktop_artifact(
    project: &Project,
    target: &'static crate::targets::Target,
    profile: crate::cli::Profile,
    binary: std::path::PathBuf,
) -> Result<std::path::PathBuf, String> {
    if project.manifest.file_types.is_empty() {
        return Ok(binary);
    }
    if target.os == "windows" {
        let dir = crate::ops::staged_root(project)
            .join("file-types")
            .join(target.name);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = windows_registry(
            &project.manifest.file_types,
            &project.manifest.resolve(target.name).id,
            &binary,
        );
        // .reg files are UTF-16LE, including a BOM, for non-ASCII project paths.
        let bytes: Vec<u8> = std::iter::once(0xfeffu16)
            .chain(text.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(dir.join("register.reg"), bytes).map_err(|e| e.to_string())?;
    }
    if target.os != "macos" {
        return Ok(binary);
    }
    let outcome = crate::ops::BuildOutcome {
        target: target.name,
        artifact: binary.clone(),
        seconds: 0.,
    };
    let spec = crate::ops::LaunchSpec {
        locale: None,
        envs: vec![],
        attached: false,
        ios_device: None,
        ios_simulator: None,
        android_device: None,
        ohos_device: None,
    };
    let plan = crate::ops::desktop_launch_plan(project, target, &outcome, &spec)?;
    let app = crate::ops::staged_root(project)
        .join(target.name)
        .join(profile.as_str())
        .join(format!("{}.app", project.manifest.app.name));
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    std::fs::create_dir_all(&macos).map_err(|e| e.to_string())?;
    std::fs::copy(&binary, macos.join(&project.manifest.app.name)).map_err(|e| e.to_string())?;
    let mut d = plist::Dictionary::new();
    for (key, value) in [
        (
            "CFBundleIdentifier",
            project.manifest.resolve(target.name).id,
        ),
        ("CFBundleName", project.manifest.resolve(target.name).title),
        ("CFBundleExecutable", project.manifest.app.name.clone()),
        ("CFBundlePackageType", "APPL".into()),
        (
            "CFBundleShortVersionString",
            project.manifest.app.version.clone(),
        ),
        ("CFBundleVersion", project.manifest.app.build.to_string()),
    ] {
        d.insert(key.into(), value.into());
    }
    let env = plan
        .env
        .into_iter()
        .map(|(k, v)| (k, plist::Value::String(v.to_string_lossy().into_owned())))
        .collect();
    d.insert("LSEnvironment".into(), plist::Value::Dictionary(env));
    let info = contents.join("Info.plist");
    plist::Value::Dictionary(d)
        .to_file_xml(&info)
        .map_err(|e| e.to_string())?;
    sync_apple(project, &info, false)?;
    Ok(app)
}

/// Build-only Windows GTK/Qt targets still get opt-in Open With registration. A build never
/// mutates the registry or changes a user's default app. Distribution installers use `nsis`.
pub fn windows_registry(types: &[FileType], id: &str, binary: &Path) -> String {
    fn q(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n");
    for (i, t) in types.iter().enumerate() {
        let prog = format!("{id}.document.{i}");
        let command = format!("\"{}\" --day-open-file \"%1\"", binary.display());
        out.push_str(&format!("\r\n[HKEY_CURRENT_USER\\Software\\Classes\\{prog}\\shell\\open\\command]\r\n@=\"{}\"\r\n",q(&command)));
        for ext in &t.extensions {
            out.push_str(&format!("\r\n[HKEY_CURRENT_USER\\Software\\Classes\\.{ext}\\OpenWithProgids]\r\n\"{}\"=\"\"\r\n",q(&prog)));
        }
    }
    out
}
