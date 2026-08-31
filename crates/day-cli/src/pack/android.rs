// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! android-mdc → release .apk + .aab. Day.toml identity/version is conveyed to Gradle via a
//! generated properties file (§17.5); the release signingConfig reads a second generated file —
//! resolved from `signing.android` `${ENV}` refs, or the fixed dev keystore embedded in the CLI
//! when unconfigured (dev tier, loud — fixed rather than generated so dev builds reproduce and
//! upgrade across machines). Gradle signs both formats (apksigner cannot sign an .aab — §16.5).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::{PackOptions, resolve_degradable};
use super::{Artifact, PackError, SignTier, run_tool};
use crate::cli::Profile;
use crate::meta::Project;
use crate::ops::{self, status};
use crate::targets::Target;

const DEV_KEYSTORE_PASS: &str = "day-dev-only"; // dev keystore: local installs only, never distribution

/// The Gradle to run for an app: its own `./gradlew` when it carries one, else `gradle` from PATH.
///
/// A wrapper pins the Gradle version inside the project (`gradle/wrapper/gradle-wrapper.properties`),
/// and that pin is what an IDE already obeys. Preferring the wrapper here is what makes `day build`
/// and Android Studio compile the app with the SAME Gradle, instead of each using whichever one it
/// happens to find — a difference that shows up as a build that works in one and not the other.
///
/// Returned as an ABSOLUTE path on purpose. `Command`'s program lookup is not consistently relative
/// to `current_dir` across platforms — on Unix the child chdirs before `exec`, on Windows the
/// program is resolved in the parent's directory — so a literal `./gradlew` would silently mean
/// two different files. Joining it onto the directory removes the question.
///
/// Presence alone decides. A `gradlew` without its executable bit fails with a permission error
/// naming the file, which is a better answer than quietly building with a different Gradle than
/// the one the project asked for.
pub(crate) fn gradle_program(android_dir: &Path) -> PathBuf {
    let wrapper = android_dir.join(if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    });
    if wrapper.is_file() {
        wrapper
    } else {
        PathBuf::from("gradle")
    }
}

