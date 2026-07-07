// Android: ClipboardManager, reached via this crate's OWN Java shim (android/java/…/
// DayClipboard.java) — staged into the app's Gradle build by `day build` through
// [package.metadata.day.android], exactly like the UI pieces, but registering NO renderer. The Java
// uses day-android's cached Context (DayBridge.ctx); Rust calls it through day-android's
// re-exported `jni`. So on Android this headless crate rides on the Day runtime (it needs the app's
// JVM + Context). Note: since Android 10, apps can only READ the clipboard while they hold input
// focus — get_text()/has_text() return None/false in the background. Writing is always allowed.

use day_android::jni::objects::{JString, JValue};
use day_android::with_env;

const CLIPBOARD_CLASS: &str = "dev/daybrite/day/clipboard/DayClipboard";

pub fn set_text(text: &str) -> bool {
    with_env(|env| {
        let Ok(s) = env.new_string(text) else {
            return false;
        };
        env.call_static_method(
            CLIPBOARD_CLASS,
            "setText",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&s)],
        )
        .ok()
        .and_then(|v| v.z().ok())
        .unwrap_or(false)
    })
}

pub fn get_text() -> Option<String> {
    with_env(|env| {
        let obj = env
            .call_static_method(CLIPBOARD_CLASS, "getText", "()Ljava/lang/String;", &[])
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None; // empty clipboard, non-text clip, or read denied (unfocused)
        }
        env.get_string(&JString::from(obj)).ok().map(|s| s.into())
    })
}

pub fn has_text() -> bool {
    with_env(|env| {
        env.call_static_method(CLIPBOARD_CLASS, "hasText", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}
