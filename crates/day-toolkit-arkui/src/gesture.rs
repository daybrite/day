// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native pan, pinch, rotation and long-press recognizers (docs/shapes.md, docs/canvas.md "Zoom
//! and pan") through ArkUI's gesture API, plus the axis events a mouse wheel or a touchpad
//! sends. Drag, pinch, rotation and viewport-pan phases use the shared gesture wire; a long
//! press delivers Day's typed event directly.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use day_spec::{
    DragAxis, Event, GestureKind, Insets, NodeId, Point, PointerKind, Touch, TouchPhase,
};
use ohos_sys::arkui::native_gesture::{
    ArkUI_GestureDirection, ArkUI_GestureEvent, ArkUI_GestureEventActionType, ArkUI_GestureMask,
    ArkUI_GesturePriority, ArkUI_GestureRecognizer, ArkUI_GroupGestureMode,
    ArkUI_NativeGestureAPI_1, OH_ArkUI_GestureEvent_GetActionType,
    OH_ArkUI_GestureEvent_GetRawInputEvent, OH_ArkUI_PanGesture_GetOffsetX,
    OH_ArkUI_PanGesture_GetOffsetY, OH_ArkUI_PanGesture_GetVelocityX,
    OH_ArkUI_PanGesture_GetVelocityY, OH_ArkUI_PinchGesture_GetCenterX,
    OH_ArkUI_PinchGesture_GetCenterY, OH_ArkUI_PinchGesture_GetScale,
    OH_ArkUI_RotationGesture_GetAngle,
};
use ohos_sys::arkui::native_interface::{
    ArkUI_NativeAPIVariantKind, OH_ArkUI_QueryModuleInterfaceByName,
};
use ohos_sys::arkui::native_node::ArkUI_NodeAttributeType as Attr;
use ohos_sys::arkui::ui_input_event::{
    ArkUI_UIInputEvent, OH_ArkUI_AxisEvent_GetHorizontalAxisValue,
    OH_ArkUI_AxisEvent_GetPinchAxisScaleValue, OH_ArkUI_AxisEvent_GetVerticalAxisValue,
    OH_ArkUI_PointerEvent_GetChangedPointerId, OH_ArkUI_PointerEvent_GetPointerCount,
    OH_ArkUI_PointerEvent_GetPointerId, OH_ArkUI_PointerEvent_GetPressure,
    OH_ArkUI_PointerEvent_GetX, OH_ArkUI_PointerEvent_GetXByIndex, OH_ArkUI_PointerEvent_GetY,
    OH_ArkUI_PointerEvent_GetYByIndex, OH_ArkUI_UIInputEvent_GetAction,
    OH_ArkUI_UIInputEvent_GetToolType,
};

use crate::node::Handle;

struct LongPress {
    id: u64,
    recognizer: NonNull<ArkUI_GestureRecognizer>,
}

/// A node's one-finger drag recognizer and what it was built for, kept so an axis hint can
/// rebuild it with a direction mask (`set_drag_axis`).
struct Pan {
    id: u64,
    recognizer: NonNull<ArkUI_GestureRecognizer>,
}

/// A node's tap kinds (`GestureKind::Tap`, `GestureKind::DoubleTap`) and the recognizer that
/// serves the double tap: a lone two-tap recognizer, or an exclusive group of a two-tap ahead
/// of a one-tap when the node also takes single taps (the group gives the single tap its wait).
struct Taps {
    id: u64,
    single: bool,
    double: bool,
    recognizer: Option<NonNull<ArkUI_GestureRecognizer>>,
}

// `ui_input_event.h`'s touch actions and tool types: anonymous enums the bindings leave
// unexported.
const UI_TOUCH_EVENT_ACTION_CANCEL: u32 = 0;
const UI_TOUCH_EVENT_ACTION_DOWN: u32 = 1;
const UI_TOUCH_EVENT_ACTION_MOVE: u32 = 2;
const UI_TOUCH_EVENT_ACTION_UP: u32 = 3;
const UI_INPUT_EVENT_TOOL_TYPE_PEN: u32 = 2;
const UI_INPUT_EVENT_TOOL_TYPE_MOUSE: u32 = 3;
const UI_INPUT_EVENT_TOOL_TYPE_TOUCHPAD: u32 = 4;

