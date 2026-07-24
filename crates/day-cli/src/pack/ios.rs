//! ios-uikit → App Store .ipa via `xcodebuild archive` + `-exportArchive` (arm64-only device
//! build; automatic signing with an App Store Connect API key — the Tauri/Flutter CI path).
//! Without `signing.ios` config this degrades LOUDLY to the zipped Simulator .app (installable
//! via simctl; there is no "simulator .ipa" — §16.5).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::{PackOptions, resolve_degradable};
use super::{Artifact, PackError, SignTier, run_tool};
use crate::meta::Project;
use crate::ops::{self, status};
use crate::targets::Target;

pub fn pack(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
) -> Result<Artifact, PackError> {
    let ios = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.ios.as_ref());
    if opts.no_sign || ios.is_none() {
        if ios.is_none() {
            status(
                "Warning",
                "no signing.ios config — packing the SIMULATOR .app.zip (a device .ipa needs \
                 signing.ios: {team, key-id, issuer, key-path})",
            );
        }
        return sim_zip(project, target, opts, dist);
    }
    let ios = ios.unwrap();

    // A team that references an unset secret degrades to the sim zip (§20).
    let Some(team) = resolve_degradable(&ios.team, "signing.ios.team").map_err(PackError::Sign)?
    else {
        status(
            "Warning",
            "signing.ios.team unresolved — packing the SIMULATOR .app.zip instead of a device .ipa",
        );
        return sim_zip(project, target, opts, dist);
    };
    let method = ios
        .export_method
        .clone()
        .unwrap_or_else(|| "app-store-connect".into());
    // ASC API key: all-or-nothing triple; without it xcodebuild uses the local Xcode account session.
    let key_id = resolve_field(ios.key_id.as_ref(), "signing.ios.key-id")?;
    let issuer = resolve_field(ios.issuer.as_ref(), "signing.ios.issuer")?;
    let key_path = resolve_field(ios.key_path.as_ref(), "signing.ios.key-path")?;
    let asc = match (key_id, issuer, key_path) {
        (Some(k), Some(i), Some(p)) => {
            if !Path::new(&p).exists() {
                return Err(PackError::Sign(format!("ASC key file not found: {p}")));
            }
            Some((k, i, p))
        }
        (None, None, None) => None,
        _ => {
            return Err(PackError::Sign(
                "signing.ios: key-id, issuer and key-path must be set together".into(),
            ));
        }
    };

    let name = &project.manifest.app.name;
    let version = &project.manifest.app.version;
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| name.clone());

    // The DayPieces SwiftPM package must exist before xcodebuild resolves the project.
    crate::pieces::write_ios_pieces(project).map_err(PackError::Other)?;
    ensure_shared_scheme(project).map_err(PackError::Other)?;

    let build_dir = project.root.join("build/day/ios-uikit");
    let archive = build_dir.join(format!("{title}.xcarchive"));
    let _ = std::fs::remove_dir_all(&archive);
    let day_bin = std::env::current_exe().map_err(|e| PackError::Other(e.to_string()))?;

    // --- archive (device, Release, automatic signing) -----------------------
    status(
        "Building",
        "ios-uikit (xcodebuild archive, generic/platform=iOS)",
    );
    let mut cmd = Command::new("xcodebuild");
    cmd.current_dir(project.root.join("platform/ios"))
        .args(["-project", "DayApp.xcodeproj", "-scheme", "Runner"])
        .args(["-configuration", "Release"])
        .args(["-destination", "generic/platform=iOS"])
        .arg("-archivePath")
        .arg(&archive)
        .arg("-derivedDataPath")
        .arg(build_dir.join("archive-dd"))
        .arg("-allowProvisioningUpdates")
        // The scaffold pbxproj disables signing for simulator development — the archive build
        // re-enables it from the command line (command-line settings override the project).
        .arg("CODE_SIGNING_ALLOWED=YES")
        .arg("CODE_SIGN_STYLE=Automatic")
        .arg("CODE_SIGN_IDENTITY=Apple Development")
        .arg(format!("DEVELOPMENT_TEAM={team}"))
        .arg(format!("MARKETING_VERSION={version}"))
        .arg(format!(
            "CURRENT_PROJECT_VERSION={}",
            project.manifest.app.build
        ))
        .arg(format!("DAY_BIN={}", day_bin.display()))
        .arg("archive");
    if let Some((k, i, p)) = &asc {
        cmd.args(["-authenticationKeyID", k])
            .args(["-authenticationKeyIssuerID", i])
            .arg("-authenticationKeyPath")
            .arg(std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)));
    }
    run_tool(&mut cmd, "xcodebuild archive").map_err(PackError::Sign)?;

    // --- export (.ipa) -------------------------------------------------------
    let export_plist = build_dir.join("ExportOptions.plist");
    std::fs::write(&export_plist, export_options(&method, &team))
        .map_err(|e| PackError::Other(e.to_string()))?;
    let export_dir = build_dir.join("export");
    let _ = std::fs::remove_dir_all(&export_dir);
    status("Packing", &format!("xcodebuild -exportArchive ({method})"));
    let mut cmd = Command::new("xcodebuild");
    cmd.current_dir(project.root.join("platform/ios"))
        .arg("-exportArchive")
        .arg("-archivePath")
        .arg(&archive)
        .arg("-exportPath")
        .arg(&export_dir)
        .arg("-exportOptionsPlist")
        .arg(&export_plist)
        .arg("-allowProvisioningUpdates");
    if let Some((k, i, p)) = &asc {
        cmd.args(["-authenticationKeyID", k])
            .args(["-authenticationKeyIssuerID", i])
            .arg("-authenticationKeyPath")
            .arg(std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)));
    }
    run_tool(&mut cmd, "xcodebuild -exportArchive").map_err(PackError::Sign)?;

    let ipa = std::fs::read_dir(&export_dir)
        .map_err(|e| PackError::Other(e.to_string()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("ipa"))
        .ok_or_else(|| {
            PackError::Other(format!("no .ipa exported under {}", export_dir.display()))
        })?;
    let out = dist.join(format!("{name}{}.ipa", opts.version_tag(version)));
    std::fs::copy(&ipa, &out).map_err(|e| PackError::Other(e.to_string()))?;
    Ok(Artifact {
        path: out,
        kind: "ipa",
        sha256: String::new(),
        tier: SignTier::Release,
    })
}

