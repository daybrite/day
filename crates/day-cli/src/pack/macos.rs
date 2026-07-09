//! macos-appkit → .app assembly → codesign (inside-out, never `--deep`) → .dmg (UDZO) →
//! notarytool submit (ASC API key) → stapler → verify. Stage order is normative (hoppack lineage,
//! DESIGN.md §16.5). Ad-hoc signing remains the default when no identity is configured.

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
    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;
    let name = &project.manifest.app.name;
    let title = project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| name.clone());
    let version = &project.manifest.app.version;

    // --- assemble ---------------------------------------------------------
    let stage = project.root.join("build/day/pack/macos-appkit");
    let app = stage.join(format!("{title}.app"));
    let _ = std::fs::remove_dir_all(&stage);
    let macos_dir = app.join("Contents/MacOS");
    let res_dir = app.join("Contents/Resources");
    std::fs::create_dir_all(&macos_dir).map_err(|e| PackError::Other(e.to_string()))?;
    std::fs::create_dir_all(&res_dir).map_err(|e| PackError::Other(e.to_string()))?;
    std::fs::copy(&outcome.artifact, macos_dir.join(name))
        .map_err(|e| PackError::Other(e.to_string()))?;
    let assets = project.root.join("assets");
    if assets.is_dir() {
        super::copy_tree(&assets, &res_dir.join("assets")).map_err(PackError::Other)?;
    }
    let icon_entry = build_icns(project, &res_dir)
        .map(|_| "  <key>CFBundleIconFile</key><string>AppIcon</string>\n")
        .unwrap_or_default();
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
{icon_entry}</dict></plist>
"#,
        id = project.manifest.app.id,
        build = project.manifest.app.build,
    );
    std::fs::write(app.join("Contents/Info.plist"), plist)
        .map_err(|e| PackError::Other(e.to_string()))?;

    // --- sign ---------------------------------------------------------------
    let tier = if opts.no_sign {
        status("Signing", "skipped (--no-sign)");
        SignTier::Unsigned
    } else {
        sign_app(project, &app).map_err(PackError::Sign)?
    };

    // --- package (dmg) ------------------------------------------------------
    // The staging folder holds the .app plus an /Applications symlink for drag-install.
    #[cfg(unix)]
    {
        let link = stage.join("Applications");
        if !link.exists() {
            let _ = std::os::unix::fs::symlink("/Applications", &link);
        }
    }
    let dmg = dist.join(format!("{title}-{version}.dmg"));
    let _ = std::fs::remove_file(&dmg);
    status("Packing", "hdiutil create (UDZO)");
    run_tool(
        Command::new("hdiutil")
            .args(["create", "-quiet", "-volname", &title, "-srcfolder"])
            .arg(&stage)
            .args(["-ov", "-format", "UDZO"])
            .arg(&dmg),
        "hdiutil",
    )
    .map_err(PackError::Other)?;

    if tier == SignTier::Release {
        // The dmg container gets its own signature (Developer ID Application identity).
        let identity = resolved_identity(project)
            .map_err(PackError::Sign)?
            .unwrap();
        status("Signing", "codesign (dmg)");
        run_tool(
            Command::new("codesign")
                .args(["--force", "--timestamp", "-s", &identity])
                .args(["-i", &format!("{}.dmg", project.manifest.app.id)])
                .arg(&dmg),
            "codesign (dmg)",
        )
        .map_err(PackError::Sign)?;

        // --- notarize + staple (outermost container only) --------------------
        if !opts.no_notarize {
            notarize(project, opts, &dmg)?;
        } else {
            status("Notarize", "skipped (--no-notarize)");
        }
    }

    Ok(Artifact {
        path: dmg,
        kind: "dmg",
        sha256: String::new(),
        tier,
    })
}

/// The resolved signing identity: None/"-" ⇒ ad-hoc. A missing secret degrades (§20), it never fails.
fn resolved_identity(project: &Project) -> Result<Option<String>, String> {
    let Some(mac) = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
    else {
        return Ok(None);
    };
    let Some(raw) = mac.identity.as_ref() else {
        return Ok(None);
    };
    match resolve_degradable(raw, "signing.macos.identity")? {
        Some(id) if id != "-" && !id.is_empty() => Ok(Some(id)),
        _ => Ok(None),
    }
}

/// Sign the bundle inside-out: nested code first (Frameworks, non-main executables), the bundle
/// last. Never `--deep` — Apple's guidance, and the class of bug that bit macdeployqt/Tauri.
fn sign_app(project: &Project, app: &Path) -> Result<SignTier, String> {
    // Finder metadata xattrs make codesign fail with "resource fork, Finder information..." — strip.
    let _ = Command::new("xattr").args(["-crs"]).arg(app).status();

    let identity = resolved_identity(project)?;
    let entitlements: Option<PathBuf> = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
        .and_then(|m| m.entitlements.as_ref())
        .map(|p| project.root.join(p));
    if let Some(e) = &entitlements
        && !e.exists()
    {
        return Err(format!("entitlements file not found: {}", e.display()));
    }

    let mut nested = nested_signables(app);
    nested.push(app.to_path_buf()); // the bundle itself is signed LAST

    match &identity {
        Some(id) => {
            status("Signing", &format!("codesign ({id})"));
            for item in &nested {
                let mut cmd = Command::new("codesign");
                cmd.args(["--force", "--timestamp", "--options", "runtime", "-s", id]);
                // Entitlements apply to the main executable (via the bundle), never to dylibs.
                if item == app
                    && let Some(e) = &entitlements
                {
                    cmd.arg("--entitlements").arg(e);
                }
                cmd.arg(item);
                run_tool(&mut cmd, "codesign")?;
            }
            Ok(SignTier::Release)
        }
        None => {
            status("Signing", "ad-hoc codesign (no signing.macos.identity)");
            for item in &nested {
                run_tool(
                    Command::new("codesign")
                        .args(["--force", "-s", "-"])
                        .arg(item),
                    "codesign (ad-hoc)",
                )?;
            }
            Ok(SignTier::DevSigned)
        }
    }
}

