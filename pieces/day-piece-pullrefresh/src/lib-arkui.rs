// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// ArkUI (HarmonyOS): the native tier, ARKUI_NODE_REFRESH, created through day-arkui's node module
// (docs/extending.md). The realized node is the Refresh node: day-core's generic insert mounts
// the wrapped scrollable into it (Refresh hosts exactly one child). Pull-begins route from
// NODE_REFRESH_ON_REFRESH through an additive per-node event receiver (so day-arkui's global
// receiver is untouched) into `day_arkui::emit`; `RefreshPatch` drives NODE_REFRESH_REFRESHING
// both ways. Registration is the same `renderer!` slice as every other backend.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::arkui_sys::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, ArkUI_NodeType,
    OH_ArkUI_NodeEvent_GetEventType, OH_ArkUI_NodeEvent_GetUserData,
};
use day_arkui::node;
use day_arkui::{AHandle, ArkUi};
use day_spec::NodeId;

/// A user pull began on the Refresh node carrying this day NodeId in its event user data.
unsafe extern "C" fn on_event(ev: *mut ArkUI_NodeEvent) {
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live node event for the callback's duration.
        let (kind, id) = unsafe {
            (
                OH_ArkUI_NodeEvent_GetEventType(ev),
                OH_ArkUI_NodeEvent_GetUserData(ev) as usize as u64,
            )
        };
        if kind == Ev::NODE_REFRESH_ON_REFRESH {
            day_arkui::emit(NodeId(id), Event::custom("pullrefresh:begin", ""));
        }
    });
}

fn set_refreshing(h: &AHandle, on: bool) {
    node::set_i32(h.0, Attr::NODE_REFRESH_REFRESHING, i32::from(on));
}

fn make(_backend: &mut ArkUi, p: &RefreshProps, id: NodeId) -> AHandle {
    let n = node::create(ArkUI_NodeType::ARKUI_NODE_REFRESH);
    // Null = Refresh unavailable on this SDK; day falls back per docs.
    if n.is_null() {
        return AHandle(n);
    }
    node::register_event(n, Ev::NODE_REFRESH_ON_REFRESH, id.0);
    node::add_event_receiver(n, on_event);
    let h = AHandle(n);
    if p.refreshing {
        set_refreshing(&h, true);
    }
    h
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &RefreshPatch) {
    let RefreshPatch::SetRefreshing(on) = patch;
    set_refreshing(h, *on);
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: RefreshProps, patch: RefreshPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
