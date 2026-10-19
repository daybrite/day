// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: Spinner (menu) / button-row LinearLayout (segmented) / RadioGroup (inline). The Java
// factory (`dev.daybrite.day.piece.picker.DayPicker`) is bundled with this crate under
// `android/java` and pulled into the app's Gradle build automatically via
// `[package.metadata.day.android]`, so the piece carries its backend Java without touching
// day-android. Rust calls it through a `java_class!` binding, the same way a standalone piece
// calls its own class (docs/android-jni.md).
// ---------------------------------------------------------------------------

use crate::jni::objects::JString;
use crate::{AHandle, Android, java, with_env};
use day_spec::NodeId;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};

// This piece's Java class (in the crate's android/java, on the app classpath at build).
crate::java_class! {
    DayPicker => dev.daybrite.day.piece.picker.DayPicker,
    methods {
        static fn makePicker(id: jlong, style: jint, options: JString, selected: jint,
            enabled: jboolean) -> android.view.View,
        static fn setPickerSelected(view: android.view.View, selected: jint),
        static fn setPickerEnabled(view: android.view.View, on: jboolean),
        static fn setPickerOptions(view: android.view.View, node: jlong, options: JString),
    },
}

day_core::tls_group! {
    /// Picker view → its node. An options patch adds Java views that must report clicks for
    /// this picker, and only Rust knows which node that is. A [`SideTable`], so the backend's
    /// release sweep drops a dead picker's entry.
    static NODES: day_spec::sidetable::SideTable<u64> = day_spec::sidetable::SideTable::new();

}

fn style_code(s: PickerStyle) -> i32 {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut Android, p: &PickerProps, id: NodeId) -> AHandle {
    let joined = p.options.join("\n");
    let h = crate::realize("picker", |env| {
        let options = JString::from_str(env, &joined)?;
        DayPicker::makePicker(
            env,
            id.0 as i64,
            style_code(p.style),
            &options,
            p.selected as i32,
            p.enabled,
        )
    });
    NODES.with(|m| m.insert(h.0.as_obj().as_raw() as usize, id.0));
    h
}

fn update(_backend: &mut Android, h: &AHandle, patch: &PickerPatch) {
    match patch {
        PickerPatch::Selected(i) => {
            java(|env| DayPicker::setPickerSelected(env, h.0.as_obj(), *i as i32));
        }
        PickerPatch::Enabled(on) => {
            java(|env| DayPicker::setPickerEnabled(env, h.0.as_obj(), *on));
        }
        PickerPatch::Options(opts) => {
            let joined = opts.join("\n");
            java(|env| {
                let options = JString::from_str(env, &joined)?;
                DayPicker::setPickerOptions(env, h.0.as_obj(), node_of(h) as i64, &options)
            });
        }
    }
}

/// The node a picker view reports for: a button the option patch adds needs it to report
/// its clicks, and only the Rust side knows it.
fn node_of(h: &AHandle) -> u64 {
    NODES
        .with(|m| m.get(h.0.as_obj().as_raw() as usize))
        .unwrap_or_default()
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
