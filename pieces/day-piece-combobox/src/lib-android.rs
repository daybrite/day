// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: AutoCompleteTextView — Android's real combo box (free-form text with a dropdown of
// suggestions). The Java factory (`dev.daybrite.day.piece.combobox.DayCombo`) is bundled with
// THIS crate under `android/java` and pulled into the app's Gradle build automatically via
// `[package.metadata.day.android]` — the piece carries its own backend Java without touching
// day-android. Typing AND picking an item (the pick writes the text) report back through
// `DayBridge.nativeOnEvent` as K_TEXT_CHANGED, like a built-in text field. It is a growing
// leaf: `measure` fills the proposed width with a natural single-line height; the Java setters
// guard on equality so a programmatic sync never echoes.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::{NodeId, Proposal, Size};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const COMBO_CLASS: &str = "dev/daybrite/day/piece/combobox/DayCombo";

fn make(_backend: &mut Android, p: &ComboProps, id: NodeId) -> AHandle {
    with_env(|env| {
        // A Java throw (e.g. the staged DayCombo class missing from the app build) must not
        // panic inside realize: the panic unwinds the JNI up-call and aborts the process
        // (the "splash-only blank app" class). Degrade to the visible placeholder instead.
        let made = env.new_string(p.items.join("\n")).ok().and_then(|items| {
            let text = env.new_string(&p.text).ok()?;
            let ph = env.new_string(&p.placeholder).ok()?;
            day_android::try_make_view_on(
                env,
                COMBO_CLASS,
                "makeCombo",
                "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&items),
                    JValue::Object(&text),
                    JValue::Object(&ph),
                ],
            )
            .ok()
        });
        AHandle(made.unwrap_or_else(|| {
            log::warn!("day-piece-combobox: DayCombo.makeCombo failed; substituting a placeholder");
            day_android::placeholder_view(env, "combo_box")
        }))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &ComboPatch) {
    match patch {
        ComboPatch::Items(items) => with_env(|env| {
            let Ok(s) = env.new_string(items.join("\n")) else {
                return;
            };
            let _ = env.dcall_static(
                COMBO_CLASS,
                "setComboItems",
                "(Landroid/view/View;Ljava/lang/String;)V",
                &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
            );
        }),
        ComboPatch::SetText(t) => with_env(|env| {
            let Ok(s) = env.new_string(t) else { return };
            let _ = env.dcall_static(
                COMBO_CLASS,
                "setComboText",
                "(Landroid/view/View;Ljava/lang/String;)V",
                &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
            );
        }),
    }
}

fn measure(_backend: &mut Android, _h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); natural single-line height.
    Size::new(p.width.unwrap_or(180.0), p.height.unwrap_or(44.0))
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
