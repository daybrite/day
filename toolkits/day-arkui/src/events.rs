// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The global node-event receiver: every event a node registered reaches it, and it turns the
//! ArkUI payload into the shared bridge wire (`day_spec::bridge::BridgeKind`, the same table the
//! Android bridge uses) for [`crate::on_event`].

use day_spec::bridge::BridgeKind as K;
use ohos_sys::arkui::native_key_event::{
    ArkUI_KeyCode, ArkUI_KeyEventType, OH_ArkUI_KeyEvent_GetKeyCode, OH_ArkUI_KeyEvent_GetType,
    OH_ArkUI_KeyEvent_SetConsumed,
};
use ohos_sys::arkui::native_node::{
    ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, OH_ArkUI_NodeEvent_GetEventType,
    OH_ArkUI_NodeEvent_GetInputEvent, OH_ArkUI_NodeEvent_GetNodeComponentEvent,
    OH_ArkUI_NodeEvent_GetNodeHandle, OH_ArkUI_NodeEvent_GetStringAsyncEvent,
    OH_ArkUI_NodeEvent_GetUserData,
};
use ohos_sys::arkui::ui_input_event::{
    ArkUI_ModifierKeyName, OH_ArkUI_UIInputEvent_GetModifierKeyStates,
};

/// The day name for a keycode the route carries, or `None` for every other key
/// (docs/menus.md). ArkUI names the arrows for a D-pad, which is the same four keys a
/// keyboard's arrows send. ArkUI draws no menu bar, so no accelerator owns the delete keys and
/// they ride the route too. A digit is the PHYSICAL key, main row or keypad (the keypad reads
/// as digits whatever Num Lock says: the query for it needs API 19), and never under Ctrl or
/// Alt, which belong to shortcuts.
fn key_name(code: ArkUI_KeyCode, held: u64) -> Option<&'static str> {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let name = match code {
        ArkUI_KeyCode::ARKUI_KEYCODE_DPAD_LEFT => Some("ArrowLeft"),
        ArkUI_KeyCode::ARKUI_KEYCODE_DPAD_RIGHT => Some("ArrowRight"),
        ArkUI_KeyCode::ARKUI_KEYCODE_DPAD_UP => Some("ArrowUp"),
        ArkUI_KeyCode::ARKUI_KEYCODE_DPAD_DOWN => Some("ArrowDown"),
        ArkUI_KeyCode::ARKUI_KEYCODE_DEL => Some("Backspace"),
        ArkUI_KeyCode::ARKUI_KEYCODE_FORWARD_DEL => Some("Delete"),
        _ => None,
    };
    if name.is_some() {
        return name;
    }
    let ctrl_or_alt = ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0 as u64
        | ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_ALT.0 as u64;
    if held & ctrl_or_alt != 0 {
        return None;
    }
    let (zero, nine) = (
        ArkUI_KeyCode::ARKUI_KEYCODE_0.0,
        ArkUI_KeyCode::ARKUI_KEYCODE_9.0,
    );
    if (zero..=nine).contains(&code.0) {
        return Some(DIGITS[(code.0 - zero) as usize]);
    }
    let (pad0, pad9) = (
        ArkUI_KeyCode::ARKUI_KEYCODE_NUMPAD_0.0,
        ArkUI_KeyCode::ARKUI_KEYCODE_NUMPAD_9.0,
    );
    if (pad0..=pad9).contains(&code.0) {
        return Some(DIGITS[(code.0 - pad0) as usize]);
    }
    None
}

/// Install the receiver. Called once at init.
pub fn install() {
    crate::node::register_event_receiver(receiver);
}

/// A pan phase from the gesture recognizer (docs/shapes.md): location and translation in px,
/// on the shared gesture wire.
pub fn gesture(id: u64, phase: f64, x: f32, y: f32, tx: f32, ty: f32) {
    crate::on_event(
        id,
        K::Gesture as i32,
        phase,
        &format!("{x:.2},{y:.2},{tx:.2},{ty:.2}"),
    );
}

