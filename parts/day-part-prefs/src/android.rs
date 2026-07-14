// Android: SharedPreferences, reached via this crate's OWN Java shim (android/java/…/DayPrefs.java) —
// staged into the app's Gradle build by `day build` through [package.metadata.day.android], exactly
// like the UI pieces, but registering NO renderer. The Java uses day-android's cached Context
// (DayBridge.ctx) to open a private SharedPreferences file ("day_part_prefs", MODE_PRIVATE); Rust
// calls it through day-android's re-exported `jni`. So on Android this headless crate rides on the
// Day runtime (it needs the app's JVM + Context). Values persist across launches like every other
// platform. No manifest permission is required — SharedPreferences is app-private storage.

use day_android::jni::objects::{JString, JValue};
use day_android::DayEnv;
use day_android::with_env;

const PREFS_CLASS: &str = "dev/daybrite/day/prefs/DayPrefs";

pub fn set(key: &str, value: &str) -> bool {
    with_env(|env| {
        let (Ok(k), Ok(v)) = (env.new_string(key), env.new_string(value)) else {
            return false;
        };
        env.dcall_static(
            PREFS_CLASS,
            "set",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            &[JValue::Object(&k), JValue::Object(&v)],
        )
        .ok()
        .and_then(|r| r.z().ok())
        .unwrap_or(false)
    })
}

pub fn get(key: &str) -> Option<String> {
    with_env(|env| {
        let k = env.new_string(key).ok()?;
        let obj = env
            .dcall_static(
                PREFS_CLASS,
                "get",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&k)],
            )
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None; // key absent
        }
        env.dstr(&day_android::as_jstring(obj)).ok().map(|s| s.into())
    })
}

pub fn remove(key: &str) -> bool {
    with_env(|env| {
        let Ok(k) = env.new_string(key) else {
            return false;
        };
        env.dcall_static(
            PREFS_CLASS,
            "remove",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&k)],
        )
        .ok()
        .and_then(|r| r.z().ok())
        .unwrap_or(false)
    })
}

pub fn contains(key: &str) -> bool {
    with_env(|env| {
        let Ok(k) = env.new_string(key) else {
            return false;
        };
        env.dcall_static(
            PREFS_CLASS,
            "contains",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&k)],
        )
        .ok()
        .and_then(|r| r.z().ok())
        .unwrap_or(false)
    })
}
