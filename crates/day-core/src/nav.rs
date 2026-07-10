//! Route registry (docs/navigation.md, docs/tabs.md): mounted `nav()` / `tabs()` hosts each
//! register a controller here. Registrations form a STACK so hosts can nest — e.g. a `tabs()`
//! inside a `nav()` route — and the stack order IS the nesting order (outermost first).
//!
//! Two addressing modes (docs/navigation.md):
//!   * A single key (`navigate("inbox")`) is RELATIVE: tried innermost-first, falling through
//!     outward — a tab key selects the tab, a key the tabs host doesn't know still resolves
//!     against the enclosing surface.
//!   * A `/`-separated path (`navigate("mail/inbox/msg-42")`) is ABSOLUTE: the first segment
//!     anchors at the outermost surface that recognizes it, every surface INSIDE the anchor is
//!     reset to its root, and the remaining segments are consumed inward — including by
//!     surfaces that only mount as the outer switch takes effect (a pending queue hands each
//!     newly registered surface the next segment).
//!
//! A trailing `?name=value&…` query carries [`route_params`] to the destination builders.
//! [`current_route`] reports the FULL path — every mounted surface's contribution, outermost
//! to innermost — so persisting navigation is `save(current_route())` + `navigate(&saved)`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A mounted host's control surface. Closures run user code (route builders), so the registry
/// NEVER holds a borrow across a call (§3.3 discipline: clone the `Rc` out, then call).
pub struct NavController {
    /// Push (or, in split/tab presentation, select) a registered route. False = unknown route.
    pub push: Box<dyn Fn(&str) -> bool>,
    /// Pop the top route. `already_popped` = the native side popped first (iOS back).
    /// False = nothing to pop (tabs hosts always return false: they have no stack).
    pub pop: Box<dyn Fn(bool) -> bool>,
    /// Current route path ("" while showing the root).
    pub current: Box<dyn Fn() -> String>,
    /// Consume one segment of an ABSOLUTE path. Selectors/tabs accept a declared key (same as
    /// `push`); a `stack` accepts ANY segment by pushing it (its destinations are open-ended).
    /// Distinct from `push` so a relative `navigate("key")` can still fall through a stack.
    pub enter: Box<dyn Fn(&str) -> bool>,
    /// This surface's contribution to the full route: `[]` at root, `[key]` for a selector /
    /// tabs, the whole path for a stack.
    pub segments: Box<dyn Fn() -> Vec<String>>,
}

/// Opaque handle from [`register_nav`]; a nested host calls [`unregister_nav`] on dispose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NavToken(u64);

