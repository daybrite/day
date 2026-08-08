// ---------------------------------------------------------------------------
// AppKit: WKWebView (WebKit). A custom navigation delegate reports the committed URL back via
// `Event::custom("webview:url", …)` so a bound text field follows navigation. WKWebView keeps its
// navigationDelegate WEAKLY, so we retain each delegate in a thread_local for the view's lifetime.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use block2::RcBlock;
use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSView;
use objc2_foundation::{NSError, NSObject, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView};

struct NavIvars {
    /// Mutable: a session-retained view outlives the node that first realized it, and must report
    /// to whichever node is currently showing it.
    node: Cell<NodeId>,
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
                day_appkit::emit(self.ivars().node.get(), Event::custom("webview:url", url));
            }
        }
    }
);

impl WebNav {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavIvars {
            node: Cell::new(node),
        });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    // Keep each navigation delegate alive as long as its web view (delegate ref is weak).
    static DELEGATES: RefCell<HashMap<usize, Retained<WebNav>>> = RefCell::new(HashMap::new());
    // Session id -> the retained web view. Day releases its own reference when the page is
    // navigated away from, which on AppKit only detaches (`removeFromSuperview`); this reference
    // is what keeps the engine — and therefore the loaded page — alive until the app returns.
    static SESSIONS: RefCell<HashMap<u64, Retained<NSView>>> = RefCell::new(HashMap::new());
}

/// The node a realized view belongs to. `update` is handed only the native handle, so the id is
/// recovered from the navigation delegate kept alive alongside it.
fn node_of(view: &Retained<NSView>) -> Option<NodeId> {
    let key = (&**view as *const NSView) as usize;
    DELEGATES.with(|m| m.borrow().get(&key).map(|nav| nav.ivars().node.get()))
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
    // A session already holding a view: re-attach it rather than build a new one. Only the node
    // changes — point the delegate at the node now showing it, and do NOT reload, since the whole
    // purpose is to come back to the page as it was left.
    if p.session != 0
        && let Some(view) = SESSIONS.with(|m| m.borrow().get(&p.session).cloned())
    {
        let key = (&*view as *const NSView) as usize;
        DELEGATES.with(|m| {
            if let Some(nav) = m.borrow().get(&key) {
                nav.ivars().node.set(id);
            }
        });
        return view;
    }

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
    if p.session != 0 {
        SESSIONS.with(|m| m.borrow_mut().insert(p.session, view.clone()));
    }
    view
}

/// Run `script` and report `1␟<json>` / `0␟<name>␟<message>` back on `node`, keyed by `req`.
///
/// The script is already wrapped by the front-end, so it always evaluates to a JS string and the
/// `id` handed to the completion is an `NSString` — no `NSJSONSerialization` walk, which matters
/// because WebKit hands back genuinely cyclic dictionaries that would hang it.
///
/// A `nil` result with a `nil` error cannot happen here for the same reason (the wrapper always
/// returns a string), so anything else is WebKit itself failing — most often a dead content
/// process, which reports as `JavaScriptResultTypeIsUnsupported` with no exception message.
fn eval(web: &WKWebView, node: NodeId, req: u64, script: &str) {
    let js = NSString::from_str(script);
    let handler = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
        let payload = if !result.is_null() {
            // SAFETY: non-null result from WebKit; the wrapper guarantees it is an NSString.
            let obj = unsafe { &*result };
            obj.downcast_ref::<NSString>()
                .map(|s| s.to_string())
                .unwrap_or_else(|| engine_error("WebKitError", "non-string reply"))
        } else if !error.is_null() {
            // SAFETY: non-null NSError from WebKit.
            let msg = unsafe { (*error).localizedDescription() }.to_string();
            engine_error("WebKitError", &msg)
        } else {
            engine_error("WebKitError", "no result")
        };
        day_appkit::emit(node, Event::Custom {
            tag: "webview:eval",
            num: req as f64,
            text: payload,
        });
    });
    // SAFETY: main thread (a renderer duty), and WebKit copies the block before returning.
    unsafe { web.evaluateJavaScript_completionHandler(&js, Some(&handler)) };
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &WebPatch) {
    let Some(web) = h.downcast_ref::<WKWebView>() else {
        return;
    };
    match patch {
        WebPatch::Eval { req, script } => {
            if let Some(node) = node_of(h) {
                eval(web, node, *req, script);
            }
        }
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

/// Drop the retained navigation delegate when the view goes away.
///
/// Without this the map grows by one entry per realized web view, and — worse — its key is the
/// view's ADDRESS, which the allocator reuses: a later view landing on a freed address would
/// inherit the dead node's id and misroute its events.
fn release(_backend: &mut AppKit, h: &Retained<NSView>) {
    let key = (&**h as *const NSView) as usize;
    // A session-retained view is not going away — day is only detaching it from the page being
    // torn down, and the next visit re-attaches it. Its delegate has to outlive this node too, or
    // the returning view would report navigations to nobody.
    let retained = SESSIONS.with(|m| {
        m.borrow()
            .values()
            .any(|v| (&**v as *const NSView) as usize == key)
    });
    if retained {
        return;
    }
    DELEGATES.with(|m| {
        m.borrow_mut().remove(&key);
    });
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure, release: release);
