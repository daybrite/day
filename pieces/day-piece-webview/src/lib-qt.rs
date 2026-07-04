// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) wrapping QWebEngineView behind a flat C ABI.
// build.rs compiles it AND links Qt6WebEngineWidgets (which day-qt-sys does not). The shim reports
// url changes through a C callback → `Event::Custom("webview:url", …)`.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Renderer};
use linkme::distributed_slice;

unsafe extern "C" {
    fn day_webview_new(
        url: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_webview_load(w: *mut c_void, url: *const c_char);
    fn day_webview_back(w: *mut c_void);
    fn day_webview_forward(w: *mut c_void);
    fn day_webview_stop(w: *mut c_void);
    fn day_webview_reload(w: *mut c_void);
}

extern "C" fn on_url(id: u64, url: *const c_char) {
    if url.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(url) }
        .to_string_lossy()
        .into_owned();
    day_qt::emit(NodeId(id), Event::Custom("webview:url", s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Qt, props: &dyn std::any::Any, id: NodeId) -> QtHandle {
    let p = props.downcast_ref::<WebProps>().unwrap();
    QtHandle(unsafe { day_webview_new(cstr(&p.url).as_ptr(), id.0, on_url) })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &dyn std::any::Any) {
    let Some(patch) = patch.downcast_ref::<WebPatch>() else {
        return;
    };
    unsafe {
        match patch {
            WebPatch::Load(url) => day_webview_load(h.0, cstr(url).as_ptr()),
            WebPatch::Back => day_webview_back(h.0),
            WebPatch::Forward => day_webview_forward(h.0),
            WebPatch::Stop => day_webview_stop(h.0),
            WebPatch::Reload => day_webview_reload(h.0),
        }
    }
}

#[distributed_slice(day_qt::RENDERERS)]
static WEBVIEW_QT: fn() -> Renderer<Qt> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: None,
};
