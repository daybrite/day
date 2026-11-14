// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Extension targets in the Xcode 27 JSON5 project (docs/app-extensions.md).
//!
//! Each extension becomes a native target whose sources are a synchronized folder, so files
//! added to `platform/<os>/<Name>/` build without regenerating anything. The Runner target
//! depends on every extension and embeds it with a copy phase: `PlugIns/` for `NSExtension`
//! extensions, `Extensions/` for ExtensionKit ones. Xcode signs each embedded bundle on copy.

use std::path::Path;

use serde_json::{Value, json};

use super::{Family, Os, Resolved};

/// Where the project sits relative to the app, which decides how its paths are written.
#[expect(
    clippy::struct_field_names,
    reason = "each field is a spelling of the project root; the suffix alone would not say of what"
)]
pub struct Layout<'a> {
    /// The project root relative to the project's directory (`SRCROOT`): `../../../..` for the
    /// generated project under `build/day/xcode/<os>/`, `../..` for one under `platform/<os>/`.
    pub root_rel: &'a str,
    /// The project root as a build setting.
    pub root_setting: &'a str,
    /// The project root in a script phase's shell.
    pub root_shell: &'a str,
}

/// The generated project's layout (`build/day/xcode/<os>/DayApp.xcodeproj`).
pub const DERIVED: Layout<'static> = Layout {
    root_rel: "../../../..",
    root_setting: "$(DAY_PROJECT_ROOT)",
    root_shell: "$DAY_PROJECT_ROOT",
};

/// A checked-in project's layout (`platform/<os>/DayApp.xcodeproj`).
pub const PERSISTENT: Layout<'static> = Layout {
    root_rel: "../..",
    root_setting: "$(PROJECT_DIR)/../..",
    root_shell: "$PROJECT_DIR/../..",
};

pub const EMBED_PLUGINS: &str = "Embed Foundation Extensions";
pub const EMBED_EXTENSIONKIT: &str = "Embed ExtensionKit Extensions";

/// Add every extension in `exts` to the project document `value`: its target, its source folder,
/// its product, its packages, and the Runner's dependency and embed phase. Extensions whose
/// target already exists are left as they are, so applying twice changes nothing.
pub fn apply(
    value: &mut Value,
    exts: &[Resolved],
    layout: &Layout<'_>,
    root: &Path,
) -> Result<(), String> {
    let existing: Vec<String> = value["targets"]
        .as_array()
        .ok_or("the project has no targets")?
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    let exts: Vec<&Resolved> = exts
        .iter()
        .filter(|r| !existing.contains(&r.name))
        .collect();
    if exts.is_empty() {
        return Ok(());
    }
    let runner_index = value["targets"]
        .as_array()
        .unwrap()
        .iter()
        .position(|t| t["name"] == "Runner")
        .ok_or("the project has no Runner target to embed extensions in")?;

    // Sources and products.
    {
        let files = value["files"]
            .as_array_mut()
            .ok_or("the project has no files")?;
        let products = files
            .iter()
            .position(|f| f["name"] == "Products")
            .ok_or("the project has no Products group")?;
        for (offset, r) in exts.iter().enumerate() {
            files.insert(products + offset, folder(r, layout, root));
        }
        let products = files.iter_mut().find(|f| f["name"] == "Products").unwrap();
        let children = products["children"]
            .as_array_mut()
            .ok_or("the Products group has no children")?;
        for r in &exts {
            let phase = if r.family.in_extensions_folder() {
                EMBED_EXTENSIONKIT
            } else {
                EMBED_PLUGINS
            };
            children.push(json!({
                "path": format!("<PRODUCTS>/{}", r.product_file()),
                "target-membership": [{
                    "build-phase": format!("Runner/copy/{phase}"),
                    "code-sign-on-copy": true,
                    "header-preservation": "remove-on-copy",
                }],
            }));
        }
    }

    // Packages the extensions link, each listed once.
    for r in &exts {
        for pkg in &r.swift_packages {
            let entry = package_entry(pkg, layout);
            if value.get("packages").is_none() {
                value["packages"] = json!([]);
            }
            let packages = value["packages"].as_array_mut().unwrap();
            if !packages.contains(&entry) {
                packages.push(entry);
            }
        }
    }

    // The targets.
    for r in &exts {
        let target = target(r, layout, root);
        value["targets"].as_array_mut().unwrap().push(target);
    }

    // The Runner: build each extension first, then embed it.
    let runner = &mut value["targets"][runner_index];
    if runner.get("dependencies").is_none() {
        runner["dependencies"] = json!([]);
    }
    for r in &exts {
        let deps = runner["dependencies"].as_array_mut().unwrap();
        let dep = json!(r.name);
        if !deps.contains(&dep) {
            deps.push(dep);
        }
    }
    let phases = runner["build-phases"]
        .as_array_mut()
        .ok_or("the Runner target has no build phases")?;
    let after_resources = phases
        .iter()
        .position(|p| p == "resources" || p["kind"] == "resources")
        .map_or(phases.len(), |i| i + 1);
    let mut insert = after_resources;
    for (needed, name) in [
        (
            exts.iter().any(|r| !r.family.in_extensions_folder()),
            EMBED_PLUGINS,
        ),
        (
            exts.iter().any(|r| r.family.in_extensions_folder()),
            EMBED_EXTENSIONKIT,
        ),
    ] {
        if needed && !phases.iter().any(|p| p["name"] == name) {
            phases.insert(insert, embed_phase(name));
            insert += 1;
        }
    }
    Ok(())
}

