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
    OH_ArkUI_NodeEvent_GetUserData, OH_ArkUI_NodeEvent_SetReturnNumberValue,
};
use ohos_sys::arkui::ui_input_event::{
    ArkUI_ModifierKeyName, OH_ArkUI_UIInputEvent_GetModifierKeyStates,
    OH_ArkUI_UIInputEvent_GetPressedKeys,
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
                Ev::NODE_LIST_ON_SCROLL_INDEX => {
                    if let Some(first) = component(0) {
                        crate::on_event(
                            id,
                            K::Custom as i32,
                            f64::from(first.i32_.max(0)),
                            "day-list-first-visible",
                        );
                    }
                }
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
                    let text = string();
                    // A read-only field's change is put back, never reported.
                    if !crate::hold_read_only(OH_ArkUI_NodeEvent_GetNodeHandle(ev), id, &text) {
                        crate::on_event(id, K::TextChanged as i32, 0.0, &text);
                    }
                }
                // A field that has been read-only asks before each edit (docs/textfield.md).
                // The answer is always written: ArkUI reads the slot whether or not it was set.
                Ev::NODE_TEXT_INPUT_ON_WILL_INSERT
                | Ev::NODE_TEXT_INPUT_ON_WILL_DELETE
                | Ev::NODE_TEXT_AREA_ON_WILL_INSERT
                | Ev::NODE_TEXT_AREA_ON_WILL_DELETE => {
                    let mut allow = [crate::node::i32v(i32::from(!crate::is_read_only(id)))];
                    OH_ArkUI_NodeEvent_SetReturnNumberValue(ev, allow.as_mut_ptr(), 1);
                }
                Ev::NODE_TEXT_AREA_ON_TEXT_SELECTION_CHANGE => {
                    if crate::area_unselectable(id)
                        && let (Some(start), Some(end)) = (component(0), component(1))
                        && start.i32_ != end.i32_
                    {
                        crate::node::collapse_textarea_selection(
                            OH_ArkUI_NodeEvent_GetNodeHandle(ev),
                            end.i32_,
                        );
                    }
                }
                Ev::NODE_ON_KEY_PRE_IME => {
                    let input = OH_ArkUI_NodeEvent_GetInputEvent(ev);
                    if !input.is_null() {
                        let mut held = 0;
                        OH_ArkUI_UIInputEvent_GetModifierKeyStates(input, &mut held);
                        // The API 18 pre-IME payload leaves modifierKeyState at zero, but
                        // supplies pressedKeyCodes. Read that list instead of tracking held
                        // keys across focus changes (which could leave a modifier stuck).
                        let mut pressed = [0; 32];
                        let mut len = pressed.len() as i32;
                        if OH_ArkUI_UIInputEvent_GetPressedKeys(
                            input,
                            pressed.as_mut_ptr(),
                            &mut len,
                        ) == 0
                        {
                            for &key in pressed.iter().take(len.max(0) as usize) {
                                let modifier = match ArkUI_KeyCode(key) {
                                    ArkUI_KeyCode::ARKUI_KEYCODE_SHIFT_LEFT
                                    | ArkUI_KeyCode::ARKUI_KEYCODE_SHIFT_RIGHT => {
                                        ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_SHIFT.0
                                    }
                                    ArkUI_KeyCode::ARKUI_KEYCODE_CTRL_LEFT
                                    | ArkUI_KeyCode::ARKUI_KEYCODE_CTRL_RIGHT => {
                                        ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0
                                    }
                                    ArkUI_KeyCode::ARKUI_KEYCODE_ALT_LEFT
                                    | ArkUI_KeyCode::ARKUI_KEYCODE_ALT_RIGHT => {
                                        ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_ALT.0
                                    }
                                    _ => 0,
                                };
                                held |= modifier as u64;
                            }
                        }
                        let code = ArkUI_KeyCode(OH_ArkUI_KeyEvent_GetKeyCode(input));
                        if crate::area_can_submit(id)
                            && held == ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_SHIFT.0 as u64
                            && matches!(
                                code,
                                ArkUI_KeyCode::ARKUI_KEYCODE_ENTER
                                    | ArkUI_KeyCode::ARKUI_KEYCODE_NUMPAD_ENTER
                            )
                        {
                            // Consume before editing: an app may remove this node in its
                            // TextChanged handler. Only key-down inserts, never key-up.
                            OH_ArkUI_KeyEvent_SetConsumed(input, true);
                            if OH_ArkUI_KeyEvent_GetType(input)
                                == ArkUI_KeyEventType::ARKUI_KEY_EVENT_DOWN
                            {
                                crate::area_newline(OH_ArkUI_NodeEvent_GetNodeHandle(ev), id);
                            }
                            return;
                        }
                        // Copy/Cut must not race an asynchronous selection-collapse event.
                        let ctrl =
                            held & ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0 as u64 != 0;
                        if crate::area_unselectable(id)
                            && ctrl
                            && matches!(
                                code,
                                ArkUI_KeyCode::ARKUI_KEYCODE_A
                                    | ArkUI_KeyCode::ARKUI_KEYCODE_C
                                    | ArkUI_KeyCode::ARKUI_KEYCODE_X
                            )
                        {
                            OH_ArkUI_KeyEvent_SetConsumed(input, true);
                        }
                    }
                }
                // A touch on a switch arms its gate (`TOGGLE_GATE`); only switches register it.
                Ev::NODE_TOUCH_EVENT => crate::arm_toggle(id),
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
                Ev::NODE_TEXT_AREA_ON_SUBMIT => {
                    if crate::area_can_submit(id) {
                        crate::on_event(id, K::Submitted as i32, 0.0, "");
                    }
                }
                // The non-text keys, for a focused node whose app asked for them
                // (docs/menus.md). Consumed only when claimed: an unclaimed arrow keeps
                // propagating, so ArkUI's own focus walking still moves between components.
                Ev::NODE_ON_KEY_EVENT => {
                    // A key on a focused switch arms its gate, like a touch.
                    crate::arm_toggle(id);
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
