// ---------------------------------------------------------------------------
// Android: AndroidX SwipeRefreshLayout — the real thing. The Java factory
// (`dev.daybrite.day.piece.pullrefresh.DayPullRefresh`) is bundled with THIS crate under
// `android/java` and pulled into the app's Gradle build via `[package.metadata.day.android]`,
// which also contributes the `androidx.swiperefreshlayout` dependency. The realized node IS the
// SwipeRefreshLayout (a ViewGroup): day-core's generic `addChild` mounts the wrapped scrollable
// directly into it, and the layout wants exactly one scrollable child — which is exactly what the
// piece provides. Pull-begins come back through DayBridge.nativeOnEvent's open Custom-event kind
// (12); `RefreshPatch` drives `setRefreshing`.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const PULLREFRESH_CLASS: &str = "dev/daybrite/day/piece/pullrefresh/DayPullRefresh";

fn make(_backend: &mut Android, p: &RefreshProps, id: NodeId) -> AHandle {
    with_env(|env| {
        let view = env
            .dcall_static(
                PULLREFRESH_CLASS,
                "makePullRefresh",
                "(JZ)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Bool(p.refreshing as day_android::jni::sys::jboolean),
                ],
            )
            .expect("DayPullRefresh.makePullRefresh")
            .l()
            .expect("View");
        AHandle(std::sync::Arc::new(
            env.new_global_ref(view).expect("global ref"),
        ))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &RefreshPatch) {
    let RefreshPatch::SetRefreshing(on) = patch;
    let on = *on;
    with_env(|env| {
        let _ = env.dcall_static(
            PULLREFRESH_CLASS,
            "refreshCommand",
            "(Landroid/view/View;Z)V",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Bool(on as day_android::jni::sys::jboolean),
            ],
        );
    });
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: RefreshProps, patch: RefreshPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
