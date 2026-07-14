// Android: Vibrator / VibrationEffect, driven through this crate's OWN Java shim
// (android/java/…/DayHaptics.java) — staged into the app's Gradle build by `day build` through
// [package.metadata.day.android], exactly like the UI pieces, but registering NO renderer (and
// contributing the VIBRATE manifest permission through the same overlay). The Java uses day-android's
// cached Context (DayBridge.ctx); Rust calls it through day-android's re-exported `jni`. So on
// Android this headless crate rides on the Day runtime (it needs the app's JVM + Context).

use super::Haptic;
use day_android::jni::objects::JValue;
use day_android::DayEnv;
use day_android::with_env;

const HAPTICS_CLASS: &str = "dev/daybrite/day/haptics/DayHaptics";

/// The wire code the Java `play(int)` switches on. Kept in lock-step with `DayHaptics.java`.
fn style_code(h: Haptic) -> i32 {
    match h {
        Haptic::Light => 0,
        Haptic::Medium => 1,
        Haptic::Heavy => 2,
        Haptic::Success => 3,
        Haptic::Warning => 4,
        Haptic::Error => 5,
        Haptic::Selection => 6,
    }
}

pub fn play(h: Haptic) {
    let code = style_code(h);
    with_env(|env| {
        // Fire-and-forget: a failed JNI call (e.g. no Context yet) is swallowed — haptics are never
        // load-bearing.
        let _ = env.dcall_static(HAPTICS_CLASS, "play", "(I)V", &[JValue::Int(code)]);
    });
}

pub fn is_supported() -> bool {
    true
}
