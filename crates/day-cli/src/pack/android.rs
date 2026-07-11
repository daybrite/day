//! android-widget → release .apk + .aab. Day.toml identity/version is conveyed to Gradle via a
//! generated properties file (§17.5); the release signingConfig reads a second generated file —
//! resolved from `signing.android` `${ENV}` refs, or a CI/dev keystore generated with keytool when
//! unconfigured (dev tier, loud). Gradle signs both formats (apksigner cannot sign an .aab — §16.5).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::{PackOptions, resolve_degradable};
use super::{Artifact, PackError, SignTier, run_tool};
use crate::meta::Project;
use crate::ops::{self, status};
use crate::targets::Target;

const DEV_KEYSTORE_PASS: &str = "day-dev-only"; // dev keystore: local installs only, never distribution

/// Day.toml → `build/day/android/day-app.properties` (applicationId, versionCode, versionName,
/// title). Written on every android build (`day build` too) so the Gradle scaffold never goes
/// stale (§17.5). Identity is RESOLVED for the android target, so `[app.android]` /
/// `[app.android-widget]` overrides in Day.toml flow into the APK.
pub(crate) fn write_app_properties(project: &Project) -> Result<(), String> {
    let dir = project.root.join("build/day/android");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let resolved = project.manifest.resolve("android-widget");
    let content = format!(
        "applicationId={}\nversionCode={}\nversionName={}\ntitle={}\n",
        resolved.id,
        resolved.build.min(i32::MAX as u64),
        resolved.version,
        resolved.title
    );
    let path = dir.join("day-app.properties");
    // Content-hashed write: only touch the file when it changed (keeps Gradle up-to-date checks warm).
    if std::fs::read_to_string(&path).ok().as_deref() != Some(&content) {
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn pack(
    project: &Project,
    target: &'static Target,
    opts: &PackOptions,
    dist: &Path,
    formats: &[String],
) -> Result<Vec<Artifact>, PackError> {
    write_app_properties(project).map_err(PackError::Other)?;

    // Resolve (or generate) the signing keystore BEFORE gradle runs — the scaffold's release
    // signingConfig reads the generated properties file at configuration time.
    let signing_props = project
        .root
        .join("build/day/android/day-signing.properties");
    let _ = std::fs::remove_file(&signing_props);
    let tier = if opts.no_sign {
        status("Signing", "skipped (--no-sign) — unsigned release apk");
        SignTier::Unsigned
    } else {
        write_signing_properties(project, &signing_props)?
    };

    // Build: cargo-ndk .so + gradle assembleRelease (ops::build), then bundleRelease for the .aab.
    let outcome = ops::build(project, target, &opts.profile).map_err(PackError::Other)?;

    let mut artifacts = Vec::new();
    let name = &project.manifest.app.name;
    let version = &project.manifest.app.version;

    if formats.iter().any(|f| f == "apk") {
        let apk = find_output(&outcome.artifact, project, &opts.profile, "apk")?;
        verify_apk(project, &apk);
        let out = dist.join(format!("{name}-{version}.apk"));
        std::fs::copy(&apk, &out).map_err(|e| PackError::Other(e.to_string()))?;
        artifacts.push(Artifact {
            path: out,
            kind: "apk",
            sha256: String::new(),
            tier,
        });
    }

    if formats.iter().any(|f| f == "aab") && opts.profile == "release" {
        status("Building", "android-widget (gradle bundleRelease)");
        let day_bin = std::env::current_exe().map_err(|e| PackError::Other(e.to_string()))?;
        let mut cmd = Command::new("gradle");
        cmd.current_dir(project.root.join("platform/android"))
            .env("DAY_BIN", &day_bin)
            .env("DAY_PROJECT_ROOT", &project.root)
            .env("DAY_PROFILE", &opts.profile)
            .args(["bundleRelease", "-q", "--console=plain"]);
        if std::env::var_os("JAVA_HOME").is_none()
            && let Some(jdk) = day_toolchain::jdk21_home()
        {
            cmd.env("JAVA_HOME", jdk);
        }
        run_tool(&mut cmd, "gradle bundleRelease").map_err(PackError::Other)?;
        let aab = project
            .root
            .join("platform/android/app/build/outputs/bundle/release/app-release.aab");
        if !aab.exists() {
            return Err(PackError::Other(format!(
                "gradle bundleRelease produced no aab at {}",
                aab.display()
            )));
        }
        let out = dist.join(format!("{name}-{version}.aab"));
        std::fs::copy(&aab, &out).map_err(|e| PackError::Other(e.to_string()))?;
        artifacts.push(Artifact {
            path: out,
            kind: "aab",
            sha256: String::new(),
            tier,
        });
    }

    Ok(artifacts)
}

/// Resolve signing.android (env-interpolated) into the generated Gradle properties file; without
/// config, generate a persistent dev keystore so release builds stay installable (dev tier, loud).
fn write_signing_properties(project: &Project, path: &Path) -> Result<SignTier, PackError> {
    let android = project
        .manifest
        .signing
        .as_ref()
        .and_then(|s| s.android.as_ref());
    // Any unresolved secret degrades the WHOLE section to the dev keystore (§20) — a half-resolved
    // signing config must never sign with mixed material.
    let release = match android {
        Some(a) => {
            let fields = (
                resolve_degradable(&a.keystore, "signing.android.keystore")
                    .map_err(PackError::Sign)?,
                resolve_degradable(&a.store_pass, "signing.android.store-pass")
                    .map_err(PackError::Sign)?,
                resolve_degradable(&a.key_alias, "signing.android.key-alias")
                    .map_err(PackError::Sign)?,
                resolve_degradable(&a.key_pass, "signing.android.key-pass")
                    .map_err(PackError::Sign)?,
            );
            match fields {
                (Some(keystore), Some(store_pass), Some(key_alias), Some(key_pass)) => {
                    let keystore = project.root.join(keystore);
                    if !keystore.exists() {
                        return Err(PackError::Sign(format!(
                            "signing.android.keystore not found: {}",
                            keystore.display()
                        )));
                    }
                    Some((keystore, store_pass, key_alias, key_pass))
                }
                _ => None,
            }
        }
        None => None,
    };
    let (store_file, store_pass, key_alias, key_pass, tier) = match release {
        Some((keystore, store_pass, key_alias, key_pass)) => {
            status("Signing", "release keystore (signing.android)");
            (keystore, store_pass, key_alias, key_pass, SignTier::Release)
        }
        None => {
            let keystore = dev_keystore(project).map_err(PackError::Other)?;
            status(
                "Signing",
                "dev keystore (release signing unavailable) — NOT for distribution",
            );
            (
                keystore,
                DEV_KEYSTORE_PASS.into(),
                "day-dev".into(),
                DEV_KEYSTORE_PASS.into(),
                SignTier::DevSigned,
            )
        }
    };
    // Gradle's Properties loader treats '\' as an escape — normalize to forward slashes (valid on
    // Windows for java.io.File too).
    let content = format!(
        "storeFile={}\nstorePassword={}\nkeyAlias={}\nkeyPassword={}\n",
        store_file.display().to_string().replace('\\', "/"),
        store_pass,
        key_alias,
        key_pass
    );
    std::fs::write(path, content).map_err(|e| PackError::Other(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(tier)
}

/// A per-project dev keystore under build/day/ (generated once with keytool; PKCS12).
fn dev_keystore(project: &Project) -> Result<PathBuf, String> {
    let path = project.root.join("build/day/android/day-dev.keystore");
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    status("Signing", "generating dev keystore (keytool)");
    run_tool(
        Command::new("keytool")
            .args(["-genkeypair", "-v", "-storetype", "PKCS12"])
            .arg("-keystore")
            .arg(&path)
            .args(["-keyalg", "RSA", "-keysize", "2048", "-validity", "10000"])
            .args(["-alias", "day-dev"])
            .args([
                "-storepass",
                DEV_KEYSTORE_PASS,
                "-keypass",
                DEV_KEYSTORE_PASS,
            ])
            .args(["-dname", "CN=Day Development"]),
        "keytool",
    )?;
    Ok(path)
}

/// The built apk: ops::build returns the conventional path, but an unsigned release build is named
/// `app-release-unsigned.apk` — fall back to any .apk in the outputs dir.
fn find_output(
    conventional: &Path,
    project: &Project,
    profile: &str,
    ext: &str,
) -> Result<PathBuf, PackError> {
    if conventional.exists() {
        return Ok(conventional.to_path_buf());
    }
    let dir = project
        .root
        .join("platform/android/app/build/outputs/apk")
        .join(profile);
    std::fs::read_dir(&dir)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
        })
        .ok_or_else(|| PackError::Other(format!("no .{ext} produced under {}", dir.display())))
}

/// Post-sign verification, best-effort (needs Android build-tools on the host): apksigner verify
/// + a 16 KB page-alignment check on the bundled .so (Play requirement for Android 15+ targets).
fn verify_apk(project: &Project, apk: &Path) {
    let Some(build_tools) = latest_build_tools() else {
        status(
            "Warning",
            "apksigner not found (skipping verify) — install Android build-tools",
        );
        return;
    };
    let apksigner = build_tools.join(exe("apksigner"));
    let ok = Command::new(&apksigner)
        .args(["verify", "--print-certs"])
        .arg(apk)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        status("Verified", "apksigner verify passed");
    } else {
        status("Warning", "apksigner verify FAILED");
    }
    // 16 KB ELF alignment of the jniLibs (zipalign -c -P 16 checks pages of uncompressed .so).
    let zipalign = build_tools.join(exe("zipalign"));
    if zipalign.exists() {
        let ok = Command::new(&zipalign)
            .args(["-c", "-P", "16", "-v", "4"])
            .arg(apk)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            status(
                "Warning",
                "apk is not 16 KB page-aligned (Play requires it for Android 15+ targets; \
                 NDK r28+ aligns by default)",
            );
        }
    }
    let _ = project;
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.bat")
    } else {
        name.to_string()
    }
}

/// Newest installed build-tools dir under the Android SDK.
fn latest_build_tools() -> Option<PathBuf> {
    let dir = crate::mobile::android_sdk_dir().join("build-tools");
    let mut versions: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions.pop()
}