/// Day.toml → `build/day/android/day-app.properties` (applicationId, versionCode, versionName,
/// title). Written on every android build (`day build` too) so the Gradle scaffold never goes
/// stale (§17.5). Identity is RESOLVED for the android target, so `[app.android]` /
/// `[app.android-mdc]` overrides in Day.toml flow into the APK.
///
/// The window block rides here too, as manifest placeholders for the activity's `<layout>` element
/// (docs/size-classes.md). Those four numbers are what multi-window and desktop windowing read to
/// decide how small the window may go and how big it opens, and a manifest is a BUILD-time
/// declaration — there is no runtime call that sets them, which is why they cannot ride
/// `WindowOptions` the way the iOS minimum does.
pub(crate) fn write_app_properties(project: &Project) -> Result<(), String> {
    let dir = project.root.join("build/day/android");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let resolved = project.manifest.resolve("android-mdc");
    let win = &project.manifest.window;
    let content = format!(
        "applicationId={}\nnamespace={}\nversionCode={}\nversionName={}\ntitle={}\nscheme={}\n\
         windowWidth={}\nwindowHeight={}\nwindowMinWidth={}\nwindowMinHeight={}\n",
        resolved.id,
        resolved.id,
        resolved.build.min(i32::MAX as u64),
        resolved.version,
        resolved.title,
        resolved.scheme(),
        win.width.round() as i64,
        win.height.round() as i64,
        win.min_width.round() as i64,
        win.min_height.round() as i64,
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
    let outcome = ops::build(project, target, opts.profile).map_err(PackError::Other)?;

    let mut artifacts = Vec::new();

    if formats.iter().any(|f| f == "apk") {
        let apk = find_output(&outcome.artifact, project, opts.profile, "apk")?;
        verify_apk(project, &apk);
        let out = dist.join(super::naming::artifact_file(
            project,
            target,
            opts,
            &[],
            "apk",
        ));
        std::fs::copy(&apk, &out).map_err(|e| PackError::Other(e.to_string()))?;
        artifacts.push(Artifact {
            path: out,
            kind: "apk",
            sha256: String::new(),
            tier,
        });
    }

    if formats.iter().any(|f| f == "aab") && opts.profile == Profile::Release {
        status("Building", "android-mdc (gradle bundleRelease)");
        let day_bin = std::env::current_exe().map_err(|e| PackError::Other(e.to_string()))?;
        let android_dir = project.root.join("platform/android");
        let mut cmd = Command::new(gradle_program(&android_dir));
        cmd.current_dir(&android_dir)
            .env("DAY_BIN", &day_bin)
            .env("DAY_PROJECT_ROOT", &project.root)
            .env("DAY_PROFILE", opts.profile.as_str())
            .args(["bundleRelease", "-q", "--console=plain"]);
        if std::env::var_os("JAVA_HOME").is_none()
            && let Some(jdk) = day_toolchain::jdk_home()
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
        let out = dist.join(super::naming::artifact_file(
            project,
            target,
            opts,
            &[],
            "aab",
        ));
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

/// The shared dev keystore, written out under `build/day/` on first use.
///
/// FIXED and embedded rather than generated per project, which is what Android's own
/// `debug.keystore` does and for the same two reasons. A freshly minted key each time meant a dev
/// `.apk` could never be byte-reproducible — two CI jobs signed the same bytes with different keys,
/// which is exactly what the container tier kept reporting (§20.3) — and it meant a build from one
/// machine could not upgrade an install from another, because Android refuses an update whose
/// signature changed.
///
/// This key is deliberately public and carries no secret: `day pack` warns loudly whenever it is
/// used, the tier is recorded as dev-signed on the artifact, and distribution requires configuring
/// `signing.android` with a real keystore.
fn dev_keystore(project: &Project) -> Result<PathBuf, String> {
    const EMBEDDED: &[u8] = include_bytes!("../../resources/day-dev.keystore");
    let path = project.root.join("build/day/android/day-dev.keystore");
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    status("Signing", "staging the shared dev keystore (dev tier)");
    std::fs::write(&path, EMBEDDED).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// The built apk: ops::build returns the conventional path, but an unsigned release build is named
/// `app-release-unsigned.apk` — fall back to any .apk in the outputs dir.
fn find_output(
    conventional: &Path,
    project: &Project,
    profile: Profile,
    ext: &str,
) -> Result<PathBuf, PackError> {
    if conventional.exists() {
        return Ok(conventional.to_path_buf());
    }
    let dir = project
        .root
        .join("platform/android/app/build/outputs/apk")
        .join(profile.as_str());
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

#[cfg(test)]
mod gradle_tests {
    use super::gradle_program;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("day-gradlew-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir"); // test-only setup
        dir
    }

    /// Without a wrapper, PATH's `gradle` — the behavior every project had before wrappers were
    /// consulted at all.
    #[test]
    fn no_wrapper_means_path_gradle() {
        let dir = scratch("bare");
        assert_eq!(gradle_program(&dir), Path::new("gradle"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A wrapper wins, and comes back ABSOLUTE. A relative `./gradlew` would be resolved against
    /// day's own working directory on Windows rather than the project's, so the two platforms would
    /// run different files from identical code.
    #[test]
    fn a_wrapper_wins_and_is_absolute() {
        let dir = scratch("wrapped");
        let name = if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        };
        std::fs::write(dir.join(name), "#!/bin/sh\n").expect("write wrapper"); // test-only setup
        let found = gradle_program(&dir);
        assert_eq!(found, dir.join(name));
        assert!(found.is_absolute(), "{} must be absolute", found.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The wrapper is a FILE. A directory of that name is not a launcher, and treating it as one
    /// would replace a working PATH build with a permission error.
    #[test]
    fn a_directory_named_gradlew_is_not_a_wrapper() {
        let dir = scratch("dir");
        let name = if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        };
        std::fs::create_dir_all(dir.join(name)).expect("make dir"); // test-only setup
        assert_eq!(gradle_program(&dir), Path::new("gradle"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The OTHER platform's wrapper name is not this platform's launcher: a `gradlew.bat` beside a
    /// Unix build is not runnable there, and vice versa.
    #[test]
    fn the_other_platforms_wrapper_is_ignored() {
        let dir = scratch("crossname");
        let other = if cfg!(windows) {
            "gradlew"
        } else {
            "gradlew.bat"
        };
        std::fs::write(dir.join(other), "").expect("write wrapper"); // test-only setup
        assert_eq!(gradle_program(&dir), Path::new("gradle"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
