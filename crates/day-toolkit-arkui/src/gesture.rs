// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native pan, pinch and long-press recognizers (docs/shapes.md, docs/canvas.md "Zoom and
//! pan") through ArkUI's gesture API, plus the axis events a mouse wheel or a touchpad sends.
//! Drag, pinch and viewport-pan phases use the shared gesture wire; a long press delivers Day's
//! typed event directly.

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
    OH_ArkUI_PinchGesture_GetCenterX, OH_ArkUI_PinchGesture_GetCenterY,
    OH_ArkUI_PinchGesture_GetScale,
};
use ohos_sys::arkui::native_interface::{
    ArkUI_NativeAPIVariantKind, OH_ArkUI_QueryModuleInterfaceByName,
};
use ohos_sys::arkui::ui_input_event::{
    ArkUI_UIInputEvent, OH_ArkUI_AxisEvent_GetHorizontalAxisValue,
    OH_ArkUI_AxisEvent_GetPinchAxisScaleValue, OH_ArkUI_AxisEvent_GetVerticalAxisValue,
    OH_ArkUI_PointerEvent_GetX, OH_ArkUI_PointerEvent_GetY,
};

use crate::node::Handle;

struct LongPress {
    id: u64,
    recognizer: NonNull<ArkUI_GestureRecognizer>,
}

/// Which axis-driven gestures a node asked for (`AXIS`): a wheel or touchpad scroll pans (a
/// canvas takes every tick, any other container only sideways ones, so vertical scrolling
/// keeps its native behavior), a touchpad pinch scales.
const AXIS_PAN: u8 = 1;
const AXIS_PINCH: u8 = 2;
const AXIS_PAN_SIDEWAYS: u8 = 4;

day_reactive::tls_slots! {
    gesture;
    static API: Cell<Option<NonNull<ArkUI_NativeGestureAPI_1>>> = const { Cell::new(None) };
    static LONG_PRESSES: RefCell<HashMap<usize, Vec<LongPress>>> = RefCell::new(HashMap::new());
    static AXIS: RefCell<HashMap<u64, u8>> = RefCell::new(HashMap::new());
    // The two-finger pan's last reported offset per node: ArkUI's offsets are cumulative
    // since the gesture began, Day's `Event::Pan` deltas incremental.
    static PAN_OFFSETS: RefCell<HashMap<u64, (f32, f32)>> = RefCell::new(HashMap::new());
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

/// Attach `gesture` to `node` reporting every phase to `receiver` against `id`, disposing it
/// when attachment fails. The recognizer stays with the node for its life, like the pan's.
unsafe fn attach(
    api: &ArkUI_NativeGestureAPI_1,
    node: Handle,
    gesture: *mut ArkUI_GestureRecognizer,
    id: u64,
    receiver: unsafe extern "C" fn(*mut ArkUI_GestureEvent, *mut c_void),
) {
    let mask = ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_END.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_CANCEL.0;
    let (Some(target), Some(add)) = (api.setGestureEventTarget, api.addGestureToNode) else {
        if let Some(dispose) = api.dispose {
            // SAFETY: a recognizer this module just created and attached nowhere.
            unsafe { dispose(gesture) };
        }
        return;
    };
    // SAFETY: the table's own functions with the argument types the header declares.
    unsafe {
        let failed = target(gesture, mask, id as usize as *mut c_void, Some(receiver)) != 0
            || add(
                node,
                gesture,
                ArkUI_GesturePriority::NORMAL,
                ArkUI_GestureMask::NORMAL_GESTURE_MASK,
            ) != 0;
        if failed && let Some(dispose) = api.dispose {
            dispose(gesture);
        }
    }
}

unsafe extern "C" fn pinch_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    let id = extra as usize as u64;
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live gesture event for the callback's duration.
        let (phase, x, y, scale) = unsafe {
            let (phase, scale) = match OH_ArkUI_GestureEvent_GetActionType(ev) {
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT => (4.0, 1.0),
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE => {
                    (5.0, OH_ArkUI_PinchGesture_GetScale(ev))
                }
                _ => (6.0, OH_ArkUI_PinchGesture_GetScale(ev)), // END or CANCEL
            };
            // The center comes in vp; the wire carries px, like every other gesture.
            let d = crate::node::density() as f32;
            (
                phase,
                OH_ArkUI_PinchGesture_GetCenterX(ev) * d,
                OH_ArkUI_PinchGesture_GetCenterY(ev) * d,
                scale,
            )
        };
        crate::events::pinch(id, phase, x, y, scale);
    });
}

/// Attach a native pinch recognizer to `node` (docs/canvas.md "Zoom and pan"): two fingers,
/// ArkUI's default 5 px before it engages. Phases 4/5/6 reach Rust against `id` with the
/// center in px and the scale, cumulative since the gesture began as ArkUI reports it. A
/// touchpad pinch arrives as an axis event instead (see [`axis`]).
pub fn enable_pinch(node: Handle, id: u64) {
    let Some(api) = api() else {
        return;
    };
    if node.is_null() {
        return;
    }
    enable_axis(node, id, AXIS_PINCH);
    // SAFETY: the table's own function with the argument types the header declares; the
    // recognizer is handed to `attach` immediately.
    unsafe {
        let Some(create) = api.createPinchGesture else {
            return;
        };
        let pinch = create(2, 0.0);
        if pinch.is_null() {
            return;
        }
        attach(api, node, pinch, id, pinch_receiver);
    }
}

