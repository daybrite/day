// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: ARKUI_NODE_LOADING_PROGRESS, the platform's indeterminate spinner, straight from the
// C node API through day-arkui's node module. `large` doubles the default 32 vp to the 64 vp
// HarmonyOS uses for a page-level loader; a stopped spinner is hidden but keeps its box, as on
// Android and iOS.
// ---------------------------------------------------------------------------

use crate::node;
use crate::{AHandle, ArkUi};
use day_spec::NodeId;
use day_spec::props::{ProgressPatch as ActivityPatch, ProgressProps as ActivityProps};

day_reactive::tls_slots! {
    spinner;
    // Entries are reclaimed by the toolkit's release sweep.
    static SIDES: day_spec::sidetable::SideTable<f64> = day_spec::sidetable::SideTable::new();
}

pub(crate) fn side(h: &AHandle) -> Option<f64> {
    SIDES.with(|sizes| sizes.get(h.0 as usize))
}

pub(crate) fn make(_backend: &mut ArkUi, p: &ActivityProps, _id: NodeId) -> AHandle {
    let n = node::create(node::LOADING);
    SIDES.with(|sizes| sizes.insert(n as usize, if p.large { 64.0 } else { 32.0 }));
    node::set_loading(n, p.animating);
    AHandle(n)
}

pub(crate) fn update(_backend: &mut ArkUi, h: &AHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Value(_) => {}
        ActivityPatch::Animating(on) => node::set_loading(h.0, *on),
    }
}
