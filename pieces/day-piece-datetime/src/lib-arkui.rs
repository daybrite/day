// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// ArkUI (HarmonyOS): the NDK picker nodes via this crate's shim (src/datetime-arkui.cpp,
// compiled by build.rs against OHOS_NDK_HOME, the pullrefresh pattern). Compact date =
// ARKUI_NODE_CALENDAR_PICKER (entry → calendar popup); Inline date = ARKUI_NODE_DATE_PICKER
// wheels (native START/END bounds); time = ARKUI_NODE_TIME_PICKER wheels for both styles (the
// wheels are HarmonyOS's embedded time UI). A null node (SDK without picker nodes) falls back
// per docs. Measure rides day-arkui-sys's generic day_ark_measure, like the built-in leaves,
// with room for the wheels where the node is wheels.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi};
use day_spec::{NodeId, Proposal, Size};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_longlong, c_void};

unsafe extern "C" {
    fn day_dtp_date_new(
        id: c_longlong,
        inline_style: c_int,
        y: c_int,
        m: c_int,
        d: c_int,
        min_iso: *const c_char,
        max_iso: *const c_char,
        cb: extern "C" fn(c_longlong, c_int, c_int, c_int),
    ) -> *mut c_void;
    fn day_dtp_date_set(node: *mut c_void, y: c_int, m: c_int, d: c_int);
    fn day_dtp_date_release(node: *mut c_void);
    fn day_dtp_time_new(
        id: c_longlong,
        hour: c_int,
        minute: c_int,
        cb: extern "C" fn(c_longlong, c_int, c_int),
    ) -> *mut c_void;
    fn day_dtp_time_set(node: *mut c_void, hour: c_int, minute: c_int);
    // From day-arkui-sys (already linked into the binary): native measure for a leaf node.
    fn day_ark_measure(n: *mut c_void, max_w: f64, max_h: f64, out_w: *mut f64, out_h: *mut f64);
}

/// The height a wheels picker needs to show its wheels: the one day-arkui gives its built-in
/// picker wheel. The native measure of a picker node reports about one row, which is all a
/// wheel shows at that height: the selected value and nothing to scroll to.
const WHEELS_H: f64 = 200.0;

std::thread_local! {
    /// The date nodes built as wheels (the inline style); the rest are the compact field.
    static DATE_WHEELS: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
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
    let (mut w, mut hh) = (0.0f64, 0.0f64);
    unsafe {
        day_ark_measure(
            h.0,
            p.width.unwrap_or(-1.0),
            p.height.unwrap_or(-1.0),
            &mut w,
            &mut hh,
        )
    };
    Size::new(w.max(60.0), hh.max(28.0))
}

/// The wheels node's START/END attribute form ("YYYY-M-D"); empty = no bound.
fn bound_cstring(d: Option<DayDate>) -> CString {
    CString::new(d.map_or(String::new(), |d| {
        format!("{}-{}-{}", d.year, d.month, d.day)
    }))
    .unwrap_or_default()
}

mod date_renderer {
    use super::*;

    /// Shim → Day: a pick (normalized to month 1–12 at the C++ boundary).
    extern "C" fn on_date(id: c_longlong, y: c_int, m: c_int, d: c_int) {
        if let Some(date) = DayDate::new(y, m as u8, d as u8) {
            day_arkui::emit(
                NodeId(id as u64),
                Event::custom("datepicker:value", date.to_string()),
            );
        }
    }

    fn make(_backend: &mut ArkUi, p: &DateProps, id: NodeId) -> AHandle {
        let min = bound_cstring(p.min);
        let max = bound_cstring(p.max);
        let h = AHandle(unsafe {
            day_dtp_date_new(
                id.0 as c_longlong,
                (p.style == Style::Inline) as c_int,
                p.date.year,
                p.date.month as c_int,
                p.date.day as c_int,
                min.as_ptr(),
                max.as_ptr(),
                on_date,
            )
        });
        if p.style == Style::Inline && !h.0.is_null() {
            DATE_WHEELS.with(|w| w.borrow_mut().insert(h.0 as usize));
        }
        h
    }

    fn update(_backend: &mut ArkUi, h: &AHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        unsafe { day_dtp_date_set(h.0, d.year, d.month as c_int, d.day as c_int) };
    }

    /// Erase the shim's wheels/calendar flag when the node goes away (see day_dtp_date_release
    /// in datetime-arkui.cpp for the address-reuse hazard it closes).
    fn release(_backend: &mut ArkUi, h: &AHandle) {
        // SAFETY: h.0 is the node this shim created (or null on the documented fallback path,
        // which the shim guards); called once, just before day-arkui disposes the node.
        unsafe { day_dtp_date_release(h.0) };
        DATE_WHEELS.with(|w| w.borrow_mut().remove(&(h.0 as usize)));
    }

    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure_date, release: release);
}

mod time_renderer {
    use super::*;

    extern "C" fn on_time(id: c_longlong, hour: c_int, minute: c_int) {
        if let Some(t) = DayTime::new(hour as u8, minute as u8, 0) {
            day_arkui::emit(
                NodeId(id as u64),
                Event::custom("timepicker:value", t.to_string()),
            );
        }
    }

    fn make(_backend: &mut ArkUi, p: &TimeProps, id: NodeId) -> AHandle {
        // `seconds` is a documented no-op: the time wheels edit hours/minutes only.
        AHandle(unsafe {
            day_dtp_time_new(
                id.0 as c_longlong,
                p.time.hour as c_int,
                p.time.minute as c_int,
                on_time,
            )
        })
    }

    fn update(_backend: &mut ArkUi, h: &AHandle, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        unsafe { day_dtp_time_set(h.0, t.hour as c_int, t.minute as c_int) };
    }

    day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure_wheels);
}