/// What a node's touch stream (`NODE_TOUCH_EVENT`) feeds: `Event::Press`, `Event::Touches`.
const TOUCH_PRESS: u8 = 1;
const TOUCH_CONTACTS: u8 = 2;

/// One node's touch state: the kinds it asked for, the press under way (its last location in
/// vp), and the live contacts by pointer id with their last position in px.
#[derive(Default)]
struct TouchState {
    id: u64,
    kinds: u8,
    pressed: Option<Point>,
    contacts: HashMap<i32, (f32, f32)>,
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
    // Each node's parallel group holding its pinch and rotation recognizers, so the two run
    // at once as one hand movement instead of competing for the fingers.
    static TWO_FINGER: RefCell<HashMap<usize, NonNull<ArkUI_GestureRecognizer>>> =
        RefCell::new(HashMap::new());
    // The rotation's last raw angle and the unwrapped total since it began, in degrees, per
    // node: ArkUI reports the angle within a half turn either way, Day's rotation keeps
    // counting past it.
    static ROTATIONS: RefCell<HashMap<u64, (f32, f64)>> = RefCell::new(HashMap::new());
    // Per node handle: the drag recognizer, the tap kinds, the touch state, the hit slop (vp)
    // with the node's last laid-out size (vp) its response region was computed from.
    static PANS: RefCell<HashMap<usize, Pan>> = RefCell::new(HashMap::new());
    static TAPS: RefCell<HashMap<usize, Taps>> = RefCell::new(HashMap::new());
    static TOUCHES: RefCell<HashMap<usize, TouchState>> = RefCell::new(HashMap::new());
    static SLOP: RefCell<HashMap<usize, (Insets, (f64, f64))>> = RefCell::new(HashMap::new());
    // The device kind behind the latest pointer down, once one has been seen.
    static POINTER_KIND: Cell<Option<PointerKind>> = const { Cell::new(None) };
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
    crate::node::accept_input(node);
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
                });
        });
    }
}

/// Detach before the native node is disposed, so callbacks and recognizers cannot outlive it.
pub(crate) fn forget(node: Handle) {
    // The group goes with the node's recognizers; a node later born at the same address
    // starts a group of its own.
    TWO_FINGER.with(|m| m.borrow_mut().remove(&(node as usize)));
    TOUCHES.with(|m| m.borrow_mut().remove(&(node as usize)));
    SLOP.with(|m| m.borrow_mut().remove(&(node as usize)));
    let pan = PANS.with(|m| m.borrow_mut().remove(&(node as usize)));
    let taps = TAPS
        .with(|m| m.borrow_mut().remove(&(node as usize)))
        .and_then(|t| t.recognizer);
    if let Some(api) = api() {
        for gesture in pan.map(|p| p.recognizer).into_iter().chain(taps) {
            // SAFETY: these tables own the recognizers and the node is still live. A tap
            // group's children are left to the group: only the group itself is disposed.
            unsafe { detach(api, node, gesture) };
        }
    }
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
            // The recognizer's own release velocity (px/s), sent just before `Ended` so
            // day-core reports it instead of its estimate. A cancelled pan has none.
            if action == ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_END {
                let d = crate::node::density();
                let velocity = Point::new(
                    f64::from(OH_ArkUI_PanGesture_GetVelocityX(ev)) / d,
                    f64::from(OH_ArkUI_PanGesture_GetVelocityY(ev)) / d,
                );
                crate::emit(
                    NodeId(id),
                    Event::GestureVelocity {
                        kind: GestureKind::Drag,
                        velocity,
                    },
                );
            }
            (phase, x, y, tx, ty)
        };
        crate::events::gesture(id, phase, x, y, tx, ty);
    });
}

/// Attach a native pan recognizer to `node`: drag phases reach Rust as gesture events against
/// `id` (location and translation in px; 1 = began, 2 = changed, 3 = ended). One finger, any
/// direction, until [`set_drag_axis`] narrows it.
pub fn enable_pan(node: Handle, id: u64) {
    if node.is_null() || PANS.with(|m| m.borrow().contains_key(&(node as usize))) {
        return;
    }
    build_pan(node, id, ArkUI_GestureDirection::GESTURE_DIRECTION_ALL);
}

