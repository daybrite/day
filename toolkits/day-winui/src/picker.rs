// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) — ComboBox / RadioButton StackPanels,
// boxed into Day handles via the `day_winui_box`/`day_winui_unbox` seam day-winui-sys exports. This
// mirrors the Qt renderer (own shim for the control; reuse the sys crate's generic measure).
// Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_spec::{NodeId, Proposal, Size};
use crate::{WinHandle, WinUi};

unsafe extern "C" {
    fn day_picker_winui_new(
        style: c_int,
        items_joined: *const c_char,
        selected: c_int,
        id: u64,
        cb: extern "C" fn(u64, c_int),
    ) -> *mut c_void;
    fn day_picker_winui_set_selected(w: *mut c_void, idx: c_int);
    // Generic size hint from day-winui-sys (already linked) — like the Qt renderer reusing
    // day-qt-sys's `day_qt_size_hint`.
    fn day_winui_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

extern "C" fn on_select(id: u64, idx: c_int) {
    crate::emit(NodeId(id), Event::SelectionChanged(idx as i64));
}

fn style_code(s: PickerStyle) -> c_int {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut WinUi, p: &PickerProps, id: NodeId) -> WinHandle {
    let joined = CString::new(p.options.join("\n")).unwrap_or_default();
    WinHandle(unsafe {
        day_picker_winui_new(
            style_code(p.style),
            joined.as_ptr(),
            p.selected as c_int,
            id.0,
            on_select,
        )
    })
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &PickerPatch) {
    {
        let PickerPatch::Selected(i) = patch;
        unsafe { day_picker_winui_set_selected(h.0, *i as c_int) };
    }
}

fn measure(_backend: &mut WinUi, h: &WinHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_winui_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w.max(120.0), hh.max(32.0))
}


// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::WinUi,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::WinHandle {
    let p = props
        .downcast_ref::<PickerProps>()
        .expect("day: picker props type");
    make(b, p, id)
}

pub(crate) fn update_any(b: &mut crate::WinUi, h: &crate::WinHandle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut crate::WinUi,
    h: &crate::WinHandle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
