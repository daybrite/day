// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day sign check` (DESIGN.md §16.5) validates the presence and resolvability of the
//! Day.toml `signing:` configuration (env vars set, referenced files exist) without ever printing
//! a secret value; `day sign status <id>` polls an async notarytool submission. Actual signing
//! runs inside `day pack` (the per-format modules in pack/).

use std::path::Path;
use std::process::Command;

use crate::meta::Project;
use crate::ops::status;
use crate::pack::settings::{interpolate, interpolate_opt};

/// One section's readiness. `configured=false` is not an error; pack degrades to the dev tier.
struct Check {
    section: &'static str,
    configured: bool,
    problems: Vec<String>,
}

/// `day sign check`: exit 0 when every configured section resolves; 6 when any fails (§16.3).
/// The per-section report prints either way, so the verdict is a code, not an `error:` line;
/// the number itself comes from the kind→code map in cli.rs.
pub fn check(project: &Project) -> i32 {
    let signing = project.manifest.signing.as_ref();
    let mut checks = Vec::new();

    // --- macos --------------------------------------------------------------
    {
        let macos = signing.and_then(|s| s.macos.as_ref());
        let mut c = Check {
            section: "macos",
            configured: macos.is_some(),
            problems: Vec::new(),
        };
        if let Some(m) = macos {
            match interpolate_opt(m.identity.as_ref()) {
                Ok(Some(id)) if id == "-" || id.is_empty() => {
                    c.problems.push("identity resolves to ad-hoc".into())
                }
                Ok(_) => {}
                Err(e) => c.problems.push(e),
            }
            if let Some(e) = &m.entitlements
                && !project.root.join(e).exists()
            {
                c.problems.push(format!("entitlements file missing: {e}"));
            }
            if let Some(n) = &m.notarize {
                for (label, raw) in [("key-id", &n.key_id), ("issuer", &n.issuer)] {
                    if let Err(e) = interpolate(raw) {
                        c.problems.push(format!("notarize.{label}: {e}"));
                    }
                }
                match interpolate(&n.key_path) {
                    Ok(p) if !Path::new(&p).exists() => {
                        c.problems.push(format!("notarize.key-path missing: {p}"))
                    }
                    Ok(_) => {}
                    Err(e) => c.problems.push(format!("notarize.key-path: {e}")),
                }
            } else {
                c.problems
                    .push("no notarize config (dmg will not pass Gatekeeper)".into());
            }
        }
        checks.push(c);
    }

    // --- ios ------------------------------------------------------------------
    {
        let ios = signing.and_then(|s| s.ios.as_ref());
        let mut c = Check {
            section: "ios",
            configured: ios.is_some(),
            problems: Vec::new(),
        };
        if let Some(i) = ios {
            if let Err(e) = interpolate(&i.team) {
                c.problems.push(format!("team: {e}"));
            }
            let triple = [
                ("key-id", &i.key_id),
                ("issuer", &i.issuer),
                ("key-path", &i.key_path),
            ];
            let set = triple.iter().filter(|(_, v)| v.is_some()).count();
            if set != 0 && set != 3 {
                c.problems
                    .push("key-id, issuer and key-path must be set together".into());
            }
            if let Some(raw) = &i.key_path {
                match interpolate(raw) {
                    Ok(p) if !Path::new(&p).exists() => {
                        c.problems.push(format!("key-path missing: {p}"))
                    }
                    Ok(_) => {}
                    Err(e) => c.problems.push(format!("key-path: {e}")),
                }
            }
        }
        checks.push(c);
    }

    // --- android ----------------------------------------------------------------
    {
        let android = signing.and_then(|s| s.android.as_ref());
        let mut c = Check {
            section: "android",
            configured: android.is_some(),
            problems: Vec::new(),
        };
        if let Some(a) = android {
            match interpolate(&a.keystore) {
                Ok(p) if !project.root.join(&p).exists() => {
                    c.problems.push(format!("keystore missing: {p}"))
                }
                Ok(_) => {}
                Err(e) => c.problems.push(format!("keystore: {e}")),
            }
            for (label, raw) in [
                ("key-alias", &a.key_alias),
                ("store-pass", &a.store_pass),
                ("key-pass", &a.key_pass),
            ] {
                if let Err(e) = interpolate(raw) {
                    c.problems.push(format!("{label}: {e}"));
                }
            }
        }
        checks.push(c);
    }

    // --- windows ------------------------------------------------------------------
    {
        let windows = signing.and_then(|s| s.windows.as_ref());
        let mut c = Check {
            section: "windows",
            configured: windows.is_some(),
            problems: Vec::new(),
        };
        if windows.is_some()
            && let Err(e) = crate::pack::msix_check(project)
        {
            c.problems.push(e);
        }
        checks.push(c);
    }

    // --- ohos ---------------------------------------------------------------------
    {
        let ohos = signing.and_then(|s| s.ohos.as_ref());
        let mut c = Check {
            section: "ohos",
            configured: ohos.is_some(),
            problems: Vec::new(),
        };
        if let Some(o) = ohos {
            for (label, raw) in [
                ("keystore", &o.keystore),
                ("cert", &o.cert),
                ("profile", &o.profile),
            ] {
                match interpolate(raw) {
                    Ok(p) if !project.root.join(&p).exists() => {
                        c.problems.push(format!("{label} missing: {p}"))
                    }
                    Ok(_) => {}
                    Err(e) => c.problems.push(format!("{label}: {e}")),
                }
            }
            for (label, raw) in [
                ("key-alias", &o.key_alias),
                ("store-pass", &o.store_pass),
                ("key-pass", &o.key_pass),
            ] {
                if let Err(e) = interpolate(raw) {
                    c.problems.push(format!("{label}: {e}"));
                }
            }
        }
        checks.push(c);
    }

    // self-signed-dev is a resolvable provider but still the dev tier; say so, don't call it ready.
    let windows_dev_provider = signing
        .and_then(|s| s.windows.as_ref())
        .is_some_and(|w| w.provider == "self-signed-dev");

    let mut failing = false;
    for c in &checks {
        if !c.configured {
            status(
                "Sign",
                &format!("{}: not configured (pack uses the dev tier)", c.section),
            );
        } else if c.problems.is_empty() {
            if c.section == "windows" && windows_dev_provider {
                status(
                    "Sign",
                    "windows: self-signed-dev provider (dev tier — not distributable)",
                );
            } else {
                status(
                    "Sign",
                    &format!("{}: ok (release signing ready)", c.section),
                );
            }
        } else {
            failing = true;
            status(
                "Sign",
                &format!("{}: NOT ready — {}", c.section, c.problems.join("; ")),
            );
        }
    }
    if failing {
        crate::cli::ErrKind::Sign.exit_code()
    } else {
        0
    }
}

