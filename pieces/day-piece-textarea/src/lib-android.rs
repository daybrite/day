// ---------------------------------------------------------------------------
// Android: a multi-line EditText (inputType textMultiLine|textCapSentences, gravity top) that grows
// between minLines and maxLines and scrolls internally past maxLines. The Java factory
// (`dev.daybrite.day.piece.textarea.DayTextArea`) is bundled with THIS crate under `android/java` and
// pulled into the app's Gradle build automatically via `[package.metadata.day.android]` — so the piece
// carries its own backend Java with ZERO edits to day-android. A TextWatcher dispatches edits back to
// Rust via `DayBridge.nativeOnEvent(id, 1, …)` (kind 1 = TextChanged). `measure` fills the proposed
// width (grow_w leaf) and asks the EditText for its content height (in dp), already clamped to the line
// band by EditText.onMeasure; `setTextAreaText` guards on equality so a programmatic sync is a no-op
// when unchanged.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::{NodeId, Proposal, Size};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const TA_CLASS: &str = "dev/daybrite/day/piece/textarea/DayTextArea";

fn make(_backend: &mut Android, p: &TextProps, id: NodeId) -> AHandle {
    with_env(|env| {
        let ph = env.new_string(&p.placeholder).expect("placeholder");
        let init = env.new_string(&p.text).expect("initial");
        let view = env
            .call_static_method(
                TA_CLASS,
                "makeTextArea",
                "(JLjava/lang/String;Ljava/lang/String;II)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&ph),
                    JValue::Object(&init),
                    JValue::Int(p.min_lines as i32),
                    JValue::Int(p.max_lines as i32),
                ],
            )
            .expect("DayTextArea.makeTextArea")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &TextPatch) {
    let TextPatch::SetText(t) = patch;
    with_env(|env| {
        let s = env.new_string(t).expect("text");
        let _ = env.call_static_method(
            TA_CLASS,
            "setTextAreaText",
            "(Landroid/view/View;Ljava/lang/String;)V",
            &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
        );
    });
}

fn measure(_backend: &mut Android, h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); content-driven height (already clamped to the line band by
    // the EditText). The Java helper returns dp, so no density conversion is needed here.
    let avail_w = p.width.unwrap_or(200.0).max(120.0);
    let h_dp = with_env(|env| {
        env.call_static_method(
            TA_CLASS,
            "measureHeight",
            "(Landroid/view/View;I)I",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Int(avail_w.round() as i32),
            ],
        )
        .expect("DayTextArea.measureHeight")
        .i()
        .unwrap_or(44)
    });
    Size::new(avail_w, (h_dp as f64).max(24.0))
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: TextProps, patch: TextPatch,
    make: make, update: update, measure: measure);
