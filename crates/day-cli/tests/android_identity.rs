// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Exercise the real launch/stop commands against a fake adb. The App Fair flavor has an
//! Apple id containing a hyphen and an Android override containing an underscore: building
//! resolved the override, but launching and dayscript cleanup used the general id.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn android_launch_and_stop_use_the_resolved_flavor_identity() {
    let fixture =
        Fixture(std::env::temp_dir().join(format!("day-material-identity-{}", std::process::id())));
    let root = &fixture.0;
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("sdk/platform-tools")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"identity-fixture\"\nversion = \"1.0.0\"\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Day.toml"),
        "schema = 1\n[app]\nid = \"dev.example.base\"\ntargets = [\"android-material\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Day-appfair.toml"),
        "[app]\nid = \"org.appfair.app.Faire-Games\"\n\
         [app.android]\nid = \"org.appfair.app.Faire_Games\"\n",
    )
    .unwrap();
    let adb = root.join("sdk/platform-tools/adb");
    std::fs::write(
        &adb,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$ADB_CALLS"
if [ "$1" = '-s' ]; then shift 2; fi
case "$*" in
  devices) printf 'List of devices attached\nidentity-device\tdevice\n' ;;
  'shell getprop ro.product.cpu.abi') echo x86_64 ;;
  'shell dumpsys window windows'*) echo '  Window #0 Window{123 u0 StatusBar}:' ;;
  'shell am start -n '*)
    if [ "$5" != "$EXPECTED_APP_ID/dev.daybrite.day.bridge.DayActivity" ]; then
      echo "Activity class $5 does not exist" >&2
      exit 1
    fi ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Synthetic signing tools: verify the launch path aligns, signs, and verifies a copy
    // before adb sees it. Real APK/certificate validation is covered by emulator testing.
    let build_tools = root.join("sdk/build-tools/35.0.0");
    std::fs::create_dir_all(&build_tools).unwrap();
    for (name, script) in [
        ("zipalign", "#!/bin/sh\ncp \"$3\" \"$4\"\n"),
        (
            "apksigner",
            r#"#!/bin/sh
for last do :; done
case "$1" in
  sign)
    [ "$DAY_SIGN_STORE_PASS" = android ] || exit 1
    [ "$DAY_SIGN_KEY_PASS" = android ] || exit 1
    [ "$4" = --ks-key-alias ] && [ "$5" = androiddebugkey ] || exit 1
    printf '\nsigned fixture\n' >> "$last" ;;
  verify)
    [ -z "$FAIL_VERIFY" ] || exit 1
    grep -q 'signed fixture' "$last" || exit 1
    printf 'Signer #1 certificate SHA-256 digest: a1b2\n' ;;
  *) exit 1 ;;
esac
"#,
        ),
    ] {
        let tool = build_tools.join(name);
        std::fs::write(&tool, script).unwrap();
        std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let apk = root.join("fixture.apk");
    std::fs::write(&apk, "fake APK: only adb is mocked").unwrap();

    // No override still uses the base identity; a flavor uses its Android-specific identity.
    for (flavor, expected, subtree) in [
        (None, "dev.example.base", "build/day"),
        (
            Some("appfair"),
            "org.appfair.app.Faire_Games",
            "build/day/flavors/appfair",
        ),
    ] {
        let artifacts = root.join(subtree).join("artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        std::fs::write(
            artifacts.join("android-material-debug.path"),
            apk.to_str().unwrap(),
        )
        .unwrap();
        let calls = root.join("adb-calls");
        std::fs::write(&calls, "").unwrap();
        for args in [
            vec![
                "launch",
                "-t",
                "android-material",
                "--skip-build",
                "--detach",
            ],
            vec!["stop", "-t", "android-material"],
        ] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_day"));
            cmd.arg("--project").arg(root);
            if let Some(flavor) = flavor {
                cmd.args(["--flavor", flavor]);
            }
            let output = cmd
                .args(&args)
                .env("ANDROID_HOME", root.join("sdk"))
                .env("ANDROID_SERIAL", "identity-device")
                .env("ADB_CALLS", &calls)
                .env("EXPECTED_APP_ID", expected)
                .env("DAY_NO_UPDATE_CHECK", "1")
                .env_remove("DAY_FLAVOR")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{args:?}, flavor={flavor:?}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        let calls = std::fs::read_to_string(calls).unwrap();
        assert!(calls.contains(&format!(
            "shell am start -n {expected}/dev.daybrite.day.bridge.DayActivity"
        )));
        assert_eq!(
            calls
                .matches(&format!("shell am force-stop {expected}\n"))
                .count(),
            2,
            "both launch cleanup and day stop must address the installed package: {calls}"
        );
    }

    let unsigned = root.join("app-release-unsigned.apk");
    let android_user = root.join("android-user");
    std::fs::create_dir_all(&android_user).unwrap();
    std::fs::write(android_user.join("debug.keystore"), "synthetic debug key").unwrap();
    let signed = root.join("day-launch/app-dev.apk");
    let signing_props = root.join("build/day/android/day-signing.properties");
    std::fs::create_dir_all(signing_props.parent().unwrap()).unwrap();
    std::fs::write(&signing_props, "synthetic signing config: leave untouched").unwrap();
    std::fs::write(
        root.join("build/day/artifacts/android-material-release.path"),
        unsigned.to_str().unwrap(),
    )
    .unwrap();
    // Reusing a build must re-sign the current input, not install a stale launch copy.
    // A verification failure must never reach adb, even with a previous good copy present.
    for (contents, fail_verify) in [
        ("first fixture", false),
        ("rebuilt fixture", false),
        ("bad fixture", true),
    ] {
        std::fs::write(&unsigned, contents).unwrap();
        let calls = root.join("adb-calls");
        std::fs::write(&calls, "").unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_day"));
        cmd.arg("--project")
            .arg(root)
            .args([
                "launch",
                "-t",
                "android-material",
                "--profile",
                "release",
                "--skip-build",
                "--detach",
            ])
            .env("ANDROID_HOME", root.join("sdk"))
            .env("ANDROID_USER_HOME", &android_user)
            .env("ANDROID_SERIAL", "identity-device")
            .env("ADB_CALLS", &calls)
            .env("EXPECTED_APP_ID", "dev.example.base")
            .env("DAY_NO_UPDATE_CHECK", "1")
            .env_remove("DAY_FLAVOR")
            .env_remove("FAIL_VERIFY");
        if fail_verify {
            cmd.env("FAIL_VERIFY", "1");
        }
        let output = cmd.output().unwrap();
        assert_eq!(
            output.status.success(),
            !fail_verify,
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = std::fs::read_to_string(calls).unwrap();
        if fail_verify {
            assert!(!calls.contains("install -r"), "{calls}");
        } else {
            assert!(
                calls.contains(&format!("install -r {}", signed.display())),
                "{calls}"
            );
            assert_eq!(
                std::fs::read_to_string(&signed).unwrap(),
                format!("{contents}\nsigned fixture\n")
            );
        }
        assert_eq!(std::fs::read_to_string(&unsigned).unwrap(), contents);
        assert_eq!(
            std::fs::read_to_string(&signing_props).unwrap(),
            "synthetic signing config: leave untouched"
        );
    }
}
