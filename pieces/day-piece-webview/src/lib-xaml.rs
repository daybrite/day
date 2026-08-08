// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) wrapping the UWP-XAML WebView,
// boxed into Day handles via the `day_xaml_box`/`day_xaml_unbox` seam day-xaml-sys exports (like
// the Qt renderer's own shim). The shim reports url changes through a C callback →
// `Event::custom("webview:url", …)`. Windows-only, built + verified in CI (not on this host).
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_spec::NodeId;
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_webview_xaml_new(
        url: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_webview_xaml_load(handle: *mut c_void, url: *const c_char);
    fn day_webview_xaml_back(handle: *mut c_void);
    fn day_webview_xaml_forward(handle: *mut c_void);
    fn day_webview_xaml_stop(handle: *mut c_void);
    fn day_webview_xaml_reload(handle: *mut c_void);
    fn day_webview_xaml_set_eval_cb(cb: extern "C" fn(u64, u64, *const c_char));
    fn day_webview_xaml_eval(handle: *mut c_void, req: u64, script: *const c_char);
}

/// One evaluation reply, keyed by request id. The shim calls this exactly once per request —
/// including from the no-engine path — so a pending future can never be stranded.
extern "C" fn on_eval(id: u64, req: u64, payload: *const c_char) {
    let text = if payload.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(payload) }
            .to_string_lossy()
            .into_owned()
    };
    day_xaml::emit(
        NodeId(id),
        Event::Custom {
            tag: "webview:eval",
            // `num` is what tells an eval reply from the URL readback sharing this channel:
            // URL reports keep 0, requests start at 1 (docs/webview-eval.md).
            num: req as f64,
            text,
        },
    );
}

extern "C" fn on_url(id: u64, url: *const c_char) {
    if url.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(url) }
        .to_string_lossy()
        .into_owned();
    day_xaml::emit(NodeId(id), Event::custom("webview:url", s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Xaml, p: &WebProps, id: NodeId) -> WinHandle {
    // The eval callback is a single file-static in the shim, shared by every web view (each reply
    // carries its own node id), so register it once rather than per view.
    static EVAL_CB: std::sync::Once = std::sync::Once::new();
    EVAL_CB.call_once(|| unsafe { day_webview_xaml_set_eval_cb(on_eval) });
    WinHandle(unsafe { day_webview_xaml_new(cstr(&p.url).as_ptr(), id.0, on_url) })
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &WebPatch) {
    unsafe {
        match patch {
            WebPatch::Load(url) => day_webview_xaml_load(h.0, cstr(url).as_ptr()),
            WebPatch::Back => day_webview_xaml_back(h.0),
            WebPatch::Forward => day_webview_xaml_forward(h.0),
            WebPatch::Stop => day_webview_xaml_stop(h.0),
            WebPatch::Reload => day_webview_xaml_reload(h.0),
            WebPatch::Eval { req, script } => {
                day_webview_xaml_eval(h.0, *req, cstr(script).as_ptr())
            }
        }
    }
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