/// `day sign status <id>`: the async-CI half of `pack --no-wait` (§16.5). The Ok
/// value is notarytool's verdict code (0 or the signing exit code); config errors are typed.
pub fn notarize_status(project: &Project, id: &str) -> Result<i32, crate::cli::CliError> {
    let Some(n) = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.macos.as_ref())
        .and_then(|m| m.notarize.as_ref())
    else {
        return Err(crate::cli::CliError::sign(
            "no signing.macos.notarize config in Day.toml",
        ));
    };
    let (key_id, issuer, key_path) = match (
        interpolate(&n.key_id),
        interpolate(&n.issuer),
        interpolate(&n.key_path),
    ) {
        (Ok(k), Ok(i), Ok(p)) => (k, i, p),
        (k, i, p) => {
            let problems: Vec<String> = [k.err(), i.err(), p.err()].into_iter().flatten().collect();
            return Err(crate::cli::CliError::sign(problems.join("\n")));
        }
    };
    let ok = Command::new("xcrun")
        .args(["notarytool", "info", id])
        .args(["--key", &key_path, "--key-id", &key_id, "--issuer", &issuer])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    Ok(if ok {
        0
    } else {
        crate::cli::ErrKind::Sign.exit_code()
    })
}

// ---------------------------------------------------------------------------
// `day sign apply` — sign a package that already exists (DESIGN.md §16.5).
//
// `day pack` builds and signs in one step, which is what a developer wants and what a project's
// own CI runs. A distributor wants the two apart: build the submitted source with no credentials
// in reach, then sign the artifact that came out. Building runs the app's own code (build scripts,
// Gradle plugins, Xcode run scripts); signing runs none of it, so the credentials only ever meet a
// finished file. That separation is the whole reason this verb exists, and why it takes an
// artifact rather than a project to build.
//
// Android lands first: `.aab` through jarsigner, `.apk` through zipalign + apksigner, both
// strictly from `[signing.android]` — no dev-keystore fallback, because an artifact signed with
// the dev key looks signed and cannot be distributed.
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use crate::cli::CliError;

