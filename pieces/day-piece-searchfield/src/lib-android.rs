// ---------------------------------------------------------------------------
// Android: an EditText styled for search (single line, IME_ACTION_SEARCH). The Java factory
// (`dev.daybrite.day.piece.searchfield.DaySearch`) is bundled with THIS crate under `android/java`
// and pulled into the app's Gradle build automatically via `[package.metadata.day.android]` — so the
// piece carries its own backend Java with ZERO edits to day-android. A TextWatcher dispatches edits
// back to Rust via `DayBridge.nativeOnEvent(id, 1, …)` (kind 1 = TextChanged). It is a growing leaf:
// `measure` fills the proposed width (see the webview grow-leaf note) with a natural single-line
// height; `setSearchText` guards on equality so a programmatic sync is a no-op when unchanged.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::{NodeId, Proposal, Size};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const SEARCH_CLASS: &str = "dev/daybrite/day/piece/searchfield/DaySearch";

fn make(_backend: &mut Android, p: &SearchProps, id: NodeId) -> AHandle {
    with_env(|env| {
        let ph = env.new_string(&p.placeholder).expect("placeholder");
        let init = env.new_string(&p.text).expect("initial");
        let view = env
            .call_static_method(
                SEARCH_CLASS,
                "makeSearch",
                "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&ph),
                    JValue::Object(&init),
                ],
            )
            .expect("DaySearch.makeSearch")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    with_env(|env| {
        let s = env.new_string(t).expect("text");
        let _ = env.call_static_method(
            SEARCH_CLASS,
            "setSearchText",
            "(Landroid/view/View;Ljava/lang/String;)V",
            &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
        );
    });
}

fn measure(_backend: &mut Android, _h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); natural single-line height.
    Size::new(p.width.unwrap_or(180.0), p.height.unwrap_or(44.0))
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
