// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use super::*;
use crate::meta::Project;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("day-extensions-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// A project at `root` with `[app] targets` and `extensions`, the rest of Day.toml minimal.
fn project(root: PathBuf, targets: &str, extensions: &str) -> Project {
    let manifest = crate::meta::parse_manifest(
        &format!("schema=1\n[app]\nid='dev.example.app'\ntargets=[{targets}]\n{extensions}"),
        "[package]\nname='ext-test'\nversion='1.2.3'",
        None,
    )
    .expect("Day.toml parses");
    Project { root, manifest }
}

fn ios(name: &str, extensions: &str) -> (Project, Vec<Resolved>) {
    let p = project(scratch(name), "'ios-uikit'", extensions);
    let r = resolve(&p, Os::Ios).expect("resolves");
    (p, r)
}

fn err(targets: &str, extensions: &str, os: Os) -> String {
    let p = project(scratch("err"), targets, extensions);
    resolve(&p, os).expect_err("must not resolve")
}

#[test]
fn a_kind_line_is_a_whole_declaration() {
    let (_, exts) = ios("kind", "[[extensions]]\nname='Widgets'\nkind='widget'\n");
    let [w] = exts.as_slice() else {
        panic!("one extension")
    };
    assert_eq!(w.point, "com.apple.widgetkit-extension");
    assert_eq!(w.family, Family::Classic);
    assert_eq!(w.bundle_setting, "$(inherited).Widgets");
    assert_eq!(w.bundle_id, "dev.example.app.Widgets");
    assert_eq!(w.sources, "platform/ios/Widgets");
    assert_eq!(w.deployment_target, "14.0");
    assert_eq!(w.display_name, "Widgets");
}

#[test]
fn platforms_default_to_the_apple_targets_the_kind_has() {
    let p = project(
        scratch("both"),
        "'ios-uikit','macos-appkit','android-material'",
        "[[extensions]]\nname='Keys'\nkind='keyboard'\n\n[[extensions]]\nname='Share'\nkind='share'\n",
    );
    let names = |os| -> Vec<String> {
        resolve(&p, os)
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect()
    };
    assert_eq!(names(Os::Ios), ["Keys", "Share"]);
    // A keyboard exists only on iOS.
    assert_eq!(names(Os::Macos), ["Share"]);
}

#[test]
fn bundle_ids_are_suffixes_or_start_with_the_apps() {
    let (_, exts) = ios(
        "ids",
        "[[extensions]]\nname='A'\nkind='share'\nbundle-id='share-it'\n\n\
         [[extensions]]\nname='B'\nkind='action'\nbundle-id='dev.example.app.act'\n",
    );
    assert_eq!(exts[0].bundle_setting, "$(inherited).share-it");
    assert_eq!(exts[0].bundle_id, "dev.example.app.share-it");
    assert_eq!(exts[1].bundle_setting, "dev.example.app.act");
    let e = err(
        "'ios-uikit'",
        "[[extensions]]\nname='A'\nkind='share'\nbundle-id='org.other.Share'\n",
        Os::Ios,
    );
    assert!(e.contains("must start with the app's id"), "{e}");
}

#[test]
fn names_and_values_that_would_break_the_build_are_refused() {
    for (decl, needle) in [
        ("name='Runner'\nkind='share'", "Runner"),
        ("name='Intents'\nkind='intents'", "system framework"),
        ("name='9lives'\nkind='share'", "start with a letter"),
        ("name='Ok'\nkind='nope'", "unknown kind"),
        ("name='Ok'", "names no `kind`"),
        ("name='Ok'\nkind='custom'", "needs `point`"),
        (
            "name='Ok'\nkind='keyboard'\nplatforms=['macos']",
            "does not exist on macos",
        ),
        (
            "name='Ok'\nkind='share'\nextensionkit=true",
            "applies only to kind = \"custom\"",
        ),
        (
            "name='Ok'\nkind='share'\nlive-activities=true",
            "only to kind = \"widget\"",
        ),
        ("name='Ok'\nkind='share'\napp-groups=['shared']", "group."),
        (
            "name='Ok'\nkind='share'\nsources='../elsewhere'",
            "inside the project",
        ),
        (
            "name='Ok'\nkind='share'\ndeployment-target='seventeen'",
            "not a version",
        ),
    ] {
        let e = err(
            "'ios-uikit','macos-appkit'",
            &format!("[[extensions]]\n{decl}\n"),
            Os::Ios,
        );
        assert!(e.contains(needle), "{decl}: {e}");
    }
    let e = err(
        "'ios-uikit'",
        "[[extensions]]\nname='Share'\nkind='share'\n\n[[extensions]]\nname='share'\nkind='action'\n",
        Os::Ios,
    );
    assert!(e.contains("two extensions named"), "{e}");
    let e = err(
        "'ios-uikit'",
        "[[extensions]]\nname='Chat'\nkind='imessage'\n\n\
         [[extensions]]\nname='Stickers'\nkind='sticker-pack'\n",
        Os::Ios,
    );
    assert!(e.contains("one Messages extension"), "{e}");
}

#[test]
fn every_kind_has_a_valid_default_name_and_a_starter_per_platform() {
    for spec in KINDS {
        let name = scaffold::default_name(spec.name);
        assert!(valid_name(&name), "{}: default name {name}", spec.name);
        if spec.name == "custom" {
            continue;
        }
        let platforms = spec.platforms();
        assert!(!platforms.is_empty(), "{} has a platform", spec.name);
        for os in platforms {
            let files = starters::files(spec.name, os, &name, "Title");
            assert!(!files.is_empty(), "{} has a starter on {os:?}", spec.name);
            for (path, body) in &files {
                assert!(!body.contains("__NAME__") && !path.contains("__NAME__"));
                assert!(!body.contains("__TITLE__"));
            }
        }
    }
}

#[test]
fn the_deployment_target_is_the_kind_floor_or_the_apps_whichever_is_later() {
    let root = scratch("floor");
    std::fs::create_dir_all(root.join("platform/ios")).unwrap();
    std::fs::write(
        root.join("platform/ios/DayApp.xcconfig"),
        "IPHONEOS_DEPLOYMENT_TARGET = 16.4\n",
    )
    .unwrap();
    let p = project(
        root,
        "'ios-uikit'",
        "[[extensions]]\nname='W'\nkind='widget'\n\n[[extensions]]\nname='M'\nkind='marketplace'\n\n\
         [[extensions]]\nname='S'\nkind='share'\ndeployment-target='26.0'\n",
    );
    let exts = resolve(&p, Os::Ios).unwrap();
    let targets: Vec<&str> = exts.iter().map(|r| r.deployment_target.as_str()).collect();
    assert_eq!(targets, ["16.4", "17.4", "26.0"]);
}

#[test]
fn info_plists_name_the_point_and_qualify_the_principal_class() {
    let (p, exts) = ios(
        "info",
        "[[extensions]]\nname='Share'\nkind='share'\ndisplay-name='Save to App'\n\
         attributes={ NSExtensionActivationRule = { NSExtensionActivationSupportsImageWithMaxCount = 3 } }\n\
         info={ NSExtension = { NSExtensionAttributes = { Custom = true } }, Extra = 'yes' }\n",
    );
    let info = info_plist(&exts[0], &p.root).unwrap();
    let ext = info["NSExtension"].as_dictionary().unwrap();
    assert_eq!(
        ext["NSExtensionPointIdentifier"].as_string(),
        Some("com.apple.share-services")
    );
    assert_eq!(
        ext["NSExtensionPrincipalClass"].as_string(),
        Some("$(PRODUCT_MODULE_NAME).ShareViewController")
    );
    let attrs = ext["NSExtensionAttributes"].as_dictionary().unwrap();
    let rule = attrs["NSExtensionActivationRule"].as_dictionary().unwrap();
    // The preset's rule and the declared key merge; neither replaces the other.
    assert!(rule.contains_key("NSExtensionActivationSupportsWebURLWithMaxCount"));
    assert!(rule.contains_key("NSExtensionActivationSupportsImageWithMaxCount"));
    assert_eq!(attrs["Custom"].as_boolean(), Some(true));
    assert_eq!(info["Extra"].as_string(), Some("yes"));
    assert_eq!(info["CFBundleDisplayName"].as_string(), Some("Save to App"));
    assert_eq!(
        info["CFBundleShortVersionString"].as_string(),
        Some("$(MARKETING_VERSION)")
    );
    // Sorted keys: the same declaration always writes the same bytes.
    let keys: Vec<&String> = info.keys().collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    assert_eq!(keys, sorted_keys);
}

#[test]
fn extensionkit_points_live_in_ex_app_extension_attributes() {
    let (p, exts) = ios(
        "exkit",
        "[[extensions]]\nname='MarketplaceExtension'\nkind='marketplace'\n",
    );
    let r = &exts[0];
    assert_eq!(r.family, Family::ExtensionKit);
    assert!(r.family.in_extensions_folder());
    let info = info_plist(r, &p.root).unwrap();
    assert!(!info.contains_key("NSExtension"));
    let ex = info["EXAppExtensionAttributes"].as_dictionary().unwrap();
    assert_eq!(
        ex["EXExtensionPointIdentifier"].as_string(),
        Some("com.apple.marketplace.extension")
    );
}

#[test]
fn the_source_folders_info_plist_and_entitlements_merge_over_the_preset() {
    let root = scratch("merge");
    let dir = root.join("platform/ios/MarketplaceExtension");
    std::fs::create_dir_all(&dir).unwrap();
    let mut own = ::plist::Dictionary::new();
    own.insert("CFBundleDisplayName".into(), "App Fair Marketplace".into());
    ::plist::Value::Dictionary(own)
        .to_file_xml(dir.join("Info.plist"))
        .unwrap();
    let mut ents = ::plist::Dictionary::new();
    ents.insert(
        "com.apple.security.application-groups".into(),
        ::plist::Value::Array(vec!["group.dev.example.app".into()]),
    );
    ::plist::Value::Dictionary(ents)
        .to_file_xml(dir.join("MarketplaceExtension.entitlements"))
        .unwrap();
    let p = project(
        root.clone(),
        "'ios-uikit'",
        "[[extensions]]\nname='MarketplaceExtension'\nkind='marketplace'\n",
    );
    let r = &resolve(&p, Os::Ios).unwrap()[0];
    assert_eq!(
        r.entitlements_file.as_deref(),
        Some("platform/ios/MarketplaceExtension/MarketplaceExtension.entitlements")
    );
    let info = info_plist(r, &root).unwrap();
    assert_eq!(
        info["CFBundleDisplayName"].as_string(),
        Some("App Fair Marketplace")
    );
    assert!(info.contains_key("EXAppExtensionAttributes"));
    let e = entitlements(r, &root).unwrap();
    assert_eq!(groups_of(&e), ["group.dev.example.app"]);
}

#[test]
fn entitlements_carry_the_kind_the_groups_and_the_macos_sandbox() {
    let p = project(
        scratch("ents"),
        "'ios-uikit','macos-appkit'",
        "[[extensions]]\nname='Tunnel'\nkind='packet-tunnel'\napp-groups=['group.dev.example.app']\n\n\
         [[extensions]]\nname='Plain'\nkind='share'\n",
    );
    let ios = resolve(&p, Os::Ios).unwrap();
    let tunnel = entitlements(&ios[0], &p.root).unwrap();
    assert_eq!(
        tunnel["com.apple.developer.networking.networkextension"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(groups_of(&tunnel), ["group.dev.example.app"]);
    assert!(!tunnel.contains_key("com.apple.security.app-sandbox"));
    // An extension with nothing to claim still names a file: an empty dictionary.
    assert!(entitlements(&ios[1], &p.root).unwrap().is_empty());
    let mac = resolve(&p, Os::Macos).unwrap();
    assert_eq!(
        entitlements(&mac[1], &p.root).unwrap()["com.apple.security.app-sandbox"].as_boolean(),
        Some(true)
    );
}

#[test]
fn an_intents_extension_with_no_intent_is_refused_before_the_build() {
    let (p, exts) = ios("intents", "[[extensions]]\nname='Siri'\nkind='intents'\n");
    let info = info_plist(&exts[0], &p.root).unwrap();
    assert!(uninstallable(&exts[0], &info).is_some());
    assert!(stage(&p, Os::Ios).is_err());
    let (p, exts) = ios(
        "intents-ok",
        "[[extensions]]\nname='Siri'\nkind='intents'\n\
         attributes={ IntentsSupported=['INSendMessageIntent'] }\n",
    );
    let info = info_plist(&exts[0], &p.root).unwrap();
    assert!(uninstallable(&exts[0], &info).is_none());
}

#[test]
fn staging_writes_once_and_prunes_what_is_no_longer_declared() {
    let (p, exts) = ios("stage", "[[extensions]]\nname='Share'\nkind='share'\n");
    stage(&p, Os::Ios).unwrap();
    let info = p.root.join(exts[0].staged_info_plist());
    let first = std::fs::metadata(&info).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    stage(&p, Os::Ios).unwrap();
    assert_eq!(std::fs::metadata(&info).unwrap().modified().unwrap(), first);
    check_staged(&p, Os::Ios).unwrap();
    let stray = p.root.join("build/day/extensions/ios/Gone");
    std::fs::create_dir_all(&stray).unwrap();
    stage(&p, Os::Ios).unwrap();
    assert!(!stray.exists());
}

#[test]
fn per_target_command_line_settings_fall_back_to_the_apps_value() {
    assert_eq!(
        cli_setting("IPHONEOS_DEPLOYMENT_TARGET", "16.0", false),
        "IPHONEOS_DEPLOYMENT_TARGET=16.0"
    );
    assert_eq!(
        cli_setting("IPHONEOS_DEPLOYMENT_TARGET", "16.0", true),
        "IPHONEOS_DEPLOYMENT_TARGET=$(DAY_TARGET_IPHONEOS_DEPLOYMENT_TARGET:default=16.0)"
    );
}

fn template(platform: &str) -> serde_json::Value {
    let file = crate::template::builtin_app()
        .into_iter()
        .find(|f| f.path == format!("platform/{platform}/DayApp.xcodeproj/project.xcproj"))
        .expect("template");
    ycode_project::ProjectDocument::parse(String::from_utf8(file.bytes).unwrap())
        .unwrap()
        .value()
        .clone()
}

#[test]
fn the_project_gains_a_target_an_embed_phase_and_a_product_per_extension() {
    let root = scratch("project");
    std::fs::create_dir_all(root.join("platform/ios/Web/Resources")).unwrap();
    std::fs::write(root.join("platform/ios/Web/Info.plist"), "").unwrap();
    let p = project(
        root.clone(),
        "'ios-uikit'",
        "[[extensions]]\nname='Widgets'\nkind='widget'\n\n\
         [[extensions]]\nname='Web'\nkind='safari-web-extension'\nframeworks=['SafariServices']\n\n\
         [[extensions]]\nname='MarketplaceExtension'\nkind='marketplace'\n\
         swift-packages=[{ url='https://github.com/example/kit', from='1.2.0', products=['Kit'] }]\n",
    );
    let exts = resolve(&p, Os::Ios).unwrap();
    let mut value = template("ios");
    project::apply(&mut value, &exts, &project::DERIVED, &root).unwrap();

    let targets: Vec<&str> = value["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        targets,
        ["Runner", "Widgets", "Web", "MarketplaceExtension"]
    );
    let runner = &value["targets"][0];
    assert_eq!(
        runner["dependencies"],
        serde_json::json!(["Widgets", "Web", "MarketplaceExtension"])
    );
    let phases = runner["build-phases"].as_array().unwrap();
    let resources = phases.iter().position(|p| p == "resources").unwrap();
    assert_eq!(phases[resources + 1]["name"], project::EMBED_PLUGINS);
    assert_eq!(phases[resources + 2]["name"], project::EMBED_EXTENSIONKIT);
    assert_eq!(
        phases[resources + 2]["relative-path"],
        "$(EXTENSIONS_FOLDER_PATH)"
    );

    let market = &value["targets"][3];
    assert_eq!(market["product-type"], "extensionkit-extension");
    let settings = &market["build-settings"];
    assert_eq!(settings["IPHONEOS_DEPLOYMENT_TARGET"], "17.4");
    assert_eq!(settings["DAY_TARGET_IPHONEOS_DEPLOYMENT_TARGET"], "17.4");
    assert_eq!(
        settings["CODE_SIGN_ENTITLEMENTS"],
        "$(DAY_PROJECT_ROOT)/build/day/extensions/ios/MarketplaceExtension/MarketplaceExtension.entitlements"
    );
    assert_eq!(market["package-product-members"][0]["product-name"], "Kit");
    assert!(
        value["packages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["repository"] == "https://github.com/example/kit")
    );
    let web = &value["targets"][2];
    assert_eq!(
        web["build-settings"]["OTHER_LDFLAGS"],
        "$(inherited) -framework SafariServices"
    );
    assert!(
        web["build-phases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "Copy Resources Folder")
    );

    let files = value["files"].as_array().unwrap();
    let folder = files
        .iter()
        .find(|f| f["path"] == "../../../../platform/ios/Web")
        .expect("the Web folder");
    assert_eq!(folder["kind"], "folder");
    assert_eq!(
        folder["membership-exceptions"][0]["exclusions"],
        serde_json::json!(["Info.plist", "Resources"])
    );
    let products = files.iter().find(|f| f["name"] == "Products").unwrap();
    let embedded: Vec<&str> = products["children"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["target-membership"][0]["build-phase"].as_str())
        .collect();
    assert_eq!(
        embedded,
        [
            "Runner/copy/Embed Foundation Extensions",
            "Runner/copy/Embed Foundation Extensions",
            "Runner/copy/Embed ExtensionKit Extensions",
        ]
    );

    // The result is a project the format accepts, and applying again changes nothing.
    let text = serde_json::to_string_pretty(&value).unwrap();
    ycode_project::ProjectDocument::parse(text.clone()).expect("a valid project.xcproj");
    project::apply(&mut value, &exts, &project::DERIVED, &root).unwrap();
    assert_eq!(serde_json::to_string_pretty(&value).unwrap(), text);
}

#[test]
fn macos_extension_targets_run_sandboxed() {
    let root = scratch("macos");
    let p = project(
        root.clone(),
        "'macos-appkit'",
        "[[extensions]]\nname='Finder'\nkind='finder-sync'\n",
    );
    let exts = resolve(&p, Os::Macos).unwrap();
    let mut value = template("macos");
    project::apply(&mut value, &exts, &project::PERSISTENT, &root).unwrap();
    let t = value["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Finder")
        .unwrap();
    assert_eq!(t["build-settings"]["ENABLE_APP_SANDBOX"], "YES");
    assert_eq!(t["build-settings"]["DAY_TARGET_ENABLE_APP_SANDBOX"], "YES");
    assert_eq!(
        t["build-settings"]["INFOPLIST_FILE"],
        "$(PROJECT_DIR)/../../build/day/extensions/macos/Finder/Info.plist"
    );
    let info = info_plist(&exts[0], &root).unwrap();
    assert_eq!(info["LSUIElement"].as_boolean(), Some(true));
}

#[test]
fn add_extension_appends_a_table_and_keeps_the_rest_of_day_toml() {
    let text = "# The manifest.\nschema = 1\n\n[app]\nid = \"dev.example.app\" # keep\n";
    let out = scaffold::append_declaration(text, "Widgets", "widget", &[]).unwrap();
    assert!(out.starts_with(text), "{out}");
    assert!(out.contains("[[extensions]]\nname = \"Widgets\"\nkind = \"widget\"\n"));
    let out = scaffold::append_declaration(&out, "Siri", "intents", &["ios".into()]).unwrap();
    let parsed: toml::Value = toml::from_str(&out).unwrap();
    let siri = &parsed["extensions"][1];
    assert_eq!(siri["platforms"][0].as_str(), Some("ios"));
    assert_eq!(
        siri["attributes"]["IntentsSupported"][0].as_str(),
        Some("INSendMessageIntent")
    );
}

#[test]
fn a_checked_in_project_gains_only_appended_elements() {
    let root = scratch("persistent");
    let p = project(
        root.clone(),
        "'ios-uikit'",
        "[[extensions]]\nname='Widgets'\nkind='widget'\n\n\
         [[extensions]]\nname='MarketplaceExtension'\nkind='marketplace'\n",
    );
    let exts = resolve(&p, Os::Ios).unwrap();
    let file = crate::template::builtin_app()
        .into_iter()
        .find(|f| f.path == "platform/ios/DayApp.xcodeproj/project.xcproj")
        .unwrap();
    let source = String::from_utf8(file.bytes).unwrap();
    let mut doc = ycode_project::ProjectDocument::parse(source.clone()).unwrap();
    project::apply_to_document(&mut doc, &exts, &root).unwrap();
    let v = doc.value();
    let names: Vec<&str> = v["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Runner", "Widgets", "MarketplaceExtension"]);
    let products = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Products")
        .unwrap();
    let paths: Vec<&str> = products["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "<PRODUCTS>/DayApp.app",
            "<PRODUCTS>/Widgets.appex",
            "<PRODUCTS>/MarketplaceExtension.appex"
        ]
    );
    assert_eq!(
        v["targets"][0]["dependencies"],
        serde_json::json!(["Widgets", "MarketplaceExtension"])
    );
    // The template's own lines are untouched: the first line still reads as it did.
    assert_eq!(
        doc.source().lines().next(),
        source.lines().next(),
        "the document was rewritten rather than appended to"
    );
    // Applying again changes nothing.
    let once = doc.source().to_string();
    project::apply_to_document(&mut doc, &exts, &root).unwrap();
    assert_eq!(doc.source(), once);
}
