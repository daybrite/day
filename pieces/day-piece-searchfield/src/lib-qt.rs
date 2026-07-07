// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a QLineEdit with setClearButtonEnabled(true) and
// a leading magnifier action, behind a flat C ABI. textChanged dispatches Event::TextChanged;
// programmatic setText is wrapped in blockSignals so it never echoes back (like the picker shim).
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_search_new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_search_set_text(w: *mut c_void, text: *const c_char);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

extern "C" fn on_text(id: u64, text: *const c_char) {
    let s = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    day_qt::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Qt, p: &SearchProps, id: NodeId) -> QtHandle {
    QtHandle(unsafe {
        day_search_new(
            cstr(&p.placeholder).as_ptr(),
            cstr(&p.text).as_ptr(),
            id.0,
            on_text,
        )
    })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    unsafe { day_search_set_text(h.0, cstr(t).as_ptr()) };
}

fn measure(_backend: &mut Qt, h: &QtHandle, p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    // Grow to the proposed width; natural height from the line edit's size hint.
    let width = p.width.unwrap_or(w).max(120.0);
    Size::new(width, hh.max(24.0))
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
