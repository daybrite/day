// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) — a swatch `Button` whose
// `Flyout` holds the system `ColorPicker`. Components cross the flat C ABI as four doubles;
// XAML's `Windows.UI.Color` is 8-bit, so a pick made here comes back quantized (docs/colorpicker.md).
// Windows-only, built in CI, NOT verified locally.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_colorpicker_xaml_new(
        r: f64,
        g: f64,
        b: f64,
        a: f64,
        with_alpha: c_int,
        title: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, f64, f64, f64, f64),
    ) -> *mut c_void;
    fn day_colorpicker_xaml_set(h: *mut c_void, r: f64, g: f64, b: f64, a: f64);
    // Generic size hint from day-xaml-sys (already linked).
    fn day_xaml_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

extern "C" fn on_pick(id: u64, r: f64, g: f64, b: f64, a: f64) {
    day_xaml::emit(
        NodeId(id),
        Event::custom(PICK_TAG, Color::rgba(r, g, b, a).to_string()),
    );
}

fn make(_backend: &mut Xaml, p: &ColorProps, id: NodeId) -> WinHandle {
    let title = CString::new(p.title.as_str()).unwrap_or_default();
    WinHandle(unsafe {
        day_colorpicker_xaml_new(
            p.color.r,
            p.color.g,
            p.color.b,
            p.color.a,
            p.alpha as c_int,
            title.as_ptr(),
            id.0,
            on_pick,
        )
    })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &ColorPatch) {
    let ColorPatch::SetColor(c) = patch;
    unsafe { day_colorpicker_xaml_set(h.0, c.r, c.g, c.b, c.a) };
}

fn measure(_backend: &mut Xaml, h: &WinHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_xaml_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w.max(88.0), hh.max(32.0))
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: ColorProps, patch: ColorPatch,
    make: make, update: update, measure: measure);
