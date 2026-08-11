// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android, whole: the Java that reads `android.os.Build`, the declaration that binds it, and the
// mapping into [`DeviceInfo`]. Nothing about this platform appears anywhere else in the crate.
//
// `Build`'s fields are static (no Context needed) but they are Java constants with no C entry
// point, so this is the crate's only foreign arm (docs/bridge.md). Written in Java rather than
// Kotlin so it compiles in any Android project. No permission is needed.
//
// Before daybridge the four fields crossed as ONE string joined by U+001F, packed in Java and split
// in Rust — a wire format written twice. Each field is now its own declaration, which is three JNI
// calls instead of one against static data read once per launch.

use super::DeviceInfo;

pub fn get() -> DeviceInfo {
    DeviceInfo {
        model: model_native().unwrap_or_else(|_| "Unknown".into()),
        // Not asked of the platform: on Android the answer is Android.
        system_name: "Android".into(),
        system_version: system_version_native().unwrap_or_else(|_| "Unknown".into()),
        is_simulator: is_emulator_native().unwrap_or(false),
    }
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// `MODEL`, prefixed with `MANUFACTURER` when it does not already start with it.
        fn model_native() -> Result<String, day_bridge::Error>;
        /// `VERSION.RELEASE`, or `"Unknown"`.
        fn system_version_native() -> Result<String, day_bridge::Error>;
        /// Whether this build looks like the AOSP/Google emulator.
        fn is_emulator_native() -> Result<bool, day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.os.Build;
        "#,
        body = r#"
            public static String model_native() {
                String model = nonEmpty(Build.MODEL);
                String manufacturer = Build.MANUFACTURER;
                if (manufacturer != null && !manufacturer.isEmpty()
                        && !model.toLowerCase().startsWith(manufacturer.toLowerCase())
                        && !model.equals("Unknown")) {
                    return manufacturer + " " + model;
                }
                return model;
            }

            public static String system_version_native() {
                return nonEmpty(Build.VERSION.RELEASE);
            }

            // Heuristic emulator detection from the standard AOSP/Google build fingerprints.
            public static boolean is_emulator_native() {
                String fingerprint = lower(Build.FINGERPRINT);
                String product = lower(Build.PRODUCT);
                String model = lower(Build.MODEL);
                String hardware = lower(Build.HARDWARE);
                return fingerprint.contains("generic")
                        || fingerprint.contains("emulator")
                        || product.contains("sdk")
                        || product.contains("emulator")
                        || model.contains("emulator")
                        || model.contains("android sdk")
                        || hardware.contains("goldfish")
                        || hardware.contains("ranchu");
            }

            private static String nonEmpty(String s) {
                return (s == null || s.isEmpty()) ? "Unknown" : s;
            }

            private static String lower(String s) {
                return s == null ? "" : s.toLowerCase();
            }
        "#,
    );

    // The fallback every bridge declares. This file is `#[cfg(target_os = "android")]`, so it is
    // never compiled — it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn model_native() -> Result<String, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn system_version_native() -> Result<String, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn is_emulator_native() -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
