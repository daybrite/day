// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! File associations declared in Day.toml (docs/documents.md): what each platform's package
//! says the app opens, how strongly it claims each type, and the types the app itself defines.
use crate::meta::Project;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileType {
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
    #[serde(default)]
    pub apple_uti: Option<String>,
    /// A Fluent message id (resource/locales) naming the type for people: Finder's Kind
    /// column, Explorer's Type column, a file manager's MIME comment. Unset: `"<EXT> document"`.
    #[serde(default)]
    pub name: Option<String>,
    /// What the app does with these files.
    #[serde(default)]
    pub role: Role,
    /// How strongly the app claims these files against other apps that open them.
    #[serde(default)]
    pub rank: Rank,
    /// The app defines this type: it is the app's own format, declared to the system as such
    /// (an exported UTI on Apple platforms, a shared-mime-info package on Linux). Leave it off for
    /// formats other apps define (PDF, Markdown, PNG): those are imported.
    #[serde(default)]
    pub exported: bool,
    /// The Apple types an exported type is a kind of. Defaults to `public.data` and
    /// `public.content`; add `public.json`, `public.text` or `public.archive` where it applies.
    #[serde(default)]
    pub conforms_to: Vec<String>,
}

/// What the app does with a file type (Apple's `CFBundleTypeRole`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Opens and shows the files.
    #[default]
    Viewer,
    /// Opens, changes and saves them.
    Editor,
    /// Declares the type without offering to open it (an exported type the app only writes).
    None,
}

/// How strongly the app claims a file type (Apple's `LSHandlerRank`, and on Windows whether the
/// installer makes the app the extension's default).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rank {
    /// One of the apps that can open these files; never the default by itself.
    #[default]
    Alternate,
    /// A good default for these files, though another app defines the format.
    Default,
    /// The app defines the format and should open it. The Windows installer registers the app as
    /// the extension's default handler (restoring the previous one on uninstall); Windows still
    /// lets a user's own choice win.
    Owner,
    /// Lists the type without wanting to be chosen for it.
    None,
}

impl Role {
    fn apple(self) -> &'static str {
        match self {
            Role::Viewer => "Viewer",
            Role::Editor => "Editor",
            Role::None => "None",
        }
    }
}

impl Rank {
    fn apple(self) -> &'static str {
        match self {
            Rank::Alternate => "Alternate",
            Rank::Default => "Default",
            Rank::Owner => "Owner",
            Rank::None => "None",
        }
    }
}

/// Each declared type's human-readable name in the app's default locale: its `name` message, or
/// `"<EXT> document"` when it has none.
pub fn descriptions(project: &Project) -> Result<Vec<String>, String> {
    let en = project.root.join("resource/locales/en");
    project
        .manifest
        .file_types
        .iter()
        .map(|t| match &t.name {
            Some(key) => crate::shortcuts::ftl_value(&en, key)?.ok_or_else(|| {
                format!("[[file_types]] name `{key}` is missing from resource/locales/en/")
            }),
            None => Ok(format!(
                "{} document",
                t.extensions
                    .first()
                    .map(|e| e.to_uppercase())
                    .unwrap_or_default()
            )),
        })
        .collect()
}

