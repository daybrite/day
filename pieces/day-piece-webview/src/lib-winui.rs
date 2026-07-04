// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/lib-winui-shim.cpp) wrapping the UWP-XAML WebView,
// boxed into day handles via the `day_winui_box`/`day_winui_unbox` seam day-winui-sys exports (like
// the Qt renderer's own shim). The shim reports url changes through a C callback →
// `Event::Custom("webview:url", …)`. Windows-only, built + verified in CI (not on this host).
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_spec::{NodeId, Proposal, Renderer, Size};
use day_winui::{WinHandle, WinUi};
use linkme::distributed_slice;

unsafe extern "C" {
    fn day_webview_winui_new(
        url: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
    ) -> *mut c_void;
    fn day_webview_winui_load(handle: *mut c_void, url: *const c_char);
    fn day_webview_winui_back(handle: *mut c_void);
    fn day_webview_winui_forward(handle: *mut c_void);
    fn day_webview_winui_stop(handle: *mut c_void);
    fn day_webview_winui_reload(handle: *mut c_void);
}

extern "C" fn on_url(id: u64, url: *const c_char) {
    if url.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(url) }
        .to_string_lossy()
        .into_owned();
    day_winui::emit(NodeId(id), Event::Custom("webview:url", s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut WinUi, props: &dyn std::any::Any, id: NodeId) -> WinHandle {
    let p = props.downcast_ref::<WebProps>().unwrap();
    WinHandle(unsafe { day_webview_winui_new(cstr(&p.url).as_ptr(), id.0, on_url) })
}

fn update(_backend: &mut WinUi, h: &WinHandle, patch: &dyn std::any::Any) {
    let Some(patch) = patch.downcast_ref::<WebPatch>() else {
        return;
    };
    unsafe {
        match patch {
            WebPatch::Load(url) => day_webview_winui_load(h.0, cstr(url).as_ptr()),
            WebPatch::Back => day_webview_winui_back(h.0),
            WebPatch::Forward => day_webview_winui_forward(h.0),
            WebPatch::Stop => day_webview_winui_stop(h.0),
            WebPatch::Reload => day_webview_winui_reload(h.0),
        }
    }
}

/// Fill the offered space (a web view has no intrinsic size).
fn measure(_backend: &mut WinUi, _h: &WinHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
}

#[distributed_slice(day_winui::RENDERERS)]
static WEBVIEW_WINUI: fn() -> Renderer<WinUi> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
