//! Route registry (docs/navigation.md, docs/tabs.md): mounted `nav()` / `tabs()` hosts each
//! register a controller here. Registrations form a STACK so hosts can nest — e.g. a `tabs()`
//! inside a `nav()` route — with `navigate()`/`nav_back()` trying the innermost host first and
//! falling through outward. A tab key thus selects the tab, while a route the tabs host does
//! not know still resolves against the enclosing `nav()`. `current_route()` reports the
//! innermost host's active path. Thread-local like the tree — one UI thread.

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
}

/// Opaque handle from [`register_nav`]; a nested host calls [`unregister_nav`] on dispose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NavToken(u64);

thread_local! {
    static NAV_STACK: RefCell<Vec<(NavToken, Rc<NavController>)>> =
        const { RefCell::new(Vec::new()) };
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
}

/// Install a controller (innermost = last). Returns its token. The root `nav()` registers once
/// and never unregisters; nested hosts (`tabs()` in a route) unregister when their scope disposes.
pub fn register_nav(ctrl: NavController) -> NavToken {
    let token = NEXT_TOKEN.with(|c| {
        let t = c.get();
        c.set(t + 1);
        NavToken(t)
    });
    NAV_STACK.with(|s| s.borrow_mut().push((token, Rc::new(ctrl))));
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

/// Navigate to a registered route ("" pops to root). False = no host / route unknown everywhere.
pub fn navigate(path: &str) -> bool {
    dispatch(|nav| (nav.push)(path))
}

/// Pop one level, day-initiated (the toolkit presents the pop). Native-initiated pops arrive as
/// `Event::NavBack` and go through the owning host's `pop` directly.
pub fn nav_back() -> bool {
    dispatch(|nav| (nav.pop)(false))
}

/// Current route path of the innermost host (None = no host mounted; "" = showing its root).
pub fn current_route() -> Option<String> {
    let ctrl = NAV_STACK.with(|s| s.borrow().last().map(|(_, c)| c.clone()))?;
    Some((ctrl.current)())
}