/// Nested code that must be signed before the bundle: dylibs and frameworks under
/// Contents/Frameworks, and helper executables in Contents/MacOS beyond the main binary.
/// Today's Day bundles carry none of these — the walk future-proofs piece-contributed dylibs.
fn nested_signables(app: &Path) -> Vec<PathBuf> {
    let mut items = Vec::new();
    let frameworks = app.join("Contents/Frameworks");
    if let Ok(entries) = std::fs::read_dir(&frameworks) {
        for e in entries.flatten() {
            items.push(e.path()); // dylib or .framework — codesign handles either
        }
    }
    items
}

/// notarytool submit → (wait) → staple → gatekeeper verify.
fn notarize(project: &Project, opts: &PackOptions, dmg: &Path) -> Result<(), PackError> {
    let Some(not) = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
        .and_then(|m| m.notarize.as_ref())
    else {
        status(
            "Notarize",
            "skipped — no signing.macos.notarize config (artifact will not pass Gatekeeper)",
        );
        return Ok(());
    };
    // Missing notary secrets degrade to "signed but not notarized" (§20) — loudly, never fatally.
    let resolved = (
        resolve_degradable(&not.key_id, "signing.macos.notarize.key-id")
            .map_err(PackError::Sign)?,
        resolve_degradable(&not.issuer, "signing.macos.notarize.issuer")
            .map_err(PackError::Sign)?,
        resolve_degradable(&not.key_path, "signing.macos.notarize.key-path")
            .map_err(PackError::Sign)?,
    );
    let (Some(key_id), Some(issuer), Some(key_path)) = resolved else {
        status(
            "Notarize",
            "skipped — notary secrets unresolved (artifact is signed but NOT notarized)",
        );
        return Ok(());
    };
    if !Path::new(&key_path).exists() {
        return Err(PackError::Sign(format!(
            "notarize key file not found: {key_path}"
        )));
    }

    status("Notarize", "notarytool submit");
    let mut cmd = Command::new("xcrun");
    cmd.args(["notarytool", "submit"])
        .arg(dmg)
        .args(["--key", &key_path, "--key-id", &key_id, "--issuer", &issuer])
        .args(["--output-format", "json"]);
    if !opts.no_wait {
        cmd.arg("--wait");
    }
    let out = cmd
        .output()
        .map_err(|e| PackError::Sign(format!("notarytool: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_default();
    let id = json["id"].as_str().unwrap_or("<unknown>").to_string();
    if !out.status.success() {
        return Err(PackError::Sign(format!(
            "notarytool submit failed (submission {id}):\n{}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    if opts.no_wait {
        status(
            "Notarize",
            &format!("submitted {id} — check later: day sign --notarize-status {id}"),
        );
        return Ok(());
    }
    let notary_status = json["status"].as_str().unwrap_or("");
    if notary_status != "Accepted" {
        // Pull the notary log into the error — the status alone ("Invalid") is undiagnosable.
        let log = Command::new("xcrun")
            .args(["notarytool", "log", &id])
            .args(["--key", &key_path, "--key-id", &key_id, "--issuer", &issuer])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        return Err(PackError::Sign(format!(
            "notarization {notary_status} (submission {id}):\n{log}"
        )));
    }
    status("Notarize", &format!("accepted ({id})"));

    status("Stapling", "xcrun stapler staple");
    run_tool(
        Command::new("xcrun").args(["stapler", "staple"]).arg(dmg),
        "stapler",
    )
    .map_err(PackError::Sign)?;

    // Verify what Gatekeeper will actually do with the shipped container.
    let ok = Command::new("spctl")
        .args(["-a", "-t", "open", "--context", "context:primary-signature"])
        .arg(dmg)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        status("Verified", "spctl accepts the notarized dmg");
    } else {
        status(
            "Warning",
            "spctl did not accept the dmg (check staple/notarization)",
        );
    }
    Ok(())
}

/// Build Contents/Resources/AppIcon.icns from the project's icons/macos PNGs via sips + iconutil.
/// Best-effort: a missing icon set or tool must not fail the pack (the dmg just has no icon).
fn build_icns(project: &Project, res_dir: &Path) -> Option<()> {
    let source = crate::resources::app_icon(project, "appkit")?;
    let iconset = project.root.join("build/day/pack/AppIcon.iconset");
    let _ = std::fs::remove_dir_all(&iconset);
    std::fs::create_dir_all(&iconset).ok()?;
    // The canonical iconset slots, rendered from the largest available PNG.
    for (px, name) in [
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ] {
        let px_s = px.to_string();
        let ok = Command::new("sips")
            .args(["-z", &px_s, &px_s])
            .arg(&source)
            .arg("--out")
            .arg(iconset.join(name))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
    }
    let ok = Command::new("iconutil")
        .args(["-c", "icns", "-o"])
        .arg(res_dir.join("AppIcon.icns"))
        .arg(&iconset)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then_some(())
}