/// The keystore material a release signature needs, resolved from `[signing.android]`.
struct AndroidKeys {
    keystore: PathBuf,
    key_alias: String,
    store_pass: String,
    key_pass: String,
}

/// What `apply` signed, for the report.
struct Signed {
    /// sha256 of the artifact as it arrived.
    input: String,
    /// sha256 of the artifact as it was written.
    output: String,
    /// The signing certificate's SHA-256 fingerprint, lowercase hex, no separators: what Play
    /// matches an upload against, and the one field that says *which* key signed this.
    fingerprint: Option<String>,
}

/// What the caller named on the command line for an Apple package, each overriding the
/// `[signing.ios]` value of the same name.
///
/// Flags matter more here than they do for Android: a distributor signs an artifact built from
/// someone else's project, and that project's `Day.toml` names the developer's profile and
/// certificate, not the distributor's.
#[derive(Clone, Copy, Default)]
pub struct AppleOverrides<'a> {
    pub profile: Option<&'a Path>,
    pub identity: Option<&'a str>,
    pub entitlements: Option<&'a Path>,
}

/// `day sign apply <artifact>`: sign an existing package in place, or into `--out`.
///
/// The input is never modified until the signed copy exists and verifies, so a failure leaves the
/// unsigned artifact exactly as it was rather than a half-signed one.
pub fn apply(
    project: &Project,
    artifact: &Path,
    out: Option<&Path>,
    apple: AppleOverrides<'_>,
    json: bool,
) -> Result<i32, CliError> {
    if !artifact.is_file() {
        return Err(CliError::sign(format!(
            "no such package: {}",
            artifact.display()
        )));
    }
    let ext = artifact
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let signed = match ext.as_str() {
        "aab" => android_keys(project).and_then(|keys| {
            sign_android(artifact, out, &keys, Format::Aab).map_err(CliError::sign)
        }),
        "apk" => android_keys(project).and_then(|keys| {
            sign_android(artifact, out, &keys, Format::Apk).map_err(CliError::sign)
        }),
        "ipa" | "app" => apple_material(project, apple)
            .and_then(|m| sign_apple(artifact, out, &m).map_err(CliError::sign)),
        // Named rather than lumped into "unsupported": these are the formats the verb is expected
        // to grow, and a caller who tries one should hear that it is coming, not that it is wrong.
        "dmg" | "pkg" => Err(CliError::sign(format!(
            "signing a {ext} is not implemented yet — `day pack -p macos-appkit` signs and \
             notarizes during the build (docs/packaging.md)"
        ))),
        _ => Err(CliError::sign(format!(
            "unknown package format {:?} — `day sign apply` takes .aab, .apk, .ipa or .app",
            artifact.display()
        ))),
    }?;

    let path = out.unwrap_or(artifact);
    if json {
        let doc = serde_json::json!({
            "artifact": path.display().to_string(),
            "sha256": signed.output,
            "sha256_unsigned": signed.input,
            "certificate_sha256": signed.fingerprint,
        });
        println!("{doc}");
    } else {
        status("Signed", &path.display().to_string());
        status("Digest", &format!("{} → {}", signed.input, signed.output));
        if let Some(fp) = &signed.fingerprint {
            status("Certificate", &format!("sha256 {fp}"));
        }
    }
    Ok(0)
}

/// Which Android container is being signed: they take different tools for the same key.
#[derive(Clone, Copy, PartialEq)]
enum Format {
    /// The Play upload format. apksigner cannot sign one; jarsigner can.
    Aab,
    /// The installable format, signed with the v2+ scheme apksigner owns.
    Apk,
}

