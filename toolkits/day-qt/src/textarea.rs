// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a QPlainTextEdit (multi-line plain-text editor) with
// a native placeholder, word wrap, and an internal scrollbar, behind a flat C ABI. textChanged
// dispatches Event::TextChanged; programmatic setPlainText is wrapped in blockSignals so it never echoes
// back (like the searchfield shim). `measure` calls the shim's content-height helper, clamped to the
// [min_lines, max_lines] band (kept per handle since `measure` receives no props).
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use crate::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_textarea_new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_textarea_set_text(w: *mut c_void, text: *const c_char);
    fn day_textarea_measure(
        w: *mut c_void,
        avail_w: f64,
        min_lines: u32,
        max_lines: u32,
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
    crate::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Qt, p: &TextProps, id: NodeId) -> QtHandle {
    let ptr = unsafe {
        day_textarea_new(
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
    QtHandle(ptr)
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &TextPatch) {
    let TextPatch::SetText(t) = patch;
    unsafe { day_textarea_set_text(h.0, cstr(t).as_ptr()) };
}

fn measure(_backend: &mut Qt, h: &QtHandle, p: Proposal) -> Size {
    let (min_lines, max_lines) =
        DIMS.with(|m| m.borrow().get(&(h.0 as usize)).copied().unwrap_or((1, 0)));
    let avail_w = p.width.unwrap_or(300.0).max(120.0);
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_textarea_measure(h.0, avail_w, min_lines, max_lines, &mut w, &mut hh) };
    Size::new(w.max(120.0), hh.max(24.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Qt,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    let p = props
        .downcast_ref::<TextProps>()
        .expect("day: textarea props type");
    make(b, p, id)
}

pub(crate) fn update_any(b: &mut crate::Qt, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<TextPatch>() {
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