fn embed_phase(name: &str) -> Value {
    if name == EMBED_EXTENSIONKIT {
        json!({
            "kind": "copy",
            "name": name,
            "bundle-base-path": "build-products-directory",
            "relative-path": "$(EXTENSIONS_FOLDER_PATH)",
        })
    } else {
        json!({
            "kind": "copy",
            "name": name,
            "bundle-base-path": "plugins-directory",
        })
    }
}

/// The source folder, synchronized, minus what is not a build input: the `Info.plist` and
/// entitlements Day merges, and a `Resources/` folder copied by its own phase.
fn folder(r: &Resolved, layout: &Layout<'_>, root: &Path) -> Value {
    let mut exclusions = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join(&r.sources)) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if name == "Info.plist"
                || path.extension().and_then(|e| e.to_str()) == Some("entitlements")
                || (name == "Resources" && path.is_dir())
            {
                exclusions.push(name);
            }
        }
    }
    exclusions.sort();
    let mut entry = json!({
        "kind": "folder",
        "path": format!("{}/{}", layout.root_rel, r.sources),
        "target-membership": [r.name],
    });
    if !exclusions.is_empty() {
        entry["membership-exceptions"] = json!([{
            "target": r.name,
            "exclusions": exclusions,
        }]);
    }
    entry
}

fn package_entry(pkg: &super::SwiftPackageDecl, layout: &Layout<'_>) -> Value {
    match (&pkg.path, &pkg.url) {
        (Some(path), _) => json!({
            "kind": "local",
            "path": format!("{}/{path}", layout.root_rel),
        }),
        (None, url) => {
            let version = if let Some(v) = &pkg.exact {
                json!({ "version": v })
            } else if let Some(b) = &pkg.branch {
                json!({ "branch": b })
            } else if let Some(r) = &pkg.revision {
                json!({ "revision": r })
            } else {
                json!({ "up-to-next-major-version": pkg.from.as_deref().unwrap_or("0.0.0") })
            };
            json!({
                "kind": "remote",
                "repository": url.as_deref().unwrap_or_default(),
                "version": version,
            })
        }
    }
}

fn target(r: &Resolved, layout: &Layout<'_>, root: &Path) -> Value {
    let root_setting = layout.root_setting;
    let deploy = r.os.deployment_setting();
    let entitlements = format!("{root_setting}/{}", r.staged_entitlements());
    let mut settings = serde_json::Map::new();
    let mut set = |k: &str, v: &str| {
        settings.insert(k.to_string(), Value::String(v.to_string()));
    };
    set("PRODUCT_NAME", &r.name);
    set("PRODUCT_BUNDLE_IDENTIFIER", &r.bundle_setting);
    set(
        "INFOPLIST_FILE",
        &format!("{root_setting}/{}", r.staged_info_plist()),
    );
    set("GENERATE_INFOPLIST_FILE", "NO");
    set("CODE_SIGN_ENTITLEMENTS", &entitlements);
    set("DAY_TARGET_CODE_SIGN_ENTITLEMENTS", &entitlements);
    set(deploy, &r.deployment_target);
    set(&format!("DAY_TARGET_{deploy}"), &r.deployment_target);
    set("SKIP_INSTALL", "YES");
    if r.has_executable() {
        set("APPLICATION_EXTENSION_API_ONLY", "YES");
    }
    match r.os {
        Os::Ios => set(
            "LD_RUNPATH_SEARCH_PATHS",
            "$(inherited) @executable_path/Frameworks @executable_path/../../Frameworks",
        ),
        Os::Macos => {
            set(
                "LD_RUNPATH_SEARCH_PATHS",
                "$(inherited) @executable_path/../Frameworks @executable_path/../../../../Frameworks",
            );
            // Every macOS app extension runs in the App Sandbox; the app itself may not.
            set("ENABLE_APP_SANDBOX", "YES");
            set("DAY_TARGET_ENABLE_APP_SANDBOX", "YES");
        }
    }
    if matches!(r.family, Family::Messages | Family::StickerPack) {
        set("ASSETCATALOG_COMPILER_APPICON_NAME", "iMessage App Icon");
    } else {
        // The Runner's app icon setting is a target setting, but an app may set it project-wide
        // in DayApp.xcconfig, and an extension has no AppIcon set to find.
        set("ASSETCATALOG_COMPILER_APPICON_NAME", "");
    }
    if !r.frameworks.is_empty() {
        let flags: Vec<String> = r
            .frameworks
            .iter()
            .map(|f| format!("-framework {f}"))
            .collect();
        set(
            "OTHER_LDFLAGS",
            &format!("$(inherited) {}", flags.join(" ")),
        );
    }
    for (k, v) in &r.build_settings {
        set(k, v);
    }

    let mut phases: Vec<Value> = if r.has_executable() {
        vec![
            json!("compile-sources"),
            json!("frameworks"),
            json!("resources"),
        ]
    } else {
        vec![json!("resources")]
    };
    if r.resources_dir(root).is_some() {
        phases.push(json!({
            "kind": "script",
            "name": "Copy Resources Folder",
            "shell": "/bin/sh",
            "script": format!(
                "# Generated by `day build` from Day.toml [[extensions]]: the Resources folder keeps its layout.\n\
                 /usr/bin/rsync -a \"{}/{}/Resources/\" \"$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH/\"\n",
                layout.root_shell, r.sources
            ),
            "run-on-every-build": true,
        }));
    }
    let mut target = json!({
        "name": r.name,
        "id": r.name,
        "product": format!("Products/{}", r.product_file()),
        "product-type": r.family.product_type(),
        "build-phases": phases,
        "build-settings": Value::Object(settings),
    });
    let members: Vec<Value> = r
        .swift_packages
        .iter()
        .flat_map(super::SwiftPackageDecl::product_names)
        .map(|product| {
            json!({
                "product-name": product,
                "build-phase": { "build-phase": "frameworks" },
            })
        })
        .collect();
    if !members.is_empty() {
        target["package-product-members"] = Value::Array(members);
    }
    target
}

