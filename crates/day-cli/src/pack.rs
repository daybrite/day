//! day pack v0 (DESIGN.md §16.5): build → sign (ad-hoc/debug) → installable artifact.
//! macos-appkit → .app + ad-hoc codesign + .dmg (hdiutil); android-widget → debug-signed .apk;
//! ios-uikit → zipped Simulator .app (there is no "simulator .ipa" — §16.5).

use std::path::PathBuf;
use std::process::Command;

use crate::meta::Project;
use crate::ops::{self, BuildOutcome, status};
use crate::targets::{Target, TargetKind};

pub fn run(project: &Project, target: &'static Target, profile: &str) -> Result<PathBuf, String> {
    let outcome = ops::build(project, target, profile)?;
    let dist = project.root.join("build/day/dist");
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let name = &project.manifest.app.name;
    let version = &project.manifest.app.version;
    match (target.name, target.kind) {
        ("macos-appkit", _) => pack_macos(project, &outcome, &dist),
        (_, TargetKind::Android) => {
            let out = dist.join(format!("{name}-{version}.apk"));
            std::fs::copy(&outcome.artifact, &out).map_err(|e| e.to_string())?;
            status("Packed", &format!("{} (debug-signed apk)", out.display()));
            Ok(out)
        }
        (_, TargetKind::IosSim) => {
            let out = dist.join(format!("{name}-{version}-sim.app.zip"));
            let ok = Command::new("ditto")
                .args(["-c", "-k", "--keepParent"])
                .arg(&outcome.artifact)
                .arg(&out)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err("ditto zip failed".into());
            }
            status("Packed", &format!("{} (installable via simctl)", out.display()));
            Ok(out)
        }
        _ => Err(format!("pack for {} lands post-MVP (§16.5)", target.name)),
    }
}

fn pack_macos(project: &Project, outcome: &BuildOutcome, dist: &PathBuf) -> Result<PathBuf, String> {
    let name = &project.manifest.app.name;
    let title = project.manifest.app.title.clone().unwrap_or_else(|| name.clone());
    let version = &project.manifest.app.version;
    let stage = project.root.join("build/day/pack/macos-appkit");
    let app = stage.join(format!("{title}.app"));
    let _ = std::fs::remove_dir_all(&stage);
    let macos_dir = app.join("Contents/MacOS");
    let res_dir = app.join("Contents/Resources/assets");
    std::fs::create_dir_all(&macos_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&res_dir).map_err(|e| e.to_string())?;
    std::fs::copy(&outcome.artifact, macos_dir.join(name)).map_err(|e| e.to_string())?;
    let assets = project.root.join("assets");
    if let Ok(entries) = std::fs::read_dir(&assets) {
        for e in entries.flatten() {
            let _ = std::fs::copy(e.path(), res_dir.join(e.file_name()));
        }
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>{name}</string>
  <key>CFBundleIdentifier</key><string>{id}</string>
  <key>CFBundleName</key><string>{title}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{build}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#,
        id = project.manifest.app.id,
        build = project.manifest.app.build,
    );
    std::fs::write(app.join("Contents/Info.plist"), plist).map_err(|e| e.to_string())?;

    // Ad-hoc sign (Developer-ID + notarization are the release path, post-MVP §16.5).
    status("Signing", "ad-hoc codesign");
    let ok = Command::new("codesign")
        .args(["--force", "--deep", "-s", "-"])
        .arg(&app)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("codesign failed".into());
    }

    let dmg = dist.join(format!("{title}-{version}.dmg"));
    let _ = std::fs::remove_file(&dmg);
    status("Packing", "hdiutil create");
    let ok = Command::new("hdiutil")
        .args(["create", "-quiet", "-volname", &title, "-srcfolder"])
        .arg(&stage)
        .args(["-ov", "-format", "UDZO"])
        .arg(&dmg)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("hdiutil failed".into());
    }
    status("Packed", &format!("{}", dmg.display()));
    Ok(dmg)
}
