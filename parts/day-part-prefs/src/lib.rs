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

/// Two-way-bind a signal to a stored preference: seed the signal from the store now (when a
/// stored value exists and parses as `T`), then persist every later change. Call it right
/// after creating the signal, before anything reads it. The write-back lives in the current
/// reactive scope, so it stops with the page that created the signal.
///
/// ```no_run
/// let count = day_reactive::Signal::new(0i64);
/// day_part_prefs::bind("controls.count", count);
/// ```
pub fn bind<T>(key: &str, signal: day_reactive::Signal<T>)
where
    T: std::str::FromStr + ToString + Clone + 'static,
{
    if let Some(stored) = get(key)
        && let Ok(v) = stored.parse::<T>()
    {
        signal.set(v);
    }
    let key = key.to_string();
    day_reactive::watch(
        move || signal.get(),
        move |v, _| {
            let _ = set(&key, &v.to_string());
        },
    );
}

/// Install this prefs store as the app's navigation-persistence store (docs/navigation.md), so a
/// [`selector`](day_pieces::selector) or [`stack`](day_pieces::stack) marked `.restore(key)`
/// remembers its state across launches (and across an Android process death, since the store is
/// disk-backed). Call once at startup, before the UI mounts. Nav keys are namespaced under a
/// `day.nav.` prefix, so `.restore("mail")` never collides with the app's own [`set`]/[`get`]
/// keys.
///
/// ```no_run
/// day_part_prefs::install_nav_store();
/// // … day::launch(app) …
/// ```
pub fn install_nav_store() {
    day_core::set_nav_store(std::rc::Rc::new(PrefsNavStore));
}

/// The [`NavStore`](day_core::NavStore) backed by this prefs store; installed by
/// [`install_nav_store`]. Keys are namespaced under `day.nav.` to stay clear of app data.
struct PrefsNavStore;

impl day_core::NavStore for PrefsNavStore {
    fn load(&self, key: &str) -> Option<String> {
        get(&nav_key(key))
    }
    fn save(&self, key: &str, value: &str) {
        let _ = set(&nav_key(key), value);
    }
}

/// Namespace a `.restore` key so nav state never collides with the app's own prefs keys.
fn nav_key(key: &str) -> String {
    format!("day.nav.{key}")
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

// The web (web-dom, docs/web.md): localStorage through the day-dom shim's imports.
#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod imp;

// Any other platform: no persistent store.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    target_arch = "wasm32"
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
