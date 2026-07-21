// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) — an EDITABLE ComboBox
// (IsEditable, Windows 10 1809+), the platform's real combo box, boxed into a Day handle via
// the day_winui_box/unbox seam that day-winui-sys exports. Windows-only, built in CI, not
// verified locally. Documented divergence: XAML's ComboBox has no per-keystroke text event, so
// free-form text reports on Enter or focus loss (TextSubmitted / LostFocus); picking an item
// reports immediately (SelectionChanged). All paths arrive here as Event::TextChanged.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_winui::{WinHandle, WinUi};

unsafe extern "C" {
    fn day_combo_winui_new(
        items_joined: *const c_char,
        text: *const c_char,
        placeholder: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_combo_winui_set_items(w: *mut c_void, items_joined: *const c_char);
    fn day_combo_winui_set_text(w: *mut c_void, text: *const c_char);
    // Generic size hint from day-winui-sys (already linked), like the Qt renderer reusing
    // day-qt-sys's day_qt_size_hint.
    fn day_winui_measure(
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
    day_winui::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn joined(items: &[String]) -> CString {
    cstr(&items.join("\n"))
}

fn make(_backend: &mut WinUi, p: &ComboProps, id: NodeId) -> WinHandle {
    WinHandle(unsafe {
        day_combo_winui_new(
            joined(&p.items).as_ptr(),
            cstr(&p.text).as_ptr(),
            cstr(&p.placeholder).as_ptr(),
            id.0,
            on_text,
        )
    })
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &ComboPatch) {
    unsafe {
        match patch {
            ComboPatch::Items(items) => day_combo_winui_set_items(h.0, joined(items).as_ptr()),
            ComboPatch::SetText(t) => day_combo_winui_set_text(h.0, cstr(t).as_ptr()),
        }
    }
}

fn measure(_backend: &mut WinUi, h: &WinHandle, p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_winui_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    let width = p.width.unwrap_or(w).max(120.0);
    Size::new(width, hh.max(32.0))
}

day_pieces::renderer!(day_winui::RENDERERS, WinUi,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
