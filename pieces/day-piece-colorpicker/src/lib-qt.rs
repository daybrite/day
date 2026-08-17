// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a swatch `QPushButton` that opens
// `QColorDialog`, Qt's real chooser. Components cross the flat C ABI as four doubles, so nothing
// is quantized on the way (both `QColor::getRgbF` and Day's `Color` are float).
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_colorpicker_new(
        r: f64,
        g: f64,
        b: f64,
        a: f64,
        with_alpha: c_int,
        title: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, f64, f64, f64, f64),
    ) -> *mut c_void;
    fn day_colorpicker_set(w: *mut c_void, r: f64, g: f64, b: f64, a: f64);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

extern "C" fn on_pick(id: u64, r: f64, g: f64, b: f64, a: f64) {
    day_qt::emit(
        NodeId(id),
        Event::custom(PICK_TAG, Color::rgba(r, g, b, a).to_string()),
    );
}

fn make(_backend: &mut Qt, p: &ColorProps, id: NodeId) -> QtHandle {
    let title = CString::new(p.title.as_str()).unwrap_or_default();
    QtHandle(unsafe {
        day_colorpicker_new(
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

fn update(_backend: &mut Qt, h: &QtHandle, patch: &ColorPatch) {
    let ColorPatch::SetColor(c) = patch;
    unsafe { day_colorpicker_set(h.0, c.r, c.g, c.b, c.a) };
}

fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    Size::new(w.max(72.0), hh.max(24.0))
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: ColorProps, patch: ColorPatch,
    make: make, update: update, measure: measure);
