// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Per-window ambient state (docs/size-classes.md): the facts a backend reports about a WINDOW
//! rather than about the app — its [`SizeClass`] and its safe-area insets.
//!
//! Both are reactive, so a piece that reads one re-runs when the backend reports a new value, and
//! both are keyed by window root for the same reason [`crate::toolbar`] is: one process can show
//! two windows at different sizes at once (a narrow window beside a wide one, iPadOS Stage
//! Manager, Android split-screen), and a single global would lay the second window out for the
//! first one's size.
//!
//! Signals are created in the ROOT reactive scope. Backends report from native callbacks that can
//! fire while some transient scope is current, and a signal owned by a scope that later disposes
//! would take the window's state with it.

use std::cell::RefCell;

use day_reactive::Signal;
use day_spec::SizeClass;

use crate::tree::{RNode, with_tree};

/// The reactive values one window carries.
#[derive(Clone, Copy)]
struct WindowAmbient {
    /// `None` until the backend reports one. The distinction matters: a backend that does not
    /// participate in size classes yet must not be read as "this window is compact", or every
    /// host on it would resolve to a stack. Callers treat `None` as "ask the toolkit instead".
    size_class: Signal<Option<SizeClass>>,
    /// The last class the BACKEND reported, kept beside the live one so a script that forces a
    /// class has something true to go back to (`size_class: { width: auto }`).
    reported_class: Signal<Option<SizeClass>>,
    safe_area: Signal<day_geometry::Insets>,
}

day_reactive::tls_slots! {
    ambient;
    static AMBIENT: RefCell<Vec<(RNode, WindowAmbient)>> = const { RefCell::new(Vec::new()) };
}

/// One window's signals, created on first touch.
///
/// Returns them by value and drops the registry borrow before the caller touches either. Writing
/// a signal runs its bindings SYNCHRONOUSLY, and those bindings read ambient state — a nav host
/// re-presenting on a class change reads the class right back — so holding the borrow across a
/// write is a reentrant panic waiting to happen.
fn ambient_of(root: RNode) -> WindowAmbient {
    AMBIENT.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, a)) = m.iter().find(|(r, _)| *r == root) {
            return *a;
        }
        let ambient = day_reactive::Scope::root().enter(|| WindowAmbient {
            size_class: Signal::global(None),
            reported_class: Signal::global(None),
            safe_area: Signal::global(initial_safe_area()),
        });
        m.push((root, ambient));
        ambient
    })
}

/// Seeded from the launch environment so a page built during startup — before the backend's first
/// live report lands — already sees the right top inset (the same lazy-env pattern as
/// `layout_direction`/`DAY_LOCALE`). Points, not px.
fn initial_safe_area() -> day_geometry::Insets {
    std::env::var("DAY_SAFE_AREA_TOP")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|top| day_geometry::Insets {
            top,
            ..Default::default()
        })
        .unwrap_or_default()
}

/// The window a read or report targets: the one being built, else the primary root. Shared with
/// [`crate::toolbar`], which scopes the same way for the same reason — an app's one `toolbar(…)`
/// or `size_class()` call inside a shared `build_shell` must mean "this window".
fn target_window() -> RNode {
    crate::toolbar::current_window()
}

/// The current window's size class, or `None` on a backend that does not report one yet.
/// Tracked: a piece that lays out from this rebuilds when the window crosses a breakpoint.
///
/// Outside a window's content build this reads the PRIMARY window, so a long-lived effect that
/// wants a secondary window's class must capture its root at build time and use
/// [`window_size_class`] — the same discipline `toolbar_reactive` follows.
pub fn size_class() -> Option<SizeClass> {
    window_size_class(target_window())
}

/// [`size_class`] against an explicit window root.
pub fn window_size_class(root: RNode) -> Option<SizeClass> {
    ambient_of(root).size_class.get()
}

/// [`size_class`] without subscribing — for code that reacts to the change itself.
pub fn window_size_class_untracked(root: RNode) -> Option<SizeClass> {
    ambient_of(root).size_class.get_untracked()
}

/// Backend-facing: report a window's size class. Call whenever the window's size changes; the
/// signal only notifies when the BUCKET changes, so a resize within one class is free.
pub fn set_window_size_class(root: RNode, class: SizeClass) {
    ambient_of(root).reported_class.set(Some(class));
    let signal = ambient_of(root).size_class;
    if signal.get_untracked() == Some(class) {
        return;
    }
    signal.set(Some(class));
    with_tree(|t| t.layout_if_needed());
}

/// [`set_window_size_class`] against the primary window — what a single-window backend reports.
pub fn set_size_class(class: SizeClass) {
    set_window_size_class(primary_root(), class);
}

/// Force a class the window is not actually at — dayscript's `size_class:` step and tests.
///
/// Deliberately does NOT touch the reported class: this is a claim ABOUT the window, not a report
/// FROM it, so [`restore_reported_size_class`] can still put back what the backend last said.
pub fn override_size_class(class: SizeClass) {
    let root = primary_root();
    let signal = ambient_of(root).size_class;
    if signal.get_untracked() == Some(class) {
        return;
    }
    signal.set(Some(class));
    with_tree(|t| t.layout_if_needed());
}

/// Undo an [`override_size_class`]: back to the class the window itself last reported.
///
/// A no-op where the backend has reported nothing yet — there is no truth to restore, and the
/// forced class is better than none.
pub fn restore_reported_size_class() {
    let root = primary_root();
    let Some(class) = ambient_of(root).reported_class.get_untracked() else {
        return;
    };
    let signal = ambient_of(root).size_class;
    if signal.get_untracked() == Some(class) {
        return;
    }
    signal.set(Some(class));
    with_tree(|t| t.layout_if_needed());
}

/// The window's safe-area insets, in points. Zero on every backend that clamps Day's root to
/// the safe area natively (the default everywhere); nonzero only where a backend runs the root
/// edge-to-edge — today day-android's opt-in immersive mode (docs/layout.md, the android
/// platform page). Compose it yourself where a background should run under the system bars:
/// paint the background unpadded, pad the content by these insets. The read is tracked, but
/// layout attributes like `.padding` capture the value at build time — a mid-run inset change
/// (rotation) does not re-pad already-built pages.
pub fn safe_area() -> day_geometry::Insets {
    window_safe_area(target_window())
}

/// [`safe_area`] against an explicit window root.
pub fn window_safe_area(root: RNode) -> day_geometry::Insets {
    ambient_of(root).safe_area.get()
}

/// Backend-facing: report a window's safe-area insets (points). Call from the native inset pass
/// whenever the value changes; apps observe it through [`safe_area`].
pub fn set_window_safe_area(root: RNode, insets: day_geometry::Insets) {
    ambient_of(root).safe_area.set(insets);
}

/// [`set_window_safe_area`] against the primary window — what a single-window backend reports.
pub fn set_safe_area(insets: day_geometry::Insets) {
    set_window_safe_area(primary_root(), insets);
}

fn primary_root() -> RNode {
    with_tree(|t| t.root_node())
}

/// Drop a closed window's ambient state (called from the window teardown path).
pub(crate) fn forget_window(root: RNode) {
    AMBIENT.with(|m| m.borrow_mut().retain(|(r, _)| *r != root));
}

/// Reset every window's ambient state (tests — pairs with `uninstall_tree`).
pub fn reset_ambient() {
    AMBIENT.with(|m| m.borrow_mut().clear());
}
