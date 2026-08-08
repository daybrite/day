// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) — an AutoSuggestBox (the XAML
// search control, with a query magnifier) boxed into a Day handle via the day_xaml_box/unbox seam
// that day-xaml-sys exports. This mirrors the picker/media XAML renderers (own shim for the
// control; reuse the sys crate's generic measure). Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_search_xaml_new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_search_xaml_set_text(w: *mut c_void, text: *const c_char);
    // Generic size hint from day-xaml-sys (already linked), like the Qt renderer reusing
    // day-qt-sys's day_qt_size_hint.
    fn day_xaml_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

extern "C" fn on_text(id: u64, text: *const c_char) {
    let s = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    day_xaml::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Xaml, p: &SearchProps, id: NodeId) -> WinHandle {
    WinHandle(unsafe {
        day_search_xaml_new(
            cstr(&p.placeholder).as_ptr(),
            cstr(&p.text).as_ptr(),
            id.0,
            on_text,
        )
    })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    unsafe { day_search_xaml_set_text(h.0, cstr(t).as_ptr()) };
}

fn measure(_backend: &mut Xaml, h: &WinHandle, p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_xaml_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    let width = p.width.unwrap_or(w).max(160.0);
    Size::new(width, hh.max(32.0))
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
