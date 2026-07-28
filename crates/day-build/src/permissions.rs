//! The portable-permission → native-declaration table (docs/permissions.md).
//!
//! Two consumers need exactly this knowledge and must never disagree about it:
//!
//! - `day-cli` turns a `[permissions]` table in `Day.toml` into `<uses-permission>` entries, iOS and
//!   macOS `Info.plist` usage-description keys, and HarmonyOS `module.json5` `requestPermissions`.
//! - `day-part-permissions` asks the OS about the same permissions at runtime.
//!
//! It lives here rather than in a new crate because `day-build` is already published, already a
//! `day-cli` dependency, and already carries the tree's other CLI-and-runtime shared mapping (the
//! resource name → identifier table), for the same reason: a generated declaration must never drift
//! from the constant the app's code names.
//!
//! Two rows break any naive version of this table, so they are worth stating up front:
//! **notifications** needs an Android permission but NO iOS/macOS plist key and no HarmonyOS entry,
//! and **photos** needs three Android permissions, one of them version-capped.

/// An Android permission id, plus the API level after which it must not be requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AndroidPermission {
    pub name: &'static str,
    /// Emitted as `android:maxSdkVersion`. Only the legacy storage permission needs it: from API 33
    /// the granular `READ_MEDIA_*` permissions replace it, and leaving it uncapped makes stores flag
    /// the app for requesting broad storage access it no longer uses.
    pub max_sdk: Option<u32>,
}

/// When a HarmonyOS permission is used, which its `module.json5` entry must declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OhosScene {
    InUse,
    Always,
}

impl OhosScene {
    pub fn as_str(self) -> &'static str {
        match self {
            OhosScene::InUse => "inuse",
            OhosScene::Always => "always",
        }
    }
}

/// A HarmonyOS permission name and the scene it is requested for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OhosPermission {
    pub name: &'static str,
    pub when: OhosScene,
}

/// One portable permission, and everything each platform needs declared for it.
#[derive(Clone, Copy, Debug)]
pub struct PermissionSpec {
    /// The kebab-case name written in `Day.toml`'s `[permissions]` table.
    pub name: &'static str,
    /// The `day_part_permissions::Permission` variant spelling, so `day lint` can map a source
    /// reference back to a declaration.
    pub variant: &'static str,
    pub android: &'static [AndroidPermission],
    pub ios: &'static [&'static str],
    pub macos: &'static [&'static str],
    pub ohos: &'static [OhosPermission],
    /// Whether a user-facing reason is required. False only for notifications, which no platform
    /// asks a reason for.
    pub needs_reason: bool,
}

const fn android(name: &'static str) -> AndroidPermission {
    AndroidPermission {
        name,
        max_sdk: None,
    }
}

const fn ohos(name: &'static str) -> OhosPermission {
    OhosPermission {
        name,
        when: OhosScene::InUse,
    }
}

/// Every portable permission Day knows how to declare.
pub const ALL: &[PermissionSpec] = &[
    PermissionSpec {
        name: "location-when-in-use",
        variant: "Location",
        android: &[
            android("android.permission.ACCESS_FINE_LOCATION"),
            android("android.permission.ACCESS_COARSE_LOCATION"),
        ],
        ios: &["NSLocationWhenInUseUsageDescription"],
        // macOS also wants the legacy key: some frameworks still read it.
        macos: &[
            "NSLocationWhenInUseUsageDescription",
            "NSLocationUsageDescription",
        ],
        ohos: &[
            ohos("ohos.permission.APPROXIMATELY_LOCATION"),
            ohos("ohos.permission.LOCATION"),
        ],
        needs_reason: true,
    },
    PermissionSpec {
        name: "location-always",
        variant: "LocationAlways",
        android: &[
            android("android.permission.ACCESS_FINE_LOCATION"),
            android("android.permission.ACCESS_COARSE_LOCATION"),
            android("android.permission.ACCESS_BACKGROUND_LOCATION"),
        ],
        // Apple requires BOTH keys: a plist carrying only the Always key suppresses the prompt.
        ios: &[
            "NSLocationAlwaysAndWhenInUseUsageDescription",
            "NSLocationWhenInUseUsageDescription",
        ],
        macos: &[
            "NSLocationAlwaysAndWhenInUseUsageDescription",
            "NSLocationWhenInUseUsageDescription",
            "NSLocationUsageDescription",
        ],
        ohos: &[
            ohos("ohos.permission.APPROXIMATELY_LOCATION"),
            ohos("ohos.permission.LOCATION"),
            OhosPermission {
                name: "ohos.permission.LOCATION_IN_BACKGROUND",
                when: OhosScene::Always,
            },
        ],
        needs_reason: true,
    },
    PermissionSpec {
        name: "camera",
        variant: "Camera",
        android: &[android("android.permission.CAMERA")],
        ios: &["NSCameraUsageDescription"],
        macos: &["NSCameraUsageDescription"],
        ohos: &[ohos("ohos.permission.CAMERA")],
        needs_reason: true,
    },
    PermissionSpec {
        name: "microphone",
        variant: "Microphone",
        android: &[android("android.permission.RECORD_AUDIO")],
        ios: &["NSMicrophoneUsageDescription"],
        macos: &["NSMicrophoneUsageDescription"],
        ohos: &[ohos("ohos.permission.MICROPHONE")],
        needs_reason: true,
    },
    PermissionSpec {
        name: "notifications",
        variant: "Notifications",
        // Ignored below API 33, so it is declared unconditionally.
        android: &[android("android.permission.POST_NOTIFICATIONS")],
        // Apple asks for notification permission at runtime with no plist key, and HarmonyOS gates
        // it through a runtime `requestEnableNotification` call rather than the manifest.
        ios: &[],
        macos: &[],
        ohos: &[],
        needs_reason: false,
    },
    PermissionSpec {
        name: "photos",
        variant: "Photos",
        android: &[
            AndroidPermission {
                name: "android.permission.READ_MEDIA_IMAGES",
                max_sdk: None,
            },
            AndroidPermission {
                name: "android.permission.READ_MEDIA_VIDEO",
                max_sdk: None,
            },
            AndroidPermission {
                name: "android.permission.READ_EXTERNAL_STORAGE",
                max_sdk: Some(32),
            },
        ],
        ios: &["NSPhotoLibraryUsageDescription"],
        macos: &["NSPhotoLibraryUsageDescription"],
        // NOTE: `READ_IMAGEVIDEO` is `system_basic` apl, which an app signed at `normal` cannot
        // hold — see OHOS_PHOTOS_APL_NOTE. The picker needs no permission at all.
        ohos: &[ohos("ohos.permission.READ_IMAGEVIDEO")],
        needs_reason: true,
    },
    PermissionSpec {
        name: "motion",
        variant: "Motion",
        android: &[android("android.permission.ACTIVITY_RECOGNITION")],
        ios: &["NSMotionUsageDescription"],
        // CoreMotion's activity APIs are iOS-only; macOS has nothing to declare.
        macos: &[],
        ohos: &[ohos("ohos.permission.ACTIVITY_MOTION")],
        needs_reason: true,
    },
];

