// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) — ComboBox / RadioButton StackPanels,
// boxed into day handles via the `day_winui_box`/`day_winui_unbox` seam day-winui-sys exports. This
// mirrors the Qt renderer (own shim for the control; reuse the sys crate's generic measure).
// Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_spec::{NodeId, Proposal, Renderer, Size};
use day_winui::{WinHandle, WinUi};
use linkme::distributed_slice;

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
    day_winui::emit(NodeId(id), Event::SelectionChanged(idx as i64));
}

fn style_code(s: PickerStyle) -> c_int {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut WinUi, props: &dyn std::any::Any, id: NodeId) -> WinHandle {
    let p = props.downcast_ref::<PickerProps>().unwrap();
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

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &dyn std::any::Any) {
    if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
        unsafe { day_picker_winui_set_selected(h.0, *i as c_int) };
    }
}

fn measure(_backend: &mut WinUi, h: &WinHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_winui_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w.max(120.0), hh.max(32.0))
}

#[distributed_slice(day_winui::RENDERERS)]
static PICKER_WINUI: fn() -> Renderer<WinUi> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
