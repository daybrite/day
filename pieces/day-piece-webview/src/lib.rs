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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

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
    /// Evaluate `script` (already wrapped by [`wrap_script`]) and report the result back as an
    /// `Event::Custom` whose `num` is `req`. See docs/webview-eval.md.
    Eval {
        req: u64,
        script: String,
    },
}

// ---------------------------------------------------------------------------
// JavaScript evaluation (docs/webview-eval.md)
// ---------------------------------------------------------------------------

/// Field separator inside an evaluation reply. A raw 0x1F can never appear inside JSON text —
/// `JSON.stringify` escapes control characters as the six ASCII chars `\u001f` — so splitting on it
/// is unambiguous and needs no JSON parser on the Rust side.
const SEP: char = '\u{1f}';

/// Why an evaluation did not produce a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    /// This backend has no engine to evaluate in (web-dom's `<iframe>`, or no renderer at all).
    Unsupported,
    /// The script threw. `name`/`message` come from the caught exception; a cyclic value that
    /// `JSON.stringify` refuses arrives here too, as a `TypeError`.
    Threw { name: String, message: String },
    /// The web view was never realized, or went away before the reply arrived.
    ViewGone,
    /// The engine ran but the reply did not decode — the raw payload is included.
    Engine(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Unsupported => write!(f, "javascript evaluation is unsupported here"),
            EvalError::Threw { name, message } => write!(f, "{name}: {message}"),
            EvalError::ViewGone => write!(f, "the web view is gone"),
            EvalError::Engine(raw) => write!(f, "undecodable reply: {raw}"),
        }
    }
}

/// Escape `s` into a JavaScript string literal, quotes included.
///
/// U+2028 and U+2029 get named escapes because they are literal line terminators in JS source and
/// would end the string; everything below U+0020 goes to `\uXXXX` because a raw control character
/// is not legal inside a literal either.
fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Wrap a user script so every backend reports errors the same way.
///
/// Qt and Android have no error channel at all — a throw, a syntax error and a genuine `null` all
/// arrive identically — so the channel is built in JavaScript instead, and every backend then
/// behaves like the ones with real errors. The reply is `1␟<json>` or `0␟<name>␟<message>`.
///
/// The script is passed to `eval` as a **string literal** rather than spliced in as source. That
/// costs one escape pass and buys three things splicing cannot:
///
/// - **Syntax errors become catchable.** Spliced in, the wrapper and the script compile as one
///   unit, so a bad script kills the wrapper too and the backend reports its uninformative
///   "no result". `eval` compiles at run time, inside the `try`.
/// - **Statements work.** `throw new Error("x")` and `var a = 1; a + 1` are statements, so
///   `v = (…)` around them is itself a syntax error. `eval` takes a program and yields its
///   completion value, which is the behaviour a console user expects.
/// - **No lexical hazards.** A trailing `//` comment or an unbalanced brace in the script cannot
///   reach the wrapper's own tokens.
///
/// The cost is that a page whose Content-Security-Policy omits `unsafe-eval` refuses to run it;
/// that surfaces as a caught `EvalError`, which is at least legible.
///
/// Two `try` blocks, not one: the second catches `JSON.stringify` refusing a cyclic value, which is
/// a different failure from the script throwing.
fn wrap_script(script: &str) -> String {
    let src = js_string_literal(script);
    format!(
        "(function(){{var v;\
         try{{v=eval({src});}}\
         catch(e){{return \"0\\u001f\"+((e&&e.name)||\"Error\")+\"\\u001f\"+((e&&e.message)||String(e));}}\
         try{{var s=JSON.stringify(v);return \"1\\u001f\"+(s===undefined?\"null\":s);}}\
         catch(e){{return \"0\\u001f\"+((e&&e.name)||\"TypeError\")+\"\\u001f\"+((e&&e.message)||String(e));}}\
         }})()"
    )
}

