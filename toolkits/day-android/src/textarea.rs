// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: a multi-line EditText (inputType textMultiLine|textCapSentences, gravity top) that grows
// between minLines and maxLines and scrolls internally past maxLines. The Java factory
// (`dev.daybrite.day.piece.textarea.DayTextArea`) is bundled with this crate under `android/java`
// and pulled into the app's Gradle build automatically via `[package.metadata.day.android]`, so
// the piece carries its backend Java without touching day-android. A TextWatcher dispatches edits
// back to Rust via `DayBridge.nativeOnEvent(id, 1, …)` (kind 1 = TextChanged). `measure` fills the
// proposed width (grow_w leaf) and asks the EditText for its content height (in dp), already
// clamped to the line band by EditText.onMeasure; `setTextAreaText` guards on equality so a
// programmatic sync is a no-op when unchanged.
// ---------------------------------------------------------------------------

use crate::jni::objects::JString;
use crate::{AHandle, Android, java, with_env};
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use day_spec::{NodeId, Proposal, Size};

// This piece's Java class (in the crate's android/java, on the app classpath at build).
crate::java_class! {
    DayTextArea => dev.daybrite.day.piece.textarea.DayTextArea,
    methods {
        static fn makeTextArea(id: jlong, placeholder: JString, text: JString, min_lines: jint,
            max_lines: jint, editable: jboolean, selectable: jboolean, spellcheck: jboolean,
            submit_on_enter: jboolean) -> android.view.View,
        static fn setTextAreaText(view: android.view.View, text: JString),
        static fn setTextAreaEditable(view: android.view.View, on: jboolean),
        static fn setTextAreaSelectable(view: android.view.View, on: jboolean),
        static fn setTextAreaSpellCheck(view: android.view.View, on: jboolean),
        static fn measureHeight(view: android.view.View, width: jint) -> jint,
    },
}

fn make(_backend: &mut Android, p: &TextProps, id: NodeId) -> AHandle {
    crate::realize("text_area", |env| {
        let placeholder = JString::from_str(env, &p.placeholder)?;
        let text = JString::from_str(env, &p.text)?;
        DayTextArea::makeTextArea(
            env,
            id.0 as i64,
            &placeholder,
            &text,
            p.min_lines as i32,
            p.max_lines as i32,
            p.editable,
            p.selectable,
            p.spellcheck,
            p.submit_on_enter,
        )
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &TextPatch) {
    let view = h.0.as_obj();
    match patch {
        TextPatch::SetText(t) => {
            java(|env| {
                let text = JString::from_str(env, t)?;
                DayTextArea::setTextAreaText(env, view, &text)
            });
        }
        TextPatch::SetEditable(v) => {
            java(|env| DayTextArea::setTextAreaEditable(env, view, *v));
        }
        TextPatch::SetSelectable(v) => {
            java(|env| DayTextArea::setTextAreaSelectable(env, view, *v));
        }
        TextPatch::SetSpellCheck(v) => {
            java(|env| DayTextArea::setTextAreaSpellCheck(env, view, *v));
        }
    }
}

fn measure(_backend: &mut Android, h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); content-driven height (already clamped to the line band by
    // the EditText). The Java helper returns dp, so no density conversion is needed here.
    let avail_w = p.width.unwrap_or(200.0).max(120.0);
    // Layout-pass JNI up-call: degrade to the default row height on a throw, don't panic.
    let h_dp = java(|env| DayTextArea::measureHeight(env, h.0.as_obj(), avail_w.round() as i32))
        .unwrap_or(44);
    Size::new(avail_w, (h_dp as f64).max(24.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Android,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::AHandle {
    // A props-type mismatch degrades to the visible placeholder (day_spec::props_of reports
    // it); this runs inside a JNI up-call, where a panic is a process kill.
    match day_spec::props_of::<TextProps>(day_spec::kinds::TEXT_AREA, "android", props) {
        Some(p) => make(b, p, id),
        None => with_env(|env| AHandle(crate::placeholder_view(env, "text_area"))),
    }
}

pub(crate) fn update_any(b: &mut crate::Android, h: &crate::AHandle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<TextPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut crate::Android,
    h: &crate::AHandle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
