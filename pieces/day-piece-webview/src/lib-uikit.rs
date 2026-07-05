// ---------------------------------------------------------------------------
// UIKit: WKWebView (WebKit) — the same control as AppKit, but a UIView subclass on iOS. objc2-web-kit
// 0.3 only generates the macOS (NSView) WKWebView binding, so here we hand-roll the iOS class via
// `extern_class!` + `msg_send!`. A navigation delegate reports the committed URL back through
// `Event::custom("webview:url", …)`; retained in a thread_local (WKWebView keeps the delegate weakly).
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_class, msg_send};
use objc2_foundation::{NSString, NSURL, NSURLRequest};
use objc2_ui_kit::{UIResponder, UIView};

// WKWebView lives in WebKit.framework. objc2-web-kit force-links it on macOS but only binds the
// AppKit variant, so on iOS we hand-roll the class below. WebKit must be LINKED or
// `objc_getClass("WKWebView")` returns nil and `alloc` aborts (SIGABRT) — declared via this crate's
// `[package.metadata.day.ios].frameworks = ["WebKit"]`, which the generated DayPieces SwiftPM package
// links into the app (no runtime `dlopen`, no xcodeproj edit — the framework-contribution seam).

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
                day_uikit::emit(self.ivars().node, Event::custom("webview:url", url));
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

fn make(_backend: &mut Uikit, p: &WebProps, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
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

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &WebPatch) {
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

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