/// Build and attach the one-finger pan with `direction`, recording it for the node.
fn build_pan(node: Handle, id: u64, direction: ArkUI_GestureDirection) {
    let Some(api) = api() else {
        return;
    };
    crate::node::accept_input(node);
    // SAFETY: the table's own functions with the argument types the header declares.
    unsafe {
        let Some(create) = api.createPanGesture else {
            return;
        };
        let Some(pan) = NonNull::new(create(1, direction.0, 3.0)) else {
            return;
        };
        if attach(api, node, pan.as_ptr(), id, pan_receiver) {
            PANS.with(|m| {
                m.borrow_mut().insert(
                    node as usize,
                    Pan {
                        id,
                        recognizer: pan,
                    },
                )
            });
        }
    }
}

/// The axis hint (`Toolkit::set_drag_axis`, docs/gestures.md "Gestures inside a scroll view"):
/// rebuild the node's pan with a direction mask, so ArkUI never recognizes a movement across
/// the axis as the node's drag and an enclosing `Scroll` or `List` takes it instead. `None`
/// restores every direction.
pub fn set_drag_axis(node: Handle, axis: Option<DragAxis>) {
    let Some(api) = api() else {
        return;
    };
    let Some(Pan { id, recognizer }) = PANS.with(|m| m.borrow_mut().remove(&(node as usize)))
    else {
        return;
    };
    // SAFETY: the table owned the recognizer, attached to this live node.
    unsafe { detach(api, node, recognizer) };
    let direction = match axis {
        Some(DragAxis::Horizontal) => ArkUI_GestureDirection::GESTURE_DIRECTION_HORIZONTAL,
        Some(DragAxis::Vertical) => ArkUI_GestureDirection::GESTURE_DIRECTION_VERTICAL,
        None => ArkUI_GestureDirection::GESTURE_DIRECTION_ALL,
    };
    build_pan(node, id, direction);
}

/// Remove `gesture` from `node` and dispose it.
///
/// # Safety
/// `gesture` must be a live recognizer this module owns, attached to the live `node` (or to
/// nothing).
unsafe fn detach(
    api: &ArkUI_NativeGestureAPI_1,
    node: Handle,
    gesture: NonNull<ArkUI_GestureRecognizer>,
) {
    // SAFETY: under this function's contract.
    unsafe {
        if let Some(remove) = api.removeGestureFromNode {
            remove(node, gesture.as_ptr());
        }
        if let Some(dispose) = api.dispose {
            dispose(gesture.as_ptr());
        }
    }
}

unsafe extern "C" fn tap_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    day_spec::ffi_guard::contain((), || tap_fired(ev, extra, false));
}

unsafe extern "C" fn double_tap_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    day_spec::ffi_guard::contain((), || tap_fired(ev, extra, true));
}

/// A tap recognizer accepted: the point is the raw input's (the last finger up), in vp.
fn tap_fired(ev: *mut ArkUI_GestureEvent, extra: *mut c_void, double: bool) {
    if ev.is_null() {
        return;
    }
    // SAFETY: ArkUI owns the event and its raw input for the duration of the callback.
    let point = unsafe {
        let input = OH_ArkUI_GestureEvent_GetRawInputEvent(ev);
        if input.is_null() {
            Point::ZERO
        } else {
            let d = crate::node::density();
            Point::new(
                f64::from(OH_ArkUI_PointerEvent_GetX(input)) / d,
                f64::from(OH_ArkUI_PointerEvent_GetY(input)) / d,
            )
        }
    };
    let id = NodeId(extra as usize as u64);
    crate::emit(
        id,
        if double {
            Event::DoubleTap(point)
        } else {
            Event::Tap(point)
        },
    );
}