/// A configured-but-optional signing field: absent config stays None; an unset secret degrades.
fn resolve_field(raw: Option<&String>, what: &str) -> Result<Option<String>, PackError> {
    match raw {
        None => Ok(None),
        Some(r) => resolve_degradable(r, what).map_err(PackError::Sign),
    }
}

/// ExportOptions for automatic signing; Xcode ≥15.4 names ("app-store-connect", "release-testing").
pub(crate) fn export_options(method: &str, team: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>{method}</string>
  <key>teamID</key><string>{team}</string>
  <key>signingStyle</key><string>automatic</string>
  <key>uploadSymbols</key><true/>
  <key>destination</key><string>export</string>
</dict></plist>
"#
    )
}

/// `xcodebuild archive` needs a scheme (targets aren't archivable). The scaffold ships none — the
/// pbxproj carries stable synthetic ids, so generate a shared Runner scheme on demand, parsing the
/// native-target id and product name out of the pbxproj.
fn ensure_shared_scheme(project: &Project) -> Result<(), String> {
    let xcodeproj = project.root.join("platform/ios/DayApp.xcodeproj");
    let scheme = xcodeproj.join("xcshareddata/xcschemes/Runner.xcscheme");
    if scheme.exists() {
        return Ok(());
    }
    let pbxproj = std::fs::read_to_string(xcodeproj.join("project.pbxproj"))
        .map_err(|e| format!("read pbxproj: {e}"))?;
    let target_id =
        find_native_target_id(&pbxproj).ok_or("no PBXNativeTarget found in project.pbxproj")?;
    let product = pbxproj
        .lines()
        .find(|l| l.contains("explicitFileType = wrapper.application"))
        .and_then(|l| l.split("path = ").nth(1))
        .and_then(|s| s.split(';').next())
        .map(str::trim)
        .ok_or("no application product reference in project.pbxproj")?;
    std::fs::create_dir_all(scheme.parent().unwrap()).map_err(|e| e.to_string())?;
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1500" version="1.7">
  <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
    <BuildActionEntries>
      <BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">
        <BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{target_id}" BuildableName="{product}" BlueprintName="Runner" ReferencedContainer="container:DayApp.xcodeproj"/>
      </BuildActionEntry>
    </BuildActionEntries>
  </BuildAction>
  <ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/>
  <LaunchAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.DebuggerFoundation.Launcher.LLDB" launchStyle="0" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugDocumentVersioning="YES" debugServiceExtension="internal" allowLocationSimulation="YES">
    <BuildableProductRunnable runnableDebuggingMode="0">
      <BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{target_id}" BuildableName="{product}" BlueprintName="Runner" ReferencedContainer="container:DayApp.xcodeproj"/>
    </BuildableProductRunnable>
  </LaunchAction>
</Scheme>
"#
    );
    std::fs::write(&scheme, xml).map_err(|e| e.to_string())?;
    status("Generated", &format!("{}", scheme.display()));
    Ok(())
}

/// The 24-hex-digit object id of the first PBXNativeTarget (`XXXX /* Name */ = {` whose next line
/// declares `isa = PBXNativeTarget`).
fn find_native_target_id(pbxproj: &str) -> Option<String> {
    let mut lines = pbxproj.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim();
        if let Some(rest) = t.strip_suffix("= {")
            && let Some(id) = rest.split_whitespace().next()
            && id.len() == 24
            && id.chars().all(|c| c.is_ascii_hexdigit())
            && lines
                .peek()
                .is_some_and(|n| n.trim() == "isa = PBXNativeTarget;")
        {
            return Some(id.to_string());
        }
    }
    None
}

/// The MVP fallback: zipped Simulator .app (installable via `simctl install`).
fn sim_zip(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
) -> Result<Artifact, PackError> {
    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;
    let name = &project.manifest.app.name;
    let version = &project.manifest.app.version;
    let out = dist.join(format!("{name}{}-sim.app.zip", opts.version_tag(version)));
    let _ = std::fs::remove_file(&out);
    run_tool(
        Command::new("ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&outcome.artifact)
            .arg(&out),
        "ditto zip",
    )
    .map_err(PackError::Other)?;
    Ok(Artifact {
        path: out,
        kind: "sim-app",
        sha256: String::new(),
        tier: SignTier::Unsigned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_options_plist_shape() {
        let plist = export_options("app-store-connect", "TEAM123");
        assert!(plist.contains("<key>method</key><string>app-store-connect</string>"));
        assert!(plist.contains("<key>teamID</key><string>TEAM123</string>"));
        assert!(plist.contains("<key>signingStyle</key><string>automatic</string>"));
    }

    #[test]
    fn native_target_id_from_pbxproj() {
        let pbx = "\t\tDA0000000000000000000020 /* Runner */ = {\n\t\t\tisa = PBXNativeTarget;\n";
        assert_eq!(
            find_native_target_id(pbx).as_deref(),
            Some("DA0000000000000000000020")
        );
        assert_eq!(find_native_target_id("nothing here"), None);
    }
}
