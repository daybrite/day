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
        Fixture(std::env::temp_dir().join(format!("day-android-identity-{}", std::process::id())));
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
        "schema = 1\n[app]\nid = \"dev.example.base\"\ntargets = [\"android-mdc\"]\n",
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
            artifacts.join("android-mdc-debug.path"),
            apk.to_str().unwrap(),
        )
        .unwrap();
        let calls = root.join("adb-calls");
        std::fs::write(&calls, "").unwrap();
        for args in [
            vec!["launch", "-p", "android-mdc", "--skip-build", "--detach"],
            vec!["stop", "-p", "android-mdc"],
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
}