/// [`apply`] to a checked-in project, as appended elements only: each new file, product, target,
/// package, dependency and phase is added at the end of its array, so the rest of the file (its
/// formatting and comments) is left exactly as it was, and the diff shows only the extension.
pub fn apply_to_document(
    doc: &mut ycode_project::ProjectDocument,
    exts: &[Resolved],
    root: &Path,
) -> Result<(), String> {
    let before = doc.value().clone();
    let mut after = before.clone();
    apply(&mut after, exts, &PERSISTENT, root)?;
    if after == before {
        return Ok(());
    }
    // `(where it is in the file, where it is in the result)`: apply() inserts the new folders
    // before the Products group, so that group's index differs between the two.
    let products = |v: &Value| {
        v["files"]
            .as_array()
            .and_then(|f| f.iter().position(|f| f["name"] == "Products"))
    };
    let runner = |v: &Value| {
        v["targets"]
            .as_array()
            .and_then(|t| t.iter().position(|t| t["name"] == "Runner"))
    };
    let mut pointers: Vec<(String, String)> = ["/files", "/targets", "/packages"]
        .iter()
        .map(|p| (p.to_string(), p.to_string()))
        .collect();
    if let (Some(b), Some(a)) = (products(&before), products(&after)) {
        pointers.push((
            format!("/files/{b}/children"),
            format!("/files/{a}/children"),
        ));
    }
    if let (Some(b), Some(a)) = (runner(&before), runner(&after)) {
        for array in ["build-phases", "dependencies"] {
            pointers.push((
                format!("/targets/{b}/{array}"),
                format!("/targets/{a}/{array}"),
            ));
        }
    }
    for (pointer, in_after) in pointers {
        let Some(new) = after.pointer(&in_after).and_then(Value::as_array) else {
            continue;
        };
        match before.pointer(&pointer).and_then(Value::as_array) {
            // A new array (a Runner with no dependencies yet): set it whole.
            None => doc
                .set(&pointer, Value::Array(new.clone()))
                .map_err(|e| e.to_string())?,
            Some(old) => {
                // An element is new when nothing in the old array has its identity: a target or
                // group by name, a file by path, anything else by value. (The Runner and the
                // Products group change in place, through their own pointers above.)
                let identity = |v: &Value| -> Value {
                    v.get("name")
                        .or_else(|| v.get("path"))
                        .cloned()
                        .unwrap_or_else(|| v.clone())
                };
                let known: Vec<Value> = old.iter().map(identity).collect();
                for item in new.iter().filter(|item| !known.contains(&identity(item))) {
                    doc.set(&format!("{pointer}/-"), item.clone())
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(())
}

/// The extensions a checked-in project lacks a target for, by name.
pub fn missing_targets(project: &ycode_project::Project, exts: &[Resolved]) -> Vec<String> {
    exts.iter()
        .filter(|r| !project.targets.iter().any(|t| t.name() == r.name))
        .map(|r| r.name.clone())
        .collect()
}
