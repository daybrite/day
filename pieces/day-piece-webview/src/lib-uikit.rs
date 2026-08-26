// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// UIKit: WKWebView (WebKit) — the same control as AppKit, but a UIView subclass on iOS. objc2-web-kit
// 0.3 only generates the macOS (NSView) WKWebView binding, so here we hand-roll the iOS class via
// `extern_class!` + `msg_send!`. A navigation delegate reports the committed URL back through
// `Event::custom("webview:url", …)`; retained in a thread_local (WKWebView keeps the delegate weakly).
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_class, msg_send};
use block2::RcBlock;
use objc2_foundation::{NSError, NSString, NSURL, NSURLRequest};
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
    /// Mutable: a session-retained view outlives the node that first realized it, and must report
    /// to whichever node is currently showing it.
    node: Cell<NodeId>,
    /// Inline mode (docs/webview.md): the `file://` URL prefix of the bundled site's root.
    /// A main-frame navigation outside it is cancelled and reported (`LINK_REPORT`); `None`
    /// (remote mode) polices nothing.
    inline_base: RefCell<Option<String>>,
}

/// WKNavigationActionPolicy, hand-rolled like the class itself: Cancel = 0, Allow = 1.
const POLICY_CANCEL: isize = 0;
const POLICY_ALLOW: isize = 1;

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
                day_uikit::emit(self.ivars().node.get(), Event::custom("webview:url", url));
            }
        }

        // Inline mode's link policy — same contract as the AppKit arm's `decide_policy`: a
        // main-frame navigation leaving the bundled site is CANCELLED and reported; the piece
        // runs the app's `LinkPolicy` (events are enqueue-only, the decision can't come back
        // through this callback). Raw msg_send shapes, like the rest of this hand-rolled arm.
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy(
            &self,
            _web_view: &WKWebView,
            action: &AnyObject,
            handler: &block2::DynBlock<dyn Fn(isize)>,
        ) {
            let policy = match &*self.ivars().inline_base.borrow() {
                None => POLICY_ALLOW,
                Some(base) => {
                    let frame: *mut AnyObject = unsafe { msg_send![action, targetFrame] };
                    // Subframes stay the page's business; a nil target frame (window.open,
                    // target=_blank) is external by definition.
                    let main_frame =
                        !frame.is_null() && unsafe { msg_send![&*frame, isMainFrame] };
                    let sub_frame = !frame.is_null() && !main_frame;
                    let req: Retained<NSURLRequest> = unsafe { msg_send![action, request] };
                    let url = req
                        .URL()
                        .and_then(|u| u.absoluteString())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let inside = url.starts_with(base.as_str()) || url == "about:blank";
                    if sub_frame || inside {
                        POLICY_ALLOW
                    } else {
                        day_uikit::emit(self.ivars().node.get(), Event::Custom {
                            tag: "webview:link",
                            num: super::LINK_REPORT,
                            text: url,
                        });
                        POLICY_CANCEL
                    }
                }
            };
            handler.call((policy,));
        }
    }
);

impl WebNav {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavIvars {
            node: Cell::new(node),
            inline_base: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }
}

