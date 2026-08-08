// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) wrapping QWebEngineView behind a flat C ABI.
// build.rs compiles it AND links Qt6WebEngineWidgets (which day-qt-sys does not). The shim reports
// url changes through a C callback → `Event::custom("webview:url", …)`.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::NodeId;

unsafe extern "C" {
    fn day_webview_new(
        url: *const c_char,
        id: u64,
        cb: extern "C" fn(u64, *const c_char),
        session: u64,
    ) -> *mut c_void;
    fn day_webview_load(w: *mut c_void, url: *const c_char);
    fn day_webview_back(w: *mut c_void);
    fn day_webview_forward(w: *mut c_void);
    fn day_webview_stop(w: *mut c_void);
    fn day_webview_reload(w: *mut c_void);
    fn day_webview_set_eval_cb(cb: extern "C" fn(u64, u64, *const c_char));
    fn day_webview_eval(w: *mut c_void, req: u64, script: *const c_char);
}

/// One evaluation reply, keyed by request id. The shim always calls this exactly once per request
/// — including from the no-engine fallback — so a pending future can never be stranded.
extern "C" fn on_eval(id: u64, req: u64, payload: *const c_char) {
    let text = if payload.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(payload) }
            .to_string_lossy()
            .into_owned()
    };
    day_qt::emit(
        NodeId(id),
        Event::Custom {
            tag: "webview:eval",
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
    day_qt::emit(NodeId(id), Event::custom("webview:url", s));
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn make(_backend: &mut Qt, p: &WebProps, id: NodeId) -> QtHandle {
    // The eval callback is a single file-static in the shim, shared by every web view (the reply
    // carries its own node id), so register it once rather than per view.
    static EVAL_CB: std::sync::Once = std::sync::Once::new();
    EVAL_CB.call_once(|| unsafe { day_webview_set_eval_cb(on_eval) });
    QtHandle(unsafe { day_webview_new(cstr(&p.url).as_ptr(), id.0, on_url, p.session) })
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &WebPatch) {
    unsafe {
        match patch {
            WebPatch::Load(url) => day_webview_load(h.0, cstr(url).as_ptr()),
            WebPatch::Back => day_webview_back(h.0),
            WebPatch::Forward => day_webview_forward(h.0),
            WebPatch::Stop => day_webview_stop(h.0),
            WebPatch::Reload => day_webview_reload(h.0),
            WebPatch::Eval { req, script } => day_webview_eval(h.0, *req, cstr(script).as_ptr()),
        }
    }
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