/// The warning both the CLI and the docs use for HarmonyOS photo access, kept in one place so they
/// cannot drift.
pub const OHOS_PHOTOS_APL_NOTE: &str = "ohos.permission.READ_IMAGEVIDEO is a system_basic permission, which an app signed at the \
     default `normal` level cannot be granted. Prefer PhotoViewPicker, which needs no permission.";

/// Look a permission up by its `Day.toml` name.
pub fn find(name: &str) -> Option<&'static PermissionSpec> {
    ALL.iter().find(|s| s.name == name)
}

/// Look a permission up by its Rust variant spelling (`day lint`'s source scan).
pub fn find_variant(variant: &str) -> Option<&'static PermissionSpec> {
    ALL.iter().find(|s| s.variant == variant)
}

/// Every valid `[permissions]` key, for error messages.
pub fn names() -> Vec<&'static str> {
    ALL.iter().map(|s| s.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_kebab_and_round_trip() {
        let mut seen = std::collections::BTreeSet::new();
        for spec in ALL {
            assert!(seen.insert(spec.name), "duplicate name {}", spec.name);
            assert!(
                spec.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} is not kebab-case",
                spec.name
            );
            assert_eq!(find(spec.name).map(|s| s.variant), Some(spec.variant));
            assert_eq!(find_variant(spec.variant).map(|s| s.name), Some(spec.name));
        }
        assert_eq!(names().len(), ALL.len());
        assert!(find("nonsense").is_none());
    }

    /// A plist carrying only `NSLocationAlwaysAndWhenInUseUsageDescription` suppresses the prompt —
    /// Apple requires the when-in-use key alongside it.
    #[test]
    fn location_always_also_declares_when_in_use() {
        let spec = find("location-always").expect("location-always");
        assert!(spec.ios.contains(&"NSLocationWhenInUseUsageDescription"));
        assert!(spec.macos.contains(&"NSLocationWhenInUseUsageDescription"));
    }

    /// The row a naive table gets wrong in the other direction: an Android permission, but nothing
    /// to declare on Apple or HarmonyOS, and no reason anywhere.
    #[test]
    fn notifications_declares_android_only() {
        let spec = find("notifications").expect("notifications");
        assert_eq!(spec.android.len(), 1);
        assert!(spec.ios.is_empty() && spec.macos.is_empty() && spec.ohos.is_empty());
        assert!(!spec.needs_reason);
    }

    /// Legacy storage must be capped at 32 or stores flag the app for over-broad access.
    #[test]
    fn photos_caps_legacy_storage() {
        let spec = find("photos").expect("photos");
        let legacy = spec
            .android
            .iter()
            .find(|p| p.name.ends_with("READ_EXTERNAL_STORAGE"))
            .expect("legacy storage permission");
        assert_eq!(legacy.max_sdk, Some(32));
        // The granular replacements must NOT be capped.
        for p in spec
            .android
            .iter()
            .filter(|p| p.name.contains("READ_MEDIA"))
        {
            assert_eq!(p.max_sdk, None, "{} should not be capped", p.name);
        }
    }

    /// Every permission that needs a reason must have somewhere to put one.
    #[test]
    fn reasons_have_a_destination() {
        for spec in ALL {
            if spec.needs_reason {
                assert!(
                    !spec.ios.is_empty() || !spec.ohos.is_empty(),
                    "{} needs a reason but no platform consumes it",
                    spec.name
                );
            } else {
                assert!(
                    spec.ios.is_empty() && spec.macos.is_empty() && spec.ohos.is_empty(),
                    "{} needs no reason, so it must declare no reason-carrying key",
                    spec.name
                );
            }
        }
    }

    /// macOS has no CoreMotion activity API, so it has nothing to declare for motion.
    #[test]
    fn motion_is_ios_only_on_apple() {
        let spec = find("motion").expect("motion");
        assert!(!spec.ios.is_empty());
        assert!(spec.macos.is_empty());
    }
}
