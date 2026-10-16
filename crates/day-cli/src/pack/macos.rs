// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! macos-appkit → the Xcode-built .app, taken as-is → codesign (inside-out, never `--deep`) →
//! .dmg (UDZO) → notarytool submit (ASC API key) → stapler → verify. Stage order is normative
//! (hoppack lineage, DESIGN.md §16.5). Ad-hoc signing remains the default when no identity is
//! configured. The bundle arrives complete from `day build` (§17.4): identity and version from
//! the xcconfig conveyance, the compiled appiconset, and resources staged into
//! `Contents/Resources` by the `day xcode-backend stage-resources` phase. This packer copies
//! it, re-signs it, and wraps it; it assembles nothing.
//!
//! `--formats pkg` is the Mac App Store's package instead (docs/packaging.md "The Mac App
//! Store"): the same bundle, its Mac App Store profile embedded, signed with the distribution
//! identity, and wrapped by `productbuild` in an installer signed with the installer identity.
//! [`mac_app_store_pkg`] does that part, and `day sign apply` calls it for a bundle built
//! elsewhere, which is how a release pipeline signs after the build.

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
    formats: &[String],
) -> Result<Vec<Artifact>, PackError> {
    let outcome = ops::build(project, target, opts.profile).map_err(PackError::Other)?;
    if outcome.artifact.extension().and_then(|e| e.to_str()) != Some("app") {
        return Err(PackError::Other(format!(
            "expected an .app bundle from the macos-appkit build, got {}",
            outcome.artifact.display()
        )));
    }
    let mut artifacts = Vec::new();
    if formats.iter().any(|f| f == "dmg") {
        artifacts.push(dmg(project, target, opts, dist, &outcome.artifact)?);
    }
    if formats.iter().any(|f| f == "pkg") {
        artifacts.push(store_pkg(project, target, opts, dist, &outcome.artifact)?);
    }
    Ok(artifacts)
}

/// The app's display title: what /Applications shows, so what the packed bundle is named.
fn display_title(project: &Project) -> String {
    project
        .manifest
        .app
        .title
        .clone()
        .unwrap_or_else(|| project.manifest.app.name.clone())
}

/// A copy of the built bundle under `stage`, renamed to the display title: signing mutates the
/// bundle, and the build output must stay reusable. The SBOM goes into it when [sbom] mode =
/// embed (§20.4), so the app can read its own license notices at runtime.
fn stage_bundle(project: &Project, built: &Path, stage: &Path) -> Result<PathBuf, PackError> {
    let app = stage.join(format!("{}.app", display_title(project)));
    let _ = std::fs::remove_dir_all(stage);
    std::fs::create_dir_all(stage).map_err(|e| PackError::Other(e.to_string()))?;
    super::copy_tree(built, &app).map_err(PackError::Other)?;
    if project.manifest.sbom.mode == crate::meta::SbomMode::Embed {
        crate::provenance::embed_into(
            &project.root.join("build/day/sbom"),
            &app.join("Contents/Resources"),
        )
        .map_err(PackError::Other)?;
    }
    Ok(app)
}

