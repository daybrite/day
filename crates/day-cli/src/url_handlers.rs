// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Additional protocol associations; registration never forces the user's default choice.
use crate::meta::Project;
/// The `CFBundleURLTypes` value for `Day.toml`'s `url_schemes`: `current` with Day's own
/// `day.external-urls` entry replaced, or `None` once no entry remains. The app's own entries keep
/// their order.
pub fn apple(project: &Project, current: Option<&plist::Value>) -> Option<plist::Value> {
    let mut types = current
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    types.retain(|v| {
        v.as_dictionary()
            .and_then(|d| d.get("CFBundleURLName"))
            .and_then(|v| v.as_string())
            != Some("day.external-urls")
    });
    if !project.manifest.url_schemes.is_empty() {
        let mut entry = plist::Dictionary::new();
        entry.insert("CFBundleURLName".into(), "day.external-urls".into());
        entry.insert("CFBundleTypeRole".into(), "Viewer".into());
        entry.insert(
            "CFBundleURLSchemes".into(),
            plist::Value::Array(
                project
                    .manifest
                    .url_schemes
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect(),
            ),
        );
        types.push(entry.into());
    }
    (!types.is_empty()).then(|| types.into())
}
pub fn android(project: &Project) -> String {
    project.manifest.url_schemes.iter().map(|s| format!("<intent-filter><action android:name=\"android.intent.action.VIEW\"/><category android:name=\"android.intent.category.DEFAULT\"/><category android:name=\"android.intent.category.BROWSABLE\"/><data android:scheme=\"{s}\"/></intent-filter>\n")).collect()
}
pub fn linux(entry: String, schemes: &[String]) -> String {
    if schemes.is_empty() {
        return entry;
    }
    let mut entry = entry;
    if !entry.contains("%U") {
        entry = entry
            .lines()
            .map(|l| {
                if l.starts_with("Exec=") {
                    format!("{l} --day-open-files %U")
                } else {
                    l.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let additions = schemes
        .iter()
        .map(|s| format!("x-scheme-handler/{s};"))
        .collect::<String>();
    if entry.contains("MimeType=") {
        entry = entry
            .lines()
            .map(|l| {
                if l.starts_with("MimeType=") {
                    format!("{l}{additions}")
                } else {
                    l.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    } else {
        entry.push_str(&format!("\nMimeType={additions}\n"));
    }
    entry
}
pub fn windows(text: String, schemes: &[String]) -> String {
    if schemes.is_empty() {
        return text;
    }
    let text = if text.contains("xmlns:uap3=") {
        text
    } else {
        text.replacen(
            "<Package ",
            "<Package xmlns:uap3=\"http://schemas.microsoft.com/appx/manifest/uap/windows10/3\" ",
            1,
        )
    };
    let entries = schemes.iter().map(|s| format!("<uap3:Extension Category=\"windows.protocol\"><uap3:Protocol Name=\"{s}\" Parameters=\"--day-open-url &quot;%1&quot;\"/></uap3:Extension>")).collect::<String>();
    if text.contains("</Extensions>") {
        text.replace("</Extensions>", &format!("{entries}</Extensions>"))
    } else {
        text.replace(
            "    </Application>",
            &format!("<Extensions>{entries}</Extensions>\n    </Application>"),
        )
    }
}

pub fn harmony(text: &str, schemes: &[String]) -> Result<String, String> {
    use crate::json5::{self, Value, get, get_mut, string};
    if schemes.is_empty() {
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
        .ok_or("no EntryAbility")?
        .value;
    if get(ability, "skills").is_none() {
        json5::insert(ability, "skills", json5::parse("[]")?.value)?;
    }
    let uris: Vec<_> = schemes
        .iter()
        .map(|s| serde_json::json!({"scheme":s}))
        .collect();
    let skill = serde_json::json!({"actions":["ohos.want.action.viewData"],"entities":["entity.system.browsable"],"uris":uris});
    json5::push(
        get_mut(ability, "skills").unwrap(),
        json5::parse(&skill.to_string())?.value,
    )?;
    Ok(doc.to_string())
}

/// Browsers permit web+ names, not arbitrary native schemes, for PWA handlers.
pub fn web(mut manifest: serde_json::Value, schemes: &[String]) -> serde_json::Value {
    let handlers: Vec<_> = schemes
        .iter()
        .filter(|s| {
            s.strip_prefix("web+").is_some_and(|name| {
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase())
            })
        })
        .map(|s| serde_json::json!({"protocol":s,"url":"./?day_url=%s"}))
        .collect();
    if !handlers.is_empty() {
        manifest["protocol_handlers"] = handlers.into();
    }
    manifest
}

/// Opt-in Windows association candidates. Do not overwrite a scheme's current default.
pub fn windows_registry(
    mut text: String,
    schemes: &[String],
    id: &str,
    binary: &std::path::Path,
) -> String {
    if schemes.is_empty() {
        return text;
    }
    fn q(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let capabilities = format!("Software\\{id}\\Capabilities");
    let prog = format!("{id}.external-url");
    text.push_str(&format!("\r\n[HKEY_CURRENT_USER\\Software\\RegisteredApplications]\r\n\"{}\"=\"{}\"\r\n\r\n[HKEY_CURRENT_USER\\{capabilities}]\r\n\"ApplicationName\"=\"{}\"\r\n\r\n[HKEY_CURRENT_USER\\{capabilities}\\URLAssociations]\r\n", q(id), q(&capabilities), q(id)));
    for scheme in schemes {
        text.push_str(&format!("\"{}\"=\"{}\"\r\n", q(scheme), q(&prog)));
    }
    text.push_str(&format!("\r\n[HKEY_CURRENT_USER\\Software\\Classes\\{prog}]\r\n\"URL Protocol\"=\"\"\r\n\r\n[HKEY_CURRENT_USER\\Software\\Classes\\{prog}\\shell\\open\\command]\r\n@=\"{}\"\r\n",q(&format!("\"{}\" --day-open-url \"%1\"",binary.display()))));
    text
}

pub fn nsis(text: String, schemes: &[String], id: &str, exe: &str) -> String {
    if schemes.is_empty() {
        return text;
    }
    fn q(s: &str) -> String {
        s.replace('$', "$$").replace('"', "$\\\"")
    }
    let id = q(id);
    let exe = q(exe);
    let caps = format!("Software\\{id}\\Capabilities");
    let prog = format!("{id}.external-url");
    let mut install = format!(
        "  WriteRegStr HKCU \"Software\\RegisteredApplications\" \"{id}\" \"{caps}\"\n  WriteRegStr HKCU \"{caps}\" \"ApplicationName\" \"{id}\"\n  WriteRegStr HKCU \"Software\\Classes\\{prog}\" \"URL Protocol\" \"\"\n"
    );
    install.push_str(&format!(r#"  WriteRegStr HKCU "Software\Classes\{prog}\shell\open\command" "" '$\"$INSTDIR\{exe}.exe$\" --day-open-url $\"%1$\"'
"#));
    for scheme in schemes {
        install.push_str(&format!(
            "  WriteRegStr HKCU \"{caps}\\URLAssociations\" \"{scheme}\" \"{prog}\"\n"
        ));
    }
    let uninstall = format!(
        "  DeleteRegValue HKCU \"Software\\RegisteredApplications\" \"{id}\"\n  DeleteRegKey HKCU \"{caps}\"\n  DeleteRegKey HKCU \"Software\\Classes\\{prog}\"\n"
    );
    text.replace(
        "Section \"Install\"",
        &format!("Section \"Install\"\n{install}"),
    )
    .replace(
        "Section \"Uninstall\"",
        &format!("Section \"Uninstall\"\n{uninstall}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_preserve_existing_types_and_defaults() {
        let schemes = vec!["feed".into(), "web+feed".into()];
        let desktop = linux(
            "[Desktop Entry]\nExec=news --day-open-files %U\nMimeType=application/rss+xml;\n"
                .into(),
            &schemes,
        );
        assert!(desktop.contains(
            "MimeType=application/rss+xml;x-scheme-handler/feed;x-scheme-handler/web+feed;"
        ));
        assert_eq!(desktop.matches("%U").count(), 1);
        let reg = windows_registry(
            String::new(),
            &schemes,
            "dev.news",
            std::path::Path::new("C:\\News App\\news.exe"),
        );
        assert!(reg.contains("URLAssociations"));
        assert!(reg.contains("--day-open-url"));
        assert!(!reg.contains("Classes\\feed]"));
        let installer = nsis(
            "Section \"Install\"\nSectionEnd\nSection \"Uninstall\"\nSectionEnd".into(),
            &schemes,
            "dev.news",
            "news",
        );
        assert!(installer.contains("URLAssociations\" \"feed\""));
        assert!(installer.contains("DeleteRegValue HKCU \"Software\\RegisteredApplications\""));
        let manifest = web(serde_json::json!({"name":"news"}), &schemes);
        assert_eq!(manifest["protocol_handlers"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["protocol_handlers"][0]["protocol"], "web+feed");
        assert_eq!(manifest["protocol_handlers"][0]["url"], "./?day_url=%s");
        let msix = windows(
            "<Application><Extensions><existing/></Extensions></Application>".into(),
            &schemes,
        );
        assert!(msix.contains("<existing/><uap3:Extension"));
    }
}