/// `[signing.android]`, resolved strictly: every `${VAR}` must be set and the keystore must exist.
///
/// `day pack` degrades a half-resolved section to the dev keystore so a developer's release build
/// still installs. Here the opposite is right: the caller asked for a distribution signature, and
/// quietly producing a dev-signed artifact would hand them something that looks finished and is
/// rejected by every store.
fn android_keys(project: &Project) -> Result<AndroidKeys, CliError> {
    let Some(a) = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.android.as_ref())
    else {
        return Err(CliError::sign(
            "no [signing.android] in Day.toml — `day sign apply` signs with the project's release \
             keystore (docs/packaging.md#signing-configuration)",
        ));
    };
    let mut problems = Vec::new();
    let mut resolve = |raw: &str, what: &str| match interpolate(raw) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("signing.android.{what}: {e}"));
            String::new()
        }
    };
    let keystore = resolve(&a.keystore, "keystore");
    let key_alias = resolve(&a.key_alias, "key-alias");
    let store_pass = resolve(&a.store_pass, "store-pass");
    let key_pass = resolve(&a.key_pass, "key-pass");
    if !problems.is_empty() {
        return Err(CliError::sign(problems.join("\n")));
    }
    let keystore = project.root.join(keystore);
    if !keystore.is_file() {
        return Err(CliError::sign(format!(
            "signing.android.keystore not found: {}",
            keystore.display()
        )));
    }
    Ok(AndroidKeys {
        keystore,
        key_alias,
        store_pass,
        key_pass,
    })
}

/// Sign one Android package, then verify what was written before it replaces anything.
fn sign_android(
    artifact: &Path,
    out: Option<&Path>,
    keys: &AndroidKeys,
    format: Format,
) -> Result<Signed, String> {
    let input = sha256_file(artifact)?;
    let dest = out.unwrap_or(artifact);
    // Beside the destination, so the final move is a rename within one filesystem rather than a
    // copy that can half-finish.
    let work = dest.with_extension(format!(
        "day-signing.{}",
        dest.extension().and_then(|e| e.to_str()).unwrap_or("tmp")
    ));
    let _ = std::fs::remove_file(&work);
    std::fs::copy(artifact, &work).map_err(|e| format!("copy to {}: {e}", work.display()))?;
    let result = (|| -> Result<Option<String>, String> {
        match format {
            Format::Apk => {
                // Alignment first: apksigner preserves it, and zipalign after signing would
                // invalidate the signature it just wrote.
                let aligned = work.with_extension("aligned");
                let _ = std::fs::remove_file(&aligned);
                run_tool(
                    &build_tool("zipalign")?,
                    &[
                        "-p".into(),
                        "4".into(),
                        work.display().to_string(),
                        aligned.display().to_string(),
                    ],
                    &[],
                )?;
                std::fs::rename(&aligned, &work).map_err(|e| format!("align: {e}"))?;
                run_tool(
                    &build_tool("apksigner")?,
                    &[
                        "sign".into(),
                        "--ks".into(),
                        keys.keystore.display().to_string(),
                        "--ks-key-alias".into(),
                        keys.key_alias.clone(),
                        // v4 writes a detached `.idsig` beside the package, for `adb install
                        // --incremental`. A distribution artifact travels alone, so the extra
                        // file would be litter at best and a half-copied signature at worst.
                        "--v4-signing-enabled".into(),
                        "false".into(),
                        // Passwords by environment, never argv: an argument list is readable by
                        // every other process on the machine.
                        "--ks-pass".into(),
                        format!("env:{STORE_PASS_VAR}"),
                        "--key-pass".into(),
                        format!("env:{KEY_PASS_VAR}"),
                        work.display().to_string(),
                    ],
                    &[
                        (STORE_PASS_VAR, keys.store_pass.as_str()),
                        (KEY_PASS_VAR, keys.key_pass.as_str()),
                    ],
                )?;
                let report = run_tool(
                    &build_tool("apksigner")?,
                    &[
                        "verify".into(),
                        "--print-certs".into(),
                        work.display().to_string(),
                    ],
                    &[],
                )?;
                Ok(apksigner_fingerprint(&report))
            }
            Format::Aab => {
                run_tool(
                    &jdk_tool("jarsigner")?,
                    &[
                        "-keystore".into(),
                        keys.keystore.display().to_string(),
                        "-storepass:env".into(),
                        STORE_PASS_VAR.into(),
                        "-keypass:env".into(),
                        KEY_PASS_VAR.into(),
                        "-sigalg".into(),
                        "SHA256withRSA".into(),
                        "-digestalg".into(),
                        "SHA-256".into(),
                        work.display().to_string(),
                        keys.key_alias.clone(),
                    ],
                    &[
                        (STORE_PASS_VAR, keys.store_pass.as_str()),
                        (KEY_PASS_VAR, keys.key_pass.as_str()),
                    ],
                )?;
                run_tool(
                    &jdk_tool("jarsigner")?,
                    &["-verify".into(), work.display().to_string()],
                    &[],
                )?;
                // jarsigner's own verify output names no certificate; keytool reads the signature
                // block out of the bundle, which is the fingerprint Play compares an upload to.
                let report = run_tool(
                    &jdk_tool("keytool")?,
                    &[
                        "-printcert".into(),
                        "-jarfile".into(),
                        work.display().to_string(),
                    ],
                    &[],
                )?;
                Ok(keytool_fingerprint(&report))
            }
        }
    })();
    let fingerprint = match result {
        Ok(fp) => fp,
        Err(e) => {
            clean_work(&work);
            return Err(e);
        }
    };
    let output = sha256_file(&work)?;
    std::fs::rename(&work, dest).map_err(|e| format!("write {}: {e}", dest.display()))?;
    clean_work(&work);
    Ok(Signed {
        input,
        output,
        fingerprint,
    })
}