unsafe extern "C" fn receiver(ev: *mut ArkUI_NodeEvent) {
    if ev.is_null() {
        return;
    }
    day_spec::ffi_guard::contain((), || {
        if crate::transfer::event(ev) {
            return;
        }
        // SAFETY: a live event for the callback's duration; the accessors below are ArkUI's.
        unsafe {
            let id = OH_ArkUI_NodeEvent_GetUserData(ev) as usize as u64;
            let kind = OH_ArkUI_NodeEvent_GetEventType(ev);
            let component =
                |slot: usize| -> Option<ohos_sys::arkui::native_type::ArkUI_NumberValue> {
                    let c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
                    if c.is_null() {
                        None
                    } else {
                        (*c).data.get(slot).copied()
                    }
                };
            let string = || -> String {
                let s = OH_ArkUI_NodeEvent_GetStringAsyncEvent(ev);
                if s.is_null() {
                    String::new()
                } else {
                    crate::node::text_of((*s).pStr)
                }
            };
            match kind {
                Ev::NODE_ON_CLICK => {
                    // A selectable list CELL's click is a row selection, not a press: resolved
                    // through the adapter's row map (cells carry no day node id).
                    let n = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
                    if !crate::list::cell_click(n) {
                        crate::on_event(id, K::Pressed as i32, 0.0, "");
                    }
                }
                // Drag-to-reorder (docs/list.md).
                Ev::NODE_ON_DRAG_START => crate::list::on_drag_start(ev),
                Ev::NODE_ON_DROP => crate::list::on_drop(ev),
                Ev::NODE_TEXT_INPUT_ON_CHANGE | Ev::NODE_TEXT_AREA_ON_CHANGE => {
                    crate::on_event(id, K::TextChanged as i32, 0.0, &string());
                }
                Ev::NODE_TOGGLE_ON_CHANGE => {
                    let on = component(0).map_or(0.0, |v| f64::from(v.i32_));
                    crate::on_event(id, K::ToggleChanged as i32, on, "");
                }
                Ev::NODE_SLIDER_EVENT_ON_CHANGE => {
                    // data[0].f32 is the value; data[1].i32 is the state that triggered the
                    // event, ArkTS's SliderChangeMode (Begin 0, Moving 1, End 2, Click 3). ArkUI
                    // is the one toolkit that hands the phase over directly, so the settled
                    // value needs no tracking flag: End ends a drag, Click is a jump to a point
                    // on the track and is already settled. The enum has no C name.
                    let value = component(0).map_or(0.0, |v| f64::from(v.f32_));
                    let mode = component(1).map_or(1, |v| v.i32_);
                    crate::on_event(id, K::ValueChanged as i32, value, "");
                    if mode == 2 || mode == 3 {
                        crate::on_event(id, K::ValueCommitted as i32, value, "");
                    }
                }
                Ev::NODE_SWIPER_EVENT_ON_CHANGE => {
                    let index = component(0).map_or(0.0, |v| f64::from(v.i32_));
                    crate::on_event(id, K::SelectionChanged as i32, index, "");
                }
                Ev::NODE_TEXT_PICKER_EVENT_ON_CHANGE => {
                    let index = component(0).map_or(0.0, |v| f64::from(v.f32_));
                    crate::on_event(id, K::SelectionChanged as i32, index, "");
                }
                // Focus pair + text-input submit (docs/focus.md).
                Ev::NODE_ON_FOCUS => crate::on_event(id, K::FocusChanged as i32, 1.0, ""),
                Ev::NODE_ON_BLUR => crate::on_event(id, K::FocusChanged as i32, 0.0, ""),
                Ev::NODE_TEXT_INPUT_ON_SUBMIT => {
                    crate::on_event(id, K::Submitted as i32, 0.0, "");
                }
                // The non-text keys, for a focused node whose app asked for them
                // (docs/menus.md). Consumed only when claimed: an unclaimed arrow keeps
                // propagating, so ArkUI's own focus walking still moves between components.
                Ev::NODE_ON_KEY_EVENT => {
                    let input = OH_ArkUI_NodeEvent_GetInputEvent(ev);
                    if input.is_null()
                        || OH_ArkUI_KeyEvent_GetType(input)
                            != ArkUI_KeyEventType::ARKUI_KEY_EVENT_DOWN
                    {
                        return;
                    }
                    let mut held = 0u64;
                    OH_ArkUI_UIInputEvent_GetModifierKeyStates(input, &mut held);
                    let code = ArkUI_KeyCode(OH_ArkUI_KeyEvent_GetKeyCode(input));
                    let Some(name) = key_name(code, held) else {
                        return;
                    };
                    if !day_spec::keys::handled(day_spec::NodeId(id)) {
                        return;
                    }
                    let mut mods = 0.0;
                    if held & ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_SHIFT.0 as u64 != 0 {
                        mods += 1.0; // day KeyEvent::SHIFT
                    }
                    if held & ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0 as u64 != 0 {
                        mods += 2.0; // PRIMARY
                    }
                    if held & ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_ALT.0 as u64 != 0 {
                        mods += 4.0; // ALT
                    }
                    crate::on_event(id, K::Key as i32, mods, name);
                    OH_ArkUI_KeyEvent_SetConsumed(input, true);
                }
                _ => {}
            }
        }
    });
}