thread_local! {
    static NAV_STACK: RefCell<Vec<(NavToken, Rc<NavController>)>> =
        const { RefCell::new(Vec::new()) };
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    /// Query params of the most recent `navigate()` (empty between navigations). Destination
    /// builders read them via [`route_params`] while their route is being entered.
    static PARAMS: RefCell<Rc<Vec<(String, String)>>> = RefCell::new(Rc::new(Vec::new()));
    /// Absolute-path segments not yet consumed: surfaces that mount during the navigation
    /// cascade take the front segment(s) as they register (see [`register_nav`]).
    static PENDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Install a controller (innermost = last). Returns its token. The root `nav()` registers once
/// and never unregisters; nested hosts (`tabs()` in a route) unregister when their scope disposes.
///
/// If an absolute navigation left unconsumed segments, the new surface consumes as many leading
/// ones as it accepts — this is how `navigate("mail/inbox/msg-42")` reaches a stack that only
/// mounts once the "mail" switch has taken effect.
pub fn register_nav(ctrl: NavController) -> NavToken {
    let token = NEXT_TOKEN.with(|c| {
        let t = c.get();
        c.set(t + 1);
        NavToken(t)
    });
    let ctrl = Rc::new(ctrl);
    NAV_STACK.with(|s| s.borrow_mut().push((token, ctrl.clone())));
    // Feed pending absolute segments to the just-mounted surface (front-first, stop at the
    // first refusal — deeper segments wait for deeper surfaces).
    while let Some(front) = PENDING.with(|p| p.borrow().first().cloned()) {
        if !(ctrl.enter)(&front) {
            break;
        }
        PENDING.with(|p| {
            p.borrow_mut().remove(0);
        });
    }
    token
}

/// Remove a controller whose host was disposed. No-op if already gone.
pub fn unregister_nav(token: NavToken) {
    NAV_STACK.with(|s| s.borrow_mut().retain(|(t, _)| *t != token));
}

/// Drop every controller — a fresh mount / test boot (called from tree install/uninstall).
pub fn clear_controllers() {
    NAV_STACK.with(|s| s.borrow_mut().clear());
    NEXT_TOKEN.with(|c| c.set(1));
    PENDING.with(|p| p.borrow_mut().clear());
    PARAMS.with(|p| *p.borrow_mut() = Rc::new(Vec::new()));
}

/// Dispatch innermost→outermost; the first controller that returns true wins. Controllers are
/// `Rc`-cloned out of the stack before the call, so their closures (which re-enter the tree and
/// may register/unregister hosts) never run while the stack is borrowed (§3.3).
fn dispatch(f: impl Fn(&NavController) -> bool) -> bool {
    let controllers: Vec<Rc<NavController>> =
        NAV_STACK.with(|s| s.borrow().iter().rev().map(|(_, c)| c.clone()).collect());
    for c in controllers {
        if f(&c) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Route strings (docs/navigation.md): `seg/seg/seg?name=value&name2=value2`
// ---------------------------------------------------------------------------

/// Split a route string into its path segments and query params. Segments and param
/// names/values are percent-decoded (`%2F` → `/`, …); everything else is taken literally.
pub fn parse_route(route: &str) -> (Vec<String>, Vec<(String, String)>) {
    let (path, query) = match route.split_once('?') {
        Some((p, q)) => (p, q),
        None => (route, ""),
    };
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect();
    let params: Vec<(String, String)> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect();
    (segments, params)
}

/// Assemble a route string from segments and params — the inverse of [`parse_route`].
/// Reserved characters (`/`, `?`, `&`, `=`, `%`) in segments and params are percent-encoded.
pub fn encode_route(segments: &[String], params: &[(String, String)]) -> String {
    let mut out = segments
        .iter()
        .map(|s| percent_encode(s))
        .collect::<Vec<_>>()
        .join("/");
    if !params.is_empty() {
        out.push('?');
        out.push_str(
            &params
                .iter()
                .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len() + 1
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'/' | b'?' | b'&' | b'=' | b'%' => out.push_str(&format!("%{b:02X}")),
            _ => out.push(b as char),
        }
    }
    out
}

/// The query params carried by the most recent [`navigate`] call (`?name=value&…`). Read them
/// inside a destination builder: `route_param("id")`. They describe the navigation in flight —
/// a push you perform by writing a path signal directly carries its data in your own state
/// instead (docs/navigation.md).
pub fn route_params() -> Rc<Vec<(String, String)>> {
    PARAMS.with(|p| p.borrow().clone())
}

/// The value of one query param of the most recent [`navigate`] (`None` = not present).
pub fn route_param(name: &str) -> Option<String> {
    route_params()
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

/// Navigate to a route (docs/navigation.md).
///
/// * `""` — pop the innermost stack to its root (falls through outward).
/// * A single key — RELATIVE: innermost surface first, falling through outward.
/// * `a/b/c` — ABSOLUTE: anchor at the outermost surface that knows `a`, reset every surface
///   inside the anchor to its root, then feed `b`, `c`, … inward (surfaces that mount during
///   the cascade consume the rest as they register).
/// * A trailing `?name=value&…` carries [`route_params`] to the destination builders.
///
/// False = no mounted surface recognized the (first) segment.
pub fn navigate(route: &str) -> bool {
    let (segments, params) = parse_route(route);
    PENDING.with(|p| p.borrow_mut().clear());
    PARAMS.with(|p| *p.borrow_mut() = Rc::new(params));
    match segments.len() {
        0 => dispatch(|nav| (nav.push)("")),
        1 => dispatch(|nav| (nav.push)(&segments[0])),
        _ => navigate_absolute(&segments),
    }
}

/// Anchor + descend for a multi-segment path. See [`navigate`].
///
/// Signal writes may propagate SYNCHRONOUSLY (an un-batched set cascades immediately), so the
/// surfaces an anchor switch mounts can register — and must find their segments waiting —
/// before the anchoring `push` even returns. Hence: queue the tail FIRST, then anchor.
fn navigate_absolute(segments: &[String]) -> bool {
    let snapshot = || -> Vec<Rc<NavController>> {
        NAV_STACK.with(|s| s.borrow().iter().map(|(_, c)| c.clone()).collect())
    };
    let controllers = snapshot();
    let first = &segments[0];

    // Already anchored: some surface is showing `first`. Reset everything inside it to its
    // root (innermost-first, so stacks pop cleanly), then feed the remaining segments to the
    // surviving inner surfaces in nesting order. Consult the LIVE registry after the resets —
    // a reset can dispose deeper surfaces (a popped page takes its sub-surfaces with it).
    if let Some(anchor) = controllers.iter().position(|c| (c.current)() == *first) {
        PENDING.with(|p| *p.borrow_mut() = segments[1..].to_vec());
        for c in controllers[anchor + 1..].iter().rev() {
            let _ = (c.push)("");
        }
        let live = snapshot();
        if let Some(anchor) = live.iter().position(|c| (c.current)() == *first) {
            for c in live[anchor + 1..].iter() {
                while let Some(front) = PENDING.with(|p| p.borrow().first().cloned()) {
                    if !(c.enter)(&front) {
                        break;
                    }
                    PENDING.with(|p| {
                        p.borrow_mut().remove(0);
                    });
                }
            }
        }
        return true;
    }

    // Switching: queue the tail so surfaces that mount during the (possibly synchronous)
    // cascade consume it as they register, then anchor at the outermost surface that accepts
    // the first segment.
    PENDING.with(|p| *p.borrow_mut() = segments[1..].to_vec());
    for c in controllers.iter() {
        if (c.push)(first) {
            return true;
        }
    }
    PENDING.with(|p| p.borrow_mut().clear());
    false
}

/// Pop one level, day-initiated (the toolkit presents the pop). Native-initiated pops arrive as
/// `Event::NavBack` and go through the owning host's `pop` directly.
pub fn nav_back() -> bool {
    dispatch(|nav| (nav.pop)(false))
}

/// The FULL current route: every mounted surface's contribution, outermost to innermost,
/// `/`-joined (docs/navigation.md). `None` = no surface mounted; `Some("")` = everything at
/// its root. Round-trips through [`navigate`], so persisting navigation state is
/// `save(current_route())` on the way out and `navigate(&saved)` on the way back in.
pub fn current_route() -> Option<String> {
    let controllers: Vec<Rc<NavController>> =
        NAV_STACK.with(|s| s.borrow().iter().map(|(_, c)| c.clone()).collect());
    if controllers.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for c in controllers {
        parts.extend((c.segments)());
    }
    Some(encode_route(&parts, &[]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_parsing_round_trips() {
        let (segs, params) = parse_route("mail/inbox/msg-42?hint=linked&x=1");
        assert_eq!(segs, vec!["mail", "inbox", "msg-42"]);
        assert_eq!(
            params,
            vec![("hint".into(), "linked".into()), ("x".into(), "1".into())]
        );
        assert_eq!(
            encode_route(&segs, &params),
            "mail/inbox/msg-42?hint=linked&x=1"
        );

        // Reserved characters survive a round trip.
        let segs = vec!["a/b".to_string()];
        let params = vec![("q".to_string(), "1&2=3".to_string())];
        let encoded = encode_route(&segs, &params);
        let (s2, p2) = parse_route(&encoded);
        assert_eq!(s2, segs);
        assert_eq!(p2, params);
    }

    #[test]
    fn route_parsing_edge_cases() {
        assert_eq!(parse_route(""), (vec![], vec![]));
        assert_eq!(parse_route("a"), (vec!["a".to_string()], vec![]));
        assert_eq!(parse_route("a//b").0, vec!["a", "b"]); // empty segments dropped
        assert_eq!(
            parse_route("?flag").1,
            vec![("flag".to_string(), String::new())]
        );
    }
}
