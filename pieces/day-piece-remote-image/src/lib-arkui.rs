// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// A native Image node: the Image Kit decodes bytes, ArkUI scales and clips the result.
// The background belongs to the same node, so the clip includes the placeholder and Fit's
// letterbox margins. Only circles need a size observer; a large border radius would make a
// capsule in a non-square frame, contrary to Clip::Circle's centered-circle contract.

use super::*;
use day_arkui::arkui_sys::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeEvent, ArkUI_NodeEventType as Ev,
    OH_ArkUI_NodeEvent_GetEventType, OH_ArkUI_NodeEvent_GetNodeComponentEvent,
    OH_ArkUI_NodeEvent_GetNodeHandle,
};
use day_arkui::arkui_sys::native_type::ArkUI_ClipType;
use day_arkui::{AHandle, ArkUi, node};
use day_spec::NodeId;

unsafe extern "C" fn on_area_change(ev: *mut ArkUI_NodeEvent) {
    day_spec::ffi_guard::contain((), || {
        // SAFETY: ArkUI supplies a live event for this callback. Only the area-change event
        // carries the width/height slots read below, in vp (not physical pixels).
        unsafe {
            if OH_ArkUI_NodeEvent_GetEventType(ev) != Ev::NODE_EVENT_ON_AREA_CHANGE {
                return;
            }
            let data = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            if data.is_null() {
                return;
            }
            node::set_values(
                OH_ArkUI_NodeEvent_GetNodeHandle(ev),
                Attr::NODE_CLIP_SHAPE,
                &[
                    node::i32v(ArkUI_ClipType::ARKUI_CLIP_TYPE_CIRCLE.0 as i32),
                    node::f32v((*data).data[6].f32_),
                    node::f32v((*data).data[7].f32_),
                ],
            );
        }
    });
}

fn make(_backend: &mut ArkUi, p: &RemoteImageProps, id: NodeId) -> AHandle {
    let n = node::create(node::IMAGE);
    let c = p.placeholder;
    let channel = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    node::set_bg_color(
        n,
        (channel(c.a) << 24) | (channel(c.r) << 16) | (channel(c.g) << 8) | channel(c.b),
    );
    node::set_image_fit(
        n,
        match p.mode {
            ContentMode::Fit => 0,
            ContentMode::Fill => 1,
        },
    );
    node::set_clip(n, true);
    match p.clip {
        Clip::None => {}
        Clip::Rounded(radius) => node::set_corner_radius(n, radius),
        Clip::Circle => {
            node::register_event(n, Ev::NODE_EVENT_ON_AREA_CHANGE, id.0);
            node::add_event_receiver(n, on_area_change);
        }
    }
    node::set_image_bytes(n, p.bytes.as_deref().map(Vec::as_slice).unwrap_or_default());
    AHandle(n)
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    node::set_image_bytes(h.0, bytes.as_deref().map(Vec::as_slice).unwrap_or_default());
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