unsafe extern "C" fn viewport_pan_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    let id = extra as usize as u64;
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live gesture event for the callback's duration.
        let (phase, x, y, ox, oy) = unsafe {
            let phase = match OH_ArkUI_GestureEvent_GetActionType(ev) {
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT => 7.0,
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE => 8.0,
                _ => 9.0, // END or CANCEL
            };
            let input = OH_ArkUI_GestureEvent_GetRawInputEvent(ev);
            let (x, y) = if input.is_null() {
                (0.0, 0.0)
            } else {
                (
                    OH_ArkUI_PointerEvent_GetX(input),
                    OH_ArkUI_PointerEvent_GetY(input),
                )
            };
            (
                phase,
                x,
                y,
                OH_ArkUI_PanGesture_GetOffsetX(ev),
                OH_ArkUI_PanGesture_GetOffsetY(ev),
            )
        };
        // Cumulative offset to the delta since the last report.
        let (dx, dy) = PAN_OFFSETS.with(|m| {
            let mut m = m.borrow_mut();
            let delta = match (phase as i32, m.get(&id)) {
                (7, _) | (_, None) => (0.0, 0.0),
                (_, Some((lx, ly))) => (ox - lx, oy - ly),
            };
            if phase as i32 == 9 {
                m.remove(&id);
            } else {
                m.insert(id, (ox, oy));
            }
            delta
        });
        crate::events::gesture(id, phase, x, y, dx, dy);
    });
}

/// Attach a two-finger pan recognizer to a canvas `node` (docs/canvas.md "Zoom and pan"):
/// phases 7/8/9 reach Rust against `id` with the location and the movement since the previous
/// report in px. A mouse wheel or a touchpad scroll arrives as an axis event instead (see
/// [`axis`]). Two fingers, so the one-finger drag of [`enable_pan`] keeps its own stream. Any
/// other container gets the sideways axis events only, and no recognizer that would compete
/// with its own scrolling.
pub fn enable_viewport_pan(node: Handle, id: u64) {
    let Some(api) = api() else {
        return;
    };
    if node.is_null() {
        return;
    }
    if !crate::canvas::is_canvas(node) {
        enable_axis(node, id, AXIS_PAN_SIDEWAYS);
        return;
    }
    enable_axis(node, id, AXIS_PAN);
    // SAFETY: as in `enable_pinch`.
    unsafe {
        let Some(create) = api.createPanGesture else {
            return;
        };
        let pan = create(2, ArkUI_GestureDirection::GESTURE_DIRECTION_ALL.0, 5.0);
        if pan.is_null() {
            return;
        }
        attach(api, node, pan, id, viewport_pan_receiver);
    }
}

/// Ask for the node's axis events (`NODE_ON_AXIS`, API 17) on behalf of one gesture kind.
fn enable_axis(node: Handle, id: u64, kind: u8) {
    let first = AXIS.with(|m| {
        let mut m = m.borrow_mut();
        let bits = m.entry(id).or_default();
        let first = *bits == 0;
        *bits |= kind;
        first
    });
    if first {
        crate::node::register_event(node, crate::node::EV_AXIS, id);
    }
}

/// An axis event over a node that enabled a pan or a pinch: a wheel notch or a touchpad
/// scroll is a lone pan `Changed` carrying the tick's displacement, a touchpad pinch a lone
/// pinch `Changed` carrying the tick's own factor (docs/canvas.md "Zoom and pan"). Scroll
/// units follow the Android bridge, 40 dp per unit, with the content moving the way the
/// axis reads.
pub(crate) fn axis(id: u64, input: *const ArkUI_UIInputEvent) {
    let bits = AXIS.with(|m| m.borrow().get(&id).copied()).unwrap_or(0);
    if input.is_null() || bits == 0 {
        return;
    }
    // SAFETY: a live input event for the receiver's duration; the accessors are ArkUI's.
    let (x, y, dx, dy, scale) = unsafe {
        (
            OH_ArkUI_PointerEvent_GetX(input),
            OH_ArkUI_PointerEvent_GetY(input),
            OH_ArkUI_AxisEvent_GetHorizontalAxisValue(input),
            OH_ArkUI_AxisEvent_GetVerticalAxisValue(input),
            OH_ArkUI_AxisEvent_GetPinchAxisScaleValue(input),
        )
    };
    if bits & AXIS_PINCH != 0 && scale > 0.0 && (scale - 1.0).abs() > f64::EPSILON {
        crate::events::pinch(id, 5.0, x, y, scale as f32);
        return;
    }
    let pans = bits & AXIS_PAN != 0 || (bits & AXIS_PAN_SIDEWAYS != 0 && dx.abs() > dy.abs());
    if pans && (dx != 0.0 || dy != 0.0) {
        let step = 40.0 * crate::node::density();
        crate::events::gesture(id, 8.0, x, y, (dx * step) as f32, (dy * step) as f32);
    }
}
