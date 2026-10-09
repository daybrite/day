// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day launch --grant <permission>`: mark a portable permission as granted on the device before
//! the app starts, so a scripted run gets past the OS consent prompt (docs/permissions.md,
//! "Testing past the prompt").
//!
//! A dayscript drives Day's own views and nothing else; the permission dialog is the system's
//! window, out of its reach, so a walkthrough that needs the camera or the microphone stalls at
//! the first `request`. Every mobile platform that can be told from outside has a tool for it:
//! `pm grant` on Android, `simctl privacy grant` on the iOS Simulator. This module is the
//! portable name's translation into those, resolved against the same table the declaration
//! pipeline writes from (`day_build::permissions`), so the grant names exactly the permissions
//! the built app declares.
//!
//! What cannot be granted is refused BEFORE the build rather than warned about after it: a
//! grant the script depends on that silently did not happen would surface minutes later as a
//! failed assertion with nothing to say about why.

use day_build::permissions::PermissionSpec;

use crate::targets::{Target, TargetKind};

/// Resolve the `--grant` values (each a portable name, or a comma- or space-separated list of
/// them) against the permission table, in first-seen order and without duplicates.
pub fn parse(flags: &[String]) -> Result<Vec<&'static PermissionSpec>, String> {
    let mut out: Vec<&'static PermissionSpec> = Vec::new();
    for name in flags
        .iter()
        .flat_map(|flag| flag.split([',', ' ']))
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let Some(spec) = day_build::permissions::find(name) else {
            return Err(format!(
                "--grant {name:?} is not a portable permission (valid: {})",
                day_build::permissions::names().join(", ")
            ));
        };
        if !out.iter().any(|known| known.name == spec.name) {
            out.push(spec);
        }
    }
    Ok(out)
}

/// The `simctl privacy` service a portable permission maps to, when the simulator has one.
///
/// The simulator gates fewer things than a device: it has no camera at all (so no camera
/// service either), and notifications and the privacy-window flag are not TCC services.
pub fn simctl_service(spec: &PermissionSpec) -> Option<&'static str> {
    match spec.name {
        "location-when-in-use" => Some("location"),
        "location-always" => Some("location-always"),
        "microphone" => Some("microphone"),
        "photos" => Some("photos"),
        "motion" => Some("motion"),
        _ => None,
    }
}

/// The Android runtime permissions `pm grant` marks for `spec` on a device at API level `sdk`.
///
/// An entry with a `maxSdkVersion` below the device's level is not in the installed app's
/// manifest at all (`READ_EXTERNAL_STORAGE` past API 32), and `pm grant` refuses a permission
/// the package never requested, so those are left out rather than failing the launch.
pub fn android_permissions(spec: &PermissionSpec, sdk: Option<u32>) -> Vec<&'static str> {
    spec.android
        .iter()
        .filter(|p| match (p.max_sdk, sdk) {
            (Some(max), Some(level)) => level <= max,
            _ => true,
        })
        .map(|p| p.name)
        .collect()
}

/// Whether every grant can be carried out on `target`, decided before anything is built.
///
/// `physical_ios` says the iOS launch goes to a device rather than the simulator (the spec's
/// `--ios-device`), which has no grant tool at all.
pub fn check_target(
    target: &Target,
    grants: &[&PermissionSpec],
    physical_ios: bool,
) -> Result<(), String> {
    if grants.is_empty() {
        return Ok(());
    }
    let names = || grants.iter().map(|g| g.name).collect::<Vec<_>>().join(", ");
    match target.kind {
        TargetKind::Android => {
            for g in grants {
                if g.android.is_empty() {
                    return Err(format!(
                        "--grant {}: nothing gates it on Android, so there is no runtime \
                         permission to grant",
                        g.name
                    ));
                }
            }
            Ok(())
        }
        TargetKind::IosSim if physical_ios => Err(format!(
            "--grant {} cannot reach a physical iOS device: the consent record is the device's \
             own, and no tool sets it from outside. Grant it in Settings on the device, or run \
             the script on the simulator",
            names()
        )),
        TargetKind::IosSim => {
            for g in grants {
                if simctl_service(g).is_none() {
                    return Err(match g.name {
                        "camera" => "--grant camera: the iOS Simulator has no camera and `simctl \
                                     privacy` no camera service. Run the script on Android, \
                                     whose emulator has a virtual camera, or on a device with \
                                     the permission already allowed"
                            .to_string(),
                        _ => format!(
                            "--grant {}: `simctl privacy` has no service for it on the iOS \
                             Simulator",
                            g.name
                        ),
                    });
                }
            }
            Ok(())
        }
        TargetKind::HarmonyOs => Err(format!(
            "--grant {}: HarmonyOS keeps its consent records inside the system and ships no tool \
             that marks a user_grant permission from outside the app; a script there stops at \
             the prompt",
            names()
        )),
        TargetKind::Web => Err(format!(
            "--grant {}: the browser's permission policy is not set from outside the page; \
             scripted web runs keep the driver's own grants (docs/web.md)",
            names()
        )),
        TargetKind::Desktop if target.os == "macos" => Err(format!(
            "--grant {}: macOS keeps consent in TCC, which `tccutil` can only reset, never \
             grant; allow it once by hand in System Settings › Privacy & Security",
            names()
        )),
        // Linux and Windows gate none of these (`Gate::Ungated`): the capability is simply there.
        TargetKind::Desktop => Ok(()),
    }
}

