// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: a multi-line EditText (inputType textMultiLine|textCapSentences, gravity top) that grows
// between minLines and maxLines and scrolls internally past maxLines. The Java factory
// (`dev.daybrite.day.piece.textarea.DayTextArea`) is bundled with THIS crate under `android/java` and
// pulled into the app's Gradle build automatically via `[package.metadata.day.android]` — so the piece
// carries its own backend Java without touching day-android. A TextWatcher dispatches edits back to
// Rust via `DayBridge.nativeOnEvent(id, 1, …)` (kind 1 = TextChanged). `measure` fills the proposed
// width (grow_w leaf) and asks the EditText for its content height (in dp), already clamped to the line
// band by EditText.onMeasure; `setTextAreaText` guards on equality so a programmatic sync is a no-op
// when unchanged.
// ---------------------------------------------------------------------------

use crate::DayEnv;
use crate::jni::objects::JValue;
use crate::{AHandle, Android, with_env};
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use day_spec::{NodeId, Proposal, Size};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const TA_CLASS: &str = "dev/daybrite/day/piece/textarea/DayTextArea";

fn make(_backend: &mut Android, p: &TextProps, id: NodeId) -> AHandle {
    with_env(|env| {
        // Same rule as the picker: a Java throw in realize must degrade to a placeholder,
        // never panic — the panic would unwind the JNI up-call and abort the process.
        let made = env.new_string(&p.placeholder).ok().and_then(|ph| {
            let init = env.new_string(&p.text).ok()?;
            crate::try_make_view_on(
                env,
                TA_CLASS,
                "makeTextArea",
                "(JLjava/lang/String;Ljava/lang/String;IIZZZZ)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&ph),
                    JValue::Object(&init),
                    JValue::Int(p.min_lines as i32),
                    JValue::Int(p.max_lines as i32),
                    JValue::Bool(p.editable),
                    JValue::Bool(p.selectable),
                    JValue::Bool(p.spellcheck),
                    JValue::Bool(p.submit_on_enter),
                ],
            )
            .ok()
        });
        AHandle(made.unwrap_or_else(|| {
            eprintln!("day-android: DayTextArea.makeTextArea failed; substituting a placeholder");
            crate::placeholder_view(env, "text_area")
        }))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &TextPatch) {
    // The three attribute patches all call `<method>(View, boolean)` on the Java shim.
    let bool_attr = |method: &str, value: bool| {
        with_env(|env| {
            let _ = env.dcall_static(
                TA_CLASS,
                method,
                "(Landroid/view/View;Z)V",
                &[JValue::Object(h.0.as_obj()), JValue::Bool(value)],
            );
        });
    };
    match patch {
        TextPatch::SetText(t) => {
            with_env(|env| {
                let Ok(s) = env.new_string(t) else { return };
                let _ = env.dcall_static(
                    TA_CLASS,
                    "setTextAreaText",
                    "(Landroid/view/View;Ljava/lang/String;)V",
                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                );
            });
        }
        TextPatch::SetEditable(v) => bool_attr("setTextAreaEditable", *v),
        TextPatch::SetSelectable(v) => bool_attr("setTextAreaSelectable", *v),
        TextPatch::SetSpellCheck(v) => bool_attr("setTextAreaSpellCheck", *v),
    }
}

fn measure(_backend: &mut Android, h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); content-driven height (already clamped to the line band by
    // the EditText). The Java helper returns dp, so no density conversion is needed here.
    let avail_w = p.width.unwrap_or(200.0).max(120.0);
    let h_dp = with_env(|env| {
        env.dcall_static(
            TA_CLASS,
            "measureHeight",
            "(Landroid/view/View;I)I",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Int(avail_w.round() as i32),
            ],
        )
        // Layout-pass JNI up-call: degrade to the default row height on a throw, don't panic.
        .and_then(|v| v.i())
        .unwrap_or(44)
    });
    Size::new(avail_w, (h_dp as f64).max(24.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Android,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::AHandle {
    let p = props
        .downcast_ref::<TextProps>()
        .expect("day: textarea props type");
    make(b, p, id)
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
