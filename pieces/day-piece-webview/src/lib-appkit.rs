// ---------------------------------------------------------------------------
// AppKit: WKWebView (WebKit). A custom navigation delegate reports the committed URL back via
// `Event::custom("webview:url", …)` so a bound text field follows navigation. WKWebView keeps its
// navigationDelegate WEAKLY, so we retain each delegate in a thread_local for the view's lifetime.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObject, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView};

struct NavIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayWebNav"]
    #[ivars = NavIvars]
    struct WebNav;

    unsafe impl NSObjectProtocol for WebNav {}

    unsafe impl WKNavigationDelegate for WebNav {
        // Fired when a navigation completes — report the new URL back to the piece.
        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            if let Some(url) = current_url(web_view) {
                day_appkit::emit(self.ivars().node, Event::custom("webview:url", url));
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
    // Keep each navigation delegate alive as long as its web view (delegate ref is weak).
    static DELEGATES: RefCell<HashMap<usize, Retained<WebNav>>> = RefCell::new(HashMap::new());
}

fn current_url(web: &WKWebView) -> Option<String> {
    let url = unsafe { web.URL() }?;
    let s = url.absoluteString()?;
    Some(s.to_string())
}

fn load_url(web: &WKWebView, url: &str) {
    let ns = NSString::from_str(url);
    let Some(nsurl) = NSURL::URLWithString(&ns) else {
        return;
    };
    let req = NSURLRequest::requestWithURL(&nsurl);
    let _ = unsafe { web.loadRequest(&req) };
}

fn make(backend: &mut AppKit, p: &WebProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    // SAFETY: creates a WKWebView with a default configuration on the main thread.
    let web = unsafe { WKWebView::new(mtm) };
    let nav = WebNav::new(mtm, id);
    unsafe { web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*nav))) };
    if !p.url.is_empty() {
        load_url(&web, &p.url);
    }
    let view: Retained<NSView> = Retained::from(<WKWebView as AsRef<NSView>>::as_ref(&web));
    DELEGATES.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const NSView) as usize, nav)
    });
    view
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &WebPatch) {
    let Some(web) = h.downcast_ref::<WKWebView>() else {
        return;
    };
    match patch {
        WebPatch::Load(url) => load_url(web, url),
        WebPatch::Back => {
            let _ = unsafe { web.goBack() };
        }
        WebPatch::Forward => {
            let _ = unsafe { web.goForward() };
        }
        WebPatch::Stop => unsafe { web.stopLoading() },
        WebPatch::Reload => {
            let _ = unsafe { web.reload() };
        }
    }
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
