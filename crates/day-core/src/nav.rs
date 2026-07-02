//! Route registry (docs/navigation.md): the mounted `nav()` host registers a controller
//! here; dayscript steps, deep links, and `navigator()` handles all navigate through it.
//! Thread-local like the tree — one UI thread, one active host (v1: nav is app-root only).

use std::cell::RefCell;

/// The active nav host's control surface. Closures run user code (route builders), so the
/// registry NEVER holds a borrow across a call (§3.3 discipline: take, call, restore).
pub struct NavController {
    /// Push (or, in split presentation, select) a registered route. False = unknown route.
    pub push: Box<dyn Fn(&str) -> bool>,
    /// Pop the top route. `already_popped` = the native side popped first (iOS back).
    /// False = nothing to pop.
    pub pop: Box<dyn Fn(bool) -> bool>,
    /// Current route path ("" while showing the root).
    pub current: Box<dyn Fn() -> String>,
}

thread_local! {
    static ACTIVE_NAV: RefCell<Option<NavController>> = const { RefCell::new(None) };
}

/// Install the controller (called by the nav piece at build; replaces any previous host).
pub fn register_nav(ctrl: NavController) {
    ACTIVE_NAV.with(|n| *n.borrow_mut() = Some(ctrl));
}

fn with_nav<R>(f: impl FnOnce(&NavController) -> R) -> Option<R> {
    // Take-call-restore: the controller's closures re-enter the tree and user builders.
    let ctrl = ACTIVE_NAV.with(|n| n.borrow_mut().take())?;
    let out = f(&ctrl);
    ACTIVE_NAV.with(|n| {
        let mut slot = n.borrow_mut();
        if slot.is_none() {
            *slot = Some(ctrl);
        }
    });
    Some(out)
}

/// Navigate to a registered route ("" pops to root). False = no host / unknown route.
pub fn navigate(path: &str) -> bool {
    with_nav(|nav| (nav.push)(path)).unwrap_or(false)
}

/// Pop one level, day-initiated (the toolkit presents the pop). Native-initiated pops
/// arrive as `Event::NavBack` and go through the controller's `pop` closure directly.
pub fn nav_back() -> bool {
    with_nav(|nav| (nav.pop)(false)).unwrap_or(false)
}

/// Current route path (None = no nav host mounted; "" = showing the root).
pub fn current_route() -> Option<String> {
    with_nav(|nav| (nav.current)())
}
