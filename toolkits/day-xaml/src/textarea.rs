// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) — a multi-line TextBox (AcceptsReturn
// = true, TextWrapping = Wrap, a native PlaceholderText) boxed into a Day handle via the
// day_xaml_box/unbox seam that day-xaml-sys exports. This mirrors the searchfield XAML renderer (own
// shim for the control; reuse the sys crate's generic measure), then clamps the natural height to the
// [min_lines, max_lines] band with an approximate line height. Windows-only, built in CI, not verified
// locally.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

use crate::{WinHandle, Xaml};
use day_spec::{NodeId, Proposal, Size};

// Approximate line height (device-independent px) for the min/max-lines clamp — XAML has no cheap exact
// per-control line metric here, and this backend is best-effort.
const LINE_H: f64 = 20.0;
const PAD: f64 = 12.0;

unsafe extern "C" {
    fn day_textarea_xaml_new(
        placeholder: *const c_char,
        initial: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_textarea_xaml_set_text(w: *mut c_void, text: *const c_char);
    // The three attributes (docs/textarea.md). editable → TextBox::IsReadOnly, spell-check →
    // IsSpellCheckEnabled; selectable has no TextBox property and is emulated in the shim.
    fn day_textarea_xaml_set_editable(w: *mut c_void, on: c_int);
    fn day_textarea_xaml_set_selectable(w: *mut c_void, on: c_int);
    fn day_textarea_xaml_set_spellcheck(w: *mut c_void, on: c_int);
    // Generic size hint from day-xaml-sys (already linked), like the searchfield renderer.
    fn day_xaml_measure(
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
    crate::emit(NodeId(id), Event::TextChanged(s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Xaml, p: &TextProps, id: NodeId) -> WinHandle {
    let ptr = unsafe {
        day_textarea_xaml_new(
            cstr(&p.placeholder).as_ptr(),
            cstr(&p.text).as_ptr(),
            id.0,
            on_text,
        )
    };
    // The attributes are applied at build too, not only through patches: a text_area that starts
    // read-only (or with spell-check off) must come up that way rather than after its first patch.
    unsafe {
        day_textarea_xaml_set_editable(ptr, p.editable as c_int);
        day_textarea_xaml_set_selectable(ptr, p.selectable as c_int);
        day_textarea_xaml_set_spellcheck(ptr, p.spellcheck as c_int);
    }
    DIMS.with(|m| {
        m.borrow_mut()
            .insert(ptr as usize, (p.min_lines, p.max_lines))
    });
    WinHandle(ptr)
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &TextPatch) {
    match patch {
        TextPatch::SetText(t) => unsafe { day_textarea_xaml_set_text(h.0, cstr(t).as_ptr()) },
        TextPatch::SetEditable(v) => unsafe { day_textarea_xaml_set_editable(h.0, *v as c_int) },
        // Emulated, not native: TextBox has no IsTextSelectionEnabled (that is TextBlock's), so the
        // shim collapses selections as they form and suppresses the context menu (docs/textarea.md).
        TextPatch::SetSelectable(v) => unsafe {
            day_textarea_xaml_set_selectable(h.0, *v as c_int)
        },
        TextPatch::SetSpellCheck(v) => unsafe {
            day_textarea_xaml_set_spellcheck(h.0, *v as c_int)
        },
    }
}

fn measure(_backend: &mut Xaml, h: &WinHandle, p: Proposal) -> Size {
    let (min_lines, max_lines) =
        DIMS.with(|m| m.borrow().get(&(h.0 as usize)).copied().unwrap_or((1, 0)));
    let avail_w = p.width.unwrap_or(300.0).max(160.0);
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_xaml_measure(h.0, avail_w, -1.0, &mut w, &mut hh) };
    let min_h = (min_lines as f64) * LINE_H + PAD;
    let max_h = if max_lines > 0 {
        (max_lines as f64) * LINE_H + PAD
    } else {
        f64::MAX
    };
    let hgt = hh.clamp(min_h, max_h);
    Size::new(avail_w, hgt)
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Xaml,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::WinHandle {
    let p = props
        .downcast_ref::<TextProps>()
        .expect("day: textarea props type");
    make(b, p, id)
}

pub(crate) fn update_any(b: &mut crate::Xaml, h: &crate::WinHandle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<TextPatch>() {
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