/// Remove the working copy and anything a tool wrote beside it (apksigner's `.idsig`), so a
/// failed signing leaves the directory as it found it.
fn clean_work(work: &Path) {
    let _ = std::fs::remove_file(work);
    let mut sidecar = work.as_os_str().to_os_string();
    sidecar.push(".idsig");
    let _ = std::fs::remove_file(PathBuf::from(sidecar));
}

// --- Apple ------------------------------------------------------------------
//
// Re-signing a built `.ipa` is the Apple half of the same separation: the build ran the app's own
// code, this runs none of it. The steps are the ones Xcode's export performs, spelled out because
// no project is present to drive it — swap the embedded profile, take the entitlements the profile
// grants, sign every nested bundle before the bundle that contains it, and repack.

/// The profile, identity and entitlements one Apple signature needs, already resolved.
struct AppleMaterial {
    profile: PathBuf,
    identity: String,
    /// `None` = take the entitlements the profile itself grants, which is what Xcode does when a
    /// project does not override them.
    entitlements: Option<PathBuf>,
}

/// Resolve the Apple signing material: the command line first, then `[signing.ios]`.
fn apple_material(project: &Project, over: AppleOverrides<'_>) -> Result<AppleMaterial, CliError> {
    let ios = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.ios.as_ref());
    let from_manifest = |raw: Option<&String>, what: &str| -> Result<Option<String>, CliError> {
        match interpolate_opt(raw) {
            Ok(v) => Ok(v),
            Err(e) => Err(CliError::sign(format!("signing.ios.{what}: {e}"))),
        }
    };
    let profile = match over.profile {
        Some(p) => p.to_path_buf(),
        None => match from_manifest(ios.and_then(|i| i.profile.as_ref()), "profile")? {
            Some(p) => project.root.join(p),
            None => {
                return Err(CliError::sign(
                    "no provisioning profile — pass --profile <file>, or set signing.ios.profile \
                     in Day.toml (docs/packaging.md#signing-an-existing-package)",
                ));
            }
        },
    };
    if !profile.is_file() {
        return Err(CliError::sign(format!(
            "provisioning profile not found: {}",
            profile.display()
        )));
    }
    let identity = match over.identity {
        Some(i) => i.to_string(),
        None => match from_manifest(ios.and_then(|i| i.identity.as_ref()), "identity")? {
            Some(i) => i,
            None => {
                return Err(CliError::sign(
                    "no signing identity — pass --identity <name>, or set signing.ios.identity in \
                     Day.toml (`security find-identity -v -p codesigning` lists what this machine \
                     holds)",
                ));
            }
        },
    };
    let entitlements = over.entitlements.map(Path::to_path_buf);
    if let Some(e) = &entitlements
        && !e.is_file()
    {
        return Err(CliError::sign(format!(
            "entitlements file not found: {}",
            e.display()
        )));
    }
    Ok(AppleMaterial {
        profile,
        identity,
        entitlements,
    })
}

