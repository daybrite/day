// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: an ArkTS composition (platform/harmony/ets/Index.ets, staged through
// `[package.metadata.day.ohos]`): a `TextInput` beside a chevron whose menu lists the
// suggestions. HarmonyOS has no combo box, and the C node API has no menu, so day-arkui's piece
// bridge builds it; typing and picking both arrive as Day's own `TextChanged`.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi, piece};
use day_spec::{NodeId, Proposal, Size};

/// Separates the props' fields and the suggestions (a control character no entry carries).
const SEP: char = '\u{1F}';

fn clean(s: &str) -> String {
    s.chars().filter(|c| *c != SEP).collect()
}

fn joined(items: &[String]) -> String {
    items.iter().map(|i| clean(i)).collect::<Vec<_>>().join(&SEP.to_string())
}

fn make(_backend: &mut ArkUi, p: &ComboProps, id: NodeId) -> AHandle {
    let mut props = format!("{}{SEP}{}", clean(&p.text), clean(&p.placeholder));
    if !p.items.is_empty() {
        props.push(SEP);
        props.push_str(&joined(&p.items));
    }
    piece::make(KIND, id, &props)
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &ComboPatch) {
    match patch {
        ComboPatch::Items(items) => piece::update(h, "items", &joined(items)),
        ComboPatch::SetText(t) => piece::update(h, "text", t),
    }
}

/// The proposed width, at a text field's 40 vp height.
fn measure(_backend: &mut ArkUi, _h: &AHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(200.0), 40.0)
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
