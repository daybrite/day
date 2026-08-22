// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) — ComboBox / RadioButton StackPanels,
// boxed into Day handles via the `day_xaml_box`/`day_xaml_unbox` seam day-xaml-sys exports. This
// mirrors the Qt renderer (own shim for the control; reuse the sys crate's generic measure).
// Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};
use std::os::raw::{c_char, c_int, c_void};

use crate::{WinHandle, Xaml};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_picker_xaml_new(
        style: c_int,
        items_joined: *const c_char,
        selected: c_int,
        id: u64,
        cb: extern "C" fn(u64, c_int),
    ) -> *mut c_void;
    fn day_picker_xaml_set_selected(w: *mut c_void, idx: c_int);
    fn day_picker_xaml_set_options(w: *mut c_void, items_joined: *const c_char);
    // Generic size hint from day-xaml-sys (already linked) — like the Qt renderer reusing
    // day-qt-sys's `day_qt_size_hint`.
    fn day_xaml_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

extern "C" fn on_select(id: u64, idx: c_int) {
    // Contained: a panic unwinding into the C++/WinRT shim frame is UB (day-spec's ffi_guard).
    day_spec::ffi_guard::contain((), || {
        crate::emit(NodeId(id), Event::SelectionChanged(idx as i64))
    });
}

fn style_code(s: PickerStyle) -> c_int {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut Xaml, p: &PickerProps, id: NodeId) -> WinHandle {
    // crate::cstr strips interior NULs, so one bad option can't blank the whole list.
    let joined = crate::cstr(&p.options.join("\n"));
    WinHandle(unsafe {
        day_picker_xaml_new(
            style_code(p.style),
            joined.as_ptr(),
            p.selected as c_int,
            id.0,
            on_select,
        )
    })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &PickerPatch) {
    match patch {
        PickerPatch::Selected(i) => unsafe { day_picker_xaml_set_selected(h.0, *i as c_int) },
        PickerPatch::Options(opts) => {
            let joined = crate::cstr(&opts.join("\n"));
            unsafe { day_picker_xaml_set_options(h.0, joined.as_ptr()) };
        }
    }
}

fn measure(_backend: &mut Xaml, h: &WinHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_xaml_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w.max(120.0), hh.max(32.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Xaml,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::WinHandle {
    match day_spec::props_of::<PickerProps>(day_spec::kinds::PICKER, "xaml", props) {
        Some(p) => make(b, p, id),
        None => crate::placeholder_handle(day_spec::kinds::PICKER),
    }
}

pub(crate) fn update_any(b: &mut crate::Xaml, h: &crate::WinHandle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut crate::Xaml,
    h: &crate::WinHandle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
