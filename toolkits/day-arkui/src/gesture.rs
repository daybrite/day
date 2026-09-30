// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The pan recognizer (docs/shapes.md) through ArkUI's gesture API (`ohos_sys::arkui`
//! `native_gesture`): drag phases reach the event dispatch as the shared gesture wire.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

use ohos_sys::arkui::native_gesture::{
    ArkUI_GestureDirection, ArkUI_GestureEvent, ArkUI_GestureEventActionType, ArkUI_GestureMask,
    ArkUI_GesturePriority, ArkUI_NativeGestureAPI_1, OH_ArkUI_GestureEvent_GetActionType,
    OH_ArkUI_GestureEvent_GetRawInputEvent, OH_ArkUI_PanGesture_GetOffsetX,
    OH_ArkUI_PanGesture_GetOffsetY,
};
use ohos_sys::arkui::native_interface::{
    ArkUI_NativeAPIVariantKind, OH_ArkUI_QueryModuleInterfaceByName,
};
use ohos_sys::arkui::ui_input_event::{OH_ArkUI_PointerEvent_GetX, OH_ArkUI_PointerEvent_GetY};

use crate::node::Handle;

thread_local! {
    static API: Cell<Option<NonNull<ArkUI_NativeGestureAPI_1>>> = const { Cell::new(None) };
}

fn api() -> Option<&'static ArkUI_NativeGestureAPI_1> {
    API.with(|slot| {
        if let Some(p) = slot.get() {
            // SAFETY: a process-lifetime table inside ArkUI.
            return Some(unsafe { &*p.as_ptr() });
        }
        // SAFETY: a name lookup with a valid C string.
        let raw = unsafe {
            OH_ArkUI_QueryModuleInterfaceByName(
                ArkUI_NativeAPIVariantKind::ARKUI_NATIVE_GESTURE,
                c"ArkUI_NativeGestureAPI_1".as_ptr(),
            )
        };
        let p = NonNull::new(raw.cast::<ArkUI_NativeGestureAPI_1>())?;
        slot.set(Some(p));
        // SAFETY: as above.
        Some(unsafe { &*p.as_ptr() })
    })
}

unsafe extern "C" fn pan_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    let id = extra as usize as u64;
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live gesture event for the callback's duration.
        let (phase, x, y, tx, ty) = unsafe {
            let action = OH_ArkUI_GestureEvent_GetActionType(ev);
            let phase = match action {
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT => 1.0,
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE => 2.0,
                _ => 3.0, // END or CANCEL
            };
            let tx = OH_ArkUI_PanGesture_GetOffsetX(ev);
            let ty = OH_ArkUI_PanGesture_GetOffsetY(ev);
            let input = OH_ArkUI_GestureEvent_GetRawInputEvent(ev);
            let (x, y) = if input.is_null() {
                (0.0, 0.0)
            } else {
                (
                    OH_ArkUI_PointerEvent_GetX(input),
                    OH_ArkUI_PointerEvent_GetY(input),
                )
            };
            (phase, x, y, tx, ty)
        };
        crate::events::gesture(id, phase, x, y, tx, ty);
    });
}

/// Attach a native pan recognizer to `node`: drag phases reach Rust as gesture events against
/// `id` (location and translation in px; 1 = began, 2 = changed, 3 = ended).
pub fn enable_pan(node: Handle, id: u64) {
    let Some(api) = api() else {
        return;
    };
    if node.is_null() {
        return;
    }
    // SAFETY: the table's own functions with the argument types the header declares.
    unsafe {
        let Some(create) = api.createPanGesture else {
            return;
        };
        let pan = create(1, ArkUI_GestureDirection::GESTURE_DIRECTION_ALL.0, 3.0);
        if pan.is_null() {
            return;
        }
        let mask = ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT.0
            | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE.0
            | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_END.0
            | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_CANCEL.0;
        if let Some(target) = api.setGestureEventTarget {
            target(pan, mask, id as usize as *mut c_void, Some(pan_receiver));
        }
        if let Some(add) = api.addGestureToNode {
            add(
                node,
                pan,
                ArkUI_GesturePriority::NORMAL,
                ArkUI_GestureMask::NORMAL_GESTURE_MASK,
            );
        }
    }
}