/// Record that `node` takes single taps (`double = false`) or double taps (`true`), and
/// (re)build its tap recognizers (docs/gestures.md "Double tap"). Single taps alone stay on
/// the node's click event (lib.rs). With a double tap, ArkUI's own two-tap recognizer
/// reports it; when the node takes both, an exclusive group holds the two-tap ahead of a
/// one-tap, so a single tap is reported only once the double-tap interval passed, and the
/// click event stops reporting taps for the node ([`taps_grouped`]). Idempotent per kind.
pub fn enable_tap(node: Handle, id: u64, double: bool) {
    if node.is_null() {
        return;
    }
    let key = node as usize;
    let (changed, rebuild, old) = TAPS.with(|m| {
        let mut m = m.borrow_mut();
        let taps = m.entry(key).or_insert(Taps {
            id,
            single: false,
            double: false,
            recognizer: None,
        });
        let flag = if double {
            &mut taps.double
        } else {
            &mut taps.single
        };
        let changed = !*flag;
        *flag = true;
        let rebuild = changed && taps.double;
        let old = if rebuild {
            taps.recognizer.take()
        } else {
            None
        };
        (changed, rebuild, old)
    });
    if !changed || !rebuild {
        return;
    }
    let Some(api) = api() else { return };
    if let Some(old) = old {
        // SAFETY: the table owned it, attached to this live node. A group is disposed alone,
        // its children with it as far as this module is concerned.
        unsafe { detach(api, node, old) };
    }
    crate::node::accept_input(node);
    let single = TAPS.with(|m| m.borrow().get(&key).is_some_and(|t| t.single));
    if single {
        // The node's click event is a tap recognizer of its own that would win every single
        // tap from the group: the group's one-tap reports them instead.
        crate::node::unregister_event(node, crate::node::EV_CLICK);
    }
    let (Some(create_tap), Some(target)) = (api.createTapGesture, api.setGestureEventTarget) else {
        return;
    };
    let accept = ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT.0;
    // SAFETY: the table's own functions with the argument types the header declares; each
    // recognizer is attached (or disposed) before this returns.
    let built = unsafe {
        let Some(two) = NonNull::new(create_tap(2, 1)) else {
            return;
        };
        if target(
            two.as_ptr(),
            accept,
            id as usize as *mut c_void,
            Some(double_tap_receiver),
        ) != 0
        {
            if let Some(dispose) = api.dispose {
                dispose(two.as_ptr());
            }
            return;
        }
        if single
            && let (Some(create_group), Some(add_child)) =
                (api.createGroupGesture, api.addChildGesture)
            && let Some(group) = NonNull::new(create_group(ArkUI_GroupGestureMode::EXCLUSIVE_GROUP))
            && let Some(one) = NonNull::new(create_tap(1, 1))
        {
            target(
                one.as_ptr(),
                accept,
                id as usize as *mut c_void,
                Some(tap_receiver),
            );
            add_child(group.as_ptr(), two.as_ptr());
            add_child(group.as_ptr(), one.as_ptr());
            add_gesture(api, node, group)
        } else {
            add_gesture(api, node, two)
        }
    };
    TAPS.with(|m| {
        if let Some(taps) = m.borrow_mut().get_mut(&key) {
            taps.recognizer = built;
        }
    });
}

/// Attach `gesture` at normal priority, disposing it when that fails.
///
/// # Safety
/// `gesture` must be a live recognizer this module created and attached nowhere.
unsafe fn add_gesture(
    api: &ArkUI_NativeGestureAPI_1,
    node: Handle,
    gesture: NonNull<ArkUI_GestureRecognizer>,
) -> Option<NonNull<ArkUI_GestureRecognizer>> {
    // SAFETY: under this function's contract.
    unsafe {
        let add = api.addGestureToNode?;
        if add(
            node,
            gesture.as_ptr(),
            ArkUI_GesturePriority::NORMAL,
            ArkUI_GestureMask::NORMAL_GESTURE_MASK,
        ) == 0
        {
            return Some(gesture);
        }
        if let Some(dispose) = api.dispose {
            dispose(gesture.as_ptr());
        }
        None
    }
}

/// Whether node `id`'s single taps come from its tap group rather than its click event.
pub(crate) fn taps_grouped(id: u64) -> bool {
    TAPS.with(|m| {
        m.borrow()
            .values()
            .any(|t| t.id == id && t.single && t.double && t.recognizer.is_some())
    })
}

/// Ask for the node's touch stream on behalf of `Event::Press` (`press`) or `Event::Touches`.
/// The stream is observed, never consumed: ArkUI keeps delivering the same touches to the
/// node's recognizers, its controls and the enclosing scroll views.
pub fn enable_touch(node: Handle, id: u64, press: bool) {
    if node.is_null() {
        return;
    }
    let kind = if press { TOUCH_PRESS } else { TOUCH_CONTACTS };
    TOUCHES.with(|m| {
        let mut m = m.borrow_mut();
        let state = m.entry(node as usize).or_default();
        state.id = id;
        state.kinds |= kind;
    });
    crate::node::register_event(node, crate::node::EV_TOUCH, id);
}

