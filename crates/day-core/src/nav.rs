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
    /// A launch deep link recorded by the platform entry before `launch_with` runs — the seam
    /// for hosts with no process environment (web-dom seeds it from the page's URL hash). The
    /// `DAY_DEEPLINK` environment variable, where one exists, still wins.
    static LAUNCH_DEEPLINK: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Query params of the most recent `navigate()` (empty between navigations). Destination
    /// builders read them via [`route_params`] while their route is being entered.
    static PARAMS: RefCell<Rc<Vec<(String, String)>>> = RefCell::new(Rc::new(Vec::new()));
    /// Absolute-path segments not yet consumed: surfaces that mount during the navigation
    /// cascade take the front segment(s) as they register (see [`register_nav`]).
    static PENDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The app-installed persistence sink a nav surface's `.restore` reads and writes through
    /// ([`set_nav_store`]). `None` = no store installed, so `.restore` is a no-op.
    static NAV_STORE: RefCell<Option<Rc<dyn NavStore>>> = const { RefCell::new(None) };
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

/// Record the host's launch deep link before `launch_with` runs (docs/navigation.md).
/// Platform glue only — apps navigate with [`navigate`]. `DAY_DEEPLINK`, where a process
/// environment exists, takes precedence.
///
/// UI THREAD ONLY: the slot is thread-local, because the web host that seeds it runs on the one
/// thread there is. Glue that may be called from another thread (a notification tap arriving on a
/// JNI or delegate thread) wants [`request_route`], which is thread-safe and works at any
/// lifecycle stage.
pub fn set_launch_deeplink(route: &str) {
    LAUNCH_DEEPLINK.with(|l| *l.borrow_mut() = Some(route.to_string()));
}

/// A route requested from outside the reactive turn, at any lifecycle stage, from any thread —
/// the rail a notification tap navigates through (docs/notify.md).
///
/// This is process-global rather than thread-local ([`LAUNCH_DEEPLINK`] is the latter) because the
/// caller is usually NOT on the UI thread: Android delivers a tap on a JNI thread and Apple on a
/// delegate callback, and both can arrive before `launch_with` has run at all.
static REQUESTED_ROUTE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn requested_slot() -> std::sync::MutexGuard<'static, Option<String>> {
    // A panic while holding this lock would otherwise wedge every later navigation; the buffer is
    // one Option<String>, so recovering the value is always safe.
    REQUESTED_ROUTE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ask the app to navigate, from anywhere, at any time.
///
/// Cold start (a tap that launches the process) and warm tap (the app is already running) are the
/// same call: before launch the route is buffered and `launch_with` applies it after the first
/// mount, so it lands once routes actually exist; after launch it is applied on the UI thread.
/// An empty route is ignored, and the newest request wins — a user who taps twice gets the second
/// destination.
pub fn request_route(route: &str) {
    if route.is_empty() {
        return;
    }
    *requested_slot() = Some(route.to_string());
    // Before a backend installs the poster there is no UI thread to post to; the launch drain
    // picks the buffer up instead.
    if day_reactive::has_main_poster() {
        day_reactive::on_main(|| {
            if let Some(route) = take_requested_route() {
                apply_route_request(&route);
            }
        });
    }
}

/// Take the buffered route, if any. `launch_with` drains it after the first mount.
pub(crate) fn take_requested_route() -> Option<String> {
    requested_slot().take()
}

/// Apply a route the backend or a part asked for. Echoes of our own `set_route` match the current
/// route and are dropped.
pub(crate) fn apply_route_request(route: &str) {
    if current_route().as_deref() != Some(route) && !navigate(route) {
        eprintln!("day: requested route {route:?} did not match");
    }
}

/// Whether a launch deep link is pending (`DAY_DEEPLINK` or a platform hint). A nav surface's
/// `.restore` reads this so a deep link wins over restored state (docs/navigation.md).
pub fn has_launch_deeplink() -> bool {
    launch_deeplink().is_some()
}

/// A key/value sink a nav surface's `.restore` persists its state through, so navigation
/// survives a relaunch or an Android process death (docs/navigation.md). The framework never
/// installs one — an app opts in by installing a store (e.g. `day_part_prefs::install_nav_store`)
/// and marking the surfaces it wants remembered with `.restore(key)`. With no store installed,
/// `.restore` is a silent no-op, so a surface's `.restore` call never fails to build.
///
/// Keys are opaque to day-core and namespaced by the surface's `.restore(key)` argument; the
/// store implementation is responsible for keeping them clear of the app's own data.
pub trait NavStore {
    /// The last value saved under `key`, or `None` if nothing was stored (first launch).
    fn load(&self, key: &str) -> Option<String>;
    /// Persist `value` under `key`, replacing any prior value.
    fn save(&self, key: &str, value: &str);
}