/// The identifier of type `i`: its declared `apple_uti`, else one minted under the app id. Also
/// what names an exported type on Windows and Linux.
fn type_id(project: &Project, i: usize, t: &FileType) -> String {
    t.apple_uti
        .clone()
        .unwrap_or_else(|| format!("{}.document.{i}", project.manifest.app.id))
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
        let uti_ok = |u: &String| {
            !u.is_empty()
                && u.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        };
        if t.apple_uti.as_ref().is_some_and(|u| !uti_ok(u)) {
            return Err("invalid [[file_types]] apple_uti".into());
        }
        if t.conforms_to.iter().any(|u| !uti_ok(u)) {
            return Err("invalid [[file_types]] conforms_to: give Apple type identifiers such as public.json".into());
        }
        if !t.conforms_to.is_empty() && !t.exported {
            return Err(
                "[[file_types]] conforms_to describes a type the app defines; set exported = true"
                    .into(),
            );
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
    if project.manifest.file_types.is_empty() && project.manifest.url_schemes.is_empty() {
        return None;
    }
    let mut s = String::from(
        "<activity android:name=\"dev.daybrite.day.bridge.DayActivity\" android:exported=\"true\">\n",
    );
    s.push_str(&crate::url_handlers::android(project));
    for t in project
        .manifest
        .file_types
        .iter()
        .filter(|t| t.role != Role::None)
    {
        // An editor also answers EDIT, which is what a file manager's "Edit with" sends.
        let edit = if t.role == Role::Editor {
            "<action android:name=\"android.intent.action.EDIT\"/>"
        } else {
            ""
        };
        for mime in &t.mime_types {
            s.push_str(&format!("<intent-filter><action android:name=\"android.intent.action.VIEW\"/>{edit}<category android:name=\"android.intent.category.DEFAULT\"/><category android:name=\"android.intent.category.BROWSABLE\"/><data android:scheme=\"content\"/><data android:scheme=\"file\"/><data android:scheme=\"http\"/><data android:scheme=\"https\"/><data android:mimeType=\"{}\"/></intent-filter>\n",xml(mime)));
        }
    }
    s.push_str("</activity>");
    Some(s)
}
/// Bring an XML Info.plist's URL-scheme and document-type keys in line with `Day.toml`.
///
/// Only the keys whose value changed are rewritten, through the `crate::plist` text editor, so a
/// build that changes nothing leaves the checked-in file byte for byte as it was.
pub fn sync_apple(project: &Project, path: &Path, ios: bool) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let p = plist::Value::from_reader_xml(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let d = p.as_dictionary().ok_or("Info.plist is not a dictionary")?;
    let mut managed = vec![(
        "CFBundleURLTypes",
        crate::url_handlers::apple(project, d.get("CFBundleURLTypes")),
    )];
    if !project.manifest.file_types.is_empty() || d.contains_key("DayManagedDocumentTypes") {
        managed.extend(document_keys(project, d, ios, &descriptions(project)?));
    }
    let mut out = text.clone();
    for (key, value) in managed {
        if d.get(key) != value.as_ref() {
            out = crate::plist::apply_value_key(&out, key, value.as_ref())?;
        }
    }
    if out != text {
        std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// The document-type keys `Day.toml`'s `[[file_types]]` call for, each with its wanted value
/// (`None` removes the key). App-authored entries in `d` are kept: Day marks its own document
/// types by name (`day.document.<i>`) and lists the type identifiers it declared under
/// `DayManagedDocumentTypes`, so a later build replaces exactly those.
fn document_keys(
    project: &Project,
    d: &plist::Dictionary,
    ios: bool,
    descriptions: &[String],
) -> Vec<(&'static str, Option<plist::Value>)> {
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
    // App-authored declarations survive; Day's own (named in `previous`) are rebuilt below.
    let keep_unmanaged = |key: &str| -> Vec<plist::Value> {
        d.get(key)
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
            .collect()
    };
    let mut imports = keep_unmanaged("UTImportedTypeDeclarations");
    let mut exports = keep_unmanaged("UTExportedTypeDeclarations");
    let mut managed = Vec::new();
    for (i, t) in project.manifest.file_types.iter().enumerate() {
        let uti = type_id(project, i, t);
        let description = descriptions.get(i).cloned().unwrap_or_default();
        let mut item = plist::Dictionary::new();
        item.insert(
            "CFBundleTypeName".into(),
            format!("day.document.{i}").into(),
        );
        item.insert("CFBundleTypeRole".into(), t.role.apple().into());
        item.insert("LSHandlerRank".into(), t.rank.apple().into());
        item.insert(
            "LSItemContentTypes".into(),
            plist::Value::Array(vec![uti.clone().into()]),
        );
        docs.push(plist::Value::Dictionary(item));
        let mut tags = plist::Dictionary::new();
        tags.insert(
            "public.filename-extension".into(),
            plist::Value::Array(t.extensions.iter().cloned().map(Into::into).collect()),
        );
        tags.insert(
            "public.mime-type".into(),
            plist::Value::Array(t.mime_types.iter().cloned().map(Into::into).collect()),
        );
        // An exported type is the app's own format and conforms to what the app says; an
        // imported one is somebody else's, described only so the system can match its files.
        let conforms: Vec<plist::Value> = if !t.exported {
            vec!["public.data".into()]
        } else if t.conforms_to.is_empty() {
            vec!["public.data".into(), "public.content".into()]
        } else {
            t.conforms_to.iter().cloned().map(Into::into).collect()
        };
        let mut decl = plist::Dictionary::new();
        decl.insert("UTTypeIdentifier".into(), uti.clone().into());
        decl.insert("UTTypeDescription".into(), description.into());
        decl.insert("UTTypeConformsTo".into(), plist::Value::Array(conforms));
        decl.insert(
            "UTTypeTagSpecification".into(),
            plist::Value::Dictionary(tags),
        );
        let list = if t.exported {
            &mut exports
        } else {
            &mut imports
        };
        if !list.iter().any(|v| {
            v.as_dictionary().and_then(|d| d.get("UTTypeIdentifier"))
                == decl.get("UTTypeIdentifier")
        }) {
            managed.push(plist::Value::String(uti));
            list.push(plist::Value::Dictionary(decl));
        }
    }
    let declared = !project.manifest.file_types.is_empty();
    let mut keys = vec![
        (
            "CFBundleDocumentTypes",
            (!docs.is_empty()).then(|| plist::Value::Array(docs)),
        ),
        (
            "UTImportedTypeDeclarations",
            (!imports.is_empty()).then(|| plist::Value::Array(imports)),
        ),
        (
            "UTExportedTypeDeclarations",
            (!exports.is_empty()).then(|| plist::Value::Array(exports)),
        ),
        (
            "DayManagedDocumentTypes",
            declared.then(|| plist::Value::Array(managed)),
        ),
    ];
    if ios && declared {
        keys.push(("LSSupportsOpeningDocumentsInPlace", Some(false.into())));
    }
    keys
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
/// The shared-mime-info package for the types the app defines (`exported = true`): installed as
/// `share/mime/packages/<app-id>.xml`, it is what gives a custom extension a MIME type on Linux
/// at all. Without it, `MimeType=` in the `.desktop` file names a type no file ever has. `None`
/// when the app defines no type.
pub fn linux_mime_package(types: &[FileType], descriptions: &[String]) -> Option<String> {
    let mut body = String::new();
    for (i, t) in types.iter().enumerate().filter(|(_, t)| t.exported) {
        for mime in &t.mime_types {
            body.push_str(&format!("  <mime-type type=\"{}\">\n", xml(mime)));
            body.push_str(&format!(
                "    <comment>{}</comment>\n",
                xml(descriptions.get(i).map(String::as_str).unwrap_or_default())
            ));
            for ext in &t.extensions {
                body.push_str(&format!("    <glob pattern=\"*.{}\"/>\n", xml(ext)));
            }
            let parent = if t.conforms_to.iter().any(|c| c == "public.json") {
                "application/json"
            } else if t
                .conforms_to
                .iter()
                .any(|c| c == "public.text" || c == "public.plain-text")
            {
                "text/plain"
            } else if t.conforms_to.iter().any(|c| c == "public.zip-archive") {
                "application/zip"
            } else {
                "application/octet-stream"
            };
            body.push_str(&format!("    <sub-class-of type=\"{parent}\"/>\n"));
            body.push_str("  </mime-type>\n");
        }
    }
    (!body.is_empty()).then(|| {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n{body}</mime-info>\n"
        )
    })
}

pub fn windows_manifest(manifest: String, types: &[FileType], descriptions: &[String]) -> String {
    if types.is_empty() {
        return manifest;
    }
    let mut s = String::from("<Extensions>");
    for (i, t) in types.iter().enumerate() {
        let name = descriptions.get(i).map(String::as_str).unwrap_or_default();
        s.push_str(&format!("<uap:Extension Category=\"windows.fileTypeAssociation\"><uap3:FileTypeAssociation Name=\"daydocument{i}\"><uap:DisplayName>{}</uap:DisplayName><uap:SupportedFileTypes>", xml(name)));
        for e in &t.extensions {
            s.push_str(&format!("<uap:FileType>.{}</uap:FileType>", xml(e)))
        }
        s.push_str("</uap:SupportedFileTypes><uap2:SupportedVerbs><uap3:Verb Id=\"open\" Parameters=\"--day-open-file &quot;%1&quot;\">Open</uap3:Verb></uap2:SupportedVerbs></uap3:FileTypeAssociation></uap:Extension>");
    }
    s.push_str("</Extensions>");
    let manifest = manifest.replace("<Package ", "<Package xmlns:uap2=\"http://schemas.microsoft.com/appx/manifest/uap/windows10/2\" xmlns:uap3=\"http://schemas.microsoft.com/appx/manifest/uap/windows10/3\" IgnorableNamespaces=\"uap uap2 uap3 rescap\" ");
    manifest.replace("    </Application>", &format!("{s}\n    </Application>"))
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

/// Per-user registration: every type lands in the extension's Open With list, and a type the
/// app owns (`rank = "owner"`) also becomes the extension's default handler, with the previous
/// default kept and restored on uninstall. Windows still lets the user's own choice win.
pub fn nsis(
    script: String,
    types: &[FileType],
    descriptions: &[String],
    id: &str,
    exe: &str,
) -> String {
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
        let name = quote(descriptions.get(i).map(String::as_str).unwrap_or_default());
        install.push_str(&format!(
            "  WriteRegStr HKCU \"Software\\Classes\\{prog}\" \"\" \"{name}\"\n  WriteRegStr HKCU \"Software\\Classes\\{prog}\\DefaultIcon\" \"\" '$\\\"$INSTDIR\\{exe}.exe$\\\",0'\n"
        ));
        install.push_str(&format!(r#"  WriteRegStr HKCU "Software\Classes\{prog}\shell\open\command" "" '$\"$INSTDIR\{exe}.exe$\" --day-open-file $\"%1$\"'
"#));
        for ext in &t.extensions {
            install.push_str(&format!("  WriteRegStr HKCU \"Software\\Classes\\.{ext}\\OpenWithProgids\" \"{prog}\" \"\"\n"));
            uninstall.push_str(&format!(
                "  DeleteRegValue HKCU \"Software\\Classes\\.{ext}\\OpenWithProgids\" \"{prog}\"\n"
            ));
            if t.rank == Rank::Owner {
                // Take the extension's default, keeping whatever held it before.
                install.push_str(&format!(
                    "  ReadRegStr $0 HKCU \"Software\\Classes\\.{ext}\" \"\"\n  StrCmp $0 \"{prog}\" +2 0\n  WriteRegStr HKCU \"Software\\Classes\\.{ext}\" \"DayPreviousDefault\" \"$0\"\n  WriteRegStr HKCU \"Software\\Classes\\.{ext}\" \"\" \"{prog}\"\n"
                ));
                // Hand it back on uninstall, but only if the app still holds it.
                uninstall.push_str(&format!(
                    "  ReadRegStr $0 HKCU \"Software\\Classes\\.{ext}\" \"\"\n  StrCmp $0 \"{prog}\" 0 +4\n  ReadRegStr $1 HKCU \"Software\\Classes\\.{ext}\" \"DayPreviousDefault\"\n  WriteRegStr HKCU \"Software\\Classes\\.{ext}\" \"\" \"$1\"\n  DeleteRegValue HKCU \"Software\\Classes\\.{ext}\" \"DayPreviousDefault\"\n"
                ));
            }
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
    if project.manifest.file_types.is_empty() && project.manifest.url_schemes.is_empty() {
        return Ok(binary);
    }
    if target.os == "windows" {
        let dir = crate::ops::staged_root(project)
            .join("file-types")
            .join(target.name);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = crate::url_handlers::windows_registry(
            windows_registry(
                &project.manifest.file_types,
                &project.manifest.resolve(target.name).id,
                &binary,
            ),
            &project.manifest.url_schemes,
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
    replace_executable(&binary, &macos.join(&project.manifest.app.name))?;
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

/// Preserve running processes' executable inode when rebuilding a development bundle.
fn replace_executable(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    let name = destination
        .file_name()
        .ok_or("executable has no filename")?
        .to_string_lossy();
    let staged = destination.with_file_name(format!(".{name}-{}.tmp", std::process::id()));
    std::fs::copy(source, &staged).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::rename(&staged, destination) {
        let _ = std::fs::remove_file(&staged);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rebuilding_bundle_preserves_running_executable_inode() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("day-executable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("new");
        let destination = dir.join("running");
        std::fs::write(&source, b"replacement executable").unwrap();
        std::fs::write(&destination, b"original executable").unwrap();
        let mut running = std::fs::File::open(&destination).unwrap();
        replace_executable(&source, &destination).unwrap();
        let mut original = String::new();
        running.read_to_string(&mut original).unwrap();
        assert_eq!(original, "original executable");
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "replacement executable"
        );
        drop(running);
        std::fs::remove_dir_all(dir).unwrap();
    }
    fn epub() -> Vec<FileType> {
        vec![FileType {
            extensions: vec!["epub".into()],
            mime_types: vec!["application/epub+zip".into()],
            apple_uti: Some("org.idpf.epub-container".into()),
            ..Default::default()
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
            windows_manifest("    </Application>".into(), &t, &["EPUB book".into()])
                .contains("<uap:FileType>.epub</uap:FileType>")
        );
    }
    #[test]
    fn generated_registrations_preserve_other_handlers_and_escape_paths() {
        let types = epub();
        let appx = windows_manifest("<Package >    </Application>".into(), &types, &[]);
        assert!(appx.contains("xmlns:uap3="));
        assert!(appx.contains("<uap2:SupportedVerbs>"));
        assert!(appx.contains("&quot;%1&quot;"));
        assert!(!appx.contains("&amp;quot;"));
        let script = nsis(
            "Section \"Install\"\nSectionEnd\nSection \"Uninstall\"\nSectionEnd".into(),
            &types,
            &[],
            "org.test.reader",
            "reader",
        );
        assert!(script.contains("OpenWithProgids"));
        // Not an owner: the extension's default stays the user's.
        assert!(!script.contains("DayPreviousDefault"));
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
        project.manifest.url_schemes = vec!["feed".into()];
        let path = dir.join("Info.plist");
        let mut doc = plist::Dictionary::new();
        let mut route = plist::Dictionary::new();
        route.insert("CFBundleURLName".into(), "app.route".into());
        route.insert(
            "CFBundleURLSchemes".into(),
            plist::Value::Array(vec!["reader".into()]),
        );
        doc.insert(
            "CFBundleURLTypes".into(),
            plist::Value::Array(vec![route.into()]),
        );
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
        assert_eq!(d["CFBundleURLTypes"].as_array().unwrap().len(), 2);
        project.manifest.url_schemes.clear();
        project.manifest.file_types.clear();
        sync_apple(&project, &path, true).unwrap();
        let value = plist::Value::from_file(&path).unwrap();
        let d = value.as_dictionary().unwrap();
        assert_eq!(d["CFBundleDocumentTypes"].as_array().unwrap().len(), 1);
        assert_eq!(d["CFBundleURLTypes"].as_array().unwrap().len(), 1);
        // Nothing left to declare: the emptied key goes too.
        assert!(!d.contains_key("UTImportedTypeDeclarations"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    /// A build must leave the scaffold's checked-in plist alone: CI asserts a pristine checkout
    /// after building, and reserializing the document reordered its keys and dropped its final
    /// newline in every Day app.
    #[test]
    fn apple_sync_keeps_the_checked_in_plist_byte_for_byte() {
        const SCAFFOLD: &str = include_str!("../templates/app/platform/macos/Runner/Info.plist");
        let dir = std::env::temp_dir().join(format!("day-url-plist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname='url-fixture'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("Day.toml"),
            "schema=1\n[app]\nid='org.test.feeds'\n",
        )
        .unwrap();
        let mut project = crate::meta::find_project(Some(&dir)).unwrap();
        let path = dir.join("Info.plist");
        let order = |text: &str| {
            let value = plist::Value::from_reader_xml(text.as_bytes()).unwrap();
            let keys: Vec<String> = value.as_dictionary().unwrap().keys().cloned().collect();
            keys
        };
        // Both line endings on every platform: a Windows checkout with core.autocrlf holds the
        // scaffold with CRLF, and a rewrite must keep whichever the file has.
        let lf = SCAFFOLD.replace("\r\n", "\n");
        for scaffold in [lf.clone(), lf.replace('\n', "\r\n")] {
            project.manifest.url_schemes.clear();
            std::fs::write(&path, &scaffold).unwrap();
            sync_apple(&project, &path, false).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), scaffold);

            // A scheme rewrites CFBundleURLTypes alone, in place, and a second sync is a no-op.
            project.manifest.url_schemes = vec!["feed".into()];
            sync_apple(&project, &path, false).unwrap();
            let added = std::fs::read_to_string(&path).unwrap();
            assert!(added.contains("<string>day.external-urls</string>"));
            assert!(added.ends_with('\n'));
            assert_eq!(order(&added), order(&scaffold));
            sync_apple(&project, &path, false).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), added);

            // Dropping the scheme restores the scaffold exactly.
            project.manifest.url_schemes.clear();
            sync_apple(&project, &path, false).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), scaffold);
        }
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
    fn daynote() -> Vec<FileType> {
        vec![FileType {
            extensions: vec!["daynote".into()],
            mime_types: vec!["application/x-daynote".into()],
            role: Role::Editor,
            rank: Rank::Owner,
            exported: true,
            conforms_to: vec!["public.json".into()],
            ..Default::default()
        }]
    }
    #[test]
    fn an_owned_type_takes_the_windows_default_and_gives_it_back() {
        let script = nsis(
            "Section \"Install\"\nSectionEnd\nSection \"Uninstall\"\nSectionEnd".into(),
            &daynote(),
            &["Day Note".into()],
            "org.test.notes",
            "notes",
        );
        // Install: keep the old default, then take the extension.
        assert!(script.contains(
            "WriteRegStr HKCU \"Software\\Classes\\.daynote\" \"DayPreviousDefault\" \"$0\""
        ));
        assert!(script.contains(
            "WriteRegStr HKCU \"Software\\Classes\\.daynote\" \"\" \"org.test.notes.document.0\""
        ));
        // The ProgID carries the type's name for Explorer's Type column.
        assert!(script.contains(
            "WriteRegStr HKCU \"Software\\Classes\\org.test.notes.document.0\" \"\" \"Day Note\""
        ));
        // Uninstall restores the previous default only while the app still holds it.
        let uninstall = &script[script.find("Section \"Uninstall\"").unwrap()..];
        assert!(uninstall.contains("StrCmp $0 \"org.test.notes.document.0\" 0 +4"));
        assert!(uninstall.contains(
            "DeleteRegValue HKCU \"Software\\Classes\\.daynote\" \"DayPreviousDefault\""
        ));
    }
    #[test]
    fn an_exported_type_gets_a_linux_mime_package() {
        let package = linux_mime_package(&daynote(), &["Day Note".into()]).unwrap();
        assert!(package.contains("<mime-type type=\"application/x-daynote\">"));
        assert!(package.contains("<comment>Day Note</comment>"));
        assert!(package.contains("<glob pattern=\"*.daynote\"/>"));
        assert!(package.contains("<sub-class-of type=\"application/json\"/>"));
        // Somebody else's format declares no MIME type of its own.
        assert!(linux_mime_package(&epub(), &[]).is_none());
    }
    #[test]
    fn conforms_to_needs_an_exported_type() {
        let mut t = daynote();
        t[0].exported = false;
        assert!(validate(&t).is_err());
        validate(&daynote()).unwrap();
    }
    #[test]
    fn apple_declares_an_owned_type_as_exported_with_its_role_and_rank() {
        let dir = std::env::temp_dir().join(format!("day-document-export-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("resource/locales/en")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname='notes-fixture'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("resource/locales/en/app.ftl"),
            "file-type-note = Day Note\n",
        )
        .unwrap();
        std::fs::write(dir.join("Day.toml"),"schema=1\n[app]\nid='org.test.notes'\n[[file_types]]\nextensions=['daynote']\nmime_types=['application/x-daynote']\nname='file-type-note'\nrole='editor'\nrank='owner'\nexported=true\nconforms_to=['public.json']\n").unwrap();
        let project = crate::meta::find_project(Some(&dir)).unwrap();
        let path = dir.join("Info.plist");
        let mut seed = plist::Dictionary::new();
        seed.insert("CFBundleName".into(), "Notes".into());
        plist::Value::Dictionary(seed).to_file_xml(&path).unwrap();
        sync_apple(&project, &path, false).unwrap();
        let v = plist::Value::from_file(&path).unwrap();
        let d = v.as_dictionary().unwrap();
        let doc = d["CFBundleDocumentTypes"].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap();
        assert_eq!(doc["CFBundleTypeRole"].as_string(), Some("Editor"));
        assert_eq!(doc["LSHandlerRank"].as_string(), Some("Owner"));
        let exported = d["UTExportedTypeDeclarations"].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap();
        assert_eq!(
            exported["UTTypeIdentifier"].as_string(),
            Some("org.test.notes.document.0")
        );
        assert_eq!(exported["UTTypeDescription"].as_string(), Some("Day Note"));
        assert_eq!(
            exported["UTTypeConformsTo"].as_array().unwrap()[0].as_string(),
            Some("public.json")
        );
        assert!(!d.contains_key("UTImportedTypeDeclarations"));
        // Dropping the declaration removes Day's entries and nothing else.
        std::fs::write(
            dir.join("Day.toml"),
            "schema=1\n[app]\nid='org.test.notes'\n",
        )
        .unwrap();
        let project = crate::meta::find_project(Some(&dir)).unwrap();
        sync_apple(&project, &path, false).unwrap();
        let v = plist::Value::from_file(&path).unwrap();
        let d = v.as_dictionary().unwrap();
        assert!(!d.contains_key("UTExportedTypeDeclarations"));
        assert!(!d.contains_key("CFBundleDocumentTypes"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
