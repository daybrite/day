// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: the ArkTS `Search` component (platform/harmony/ets/Index.ets), which `day build`
// stages through `[package.metadata.day.ohos]`; the ArkUI C node API has no search kind.
// day-arkui's piece bridge builds it and hands back its FrameNode; edits arrive as Day's own
// `TextChanged`, so the piece's binding needs nothing HarmonyOS-specific.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi, piece};
use day_spec::{NodeId, Proposal, Size};

/// Separates the props' text from its placeholder (a control character no query carries).
const SEP: char = '\u{1F}';

fn make(_backend: &mut ArkUi, p: &SearchProps, id: NodeId) -> AHandle {
    let text: String = p.text.chars().filter(|c| *c != SEP).collect();
    piece::make(KIND, id, &format!("{text}{SEP}{}", p.placeholder))
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &SearchPatch) {
    match patch {
        SearchPatch::SetText(t) => piece::update(h, "text", t),
    }
}

/// The proposed width, at the Search component's own 40 vp height.
fn measure(_backend: &mut ArkUi, _h: &AHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(240.0), 40.0)
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