/// The kind of device behind a touch event.
///
/// # Safety
/// `input` must be a live input event.
unsafe fn tool_kind(input: *const ArkUI_UIInputEvent) -> PointerKind {
    // SAFETY: under this function's contract.
    let tool = unsafe { OH_ArkUI_UIInputEvent_GetToolType(input) };
    match u32::try_from(tool).unwrap_or(0) {
        t if t == UI_INPUT_EVENT_TOOL_TYPE_PEN => PointerKind::Stylus,
        t if t == UI_INPUT_EVENT_TOOL_TYPE_MOUSE || t == UI_INPUT_EVENT_TOOL_TYPE_TOUCHPAD => {
            PointerKind::Mouse
        }
        _ => PointerKind::Touch,
    }
}

/// A `NODE_TOUCH_EVENT` on node `n` (registered for `id`): report the device kind when it
/// changed (docs/gestures.md "Input kind"), then feed the node's press and contact reports.
pub(crate) fn touch(n: Handle, input: *const ArkUI_UIInputEvent) {
    if input.is_null() {
        return;
    }
    // SAFETY: a live input event for the receiver's duration; the accessors are ArkUI's.
    let (action, kind, count, changed) = unsafe {
        let count = OH_ArkUI_PointerEvent_GetPointerCount(input);
        let mut changed = 0u32;
        // A lone contact is the one that changed, whatever the query answers.
        let changed = if count == 1 {
            Some(0)
        } else {
            (OH_ArkUI_PointerEvent_GetChangedPointerId(input, &raw mut changed) == 0
                && changed < count)
                .then_some(changed)
        };
        (
            u32::try_from(OH_ArkUI_UIInputEvent_GetAction(input)).unwrap_or(u32::MAX),
            tool_kind(input),
            count,
            changed,
        )
    };
    if action == UI_TOUCH_EVENT_ACTION_DOWN
        && POINTER_KIND.with(|k| k.replace(Some(kind))) != Some(kind)
    {
        crate::emit(day_spec::WINDOW_NODE, Event::PointerKindChanged(kind));
    }
    let Some((id, kinds)) =
        TOUCHES.with(|m| m.borrow().get(&(n as usize)).map(|s| (s.id, s.kinds)))
    else {
        return;
    };
    let d = crate::node::density();
    // The point that changed, in px relative to the node.
    // SAFETY: as above; an index the event itself named.
    let (cx, cy) = unsafe {
        match changed {
            Some(i) => (
                OH_ArkUI_PointerEvent_GetXByIndex(input, i),
                OH_ArkUI_PointerEvent_GetYByIndex(input, i),
            ),
            None => (
                OH_ArkUI_PointerEvent_GetX(input),
                OH_ArkUI_PointerEvent_GetY(input),
            ),
        }
    };
    if kinds & TOUCH_PRESS != 0 {
        press(
            n,
            id,
            action,
            count,
            Point::new(f64::from(cx) / d, f64::from(cy) / d),
        );
    }
    if kinds & TOUCH_CONTACTS != 0 {
        contacts(n, id, input, action, kind, changed);
    }
}

/// `Event::Press` from the touch stream: in at the first finger's down, out once at the last
/// finger's release, a cancellation, or the finger leaving the node's hit area (its bounds
/// grown by the hit slop). Re-entering does not press again.
fn press(n: Handle, id: u64, action: u32, count: u32, at: Point) {
    let event = TOUCHES.with(|m| {
        let mut m = m.borrow_mut();
        let state = m.get_mut(&(n as usize))?;
        match action {
            a if a == UI_TOUCH_EVENT_ACTION_DOWN => {
                (count <= 1 && state.pressed.is_none()).then(|| {
                    state.pressed = Some(at);
                    true
                })
            }
            a if a == UI_TOUCH_EVENT_ACTION_MOVE => {
                state.pressed?;
                if inside_hit_area(n, at) {
                    state.pressed = Some(at);
                    None
                } else {
                    state.pressed = None;
                    Some(false)
                }
            }
            a if a == UI_TOUCH_EVENT_ACTION_UP && count > 1 => None,
            _ => state.pressed.take().map(|_| false),
        }
    });
    if let Some(pressed) = event {
        crate::emit(
            NodeId(id),
            Event::Press {
                pressed,
                location: at,
            },
        );
    }
}

