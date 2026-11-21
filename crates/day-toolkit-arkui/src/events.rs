// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The global node-event receiver: every event a node registered reaches it, and it turns the
//! ArkUI payload into the shared bridge wire (`day_spec::bridge::BridgeKind`, the same table the
//! Android bridge uses) for [`crate::on_event`].

use day_spec::bridge::BridgeKind as K;
use ohos_sys::arkui::native_key_event::{
    ArkUI_KeyCode, ArkUI_KeyEventType, OH_ArkUI_KeyEvent_GetKeyCode, OH_ArkUI_KeyEvent_GetType,
    OH_ArkUI_KeyEvent_GetUnicode, OH_ArkUI_KeyEvent_SetConsumed,
};
use ohos_sys::arkui::native_node::{
    ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, OH_ArkUI_NodeEvent_GetEventType,
    OH_ArkUI_NodeEvent_GetInputEvent, OH_ArkUI_NodeEvent_GetNodeComponentEvent,
    OH_ArkUI_NodeEvent_GetNodeHandle, OH_ArkUI_NodeEvent_GetStringAsyncEvent,
    OH_ArkUI_NodeEvent_GetUserData, OH_ArkUI_NodeEvent_SetReturnNumberValue,
};
use ohos_sys::arkui::ui_input_event::{
    ArkUI_ModifierKeyName, ArkUI_UIInputEvent, OH_ArkUI_UIInputEvent_GetModifierKeyStates,
    OH_ArkUI_UIInputEvent_GetPressedKeys,
};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::paint::{KeyNote, KeysDown};

day_reactive::tls_slots! {
    events;
    // The keys each node holds down (ArkUI reports no repeat flag: a second down is the
    // auto-repeat) and the name each claimed down was reported under, for its release.
    static KEYS: RefCell<KeysDown> = RefCell::new(KeysDown::default());
    static KEY_NAMES: RefCell<HashMap<(u64, i32), String>> = RefCell::new(HashMap::new());
    // Scroll views that report offsets and phases (docs/scroll.md), by native node: whether
    // each is tracked, and the phase last sent.
    static SCROLLS: RefCell<HashMap<usize, (bool, bool, day_spec::ScrollPhase)>> =
        RefCell::new(HashMap::new());
}

/// Report scroll node `n`'s offsets (`ScrollChanged`) and user-driven phases (`ScrollPhase`)
/// as node `id`, for what the app observes; nothing is registered for neither.
pub fn track_scroll(n: crate::node::Handle, id: u64, offset: bool, phase: bool) {
    if !offset && !phase {
        return;
    }
    SCROLLS.with(|m| {
        m.borrow_mut()
            .insert(n as usize, (offset, phase, day_spec::ScrollPhase::Idle))
    });
    crate::node::register_event(n, Ev::NODE_SCROLL_EVENT_ON_SCROLL, id);
    crate::node::register_event(n, Ev::NODE_SCROLL_EVENT_ON_SCROLL_STOP, id);
    if phase {
        crate::node::register_event(n, Ev::NODE_SCROLL_EVENT_ON_WILL_SCROLL, id);
    }
}

/// Drop node `n`'s scroll tracking: it is being disposed.
pub fn forget(n: crate::node::Handle) {
    SCROLLS.with(|m| m.borrow_mut().remove(&(n as usize)));
}

/// A scroll event on tracked node `id` (`n` its native node): the offset after this frame,
/// and the phase the frame's state and source mean. `state` is `ArkUI_ScrollState` and `source`
/// `ArkUI_ScrollSource` from `ON_WILL_SCROLL`; `None` for the other events.
fn scrolled(n: crate::node::Handle, id: u64, kind: Ev, will: Option<(i32, i32)>) {
    use day_spec::ScrollPhase as P;
    let Some((offset, phase, last)) = SCROLLS.with(|m| m.borrow().get(&(n as usize)).copied())
    else {
        return;
    };
    let node = day_spec::NodeId(id);
    if offset && kind != Ev::NODE_SCROLL_EVENT_ON_WILL_SCROLL {
        // A float at rest on an edge can read back as -0.0001: that is the edge.
        let (x, y) = crate::node::scroll_offset(n);
        let edge = |v: f64| if v.abs() < 0.01 { 0.0 } else { v };
        crate::emit(
            node,
            day_spec::Event::ScrollChanged(day_spec::Point::new(edge(x), edge(y))),
        );
    }
    if !phase {
        return;
    }
    // User input only: a scroller (programmatic) or its animation sends no phase. States:
    // IDLE 0, SCROLL 1, FLING 2. Sources: DRAG 0, FLING 1, EDGE_EFFECT 2, OTHER_USER_INPUT 3,
    // SCROLL_BAR 4, SCROLL_BAR_FLING 5, SCROLLER 6, ANIMATION 7.
    let next = match (kind, will) {
        (Ev::NODE_SCROLL_EVENT_ON_SCROLL_STOP, _) => Some(P::Idle),
        (_, Some((1, 0 | 3 | 4))) => Some(P::Dragging),
        (_, Some((2 | 1, 1 | 2 | 5))) => Some(P::Coasting),
        _ => None,
    };
    if let Some(next) = next
        && next != last
    {
        SCROLLS.with(|m| {
            if let Some(entry) = m.borrow_mut().get_mut(&(n as usize)) {
                entry.2 = next;
            }
        });
        crate::emit(node, day_spec::Event::ScrollPhase(next));
    }
}