/// What the launch reports for an ungated desktop target, so a `--grant` that did nothing says
/// so rather than passing in silence.
pub fn ungated_note(target: &Target, grants: &[&PermissionSpec]) -> Option<String> {
    if grants.is_empty() || target.kind != TargetKind::Desktop {
        return None;
    }
    Some(format!(
        "{} needs no grant on {}: nothing gates it there",
        grants.iter().map(|g| g.name).collect::<Vec<_>>().join(", "),
        target.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &'static str) -> &'static Target {
        crate::targets::find(name).expect("builtin target")
    }

    #[test]
    fn parse_accepts_lists_and_dedups() {
        let specs = parse(&["camera".into(), "microphone, camera photos".into()]).unwrap();
        let names: Vec<_> = specs.iter().map(|s| s.name).collect();
        assert_eq!(names, ["camera", "microphone", "photos"]);
        assert!(parse(&[]).unwrap().is_empty());
    }

    #[test]
    fn parse_rejects_unknown_names_listing_the_valid_ones() {
        let err = parse(&["kamera".into()]).unwrap_err();
        assert!(err.contains("\"kamera\""), "{err}");
        assert!(err.contains("camera"), "{err}");
    }

    #[test]
    fn simulator_services_cover_what_tcc_gates_there() {
        let service = |n: &str| simctl_service(day_build::permissions::find(n).unwrap());
        assert_eq!(service("location-when-in-use"), Some("location"));
        assert_eq!(service("location-always"), Some("location-always"));
        assert_eq!(service("microphone"), Some("microphone"));
        assert_eq!(service("photos"), Some("photos"));
        assert_eq!(service("motion"), Some("motion"));
        assert_eq!(service("camera"), None);
        assert_eq!(service("notifications"), None);
    }

    #[test]
    fn android_permissions_honor_the_max_sdk_cap() {
        let photos = day_build::permissions::find("photos").unwrap();
        assert!(
            android_permissions(photos, Some(36))
                .iter()
                .all(|p| !p.ends_with("EXTERNAL_STORAGE"))
        );
        assert!(
            android_permissions(photos, Some(32))
                .contains(&"android.permission.READ_EXTERNAL_STORAGE")
        );
        // An unknown level keeps everything: `pm grant`'s own error is then the diagnostic.
        assert_eq!(android_permissions(photos, None).len(), 3);
        let camera = day_build::permissions::find("camera").unwrap();
        assert_eq!(
            android_permissions(camera, Some(36)),
            ["android.permission.CAMERA"]
        );
    }

    #[test]
    fn targets_that_cannot_grant_are_refused_up_front() {
        let camera = parse(&["camera".into()]).unwrap();
        let mic = parse(&["microphone".into()]).unwrap();
        assert!(check_target(target("android-mdc"), &camera, false).is_ok());
        assert!(check_target(target("ios-uikit"), &mic, false).is_ok());
        assert!(
            check_target(target("ios-uikit"), &camera, false)
                .unwrap_err()
                .contains("no camera")
        );
        assert!(
            check_target(target("ios-uikit"), &mic, true)
                .unwrap_err()
                .contains("physical")
        );
        assert!(check_target(target("harmony-arkui"), &camera, false).is_err());
        assert!(check_target(target("web-dom"), &camera, false).is_err());
        assert!(
            check_target(target("macos-appkit"), &camera, false)
                .unwrap_err()
                .contains("TCC")
        );
        assert!(check_target(target("linux-gtk"), &camera, false).is_ok());
        assert!(check_target(target("linux-gtk"), &[], false).is_ok());
        let privacy = parse(&["screen-privacy".into()]).unwrap();
        assert!(check_target(target("android-mdc"), &privacy, false).is_err());
    }

    #[test]
    fn ungated_note_names_the_desktop_only() {
        let camera = parse(&["camera".into()]).unwrap();
        assert!(
            ungated_note(target("linux-gtk"), &camera)
                .unwrap()
                .contains("linux-gtk")
        );
        assert!(ungated_note(target("android-mdc"), &camera).is_none());
        assert!(ungated_note(target("linux-gtk"), &[]).is_none());
    }
}