/// Whether `at` (vp, node-local) lies inside the node's bounds grown by its hit slop.
fn inside_hit_area(n: Handle, at: Point) -> bool {
    let d = crate::node::density();
    let Some((_, _, w, h)) = crate::node::layout_frame_px(n) else {
        return true;
    };
    let slop = SLOP
        .with(|m| m.borrow().get(&(n as usize)).map(|(s, _)| *s))
        .unwrap_or_default();
    at.x >= -slop.leading
        && at.y >= -slop.top
        && at.x <= w / d + slop.trailing
        && at.y <= h / d + slop.bottom
}

/// `Event::Touches` from the touch stream: every contact on the node, the one that changed
/// with its own phase and the rest moved or stationary; ended and cancelled contacts appear
/// once with that phase and are then forgotten. A mouse is the one contact 0.
fn contacts(
    n: Handle,
    id: u64,
    input: *const ArkUI_UIInputEvent,
    action: u32,
    kind: PointerKind,
    changed: Option<u32>,
) {
    let d = crate::node::density();
    // SAFETY: a live input event; every index is below the count the event reports.
    let points: Vec<(i32, f32, f32, f64)> = unsafe {
        (0..OH_ArkUI_PointerEvent_GetPointerCount(input))
            .map(|i| {
                let pressure = f64::from(OH_ArkUI_PointerEvent_GetPressure(input, i));
                (
                    OH_ArkUI_PointerEvent_GetPointerId(input, i),
                    OH_ArkUI_PointerEvent_GetXByIndex(input, i),
                    OH_ArkUI_PointerEvent_GetYByIndex(input, i),
                    // A device that reports no pressure says 0 while it is down.
                    if pressure > 0.0 {
                        pressure.min(1.0)
                    } else {
                        1.0
                    },
                )
            })
            .collect()
    };
    // SAFETY: as above.
    let changed_id = changed.map(|i| unsafe { OH_ArkUI_PointerEvent_GetPointerId(input, i) });
    let touches = TOUCHES.with(|m| {
        let mut m = m.borrow_mut();
        let state = m.get_mut(&(n as usize))?;
        let mut touches = Vec::with_capacity(points.len());
        let mut ended = Vec::new();
        for &(pid, x, y, pressure) in &points {
            let phase = if action == UI_TOUCH_EVENT_ACTION_CANCEL {
                TouchPhase::Cancelled
            } else if Some(pid) == changed_id && action == UI_TOUCH_EVENT_ACTION_UP {
                TouchPhase::Ended
            } else {
                match state.contacts.get(&pid) {
                    None => TouchPhase::Began,
                    Some(&(lx, ly)) if (lx - x).abs() > 0.01 || (ly - y).abs() > 0.01 => {
                        TouchPhase::Moved
                    }
                    Some(_) => TouchPhase::Stationary,
                }
            };
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                ended.push(pid);
            } else {
                state.contacts.insert(pid, (x, y));
            }
            let contact = if kind == PointerKind::Mouse {
                0
            } else {
                u64::from(pid.unsigned_abs())
            };
            touches.push(
                Touch::new(
                    contact,
                    phase,
                    Point::new(f64::from(x) / d, f64::from(y) / d),
                )
                .with_pressure(pressure)
                .with_kind(kind),
            );
        }
        for pid in ended {
            state.contacts.remove(&pid);
        }
        if action == UI_TOUCH_EVENT_ACTION_CANCEL {
            state.contacts.clear();
        }
        (!touches.is_empty()).then_some(touches)
    });
    if let Some(touches) = touches {
        crate::emit(NodeId(id), Event::Touches(touches));
    }
}

/// Grow the area in which `node` takes input by `slop` vp on each edge (docs/gestures.md
/// "Hit slop") through its response region (`NODE_RESPONSE_REGION`: an offset in vp and a
/// size in percent of the node). Kept so each new frame size recomputes the percentages;
/// zero insets reset the region to the node's bounds.
pub fn set_hit_slop(node: Handle, slop: Insets) {
    let size = SLOP.with(|m| m.borrow().get(&(node as usize)).map(|(_, s)| *s));
    let size = size.or_else(|| {
        let d = crate::node::density();
        crate::node::layout_frame_px(node).map(|(_, _, w, h)| (w / d, h / d))
    });
    if slop == Insets::ZERO {
        SLOP.with(|m| m.borrow_mut().remove(&(node as usize)));
        crate::node::reset(node, Attr::NODE_RESPONSE_REGION);
        return;
    }
    let size = size.unwrap_or((0.0, 0.0));
    SLOP.with(|m| m.borrow_mut().insert(node as usize, (slop, size)));
    apply_slop(node, slop, size);
}