/// Bits 8 and 9 of a key's `num` on the wire: a repeat, a release (`KeyPhase::from_wire`).
const KEY_REPEAT: f64 = 256.0;
const KEY_UP: f64 = 512.0;

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
    let ctrl_or_alt = u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0)
        | u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_ALT.0);
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

/// A key on the full route (`Decorate::keyboard`, docs/menus.md "The full route"): the bridge
/// kind and the name or text to send. A key that types nothing is named by its web name; a
/// Ctrl or Alt chord names its key (no text comes under those, so the key's own character
/// names it, the letters cased by Shift, the punctuation keys by what they type unshifted on
/// the US layout); what is left is the text the event carries
/// (`OH_ArkUI_KeyEvent_GetUnicode`, which already reflects Shift). `None` for a lone
/// modifier. Hardware keyboards only: a canvas is not an input-method client on this
/// platform, and the docs say so.
unsafe fn full_route_key(
    input: *const ArkUI_UIInputEvent,
    code: ArkUI_KeyCode,
    held: u64,
) -> Option<(K, String)> {
    use ArkUI_KeyCode as Code;
    let named = match code {
        Code::ARKUI_KEYCODE_DPAD_LEFT => "ArrowLeft",
        Code::ARKUI_KEYCODE_DPAD_RIGHT => "ArrowRight",
        Code::ARKUI_KEYCODE_DPAD_UP => "ArrowUp",
        Code::ARKUI_KEYCODE_DPAD_DOWN => "ArrowDown",
        Code::ARKUI_KEYCODE_ENTER | Code::ARKUI_KEYCODE_NUMPAD_ENTER => "Enter",
        Code::ARKUI_KEYCODE_TAB => "Tab",
        Code::ARKUI_KEYCODE_ESCAPE => "Escape",
        Code::ARKUI_KEYCODE_DEL => "Backspace",
        Code::ARKUI_KEYCODE_FORWARD_DEL => "Delete",
        Code::ARKUI_KEYCODE_MOVE_HOME => "Home",
        Code::ARKUI_KEYCODE_MOVE_END => "End",
        Code::ARKUI_KEYCODE_PAGE_UP => "PageUp",
        Code::ARKUI_KEYCODE_PAGE_DOWN => "PageDown",
        Code::ARKUI_KEYCODE_INSERT => "Insert",
        _ => "",
    };
    if !named.is_empty() {
        return Some((K::Key, named.to_string()));
    }
    let (f1, f12) = (Code::ARKUI_KEYCODE_F1.0, Code::ARKUI_KEYCODE_F12.0);
    if (f1..=f12).contains(&code.0) {
        return day_spec::KeyEvent::function_name((code.0 - f1 + 1) as u8)
            .map(|name| (K::Key, name.to_string()));
    }
    let chord = u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0)
        | u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_ALT.0);
    if held & chord != 0 {
        let shift = held & u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_SHIFT.0) != 0;
        let (a, z) = (Code::ARKUI_KEYCODE_A.0, Code::ARKUI_KEYCODE_Z.0);
        let (zero, nine) = (Code::ARKUI_KEYCODE_0.0, Code::ARKUI_KEYCODE_9.0);
        let c = if (a..=z).contains(&code.0) {
            let letter = (b'a' + (code.0 - a) as u8) as char;
            if shift {
                letter.to_ascii_uppercase()
            } else {
                letter
            }
        } else if (zero..=nine).contains(&code.0) {
            (b'0' + (code.0 - zero) as u8) as char
        } else {
            match code {
                Code::ARKUI_KEYCODE_SPACE => ' ',
                Code::ARKUI_KEYCODE_LEFT_BRACKET => '[',
                Code::ARKUI_KEYCODE_RIGHT_BRACKET => ']',
                Code::ARKUI_KEYCODE_BACKSLASH => '\\',
                Code::ARKUI_KEYCODE_SEMICOLON => ';',
                Code::ARKUI_KEYCODE_APOSTROPHE => '\'',
                Code::ARKUI_KEYCODE_COMMA => ',',
                Code::ARKUI_KEYCODE_PERIOD => '.',
                Code::ARKUI_KEYCODE_SLASH => '/',
                Code::ARKUI_KEYCODE_GRAVE => '`',
                Code::ARKUI_KEYCODE_MINUS => '-',
                Code::ARKUI_KEYCODE_EQUALS => '=',
                _ => return None,
            }
        };
        return Some((K::Key, c.to_string()));
    }
    // SAFETY: `input` is the live key event the node event handed over, valid for the call.
    let unicode = unsafe { OH_ArkUI_KeyEvent_GetUnicode(input) };
    char::from_u32(unicode)
        .filter(|c| *c != '\0' && !c.is_control())
        .map(|c| (K::Text, c.to_string()))
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

