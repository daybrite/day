// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day sign check` (DESIGN.md §16.5) validates the presence and resolvability of the
//! Day.toml `signing:` configuration (env vars set, referenced files exist) without ever printing
//! a secret value; `day sign status <id>` polls an async notarytool submission. Actual signing
//! runs inside `day pack` (the per-format modules in pack/).

use std::collections::BTreeMap;
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

    // --- macos.app-store ----------------------------------------------------
    // Its own section: a direct download and a store build are signed by different
    // certificates, and an app may well have only one of the two.
    {
        let mas = signing
            .and_then(|s| s.macos.as_ref())
            .and_then(|m| m.app_store.as_ref());
        let mut c = Check {
            section: "macos.app-store",
            configured: mas.is_some(),
            problems: Vec::new(),
        };
        if let Some(m) = mas {
            for (label, raw) in [
                ("identity", &m.identity),
                ("installer-identity", &m.installer_identity),
            ] {
                if let Err(e) = interpolate(raw) {
                    c.problems.push(format!("{label}: {e}"));
                }
            }
            match interpolate(&m.profile) {
                Ok(p) if !project.root.join(&p).is_file() => {
                    c.problems.push(format!("profile missing: {p}"))
                }
                Ok(_) => {}
                Err(e) => c.problems.push(format!("profile: {e}")),
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
            if let Some(e) = &i.entitlements
                && !project.root.join(e).exists()
            {
                c.problems.push(format!("entitlements file missing: {e}"));
            }
            for (name, ext) in &i.extensions {
                if !project.manifest.extensions.iter().any(|e| &e.name == name) {
                    c.problems.push(format!(
                        "extensions.{name}: Day.toml declares no [[extensions]] named {name}"
                    ));
                }
                for (what, file) in [
                    ("profile", &ext.profile),
                    ("entitlements", &ext.entitlements),
                ] {
                    if let Some(raw) = file {
                        match interpolate(raw) {
                            Ok(p) if !project.root.join(&p).exists() => c
                                .problems
                                .push(format!("extensions.{name}.{what} missing: {p}")),
                            Ok(_) => {}
                            Err(e) => c.problems.push(format!("extensions.{name}.{what}: {e}")),
                        }
                    }
                }
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

/// Gradle deliberately emits an `-unsigned.apk` when no release key is configured.
/// Launch a verified dev-signed copy, including for `launch --skip-build`, without changing
/// the build artifact or the signing properties consumed by `pack --no-sign`.
pub(crate) fn android_launch_apk(artifact: &Path) -> Result<PathBuf, String> {
    if !artifact
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("-unsigned.apk"))
    {
        return Ok(artifact.to_path_buf());
    }
    let keys = android_debug_keys()?;
    // A subdirectory keeps build/pack's APK discovery from mistaking this local install copy
    // for a distribution artifact. Always regenerate it so a rebuild cannot launch stale code.
    let dest = artifact.parent().unwrap().join("day-launch/app-dev.apk");
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    sign_android(artifact, Some(&dest), &keys, Format::Apk)?;
    Ok(dest)
}

/// Use Android's local debug identity so switching debug/release for a local run can update
/// the installed app without uninstalling it and losing its data. Distribution packaging
/// still owns its separate release/shared-dev key policy.
fn android_debug_keys() -> Result<AndroidKeys, String> {
    let directory = if let Some(dir) = std::env::var_os("ANDROID_USER_HOME") {
        PathBuf::from(dir)
    } else {
        PathBuf::from(
            std::env::var_os("ANDROID_SDK_HOME")
                .or_else(|| std::env::var_os("HOME"))
                .or_else(|| std::env::var_os("USERPROFILE"))
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound).to_string())?,
        )
        .join(".android")
    };
    let keys = AndroidKeys {
        keystore: directory.join("debug.keystore"),
        key_alias: "androiddebugkey".into(),
        store_pass: "android".into(),
        key_pass: "android".into(),
    };
    if !keys.keystore.exists() {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        run_tool(
            &jdk_tool("keytool")?,
            &[
                "-genkeypair".into(),
                "-keystore".into(),
                keys.keystore.display().to_string(),
                "-alias".into(),
                keys.key_alias.clone(),
                "-storepass:env".into(),
                STORE_PASS_VAR.into(),
                "-keypass:env".into(),
                KEY_PASS_VAR.into(),
                "-keyalg".into(),
                "RSA".into(),
                "-keysize".into(),
                "2048".into(),
                "-validity".into(),
                "10950".into(),
                "-dname".into(),
                "CN=Android Debug,O=Android,C=US".into(),
            ],
            &[
                (STORE_PASS_VAR, &keys.store_pass),
                (KEY_PASS_VAR, &keys.key_pass),
            ],
        )?;
    }
    Ok(keys)
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
    /// App extension profiles, `(name or bundle id, profile)`: `--extension-profile`, or the App
    /// Store profiles `day pack` found installed for each extension.
    pub extension_profiles: &'a [(String, PathBuf)],
    /// The Mac App Store's installer identity, for a macOS bundle signed into a `.pkg`.
    pub installer_identity: Option<&'a str>,
}

/// Android signing material passed on the command line, for a caller that signs a package
/// without the project that built it.
///
/// A submission queue is the case this exists for: it signs other people's packages with its own
/// release key, and reading `[signing.android]` out of the submitted app would let that app
/// choose which of the queue's environment variables are read and reported. With both values
/// here, the manifest is never consulted; the passwords still come from `DAY_KS_PASS` and
/// `DAY_KEY_PASS`, which keep them off the command line.
#[derive(Debug, Default, Clone, Copy)]
pub struct AndroidOverrides<'a> {
    pub keystore: Option<&'a Path>,
    pub key_alias: Option<&'a str>,
}

/// HarmonyOS signing material on the command line, for the same caller: a `.hap` is signed
/// from the keystore, key alias and passwords the Android half takes (`--keystore`,
/// `--key-alias`, `DAY_SIGN_STORE_PASS`, `DAY_SIGN_KEY_PASS`), plus the app certificate
/// (`--cert`) and the `.p7b` provisioning profile (`--profile`). All on the command line: a
/// package signed after the build has no project beside it to read `[signing.ohos]` from.
#[derive(Debug, Default, Clone, Copy)]
pub struct OhosOverrides<'a> {
    pub keystore: Option<&'a Path>,
    pub cert: Option<&'a Path>,
    pub profile: Option<&'a Path>,
    pub key_alias: Option<&'a str>,
}