/// Sign an `.ipa` (or a bare `.app`), then verify what was written before it replaces anything.
fn sign_apple(artifact: &Path, out: Option<&Path>, m: &AppleMaterial) -> Result<Signed, String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "signing an Apple package needs macOS: codesign, security and ditto ship with Xcode"
                .into(),
        );
    }
    let input = sha256_file(artifact)?;
    let dest = out.unwrap_or(artifact);
    let work = dest.with_extension("day-signing.work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("mkdir {}: {e}", work.display()))?;

    let result = (|| -> Result<(PathBuf, Option<String>), String> {
        // `ditto` rather than `unzip`: an .ipa carries symlinks and resource forks, and ditto is
        // the tool Xcode itself packs one with.
        let payload = work.join("Payload");
        let bundle = if artifact.extension().and_then(|e| e.to_str()) == Some("app") {
            std::fs::create_dir_all(&payload).map_err(|e| e.to_string())?;
            let dest_app = payload.join(
                artifact
                    .file_name()
                    .ok_or_else(|| "the .app has no name".to_string())?,
            );
            run_tool(
                Path::new("/usr/bin/ditto"),
                &[
                    artifact.display().to_string(),
                    dest_app.display().to_string(),
                ],
                &[],
            )?;
            dest_app
        } else {
            run_tool(
                Path::new("/usr/bin/ditto"),
                &[
                    "-x".into(),
                    "-k".into(),
                    artifact.display().to_string(),
                    work.display().to_string(),
                ],
                &[],
            )?;
            app_bundle_in(&payload)?
        };

        // The profile the signature is checked against on device, and the entitlements it grants.
        std::fs::copy(&m.profile, bundle.join("embedded.mobileprovision"))
            .map_err(|e| format!("embed the profile: {e}"))?;
        let entitlements = match &m.entitlements {
            Some(e) => e.clone(),
            None => profile_entitlements(&m.profile, &work)?,
        };

        // Inside out: a nested bundle's signature is part of what seals the bundle above it, so
        // signing the app first would invalidate it the moment a framework is re-signed.
        for nested in nested_code(&bundle) {
            codesign(&nested, &m.identity, None)?;
        }
        codesign(&bundle, &m.identity, Some(&entitlements))?;

        // Verify before anything is moved into place: `--deep --strict` walks the nested code the
        // loop above signed, which is where an out-of-order signature shows up.
        run_tool(
            Path::new("/usr/bin/codesign"),
            &[
                "--verify".into(),
                "--deep".into(),
                "--strict".into(),
                "--verbose=2".into(),
                bundle.display().to_string(),
            ],
            &[],
        )?;
        let fingerprint = leaf_certificate_sha256(&bundle, &work);

        let signed_ipa = work.join("signed.ipa");
        run_tool(
            Path::new("/usr/bin/ditto"),
            &[
                "-c".into(),
                "-k".into(),
                "--sequesterRsrc".into(),
                "--keepParent".into(),
                payload.display().to_string(),
                signed_ipa.display().to_string(),
            ],
            &[],
        )?;
        Ok((signed_ipa, fingerprint))
    })();

    let (signed_ipa, fingerprint) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
    };
    let output = sha256_file(&signed_ipa)?;
    // A `.app` in, an `.ipa` out: the signed artifact is what a store takes, and writing the
    // bundle back over a directory would leave the caller with something they cannot upload.
    let dest = if dest.extension().and_then(|e| e.to_str()) == Some("app") {
        dest.with_extension("ipa")
    } else {
        dest.to_path_buf()
    };
    std::fs::rename(&signed_ipa, &dest).map_err(|e| format!("write {}: {e}", dest.display()))?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(Signed {
        input,
        output,
        fingerprint,
    })
}

/// The one `.app` inside an unpacked `Payload/`.
fn app_bundle_in(payload: &Path) -> Result<PathBuf, String> {
    let mut apps: Vec<PathBuf> = std::fs::read_dir(payload)
        .map_err(|_| "the package has no Payload/ directory — is it an .ipa?".to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))
        .collect();
    apps.sort();
    match apps.len() {
        1 => Ok(apps.remove(0)),
        0 => Err("no .app inside Payload/".into()),
        n => Err(format!("{n} .app bundles inside Payload/ — expected one")),
    }
}