/// A new frame size for `node` (vp): recompute its response region when it has a hit slop.
pub(crate) fn resized(node: Handle, w: f64, h: f64) {
    let slop = SLOP.with(|m| {
        let mut m = m.borrow_mut();
        let entry = m.get_mut(&(node as usize))?;
        (entry.1 != (w, h)).then(|| {
            entry.1 = (w, h);
            entry.0
        })
    });
    if let Some(slop) = slop {
        apply_slop(node, slop, (w, h));
    }
}

fn apply_slop(node: Handle, slop: Insets, (w, h): (f64, f64)) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let [x, y, pw, ph] =
        crate::paint::response_region([slop.top, slop.leading, slop.bottom, slop.trailing], w, h);
    crate::node::set_values(
        node,
        Attr::NODE_RESPONSE_REGION,
        &[
            crate::node::f32v(x),
            crate::node::f32v(y),
            crate::node::f32v(pw),
            crate::node::f32v(ph),
        ],
    );
}

/// Attach `gesture` to `node` reporting every phase to `receiver` against `id`, disposing it
/// when attachment fails (`false`). The recognizer stays with the node for its life.
unsafe fn attach(
    api: &ArkUI_NativeGestureAPI_1,
    node: Handle,
    gesture: *mut ArkUI_GestureRecognizer,
    id: u64,
    receiver: unsafe extern "C" fn(*mut ArkUI_GestureEvent, *mut c_void),
) -> bool {
    let mask = ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_END.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_CANCEL.0;
    let (Some(target), Some(add)) = (api.setGestureEventTarget, api.addGestureToNode) else {
        if let Some(dispose) = api.dispose {
            // SAFETY: a recognizer this module just created and attached nowhere.
            unsafe { dispose(gesture) };
        }
        return false;
    };
    crate::node::accept_input(node);
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
        !failed
    }
}

/// Attach a pinch or rotation `gesture` to `node` through the node's parallel group (created
/// on first use), reporting every phase to `receiver` against `id`. A group already on the
/// node is detached, given the new child, and attached again, so the node's recognizers are
/// rebuilt with it. Without a group API the gesture goes on alone, as `attach` would put it.
unsafe fn attach_two_finger(
    api: &ArkUI_NativeGestureAPI_1,
    node: Handle,
    gesture: *mut ArkUI_GestureRecognizer,
    id: u64,
    receiver: unsafe extern "C" fn(*mut ArkUI_GestureEvent, *mut c_void),
) {
    let (Some(create_group), Some(add_child), Some(target), Some(add)) = (
        api.createGroupGesture,
        api.addChildGesture,
        api.setGestureEventTarget,
        api.addGestureToNode,
    ) else {
        // SAFETY: forwarded under this function's own contract.
        unsafe { attach(api, node, gesture, id, receiver) };
        return;
    };
    crate::node::accept_input(node);
    let mask = ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_END.0
        | ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_CANCEL.0;
    let existing = TWO_FINGER.with(|m| m.borrow().get(&(node as usize)).copied());
    // SAFETY: the table's own functions with the argument types the header declares; the
    // group and its children stay with the node for its life, like the other recognizers.
    unsafe {
        let dispose = |g: *mut ArkUI_GestureRecognizer| {
            if let Some(dispose) = api.dispose {
                dispose(g);
            }
        };
        if target(gesture, mask, id as usize as *mut c_void, Some(receiver)) != 0 {
            dispose(gesture);
            return;
        }
        let group = if let Some(group) = existing {
            if let Some(remove) = api.removeGestureFromNode {
                remove(node, group.as_ptr());
            }
            group
        } else {
            let Some(group) = NonNull::new(create_group(ArkUI_GroupGestureMode::PARALLEL_GROUP))
            else {
                // No group: the gesture alone, its target already set.
                if add(
                    node,
                    gesture,
                    ArkUI_GesturePriority::NORMAL,
                    ArkUI_GestureMask::NORMAL_GESTURE_MASK,
                ) != 0
                {
                    dispose(gesture);
                }
                return;
            };
            TWO_FINGER.with(|m| m.borrow_mut().insert(node as usize, group));
            group
        };
        if add_child(group.as_ptr(), gesture) != 0 {
            dispose(gesture);
        }
        add(
            node,
            group.as_ptr(),
            ArkUI_GesturePriority::NORMAL,
            ArkUI_GestureMask::NORMAL_GESTURE_MASK,
        );
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
        attach_two_finger(api, node, pinch, id, pinch_receiver);
    }
}

