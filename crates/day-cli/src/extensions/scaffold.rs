// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day project add-extension`: declare an extension in Day.toml and write its starter sources.

use std::path::Path;

use crate::cli::CliError;
use crate::meta::Project;
use crate::ops::status;

use super::{ExtensionDecl, KINDS, Os, kind_spec, platforms_of, starters, valid_name};

/// The name an extension gets when `--name` is omitted. None of these is the name of a system
/// framework: the name is also the Swift module's, and a module named `Intents` hides Apple's.
pub fn default_name(kind: &str) -> String {
    match kind {
        "widget" => "Widgets",
        "share" => "ShareExtension",
        "action" => "ActionExtension",
        "notification-service" => "NotificationService",
        "notification-content" => "NotificationContent",
        "safari-web-extension" => "SafariExtension",
        "content-blocker" => "ContentBlocker",
        "keyboard" => "Keyboard",
        "intents" => "IntentsExtension",
        "intents-ui" => "IntentsUIExtension",
        "app-intents" => "AppIntentsExtension",
        "file-provider" => "Files",
        "file-provider-ui" => "FileActions",
        "photo-editing" => "PhotoEditing",
        "broadcast-upload" => "BroadcastUpload",
        "broadcast-setup-ui" => "BroadcastSetupUI",
        "call-directory" => "CallDirectory",
        "message-filter" => "MessageFilter",
        "credential-provider" => "CredentialProvider",
        "packet-tunnel" => "PacketTunnel",
        "app-proxy" => "AppProxy",
        "dns-proxy" => "DNSProxy",
        "filter-data" => "FilterData",
        "filter-control" => "FilterControl",
        "quicklook-preview" => "QuickLookPreview",
        "quicklook-thumbnail" => "QuickLookThumbnail",
        "spotlight-import" => "SpotlightImport",
        "spotlight-index" => "SpotlightIndex",
        "imessage" => "MessagesExtension",
        "sticker-pack" => "Stickers",
        "location-push" => "LocationPush",
        "finder-sync" => "FinderSyncExtension",
        "marketplace" => "MarketplaceExtension",
        _ => "Extension",
    }
    .to_string()
}