/// Every nested bundle and loadable binary that must be signed before the app that contains them,
/// deepest first.
fn nested_code(bundle: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in ["Frameworks", "PlugIns"] {
        let Ok(entries) = std::fs::read_dir(bundle.join(dir)) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if matches!(ext, "framework" | "appex" | "dylib" | "bundle") {
                found.push(path);
            }
        }
    }
    // Deepest first, so a framework inside a plug-in is signed before the plug-in.
    found.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    found
}

/// Sign one bundle or binary. Entitlements are for the app itself; nested code carries none of
/// its own, which is what `--preserve-metadata` would otherwise smuggle in from the old signature.
fn codesign(target: &Path, identity: &str, entitlements: Option<&Path>) -> Result<(), String> {
    let mut args: Vec<String> = vec![
        "--force".into(),
        "--sign".into(),
        identity.to_string(),
        // Apple requires the DER form on iOS 15 and later; codesign writes it only when asked.
        "--generate-entitlement-der".into(),
        "--timestamp".into(),
    ];
    if let Some(e) = entitlements {
        args.push("--entitlements".into());
        args.push(e.display().to_string());
    }
    args.push(target.display().to_string());
    run_tool(Path::new("/usr/bin/codesign"), &args, &[]).map(|_| ())
}

/// The entitlements a provisioning profile grants, written out as a plist for codesign.
///
/// A profile is CMS-signed, so its plist has to be decoded first; `Entitlements` inside it is the
/// set Apple will honor for this app id, which is why it is the right default: entitlements the
/// profile does not grant are rejected at install, and asking for fewer than it grants silently
/// drops capabilities the app shipped with.
fn profile_entitlements(profile: &Path, work: &Path) -> Result<PathBuf, String> {
    let decoded = work.join("profile.plist");
    run_tool(
        Path::new("/usr/bin/security"),
        &[
            "cms".into(),
            "-D".into(),
            "-i".into(),
            profile.display().to_string(),
            "-o".into(),
            decoded.display().to_string(),
        ],
        &[],
    )?;
    let out = work.join("entitlements.plist");
    let printed = run_tool(
        Path::new("/usr/libexec/PlistBuddy"),
        &[
            "-x".into(),
            "-c".into(),
            "Print :Entitlements".into(),
            decoded.display().to_string(),
        ],
        &[],
    )?;
    std::fs::write(&out, printed).map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(out)
}

/// The sha256 of the signing certificate itself, so an Apple signature reports the same "which
/// key" field an Android one does. Best effort: a missing certificate is not a signing failure.
fn leaf_certificate_sha256(bundle: &Path, work: &Path) -> Option<String> {
    let prefix = work.join("cert");
    run_tool(
        Path::new("/usr/bin/codesign"),
        &[
            "-d".into(),
            format!("--extract-certificates={}", prefix.display()),
            bundle.display().to_string(),
        ],
        &[],
    )
    .ok()?;
    // `--extract-certificates=<prefix>` writes <prefix>0 for the leaf, then the chain above it.
    let leaf = PathBuf::from(format!("{}0", prefix.display()));
    std::fs::read(leaf).ok().map(|der| sha256_hex(&der))
}

/// The environment variables the signing tools read the passwords from. Named for this process:
/// they exist only in the child's environment, for the length of one call.
const STORE_PASS_VAR: &str = "DAY_SIGN_STORE_PASS";
const KEY_PASS_VAR: &str = "DAY_SIGN_KEY_PASS";

