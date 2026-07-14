// ---------------------------------------------------------------------------
// Android: Spinner (menu) / button-row LinearLayout (segmented) / RadioGroup (inline). The Java
// factory (`dev.daybrite.day.piece.picker.DayPicker`) is bundled with THIS crate under `android/java` and
// pulled into the app's Gradle build automatically via `[package.metadata.day.android]` — so the
// piece carries its own backend Java without touching day-android. Rust calls its own class
// through the re-exported `jni` (day-android's `make_view` is hardcoded to DayBridge; a standalone
// piece uses raw `call_static_method` on ITS class).
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::DayEnv;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

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
        let s = env.new_string(&joined).expect("items");
        let view = env
            .dcall_static(
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
            .expect("DayPicker.makePicker")
            .l()
            .expect("View");
        AHandle(std::sync::Arc::new(env.new_global_ref(view).expect("global ref")))
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

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: PickerProps, patch: PickerPatch,
    make: make, update: update);