/// A pinch on the gesture wire (phases 4/5/6): the center in px and the scale, with the
/// precision a smooth zoom needs.
pub fn pinch(id: u64, phase: f64, x: f32, y: f32, scale: f32) {
    crate::on_event(
        id,
        K::Gesture as i32,
        phase,
        &format!("{x:.2},{y:.2},{scale:.5},0"),
    );
}

/// A rotation on the gesture wire (phases 13/14/15): the centroid in px and the cumulative
/// angle in radians, clockwise positive.
pub fn rotate(id: u64, phase: f64, x: f32, y: f32, radians: f64) {
    crate::on_event(
        id,
        K::Gesture as i32,
        phase,
        &format!("{x:.2},{y:.2},{radians:.6},0"),
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
                        // A sectioned list's pinned header follows the visible range, and an
                        // observed viewport is reported.
                        let last = component(1).map_or(first.i32_, |last| last.i32_);
                        let list = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
                        crate::list::on_visible_rows(list, first.i32_, last);
                        crate::list::on_scroll_index(list, first.i32_, last);
                    }
                }
                // Registered on sectioned lists (the pin moves with every scrolled frame) and on
                // lists whose viewport the app observes.
                Ev::NODE_LIST_ON_DID_SCROLL => {
                    let list = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
                    crate::list::on_did_scroll(list);
                    crate::list::on_scrolled(list);
                }
                Ev::NODE_ON_CLICK => {
                    // A selectable list CELL's click is a row selection, not a press: resolved
                    // through the adapter's row map (cells carry no day node id).
                    let n = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
                    if let Some((label, target)) = crate::node::span_link(n) {
                        crate::on_event(label, K::LinkActivated as i32, 0.0, &target);
                    } else if crate::gesture::taps_grouped(id) {
                        // The node's tap group reports its single taps, after the double-tap
                        // interval (gesture.rs `enable_tap`).
                    } else if !crate::list::cell_click(n) {
                        // The click's point in the node, in vp: what a tap reports.
                        let (x, y) = (
                            component(0).map_or(0.0, |v| v.f32_),
                            component(1).map_or(0.0, |v| v.f32_),
                        );
                        crate::on_event(id, K::Pressed as i32, 0.0, &format!("{x},{y}"));
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
                // A tracked scroll view's frames, stop, and the state behind each frame. A
                // list that reports the reader's scrolls hears its stop too.
                Ev::NODE_SCROLL_EVENT_ON_SCROLL | Ev::NODE_SCROLL_EVENT_ON_SCROLL_STOP => {
                    let n = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
                    if kind == Ev::NODE_SCROLL_EVENT_ON_SCROLL_STOP {
                        crate::list::on_scroll_stop(n);
                    }
                    scrolled(n, id, kind, None);
                }
                // A list frame about to scroll, and what moves it (`ListSource::reader_scroll`):
                // data[0] the offset, data[2] the `ArkUI_ScrollSource`.
                Ev::NODE_LIST_ON_WILL_SCROLL => {
                    crate::list::on_will_scroll(
                        OH_ArkUI_NodeEvent_GetNodeHandle(ev),
                        component(0).map_or(0.0, |v| v.f32_),
                        component(2).map_or(-1, |v| v.i32_),
                    );
                }
                Ev::NODE_SCROLL_EVENT_ON_WILL_SCROLL => {
                    let state = component(2).map_or(0, |v| v.i32_);
                    let source = component(3).map_or(0, |v| v.i32_);
                    scrolled(
                        OH_ArkUI_NodeEvent_GetNodeHandle(ev),
                        id,
                        kind,
                        Some((state, source)),
                    );
                }
                // A wheel or touchpad tick over a node that asked for a pan or a pinch.
                Ev::NODE_ON_AXIS => {
                    crate::gesture::axis(id, OH_ArkUI_NodeEvent_GetInputEvent(ev));
                }
                Ev::NODE_ON_KEY_PRE_IME => {
                    let input = OH_ArkUI_NodeEvent_GetInputEvent(ev);
                    if !input.is_null() {
                        let mut held = 0;
                        OH_ArkUI_UIInputEvent_GetModifierKeyStates(input, &raw mut held);
                        // The API 18 pre-IME payload leaves modifierKeyState at zero, but
                        // supplies pressedKeyCodes. Read that list instead of tracking held
                        // keys across focus changes (which could leave a modifier stuck).
                        let mut pressed = [0; 32];
                        let mut len = pressed.len() as i32;
                        if OH_ArkUI_UIInputEvent_GetPressedKeys(
                            input,
                            pressed.as_mut_ptr(),
                            &raw mut len,
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
                                held |= u64::from(modifier);
                            }
                        }
                        let code = ArkUI_KeyCode(OH_ArkUI_KeyEvent_GetKeyCode(input));
                        if crate::area_can_submit(id)
                            && held == u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_SHIFT.0)
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
                            held & u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0) != 0;
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
                // Also the press, contact and input-kind reports of a node that asked for
                // them (gesture.rs); observed, never consumed.
                Ev::NODE_TOUCH_EVENT => {
                    crate::arm_toggle(id);
                    crate::gesture::touch(
                        OH_ArkUI_NodeEvent_GetNodeHandle(ev),
                        OH_ArkUI_NodeEvent_GetInputEvent(ev),
                    );
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
                Ev::NODE_ON_BLUR => {
                    // Keys held while focus leaves send no release to this node.
                    KEYS.with(|k| k.borrow_mut().clear(id));
                    KEY_NAMES.with(|m| m.borrow_mut().retain(|(node, _), _| *node != id));
                    crate::on_event(id, K::FocusChanged as i32, 0.0, "");
                }
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
                    if input.is_null() {
                        return;
                    }
                    let down = match OH_ArkUI_KeyEvent_GetType(input) {
                        ArkUI_KeyEventType::ARKUI_KEY_EVENT_DOWN => true,
                        ArkUI_KeyEventType::ARKUI_KEY_EVENT_UP => false,
                        _ => return,
                    };
                    let node = day_spec::NodeId(id);
                    if !day_spec::keys::handled(node) {
                        return;
                    }
                    let mut held = 0u64;
                    OH_ArkUI_UIInputEvent_GetModifierKeyStates(input, &raw mut held);
                    let code = ArkUI_KeyCode(OH_ArkUI_KeyEvent_GetKeyCode(input));
                    let mut mods = 0.0;
                    if held & u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_SHIFT.0) != 0 {
                        mods += 1.0; // Modifiers::SHIFT
                    }
                    if held & u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_CTRL.0) != 0 {
                        mods += 2.0 + 8.0; // PRIMARY, and the Control key itself
                    }
                    if held & u64::from(ArkUI_ModifierKeyName::ARKUI_MODIFIER_KEY_ALT.0) != 0 {
                        mods += 4.0; // ALT
                    }
                    // ArkUI's key events carry no repeat flag: a down for a key this node
                    // already holds is the auto-repeat, and only a claimed down's release is
                    // reported, under the name its down had and with no text.
                    let note = KEYS.with(|k| k.borrow_mut().note(id, code.0, down));
                    if note == KeyNote::Up {
                        if let Some(name) = KEY_NAMES.with(|m| m.borrow_mut().remove(&(id, code.0)))
                        {
                            crate::on_event(id, K::Key as i32, mods + KEY_UP, &name);
                            OH_ArkUI_KeyEvent_SetConsumed(input, true);
                        }
                        return;
                    }
                    if note == KeyNote::Stray {
                        return;
                    }
                    let claimed = if day_spec::keys::text(node) {
                        full_route_key(input, code, held)
                    } else {
                        key_name(code, held).map(|name| (K::Key, name.to_string()))
                    };
                    let Some((kind, name)) = claimed else {
                        KEYS.with(|k| k.borrow_mut().forget(id, code.0));
                        return;
                    };
                    let phase = if note == KeyNote::Repeat {
                        KEY_REPEAT
                    } else {
                        0.0
                    };
                    KEY_NAMES.with(|m| m.borrow_mut().insert((id, code.0), name.clone()));
                    crate::on_event(id, kind as i32, mods + phase, &name);
                    OH_ArkUI_KeyEvent_SetConsumed(input, true);
                }
                _ => {}
            }
        }
    });
}
