// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — QComboBox / checkable QPushButtons /
// QRadioButtons, one DayPicker widget per style behind a flat C ABI.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Renderer, Size};
use linkme::distributed_slice;

unsafe extern "C" {
    fn day_picker_new(
        style: c_int,
        items_joined: *const c_char,
        selected: c_int,
        id: u64,
        cb: extern "C" fn(u64, c_int),
    ) -> *mut c_void;
    fn day_picker_set_selected(w: *mut c_void, idx: c_int);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

extern "C" fn on_select(id: u64, idx: c_int) {
    day_qt::emit(NodeId(id), Event::SelectionChanged(idx as i64));
}

fn joined(items: &[String]) -> CString {
    CString::new(items.join("\n")).unwrap_or_default()
}

fn style_code(s: PickerStyle) -> c_int {
    match s {
        PickerStyle::Menu => 0,
        PickerStyle::Segmented => 1,
        PickerStyle::Inline => 2,
    }
}

fn make(_backend: &mut Qt, props: &dyn std::any::Any, id: NodeId) -> QtHandle {
    let p = props.downcast_ref::<PickerProps>().unwrap();
    QtHandle(unsafe {
        day_picker_new(
            style_code(p.style),
            joined(&p.options).as_ptr(),
            p.selected as c_int,
            id.0,
            on_select,
        )
    })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &dyn std::any::Any) {
    if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
        unsafe { day_picker_set_selected(h.0, *i as c_int) };
    }
}

fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    Size::new(w.max(60.0), hh.max(22.0))
}

#[distributed_slice(day_qt::RENDERERS)]
static PICKER_QT: fn() -> Renderer<Qt> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
