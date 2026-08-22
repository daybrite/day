// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — QComboBox / checkable QPushButtons /
// QRadioButtons, one DayPicker widget per style behind a flat C ABI.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use crate::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_picker_new(
        style: c_int,
        items_joined: *const c_char,
        selected: c_int,
        id: u64,
        cb: extern "C" fn(u64, c_int),
    ) -> *mut c_void;
    fn day_picker_set_selected(w: *mut c_void, idx: c_int);
    fn day_picker_set_options(w: *mut c_void, items_joined: *const c_char);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

extern "C" fn on_select(id: u64, idx: c_int) {
    // Contained: a panic unwinding into the C++ shim frame is UB (day-spec's ffi_guard).
    day_spec::ffi_guard::contain((), || {
        crate::emit(NodeId(id), Event::SelectionChanged(idx as i64))
    });
}

fn joined(items: &[String]) -> CString {
    // crate::cstr strips interior NULs, so one bad option can't blank the whole list.
    crate::cstr(&items.join("\n"))
}

fn style_code(s: PickerStyle) -> c_int {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut Qt, p: &PickerProps, id: NodeId) -> QtHandle {
    QtHandle(unsafe {
        day_picker_new(
            style_code(p.style),
            joined(&p.options).as_ptr(),
            p.selected as c_int,
            id.0,
            on_select,
        )
    })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &PickerPatch) {
    match patch {
        PickerPatch::Selected(i) => unsafe { day_picker_set_selected(h.0, *i as c_int) },
        PickerPatch::Options(opts) => unsafe { day_picker_set_options(h.0, joined(opts).as_ptr()) },
    }
}

fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    Size::new(w.max(60.0), hh.max(22.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Qt,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    match day_spec::props_of::<PickerProps>(day_spec::kinds::PICKER, "qt", props) {
        Some(p) => make(b, p, id),
        None => crate::placeholder_handle(day_spec::kinds::PICKER),
    }
}

pub(crate) fn update_any(b: &mut crate::Qt, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut crate::Qt,
    h: &crate::Handle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
