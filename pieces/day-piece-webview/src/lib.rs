//! day-piece-webview — an EXTERNAL Day Piece (DESIGN.md §15) wrapping each toolkit's NATIVE web view:
//! WKWebView on AppKit/UIKit, QWebEngineView on Qt, `android.webkit.WebView` on Android. One Rust API
//! registered link-time into each backend's renderer slice without touching day. Alongside the
//! picker it's a reference for pieces that carry both a front-end AND their own native backend — here
//! including an Android manifest permission contribution (INTERNET), see docs/extending.md.
//!
//! The view is a growing leaf that fills its space. Navigation is imperative and modeled with `Copy`
//! `Trigger`s — `.go()` loads the bound URL, `.back()`/`.forward()`/`.stop()`/`.reload()` drive
//! history — each `watch`ed to a `WebPatch`. The bound URL is two-way: `.go()` loads it, and native
//! navigation reports the current URL back so a bound text field follows along.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Signal, Trigger, watch};
use day_spec::Event;

pub const KIND: &str = "day.piece.webview";

/// Full props (realize). The initial `url` is loaded when the native view is created.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebProps {
    pub url: String,
}

/// Sparse imperative commands sent to the native view after creation.
#[derive(Clone, Debug, PartialEq)]
pub enum WebPatch {
    /// Load a URL (from `.go()`).
    Load(String),
    /// History back / forward.
    Back,
    Forward,
    /// Stop the in-flight load (the demo's "cancel").
    Stop,
    /// Reload the current page.
    Reload,
}

/// A native web view bound to `url`. Attach command triggers with `.go()/.back()/.forward()/
/// .stop()/.reload()`; fire them (`Trigger::notify`) from buttons.
pub struct WebView {
    url: Signal<String>,
    go: Option<Trigger>,
    back: Option<Trigger>,
    forward: Option<Trigger>,
    stop: Option<Trigger>,
    reload: Option<Trigger>,
}

/// `web_view(url)` — a native web view showing `url`. The initial value loads on creation; call
/// `.go(trigger)` and fire the trigger to (re)load whatever `url` currently holds.
pub fn web_view(url: Signal<String>) -> WebView {
    WebView {
        url,
        go: None,
        back: None,
        forward: None,
        stop: None,
        reload: None,
    }
}

impl WebView {
    /// Load the current value of the bound `url` whenever `trigger` fires.
    pub fn go(mut self, trigger: Trigger) -> Self {
        self.go = Some(trigger);
        self
    }
    /// Navigate back in history whenever `trigger` fires.
    pub fn back(mut self, trigger: Trigger) -> Self {
        self.back = Some(trigger);
        self
    }
    /// Navigate forward in history whenever `trigger` fires.
    pub fn forward(mut self, trigger: Trigger) -> Self {
        self.forward = Some(trigger);
        self
    }
    /// Stop the current load whenever `trigger` fires.
    pub fn stop(mut self, trigger: Trigger) -> Self {
        self.stop = Some(trigger);
        self
    }
    /// Reload the current page whenever `trigger` fires.
    pub fn reload(mut self, trigger: Trigger) -> Self {
        self.reload = Some(trigger);
        self
    }
}

impl Piece for WebView {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let WebView {
            url,
            go,
            back,
            forward,
            stop,
            reload,
        } = self;
        let initial = WebProps {
            url: url.get_untracked(),
        };
        // A web view has no intrinsic size — it fills whatever space its container offers.
        let node = cx.leaf(
            KIND,
            &initial,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );

        let send = move |patch: WebPatch| {
            with_tree(|t| t.patch(node, Box::new(patch), false));
        };

        // Each command trigger → one patch. `watch` never fires for the initial value, so wiring
        // these does not issue a spurious command at build time (the initial URL loads via props).
        if let Some(go) = go {
            watch(
                move || go.track(),
                move |_, _| send(WebPatch::Load(url.get_untracked())),
            );
        }
        if let Some(back) = back {
            watch(move || back.track(), move |_, _| send(WebPatch::Back));
        }
        if let Some(forward) = forward {
            watch(move || forward.track(), move |_, _| send(WebPatch::Forward));
        }
        if let Some(stop) = stop {
            watch(move || stop.track(), move |_, _| send(WebPatch::Stop));
        }
        if let Some(reload) = reload {
            watch(move || reload.track(), move |_, _| send(WebPatch::Reload));
        }

        // Native navigation reports the current URL back via `Event::Custom` so a bound text field
        // follows along. In-process backends tag it "webview:url"; Android's cross-boundary Custom
        // carries just the payload. This node emits only that event, so any Custom is the URL — no
        // more hijacking `TextChanged` on Android (§8.2's opened event channel).
        cx.on(node, move |ev| {
            if let Event::Custom { text, .. } = ev {
                url.set(text.clone());
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend (this crate is a reference implementation,
// so each toolkit is split out for clarity). Each module registers a `Renderer` link-time into its
// backend's `RENDERERS` slice; `#[cfg]` gates each to its feature + target, and `#[path]` keeps the
// files grouped next to lib.rs.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "appkit", target_os = "macos"))]
#[path = "lib-appkit.rs"]
mod appkit_impl;

// GTK web view is Linux only — WebKitGTK 6 (webkit6) isn't viable on macOS and has no MSYS2 package
// on Windows, so both fall back to Day's placeholder leaf (see Cargo.toml's webkit6 target gate).
#[cfg(all(feature = "gtk", not(target_os = "macos"), not(windows)))]
#[path = "lib-gtk.rs"]
mod gtk_impl;

#[cfg(feature = "qt")]
#[path = "lib-qt.rs"]
mod qt_impl;

#[cfg(all(feature = "uikit", target_os = "ios"))]
#[path = "lib-uikit.rs"]
mod uikit_impl;

#[cfg(all(feature = "widget", target_os = "android"))]
#[path = "lib-android.rs"]
mod android_impl;

#[cfg(all(feature = "winui", windows))]
#[path = "lib-winui.rs"]
mod winui_impl;