unsafe extern "C" fn rotation_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    let id = extra as usize as u64;
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live gesture event, and its raw input, for the callback's duration.
        let (phase, angle, x, y) = unsafe {
            let phase = match OH_ArkUI_GestureEvent_GetActionType(ev) {
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT => 13.0,
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE => 14.0,
                _ => 15.0, // END or CANCEL
            };
            // The rotation event carries no center, so the location is the fingers' centroid
            // from the raw input, in px like the wire wants; ZERO when there is none.
            let input = OH_ArkUI_GestureEvent_GetRawInputEvent(ev);
            let count = if input.is_null() {
                0
            } else {
                OH_ArkUI_PointerEvent_GetPointerCount(input)
            };
            let (mut x, mut y) = (0.0f32, 0.0f32);
            for i in 0..count {
                x += OH_ArkUI_PointerEvent_GetXByIndex(input, i);
                y += OH_ArkUI_PointerEvent_GetYByIndex(input, i);
            }
            if count > 0 {
                x /= count as f32;
                y /= count as f32;
            }
            (phase, OH_ArkUI_RotationGesture_GetAngle(ev), x, y)
        };
        let total = ROTATIONS.with(|m| {
            let mut m = m.borrow_mut();
            let total = match (phase as i32, m.get(&id).copied()) {
                (13, _) | (_, None) => f64::from(angle),
                (_, Some((last, total))) => {
                    let step = f64::from(angle - last);
                    total + step - (step / 360.0).round() * 360.0
                }
            };
            if phase as i32 == 15 {
                m.remove(&id);
            } else {
                m.insert(id, (angle, total));
            }
            total
        });
        crate::events::rotate(id, phase, x, y, total.to_radians());
    });
}

/// Attach a native rotation recognizer to `node` (docs/canvas.md "Zoom and pan"): two
/// fingers, ArkUI's default 1° before it engages, in the same parallel group as the node's
/// pinch so a twist and a zoom run together. Phases 13/14/15 reach Rust against `id` with the
/// fingers' centroid in px and the angle in radians, clockwise positive and cumulative since
/// the fingers went down, as ArkUI reports it.
pub fn enable_rotation(node: Handle, id: u64) {
    let Some(api) = api() else {
        return;
    };
    if node.is_null() {
        return;
    }
    // SAFETY: as in `enable_pinch`.
    unsafe {
        let Some(create) = api.createRotationGesture else {
            return;
        };
        let rotation = create(2, 1.0);
        if rotation.is_null() {
            return;
        }
        attach_two_finger(api, node, rotation, id, rotation_receiver);
    }
}

unsafe extern "C" fn viewport_pan_receiver(ev: *mut ArkUI_GestureEvent, extra: *mut c_void) {
    let id = extra as usize as u64;
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live gesture event for the callback's duration.
        let (phase, x, y, ox, oy) = unsafe {
            let action = OH_ArkUI_GestureEvent_GetActionType(ev);
            let phase = match action {
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_ACCEPT => 7.0,
                ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_UPDATE => 8.0,
                _ => 9.0, // END or CANCEL
            };
            if action == ArkUI_GestureEventActionType::GESTURE_EVENT_ACTION_END {
                let d = crate::node::density();
                crate::emit(
                    NodeId(id),
                    Event::GestureVelocity {
                        kind: GestureKind::Pan,
                        velocity: Point::new(
                            f64::from(OH_ArkUI_PanGesture_GetVelocityX(ev)) / d,
                            f64::from(OH_ArkUI_PanGesture_GetVelocityY(ev)) / d,
                        ),
                    },
                );
            }
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