/// Install the app's navigation persistence store (docs/navigation.md). Call once at startup,
/// before the UI mounts, so a `.restore` surface reads it on first build. A later call replaces
/// the store.
pub fn set_nav_store(store: Rc<dyn NavStore>) {
    NAV_STORE.with(|s| *s.borrow_mut() = Some(store));
}

/// Read a `.restore` key through the installed [`NavStore`] (`None` if none is installed or the
/// key was never saved).
pub fn nav_store_load(key: &str) -> Option<String> {
    match NAV_STORE.with(|s| s.borrow().clone()) {
        Some(st) => st.load(key),
        None => {
            warn_no_nav_store(key);
            None
        }
    }
}

/// Debug-build diagnostic, once per process: a surface asked to `.restore(key)` while no
/// [`NavStore`] is installed persists nothing, and does so SILENTLY — the surface still builds,
/// the app still runs, and the setting simply never survives a relaunch. That is a deliberate
/// design (`.restore` must never fail to build), but it reads as a bug, so say it out loud where
/// a developer will see it.
fn warn_no_nav_store(key: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    thread_local! {
        static WARNED: Cell<bool> = const { Cell::new(false) };
    }
    WARNED.with(|w| {
        if !w.replace(true) {
            crate::diag(format_args!(
                "day: .restore({key:?}) has no NavStore installed — navigation state will not \
                 persist. Call `day::prefs::install_nav_store()` before the UI mounts \
                 (docs/navigation.md)."
            ));
        }
    });
}

/// Write a `.restore` key through the installed [`NavStore`] (a no-op if none is installed).
pub fn nav_store_save(key: &str, value: &str) {
    if let Some(st) = NAV_STORE.with(|s| s.borrow().clone()) {
        st.save(key, value);
    }
}

/// The launch deep link: the `DAY_DEEPLINK` environment variable, else the platform's
/// [`set_launch_deeplink`] hint. Consumed by `launch_with` after the first mount.
pub(crate) fn launch_deeplink() -> Option<String> {
    std::env::var("DAY_DEEPLINK")
        .ok()
        .filter(|r| !r.is_empty())
        // A route buffered by `request_route` before launch — a notification tap that cold-started
        // the process. Peeked, not taken, so `has_launch_deeplink()` keeps answering true for a
        // nav surface's `.restore` (a tap must beat restored state); `launch_with` takes it.
        .or_else(|| requested_slot().clone())
        .or_else(|| LAUNCH_DEEPLINK.with(|l| l.borrow().clone()))
        .filter(|r| !r.is_empty())
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

    /// `REQUESTED_ROUTE` is process-global (it must be — see `request_route`), so the tests that
    /// touch it cannot run in parallel with each other. Serialize them rather than making the
    /// production type thread-local, which would defeat the point of the buffer.
    static ROUTE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn route_test<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ROUTE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_requested_route();
        let out = f();
        let _ = take_requested_route();
        out
    }

    /// A tap that cold-starts the process buffers a route, and the launch path sees it — without
    /// this, `launch_deeplink()` would miss it and the app would open on its default screen.
    #[test]
    fn requested_route_before_launch_is_visible_to_the_launch_path() {
        route_test(|| {
            let _ = take_requested_route(); // isolate from other tests in this process
            assert_eq!(launch_deeplink(), None);
            request_route("clock/timer");
            assert_eq!(launch_deeplink().as_deref(), Some("clock/timer"));
            // Peeked, not taken: a nav surface's `.restore` asks this so a tap beats restored state.
            assert!(has_launch_deeplink());
            assert_eq!(take_requested_route().as_deref(), Some("clock/timer"));
            assert_eq!(take_requested_route(), None);
        });
    }

    /// The newest tap wins: a user who taps a second notification before launch completes gets
    /// the second destination, not the first.
    #[test]
    fn newest_requested_route_wins() {
        route_test(|| {
            let _ = take_requested_route();
            request_route("mail/inbox");
            request_route("clock/alarm");
            assert_eq!(take_requested_route().as_deref(), Some("clock/alarm"));
        });
    }

    /// An empty route is not a navigation request — `navigate("")` means "pop to root", which a
    /// missing intent extra must never trigger.
    #[test]
    fn empty_requested_route_is_ignored() {
        route_test(|| {
            let _ = take_requested_route();
            request_route("");
            assert_eq!(take_requested_route(), None);
        });
    }

    /// `request_route` must not panic when no backend has started, which is exactly the state a
    /// cold-start notification tap arrives in (`on_main` panics without a poster).
    #[test]
    fn request_route_does_not_require_a_running_backend() {
        route_test(|| {
            assert!(!day_reactive::has_main_poster());
            request_route("some/route"); // would panic if it posted unconditionally
            let _ = take_requested_route();
        });
    }
}
