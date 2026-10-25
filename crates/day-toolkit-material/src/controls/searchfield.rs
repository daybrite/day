// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: an EditText styled for search (single line, IME_ACTION_SEARCH). The Java factory
// (`dev.daybrite.day.piece.searchfield.DaySearch`) is bundled with this crate in `src/DaySearch.java`
// and pulled into the app's Gradle build automatically via `[package.metadata.day.android]`, so the
// piece carries its backend Java without touching day-toolkit-material. A TextWatcher dispatches edits
// back to Rust via `DayBridge.nativeOnEvent(id, 1, …)` (kind 1 = TextChanged). It is a growing leaf:
// `measure` fills the proposed width (see the webview grow-leaf note) with a natural single-line
// height; `setSearchText` guards on equality so a programmatic sync is a no-op when unchanged.
// ---------------------------------------------------------------------------

use day_pieces::searchfield::*;

use crate::jni::objects::JString;
use crate::{AHandle, Material, java};
use day_spec::{NodeId, Proposal, Size};

// This piece's Java class (src/DaySearch.java, on the app classpath at build).
crate::java_class! {
    DaySearch => dev.daybrite.day.piece.searchfield.DaySearch,
    methods {
        static fn makeSearch(id: jlong, placeholder: JString, text: JString) -> android.view.View,
        static fn setSearchText(view: android.view.View, text: JString),
    },
}

fn make(_backend: &mut Material, p: &SearchProps, id: NodeId) -> AHandle {
    crate::realize("search_field", |env| {
        let placeholder = JString::from_str(env, &p.placeholder)?;
        let text = JString::from_str(env, &p.text)?;
        DaySearch::makeSearch(env, id.0 as i64, &placeholder, &text)
    })
}

fn update(_backend: &mut Material, h: &AHandle, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    java(|env| {
        let text = JString::from_str(env, t)?;
        DaySearch::setSearchText(env, h.0.as_obj(), &text)
    });
}

fn measure(_backend: &mut Material, _h: &AHandle, p: Proposal) -> Size {
    // Fill the proposed width (grow_w leaf); natural single-line height.
    Size::new(p.width.unwrap_or(180.0), p.height.unwrap_or(44.0))
}

day_pieces::renderer!(crate::RENDERERS, Material,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
