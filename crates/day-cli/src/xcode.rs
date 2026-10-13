// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Xcode 27 JSON5 hosts. Only project.xcproj is supported.
use std::path::{Path, PathBuf};
use ycode_project::{BuildPhase, BuildPhaseKind, BuildSetting, ProjectDocument, Target};

pub fn path(root: &Path, platform: &str) -> PathBuf {
    root.join(format!(
        "platform/{platform}/DayApp.xcodeproj/project.xcproj"
    ))
}

/// A checked-in project is an explicit customization override, using project.xcproj.
/// An incomplete override is an error rather than permission to silently replace it.
pub fn project_dir(root: &Path, platform: &str) -> PathBuf {
    let persistent = path(root, platform).parent().unwrap().to_path_buf();
    if persistent.exists() {
        persistent
    } else {
        root.join(format!("build/day/xcode/{platform}/DayApp.xcodeproj"))
    }
}

pub fn is_project_file(path: &str) -> bool {
    ["macos", "ios"]
        .iter()
        .any(|os| path.starts_with(&format!("platform/{os}/DayApp.xcodeproj/")))
}

/// Derive a relocatable project from Day's template. Source/configuration files stay app-owned;
/// only the project document is generated. All paths are relative to build/day/xcode/<os>/.
fn derived_document(root: &Path, platform: &str) -> Result<ProjectDocument, String> {
    let template = crate::template::builtin_app()
        .into_iter()
        .find(|f| f.path == format!("platform/{platform}/DayApp.xcodeproj/project.xcproj"))
        .ok_or_else(|| format!("no Xcode template for {platform}"))?;
    let doc = ProjectDocument::parse(String::from_utf8(template.bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut value = doc.value().clone();
    let source = format!("../../../../platform/{platform}");
    // Configuration `file` references are navigator names, so preserve each reference's name
    // when rebasing its on-disk path. Children of groups are already relative to their parent.
    for entry in value["files"]
        .as_array_mut()
        .ok_or("missing template files")?
    {
        let Some(old) = entry["path"].as_str().map(str::to_owned) else {
            continue;
        };
        if entry.get("name").is_none() {
            entry["name"] = old.rsplit('/').next().unwrap_or(&old).into();
        }
        entry["path"] = if old == "../.." {
            "../../../..".to_owned()
        } else if let Some(rest) = old.strip_prefix("../../build/day/") {
            format!("../../{rest}")
        } else {
            format!("{source}/{old}")
        }
        .into();
    }
    value["packages"][0]["path"] = format!("../../{platform}/DayPieces").into();
    value["build-settings"]["DAY_PROJECT_ROOT"] = "$(PROJECT_DIR)/../../../..".into();
    value["targets"][0]["build-settings"]["INFOPLIST_FILE"] =
        format!("$(DAY_PROJECT_ROOT)/platform/{platform}/Runner/Info.plist").into();
    for phase in value["targets"][0]["build-phases"]
        .as_array_mut()
        .ok_or("missing template phases")?
    {
        if let Some(script) = phase["script"].as_str() {
            phase["script"] = script
                .replace(
                    "$PROJECT_DIR/day-cli.sh",
                    &format!("$DAY_PROJECT_ROOT/platform/{platform}/day-cli.sh"),
                )
                .into();
        }
    }
    // Older single-language hosts have no string catalog. Preparation creates it when
    // translations require it; never give Xcode a resources entry for a missing input.
    if !root
        .join(format!("platform/{platform}/Runner/InfoPlist.xcstrings"))
        .exists()
    {
        for group in value["files"].as_array_mut().unwrap() {
            if let Some(children) = group["children"].as_array_mut() {
                children.retain(|file| file["path"] != "InfoPlist.xcstrings");
            }
        }
    }
    configure_identity(root, platform, &mut value);
    // GUI builds should use the same CLI that prepared the host (including VS Code's managed
    // CLI), not an older installation found by the shell wrapper's PATH fallback.
    value["build-settings"]["DAY_BIN"] = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned()
        .into();
    ProjectDocument::parse(serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn configure_identity(root: &Path, platform: &str, value: &mut serde_json::Value) {
    let locales = crate::localize::dir_locales(&root.join("resource/locales")).unwrap_or_default();
    value["localizations"]["supported"] = serde_json::json!(
        locales
            .iter()
            .map(|l| crate::localize::xcode_region(l))
            .collect::<Vec<_>>()
    );
    let config = std::fs::read_to_string(root.join(format!("platform/{platform}/DayApp.xcconfig")))
        .unwrap_or_default();
    if let Some(name) = config
        .lines()
        .filter_map(|l| l.split_once('='))
        .find_map(|(k, v)| (k.trim() == "PRODUCT_NAME").then(|| v.trim()))
        .filter(|s| !s.is_empty() && !s.contains('$'))
    {
        if let Some(products) = value["files"]
            .as_array_mut()
            .and_then(|files| files.iter_mut().find(|f| f["name"] == "Products"))
        {
            products["children"][0]["path"] = format!("<PRODUCTS>/{name}.app").into();
        }
        value["targets"][0]["product"] = format!("Products/{name}.app").into();
    }
}

/// Initialize a newly persisted built-in host from the app's current identity and locales.
/// Existing overrides and custom templates are never passed here.
pub fn initialize_persistent(
    root: &Path,
    platform: &str,
    source: &[u8],
) -> Result<Vec<u8>, String> {
    let mut doc =
        ProjectDocument::parse(String::from_utf8(source.to_vec()).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut value = doc.value().clone();
    configure_identity(root, platform, &mut value);
    // Individual edits keep the readable template's formatting intact.
    doc.set("/localizations", value["localizations"].clone())
        .map_err(|e| e.to_string())?;
    doc.set("/files", value["files"].clone())
        .map_err(|e| e.to_string())?;
    doc.set("/targets/0/product", value["targets"][0]["product"].clone())
        .map_err(|e| e.to_string())?;
    Ok(doc.source().as_bytes().to_vec())
}

/// `day prepare --check` must not repair or write build output.
pub fn check_generated(project: &crate::meta::Project, platform: &str) -> Result<(), String> {
    if path(&project.root, platform).parent().unwrap().exists() {
        open(&path(&project.root, platform))?;
        return Ok(());
    }
    let file = project_dir(&project.root, platform).join("project.xcproj");
    let doc = derived_document(&project.root, platform)?;
    if std::fs::read_to_string(&file).ok().as_deref() != Some(doc.source()) {
        return Err(format!(
            "{} is missing or stale — run `day prepare`",
            file.display()
        ));
    }
    Ok(())
}

/// Select the persistent override or refresh Day-owned build output, without touching schemes
/// and IDE user state. A deleted build directory is recreated on the next invocation.
pub fn ensure(project: &crate::meta::Project, platform: &str) -> Result<PathBuf, String> {
    let dir = project_dir(&project.root, platform);
    if dir == path(&project.root, platform).parent().unwrap() {
        open(&dir.join("project.xcproj"))?;
        return Ok(dir);
    }
    for file in [
        "DayApp.xcconfig",
        "Runner/main.swift",
        "Runner/Info.plist",
        "day-cli.sh",
    ] {
        let source = project.root.join(format!("platform/{platform}/{file}"));
        if !source.is_file() {
            return Err(format!(
                "{} is missing; run `day project add-target {}` to restore the host sources",
                source.display(),
                if platform == "ios" {
                    "ios-uikit"
                } else {
                    "macos-appkit"
                }
            ));
        }
    }
    let doc = derived_document(&project.root, platform)?;
    let file = dir.join("project.xcproj");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if std::fs::read_to_string(&file).ok().as_deref() != Some(doc.source()) {
        std::fs::write(&file, doc.source()).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(dir)
}

pub fn open(path: &Path) -> Result<ProjectDocument, String> {
    ProjectDocument::open(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn lint(root: &Path) -> Vec<crate::lint::Finding> {
    use crate::lint::{Finding, Location};
    let mut findings = Vec::new();
    for platform in ["macos", "ios"] {
        let json = path(root, platform);
        let persistent = json.parent().is_some_and(Path::is_dir);
        if !persistent && !root.join(format!("platform/{platform}")).is_dir() {
            continue;
        }
        let document = if persistent {
            open(&json)
        } else {
            derived_document(root, platform)
        };
        let messages = match document {
            Ok(doc) => validate_host(doc.project(), platform),
            Err(e) => vec![e],
        };
        findings.extend(messages.into_iter().map(|message| Finding {
            code: "day::lint::xcode-project",
            message,
            location: Some(Location::head(format!(
                "platform/{platform}/DayApp.xcodeproj/project.xcproj"
            ))),
            ..Default::default()
        }));
    }
    findings
}

/// Validate Day's host contract without requiring generated packages/assets on a fresh checkout.
fn validate_host(project: &ycode_project::Project, platform: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut check = |ok: bool, message: &str| {
        if !ok {
            problems.push(message.to_owned());
        }
    };
    for name in ["Debug", "Release"] {
        check(
            project
                .configurations
                .iter()
                .any(|c| c.name().as_ref() == name),
            &format!("missing {name} project configuration"),
        );
    }
    check(
        project
            .configurations
            .iter()
            .any(|c| c.name() == &project.default_configuration),
        "default-configuration does not name a project configuration",
    );
    // Xcode's converter emits a broad root file reference. Xcode then recursively opens
    // build/day/*/DayPieces as a Folder before SwiftPM can open it as a package dependency.
    check(
        !project.files.iter().any(|r| {
            r.path().path() == "../.." && !matches!(r, ycode_project::Reference::Group(_))
        }),
        "the app root ../.. must be an explicit group of source files, not a recursive folder reference; otherwise Xcode cannot load the generated DayPieces package",
    );
    let Some(Target::Native(target)) = project.targets.iter().find(|t| t.name() == "Runner") else {
        problems.push("missing native Runner application target".into());
        return problems;
    };
    check(
        target
            .product_type_id()
            .is_some_and(|t| t.as_ref() == "com.apple.product-type.application"),
        "Runner must build an application",
    );
    check(
        target.product.is_some(),
        "Runner has no application product reference",
    );
    for name in ["Debug", "Release"] {
        let wired = project.configurations.iter().chain(&target.specialized_configurations).any(|c| {
            matches!(c, ycode_project::Configuration::Detailed(d) if d.name.as_ref() == name && d.file.is_some())
        });
        check(
            wired,
            &format!("{name} needs an xcconfig file for Day-managed build settings"),
        );
    }
    let phases = &target.build_phases;
    let build = phases.iter().position(
        |p| matches!(p, BuildPhase::Script(s) if s.script.0.contains("xcode-backend build")),
    );
    let compile = phases
        .iter()
        .position(|p| p.kind() == BuildPhaseKind::Sources);
    check(
        build.zip(compile).is_some_and(|(b, c)| b < c),
        "Runner must run xcode-backend build before compile-sources",
    );
    for kind in [BuildPhaseKind::Frameworks, BuildPhaseKind::Resources] {
        check(
            phases.iter().any(|p| p.kind() == kind),
            &format!("Runner is missing the {kind:?} build phase"),
        );
    }
    let staging = if platform == "ios" {
        "stage-strings"
    } else {
        "stage-resources"
    };
    check(phases.iter().any(|p| matches!(p, BuildPhase::Script(s) if s.script.0.contains(&format!("xcode-backend {staging}")))), &format!("Runner needs an xcode-backend {staging} script phase"));
    for phase in phases {
        if let BuildPhase::Script(s) = phase
            && s.script.0.contains("xcode-backend ")
        {
            check(
                s.run_on_every_build,
                "Day script phases must set run-on-every-build to true",
            );
        }
    }
    let sandbox = target
        .build_settings
        .get("ENABLE_USER_SCRIPT_SANDBOXING")
        .or_else(|| project.build_settings.get("ENABLE_USER_SCRIPT_SANDBOXING"));
    check(
        sandbox == Some(&BuildSetting::from("NO")),
        "Day callbacks require ENABLE_USER_SCRIPT_SANDBOXING = NO",
    );
    check(project.packages.iter().any(|p| matches!(&p.location, ycode_project::SwiftPackageLocation::Local { path } if [format!("../../build/day/{platform}/DayPieces"), format!("../../{platform}/DayPieces")].contains(path))), "missing generated local DayPieces package reference");
    check(
        target
            .package_product_members
            .iter()
            .any(|p| p.package_product.product_name == "DayPieces"),
        "Runner must link the DayPieces package product",
    );
    problems
}

pub fn regions(root: &Path) -> Option<Vec<String>> {
    let doc = open(&path(root, "ios")).ok()?;
    let l = &doc.project().localizations;
    let mut regions: Vec<_> = l
        .supported
        .iter()
        .chain(std::iter::once(&l.development))
        .filter(|r| r.as_ref() != "Base")
        .map(|r| crate::localize::day_region(r.as_ref()))
        .collect();
    regions.sort();
    regions.dedup();
    Some(regions)
}

pub fn edit_region(root: &Path, tag: &str, add: bool) -> Result<Vec<String>, String> {
    let region = crate::localize::xcode_region(tag);
    let mut done = Vec::new();
    for platform in ["ios", "macos"] {
        let file = path(root, platform);
        if !file.exists() {
            continue;
        }
        let mut doc = open(&file)?;
        let before = doc.source().to_owned();
        doc.edit_project(|p| {
            if add {
                p.localizations.supported.insert(region.as_str().into());
            } else {
                p.localizations.supported.remove(&region.as_str().into());
            }
        })
        .map_err(|e| e.to_string())?;
        if doc.source() != before {
            doc.save().map_err(|e| e.to_string())?;
            done.push(format!(
                "{} {region} {} platform/{platform}/DayApp.xcodeproj/project.xcproj localizations",
                if add { "added" } else { "removed" },
                if add { "to" } else { "from" }
            ));
        }
    }
    Ok(done)
}

/// Read only Runner/project settings; unrelated extension targets must not raise the app floor.
pub fn ios_deployment_targets(root: &Path) -> Vec<String> {
    let Ok(doc) = open(&path(root, "ios")) else {
        return Vec::new();
    };
    let p = doc.project();
    p.build_settings
        .iter()
        .chain(
            p.targets
                .iter()
                .filter(|t| t.name() == "Runner")
                .flat_map(|t| &t.common().build_settings),
        )
        .filter(|(key, _)| {
            key.as_str() == "IPHONEOS_DEPLOYMENT_TARGET"
                || key.starts_with("IPHONEOS_DEPLOYMENT_TARGET[")
        })
        .filter_map(|(_, value)| match value {
            BuildSetting::String(v) => Some(v.clone()),
            _ => None,
        })
        .collect()
}

pub fn scheme_identity(file: &Path) -> Result<(String, String), String> {
    let doc = open(file)?;
    let target = doc
        .project()
        .targets
        .iter()
        .find(|t| t.name() == "Runner")
        .ok_or("project.xcproj has no Runner target")?
        .common();
    let reference = target
        .product
        .as_ref()
        .ok_or("Runner has no product reference")?;
    // Xcode accepts a group-tree name path or a stable file-reference ID.
    let product = match reference {
        ycode_project::GroupTreeReference::NamePath(p) => {
            p.components.last().and_then(|c| match c {
                ycode_project::NamePathComponent::Child(name) => Some(name.clone()),
                _ => None,
            })
        }
        ycode_project::GroupTreeReference::ObjectId(id) => {
            fn find(value: &serde_json::Value, id: &str) -> Option<String> {
                for entry in value.as_array()? {
                    if entry["id"].as_str() == Some(id) {
                        return entry["path"]
                            .as_str()
                            .and_then(|p| p.rsplit('/').next())
                            .map(str::to_owned);
                    }
                    if let Some(found) = find(&entry["children"], id) {
                        return Some(found);
                    }
                }
                None
            }
            find(&doc.value()["files"], id.as_ref())
        }
    }
    .ok_or("Runner application product reference cannot be resolved")?;
    Ok((target.id.to_string(), product))
}

pub fn ensure_strings_phase(file: &Path) -> Result<(), String> {
    let mut doc = open(file)?;
    let runner = doc
        .project()
        .targets
        .iter()
        .position(|t| t.name() == "Runner")
        .ok_or("missing Runner target")?;
    if doc.project().targets[runner].common().build_phases.iter().any(|p| matches!(p, BuildPhase::Script(s) if s.script.0.contains("xcode-backend stage-strings"))) {
        return Ok(());
    }
    let phase = serde_json::from_value(serde_json::json!({
        "kind": "script", "name": "Stage Day Strings", "shell": "/bin/sh",
        "run-on-every-build": true,
        "script": "/bin/sh \"$PROJECT_DIR/day-cli.sh\" xcode-backend stage-strings\n"
    }))
    .map_err(|e| e.to_string())?;
    doc.edit_project(|p| p.targets[runner].common_mut().build_phases.push(phase))
        .map_err(|e| e.to_string())?;
    doc.save().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "day-xcproj-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn host(&self, platform: &str) -> PathBuf {
            let file = path(&self.0, platform);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, template(platform)).unwrap();
            file
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn template(platform: &str) -> String {
        let files = crate::template::builtin_app();
        assert!(!files.iter().any(|f| f.path.ends_with("project.pbxproj")));
        String::from_utf8(
            files
                .into_iter()
                .find(|f| f.path == format!("platform/{platform}/DayApp.xcodeproj/project.xcproj"))
                .unwrap()
                .bytes,
        )
        .unwrap()
    }

    #[test]
    fn derived_hosts_refresh_and_persistent_overrides_win() {
        let fixture = Fixture::new();
        let project = crate::meta::Project {
            root: fixture.0.clone(),
            manifest: toml::from_str(
                "schema = 1\n[app]\nid = 'dev.example.fixture'\ntitle = 'Fixture'\n",
            )
            .unwrap(),
        };
        for locale in ["en", "fr", "zh-Hans", "en-XA"] {
            std::fs::create_dir_all(fixture.0.join(format!("resource/locales/{locale}"))).unwrap();
        }
        for platform in ["macos", "ios"] {
            let host = fixture.0.join(format!("platform/{platform}"));
            std::fs::create_dir_all(host.join("Runner")).unwrap();
            for file in ["Runner/main.swift", "Runner/Info.plist", "day-cli.sh"] {
                std::fs::write(host.join(file), "synthetic fixture").unwrap();
            }
            std::fs::write(host.join("DayApp.xcconfig"), "PRODUCT_NAME = Fixture\n").unwrap();
            assert!(check_generated(&project, platform).is_err());
            let dir = ensure(&project, platform).unwrap();
            check_generated(&project, platform).unwrap();
            assert_eq!(
                dir,
                fixture
                    .0
                    .join(format!("build/day/xcode/{platform}/DayApp.xcodeproj"))
            );
            assert!(!host.join("DayApp.xcodeproj").exists());
            let file = dir.join("project.xcproj");
            let document = open(&file).unwrap();
            assert!(validate_host(document.project(), platform).is_empty());
            assert_eq!(
                scheme_identity(&file).unwrap(),
                ("Runner".into(), "Fixture.app".into())
            );
            assert!(
                document
                    .project()
                    .localizations
                    .supported
                    .contains(&"fr".into())
            );
            assert!(
                !document
                    .project()
                    .localizations
                    .supported
                    .contains(&"en-XA".into())
            );
            // Repeated preparation preserves IDE state while refreshing a damaged/stale document.
            let state = dir.join("xcuserdata/fixture");
            std::fs::create_dir_all(state.parent().unwrap()).unwrap();
            std::fs::write(&state, "user state").unwrap();
            std::fs::write(&file, "stale generated document").unwrap();
            assert!(check_generated(&project, platform).is_err());
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                "stale generated document"
            );
            ensure(&project, platform).unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), document.source());
            assert_eq!(std::fs::read_to_string(state).unwrap(), "user state");
            // Build output is disposable and can always be recreated.
            std::fs::remove_dir_all(&dir).unwrap();
            ensure(&project, platform).unwrap();
            let persistent = fixture.host(platform);
            let source = std::fs::read_to_string(&persistent).unwrap();
            assert_eq!(
                ensure(&project, platform).unwrap(),
                persistent.parent().unwrap()
            );
            assert_eq!(std::fs::read_to_string(&persistent).unwrap(), source);
            std::fs::remove_file(&persistent).unwrap();
            let pbx = persistent.with_file_name("project.pbxproj");
            std::fs::write(&pbx, "synthetic legacy override").unwrap();
            assert!(ensure(&project, platform).is_err());
            assert!(check_generated(&project, platform).is_err());
            assert!(!persistent.exists());
            std::fs::remove_file(pbx).unwrap();
            assert!(ensure(&project, platform).is_err());
        }
    }

    #[test]
    fn derived_host_references_a_string_catalog_only_when_it_exists() {
        let fixture = Fixture::new();
        let has_catalog = |doc: &ProjectDocument| doc.source().contains("InfoPlist.xcstrings");
        assert!(!has_catalog(&derived_document(&fixture.0, "ios").unwrap()));
        let catalog = fixture.0.join("platform/ios/Runner/InfoPlist.xcstrings");
        std::fs::create_dir_all(catalog.parent().unwrap()).unwrap();
        std::fs::write(&catalog, "{}").unwrap();
        assert!(has_catalog(&derived_document(&fixture.0, "ios").unwrap()));
    }

    #[test]
    fn both_builtin_hosts_validate_without_generated_files() {
        let fixture = Fixture::new();
        for platform in ["macos", "ios"] {
            let file = fixture.host(platform);
            let document = open(&file).unwrap();
            assert!(validate_host(document.project(), platform).is_empty());
            assert_eq!(
                scheme_identity(&file).unwrap(),
                ("Runner".into(), "DayApp.app".into())
            );
        }
        assert!(lint(&fixture.0).is_empty());
    }

    #[test]
    fn broken_day_contracts_are_reported() {
        let mut p = ProjectDocument::parse(template("ios"))
            .unwrap()
            .project()
            .clone();
        p.targets[0].common_mut().build_phases.reverse();
        p.packages.clear();
        p.targets[0].common_mut().package_product_members.clear();
        p.build_settings
            .insert("ENABLE_USER_SCRIPT_SANDBOXING".into(), "YES".into());
        p.default_configuration = "Missing".into();
        let findings = validate_host(&p, "ios").join("\n");
        for expected in [
            "before compile-sources",
            "DayPieces package reference",
            "DayPieces package product",
            "SANDBOXING",
            "default-configuration",
        ] {
            assert!(findings.contains(expected), "{findings}");
        }
        p.targets.clear();
        assert!(
            validate_host(&p, "ios")
                .iter()
                .any(|s| s.contains("missing native Runner"))
        );
    }

    #[test]
    fn legacy_only_and_invalid_json_are_errors() {
        let fixture = Fixture::new();
        let json = fixture.host("ios");
        let legacy = json.with_file_name("project.pbxproj");
        std::fs::write(&legacy, "// synthetic legacy fixture").unwrap();
        let findings = lint(&fixture.0);
        assert!(findings.is_empty());
        std::fs::write(&json, "{ invalid JSON5").unwrap();
        assert!(
            lint(&fixture.0)
                .iter()
                .any(|f| f.severity() == crate::lint::Severity::Error)
        );
        std::fs::remove_file(&json).unwrap();
        let findings = lint(&fixture.0);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity(), crate::lint::Severity::Error);
    }

    #[test]
    fn locale_edits_preserve_comments_and_include_development_language() {
        let fixture = Fixture::new();
        for platform in ["ios", "macos"] {
            let file = fixture.host(platform);
            std::fs::write(
                file,
                format!("// synthetic fixture comment\n{}", template(platform)),
            )
            .unwrap();
        }
        assert_eq!(regions(&fixture.0).unwrap(), ["en"]);
        edit_region(&fixture.0, "zh-CN", true).unwrap();
        assert_eq!(regions(&fixture.0).unwrap(), ["en", "zh-CN"]);
        assert!(edit_region(&fixture.0, "zh-CN", true).unwrap().is_empty());
        for platform in ["ios", "macos"] {
            let doc = open(&path(&fixture.0, platform)).unwrap();
            assert!(doc.source().starts_with("// synthetic fixture comment"));
            assert!(
                doc.project()
                    .localizations
                    .supported
                    .contains(&"zh-Hans".into())
            );
        }
        edit_region(&fixture.0, "zh-CN", false).unwrap();
        assert_eq!(regions(&fixture.0).unwrap(), ["en"]);
    }

    #[test]
    fn converted_recursive_app_root_is_rejected() {
        let mut doc = ProjectDocument::parse(template("macos")).unwrap();
        doc.set("/files/0", serde_json::json!({"path": "../.."}))
            .unwrap();
        assert!(
            validate_host(doc.project(), "macos")
                .iter()
                .any(|m| m.contains("recursive folder"))
        );
    }

    #[test]
    fn archive_product_ids_resolve() {
        let fixture = Fixture::new();
        let file = fixture.host("ios");
        let mut doc = open(&file).unwrap();
        // Explicit IDs are valid in customized JSON projects.
        doc.set("/files/4/children/0/id", "fixture-product-id".into())
            .unwrap();
        doc.set("/targets/0/product", "id:fixture-product-id".into())
            .unwrap();
        doc.save().unwrap();
        assert_eq!(scheme_identity(&file).unwrap().1, "DayApp.app");
    }

    #[test]
    fn ycode_rejects_duplicate_targets_and_invalid_setting_types() {
        let fixture = Fixture::new();
        let file = fixture.host("macos");
        let doc = open(&file).unwrap();
        let mut value = doc.value().clone();
        let duplicate = value["targets"][0].clone();
        value["targets"].as_array_mut().unwrap().push(duplicate);
        std::fs::write(&file, value.to_string()).unwrap();
        assert!(
            lint(&fixture.0)
                .iter()
                .any(|f| f.severity() == crate::lint::Severity::Error)
        );
        value = doc.value().clone();
        value["build-settings"]["SWIFT_VERSION"] = 5.into();
        std::fs::write(&file, value.to_string()).unwrap();
        assert!(
            lint(&fixture.0)
                .iter()
                .any(|f| f.severity() == crate::lint::Severity::Error)
        );
    }

    #[test]
    fn shortcut_phase_repair_is_idempotent_and_deployment_overrides_are_read() {
        let fixture = Fixture::new();
        let file = fixture.host("ios");
        let mut doc = open(&file).unwrap();
        doc.edit_project(|p| {
            p.targets[0].common_mut().build_phases.retain(
                |p| !matches!(p, BuildPhase::Script(s) if s.script.0.contains("stage-strings")),
            );
        })
        .unwrap();
        doc.set_build_setting(
            Some("Runner"),
            "IPHONEOS_DEPLOYMENT_TARGET[config=Release]",
            "18.0".into(),
        )
        .unwrap();
        doc.save().unwrap();
        ensure_strings_phase(&file).unwrap();
        let source = std::fs::read_to_string(&file).unwrap();
        ensure_strings_phase(&file).unwrap();
        assert_eq!(source, std::fs::read_to_string(&file).unwrap());
        assert_eq!(ios_deployment_targets(&fixture.0), ["18.0"]);
        assert!(lint(&fixture.0).is_empty());
    }
}
