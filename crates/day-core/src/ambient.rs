// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Ambient state — values a piece reads without being handed them.
//!
//! Two families live here. The first is what a BACKEND reports about a window
//! (docs/size-classes.md): its [`SizeClass`] and its safe-area insets. The second is what an
//! APP provides for its own subtree (docs/state.md): [`with_environment`] / [`environment`],
//! and the [`Ambient`] trait over them that carries per-window and app-wide state.
//!
//! They sit together because they answer the same question from opposite ends, and because the
//! app half belongs BELOW day-pieces: an app's `*-core` crate holds its view-model and has to be
//! able to `impl Ambient` for it, which the orphan rule forbids when the trait lives one crate
//! further up.
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

use day_reactive::{Scope, Signal};
use day_spec::SizeClass;

use crate::build::Piece;
use crate::{AnyPiece, piece_fn};

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
// ---------------------------------------------------------------------------
// @Environment — ambient values over day-reactive's scope context (§4.3). No backend work.
// ---------------------------------------------------------------------------

/// Provide an ambient value `T` to `content` and its ENTIRE descendant subtree (the SwiftUI
/// `@Environment`/`.environment(_)` analog, layered over day-reactive's scope context). `content`
/// — and any piece built within it — reads it back with [`environment`]. A thin, non-reactive
/// wrapper: `T` is a snapshot captured here; for a value that must react, provide a `Signal<T>`
/// (or a `Memo<T>`) and read it reactively inside the subtree.
///
/// ```ignore
/// #[derive(Clone)] struct Theme { accent: Color }
/// with_environment(Theme { accent: BLUE }, || my_screen())
/// // deep inside my_screen():  let accent = environment::<Theme>().unwrap().accent;
/// ```
pub fn with_environment<T: Clone + 'static, P: Piece>(
    value: T,
    content: impl FnOnce() -> P + 'static,
) -> impl Piece {
    piece_fn(move |cx| {
        // A child scope carrying `T`, entered for the whole of `content`'s construction AND build,
        // so both `content`'s own body and every descendant piece's build resolve it via
        // `use_context` (which walks scope → ancestors). Owned by the current build scope, so it is
        // disposed with the enclosing subtree (e.g. a `when` arm) exactly like `when`/`each` scopes.
        let scope = Scope::child();
        scope.provide(value);
        scope.enter(|| content().build(cx))
    })
}

/// Read the nearest ambient `T` provided by an enclosing [`with_environment`], or `None` if none is
/// in scope. Call it while constructing or building a piece within that subtree.
pub fn environment<T: Clone + 'static>() -> Option<T> {
    Scope::current().use_context::<T>()
}

/// The ambient `T` of the window that currently has FOCUS (docs/state.md) — SwiftUI's
/// `@FocusedValue`.
///
/// [`environment`] answers "what did MY ancestors provide", which is the right question inside a
/// piece and the wrong one inside an app-wide menu action: a desktop menu bar is one bar for the
/// whole app, and its commands act on the front window. This resolves through that window's own
/// scope instead of the calling scope, so `File ▸ New Item` adds to the list the user is
/// looking at. `None` ⇒ no window is open, or the front one provides no `T`.
pub fn focused_environment<T: Clone + 'static>() -> Option<T> {
    crate::windows::focused_scope()?.use_context::<T>()
}

/// The app-wide `T`: created on the reactive ROOT scope the first time it is asked for, and
/// returned unchanged by every later call (docs/state.md).
///
/// The counterpart to [`with_environment`]'s subtree scope — state that belongs to the APP
/// rather than to a window or a page, reachable from every window, every menu action, and every
/// task, and alive for as long as the process. `make` runs at most once.
pub fn app_environment<T: Clone + 'static>(make: impl FnOnce() -> T) -> T {
    let root = Scope::root();
    if let Some(existing) = root.use_context::<T>() {
        return existing;
    }
    // Created IN the root scope, not merely stored there: signals inside `T` must outlive
    // whatever window happened to ask for it first.
    let value = root.enter(make);
    root.provide(value.clone());
    value
}