/// Build a reply an arm can send when the ENGINE failed rather than the script — a dead content
/// process, a missing web view, a reply of the wrong type. Shaped like the wrapper's own error arm
/// so [`decode`] needs only one format.
// Used by the per-backend arms, each of which is `#[cfg]`-gated to one toolkit — so on any single
// build all but one caller is compiled out, and on a build whose backend has no eval arm yet there
// are none. Which callers exist is a build-configuration accident, not a sign this is unused.
#[allow(dead_code)]
pub(crate) fn engine_error(name: &str, message: &str) -> String {
    format!("0{SEP}{name}{SEP}{message}")
}

/// Decode one reply produced by [`wrap_script`]. `undefined` and values `JSON.stringify` drops
/// (a function, a symbol) both arrive as `null` — the wrapper normalizes them so the payload is
/// always valid JSON.
fn decode(payload: &str) -> Result<String, EvalError> {
    match payload.split_once(SEP) {
        Some(("1", json)) => Ok(json.to_string()),
        Some(("0", rest)) => {
            let (name, message) = rest.split_once(SEP).unwrap_or(("Error", rest));
            Err(EvalError::Threw {
                name: name.to_string(),
                message: message.to_string(),
            })
        }
        _ => Err(EvalError::Engine(payload.to_string())),
    }
}

struct EvalShared {
    result: RefCell<Option<Result<String, EvalError>>>,
    waker: RefCell<Option<std::task::Waker>>,
}

thread_local! {
    /// Request ids start at 1 so 0 stays free for the URL report, which shares this channel.
    static NEXT_REQ: Cell<u64> = const { Cell::new(1) };
    static PENDING: RefCell<HashMap<u64, Rc<EvalShared>>> = RefCell::new(HashMap::new());
}

/// Deliver a reply to whichever future is waiting on `req`. A reply for a dropped future finds
/// nothing pending and is discarded.
fn resolve(req: u64, payload: &str) {
    let Some(shared) = PENDING.with(|p| p.borrow_mut().remove(&req)) else {
        return;
    };
    *shared.result.borrow_mut() = Some(decode(payload));
    if let Some(waker) = shared.waker.borrow_mut().take() {
        waker.wake();
    }
}

/// A handle for running JavaScript in a web view. `Copy`, like `Trigger`, so it can be captured by
/// several closures. Bind it with [`WebView::js`]; evaluating before the view is realized fails
/// with [`EvalError::ViewGone`].
#[derive(Clone, Copy)]
pub struct JsHandle {
    node: Signal<Option<RNode>>,
}

impl Default for JsHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl JsHandle {
    #[track_caller]
    pub fn new() -> Self {
        JsHandle {
            node: Signal::new(None),
        }
    }

    /// Evaluate `script` and resolve with its result as JSON text.
    ///
    /// Nothing is dispatched until the future is polled. Dropping it deregisters the request, so a
    /// late reply is discarded — but the script keeps running: no backend can cancel one.
    pub fn eval(&self, script: impl AsRef<str>) -> EvalFuture {
        EvalFuture {
            req: NEXT_REQ.with(|c| {
                let v = c.get();
                c.set(v + 1);
                v
            }),
            node: self.node,
            script: Some(wrap_script(script.as_ref())),
            shared: Rc::new(EvalShared {
                result: RefCell::new(None),
                waker: RefCell::new(None),
            }),
            sent: false,
        }
    }
}

/// The pending result of [`JsHandle::eval`].
pub struct EvalFuture {
    req: u64,
    node: Signal<Option<RNode>>,
    script: Option<String>,
    shared: Rc<EvalShared>,
    sent: bool,
}