/// The kinds, one per line, for `--list`.
pub fn list() -> String {
    let width = KINDS.iter().map(|k| k.name.len()).max().unwrap_or(0);
    KINDS
        .iter()
        .map(|k| {
            let platforms = match (k.ios.is_some(), k.macos.is_some()) {
                (true, true) => "ios, macos",
                (true, false) => "ios",
                (false, true) => "macos",
                (false, false) => "any",
            };
            format!("  {:width$}  {} ({platforms})", k.name, k.summary)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Declare an extension of `kind` in Day.toml, write its starter sources for each platform, and
/// add its target to any checked-in Xcode project (the generated one picks it up on the next
/// build by itself).
pub fn add_extension(
    project: &Project,
    kind: &str,
    name: Option<&str>,
    platforms: &[String],
) -> Result<(), CliError> {
    let spec = kind_spec(kind).ok_or_else(|| {
        CliError::usage(format!(
            "unknown extension kind {kind:?}; `day project add-extension --list` shows them"
        ))
    })?;
    if spec.name == "custom" {
        return Err(CliError::usage(
            "a custom extension needs its extension point: add an [[extensions]] table with \
             `kind = \"custom\"` and `point = \"…\"` to Day.toml (docs/app-extensions.md)",
        ));
    }
    let name = name.map_or_else(|| default_name(kind), str::to_string);
    if !valid_name(&name) {
        return Err(CliError::usage(format!(
            "{name:?} cannot name an extension: start with a letter, use ASCII letters, digits \
             and underscores, and avoid \"Runner\" and the names of system frameworks"
        )));
    }
    if project
        .manifest
        .extensions
        .iter()
        .any(|e| e.name.eq_ignore_ascii_case(&name))
    {
        return Err(CliError::usage(format!(
            "Day.toml already declares an extension named {name}; pass --name for another"
        )));
    }
    let app_targets: Vec<Os> = project
        .manifest
        .app
        .targets
        .iter()
        .filter_map(|t| match t.as_str() {
            "ios-uikit" => Some(Os::Ios),
            "macos-appkit" => Some(Os::Macos),
            _ => None,
        })
        .collect();
    let decl = ExtensionDecl {
        name: name.clone(),
        kind: Some(kind.to_string()),
        platforms: (!platforms.is_empty()).then(|| platforms.to_vec()),
        ..Default::default()
    };
    let oses = platforms_of(&decl, &app_targets).map_err(CliError::usage)?;
    if oses.is_empty() {
        return Err(CliError::usage(format!(
            "no platform to add a {kind} extension for: the app targets {} and a {kind} \
             extension exists on {}; add ios-uikit or macos-appkit first, or pass --platform",
            if app_targets.is_empty() {
                "neither ios-uikit nor macos-appkit".to_string()
            } else {
                app_targets
                    .iter()
                    .map(|o| o.target())
                    .collect::<Vec<_>>()
                    .join(" and ")
            },
            spec.platforms()
                .iter()
                .map(|o| o.key())
                .collect::<Vec<_>>()
                .join(" and ")
        )));
    }

    let mut written = 0;
    for os in &oses {
        let dir = project.root.join(format!("platform/{}/{name}", os.key()));
        for (rel, body) in starters::files(kind, *os, &name, &name) {
            let path = dir.join(&rel);
            if path.exists() {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CliError::failure(format!("creating {}: {e}", parent.display()))
                })?;
            }
            std::fs::write(&path, body)
                .map_err(|e| CliError::failure(format!("writing {}: {e}", path.display())))?;
            written += 1;
        }
    }

    let day_toml = project.root.join("Day.toml");
    let text = std::fs::read_to_string(&day_toml)
        .map_err(|e| CliError::failure(format!("reading {}: {e}", day_toml.display())))?;
    let updated = append_declaration(&text, &name, kind, platforms).map_err(CliError::failure)?;
    std::fs::write(&day_toml, updated)
        .map_err(|e| CliError::failure(format!("writing {}: {e}", day_toml.display())))?;

    // A checked-in project is the app's own, so `day build` never rewrites it; this command is
    // the one explicit moment to add the target there.
    let reloaded = crate::meta::find_project(Some(&project.root)).map_err(CliError::failure)?;
    for os in &oses {
        add_to_persistent_project(&reloaded, *os, &name).map_err(CliError::failure)?;
    }

    status(
        "Added",
        &format!(
            "{kind} extension {name} → Day.toml [[extensions]] ({written} starter file(s) under \
             {})",
            oses.iter()
                .map(|o| format!("platform/{}/{name}/", o.key()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    if let Some(hint) = next_step(kind) {
        eprintln!("\n  next: {hint}");
    }
    eprintln!(
        "\n  build it:\n    day build -t {}\n",
        oses.first().map_or("ios-uikit", |o| o.target())
    );
    Ok(())
}

/// What the starter leaves for the developer to decide, said once after it is written.
fn next_step(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "quicklook-preview" | "quicklook-thumbnail" => {
            "list the document types it handles: attributes.QLSupportedContentTypes = [\"…\"]"
        }
        "spotlight-import" => {
            "list the document types it indexes: attributes.CSSupportedContentTypes = [\"…\"]"
        }
        "intents" | "intents-ui" => {
            "replace the placeholder in attributes.IntentsSupported with the intents it handles"
        }
        "notification-content" => {
            "set the notification category it draws: attributes.UNNotificationExtensionCategory"
        }
        "file-provider" | "widget" | "share" | "keyboard" | "credential-provider" => {
            "to share data with the app, add `app-groups = [\"group.<app id>\"]` here and to the \
             app's entitlements"
        }
        "safari-web-extension" => "edit the WebExtensions manifest in Resources/manifest.json",
        "content-blocker" => "edit the rule list in Resources/blockerList.json",
        "marketplace" => {
            "MarketplaceKit needs the com.apple.developer.marketplace.app-installation \
             entitlement on the app, which Apple grants to approved marketplaces"
        }
        _ => return None,
    })
}

/// Append an `[[extensions]]` table to Day.toml through `toml_edit`, so the rest of the file
/// comes back byte-identical.
pub fn append_declaration(
    text: &str,
    name: &str,
    kind: &str,
    platforms: &[String],
) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("Day.toml: {e}"))?;
    let tables = doc
        .entry("extensions")
        .or_insert(toml_edit::Item::ArrayOfTables(
            toml_edit::ArrayOfTables::new(),
        ))
        .as_array_of_tables_mut()
        .ok_or("Day.toml: `extensions` is not an array of tables ([[extensions]])")?;
    let mut table = toml_edit::Table::new();
    table.insert("name", toml_edit::value(name));
    table.insert("kind", toml_edit::value(kind));
    if !platforms.is_empty() {
        let mut arr = toml_edit::Array::new();
        for p in platforms {
            arr.push(p.as_str());
        }
        table.insert("platforms", toml_edit::value(arr));
    }
    // The system refuses to install an app whose Intents extension lists no intent, so the
    // starter names one to replace.
    if matches!(kind, "intents" | "intents-ui") {
        let mut supported = toml_edit::Array::new();
        supported.push("INSendMessageIntent");
        let mut attributes = toml_edit::InlineTable::new();
        attributes.insert("IntentsSupported", supported.into());
        table.insert("attributes", toml_edit::value(attributes));
        if let Some(mut key) = table.key_mut("attributes") {
            key.leaf_decor_mut()
                .set_prefix("# The SiriKit intents it handles: replace with yours.\n");
        }
    }
    if tables.is_empty() {
        table.decor_mut().set_prefix(
            "\n# App extensions, each built as its own target and embedded in the app:\n\
             # https://daybrite.dev/docs/how-to/app-extensions\n",
        );
    } else {
        table.decor_mut().set_prefix("\n");
    }
    tables.push(table);
    Ok(doc.to_string())
}

/// Add `name`'s target to the checked-in `platform/<os>/DayApp.xcodeproj`, when the app has one.
pub fn add_to_persistent_project(project: &Project, os: Os, name: &str) -> Result<(), String> {
    let file = crate::xcode::path(&project.root, os.key());
    if !file.is_file() {
        return Ok(());
    }
    let exts: Vec<_> = super::resolve(project, os)?
        .into_iter()
        .filter(|r| r.name == name)
        .collect();
    let mut doc = crate::xcode::open(&file)?;
    super::project::apply_to_document(&mut doc, &exts, &project.root)?;
    doc.save().map_err(|e| format!("{}: {e}", file.display()))?;
    status("Updated", &display(&file, &project.root));
    Ok(())
}

fn display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
