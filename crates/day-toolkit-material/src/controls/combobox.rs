// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: AutoCompleteTextView, Android's combo box (free-form text with a dropdown of
// suggestions). The Java factory (`dev.daybrite.day.piece.combobox.DayCombo`) is bundled with
// this crate in `src/DayCombo.java` and pulled into the app's Gradle build automatically via
// `[package.metadata.day.android]`, so the piece carries its own backend Java without touching
// day-toolkit-material. Typing and picking an item (the pick writes the text) report back through
// `DayBridge.nativeOnEvent` as K_TEXT_CHANGED, like a built-in text field. It is a growing
// leaf: `measure` fills the proposed width with a natural single-line height; the Java setters
// guard on equality so a programmatic sync never echoes.
// ---------------------------------------------------------------------------

use day_pieces::combobox::*;

use crate::jni::objects::JString;
use crate::{AHandle, Material, java};
use day_spec::{NodeId, Proposal, Size};

// This piece's Java class (src/DayCombo.java, on the app classpath at build).
crate::java_class! {
    DayCombo => dev.daybrite.day.piece.combobox.DayCombo,
    methods {
        static fn makeCombo(id: jlong, items: JString, text: JString, placeholder: JString)
            -> android.view.View,
        static fn setComboItems(view: android.view.View, items: JString),
        static fn setComboText(view: android.view.View, text: JString),
    },
}

fn make(_backend: &mut Material, p: &ComboProps, id: NodeId) -> AHandle {
    crate::realize("combo_box", |env| {
        let items = JString::from_str(env, p.items.join("\n"))?;
        let text = JString::from_str(env, &p.text)?;
        let placeholder = JString::from_str(env, &p.placeholder)?;
        DayCombo::makeCombo(env, id.0 as i64, &items, &text, &placeholder)
    })
}

fn update(_backend: &mut Material, h: &AHandle, patch: &ComboPatch) {
    match patch {
        ComboPatch::Items(items) => {
            java(|env| {
                let items = JString::from_str(env, items.join("\n"))?;
                DayCombo::setComboItems(env, h.0.as_obj(), &items)
            });
        }
        ComboPatch::SetText(t) => {
            java(|env| {
                let text = JString::from_str(env, t)?;
                DayCombo::setComboText(env, h.0.as_obj(), &text)
            });
        }
    }
}

fn measure(_backend: &mut Material, _h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); natural single-line height.
    Size::new(p.width.unwrap_or(180.0), p.height.unwrap_or(44.0))
}

day_pieces::renderer!(crate::RENDERERS, Material,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