impl Future for EvalFuture {
    type Output = Result<String, EvalError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result);
        }
        // Dispatch on first poll, not at construction — an eval that is never awaited never runs.
        if !self.sent {
            self.sent = true;
            if eval_support() != day_spec::Support::Native {
                return Poll::Ready(Err(EvalError::Unsupported));
            }
            let Some(node) = self.node.get_untracked() else {
                return Poll::Ready(Err(EvalError::ViewGone));
            };
            let (req, script) = (self.req, self.script.take().unwrap_or_default());
            PENDING.with(|p| p.borrow_mut().insert(req, self.shared.clone()));
            with_tree(|t| t.patch(node, Box::new(WebPatch::Eval { req, script }), false));
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for EvalFuture {
    fn drop(&mut self) {
        PENDING.with(|p| p.borrow_mut().remove(&self.req));
    }
}

/// Whether this backend can evaluate JavaScript. A separate axis from [`support`]: web-dom loads
/// pages but cannot evaluate in them, so the two answers differ there.
pub fn eval_support() -> day_spec::Support {
    if cfg!(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
        feature = "qt",
    )) {
        day_spec::Support::Native
    } else {
        // GTK, Android, XAML and ArkUI all have an engine and an equivalent call; their arms are
        // not written yet (docs/webview-eval.md lists them in order). web-dom cannot ever do this
        // — `contentWindow.eval` throws across origins.
        day_spec::Support::Unsupported
    }
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
    js: Option<JsHandle>,
}

