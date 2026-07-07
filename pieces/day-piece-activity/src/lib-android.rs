// ---------------------------------------------------------------------------
// Android: android.widget.ProgressBar — the default style is a circular indeterminate spinner, so
// this piece adds ZERO Gradle dependencies and no permissions. The Java factory
// (`dev.daybrite.day.piece.activity.DayActivity`) is bundled with THIS crate under `android/java`
// and pulled into the app's Gradle build via `[package.metadata.day.android]`, using only
// day-android's PUBLIC Java surface (DayBridge.ctx). See docs/extending.md.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const ACTIVITY_CLASS: &str = "dev/daybrite/day/piece/activity/DayActivity";

fn make(_backend: &mut Android, p: &ActivityProps, _id: NodeId) -> AHandle {
    with_env(|env| {
        let view = env
            .call_static_method(
                ACTIVITY_CLASS,
                "makeActivity",
                "(ZZ)Landroid/view/View;",
                &[JValue::Bool(p.animating as u8), JValue::Bool(p.large as u8)],
            )
            .expect("DayActivity.makeActivity")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Animating(on) => {
            with_env(|env| {
                let _ = env.call_static_method(
                    ACTIVITY_CLASS,
                    "setActivityAnimating",
                    "(Landroid/view/View;Z)V",
                    &[JValue::Object(h.0.as_obj()), JValue::Bool(*on as u8)],
                );
            });
        }
    }
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
