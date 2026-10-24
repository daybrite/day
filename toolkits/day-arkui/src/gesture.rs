// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native pan and long-press recognizers (docs/shapes.md) through ArkUI's gesture API.
//! Drag phases use the shared gesture wire; a long press delivers Day's typed event directly.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use ohos_sys::arkui::native_gesture::{
    ArkUI_GestureDirection, ArkUI_GestureEvent, ArkUI_GestureEventActionType, ArkUI_GestureMask,
    ArkUI_GesturePriority, ArkUI_GestureRecognizer, ArkUI_NativeGestureAPI_1,
    OH_ArkUI_GestureEvent_GetActionType, OH_ArkUI_GestureEvent_GetRawInputEvent,
    OH_ArkUI_PanGesture_GetOffsetX, OH_ArkUI_PanGesture_GetOffsetY,
};
use ohos_sys::arkui::native_interface::{
    ArkUI_NativeAPIVariantKind, OH_ArkUI_QueryModuleInterfaceByName,
};
use ohos_sys::arkui::ui_input_event::{OH_ArkUI_PointerEvent_GetX, OH_ArkUI_PointerEvent_GetY};

use crate::node::Handle;

struct LongPress {
    id: u64,
    recognizer: NonNull<ArkUI_GestureRecognizer>,
}

day_reactive::tls_slots! {
    gesture;
    static API: Cell<Option<NonNull<ArkUI_NativeGestureAPI_1>>> = const { Cell::new(None) };
    static LONG_PRESSES: RefCell<HashMap<usize, Vec<LongPress>>> = RefCell::new(HashMap::new());
}

unsafe extern "C" fn long_press_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    day_spec::ffi_guard::contain((), || {
        if ev.is_null() {
            return;
        }
        // SAFETY: ArkUI owns the event and its raw input for the duration of this callback.
        let point = unsafe {
            if OH_ArkUI_GestureEvent_GetActionType(ev)
                != ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT
            {
                return;
            }
            let input = OH_ArkUI_GestureEvent_GetRawInputEvent(ev);
            if input.is_null() {
                return;
            }
            let density = crate::node::density();
            day_spec::Point::new(
                f64::from(OH_ArkUI_PointerEvent_GetX(input)) / density,
                f64::from(OH_ArkUI_PointerEvent_GetY(input)) / density,
            )
        };
        crate::emit(
            day_spec::NodeId(extra as usize as u64),
            day_spec::Event::LongPress(point),
        );
    });
}

/// One finger held for ArkUI's default 500 ms. Accept fires once; release/cancellation and
/// continued holding do not repeat it. Normal priority leaves native scrolling/drag arbitration
/// to ArkUI instead of stealing every touch from the enclosing control.
pub fn enable_long_press(node: Handle, id: u64) {
    if node.is_null()
        || LONG_PRESSES.with(|m| {
            m.borrow()
                .get(&(node as usize))
                .is_some_and(|items| items.iter().any(|registered| registered.id == id))
        })
    {
        return;
    }
    let Some(api) = api() else { return };
    let (Some(create), Some(target), Some(add), Some(dispose)) = (
        api.createLongPressGesture,
        api.setGestureEventTarget,
        api.addGestureToNode,
        api.dispose,
    ) else {
        return;
    };
    // SAFETY: the runtime API with its declared argument types; retain the recognizer until
    // its node is released, and dispose immediately if attachment fails.
    unsafe {
        let Some(gesture) = NonNull::new(create(1, false, 500)) else {
            return;
        };
        if target(
            gesture.as_ptr(),
            ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT.0,
            id as usize as *mut c_void,
            Some(long_press_receiver),
        ) != 0
            || add(
                node,
                gesture.as_ptr(),
                ArkUI_GesturePriority::NORMAL,
                ArkUI_GestureMask::NORMAL_GESTURE_MASK,
            ) != 0
        {
            dispose(gesture.as_ptr());
            return;
        }
        LONG_PRESSES.with(|m| {
            m.borrow_mut()
                .entry(node as usize)
                .or_default()
                .push(LongPress {
                    id,
                    recognizer: gesture,
                })
        });
    }
}

/// Detach before the native node is disposed, so callbacks and recognizers cannot outlive it.
pub(crate) fn forget(node: Handle) {
    let gestures = LONG_PRESSES.with(|m| m.borrow_mut().remove(&(node as usize)));
    if let (Some(gestures), Some(api)) = (gestures, api()) {
        for LongPress {
            recognizer: gesture,
            ..
        } in gestures
        {
            // SAFETY: this table owns the recognizer and the node is still live.
            unsafe {
                if let Some(remove) = api.removeGestureFromNode {
                    remove(node, gesture.as_ptr());
                }
                if let Some(dispose) = api.dispose {
                    dispose(gesture.as_ptr());
                }
            }
        }
    }
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
