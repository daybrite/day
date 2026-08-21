// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: Spinner (menu) / button-row LinearLayout (segmented) / RadioGroup (inline). The Java
// factory (`dev.daybrite.day.piece.picker.DayPicker`) is bundled with THIS crate under `android/java` and
// pulled into the app's Gradle build automatically via `[package.metadata.day.android]` — so the
// piece carries its own backend Java without touching day-android. Rust calls its own class
// through the re-exported `jni` (day-android's `make_view` is hardcoded to DayBridge; a standalone
// piece uses raw `call_static_method` on ITS class).
// ---------------------------------------------------------------------------

use crate::DayEnv;
use crate::jni::objects::JValue;
use crate::{AHandle, Android, with_env};
use day_spec::NodeId;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const PICKER_CLASS: &str = "dev/daybrite/day/piece/picker/DayPicker";

fn style_code(s: PickerStyle) -> i32 {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut Android, p: &PickerProps, id: NodeId) -> AHandle {
    let joined = p.options.join("\n");
    with_env(|env| {
        // A Java throw (e.g. the staged DayPicker class missing from the app build) must
        // not panic inside realize: the panic unwinds the JNI up-call and aborts the
        // process. Degrade to the bridge's visible placeholder label instead.
        let made = env.new_string(&joined).ok().and_then(|s| {
            crate::try_make_view_on(
                env,
                PICKER_CLASS,
                "makePicker",
                "(JILjava/lang/String;I)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Int(style_code(p.style)),
                    JValue::Object(&s),
                    JValue::Int(p.selected as i32),
                ],
            )
            .ok()
        });
        AHandle(made.unwrap_or_else(|| {
            log::warn!("day-android: DayPicker.makePicker failed; substituting a placeholder");
            crate::placeholder_view(env, "picker")
        }))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &PickerPatch) {
    {
        let PickerPatch::Selected(i) = patch;
        with_env(|env| {
            let _ = env.dcall_static(
                PICKER_CLASS,
                "setPickerSelected",
                "(Landroid/view/View;I)V",
                &[JValue::Object(h.0.as_obj()), JValue::Int(*i as i32)],
            );
        });
    }
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Android,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::AHandle {
    // A props-type mismatch degrades to the visible placeholder (day_spec::props_of reports
    // it) — this runs inside a JNI up-call, where a panic is a process kill.
    match day_spec::props_of::<PickerProps>(day_spec::kinds::PICKER, "android", props) {
        Some(p) => make(b, p, id),
        None => with_env(|env| AHandle(crate::placeholder_view(env, "picker"))),
    }
}

pub(crate) fn update_any(b: &mut crate::Android, h: &crate::AHandle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
        update(b, h, p);
    }
}