/// State an ancestor provides once and any descendant reads back BY TYPE — SwiftUI's
/// `@EnvironmentObject`, and Day's answer to "where does app state live?" (docs/state.md).
///
/// Implement it on a `Copy` struct of HANDLES. `Signal`, `Memo`, `Trigger` and `Store` are all
/// `Copy` and all cheap, so the struct is a bundle of pointers that rides into closures without
/// `Rc` or `clone()` ceremony:
///
/// ```ignore
/// #[derive(Clone, Copy)]
/// struct Scene { selected: Signal<Option<u32>>, items: Store<Keyed<Item>> }
///
/// impl Ambient for Scene {
///     fn create() -> Self { Scene { selected: Signal::new(None), items: Store::new(..) } }
/// }
///
/// // one per window — File ▸ New Window gets its own:
/// Scene::scoped(|scene| my_shell(scene))
/// // anywhere below it:
/// let scene = Scene::ambient();
/// // in an app-wide menu action, which belongs to no window:
/// menu_item("New").action(|| if let Some(s) = Scene::focused() { s.add() })
/// ```
///
/// The alternative — a `thread_local!` holding `Signal::global` — is one instance for the whole
/// process, which is indistinguishable from correct until the app opens a second window
/// (docs/windows.md) and both windows start sharing a selection.
pub trait Ambient: Clone + 'static {
    /// A fresh instance. Called once per providing site: once per window for [`Ambient::scoped`],
    /// once per process for [`Ambient::app`].
    fn create() -> Self;

    /// One instance owned by the scope this piece BUILDS in, provided to `content` and
    /// everything under it. The per-window idiom: call it from a window's shell and each window
    /// gets its own.
    ///
    /// Creation is deferred to build time, which is what makes that true — a piece's
    /// construction runs in the CALLER's scope, and only its build runs inside the window's
    /// (`day_core::launch_with` for the primary, `open_window` for the rest). Creating the state
    /// eagerly would hand every window the first one's.
    ///
    /// It provides on the CURRENT scope rather than a fresh child, and at a window's root that
    /// scope is the window's own. That is what lets [`Ambient::focused`] find it: a context
    /// lookup walks ancestors, so a value tucked into a child of the window scope would be
    /// invisible to anything resolving from the window down — including every app-wide menu
    /// command.
    fn scoped<P: Piece>(content: impl FnOnce(Self) -> P + 'static) -> AnyPiece
    where
        Self: Sized,
    {
        AnyPiece::new(piece_fn(move |cx| {
            let scope = Scope::current();
            let value = Self::create();
            scope.provide(value.clone());
            content(value).build(cx)
        }))
    }

    /// The one app-wide instance, created on first use and alive for the whole process. Visible
    /// from every window, menu action, and task — no `with_environment` needed.
    fn app() -> Self
    where
        Self: Sized,
    {
        app_environment(Self::create)
    }

    /// The nearest instance an ancestor provided, panicking when there is none. The read a piece
    /// writes when the value is a precondition of it existing at all — like
    /// `@EnvironmentObject`, which likewise traps rather than rendering something wrong.
    ///
    /// A BUILD-TIME read, like [`environment`]: call it in a piece's body and capture the value
    /// in whatever closures need it. Calling it *inside* a reactive closure works on the first
    /// run and panics on the next, because a re-running reaction is no longer inside the scope
    /// that provided the value.
    fn ambient() -> Self
    where
        Self: Sized,
    {
        Self::try_ambient().unwrap_or_else(|| {
            let t = std::any::type_name::<Self>();
            panic!(
                "day: no ambient `{t}` in scope. Provide one above this piece with \
                 `{t}::scoped(…)` (per window) or `{t}::app()` (app-wide). If there IS one, \
                 this read is running too late: `ambient()` resolves while a piece BUILDS, so \
                 read it in the piece's body and capture the value rather than calling it \
                 inside a reactive closure (docs/state.md)."
            )
        })
    }

    /// [`Ambient::ambient`] without the panic.
    fn try_ambient() -> Option<Self>
    where
        Self: Sized,
    {
        environment::<Self>()
    }

    /// The instance belonging to the window that currently has focus — what an app-wide menu
    /// command acts on. See [`focused_environment`].
    fn focused() -> Option<Self>
    where
        Self: Sized,
    {
        focused_environment::<Self>()
    }
}
