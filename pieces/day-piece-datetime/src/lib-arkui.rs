// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// ArkUI (HarmonyOS): the NDK picker nodes, API 12+, through day-arkui's node module
// (docs/extending.md). Compact date = ARKUI_NODE_CALENDAR_PICKER (entry field → calendar popup);
// Inline date = ARKUI_NODE_DATE_PICKER wheels (native START/END bounds); time =
// ARKUI_NODE_TIME_PICKER wheels for both styles (the wheels are HarmonyOS's embedded time UI).
// A null node (SDK without picker nodes) falls back per docs. Measure rides day-arkui's generic
// `node::measure`, like the built-in leaves, with room for the wheels where the node is wheels.
//
// Payload notes straight from native_node.h: the wheels DATE_PICKER reports month 0–11 (the
// calendar picker and the SELECTED_DATE attribute use 1–12), normalized to 1–12 at this
// boundary; the wheels SELECTED/START/END attributes are "YYYY-M-D" strings. Each node's event
// user data carries the day NodeId, and the events arrive through an additive per-node
// receiver, so day-arkui's global receiver is untouched.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::arkui_sys::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, ArkUI_NodeType,
    OH_ArkUI_NodeEvent_GetEventType, OH_ArkUI_NodeEvent_GetNodeComponentEvent,
    OH_ArkUI_NodeEvent_GetUserData,
};
use day_arkui::node::{self, Handle};
use day_arkui::{AHandle, ArkUi};
use day_spec::{NodeId, Proposal, Size};

/// The height a wheels picker needs to show its wheels: the one day-arkui gives its built-in
/// picker wheel. The native measure of a picker node reports about one row, which is all a
/// wheel shows at that height: the selected value and nothing to scroll to.
const WHEELS_H: f64 = 200.0;

std::thread_local! {
    /// The date nodes built as wheels (the inline style); the rest are the compact field.
    static DATE_WHEELS: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The picker nodes' change events: a date (normalized to month 1–12) or a time.
unsafe extern "C" fn on_event(ev: *mut ArkUI_NodeEvent) {
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live node event; the component payload is ArkUI's for the callback.
        unsafe {
            let c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            if c.is_null() {
                return;
            }
            let data = &(*c).data;
            let id = NodeId(OH_ArkUI_NodeEvent_GetUserData(ev) as usize as u64);
            match OH_ArkUI_NodeEvent_GetEventType(ev) {
                // u32 year / month (1-12) / day
                Ev::NODE_CALENDAR_PICKER_EVENT_ON_CHANGE => {
                    emit_date(id, data[0].u32_ as i32, data[1].u32_ as i32, data[2].u32_ as i32);
                }
                // i32 year / month (0-11!) / day
                Ev::NODE_DATE_PICKER_EVENT_ON_DATE_CHANGE => {
                    emit_date(id, data[0].i32_, data[1].i32_ + 1, data[2].i32_);
                }
                // i32 hour / minute
                Ev::NODE_TIME_PICKER_EVENT_ON_CHANGE => {
                    if let Some(t) = DayTime::new(data[0].i32_ as u8, data[1].i32_ as u8, 0) {
                        day_arkui::emit(id, Event::custom("timepicker:value", t.to_string()));
                    }
                }
                _ => {}
            }
        }
    });
}

fn emit_date(id: NodeId, y: i32, m: i32, d: i32) {
    if let Some(date) = DayDate::new(y, m as u8, d as u8) {
        day_arkui::emit(id, Event::custom("datepicker:value", date.to_string()));
    }
}

fn set_calendar_date(n: Handle, d: DayDate) {
    node::set_values(
        n,
        Attr::NODE_CALENDAR_PICKER_SELECTED_DATE,
        &[
            node::u32v(d.year as u32),
            node::u32v(u32::from(d.month)),
            node::u32v(u32::from(d.day)),
        ],
    );
}

fn set_wheels_date(n: Handle, d: DayDate) {
    node::set_str(n, Attr::NODE_DATE_PICKER_SELECTED, &iso(d));
}

/// The wheels node's SELECTED/START/END attribute form ("YYYY-M-D").
fn iso(d: DayDate) -> String {
    format!("{}-{}-{}", d.year, d.month, d.day)
}

