// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) — a multi-line TextBox (AcceptsReturn
// = true, TextWrapping = Wrap, a native PlaceholderText) boxed into a Day handle via the
// day_winui_box/unbox seam that day-winui-sys exports. This mirrors the searchfield WinUI renderer (own
// shim for the control; reuse the sys crate's generic measure), then clamps the natural height to the
// [min_lines, max_lines] band with an approximate line height. Windows-only, built in CI, not verified
// locally.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_winui::{WinHandle, WinUi};

// Approximate line height (device-independent px) for the min/max-lines clamp — WinUI has no cheap exact
// per-control line metric here, and this backend is best-effort.
const LINE_H: f64 = 20.0;
const PAD: f64 = 12.0;

unsafe extern "C" {
    fn day_textarea_winui_new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_textarea_winui_set_text(w: *mut c_void, text: *const c_char);
    // Generic size hint from day-winui-sys (already linked), like the searchfield renderer.
    fn day_winui_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

thread_local! {
    // The line band per handle — `measure` gets no props, so remember min/max lines from `make`.
    static DIMS: RefCell<HashMap<usize, (u32, u32)>> = RefCell::new(HashMap::new());
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

fn make(_backend: &mut WinUi, p: &TextProps, id: NodeId) -> WinHandle {
    let ptr = unsafe {
        day_textarea_winui_new(
            cstr(&p.placeholder).as_ptr(),
            cstr(&p.text).as_ptr(),
            id.0,
            on_text,
        )
    };
    DIMS.with(|m| {
        m.borrow_mut()
            .insert(ptr as usize, (p.min_lines, p.max_lines))
    });
    WinHandle(ptr)
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &TextPatch) {
    let TextPatch::SetText(t) = patch;
    unsafe { day_textarea_winui_set_text(h.0, cstr(t).as_ptr()) };
}

fn measure(_backend: &mut WinUi, h: &WinHandle, p: Proposal) -> Size {
    let (min_lines, max_lines) =
        DIMS.with(|m| m.borrow().get(&(h.0 as usize)).copied().unwrap_or((1, 0)));
    let avail_w = p.width.unwrap_or(300.0).max(160.0);
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_winui_measure(h.0, avail_w, -1.0, &mut w, &mut hh) };
    let min_h = (min_lines as f64) * LINE_H + PAD;
    let max_h = if max_lines > 0 {
        (max_lines as f64) * LINE_H + PAD
    } else {
        f64::MAX
    };
    let hgt = hh.clamp(min_h, max_h);
    Size::new(avail_w, hgt)
}

day_pieces::renderer!(day_winui::RENDERERS, WinUi,
    kind: KIND, props: TextProps, patch: TextPatch,
    make: make, update: update, measure: measure);
