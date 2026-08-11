// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android, whole: the Java that drives SharedPreferences, the declaration that binds it, and the
// mapping into this crate's API. Nothing about this platform appears anywhere else in the crate.
//
// SharedPreferences needs a `Context` and has no C entry point, so it is this crate's only foreign
// arm (docs/bridge.md). Written in Java rather than Kotlin so it compiles in any Android project.
// Values persist across launches like every other platform, and no manifest permission is required
// — this is app-private storage.

pub fn set(key: &str, value: &str) -> bool {
    set_native(key, value).unwrap_or(false)
}

pub fn get(key: &str) -> Option<String> {
    // `contains` is the presence check; the getter answers with an empty string for absent, because
    // `Option` does not cross a bridge (docs/bridge.md "Types"). A stored empty string reads back
    // as absent, which is why `contains` exists.
    match get_native(key) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

pub fn remove(key: &str) -> bool {
    remove_native(key).unwrap_or(false)
}

pub fn contains(key: &str) -> bool {
    contains_native(key).unwrap_or(false)
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Persist `value` under `key`; whether the commit succeeded.
        fn set_native(key: &str, value: &str) -> Result<bool, day_bridge::Error>;
        /// The string stored under `key`, or `""` when absent.
        fn get_native(key: &str) -> Result<String, day_bridge::Error>;
        /// Remove `key`; true only if it existed and the delete committed.
        fn remove_native(key: &str) -> Result<bool, day_bridge::Error>;
        /// Whether a value is currently stored under `key`.
        fn contains_native(key: &str) -> Result<bool, day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.content.SharedPreferences;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            /** App-private store name, separate from the app's own preferences. */
            private static final String STORE = "day_part_prefs";

            private static SharedPreferences prefs() {
                Context ctx = DayBridge.ctx;
                if (ctx == null) return null;
                return ctx.getSharedPreferences(STORE, Context.MODE_PRIVATE);
            }

            public static boolean set_native(String key, String value) {
                SharedPreferences p = prefs();
                if (p == null || key == null || value == null) return false;
                return p.edit().putString(key, value).commit();
            }

            public static String get_native(String key) {
                SharedPreferences p = prefs();
                if (p == null || key == null) return "";
                return p.getString(key, "");
            }

            public static boolean remove_native(String key) {
                SharedPreferences p = prefs();
                if (p == null || key == null || !p.contains(key)) return false;
                return p.edit().remove(key).commit();
            }

            public static boolean contains_native(String key) {
                SharedPreferences p = prefs();
                if (p == null || key == null) return false;
                return p.contains(key);
            }
        "#,
    );

    // The fallback every bridge declares. This file is `#[cfg(target_os = "android")]`, so it is
    // never compiled — it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn set_native(_key: &str, _value: &str) -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn get_native(_key: &str) -> Result<String, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn remove_native(_key: &str) -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn contains_native(_key: &str) -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
