//! day-part-prefs — a HEADLESS cross-platform persistent key/value store. No UI; any Rust code can
//! depend on this crate and call [`set`] / [`get`] / [`remove`] / [`contains`] to persist small
//! strings across launches through the platform's NATIVE preferences facility.
//!
//! ```no_run
//! day_part_prefs::set("greeting", "hello");
//! assert_eq!(day_part_prefs::get("greeting").as_deref(), Some("hello"));
//! assert!(day_part_prefs::contains("greeting"));
//! day_part_prefs::remove("greeting");
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (persistence is an OS
//! concern, not a widget-toolkit one): macOS and iOS share one `NSUserDefaults` file, Android uses
//! `SharedPreferences` (via a Java shim staged by `day build`), and Linux / Windows / HarmonyOS
//! share a file-backed store under the per-user config directory. Values persist until removed and
//! survive process restarts. Platforms without any store fall back to a no-op that always reports
//! failure/absence.
//!
//! This is a small **string** store for user settings and lightweight app state — not a database.
//! Keep values modest; large blobs belong in a file. See docs/prefs.md for the per-platform matrix.

/// Persist `value` under `key`, overwriting any previous value. Returns `true` when the write was
/// committed. On Apple platforms this always succeeds; on Android it reflects
/// `SharedPreferences.Editor.commit()`; on the file-backed platforms it reflects whether the store
/// file could be written (a missing config directory or a read-only home yields `false`).
pub fn set(key: &str, value: &str) -> bool {
    imp::set(key, value)
}

/// Read the string stored under `key`, or `None` if it is absent (or no store is available on the
/// platform). A stored empty string is `Some("")`, distinct from an absent key.
pub fn get(key: &str) -> Option<String> {
    imp::get(key)
}

/// Delete the value stored under `key`. Returns `true` if a value existed and was removed, `false`
/// if the key was already absent (or the delete could not be committed).
pub fn remove(key: &str) -> bool {
    imp::remove(key)
}

/// Whether a value is currently stored under `key`.
pub fn contains(key: &str) -> bool {
    imp::contains(key)
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes:
//   fn set(&str, &str) -> bool
//   fn get(&str) -> Option<String>
//   fn remove(&str) -> bool
//   fn contains(&str) -> bool
// ---------------------------------------------------------------------------

// macOS + iOS share one NSUserDefaults impl.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

// Android rides day-android's JVM/Context to reach SharedPreferences via a bundled Java shim.
#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Linux, Windows, and HarmonyOS (also `target_os = "linux"`, with `target_env = "ohos"`) all use the
// same file-backed store; file.rs resolves the per-OS config directory internally.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[path = "file.rs"]
mod imp;

// Any other platform: no persistent store.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_os = "linux",
    target_os = "windows"
)))]
mod imp {
    pub fn set(_key: &str, _value: &str) -> bool {
        false
    }
    pub fn get(_key: &str) -> Option<String> {
        None
    }
    pub fn remove(_key: &str) -> bool {
        false
    }
    pub fn contains(_key: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    // A full round-trip on platforms with a usable store in a plain test process (Apple
    // NSUserDefaults / the desktop file store). Android and iOS need a device runtime + Context, so
    // they are excluded here. The values deliberately contain `=` and a newline to exercise the
    // file store's escaping.
    #[cfg(any(
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "windows"
    ))]
    #[test]
    fn round_trip() {
        let key = "day-part-prefs::test::round_trip";
        // Start from a clean slate regardless of a prior aborted run.
        super::remove(key);
        assert!(!super::contains(key));
        assert_eq!(super::get(key), None);

        assert!(super::set(key, "hello=day\nsecond line"));
        assert!(super::contains(key));
        assert_eq!(super::get(key).as_deref(), Some("hello=day\nsecond line"));

        // Overwrite.
        assert!(super::set(key, "again"));
        assert_eq!(super::get(key).as_deref(), Some("again"));

        // Remove, then removing again reports "was already absent".
        assert!(super::remove(key));
        assert!(!super::contains(key));
        assert_eq!(super::get(key), None);
        assert!(!super::remove(key));
    }

    // Reading or probing a missing key must never panic on any platform.
    #[test]
    fn missing_key_does_not_panic() {
        let absent = "day-part-prefs::test::definitely-absent-key";
        let _ = super::get(absent);
        let _ = super::contains(absent);
    }
}