/// Run one signing tool, returning its stdout. A failure carries the tool's own words: these
/// tools explain a bad password or a missing alias better than any wrapper can.
fn run_tool(tool: &Path, args: &[String], env: &[(&str, &str)]) -> Result<String, String> {
    let mut cmd = Command::new(tool);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("{}: {e}", tool.display()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let said = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("no output");
        return Err(format!(
            "{} failed: {said}",
            tool.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A tool from the newest installed Android build-tools (`apksigner`, `zipalign`).
fn build_tool(name: &str) -> Result<PathBuf, String> {
    let dir = day_toolchain::android_sdk_dir().join("build-tools");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|_| {
            format!(
                "no Android build-tools under {} — install them (Android Studio, or \
                 `sdkmanager \"build-tools;35.0.0\"`) or set ANDROID_HOME",
                dir.display()
            )
        })?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // Newest wins, compared as version numbers: "9.0.0" sorts after "35.0.0" as a string.
    versions
        .sort_by_key(|p| version_key(p.file_name().and_then(|n| n.to_str()).unwrap_or_default()));
    let exe = if cfg!(windows) {
        // apksigner ships as a .bat wrapper; zipalign is a plain .exe.
        if name == "zipalign" {
            format!("{name}.exe")
        } else {
            format!("{name}.bat")
        }
    } else {
        name.to_string()
    };
    versions
        .iter()
        .rev()
        .map(|v| v.join(&exe))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{name} not found in {}", dir.display()))
}

/// A tool from the JDK (`jarsigner`, `keytool`): `$JAVA_HOME` first, then whatever is on PATH.
fn jdk_tool(name: &str) -> Result<PathBuf, String> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(home) = day_toolchain::jdk_home() {
        let p = home.join("bin").join(&exe);
        if p.is_file() {
            return Ok(p);
        }
    }
    // On PATH: `Command` resolves it, and a missing tool surfaces as the spawn error.
    Ok(PathBuf::from(exe))
}

/// `"35.0.0"` → `(35, 0, 0)`, for ordering build-tools directories.
fn version_key(name: &str) -> (u32, u32, u32) {
    let mut it = name.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// The signer fingerprint from `apksigner verify --print-certs`, normalized to lowercase hex.
fn apksigner_fingerprint(report: &str) -> Option<String> {
    report
        .lines()
        .find(|l| l.contains("certificate SHA-256 digest:"))
        .and_then(|l| l.split(':').next_back())
        .map(normalize_fingerprint)
}

/// The signer fingerprint from `keytool -printcert -jarfile`, normalized the same way: keytool
/// prints `SHA256: AA:BB:…`, apksigner a bare hex run, and a caller comparing them to a key they
/// hold should not have to care which tool answered.
fn keytool_fingerprint(report: &str) -> Option<String> {
    report
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("SHA256:"))
        .and_then(|l| l.strip_prefix("SHA256:"))
        .map(normalize_fingerprint)
}

fn normalize_fingerprint(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod apply_tests {
    use super::*;

    #[test]
    fn a_build_tools_directory_orders_by_version_not_by_string() {
        let mut names = ["9.0.0", "35.0.0", "34.0.1"];
        names.sort_by_key(|n| version_key(n));
        assert_eq!(names, ["9.0.0", "34.0.1", "35.0.0"]);
    }

    #[test]
    fn apksigner_reports_its_signer() {
        let report = "Verifies\n\
            Verified using v1 scheme (JAR signing): false\n\
            Signer #1 certificate DN: CN=Day\n\
            Signer #1 certificate SHA-256 digest: A1B2C3d4\n";
        assert_eq!(apksigner_fingerprint(report).as_deref(), Some("a1b2c3d4"));
    }

    #[test]
    fn nested_code_is_signed_from_the_inside_out() {
        // A framework nested inside a plug-in has to be signed before the plug-in that seals it.
        let root = std::env::temp_dir().join(format!("day-sign-test-{}", std::process::id()));
        let plugin = root.join("PlugIns/Share.appex");
        let inner = plugin.join("Frameworks/Inner.framework");
        std::fs::create_dir_all(&inner).expect("temp bundle");
        std::fs::create_dir_all(root.join("Frameworks/Day.framework")).expect("temp bundle");
        std::fs::write(root.join("Frameworks/libday.dylib"), b"").expect("temp bundle");
        let order = nested_code(&root);
        let depth = |p: &Path| p.components().count();
        assert!(
            order.windows(2).all(|w| depth(&w[0]) >= depth(&w[1])),
            "not deepest-first: {order:?}"
        );
        assert!(order.iter().any(|p| p.ends_with("libday.dylib")));
        assert!(order.iter().any(|p| p.ends_with("Share.appex")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keytool_reports_the_same_shape() {
        let report = "Owner: CN=Day\n         SHA256: A1:B2:C3:D4\n";
        assert_eq!(keytool_fingerprint(report).as_deref(), Some("a1b2c3d4"));
        assert_eq!(apksigner_fingerprint("nothing here"), None);
    }
}