day_core::tls_group! {
    // Keep each navigation delegate alive as long as its web view (delegate ref is weak).
    static DELEGATES: RefCell<HashMap<usize, Retained<WebNav>>> = RefCell::new(HashMap::new());
    // Session id -> the retained web view. Day drops its own reference when the page is navigated
    // away from, which on UIKit only detaches (`removeFromSuperview`); this reference is what keeps
    // the engine — and so the loaded page and its JavaScript context — alive until the app returns.
    static SESSIONS: RefCell<HashMap<u64, Retained<UIView>>> = RefCell::new(HashMap::new());

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
    // A session already holding a view: re-attach it rather than build a new one. Only the node
    // changes — point the delegate at the node now showing it, and do NOT reload, since the whole
    // purpose is to come back to the page as it was left.
    if p.session != 0
        && let Some(view) = SESSIONS.with(|m| m.borrow().get(&p.session).cloned())
    {
        let key = (&*view as *const UIView) as usize;
        DELEGATES.with(|m| {
            if let Some(nav) = m.borrow().get(&key) {
                nav.ivars().node.set(id);
            }
        });
        return view;
    }

    let mtm = MainThreadMarker::new().unwrap();
    let web: Retained<WKWebView> = unsafe { msg_send![WKWebView::alloc(mtm), init] };
    let nav = WebNav::new(mtm, id);
    let _: () = unsafe { msg_send![&web, setNavigationDelegate: &*nav] };
    if !p.inline_root.is_empty() {
        // Inline mode (docs/webview.md): the assets tree is loose files in the app bundle, so
        // `loadFileURL:allowingReadAccessToURL:` with the site ROOT is the whole load path —
        // WebKit resolves the page's relative references natively.
        if let Some(dir) = day_spec::resolve_asset_dir(&p.inline_root) {
            let root = NSURL::fileURLWithPath(&NSString::from_str(&dir.display().to_string()));
            let index = NSURL::fileURLWithPath(&NSString::from_str(
                &dir.join(&p.inline_start).display().to_string(),
            ));
            if let Some(base) = root.absoluteString() {
                *nav.ivars().inline_base.borrow_mut() = Some(base.to_string());
            }
            let _: *mut AnyObject =
                unsafe { msg_send![&web, loadFileURL: &*index, allowingReadAccessToURL: &*root] };
        } else {
            log::warn!(
                "day-piece-webview: inline site {:?} not found in the staged assets",
                p.inline_root
            );
        }
    } else if !p.url.is_empty() {
        load_url(&web, &p.url);
    }
    let view: Retained<UIView> = Retained::from(<WKWebView as AsRef<UIView>>::as_ref(&web));
    DELEGATES.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const UIView) as usize, nav)
    });
    if p.session != 0 {
        SESSIONS.with(|m| m.borrow_mut().insert(p.session, view.clone()));
    }
    view
}

/// The node a realized view belongs to. `update` gets only the native handle, so the id comes back
/// from the navigation delegate retained alongside it.
fn node_of(view: &Retained<UIView>) -> Option<NodeId> {
    let key = (&**view as *const UIView) as usize;
    DELEGATES.with(|m| m.borrow().get(&key).map(|nav| nav.ivars().node.get()))
}

/// Same contract as the AppKit arm (see its `eval`): the front-end's wrapper makes the result
/// always a JS string, so the completion's `id` is an `NSString` and no JSON walk is needed.
fn eval(web: &WKWebView, node: NodeId, req: u64, script: &str) {
    let js = NSString::from_str(script);
    let handler = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
        let payload = if !result.is_null() {
            // SAFETY: non-null result from WebKit; the wrapper guarantees an NSString.
            unsafe { &*result }
                .downcast_ref::<NSString>()
                .map(|s| s.to_string())
                .unwrap_or_else(|| engine_error("WebKitError", "non-string reply"))
        } else if !error.is_null() {
            // SAFETY: non-null NSError from WebKit.
            engine_error(
                "WebKitError",
                &unsafe { (*error).localizedDescription() }.to_string(),
            )
        } else {
            engine_error("WebKitError", "no result")
        };
        day_uikit::emit(
            node,
            Event::Custom {
                tag: "webview:eval",
                num: req as f64,
                text: payload,
            },
        );
    });
    // SAFETY: main thread (a renderer duty); WebKit copies the block before returning.
    let _: () = unsafe { msg_send![web, evaluateJavaScript: &*js, completionHandler: &*handler] };
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &WebPatch) {
    let Some(web) = (**h).downcast_ref::<WKWebView>() else {
        return;
    };
    unsafe {
        match patch {
            WebPatch::Eval { req, script } => {
                if let Some(node) = node_of(h) {
                    eval(web, node, *req, script);
                }
            }
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

/// Drop the retained navigation delegate when the view goes away.
///
/// Without this the map grows by one entry per realized web view, and — worse — its key is the
/// view's ADDRESS, which the allocator reuses: a later view landing on a freed address would
/// inherit the dead node's id and misroute its events.
fn release(_backend: &mut Uikit, h: &Retained<UIView>) {
    let key = (&**h as *const UIView) as usize;
    // A session-retained view is not going away — day is only detaching it from the page being
    // torn down, and the next visit re-attaches it. Its delegate has to outlive this node too, or
    // the returning view would report navigations to nobody.
    let retained = SESSIONS.with(|m| {
        m.borrow()
            .values()
            .any(|v| (&**v as *const UIView) as usize == key)
    });
    if retained {
        return;
    }
    DELEGATES.with(|m| {
        m.borrow_mut().remove(&key);
    });
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure, release: release);