/// `day sign apply <artifact>`: sign an existing package in place, or into `--out`.
///
/// The input is never modified until the signed copy exists and verifies, so a failure leaves the
/// unsigned artifact exactly as it was rather than a half-signed one.
///
/// `project` is optional. Given the material on the command line, this signs a package that has
/// no project beside it at all, which is what lets a signing step run in a directory holding
/// nothing but the package it was handed.
pub fn apply(
    project: Option<&Project>,
    artifact: &Path,
    out: Option<&Path>,
    apple: AppleOverrides<'_>,
    android: AndroidOverrides<'_>,
    ohos: OhosOverrides<'_>,
    json: bool,
) -> Result<i32, CliError> {
    let ext = artifact
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // A bundle is a directory; every other package is a file.
    let present = if ext == "app" {
        artifact.is_dir()
    } else {
        artifact.is_file()
    };
    if !present {
        return Err(CliError::sign(format!(
            "no such package: {}",
            artifact.display()
        )));
    }
    let signed = match ext.as_str() {
        // A macOS bundle (it has `Contents/`): signed for the Mac App Store, into a `.pkg`.
        "app" if artifact.join("Contents/Info.plist").is_file() => mas_material(project, apple)
            .and_then(|m| sign_mac_app_store(artifact, out, &m).map_err(CliError::sign)),
        "aab" => android_keys(project, android).and_then(|keys| {
            sign_android(artifact, out, &keys, Format::Aab).map_err(CliError::sign)
        }),
        "apk" => android_keys(project, android).and_then(|keys| {
            sign_android(artifact, out, &keys, Format::Apk).map_err(CliError::sign)
        }),
        "ipa" | "app" => apple_material(project, apple)
            .and_then(|m| sign_apple(artifact, out, &m).map_err(CliError::sign)),
        "hap" => {
            ohos_material(ohos).and_then(|m| sign_hap(artifact, out, &m).map_err(CliError::sign))
        }
        // Named rather than lumped into "unsupported": these are the formats the verb is expected
        // to grow, and a caller who tries one should hear that it is coming, not that it is wrong.
        "dmg" | "pkg" => Err(CliError::sign(format!(
            "signing a {ext} is not implemented yet — `day pack -t macos-appkit` signs and \
             notarizes the .dmg during the build, and a Mac App Store .pkg is signed from the \
             .app inside it (`day sign apply <App>.app`, docs/packaging.md)"
        ))),
        _ => Err(CliError::sign(format!(
            "unknown package format {:?} — `day sign apply` takes .aab, .apk, .ipa, .app or .hap",
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
fn android_keys(
    project: Option<&Project>,
    over: AndroidOverrides<'_>,
) -> Result<AndroidKeys, CliError> {
    // Material on the command line stands alone: the keystore, the alias, and the two passwords
    // from the environment. Nothing about the package's own project is read.
    if let (Some(keystore), Some(alias)) = (over.keystore, over.key_alias) {
        if !keystore.is_file() {
            return Err(CliError::sign(format!(
                "--keystore not found: {}",
                keystore.display()
            )));
        }
        let pass = |var: &str| -> Result<String, CliError> {
            std::env::var(var).map_err(|_| {
                CliError::sign(format!(
                    "{var} is not set — `day sign apply --keystore` takes the passwords from the \
                     environment so they stay off the command line"
                ))
            })
        };
        return Ok(AndroidKeys {
            keystore: keystore.to_path_buf(),
            key_alias: alias.to_string(),
            store_pass: pass(STORE_PASS_VAR)?,
            key_pass: pass(KEY_PASS_VAR)?,
        });
    }
    if over.keystore.is_some() || over.key_alias.is_some() {
        return Err(CliError::sign(
            "--keystore and --key-alias go together; naming one without the other would sign with \
             half this key and half the project's",
        ));
    }
    let Some(project) = project else {
        return Err(CliError::sign(
            "no project here, and no key on the command line — run `day sign apply` inside the \
             app, or pass --keystore and --key-alias (docs/packaging.md#signing-an-existing-package)",
        ));
    };
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

// --- HarmonyOS --------------------------------------------------------------
//
// The hap half of the same separation: `day pack --no-sign` leaves the unsigned hap hvigor
// built (named `-unsigned.hap`), and this signs it with hap-sign-tool from the OpenHarmony SDK,
// the way `day pack` does with `[signing.ohos]`, from material named on the command line.

/// The HarmonyOS material, from the command line alone: every piece is required, since a hap
/// signed without its certificate or profile installs nowhere.
fn ohos_material(over: OhosOverrides<'_>) -> Result<crate::pack::ohos::OhosMaterial, CliError> {
    let need = |what: &str, flag: &str| {
        CliError::sign(format!(
            "a .hap needs {what} ({flag}), plus the passwords in {STORE_PASS_VAR} and {KEY_PASS_VAR}"
        ))
    };
    let keystore = over
        .keystore
        .ok_or_else(|| need("the keystore", "--keystore"))?;
    let cert = over
        .cert
        .ok_or_else(|| need("the app certificate", "--cert"))?;
    let profile = over
        .profile
        .ok_or_else(|| need("the provisioning profile", "--profile"))?;
    let key_alias = over
        .key_alias
        .ok_or_else(|| need("the key alias", "--key-alias"))?;
    for (label, p) in [
        ("--keystore", keystore),
        ("--cert", cert),
        ("--profile", profile),
    ] {
        if !p.is_file() {
            return Err(CliError::sign(format!(
                "{label} not found: {}",
                p.display()
            )));
        }
    }
    let pass = |var: &str| {
        std::env::var(var)
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                CliError::sign(format!("{var} is not set — the hap keystore's password"))
            })
    };
    Ok(crate::pack::ohos::OhosMaterial {
        keystore: keystore.to_path_buf(),
        cert: cert.to_path_buf(),
        profile: profile.to_path_buf(),
        key_alias: key_alias.to_string(),
        store_pass: pass(STORE_PASS_VAR)?,
        key_pass: pass(KEY_PASS_VAR)?,
    })
}

/// Sign a hap into `out`, or in place: the signed copy is written beside the destination and
/// renamed over it only once the tool has finished, so a failure leaves the unsigned package as
/// it was.
fn sign_hap(
    artifact: &Path,
    out: Option<&Path>,
    m: &crate::pack::ohos::OhosMaterial,
) -> Result<Signed, String> {
    let input = sha256_file(artifact)?;
    let dest = out.unwrap_or(artifact);
    let work = dest.with_extension("day-signing.hap");
    let _ = std::fs::remove_file(&work);
    if let Err(e) = crate::pack::ohos::release_sign(m, artifact, &work) {
        let _ = std::fs::remove_file(&work);
        return Err(e);
    }
    let output = sha256_file(&work)?;
    std::fs::rename(&work, dest).map_err(|e| format!("write {}: {e}", dest.display()))?;
    Ok(Signed {
        input,
        output,
        fingerprint: None,
    })
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
    /// The app's own entitlements (`--entitlements`, `[signing.ios] entitlements`, or the
    /// project's `CODE_SIGN_ENTITLEMENTS`), resolved and intersected with the profile at signing
    /// time. `None` = take the entitlements the profile itself grants.
    entitlements: Option<AppEntitlements>,
    /// Each embedded app extension's material, by the extension's name (its `.appex` stem).
    extensions: Vec<ExtensionMaterial>,
    /// Profiles named on the command line, by extension name or bundle id.
    extension_profiles: Vec<(String, PathBuf)>,
}

/// One app extension's signing material, from the project. The profile may be absent here and
/// found later: on the command line, or the app's own profile when it covers the extension.
struct ExtensionMaterial {
    name: String,
    profile: Option<PathBuf>,
    /// What the extension claims, resolved and intersected with its profile at signing time.
    entitlements: Option<AppEntitlements>,
}

/// Resolve the Apple signing material: the command line first, then `[signing.ios]`.
fn apple_material(
    project: Option<&Project>,
    over: AppleOverrides<'_>,
) -> Result<AppleMaterial, CliError> {
    // The same rule as the Android half: what the command line states is what is used, and the
    // project is consulted only for what it leaves out.
    let ios = project
        .and_then(|p| p.manifest.signing.as_ref())
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
            // Relative to the project that declared it; with no project here, the manifest was
            // never read and this arm is unreachable.
            Some(p) => project
                .map(|pr| pr.root.join(&p))
                .unwrap_or_else(|| p.into()),
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
    // The command line's file, else the project's own (a store build signs Release).
    let entitlements = match over.entitlements {
        Some(e) if !e.is_file() => {
            return Err(CliError::sign(format!(
                "entitlements file not found: {}",
                e.display()
            )));
        }
        Some(e) => Some(AppEntitlements {
            path: e.to_path_buf(),
            settings: BTreeMap::new(),
        }),
        None => match project {
            Some(p) => ios_app_entitlements(p, "Release").map_err(CliError::sign)?,
            None => None,
        },
    };
    let extensions = match project {
        Some(p) => extension_material(p, ios).map_err(CliError::sign)?,
        None => Vec::new(),
    };
    Ok(AppleMaterial {
        profile,
        identity,
        entitlements,
        extensions,
        extension_profiles: over.extension_profiles.to_vec(),
    })
}

/// The project's material for every iOS app extension it declares: `[signing.ios.extensions]`
/// first, then the entitlements Day generates from `[[extensions]]` (docs/app-extensions.md),
/// written into the build directory so a package can be signed without being rebuilt.
fn extension_material(
    project: &Project,
    ios: Option<&crate::meta::IosSigning>,
) -> Result<Vec<ExtensionMaterial>, String> {
    let exts = crate::extensions::resolve(project, crate::extensions::Os::Ios)?;
    let settings = ios_app_entitlements(project, "Release")
        .ok()
        .flatten()
        .map(|e| e.settings)
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in exts {
        let declared = ios.and_then(|i| i.extensions.get(&r.name));
        let profile = match declared.and_then(|d| d.profile.as_ref()) {
            Some(raw) => Some(
                project.root.join(
                    interpolate(raw)
                        .map_err(|e| format!("signing.ios.extensions.{}.profile: {e}", r.name))?,
                ),
            ),
            None => None,
        };
        let path = match declared.and_then(|d| d.entitlements.as_ref()) {
            Some(raw) => project.root.join(
                interpolate(raw)
                    .map_err(|e| format!("signing.ios.extensions.{}.entitlements: {e}", r.name))?,
            ),
            None => {
                let staged = project.root.join(r.staged_entitlements());
                if let Some(dir) = staged.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                let mut bytes = Vec::new();
                plist::Value::Dictionary(crate::extensions::entitlements(&r, &project.root)?)
                    .to_writer_xml(&mut bytes)
                    .map_err(|e| e.to_string())?;
                bytes.push(b'\n');
                crate::pieces::write_bytes_if_changed(&staged, &bytes)?;
                staged
            }
        };
        out.push(ExtensionMaterial {
            name: r.name.clone(),
            profile,
            entitlements: Some(AppEntitlements {
                path,
                settings: settings.clone(),
            }),
        });
    }
    Ok(out)
}

/// Whether the profile at `profile` (decoded under `work`) provisions `bundle_id`.
fn profile_covers_id(profile: &Path, bundle_id: &str, work: &Path) -> bool {
    let decoded = work.join("covers.plist");
    let ok = run_tool(
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
    )
    .is_ok();
    ok && decoded_profile_entitlements(&decoded)
        .ok()
        .and_then(|d| {
            d.get("application-identifier")
                .and_then(plist::Value::as_string)
                .map(str::to_string)
        })
        .and_then(|id| crate::mobile::profile_covers(&id, bundle_id))
        .is_some()
}

/// Sign one embedded app extension: embed its profile, sign its frameworks, then sign it with its
/// entitlements cut down to what that profile grants.
fn sign_apple_extension(appex: &Path, m: &AppleMaterial, work: &Path) -> Result<(), String> {
    let name = appex
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let bundle_id = crate::extensions::bundle_id_of(appex)
        .ok_or_else(|| format!("{name}.appex has no CFBundleIdentifier"))?;
    let own = m.extensions.iter().find(|e| e.name == name);
    let profile = m
        .extension_profiles
        .iter()
        .find(|(key, _)| key == &name || key == &bundle_id)
        .map(|(_, p)| p.clone())
        .or_else(|| own.and_then(|e| e.profile.clone()))
        .or_else(|| profile_covers_id(&m.profile, &bundle_id, work).then(|| m.profile.clone()))
        .ok_or_else(|| {
            format!(
                "no provisioning profile for the app extension {bundle_id} ({name}.appex): pass \
                 --extension-profile {name}=<file>, or set [signing.ios.extensions.{name}] \
                 profile in Day.toml (docs/app-extensions.md, \"Signing\")"
            )
        })?;
    if !profile.is_file() {
        return Err(format!(
            "provisioning profile not found: {}",
            profile.display()
        ));
    }
    std::fs::copy(&profile, appex.join("embedded.mobileprovision"))
        .map_err(|e| format!("embed the profile in {name}.appex: {e}"))?;
    let ext_work = work.join(format!("ext-{name}"));
    std::fs::create_dir_all(&ext_work).map_err(|e| e.to_string())?;
    let decoded = profile_entitlements(&profile, &ext_work)?;
    let granted = plist::Value::from_file(&decoded)
        .map_err(|e| format!("read {}: {e}", decoded.display()))?
        .into_dictionary()
        .ok_or("the extension profile's Entitlements is not a dictionary")?;
    let entitlements = ext_work.join("signing.entitlements");
    write_ios_signing_entitlements(
        &granted,
        own.and_then(|e| e.entitlements.as_ref())
            .filter(|e| e.path.is_file()),
        &bundle_id,
        &profile.display().to_string(),
        &entitlements,
    )?;
    if let Ok(rd) = std::fs::read_dir(appex.join("Frameworks")) {
        let mut inner: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        inner.sort();
        for item in inner {
            codesign(&item, &m.identity, None)?;
        }
    }
    codesign(appex, &m.identity, Some(&entitlements))
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

        // The profile the signature is checked against on device, and the entitlements: the
        // app's own within what the profile grants, or the profile's when the app names none.
        std::fs::copy(&m.profile, bundle.join("embedded.mobileprovision"))
            .map_err(|e| format!("embed the profile: {e}"))?;
        let decoded = profile_entitlements(&m.profile, &work)?;
        let granted = plist::Value::from_file(&decoded)
            .map_err(|e| format!("read {}: {e}", decoded.display()))?
            .into_dictionary()
            .ok_or("the profile's Entitlements is not a dictionary")?;
        let bundle_id = plist::Value::from_file(bundle.join("Info.plist"))
            .ok()
            .and_then(|v| {
                v.as_dictionary()
                    .and_then(|d| d.get("CFBundleIdentifier"))
                    .and_then(plist::Value::as_string)
                    .map(str::to_string)
            })
            .ok_or("the bundle's Info.plist has no CFBundleIdentifier")?;
        let entitlements = work.join("signing.entitlements");
        write_ios_signing_entitlements(
            &granted,
            m.entitlements.as_ref(),
            &bundle_id,
            &m.profile.display().to_string(),
            &entitlements,
        )?;

        // Inside out: a nested bundle's signature is part of what seals the bundle above it, so
        // signing the app first would invalidate it the moment a framework is re-signed. App
        // extensions are bundles of their own, each with its profile and entitlements.
        for appex in crate::extensions::embedded(&bundle) {
            sign_apple_extension(&appex, m, &work)?;
        }
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

/// The Mac App Store material: the command line first, then `[signing.macos.app-store]`. Strict
/// like the rest of `sign apply`: the caller asked for a store signature, so a missing piece is an
/// error, not a quietly unsigned package.
fn mas_material(
    project: Option<&Project>,
    over: AppleOverrides<'_>,
) -> Result<crate::pack::macos::MasMaterial, CliError> {
    let mas = project
        .and_then(|p| p.manifest.signing.as_ref())
        .and_then(|s| s.macos.as_ref())
        .and_then(|m| m.app_store.as_ref());
    let from_manifest = |raw: Option<&String>, what: &str| -> Result<Option<String>, CliError> {
        interpolate_opt(raw)
            .map_err(|e| CliError::sign(format!("signing.macos.app-store.{what}: {e}")))
    };
    let missing = |what: &str, flag: &str, key: &str| {
        CliError::sign(format!(
            "no {what} — pass {flag}, or set signing.macos.app-store.{key} in Day.toml \
             (docs/packaging.md#the-mac-app-store)"
        ))
    };
    let profile = match over.profile {
        Some(p) => p.to_path_buf(),
        None => match from_manifest(mas.map(|m| &m.profile), "profile")? {
            Some(p) => project
                .map(|pr| pr.root.join(&p))
                .unwrap_or_else(|| p.into()),
            None => {
                return Err(missing(
                    "Mac App Store profile",
                    "--profile <file>",
                    "profile",
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
        None => from_manifest(mas.map(|m| &m.identity), "identity")?
            .ok_or_else(|| missing("signing identity", "--identity <name>", "identity"))?,
    };
    let installer_identity = match over.installer_identity {
        Some(i) => i.to_string(),
        None => from_manifest(mas.map(|m| &m.installer_identity), "installer-identity")?
            .ok_or_else(|| {
                missing(
                    "installer identity",
                    "--installer-identity <name>",
                    "installer-identity",
                )
            })?,
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
    Ok(crate::pack::macos::MasMaterial {
        profile,
        identity,
        installer_identity,
        entitlements,
    })
}

/// A macOS bundle signed for the Mac App Store, as an installer package: `--out`, else the
/// bundle's name with `.pkg`. The bundle itself is left as it was.
fn sign_mac_app_store(
    artifact: &Path,
    out: Option<&Path>,
    m: &crate::pack::macos::MasMaterial,
) -> Result<Signed, String> {
    let input = sha256_tree(artifact)?;
    let dest = match out {
        Some(o) if o.extension().and_then(|e| e.to_str()) == Some("pkg") => o.to_path_buf(),
        Some(o) => {
            return Err(format!(
                "--out {}: a Mac App Store package is a .pkg",
                o.display()
            ));
        }
        None => artifact.with_extension("pkg"),
    };
    let fingerprint = crate::pack::macos::mac_app_store_pkg(artifact, &dest, m)?;
    Ok(Signed {
        input,
        output: sha256_file(&dest)?,
        fingerprint,
    })
}

/// A digest of a bundle directory, for the report's "unsigned" side: every file's path relative
/// to the root and its bytes, and every symlink's target, in sorted order. Stable across copies,
/// which is all the report needs from it.
fn sha256_tree(root: &Path) -> Result<String, String> {
    use sha2::Digest;
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let path = e.path();
            let kind = e.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                walk(&path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, &mut files)?;
    files.sort();
    let mut hasher = sha2::Sha256::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap_or(&path);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0]);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
            hasher.update(target.to_string_lossy().as_bytes());
        } else {
            hasher
                .update(std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?);
        }
        hasher.update([0]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
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
/// deepest first. App extensions are signed separately ([`sign_apple_extension`]), each with its
/// own profile and entitlements.
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
            if matches!(ext, "framework" | "dylib" | "bundle") {
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

// --- iOS entitlements -------------------------------------------------------
//
// The app's own entitlements win, as they do in Xcode: a provisioning profile lists what Apple
// allows for the app id, which is usually more than the app claims (a team profile grants Game
// Center and a wildcard keychain group to every app it covers). Signing with the profile's set
// verbatim ships capabilities the app never asked for. So when the project names an entitlements
// file, Day signs with that file: its `$(VAR)` references resolved the way Xcode resolves them,
// then intersected with the profile, because a signature claiming what the profile does not grant
// fails to install. Only with no file at all does the profile's set stand in, which is what a
// project without entitlements has always been signed with.

/// The app's own iOS entitlements file, and the build settings its `$(VAR)` references resolve
/// against.
#[derive(Debug, Default)]
pub(crate) struct AppEntitlements {
    /// The plist, absolute.
    pub path: PathBuf,
    /// Settings from the `platform/ios/DayApp.xcconfig` chain for one build configuration, plus
    /// the project-level ones Xcode defines (`DAY_PROJECT_ROOT`, `SRCROOT`). Empty for a file
    /// named on the command line, which then resolves the signing-time variables only.
    pub settings: BTreeMap<String, String>,
}

/// The project's iOS entitlements file: `[signing.ios] entitlements` first, else the
/// `CODE_SIGN_ENTITLEMENTS` setting the app's `platform/ios/DayApp.xcconfig` chain gives a
/// device build (`iphoneos`, `arm64`) of `configuration` ("Debug" or "Release"), conditional
/// settings included. `None` when the project names neither, with a warning when the xcconfig
/// sets it only for other builds.
pub(crate) fn ios_app_entitlements(
    project: &Project,
    configuration: &str,
) -> Result<Option<AppEntitlements>, String> {
    let xcconfig = project.root.join("platform/ios/DayApp.xcconfig");
    let srcroot = crate::xcode::project_dir(&project.root, "ios")
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project.root.join("platform/ios"));
    // Day signs device and store builds only; a simulator build is signed by Xcode itself.
    let read = xcconfig_settings(
        &xcconfig,
        &XcconfigBuild {
            configuration,
            sdk: "iphoneos",
            arch: "arm64",
        },
    );
    let mut settings = read.values;
    let root = project.root.display().to_string();
    let src = srcroot.display().to_string();
    for (key, value) in [
        ("DAY_PROJECT_ROOT", root.as_str()),
        ("SRCROOT", src.as_str()),
        ("SOURCE_ROOT", src.as_str()),
        ("PROJECT_DIR", src.as_str()),
        ("CONFIGURATION", configuration),
    ] {
        settings.insert(key.into(), value.into());
    }
    let declared = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.ios.as_ref())
        .and_then(|i| i.entitlements.as_ref());
    let path = if let Some(raw) = declared {
        let p = interpolate(raw).map_err(|e| format!("signing.ios.entitlements: {e}"))?;
        project.root.join(p)
    } else {
        match settings.get("CODE_SIGN_ENTITLEMENTS") {
            Some(raw) if !raw.trim().is_empty() => {
                let p = expand_build_variables(raw, &settings).map_err(|var| {
                    format!(
                        "CODE_SIGN_ENTITLEMENTS in {} references $({var}), which Day cannot \
                         resolve — write the path relative to the project, or set \
                         [signing.ios] entitlements in Day.toml",
                        xcconfig.display()
                    )
                })?;
                // Xcode reads a relative path against the project directory.
                srcroot.join(p)
            }
            _ => {
                // Assigned, but only for another build: Xcode may sign with that file where Day
                // falls back to the profile's set, so say so rather than differ in silence.
                if let Some(skipped) = read.unmatched.get("CODE_SIGN_ENTITLEMENTS") {
                    status(
                        "Warning",
                        &format!(
                            "{} sets CODE_SIGN_ENTITLEMENTS only as {}, which does not apply to \
                             the {configuration} iphoneos arm64 build Day signs; signing with \
                             the provisioning profile's entitlements. Add an unconditional or \
                             [sdk=iphoneos*] setting, or set [signing.ios] entitlements in \
                             Day.toml",
                            xcconfig.display(),
                            skipped.join(", ")
                        ),
                    );
                }
                return Ok(None);
            }
        }
    };
    if !path.is_file() {
        return Err(format!("entitlements file not found: {}", path.display()));
    }
    Ok(Some(AppEntitlements { path, settings }))
}

/// The build an xcconfig is read for: what its conditional settings (`NAME[sdk=iphoneos*] = …`)
/// are matched against, as Xcode matches them for one build of one target.
#[derive(Clone, Copy, Debug)]
struct XcconfigBuild<'a> {
    /// The build configuration, `[config=…]`: "Debug" or "Release".
    configuration: &'a str,
    /// The SDK's platform name without its version, `[sdk=…]`: "iphoneos" for a device,
    /// "iphonesimulator" for the simulator, "macosx" for a Mac.
    sdk: &'a str,
    /// The architecture, `[arch=…]`: "arm64" or "x86_64".
    arch: &'a str,
}

/// What [`xcconfig_settings`] read: each setting's value for the build, and the settings that
/// were assigned only under conditions the build does not meet.
#[derive(Debug, Default)]
struct XcconfigSettings {
    /// The resolved value of every setting with an assignment that applies to the build.
    values: BTreeMap<String, String>,
    /// Settings assigned somewhere, but never by an assignment that applies to the build:
    /// conditioned on another SDK, configuration or architecture, or on a condition Day does not
    /// evaluate. A setting here that the caller needs is worth a warning, since Xcode itself may
    /// well apply it.
    unmatched: BTreeMap<String, Vec<String>>,
}

/// Read one xcconfig and the files it `#include`s, the way Xcode does for `build`.
///
/// A setting takes the value of its most specific assignment whose conditions all match the build
/// (an unconditional one has none, so any matching conditional assignment wins over it), and of
/// the last such assignment where two are equally specific. An included file's lines count as if
/// they appeared where it is included. `$(inherited)` expands to the value held so far. Best
/// effort: a missing file contributes nothing, which is what Xcode does for `#include?`.
fn xcconfig_settings(path: &Path, build: &XcconfigBuild<'_>) -> XcconfigSettings {
    let mut held: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut unmatched: BTreeMap<String, Vec<String>> = BTreeMap::new();
    read_xcconfig(path, build, &mut held, &mut unmatched, 0);
    let values: BTreeMap<String, String> = held.into_iter().map(|(k, (_, v))| (k, v)).collect();
    unmatched.retain(|name, _| !values.contains_key(name));
    XcconfigSettings { values, unmatched }
}

/// One file of [`xcconfig_settings`]: `held` is each setting's value so far with the number of
/// conditions the assignment that set it carried.
fn read_xcconfig(
    path: &Path,
    build: &XcconfigBuild<'_>,
    held: &mut BTreeMap<String, (usize, String)>,
    unmatched: &mut BTreeMap<String, Vec<String>>,
    depth: usize,
) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("#include?")
            .or_else(|| line.strip_prefix("#include"))
        {
            let included = rest.trim().trim_matches('"');
            if depth < 8 && !included.is_empty() {
                read_xcconfig(&dir.join(included), build, held, unmatched, depth + 1);
            }
            continue;
        }
        let line = line.split("//").next().unwrap_or_default().trim();
        // The `=` that ends the key is the first one outside a `[condition=value]`.
        let mut nesting = 0;
        let Some(eq) = line.char_indices().find_map(|(i, c)| {
            match c {
                '[' => nesting += 1,
                ']' => nesting -= 1,
                '=' if nesting == 0 => return Some(i),
                _ => {}
            }
            None
        }) else {
            continue;
        };
        let (key, value) = (line[..eq].trim(), &line[eq + 1..]);
        let value = value.trim().trim_end_matches(';').trim();
        let (name, conditions) = match key.split_once('[') {
            Some((name, conds)) => (name.trim(), conds),
            None => (key, ""),
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        // `[config=Debug][sdk=iphoneos*]` is two conditions, both of which have to hold.
        let conditions: Vec<&str> = conditions
            .split('[')
            .map(|c| c.trim().trim_end_matches(']').trim())
            .filter(|c| !c.is_empty())
            .collect();
        if !conditions
            .iter()
            .all(|c| xcconfig_condition_holds(c, build))
        {
            unmatched
                .entry(name.to_string())
                .or_default()
                .push(format!("{name}[{}]", conditions.join("][")));
            continue;
        }
        let specificity = conditions.len();
        if held.get(name).is_some_and(|(s, _)| *s > specificity) {
            continue;
        }
        let inherited = held.get(name).map(|(_, v)| v.as_str()).unwrap_or_default();
        let value = value.replace("$(inherited)", inherited).trim().to_string();
        held.insert(name.to_string(), (specificity, value));
    }
}

/// Whether one `key=pattern` condition of an xcconfig assignment holds for `build`. Patterns take
/// `*` wildcards; an SDK pattern may also name a version (`iphoneos18.0`), which Day does not
/// know, so it matches on the platform name. Conditions Day does not evaluate never hold.
fn xcconfig_condition_holds(condition: &str, build: &XcconfigBuild<'_>) -> bool {
    let Some((key, pattern)) = condition.split_once('=') else {
        return false;
    };
    let pattern = pattern.trim();
    match key.trim() {
        "config" => glob_matches(pattern, build.configuration),
        "arch" => glob_matches(pattern, build.arch),
        "sdk" => {
            glob_matches(pattern, build.sdk)
                || pattern.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '*')
                    == build.sdk
        }
        _ => false,
    }
}

/// Whether `value` matches `pattern`, where `*` stands for any run of characters.
fn glob_matches(pattern: &str, value: &str) -> bool {
    let Some((first, rest)) = pattern.split_once('*') else {
        return pattern == value;
    };
    let Some(mut tail) = value.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = rest.split('*').collect();
    let last = parts.pop().unwrap_or_default();
    for part in parts {
        match tail.find(part) {
            Some(at) => tail = &tail[at + part.len()..],
            None => return false,
        }
    }
    tail.len() >= last.len() && tail.ends_with(last)
}

/// Expand `$(VAR)` and `${VAR}` references against `vars`, recursively (a setting may name
/// another). Understands the `:lower`, `:upper` and `:rfc1034identifier` modifiers; `$(inherited)`
/// expands to nothing. The error is the first variable `vars` does not define.
pub(crate) fn expand_build_variables(
    raw: &str,
    vars: &BTreeMap<String, String>,
) -> Result<String, String> {
    fn go(raw: &str, vars: &BTreeMap<String, String>, depth: usize) -> Result<String, String> {
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(at) = rest.find('$') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            let close = match after.as_bytes().first() {
                Some(b'(') => ')',
                Some(b'{') => '}',
                _ => {
                    out.push('$');
                    rest = after;
                    continue;
                }
            };
            let Some(end) = after.find(close) else {
                out.push_str(&rest[at..]);
                rest = "";
                break;
            };
            let reference = &after[1..end];
            rest = &after[end + 1..];
            let (name, modifier) = match reference.split_once(':') {
                Some((n, m)) => (n, Some(m)),
                None => (reference, None),
            };
            if name == "inherited" {
                continue;
            }
            let value = vars.get(name).ok_or_else(|| name.to_string())?;
            let value = if depth < 16 {
                go(value, vars, depth + 1)?
            } else {
                value.clone()
            };
            out.push_str(&match modifier {
                Some("lower") => value.to_lowercase(),
                Some("upper") => value.to_uppercase(),
                Some("rfc1034identifier") => value
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect(),
                _ => value,
            });
        }
        out.push_str(rest);
        Ok(out)
    }
    go(raw, vars, 0)
}

/// Which side of Apple's development/distribution split a signature is for. A development
/// profile (it grants `get-task-allow`) signs device launches; a distribution profile signs what
/// `day pack` hands a store. The push environment follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApsEnvironment {
    Development,
    Production,
}

impl ApsEnvironment {
    /// The kind of signature `profile` makes: development when it grants `get-task-allow`.
    pub(crate) fn of_profile(profile: &plist::Dictionary) -> Self {
        if profile
            .get("get-task-allow")
            .and_then(plist::Value::as_boolean)
            == Some(true)
        {
            Self::Development
        } else {
            Self::Production
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

/// The entitlements to sign an iOS bundle with, and what had to be dropped from the app's file
/// because the profile does not grant it (one line per entitlement or value, for a warning).
///
/// `granted` is the profile's `Entitlements` dictionary. With `app` (the app's own file, as read,
/// and the build settings it resolves against), the result is that file with every `$(VAR)`
/// resolved — the identifier prefixes and the bundle id from the profile and the bundle, a
/// variable `aps-environment` from the kind of signature — intersected with `granted`, plus the
/// identifiers every signature carries (`application-identifier`, the team, `get-task-allow`,
/// `beta-reports-active`), which Xcode takes from the profile too. Without `app`, it is `granted`
/// with a wildcard `application-identifier` narrowed to this bundle.
pub(crate) fn ios_signing_entitlements(
    granted: &plist::Dictionary,
    app: Option<(&plist::Dictionary, &BTreeMap<String, String>)>,
    bundle_id: &str,
) -> Result<(plist::Dictionary, Vec<String>), String> {
    let string = |key: &str| {
        granted
            .get(key)
            .and_then(plist::Value::as_string)
            .map(str::to_string)
    };
    let profile_app_id = string("application-identifier").unwrap_or_default();
    // The App ID prefix is usually the team id, but not for an app id registered before teams
    // had one; Xcode's `AppIdentifierPrefix` is the profile's, so this is too.
    let app_prefix = profile_app_id
        .split_once('.')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    let team = string("com.apple.developer.team-identifier").unwrap_or_else(|| app_prefix.clone());
    let concrete_app_id = if profile_app_id.ends_with('*') || profile_app_id.is_empty() {
        format!("{app_prefix}.{bundle_id}")
    } else {
        profile_app_id.clone()
    };

    let Some((requested, settings)) = app else {
        let mut all = granted.clone();
        if !profile_app_id.is_empty() {
            all.insert("application-identifier".into(), concrete_app_id.into());
        }
        return Ok((all, Vec::new()));
    };

    let mut vars = settings.clone();
    for (key, value) in [
        ("AppIdentifierPrefix", format!("{app_prefix}.")),
        ("TeamIdentifierPrefix", format!("{team}.")),
        ("DEVELOPMENT_TEAM", team.clone()),
        ("PRODUCT_BUNDLE_IDENTIFIER", bundle_id.to_string()),
        ("CFBundleIdentifier", bundle_id.to_string()),
    ] {
        vars.insert(key.into(), value);
    }
    let aps = ApsEnvironment::of_profile(granted);
    let mut resolved = plist::Dictionary::new();
    for (key, value) in requested {
        // The push environment is the one value Xcode projects leave to the configuration
        // (`$(DAY_APNS_ENVIRONMENT)`). Whatever the variable is called, the signature's kind
        // decides it: a development profile only ever grants development, a store one production.
        if key == "aps-environment"
            && value
                .as_string()
                .is_some_and(|s| s.contains("$(") || s.contains("${"))
        {
            resolved.insert(key.clone(), aps.as_str().into());
            continue;
        }
        let value = expand_plist_value(value, &vars).map_err(|var| {
            format!(
                "entitlement {key} references $({var}), which Day cannot resolve — define it in \
                 platform/ios/DayApp.xcconfig or write the value out"
            )
        })?;
        resolved.insert(key.clone(), value);
    }

    let (mut signed, dropped) = intersect_entitlements(resolved, granted);
    signed.insert("application-identifier".into(), concrete_app_id.into());
    if !team.is_empty() {
        signed.insert(
            "com.apple.developer.team-identifier".into(),
            team.clone().into(),
        );
    }
    for key in ["get-task-allow", "beta-reports-active"] {
        if let Some(v) = granted.get(key) {
            signed.insert(key.into(), v.clone());
        }
    }
    Ok((signed, dropped))
}

/// `value` with every string in it expanded by [`expand_build_variables`].
fn expand_plist_value(
    value: &plist::Value,
    vars: &BTreeMap<String, String>,
) -> Result<plist::Value, String> {
    Ok(match value {
        plist::Value::String(s) => plist::Value::String(expand_build_variables(s, vars)?),
        plist::Value::Array(items) => plist::Value::Array(
            items
                .iter()
                .map(|v| expand_plist_value(v, vars))
                .collect::<Result<_, _>>()?,
        ),
        plist::Value::Dictionary(d) => {
            let mut out = plist::Dictionary::new();
            for (k, v) in d {
                out.insert(k.clone(), expand_plist_value(v, vars)?);
            }
            plist::Value::Dictionary(out)
        }
        other => other.clone(),
    })
}

/// Keep what `granted` allows of `requested`: a key the profile lacks goes, an array keeps the
/// elements the profile's value covers, and a string has to match it. A profile value covers a
/// request when it is equal, `*`, or a prefix ending in `*` (`TEAMID.*` covers every keychain
/// group of the team). Returns what is kept and a line for everything dropped.
fn intersect_entitlements(
    requested: plist::Dictionary,
    granted: &plist::Dictionary,
) -> (plist::Dictionary, Vec<String>) {
    fn covers(grant: &plist::Value, want: &str) -> bool {
        let one = |g: &str| {
            g == want || g == "*" || g.strip_suffix('*').is_some_and(|p| want.starts_with(p))
        };
        match grant {
            plist::Value::String(g) => one(g),
            plist::Value::Array(gs) => gs.iter().filter_map(plist::Value::as_string).any(one),
            _ => false,
        }
    }
    let mut kept = plist::Dictionary::new();
    let mut dropped = Vec::new();
    for (key, value) in requested {
        let Some(grant) = granted.get(&key) else {
            dropped.push(key.clone());
            continue;
        };
        match &value {
            // `false` claims nothing.
            plist::Value::Boolean(false) => {
                kept.insert(key, value);
            }
            plist::Value::Boolean(true) => {
                if grant.as_boolean() == Some(false) {
                    dropped.push(format!("{key} (granted as false)"));
                } else {
                    kept.insert(key, value);
                }
            }
            plist::Value::String(want) => {
                if covers(grant, want) {
                    kept.insert(key, value);
                } else {
                    dropped.push(format!("{key} = {want}"));
                }
            }
            plist::Value::Array(items) => {
                let mut allowed = Vec::new();
                for item in items {
                    match item.as_string() {
                        Some(want) if covers(grant, want) => allowed.push(item.clone()),
                        Some(want) => dropped.push(format!("{key} {want}")),
                        None => dropped.push(format!("{key} (a value that is not a string)")),
                    }
                }
                if !allowed.is_empty() {
                    kept.insert(key, plist::Value::Array(allowed));
                }
            }
            _ => {
                if *grant == value {
                    kept.insert(key, value);
                } else {
                    dropped.push(format!("{key} (differs from the profile's value)"));
                }
            }
        }
    }
    (kept, dropped)
}

/// The profile's `Entitlements` dictionary, from a profile already decoded to a plist.
pub(crate) fn decoded_profile_entitlements(decoded: &Path) -> Result<plist::Dictionary, String> {
    plist::Value::from_file(decoded)
        .map_err(|e| format!("read {}: {e}", decoded.display()))?
        .as_dictionary()
        .and_then(|d| d.get("Entitlements"))
        .and_then(plist::Value::as_dictionary)
        .cloned()
        .ok_or_else(|| format!("{} has no Entitlements", decoded.display()))
}

/// Write the entitlements an iOS bundle is signed with to `out`, warning about each one the
/// profile made Day drop. `app` is the app's own file (see [`ios_signing_entitlements`]).
pub(crate) fn write_ios_signing_entitlements(
    granted: &plist::Dictionary,
    app: Option<&AppEntitlements>,
    bundle_id: &str,
    profile_name: &str,
    out: &Path,
) -> Result<plist::Dictionary, String> {
    let requested = match app {
        Some(a) => Some(
            plist::Value::from_file(&a.path)
                .map_err(|e| format!("entitlements {}: {e}", a.path.display()))?
                .into_dictionary()
                .ok_or_else(|| format!("{} is not a plist dictionary", a.path.display()))?,
        ),
        None => None,
    };
    let (signed, dropped) = ios_signing_entitlements(
        granted,
        requested.as_ref().zip(app.map(|a| &a.settings)),
        bundle_id,
    )?;
    if let Some(a) = app {
        for line in &dropped {
            status(
                "Warning",
                &format!(
                    "{} asks for entitlement {line}, which the profile {profile_name:?} does not \
                     grant — signing without it",
                    a.path.display()
                ),
            );
        }
    }
    plist::Value::Dictionary(signed.clone())
        .to_file_xml(out)
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(signed)
}

/// The sha256 of the signing certificate itself, so an Apple signature reports the same "which
/// key" field an Android one does. Best effort: a missing certificate is not a signing failure.
pub(crate) fn leaf_certificate_sha256(bundle: &Path, work: &Path) -> Option<String> {
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
        // Nested frameworks and libraries come deepest first. App extensions are not in this list:
        // each is signed on its own, with its own profile and entitlements, before the app.
        let root = std::env::temp_dir().join(format!("day-sign-test-{}", std::process::id()));
        let plugin = root.join("PlugIns/Share.appex");
        let inner = plugin.join("Frameworks/Inner.framework");
        std::fs::create_dir_all(&inner).expect("temp bundle");
        std::fs::create_dir_all(root.join("Extensions/Market.appex")).expect("temp bundle");
        std::fs::create_dir_all(root.join("Frameworks/Day.framework")).expect("temp bundle");
        std::fs::write(root.join("Frameworks/libday.dylib"), b"").expect("temp bundle");
        let order = nested_code(&root);
        let depth = |p: &Path| p.components().count();
        assert!(
            order.windows(2).all(|w| depth(&w[0]) >= depth(&w[1])),
            "not deepest-first: {order:?}"
        );
        assert!(order.iter().any(|p| p.ends_with("libday.dylib")));
        assert!(!order.iter().any(|p| p.ends_with("Share.appex")));
        let appex = crate::extensions::embedded(&root);
        assert!(appex.iter().any(|p| p.ends_with("PlugIns/Share.appex")));
        assert!(appex.iter().any(|p| p.ends_with("Extensions/Market.appex")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keytool_reports_the_same_shape() {
        let report = "Owner: CN=Day\n         SHA256: A1:B2:C3:D4\n";
        assert_eq!(keytool_fingerprint(report).as_deref(), Some("a1b2c3d4"));
        assert_eq!(apksigner_fingerprint("nothing here"), None);
    }
}

#[cfg(test)]
mod ios_entitlement_tests {
    use super::*;

    /// A team profile as Apple issues it: more than any one app claims, with wildcards.
    fn store_profile() -> plist::Dictionary {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>application-identifier</key><string>TEAM123.org.example.Browser</string>
  <key>com.apple.developer.team-identifier</key><string>TEAM123</string>
  <key>aps-environment</key><string>production</string>
  <key>beta-reports-active</key><true/>
  <key>get-task-allow</key><false/>
  <key>com.apple.developer.game-center</key><true/>
  <key>com.apple.developer.web-browser</key><true/>
  <key>com.apple.developer.associated-domains</key><string>*</string>
  <key>com.apple.security.application-groups</key>
  <array><string>group.org.example</string></array>
  <key>keychain-access-groups</key>
  <array><string>TEAM123.*</string><string>com.apple.token</string></array>
</dict></plist>"#;
        plist::Value::from_reader_xml(text.as_bytes())
            .unwrap()
            .into_dictionary()
            .unwrap()
    }

    fn dev_profile() -> plist::Dictionary {
        let mut p = store_profile();
        p.insert(
            "application-identifier".into(),
            "TEAM123.*".to_string().into(),
        );
        p.insert("aps-environment".into(), "development".to_string().into());
        p.insert("get-task-allow".into(), true.into());
        p.remove("beta-reports-active");
        p
    }

    fn app_file() -> plist::Dictionary {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>aps-environment</key><string>$(DAY_APNS_ENVIRONMENT)</string>
  <key>com.apple.developer.web-browser</key><true/>
  <key>com.apple.developer.siri</key><true/>
  <key>com.apple.developer.associated-domains</key>
  <array><string>applinks:$(DAY_LINK_HOST)</string></array>
  <key>com.apple.security.application-groups</key>
  <array><string>group.org.example</string><string>group.org.other</string></array>
  <key>keychain-access-groups</key>
  <array><string>$(AppIdentifierPrefix)org.example</string></array>
</dict></plist>"#;
        plist::Value::from_reader_xml(text.as_bytes())
            .unwrap()
            .into_dictionary()
            .unwrap()
    }

    fn settings() -> BTreeMap<String, String> {
        BTreeMap::from([("DAY_LINK_HOST".to_string(), "example.org".to_string())])
    }

    fn strings(d: &plist::Dictionary, key: &str) -> Vec<String> {
        d.get(key)
            .and_then(plist::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(plist::Value::as_string)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn build_variables_resolve_like_xcode() {
        let vars = BTreeMap::from([
            ("ROOT".to_string(), "/p".to_string()),
            ("FILE".to_string(), "$(ROOT)/a.entitlements".to_string()),
            ("NAME".to_string(), "Net Skip".to_string()),
        ]);
        assert_eq!(
            expand_build_variables("$(FILE)", &vars).unwrap(),
            "/p/a.entitlements"
        );
        assert_eq!(
            expand_build_variables("${ROOT}-$(NAME:rfc1034identifier)-$(NAME:lower)", &vars)
                .unwrap(),
            "/p-Net-Skip-net skip"
        );
        assert_eq!(
            expand_build_variables("$(inherited) $5", &vars).unwrap(),
            " $5"
        );
        assert_eq!(
            expand_build_variables("$(MISSING)/x", &vars).unwrap_err(),
            "MISSING"
        );
    }

    #[test]
    fn the_app_file_wins_within_what_the_profile_grants() {
        let (signed, dropped) = ios_signing_entitlements(
            &store_profile(),
            Some((&app_file(), &settings())),
            "org.example.Browser",
        )
        .unwrap();
        // The variables resolve: the App ID prefix, a setting, and the push environment from the
        // kind of profile.
        assert_eq!(
            strings(&signed, "keychain-access-groups"),
            ["TEAM123.org.example"]
        );
        assert_eq!(
            strings(&signed, "com.apple.developer.associated-domains"),
            ["applinks:example.org"]
        );
        assert_eq!(
            signed.get("aps-environment").and_then(|v| v.as_string()),
            Some("production")
        );
        // What the profile grants and the app never asked for stays out.
        assert!(!signed.contains_key("com.apple.developer.game-center"));
        // What the app asks for and the profile lacks is dropped, by name.
        assert!(!signed.contains_key("com.apple.developer.siri"));
        assert_eq!(
            strings(&signed, "com.apple.security.application-groups"),
            ["group.org.example"]
        );
        assert_eq!(
            dropped,
            [
                "com.apple.developer.siri",
                "com.apple.security.application-groups group.org.other"
            ]
        );
        // The identifiers every signature carries come from the profile.
        let s = |k: &str| {
            signed
                .get(k)
                .and_then(|v| v.as_string())
                .map(str::to_string)
        };
        assert_eq!(
            s("application-identifier").as_deref(),
            Some("TEAM123.org.example.Browser")
        );
        assert_eq!(
            s("com.apple.developer.team-identifier").as_deref(),
            Some("TEAM123")
        );
        assert_eq!(
            signed
                .get("beta-reports-active")
                .and_then(|v| v.as_boolean()),
            Some(true)
        );
        assert_eq!(
            signed.get("get-task-allow").and_then(|v| v.as_boolean()),
            Some(false)
        );
    }

    #[test]
    fn a_development_profile_signs_for_development_push() {
        let (signed, _) = ios_signing_entitlements(
            &dev_profile(),
            Some((&app_file(), &settings())),
            "org.example.Browser.dev",
        )
        .unwrap();
        let s = |k: &str| {
            signed
                .get(k)
                .and_then(|v| v.as_string())
                .map(str::to_string)
        };
        assert_eq!(s("aps-environment").as_deref(), Some("development"));
        // A wildcard profile's app id narrows to the bundle being signed.
        assert_eq!(
            s("application-identifier").as_deref(),
            Some("TEAM123.org.example.Browser.dev")
        );
        assert_eq!(
            signed.get("get-task-allow").and_then(|v| v.as_boolean()),
            Some(true)
        );
        // A literal environment the profile does not grant is dropped, not rewritten.
        let mut literal = app_file();
        literal.insert("aps-environment".into(), "production".to_string().into());
        let (signed, dropped) =
            ios_signing_entitlements(&dev_profile(), Some((&literal, &settings())), "x.y").unwrap();
        assert!(!signed.contains_key("aps-environment"));
        assert!(dropped.contains(&"aps-environment = production".to_string()));
    }

    #[test]
    fn an_unresolved_variable_is_an_error_naming_it() {
        let err = ios_signing_entitlements(
            &store_profile(),
            Some((&app_file(), &BTreeMap::new())),
            "org.example.Browser",
        )
        .unwrap_err();
        assert!(err.contains("DAY_LINK_HOST"), "{err}");
    }

    #[test]
    fn without_an_app_file_the_profile_set_stands() {
        let (signed, dropped) =
            ios_signing_entitlements(&dev_profile(), None, "org.example.B").expect("profile set");
        assert!(dropped.is_empty());
        assert!(signed.contains_key("com.apple.developer.game-center"));
        assert_eq!(
            signed
                .get("application-identifier")
                .and_then(|v| v.as_string()),
            Some("TEAM123.org.example.B")
        );
    }

    /// A device build of `configuration`, the build Day signs.
    fn device(configuration: &str) -> XcconfigBuild<'_> {
        XcconfigBuild {
            configuration,
            sdk: "iphoneos",
            arch: "arm64",
        }
    }

    #[test]
    fn xcconfig_settings_follow_includes_and_the_configuration() {
        let dir = std::env::temp_dir().join(format!("day-xcconfig-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("gen")).unwrap();
        std::fs::write(
            dir.join("DayApp.xcconfig"),
            "// comment\nA = one\nAPS = production\nAPS[config=Debug] = development\n\
             B[sdk=macosx*] = skipped\n#include? \"gen/ios.xcconfig\"\n#include? \"absent\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("gen/ios.xcconfig"),
            "A = $(inherited) two // trailing\n",
        )
        .unwrap();
        let debug = xcconfig_settings(&dir.join("DayApp.xcconfig"), &device("Debug"));
        assert_eq!(debug.values.get("A").map(String::as_str), Some("one two"));
        assert_eq!(
            debug.values.get("APS").map(String::as_str),
            Some("development")
        );
        assert!(!debug.values.contains_key("B"));
        assert_eq!(debug.unmatched["B"], ["B[sdk=macosx*]"]);
        let release = xcconfig_settings(&dir.join("DayApp.xcconfig"), &device("Release"));
        assert_eq!(
            release.values.get("APS").map(String::as_str),
            Some("production")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Conditional settings resolve for the build being signed, as Xcode resolves them: every
    /// condition has to hold, `*` is a wildcard, the most specific match wins whatever order the
    /// lines come in, and a later line wins among equally specific ones.
    #[test]
    fn xcconfig_conditions_pick_the_most_specific_match_for_the_build() {
        let dir = std::env::temp_dir().join(format!("day-xcconfig-cond-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("DayApp.xcconfig");
        std::fs::write(
            &file,
            "ENTS[sdk=iphoneos*] = App.entitlements\n\
             ENTS[sdk=iphonesimulator*] = Sim.entitlements\n\
             ENTS[sdk=macosx*] = Mac.entitlements\n\
             ENTS = Plain.entitlements\n\
             ENTS[config=Release][sdk=iphoneos*] = Store.entitlements\n\
             ARCH[arch=x86_64] = intel\n\
             ARCH[arch=arm*] = arm\n\
             VER[sdk=iphoneos18.0] = versioned\n\
             LAST[sdk=iphoneos*] = first\n\
             LAST[sdk=*] = second\n\
             ODD[variant=profile] = unevaluated\n\
             ONLY[sdk=iphonesimulator*] = sim\n",
        )
        .unwrap();
        let read = |configuration, sdk, arch| {
            xcconfig_settings(
                &file,
                &XcconfigBuild {
                    configuration,
                    sdk,
                    arch,
                },
            )
        };
        let get = |s: &XcconfigSettings, k: &str| s.values.get(k).cloned().unwrap_or_default();

        let debug = read("Debug", "iphoneos", "arm64");
        assert_eq!(get(&debug, "ENTS"), "App.entitlements");
        assert_eq!(get(&debug, "ARCH"), "arm");
        assert_eq!(get(&debug, "VER"), "versioned");
        assert_eq!(get(&debug, "LAST"), "second");
        assert!(!debug.values.contains_key("ODD"));
        assert_eq!(debug.unmatched["ONLY"], ["ONLY[sdk=iphonesimulator*]"]);
        assert!(!debug.unmatched.contains_key("ENTS"), "ENTS did apply");

        let release = read("Release", "iphoneos", "arm64");
        assert_eq!(get(&release, "ENTS"), "Store.entitlements");
        let sim = read("Debug", "iphonesimulator", "x86_64");
        assert_eq!(get(&sim, "ENTS"), "Sim.entitlements");
        assert_eq!(get(&sim, "ARCH"), "intel");
        assert_eq!(get(&sim, "ONLY"), "sim");
        assert_eq!(
            get(&read("Release", "macosx", "arm64"), "ENTS"),
            "Mac.entitlements"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn glob_patterns_match_like_xcode_conditions() {
        assert!(glob_matches("iphoneos*", "iphoneos"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("*os", "iphoneos"));
        assert!(glob_matches("i*o*s", "iphoneos"));
        assert!(!glob_matches("iphoneos*", "iphonesimulator"));
        assert!(!glob_matches("Debug", "Release"));
        assert!(glob_matches("a*ba", "aba"));
        assert!(!glob_matches("ab*ba", "aba"), "the two ends cannot overlap");
    }

    #[test]
    fn signing_ios_entitlements_wins_over_the_xcconfig() {
        let dir = std::env::temp_dir()
            .join(format!("day-ios-ents-{}", std::process::id()))
            .join("ents-fixture-root");
        std::fs::create_dir_all(dir.join("platform/ios/Runner")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname='ents-fixture'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        let plist = "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict/></plist>";
        std::fs::write(dir.join("platform/ios/Runner/App.entitlements"), plist).unwrap();
        std::fs::write(dir.join("Store.entitlements"), plist).unwrap();
        std::fs::write(
            dir.join("platform/ios/DayApp.xcconfig"),
            "CODE_SIGN_ENTITLEMENTS = $(DAY_PROJECT_ROOT)/platform/ios/Runner/App.entitlements\n",
        )
        .unwrap();
        let manifest = |extra: &str| {
            std::fs::write(
                dir.join("Day.toml"),
                format!("schema=1\n[app]\nid='org.test.ents'\n{extra}"),
            )
            .unwrap();
            crate::meta::find_project(Some(&dir)).unwrap()
        };
        // The xcconfig's setting, with its variable resolved.
        let found = ios_app_entitlements(&manifest(""), "Release")
            .unwrap()
            .unwrap();
        assert!(found.path.ends_with("platform/ios/Runner/App.entitlements"));
        // An SDK-conditioned setting is the device build's file, not a quiet fallback to the
        // profile's set.
        std::fs::write(
            dir.join("platform/ios/DayApp.xcconfig"),
            "CODE_SIGN_ENTITLEMENTS[sdk=iphoneos*] = $(DAY_PROJECT_ROOT)/platform/ios/Runner/App.entitlements\n\
             CODE_SIGN_ENTITLEMENTS[sdk=iphonesimulator*] = Runner/Sim.entitlements\n",
        )
        .unwrap();
        let found = ios_app_entitlements(&manifest(""), "Release")
            .unwrap()
            .unwrap();
        assert!(found.path.ends_with("platform/ios/Runner/App.entitlements"));
        // Set only for the simulator: nothing for the device build (Day warns about it).
        std::fs::write(
            dir.join("platform/ios/DayApp.xcconfig"),
            "CODE_SIGN_ENTITLEMENTS[sdk=iphonesimulator*] = Runner/App.entitlements\n",
        )
        .unwrap();
        assert!(
            ios_app_entitlements(&manifest(""), "Release")
                .unwrap()
                .is_none()
        );
        // Day.toml's key wins.
        let project =
            manifest("[signing.ios]\nteam='TEAM123'\nentitlements='Store.entitlements'\n");
        let found = ios_app_entitlements(&project, "Release").unwrap().unwrap();
        assert!(found.path.ends_with("ents-fixture-root/Store.entitlements"));
        // A missing file is an error, not a quiet fallback to the profile's set.
        let project = manifest("[signing.ios]\nteam='TEAM123'\nentitlements='Gone.entitlements'\n");
        assert!(ios_app_entitlements(&project, "Release").is_err());
        // Neither: the profile's set.
        std::fs::write(dir.join("platform/ios/DayApp.xcconfig"), "A = 1\n").unwrap();
        assert!(
            ios_app_entitlements(&manifest(""), "Release")
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
