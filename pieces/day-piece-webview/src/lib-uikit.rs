// ---------------------------------------------------------------------------
// UIKit: WKWebView (WebKit) — the same control as AppKit, but a UIView subclass on iOS. objc2-web-kit
// 0.3 only generates the macOS (NSView) WKWebView binding, so here we hand-roll the iOS class via
// `extern_class!` + `msg_send!`. A navigation delegate reports the committed URL back through
// `Event::Custom("webview:url", …)`; retained in a thread_local (WKWebView keeps the delegate weakly).
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{NodeId, Renderer};
use day_uikit::Uikit;
use linkme::distributed_slice;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_class, msg_send};
use objc2_foundation::{NSString, NSURL, NSURLRequest};
use objc2_ui_kit::{UIResponder, UIView};

// WKWebView lives in WebKit.framework. objc2-web-kit force-links it on macOS, but it only binds the
// AppKit variant, so on iOS we hand-roll the class below — and must ensure WebKit is loaded or
// `objc_getClass("WKWebView")` returns nil and `alloc` aborts (SIGABRT). A `#[link]` autolink hint
// is unreliable here because xcode (not cargo) performs the final link of the app; instead we
// `dlopen` the (public) framework once at first use. This keeps the piece fully self-contained —
// it needs no framework entry in the app's xcode project.
unsafe extern "C" {
    fn dlopen(
        path: *const std::os::raw::c_char,
        mode: std::os::raw::c_int,
    ) -> *mut std::ffi::c_void;
}

fn ensure_webkit_loaded() {
    use std::sync::Once;
    static LOAD: Once = Once::new();
    LOAD.call_once(|| {
        const RTLD_LAZY: std::os::raw::c_int = 0x1;
        let path = c"/System/Library/Frameworks/WebKit.framework/WebKit";
        // Loading registers the WKWebView Objective-C class; a no-op if already loaded.
        unsafe { dlopen(path.as_ptr(), RTLD_LAZY) };
    });
}

// The iOS WKWebView (a UIView subclass). We only need a handful of methods, called via msg_send!.
extern_class!(
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct WKWebView;
);

struct NavIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayWebNavUIKit"]
    #[ivars = NavIvars]
    struct WebNav;

    unsafe impl NSObjectProtocol for WebNav {}

    impl WebNav {
        // WKNavigationDelegate's webView:didFinishNavigation: — WKWebView calls it on the object we
        // set as its navigationDelegate; responding to the selector is all that's required.
        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WKWebView, _navigation: *mut AnyObject) {
            if let Some(url) = current_url(web_view) {
                day_uikit::emit(self.ivars().node, Event::Custom("webview:url", url));
            }
        }
    }
);

impl WebNav {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static DELEGATES: RefCell<HashMap<usize, Retained<WebNav>>> = RefCell::new(HashMap::new());
}

fn current_url(web: &WKWebView) -> Option<String> {
    let url: Option<Retained<NSURL>> = unsafe { msg_send![web, URL] };
    let s = url?.absoluteString()?;
    Some(s.to_string())
}

fn load_url(web: &WKWebView, url: &str) {
    let ns = NSString::from_str(url);
    let Some(nsurl) = NSURL::URLWithString(&ns) else {
        return;
    };
    let req = NSURLRequest::requestWithURL(&nsurl);
    let _: *mut AnyObject = unsafe { msg_send![web, loadRequest: &*req] };
}

fn make(_backend: &mut Uikit, props: &dyn std::any::Any, id: NodeId) -> Retained<UIView> {
    let p = props.downcast_ref::<WebProps>().unwrap();
    let mtm = MainThreadMarker::new().unwrap();
    ensure_webkit_loaded();
    let web: Retained<WKWebView> = unsafe { msg_send![WKWebView::alloc(mtm), init] };
    let nav = WebNav::new(mtm, id);
    let _: () = unsafe { msg_send![&web, setNavigationDelegate: &*nav] };
    if !p.url.is_empty() {
        load_url(&web, &p.url);
    }
    let view: Retained<UIView> = Retained::from(<WKWebView as AsRef<UIView>>::as_ref(&web));
    DELEGATES.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const UIView) as usize, nav)
    });
    view
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &dyn std::any::Any) {
    let Some(patch) = patch.downcast_ref::<WebPatch>() else {
        return;
    };
    let Some(web) = (**h).downcast_ref::<WKWebView>() else {
        return;
    };
    unsafe {
        match patch {
            WebPatch::Load(url) => load_url(web, url),
            WebPatch::Back => {
                let _: *mut AnyObject = msg_send![web, goBack];
            }
            WebPatch::Forward => {
                let _: *mut AnyObject = msg_send![web, goForward];
            }
            WebPatch::Stop => {
                let _: () = msg_send![web, stopLoading];
            }
            WebPatch::Reload => {
                let _: *mut AnyObject = msg_send![web, reload];
            }
        }
    }
}

#[distributed_slice(day_uikit::RENDERERS)]
static WEBVIEW_UIKIT: fn() -> Renderer<Uikit> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: None,
};