/// `web_view(url)` — a native web view showing `url`. The initial value loads on creation; call
/// `.go(trigger)` and fire the trigger to (re)load whatever `url` currently holds.
pub fn web_view(url: Signal<String>) -> WebView {
    // Self-register the web renderer. wasm has no link-time renderer slice, and a constructor is
    // the earliest point the piece is known to be in play — always before its node is realized.
    #[cfg(all(feature = "dom", target_arch = "wasm32"))]
    dom_impl::register();
    WebView {
        url,
        go: None,
        back: None,
        forward: None,
        stop: None,
        reload: None,
        js: None,
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
    /// Bind a [`JsHandle`] so `handle.eval(…)` runs in this view (docs/webview-eval.md).
    pub fn js(mut self, handle: JsHandle) -> Self {
        self.js = Some(handle);
        self
    }
}

/// What this backend realizes. `Native` is a real embedded browser engine with the full command
/// set; `Emulated` loads pages but cannot drive history or report navigation back (web-dom's
/// `<iframe>`, see docs/webview.md); `Unsupported` renders day's placeholder leaf.
///
/// Gate history controls on this: `.back()`, `.forward()` and `.stop()` are no-ops below `Native`,
/// so an app should disable those buttons rather than offer ones that do nothing.
pub fn support() -> day_spec::Support {
    // WebKitGTK 6 ships as a package only on Linux, so the gtk arm is compiled out on macos-gtk and
    // windows-gtk and those two combos realize the placeholder (Cargo.toml scopes `webkit6` to
    // match). Checked first: the `gtk` feature is on for all three.
    if cfg!(all(feature = "gtk", any(target_os = "macos", windows))) {
        day_spec::Support::Unsupported
    } else if cfg!(all(feature = "dom", target_arch = "wasm32")) {
        day_spec::Support::Emulated
    } else if cfg!(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
        all(feature = "mdc", target_os = "android"),
        all(feature = "gtk", not(target_os = "macos"), not(windows)),
        feature = "qt",
        all(feature = "xaml", windows),
        all(feature = "arkui", target_env = "ohos"),
    )) {
        day_spec::Support::Native
    } else {
        day_spec::Support::Unsupported
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
            js,
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

        // Bind the eval handle to the realized node so `handle.eval(…)` knows where to send.
        if let Some(js) = js {
            js.node.set(Some(node));
        }

        // Two kinds of report share this node's `Event::Custom` channel, told apart by `num`:
        // 0 is navigation (the URL, so a bound text field follows along), anything else is an
        // evaluation reply keyed by its request id. In-process backends also tag them, but a
        // cross-boundary Custom (JNI, C-ABI) carries only `num`/`text` — so `num` is the
        // discriminator that works everywhere (§8.2's opened event channel).
        cx.on(node, move |ev| {
            if let Event::Custom { num, text, .. } = ev {
                if *num >= 1.0 {
                    resolve(*num as u64, text);
                } else {
                    url.set(text.clone());
                }
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

day_pieces::glue_modules!(appkit, qt, uikit, mdc, xaml, arkui, dom);

// GTK web view is Linux only — WebKitGTK 6 (webkit6) isn't viable on macOS and has no MSYS2 package
// on Windows, so both fall back to Day's placeholder leaf (see Cargo.toml's webkit6 target gate).
#[cfg(all(feature = "gtk", not(target_os = "macos"), not(windows)))]
#[path = "lib-gtk.rs"]
mod gtk_impl;

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a reply the way an arm would, without a literal control char in the test source.
    fn reply(parts: &[&str]) -> String {
        parts.join(&SEP.to_string())
    }

    #[test]
    fn decodes_a_value() {
        assert_eq!(decode(&reply(&["1", "2"])), Ok("2".into()));
        assert_eq!(decode(&reply(&["1", "null"])), Ok("null".into()));
        let obj = r#"{"a":1,"b":"x"}"#;
        assert_eq!(decode(&reply(&["1", obj])), Ok(obj.into()));
    }

    #[test]
    fn decodes_a_throw() {
        assert_eq!(
            decode(&reply(&["0", "TypeError", "boom"])),
            Err(EvalError::Threw {
                name: "TypeError".into(),
                message: "boom".into(),
            })
        );
    }

    /// Only the FIRST separator splits the name off, so a message carrying more stays intact.
    #[test]
    fn a_message_may_contain_the_separator() {
        assert_eq!(
            decode(&reply(&["0", "Error", "a", "b"])),
            Err(EvalError::Threw {
                name: "Error".into(),
                message: reply(&["a", "b"]),
            })
        );
    }

    /// JSON text can never hold a RAW separator — `JSON.stringify` escapes control characters as
    /// six ASCII chars — so splitting on it cannot corrupt a value. This pins that.
    #[test]
    fn an_escaped_separator_inside_json_survives() {
        let json = r#""a\u001fb""#;
        assert_eq!(decode(&reply(&["1", json])), Ok(json.into()));
    }

    #[test]
    fn an_undecodable_reply_is_an_engine_error() {
        assert_eq!(decode(""), Err(EvalError::Engine(String::new())));
        // What a backend reports when the wrapper never ran at all.
        assert_eq!(decode("null"), Err(EvalError::Engine("null".into())));
    }

    /// The arms build engine failures with `engine_error`; it must decode like any other throw.
    #[test]
    fn engine_errors_round_trip() {
        assert_eq!(
            decode(&engine_error("WebKitError", "process gone")),
            Err(EvalError::Threw {
                name: "WebKitError".into(),
                message: "process gone".into(),
            })
        );
    }

    /// The script rides inside a string literal, so nothing in it can reach the wrapper's own
    /// tokens — a trailing line comment, an unbalanced brace, a quote, a newline.
    #[test]
    fn the_wrapper_is_lexically_sealed() {
        for hostile in [
            "1 + 1 // add",
            "\"unterminated",
            "}}})(){{{",
            "a\nb",
            "x\\y",
        ] {
            let js = wrap_script(hostile);
            assert!(
                js.trim_end().ends_with("})()"),
                "wrapper not closed for {hostile:?}: {js}"
            );
            assert!(
                !js.contains("\n"),
                "raw newline leaked for {hostile:?}: {js}"
            );
        }
    }

    #[test]
    fn escapes_a_script_into_a_literal() {
        assert_eq!(js_string_literal("a"), "\"a\"");
        assert_eq!(js_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_string_literal("a\\b"), "\"a\\\\b\"");
        assert_eq!(js_string_literal("a\nb"), "\"a\\nb\"");
        // A literal line terminator would end the string mid-source.
        assert_eq!(js_string_literal("a\u{2028}b"), "\"a\\u2028b\"");
        assert_eq!(js_string_literal("a\u{1}b"), "\"a\\u0001b\"");
    }
}
