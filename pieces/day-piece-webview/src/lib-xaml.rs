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
            // Not implemented on this backend yet (docs/webview-eval.md). `eval_support()`
            // reports Unsupported, so the front-end resolves the future without dispatching
            // and this arm is unreachable — it exists to keep the match exhaustive.
            WebPatch::Eval { .. } => {}
        }
    }
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