fn set_time(n: Handle, t: DayTime) {
    node::set_str(
        n,
        Attr::NODE_TIME_PICKER_SELECTED,
        &format!("{:02}:{:02}", t.hour, t.minute),
    );
}

/// A wheels picker: its native width, and room for its wheels.
fn measure_wheels(backend: &mut ArkUi, h: &AHandle, p: Proposal) -> Size {
    let natural = measure(backend, h, p);
    Size::new(natural.width, natural.height.max(WHEELS_H))
}

/// A date picker: the wheels when inline, the compact field's own size otherwise.
fn measure_date(backend: &mut ArkUi, h: &AHandle, p: Proposal) -> Size {
    if DATE_WHEELS.with(|w| w.borrow().contains(&(h.0 as usize))) {
        measure_wheels(backend, h, p)
    } else {
        measure(backend, h, p)
    }
}

fn measure(_backend: &mut ArkUi, h: &AHandle, p: Proposal) -> Size {
    let (w, hh) = node::measure(h.0, p.width.unwrap_or(-1.0), p.height.unwrap_or(-1.0));
    Size::new(w.max(60.0), hh.max(28.0))
}

mod date_renderer {
    use super::*;

    fn make(_backend: &mut ArkUi, p: &DateProps, id: NodeId) -> AHandle {
        if p.style == Style::Inline {
            let n = node::create(ArkUI_NodeType::ARKUI_NODE_DATE_PICKER);
            if n.is_null() {
                return AHandle(n); // unavailable on this SDK; day falls back per docs
            }
            // Native bounds (the wheels won't scroll outside them); none = the node default.
            if let Some(min) = p.min {
                node::set_str(n, Attr::NODE_DATE_PICKER_START, &iso(min));
            }
            if let Some(max) = p.max {
                node::set_str(n, Attr::NODE_DATE_PICKER_END, &iso(max));
            }
            set_wheels_date(n, p.date);
            node::register_event(n, Ev::NODE_DATE_PICKER_EVENT_ON_DATE_CHANGE, id.0);
            node::add_event_receiver(n, on_event);
            DATE_WHEELS.with(|w| w.borrow_mut().insert(n as usize));
            return AHandle(n);
        }
        // Compact: the calendar-picker entry field (no min/max attribute in the NDK; the
        // piece's own clamp bounds the value; docs/datepicker.md).
        let n = node::create(ArkUI_NodeType::ARKUI_NODE_CALENDAR_PICKER);
        if n.is_null() {
            return AHandle(n);
        }
        set_calendar_date(n, p.date);
        node::register_event(n, Ev::NODE_CALENDAR_PICKER_EVENT_ON_CHANGE, id.0);
        node::add_event_receiver(n, on_event);
        AHandle(n)
    }

    fn update(_backend: &mut ArkUi, h: &AHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        if h.0.is_null() {
            return;
        }
        if DATE_WHEELS.with(|w| w.borrow().contains(&(h.0 as usize))) {
            set_wheels_date(h.0, *d);
        } else {
            set_calendar_date(h.0, *d);
        }
    }

    /// Erase the wheels/calendar flag when the node goes away. Without this the set grows by
    /// one entry per realized date picker, and, worse, its key is the node's address, which
    /// the allocator reuses: a later picker landing on a freed address would inherit the dead
    /// entry's flag and be patched through the wrong attribute.
    fn release(_backend: &mut ArkUi, h: &AHandle) {
        DATE_WHEELS.with(|w| w.borrow_mut().remove(&(h.0 as usize)));
    }

    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure_date, release: release);
}

mod time_renderer {
    use super::*;

    fn make(_backend: &mut ArkUi, p: &TimeProps, id: NodeId) -> AHandle {
        // `seconds` is a documented no-op: the time wheels edit hours/minutes only.
        let n = node::create(ArkUI_NodeType::ARKUI_NODE_TIME_PICKER);
        if n.is_null() {
            return AHandle(n);
        }
        set_time(n, p.time);
        node::register_event(n, Ev::NODE_TIME_PICKER_EVENT_ON_CHANGE, id.0);
        node::add_event_receiver(n, on_event);
        AHandle(n)
    }

    fn update(_backend: &mut ArkUi, h: &AHandle, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        if !h.0.is_null() {
            set_time(h.0, *t);
        }
    }

    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure_wheels);
}