/// The direct download: a Developer ID (or ad-hoc) signed bundle in a notarized .dmg.
fn dmg(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
    built: &Path,
) -> Result<Artifact, PackError> {
    let title = display_title(project);

    // --- stage --------------------------------------------------------------
    // The release workflow hands the bundle staged here to its signing job, so the path is part
    // of that contract.
    let stage = project.root.join("build/day/pack/macos-appkit");
    let app = stage_bundle(project, built, &stage)?;

    // --- sign ---------------------------------------------------------------
    let tier = if opts.no_sign {
        status("Signing", "skipped (--no-sign)");
        SignTier::Unsigned
    } else {
        sign_app(project, &app, opts.profile).map_err(PackError::Sign)?
    };

    crate::sandbox::verify(project, &app, opts.profile).map_err(PackError::Sign)?;

    // --- package (dmg) ------------------------------------------------------
    // The staging folder holds the .app plus an /Applications symlink for drag-install.
    #[cfg(unix)]
    {
        let link = stage.join("Applications");
        if !link.exists() {
            let _ = std::os::unix::fs::symlink("/Applications", &link);
        }
    }
    // The .app inside keeps the display title (that is what /Applications shows); the container
    // takes the shared release name, so it matches every other target's artifact (§20.4).
    let dmg = dist.join(super::naming::artifact_file(
        project,
        target,
        opts,
        &[],
        "dmg",
    ));
    let _ = std::fs::remove_file(&dmg);
    status("Packing", "hdiutil create (UDZO)");
    create_dmg(
        Command::new("hdiutil")
            .args(["create", "-volname", &title, "-srcfolder"])
            .arg(&stage)
            .args(["-ov", "-format", "UDZO"])
            .arg(&dmg),
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

/// Disk Arbitration can briefly keep disk-image creation busy on hosted macOS runners.
/// Retry only that transient failure; keep diagnostics (including under --verbose) so a
/// different failure is actionable. In particular, `hdiutil -quiet` hides its errors too.
fn create_dmg(cmd: &mut Command) -> Result<(), String> {
    for attempt in 1..=3 {
        let out = crate::ops::run_capture(cmd, "hdiutil create")?;
        if out.status.success() {
            return Ok(());
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < 3 && text.to_ascii_lowercase().contains("resource busy") {
            status(
                "Warning",
                "hdiutil: resource busy; retrying disk-image creation",
            );
            std::thread::sleep(std::time::Duration::from_secs(attempt));
            continue;
        }
        let tail: Vec<_> = text.lines().rev().take(25).collect();
        return Err(format!(
            "hdiutil create failed ({}):\n{}",
            out.status,
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    unreachable!("the last attempt returns its error")
}

#[cfg(all(test, unix))]
mod dmg_tests {
    use super::*;

    fn fake_create(name: &str, message: &str, failures: usize) -> (Result<(), String>, usize) {
        let dir = std::env::temp_dir().join(format!("day-dmg-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let counter = dir.join("attempts");
        std::fs::write(&counter, "").unwrap();
        let result = create_dmg(Command::new("sh").args([
            "-c",
            "echo attempt >> \"$1\"; n=$(wc -l < \"$1\"); if [ \"$n\" -le \"$3\" ]; then echo \"$2\" >&2; exit 1; fi",
            "fake-hdiutil",
            counter.to_str().unwrap(),
            message,
            &failures.to_string(),
        ]));
        let attempts = std::fs::read_to_string(counter).unwrap().lines().count();
        std::fs::remove_dir_all(dir).unwrap();
        (result, attempts)
    }

    #[test]
    fn busy_image_creation_is_retried() {
        let (result, attempts) = fake_create("busy", "hdiutil: create failed - Resource busy", 2);
        result.unwrap();
        assert_eq!(attempts, 3);
    }

    #[test]
    fn permanent_image_errors_keep_their_diagnostics_and_are_not_retried() {
        let (result, attempts) = fake_create("full", "hdiutil: No space left on device", 3);
        assert!(result.unwrap_err().contains("No space left on device"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn busy_image_retries_are_bounded() {
        let (result, attempts) = fake_create("stuck", "hdiutil: create failed - Resource busy", 4);
        assert!(result.unwrap_err().contains("Resource busy"));
        assert_eq!(attempts, 3);
    }
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
/// last. Never `--deep`: Apple's guidance, and the class of bug that bit macdeployqt/Tauri.
fn sign_app(
    project: &Project,
    app: &Path,
    profile: crate::cli::Profile,
) -> Result<SignTier, String> {
    // Finder metadata xattrs make codesign fail with "resource fork, Finder information...", so
    // strip them.
    let _ = Command::new("xattr").args(["-crs"]).arg(app).status();

    let identity = resolved_identity(project)?;
    let entitlements = crate::sandbox::entitlements(project, profile)?;

    let mut nested = nested_signables(app);
    nested.push(app.to_path_buf()); // the bundle itself is signed last

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
                let mut cmd = Command::new("codesign");
                cmd.args(["--force", "-s", "-"]);
                if item == app
                    && let Some(e) = &entitlements
                {
                    cmd.arg("--entitlements").arg(e);
                }
                run_tool(cmd.arg(item), "codesign (ad-hoc)")?;
            }
            Ok(SignTier::DevSigned)
        }
    }
}

/// Nested code that must be signed before the bundle: dylibs and frameworks under
/// Contents/Frameworks, plug-ins, helpers, XPC services and login items. Today's Day bundles carry
/// none of these; the walk future-proofs piece-contributed code. Sorted, so the order (and with it
/// any failure) is the same on every run.
fn nested_signables(app: &Path) -> Vec<PathBuf> {
    let mut items = Vec::new();
    for dir in [
        "Contents/Frameworks",
        "Contents/PlugIns",
        "Contents/Helpers",
        "Contents/XPCServices",
        "Contents/Library/LoginItems",
    ] {
        if let Ok(entries) = std::fs::read_dir(app.join(dir)) {
            for e in entries.flatten() {
                items.push(e.path()); // a dylib or a bundle; codesign handles either
            }
        }
    }
    items.sort();
    items
}

// --- The Mac App Store ------------------------------------------------------------------------
//
// The store takes an installer package, not a disk image: `productbuild` wraps the bundle for
// /Applications and signs the package with the installer identity, and the bundle inside is signed
// with the distribution identity over the Mac App Store profile it embeds. These are the steps
// Xcode's "App Store Connect" export performs, written out so a bundle built anywhere (including
// a CI leg holding no keys) can be signed without the project that built it.

/// What one Mac App Store package is signed with, already resolved.
pub(crate) struct MasMaterial {
    /// The Mac App Store provisioning profile (`.provisionprofile`).
    pub profile: PathBuf,
    /// The app's identity ("Apple Distribution: …"), by name or SHA-1.
    pub identity: String,
    /// The installer's identity ("3rd Party Mac Developer Installer: …"), by name.
    pub installer_identity: String,
    /// The entitlements to sign with. `None` keeps the ones the bundle is signed with already: a
    /// Day build signs its sandbox policy in, even ad hoc, and that policy is what a signer without
    /// the project has to go on.
    pub entitlements: Option<PathBuf>,
}

/// The Mac App Store package from `[signing.macos.app-store]`, or, without that material, an
/// unsigned installer around the ad-hoc bundle, said loudly. The direct download's staging
/// directory is left alone: the release workflow hands that bundle to its own signing job.
fn store_pkg(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
    built: &Path,
) -> Result<Artifact, PackError> {
    let stage = project.root.join("build/day/pack/macos-appkit-store");
    let app = stage_bundle(project, built, &stage)?;
    let entitlements =
        crate::sandbox::entitlements(project, opts.profile).map_err(PackError::Sign)?;
    let material = if opts.no_sign {
        status("Signing", "skipped (--no-sign)");
        None
    } else {
        store_material(project, entitlements.clone()).map_err(PackError::Sign)?
    };
    match material {
        Some(m) => {
            let pkg = dist.join(super::naming::artifact_file(
                project,
                target,
                opts,
                &[],
                "pkg",
            ));
            status(
                "Signing",
                &format!("for the Mac App Store ({})", m.identity),
            );
            mac_app_store_pkg(&app, &pkg, &m).map_err(PackError::Sign)?;
            status(
                "Packed",
                &format!("{} (installer: {})", pkg.display(), m.installer_identity),
            );
            Ok(Artifact {
                path: pkg,
                kind: "pkg",
                sha256: String::new(),
                tier: SignTier::Release,
            })
        }
        None => {
            // Ad hoc with the app's entitlements, so the bundle inside still carries its sandbox
            // policy for the `day sign apply` that finishes the job.
            status("Signing", "ad-hoc codesign (no Mac App Store material)");
            for item in nested_signables(&app) {
                run_tool(
                    Command::new("codesign")
                        .args(["--force", "-s", "-"])
                        .arg(&item),
                    "codesign (ad-hoc)",
                )
                .map_err(PackError::Sign)?;
            }
            let mut cmd = Command::new("codesign");
            cmd.args(["--force", "-s", "-"]);
            if let Some(e) = &entitlements {
                cmd.arg("--entitlements").arg(e);
            }
            run_tool(cmd.arg(&app), "codesign (ad-hoc)").map_err(PackError::Sign)?;
            let pkg = dist.join(super::naming::artifact_file(
                project,
                target,
                opts,
                &["unsigned"],
                "pkg",
            ));
            let _ = std::fs::remove_file(&pkg);
            status("Packing", "productbuild (unsigned)");
            run_tool(
                Command::new("productbuild")
                    .arg("--component")
                    .arg(&app)
                    .arg("/Applications")
                    .arg(&pkg),
                "productbuild",
            )
            .map_err(PackError::Other)?;
            Ok(Artifact {
                path: pkg,
                kind: "pkg",
                sha256: String::new(),
                tier: SignTier::Unsigned,
            })
        }
    }
}

/// `[signing.macos.app-store]`, resolved the way pack resolves every signing section (§20): a
/// variable that is not set degrades to an unsigned package with a warning naming it; a profile
/// that is named but missing is an error, since that is a typo rather than an absent secret.
fn store_material(
    project: &Project,
    entitlements: Option<PathBuf>,
) -> Result<Option<MasMaterial>, String> {
    let Some(m) = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
        .and_then(|m| m.app_store.as_ref())
    else {
        status(
            "Warning",
            "no [signing.macos.app-store] in Day.toml — packing an UNSIGNED .pkg, which the Mac \
             App Store refuses; sign it with `day sign apply` or add the table \
             (docs/packaging.md#the-mac-app-store)",
        );
        return Ok(None);
    };
    let identity = resolve_degradable(&m.identity, "signing.macos.app-store.identity")?;
    let installer = resolve_degradable(
        &m.installer_identity,
        "signing.macos.app-store.installer-identity",
    )?;
    let profile = resolve_degradable(&m.profile, "signing.macos.app-store.profile")?;
    let (Some(identity), Some(installer_identity), Some(profile)) = (identity, installer, profile)
    else {
        return Ok(None);
    };
    let profile = project.root.join(profile);
    if !profile.is_file() {
        return Err(format!(
            "signing.macos.app-store.profile not found: {}",
            profile.display()
        ));
    }
    Ok(Some(MasMaterial {
        profile,
        identity,
        installer_identity,
        entitlements,
    }))
}

/// Sign `app` for the Mac App Store and wrap it in a signed installer package at `pkg`,
/// returning the signing certificate's sha256. The input bundle is never modified: the work
/// happens on a copy beside `pkg`, and `pkg` is written only once the package verifies.
///
/// What the store would refuse is refused here first, in words: a profile for another app, a
/// development or Developer ID profile, an expired one, a bundle with no sandbox, no
/// `LSApplicationCategoryType`, or a device entitlement without the usage description macOS
/// shows at the prompt.
pub(crate) fn mac_app_store_pkg(
    app: &Path,
    pkg: &Path,
    m: &MasMaterial,
) -> Result<Option<String>, String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "a Mac App Store package needs macOS: codesign, productbuild and pkgutil ship with \
             Xcode"
                .into(),
        );
    }
    let work = pkg.with_extension("day-signing.work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("mkdir {}: {e}", work.display()))?;
    let result = (|| -> Result<(PathBuf, Option<String>), String> {
        let name = app
            .file_name()
            .ok_or_else(|| format!("{} has no name", app.display()))?;
        let bundle = work.join(name);
        // ditto, the bundle-aware copy: it keeps symlinks, modes and the bundle's structure.
        run_tool(Command::new("ditto").arg(app).arg(&bundle), "ditto")?;

        let info = plist::Value::from_file(bundle.join("Contents/Info.plist"))
            .map_err(|e| format!("{}: Contents/Info.plist: {e}", app.display()))?
            .into_dictionary()
            .ok_or("Contents/Info.plist is not a dictionary")?;
        let bundle_id = info
            .get("CFBundleIdentifier")
            .and_then(plist::Value::as_string)
            .ok_or("Contents/Info.plist has no CFBundleIdentifier")?
            .to_string();
        let profile = decode_profile(&m.profile, &work)?;
        let team = check_store_profile(&profile, &bundle_id, std::time::SystemTime::now())?;
        let granted = profile
            .get("Entitlements")
            .and_then(plist::Value::as_dictionary)
            .cloned()
            .unwrap_or_default();
        let requested = match &m.entitlements {
            Some(path) => plist::Value::from_file(path)
                .map_err(|e| format!("entitlements {}: {e}", path.display()))?
                .into_dictionary()
                .ok_or("the entitlements are not a plist dictionary")?,
            None => signed_entitlements(&bundle)?,
        };
        let entitlements = store_entitlements(requested, &granted, &team, &bundle_id)?;
        let problems = store_problems(&info, &entitlements);
        if !problems.is_empty() {
            return Err(format!(
                "the Mac App Store would refuse this bundle:\n  {}",
                problems.join("\n  ")
            ));
        }
        let ents_path = work.join("entitlements.plist");
        plist::Value::Dictionary(entitlements)
            .to_file_xml(&ents_path)
            .map_err(|e| format!("write {}: {e}", ents_path.display()))?;

        // The profile the store checks the signature against. Its bytes only: a profile
        // downloaded from the developer portal carries the browser's `com.apple.quarantine`
        // attribute, which `fs::copy` would carry along, and App Store Connect rejects a package
        // holding any quarantined file (ITMS-91109) after the upload has already succeeded.
        let profile_bytes =
            std::fs::read(&m.profile).map_err(|e| format!("read {}: {e}", m.profile.display()))?;
        std::fs::write(
            bundle.join("Contents/embedded.provisionprofile"),
            profile_bytes,
        )
        .map_err(|e| format!("embed the profile: {e}"))?;
        // Every extended attribute off, after the last file went in: quarantine for the store,
        // Finder metadata because codesign fails on it ("resource fork, Finder information…").
        run_tool(Command::new("xattr").arg("-cr").arg(&bundle), "xattr -cr")?;
        let quarantined = quarantined_files(&bundle)?;
        if !quarantined.is_empty() {
            return Err(format!(
                "files still carry com.apple.quarantine, which App Store Connect rejects \
                 (ITMS-91109): {}",
                quarantined.join(", ")
            ));
        }
        // Inside out: a nested bundle's signature is part of what seals the bundle above it.
        for nested in nested_signables(&bundle) {
            store_codesign(&nested, &m.identity, None)?;
        }
        store_codesign(&bundle, &m.identity, Some(&ents_path))?;
        run_tool(
            Command::new("codesign")
                .args(["--verify", "--deep", "--strict", "--verbose=2"])
                .arg(&bundle),
            "codesign --verify",
        )?;
        let fingerprint = crate::sign::leaf_certificate_sha256(&bundle, &work);

        let out = work.join("store.pkg");
        status("Packing", "productbuild (Mac App Store installer)");
        run_tool(
            Command::new("productbuild")
                .arg("--component")
                .arg(&bundle)
                .arg("/Applications")
                .args(["--sign", &m.installer_identity])
                .arg(&out),
            "productbuild",
        )?;
        run_tool(
            Command::new("pkgutil").arg("--check-signature").arg(&out),
            "pkgutil --check-signature",
        )?;
        Ok((out, fingerprint))
    })();
    let (out, fingerprint) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(pkg);
    std::fs::rename(&out, pkg).map_err(|e| format!("write {}: {e}", pkg.display()))?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(fingerprint)
}

/// The files under `bundle` that carry `com.apple.quarantine`, relative to it. `xattr -lr`
/// prints one `<path>: <name>: <value>` line per attribute.
fn quarantined_files(bundle: &Path) -> Result<Vec<String>, String> {
    let out = crate::ops::run_capture(Command::new("xattr").arg("-lr").arg(bundle), "xattr -lr")?;
    Ok(quarantine_paths(
        &String::from_utf8_lossy(&out.stdout),
        bundle,
    ))
}

/// The paths in an `xattr -lr` listing whose attribute is `com.apple.quarantine`.
fn quarantine_paths(listing: &str, bundle: &Path) -> Vec<String> {
    let root = format!("{}/", bundle.display());
    let mut paths: Vec<String> = listing
        .lines()
        .filter_map(|line| line.split_once(": com.apple.quarantine:").map(|(p, _)| p))
        .map(|p| p.strip_prefix(&root).unwrap_or(p).to_string())
        .collect();
    paths.dedup();
    paths
}

/// Sign one bundle or binary for the store. Hardened runtime as the direct download has it; the
/// entitlements in the DER form current macOS checks as well as the XML one.
fn store_codesign(
    target: &Path,
    identity: &str,
    entitlements: Option<&Path>,
) -> Result<(), String> {
    let mut cmd = Command::new("codesign");
    cmd.args([
        "--force",
        "--sign",
        identity,
        "--options",
        "runtime",
        "--generate-entitlement-der",
    ]);
    if let Some(e) = entitlements {
        cmd.arg("--entitlements").arg(e);
    }
    run_tool(cmd.arg(target), "codesign")
}

/// The provisioning profile's own plist: a profile is CMS-signed, so it is decoded first.
fn decode_profile(profile: &Path, work: &Path) -> Result<plist::Dictionary, String> {
    let decoded = work.join("profile.plist");
    run_tool(
        Command::new("security")
            .args(["cms", "-D", "-i"])
            .arg(profile)
            .arg("-o")
            .arg(&decoded),
        "security cms",
    )?;
    plist::Value::from_file(&decoded)
        .map_err(|e| format!("{}: {e}", profile.display()))?
        .into_dictionary()
        .ok_or_else(|| format!("{} is not a provisioning profile", profile.display()))
}

/// The entitlements a bundle is signed with now, empty when it carries none.
fn signed_entitlements(bundle: &Path) -> Result<plist::Dictionary, String> {
    let out = crate::ops::run_capture(
        Command::new("codesign")
            .args(["--display", "--entitlements", "-", "--xml"])
            .arg(bundle),
        "codesign --display",
    )?;
    if !out.status.success() {
        return Err(format!(
            "{}: {}",
            bundle.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if out.stdout.is_empty() {
        return Ok(plist::Dictionary::new());
    }
    plist::Value::from_reader(std::io::Cursor::new(out.stdout))
        .map_err(|e| format!("{}: signed entitlements: {e}", bundle.display()))?
        .into_dictionary()
        .ok_or_else(|| "signed entitlements are not a dictionary".to_string())
}

/// Hold a provisioning profile to what the store takes for `bundle_id`, returning its team: a Mac
/// profile, for distribution through the store (no device list, not Developer ID), for this
/// app's id, and still valid.
fn check_store_profile(
    profile: &plist::Dictionary,
    bundle_id: &str,
    now: std::time::SystemTime,
) -> Result<String, String> {
    let name = profile
        .get("Name")
        .and_then(plist::Value::as_string)
        .unwrap_or("the profile");
    let platforms: Vec<&str> = profile
        .get("Platform")
        .and_then(plist::Value::as_array)
        .map(|a| a.iter().filter_map(plist::Value::as_string).collect())
        .unwrap_or_default();
    if !platforms.contains(&"OSX") {
        return Err(format!(
            "{name} is a profile for {}, not macOS — create a Mac App Store Connect profile",
            if platforms.is_empty() {
                "no platform".to_string()
            } else {
                platforms.join(", ")
            }
        ));
    }
    if profile.contains_key("ProvisionedDevices") {
        return Err(format!(
            "{name} is a development profile (it lists devices) — the Mac App Store needs a Mac \
             App Store Connect distribution profile"
        ));
    }
    if profile
        .get("ProvisionsAllDevices")
        .and_then(plist::Value::as_boolean)
        == Some(true)
    {
        return Err(format!(
            "{name} is a Developer ID profile — the Mac App Store needs a Mac App Store Connect \
             distribution profile"
        ));
    }
    if let Some(expires) = profile
        .get("ExpirationDate")
        .and_then(plist::Value::as_date)
        && std::time::SystemTime::from(expires) < now
    {
        return Err(format!("{name} has expired — download a fresh one"));
    }
    let team = profile
        .get("TeamIdentifier")
        .and_then(plist::Value::as_array)
        .and_then(|a| a.first())
        .and_then(plist::Value::as_string)
        .ok_or_else(|| format!("{name} names no team"))?
        .to_string();
    let app_id = profile
        .get("Entitlements")
        .and_then(plist::Value::as_dictionary)
        .and_then(|e| e.get("com.apple.application-identifier"))
        .and_then(plist::Value::as_string)
        .unwrap_or_default();
    let wanted = format!("{team}.{bundle_id}");
    let covers = app_id == wanted
        || app_id
            .strip_suffix('*')
            .is_some_and(|prefix| wanted.starts_with(prefix));
    if !covers {
        return Err(format!(
            "{name} is for {app_id:?}, not {wanted:?} — the profile must be for this app's bundle id"
        ));
    }
    Ok(team)
}

/// The entitlements the store build signs with: the app's own, less the debugger grant, plus the
/// two identifiers the profile ties the signature to. A capability the app asks for that only a
/// profile can grant (`com.apple.developer.*`, push) has to be in the profile, or the store
/// refuses the upload; that is reported here instead.
fn store_entitlements(
    mut requested: plist::Dictionary,
    granted: &plist::Dictionary,
    team: &str,
    bundle_id: &str,
) -> Result<plist::Dictionary, String> {
    requested.remove("com.apple.security.get-task-allow");
    if requested
        .get("com.apple.security.app-sandbox")
        .and_then(plist::Value::as_boolean)
        != Some(true)
    {
        return Err(
            "the bundle is not sandboxed, and the Mac App Store takes sandboxed apps only — set \
             [sandbox.macos-appkit] enabled = true in Day.toml and rebuild (docs/sandbox.md)"
                .into(),
        );
    }
    let ungranted: Vec<&str> = requested
        .keys()
        .map(String::as_str)
        .filter(|k| {
            (k.starts_with("com.apple.developer.") || *k == "aps-environment")
                && *k != "com.apple.developer.team-identifier"
                && !granted.contains_key(k)
        })
        .collect();
    if !ungranted.is_empty() {
        return Err(format!(
            "the app asks for {}, which the profile does not grant — enable the capability for \
             {bundle_id} in the developer portal and download the profile again",
            ungranted.join(", ")
        ));
    }
    requested.insert(
        "com.apple.application-identifier".into(),
        format!("{team}.{bundle_id}").into(),
    );
    requested.insert(
        "com.apple.developer.team-identifier".into(),
        team.to_string().into(),
    );
    Ok(requested)
}

/// What App Review or the upload would refuse in a bundle's Info.plist, given what it is signed to
/// do: no category, no version, or a device or personal-data entitlement without the usage
/// description macOS shows at the prompt (TCC terminates an app that has none).
fn store_problems(info: &plist::Dictionary, entitlements: &plist::Dictionary) -> Vec<String> {
    let has = |key: &str| {
        info.get(key)
            .and_then(plist::Value::as_string)
            .is_some_and(|v| !v.trim().is_empty())
    };
    let mut problems = Vec::new();
    if !has("LSApplicationCategoryType") {
        problems.push(
            "Info.plist has no LSApplicationCategoryType, which the Mac App Store requires: add \
             it to platform/macos/Runner/Info.plist (public.app-category.developer-tools, say)"
                .to_string(),
        );
    }
    for key in ["CFBundleShortVersionString", "CFBundleVersion"] {
        if !has(key) {
            problems.push(format!("Info.plist has no {key}"));
        }
    }
    // Entitlement → the usage descriptions that satisfy it (any one).
    const USAGE: &[(&str, &[&str])] = &[
        (
            "com.apple.security.device.camera",
            &["NSCameraUsageDescription"],
        ),
        (
            "com.apple.security.device.audio-input",
            &["NSMicrophoneUsageDescription"],
        ),
        (
            "com.apple.security.device.microphone",
            &["NSMicrophoneUsageDescription"],
        ),
        (
            "com.apple.security.personal-information.location",
            &[
                "NSLocationUsageDescription",
                "NSLocationWhenInUseUsageDescription",
            ],
        ),
        (
            "com.apple.security.personal-information.photos-library",
            &["NSPhotoLibraryUsageDescription"],
        ),
        (
            "com.apple.security.personal-information.addressbook",
            &["NSContactsUsageDescription"],
        ),
        (
            "com.apple.security.personal-information.calendars",
            &[
                "NSCalendarsUsageDescription",
                "NSCalendarsFullAccessUsageDescription",
            ],
        ),
    ];
    for (entitlement, keys) in USAGE {
        let on = entitlements
            .get(entitlement)
            .and_then(plist::Value::as_boolean)
            == Some(true);
        if on && !keys.iter().any(|k| has(k)) {
            problems.push(format!(
                "{entitlement} is granted but Info.plist has no {} — declare the permission in \
                 Day.toml [permissions] and rebuild (docs/permissions.md)",
                keys.join(" or ")
            ));
        }
    }
    problems
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
    // Missing notary secrets degrade to "signed but not notarized" (§20): loudly, never fatally.
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
            &format!("submitted {id} — check later: day sign status {id}"),
        );
        return Ok(());
    }
    let notary_status = json["status"].as_str().unwrap_or("");
    if notary_status != "Accepted" {
        // Pull the notary log into the error; the status alone ("Invalid") is undiagnosable.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(entries: &[(&str, plist::Value)]) -> plist::Dictionary {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn store_profile(app_id: &str) -> plist::Dictionary {
        dict(&[
            ("Name", "Example Mac App Store".into()),
            ("Platform", plist::Value::Array(vec!["OSX".into()])),
            (
                "TeamIdentifier",
                plist::Value::Array(vec!["TEAM123".into()]),
            ),
            (
                "Entitlements",
                plist::Value::Dictionary(dict(&[(
                    "com.apple.application-identifier",
                    app_id.into(),
                )])),
            ),
        ])
    }

    #[test]
    fn a_store_profile_for_this_app_names_its_team() {
        let now = std::time::SystemTime::now();
        assert_eq!(
            check_store_profile(
                &store_profile("TEAM123.dev.example.app"),
                "dev.example.app",
                now
            ),
            Ok("TEAM123".to_string())
        );
        // A wildcard profile covers the ids under it.
        assert!(check_store_profile(&store_profile("TEAM123.*"), "dev.example.app", now).is_ok());
        let other =
            check_store_profile(&store_profile("TEAM123.dev.other"), "dev.example.app", now);
        assert!(
            other
                .unwrap_err()
                .contains("not \"TEAM123.dev.example.app\"")
        );
    }

    #[test]
    fn the_wrong_kind_of_profile_is_named_for_what_it_is() {
        let now = std::time::SystemTime::now();
        let mut ios = store_profile("TEAM123.dev.example.app");
        ios.insert(
            "Platform".into(),
            plist::Value::Array(vec!["iOS".into(), "xrOS".into()]),
        );
        assert!(
            check_store_profile(&ios, "dev.example.app", now)
                .unwrap_err()
                .contains("iOS")
        );
        let mut development = store_profile("TEAM123.dev.example.app");
        development.insert(
            "ProvisionedDevices".into(),
            plist::Value::Array(vec!["00000000-0000".into()]),
        );
        assert!(
            check_store_profile(&development, "dev.example.app", now)
                .unwrap_err()
                .contains("development profile")
        );
        let mut developer_id = store_profile("TEAM123.dev.example.app");
        developer_id.insert("ProvisionsAllDevices".into(), true.into());
        assert!(
            check_store_profile(&developer_id, "dev.example.app", now)
                .unwrap_err()
                .contains("Developer ID")
        );
        let mut expired = store_profile("TEAM123.dev.example.app");
        expired.insert(
            "ExpirationDate".into(),
            plist::Value::Date(std::time::SystemTime::UNIX_EPOCH.into()),
        );
        assert!(
            check_store_profile(&expired, "dev.example.app", now)
                .unwrap_err()
                .contains("expired")
        );
    }

    #[test]
    fn store_entitlements_keep_the_policy_and_add_the_identifiers() {
        let requested = dict(&[
            ("com.apple.security.app-sandbox", true.into()),
            ("com.apple.security.device.camera", true.into()),
            ("com.apple.security.get-task-allow", true.into()),
        ]);
        let signed = store_entitlements(
            requested,
            &plist::Dictionary::new(),
            "TEAM123",
            "dev.example.app",
        )
        .unwrap();
        assert!(!signed.contains_key("com.apple.security.get-task-allow"));
        assert_eq!(
            signed
                .get("com.apple.application-identifier")
                .and_then(plist::Value::as_string),
            Some("TEAM123.dev.example.app")
        );
        assert_eq!(
            signed
                .get("com.apple.developer.team-identifier")
                .and_then(plist::Value::as_string),
            Some("TEAM123")
        );
        assert!(signed.contains_key("com.apple.security.device.camera"));
    }

    #[test]
    fn store_entitlements_refuse_an_unsandboxed_app_and_an_ungranted_capability() {
        let unsandboxed = dict(&[("com.apple.security.network.client", true.into())]);
        assert!(
            store_entitlements(unsandboxed, &plist::Dictionary::new(), "T", "a.b")
                .unwrap_err()
                .contains("sandboxed")
        );
        let icloud = dict(&[
            ("com.apple.security.app-sandbox", true.into()),
            ("com.apple.developer.icloud-services", true.into()),
        ]);
        assert!(
            store_entitlements(icloud.clone(), &plist::Dictionary::new(), "T", "a.b")
                .unwrap_err()
                .contains("com.apple.developer.icloud-services")
        );
        let granted = dict(&[("com.apple.developer.icloud-services", true.into())]);
        assert!(store_entitlements(icloud, &granted, "T", "a.b").is_ok());
    }

    #[test]
    fn a_quarantined_file_is_found_in_an_xattr_listing() {
        let bundle = Path::new("/w/Day Showcase.app");
        let listing = "/w/Day Showcase.app/Contents/embedded.provisionprofile: com.apple.quarantine: 0083;6a1b;Safari;\n\
                       /w/Day Showcase.app/Contents/embedded.provisionprofile: com.apple.metadata:kMDItemWhereFroms: bplist00\n\
                       /w/Day Showcase.app/Contents/Info.plist: com.apple.FinderInfo: 00\n";
        assert_eq!(
            quarantine_paths(listing, bundle),
            vec!["Contents/embedded.provisionprofile".to_string()]
        );
        assert!(quarantine_paths("", bundle).is_empty());
    }

    #[test]
    fn store_problems_name_a_missing_category_and_usage_description() {
        let entitlements = dict(&[
            ("com.apple.security.app-sandbox", true.into()),
            ("com.apple.security.device.camera", true.into()),
            (
                "com.apple.security.personal-information.location",
                true.into(),
            ),
        ]);
        let bare = dict(&[
            ("CFBundleShortVersionString", "1.0".into()),
            ("CFBundleVersion", "7".into()),
        ]);
        let problems = store_problems(&bare, &entitlements);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("LSApplicationCategoryType"));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("NSCameraUsageDescription"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("NSLocationUsageDescription"))
        );
        let complete = dict(&[
            ("CFBundleShortVersionString", "1.0".into()),
            ("CFBundleVersion", "7".into()),
            (
                "LSApplicationCategoryType",
                "public.app-category.developer-tools".into(),
            ),
            ("NSCameraUsageDescription", "Take a photo.".into()),
            (
                "NSLocationWhenInUseUsageDescription",
                "Show where you are.".into(),
            ),
        ]);
        assert!(store_problems(&complete, &entitlements).is_empty());
    }
}
