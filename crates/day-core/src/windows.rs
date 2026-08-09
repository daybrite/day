// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Secondary windows (docs/windows.md): the app-facing `open_window` API, the per-window
//! registry, and the fallback that presents window content as a fullscreen cover where the
//! toolkit cannot open windows (`Cap::MultiWindow` = `Unsupported`).
//!
//! Windows live in the ONE thread-local tree as additional boundary roots (the same
//! "wrap an externally-owned handle" record as the primary root and the list cell
//! anchors), so bindings, `find_by_id`, and dayscript work across windows unchanged.
//! Close is asynchronous everywhere: the platform (or the cover's hide transition)
//! confirms, and teardown runs THEN — one path for native and programmatic closes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_reactive::Scope;
use day_spec::props::{CoverPatch, CoverProps};
use day_spec::{Event, NodeId, Size, WindowKind, WindowOptions, WindowRole, kinds};

use crate::AnyPiece;
use crate::build::{Boundary, BuildCx, Piece as _};
use crate::tree::{RNode, WindowRootReply, id_to_rnode, rnode_to_id, with_tree};

/// How a window record is realized.
enum Tier {
    /// A live native OS window; the root's handle is its content container.
    Native,
    /// The toolkit answered `Pending` — native creation is in flight; the content builds
    /// when the backend calls [`finish_window_open`].
    PendingNative {
        build: Option<Box<dyn FnOnce() -> AnyPiece>>,
        title: String,
    },
    /// No native window — the content presents as a fullscreen cover in the primary
    /// window; `cover` is the COVER node, `closing` gates the dismiss transition.
    Cover {
        cover: RNode,
        closing: Rc<Cell<bool>>,
    },
}

/// Close callbacks, shared so they outlive the registry entry during teardown.
type OnCloseList = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

struct WindowRecord {
    root: RNode,
    key: Option<String>,
    kind: WindowKind,
    /// Whether this window keeps the app alive (docs/windows.md close policy). Derived from
    /// `kind` today; a per-window override is the natural next step.
    role: WindowRole,
    scope: Scope,
    tier: Tier,
    focused: bool,
    on_close: OnCloseList,
}

thread_local! {
    static WINDOWS: RefCell<Vec<WindowRecord>> = const { RefCell::new(Vec::new()) };
}

/// A live secondary window (docs/windows.md). Cheap to clone; inert after close.
#[derive(Clone)]
pub struct WindowHandle {
    root: RNode,
}

impl WindowHandle {
    /// Close the window. Asynchronous: the platform (or the cover's hide transition)
    /// confirms, then the content is disposed and `on_close` callbacks run. Idempotent.
    pub fn close(&self) {
        let root = self.root;
        let action = WINDOWS.with(|w| {
            w.borrow()
                .iter()
                .find(|r| r.root == root)
                .map(|r| match &r.tier {
                    Tier::Native => CloseAction::Native,
                    Tier::PendingNative { .. } => CloseAction::Immediate,
                    Tier::Cover { cover, closing } => {
                        if closing.get() {
                            CloseAction::None
                        } else {
                            closing.set(true);
                            CloseAction::Cover(*cover)
                        }
                    }
                })
        });
        match action {
            Some(CloseAction::Native) => with_tree(|t| t.close_native_window(root)),
            // Parked pending: nothing native to wait for — tear down now; the backend's
            // later `finish_window_open` finds the record gone and drops its window.
            Some(CloseAction::Immediate) => teardown(root),
            Some(CloseAction::Cover(cover)) => {
                with_tree(|t| t.patch(cover, Box::new(CoverPatch::Dismiss), false));
            }
            Some(CloseAction::None) | None => {}
        }
    }

    /// Bring the window to front and make it key. No-op on the cover tier (the cover is
    /// already frontmost in the primary window) and while a `Pending` open is in flight.
    pub fn focus(&self) {
        let root = self.root;
        if WINDOWS.with(|w| {
            w.borrow()
                .iter()
                .any(|r| r.root == root && matches!(r.tier, Tier::Native))
        }) {
            with_tree(|t| t.focus_native_window(root));
        }
    }

    /// Retitle the window. Applies to live native windows; a `Pending` window picks the
    /// new title up at completion; the cover tier has no title bar (no-op).
    pub fn set_title(&self, title: &str) {
        let root = self.root;
        let title = &crate::decorate_window_title(title);
        let live = WINDOWS.with(|w| {
            let mut windows = w.borrow_mut();
            match windows.iter_mut().find(|r| r.root == root) {
                Some(r) => match &mut r.tier {
                    Tier::Native => true,
                    Tier::PendingNative { title: t, .. } => {
                        *t = title.to_string();
                        false
                    }
                    Tier::Cover { .. } => false,
                },
                None => false,
            }
        });
        if live {
            let title = title.to_string();
            with_tree(|t| t.set_native_window_title(root, &title));
        }
    }

    /// Whether the window is still open (its record exists — a `Pending` open counts).
    pub fn is_open(&self) -> bool {
        let root = self.root;
        WINDOWS.with(|w| w.borrow().iter().any(|r| r.root == root))
    }

    /// Run `f` after the window has closed and its content is disposed — any close path
    /// (title-bar, platform gesture, [`WindowHandle::close`]). No-op if already closed.
    pub fn on_close(&self, f: impl Fn() + 'static) {
        let root = self.root;
        WINDOWS.with(|w| {
            if let Some(r) = w.borrow().iter().find(|r| r.root == root) {
                r.on_close.borrow_mut().push(Box::new(f));
            }
        });
    }
}

enum CloseAction {
    Native,
    Immediate,
    Cover(RNode),
    None,
}

/// Open a secondary window (docs/windows.md). `key` names the LOGICAL window: when a
/// window with this key is already open it is focused and returned instead of duplicated
/// (the preferences pattern); `None` always opens a new one. Where the toolkit cannot
/// open windows (`Cap::MultiWindow` = `Unsupported` — probe it to adapt chrome) the
/// content presents as a fullscreen cover in the primary window instead, closable the
/// same way.
pub fn open_window(
    key: Option<&str>,
    mut options: WindowOptions,
    kind: WindowKind,
    build: impl FnOnce() -> AnyPiece + 'static,
) -> WindowHandle {
    if let Some(k) = key
        && let Some(existing) = window_by_key(k)
    {
        existing.focus();
        return existing;
    }
    // A secondary window carries the same debug tag as the primary — with several windows of
    // several builds open, that is what tells them apart.
    options.title = crate::decorate_window_title(&options.title);

    // The window's lifetime is app-owned, not caller-owned: a window opened from a button
    // must survive that page popping, so its scope hangs off the root scope.
    let scope = Scope::root().enter(Scope::child);

    match with_tree(|t| t.open_window_root(&options, kind)) {
        WindowRootReply::Open(root) => {
            register(root, key, kind, scope, Tier::Native);
            wire_window_events(root);
            scope.enter(|| {
                // Name this window for the duration of its build, so a `toolbar(...)` inside a
                // shared window builder installs on THIS window (docs/toolbars.md).
                crate::toolbar::with_window(root, || {
                    let piece = build();
                    let mut cx = BuildCx::new(root);
                    let _ = piece.build(&mut cx);
                })
            });
            with_tree(|t| {
                t.mark_layout_dirty();
                t.layout_if_needed();
            });
            WindowHandle { root }
        }
        WindowRootReply::Pending(root) => {
            register(
                root,
                key,
                kind,
                scope,
                Tier::PendingNative {
                    build: Some(Box::new(build)),
                    title: options.title.clone(),
                },
            );
            wire_window_events(root);
            WindowHandle { root }
        }
        WindowRootReply::Unsupported => open_as_cover(key, kind, scope, Box::new(build)),
    }
}

/// The open window registered under `key`, if any (the dayscript `screenshot` step's
/// `window:` target resolves here).
pub fn window_by_key(key: &str) -> Option<WindowHandle> {
    WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .find(|r| r.key.as_deref() == Some(key))
            .map(|r| WindowHandle { root: r.root })
    })
}

/// The most recently focused open secondary window, if any. `None` ⇒ the primary window
/// is key (or no secondary window exists).
pub fn focused_window() -> Option<WindowHandle> {
    WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .find(|r| r.focused)
            .map(|r| WindowHandle { root: r.root })
    })
}

/// The tree root of the window registered under `key` — day-script's snapshot target.
pub fn window_root_by_key(key: &str) -> Option<RNode> {
    WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .find(|r| r.key.as_deref() == Some(key))
            .map(|r| match &r.tier {
                // The cover renders inside the primary window: its pixels are the
                // primary's pixels.
                Tier::Cover { .. } => with_tree(|t| t.root_node()),
                _ => r.root,
            })
    })
}

/// Backend-facing: complete a `Pending` window open (docs/windows.md). `id` is the node
/// the backend's `open_window` received; `raw` is the new window's CONTENT container;
/// `size` its content size in points. `false` ⇒ the window was closed before completion —
/// the backend should drop the native window it just created.
pub fn finish_window_open(id: NodeId, raw: day_spec::RawHandle, size: Size) -> bool {
    let root = id_to_rnode(id);
    let pending = WINDOWS.with(|w| {
        let mut windows = w.borrow_mut();
        let r = windows.iter_mut().find(|r| r.root == root)?;
        match &mut r.tier {
            Tier::PendingNative { build, title } => {
                let b = build.take();
                let title = title.clone();
                r.tier = Tier::Native;
                Some((b, title, r.scope))
            }
            _ => None,
        }
    });
    let Some((build, title, scope)) = pending else {
        return false;
    };
    if !with_tree(|t| t.adopt_window_root(root, raw, size)) {
        return false;
    }
    with_tree(|t| t.set_native_window_title(root, &title));
    if let Some(build) = build {
        scope.enter(|| {
            crate::toolbar::with_window(root, || {
                let piece = build();
                let mut cx = BuildCx::new(root);
                let _ = piece.build(&mut cx);
            })
        });
    }
    with_tree(|t| {
        t.mark_layout_dirty();
        t.layout_if_needed();
    });
    true
}

fn register(root: RNode, key: Option<&str>, kind: WindowKind, scope: Scope, tier: Tier) {
    WINDOWS.with(|w| {
        w.borrow_mut().push(WindowRecord {
            root,
            key: key.map(str::to_string),
            kind,
            role: WindowRole::from(kind),
            scope,
            tier,
            focused: false,
            on_close: Rc::default(),
        })
    });
}

/// The per-window event rail (native + pending tiers): resize relayouts that window,
/// focus updates the registry, close tears down — DEFERRED one main-loop hop so the
/// disposal never runs inside the platform's own close callback (a released view inside
/// `windowWillClose`/`close-request`/`closeEvent` is the top reentrancy hazard).
fn wire_window_events(root: RNode) {
    with_tree(|t| {
        t.on_event(
            root,
            Rc::new(move |ev| match ev {
                Event::WindowResized(size) => {
                    let s = *size;
                    with_tree(|t| t.set_root_size(root, s));
                }
                Event::WindowFocused(f) => {
                    let f = *f;
                    WINDOWS.with(|w| {
                        for r in w.borrow_mut().iter_mut() {
                            if r.root == root {
                                r.focused = f;
                            } else if f {
                                r.focused = false;
                            }
                        }
                    });
                }
                Event::WindowClosed => {
                    day_reactive::on_main(move || teardown(root));
                }
                _ => {}
            }),
        );
    });
}

/// How many REGISTERED windows keep the app alive (docs/windows.md close policy). The initial
/// window is not among them — see [`initial_primary_open`].
pub fn primary_window_count() -> usize {
    WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .filter(|r| r.role == WindowRole::Primary)
            .count()
    })
}

thread_local! {
    /// The app's first window, so [`initial_window`] can name it. Nothing about the close
    /// policy consults this: that window is an ordinary registry record and counts exactly
    /// like the ones opened after it.
    static INITIAL_WINDOW: Cell<Option<RNode>> = const { Cell::new(None) };
}

/// Whether any window is still holding the app open.
fn app_has_primary_window() -> bool {
    primary_window_count() > 0
}

/// Whether closing the last primary window ends the process on this platform.
///
/// macOS says no, and means it: an app with no windows stays running with its menu bar live,
/// and ⌘N reopens one — `applicationShouldTerminateAfterLastWindowClosed` defaults to false
/// for exactly this. Quitting there would be the framework overriding a platform convention
/// its users rely on. Every other desktop treats the last window as the app.
fn last_primary_close_quits() -> bool {
    !cfg!(target_os = "macos")
}

/// End the app because its last [`WindowRole::Primary`] window has closed.
///
/// One place, for every backend: dispose whatever is still open — a settings panel does not
/// keep an app alive, however long it has been up — deliver `WillTerminate` once, and then
/// ask the toolkit for the platform's own exit.
fn quit_after_last_primary() {
    if !last_primary_close_quits() {
        return;
    }
    // Secondary windows go with the app. Collected first: teardown mutates the registry.
    let remaining: Vec<RNode> = WINDOWS.with(|w| w.borrow().iter().map(|r| r.root).collect());
    for root in remaining {
        teardown(root);
    }
    crate::lifecycle::dispatch_lifecycle(day_spec::Lifecycle::WillTerminate);
    with_tree(|t| t.quit_app());
}

/// The single teardown path (any close route lands here). Idempotent via registry
/// membership: registry-remove → scope dispose → content removal → root removal →
/// `on_close` callbacks.
fn teardown(root: RNode) {
    let Some(record) = WINDOWS.with(|w| {
        let mut windows = w.borrow_mut();
        windows
            .iter()
            .position(|r| r.root == root)
            .map(|i| windows.remove(i))
    }) else {
        return;
    };
    record.scope.dispose();
    crate::toolbar::forget_window(root);
    match record.tier {
        Tier::Native | Tier::PendingNative { .. } => {
            while let Some(c) = with_tree(|t| t.first_child(root)) {
                with_tree(|t| t.remove_subtree(c));
            }
            with_tree(|t| t.remove_window_root(root));
        }
        Tier::Cover { cover, .. } => {
            // The cover node lives under the primary root — drop its independent layout
            // entry, then the ordinary subtree removal takes its content with it.
            with_tree(|t| {
                t.drop_extra_layout_root(cover);
                t.remove_subtree(cover);
            });
        }
    }
    with_tree(|t| {
        t.mark_layout_dirty();
        t.layout_if_needed();
    });
    for f in record.on_close.borrow().iter() {
        f();
    }
    // The app's life is the life of its primary windows, not of the first one opened. Checked
    // after the callbacks so a handler that opens a replacement window is counted.
    if record.role == WindowRole::Primary && !app_has_primary_window() {
        quit_after_last_primary();
    }
}

/// The fallback tier: present the window content as a fullscreen cover in the primary
/// window (docs/cover.md semantics — NavBack dismisses, `cover-hidden` confirms).
fn open_as_cover(
    key: Option<&str>,
    kind: WindowKind,
    scope: Scope,
    build: Box<dyn FnOnce() -> AnyPiece>,
) -> WindowHandle {
    let size: Rc<RefCell<Option<Size>>> = Rc::default();
    let primary = with_tree(|t| t.root_node());
    let cover = {
        let mut cx = BuildCx::new(primary);
        cx.native(
            kinds::COVER,
            &CoverProps::default(),
            Rc::new(crate::CoverLayout { size: size.clone() }),
            crate::Flex::default(),
            Boundary::Yes,
        )
    };
    with_tree(|t| t.add_extra_layout_root(cover, Size::new(0.0, 0.0)));
    let closing: Rc<Cell<bool>> = Rc::default();
    // The window root IS the cover node on this tier — one id for close/teardown.
    register(
        cover,
        key,
        kind,
        scope,
        Tier::Cover {
            cover,
            closing: closing.clone(),
        },
    );

    {
        let (size, closing) = (size.clone(), closing.clone());
        with_tree(|t| {
            t.on_event(
                cover,
                Rc::new(move |ev| match ev {
                    Event::FrameChanged(sz) => {
                        if *size.borrow() != Some(*sz) {
                            *size.borrow_mut() = Some(*sz);
                            with_tree(|t| {
                                t.set_root_size(cover, *sz);
                                t.mark_layout_dirty();
                                t.layout_if_needed();
                            });
                        }
                    }
                    // Native dismissal request (Android system back): a fallback "window"
                    // closes like a window, not like a page.
                    Event::NavBack { .. } => {
                        if !closing.get() {
                            closing.set(true);
                            with_tree(|t| t.patch(cover, Box::new(CoverPatch::Dismiss), false));
                        }
                    }
                    // The hide transition finished — now the content can go.
                    Event::Custom { tag, text, .. }
                        if (*tag == "cover-hidden" || text.as_str() == "cover-hidden")
                            && closing.get() =>
                    {
                        day_reactive::on_main(move || teardown(cover));
                    }
                    _ => {}
                }),
            );
        });
    }

    scope.enter(|| {
        crate::toolbar::with_window(cover, || {
            let piece = build();
            let mut cx = BuildCx::new(cover);
            let _ = piece.build(&mut cx);
        })
    });
    with_tree(|t| {
        t.patch(
            cover,
            Box::new(CoverPatch::Present {
                background: None,
                dismiss_disabled: false,
            }),
            false,
        );
        t.mark_needs_measure(cover);
        t.mark_layout_dirty();
        t.layout_if_needed();
    });
    WindowHandle { root: cover }
}

// ---------------------------------------------------------------------------
// Preferences + New Window registration (docs/windows.md)
// ---------------------------------------------------------------------------

/// The registered preferences piece: its builder plus the window options it opens with.
type PrefsRegistration = (Rc<dyn Fn() -> AnyPiece>, WindowOptions);

thread_local! {
    static PREFS: RefCell<Option<PrefsRegistration>> = const { RefCell::new(None) };
    static PREFS_ACTION: Cell<u64> = const { Cell::new(0) };
    static NEW_WINDOW: RefCell<Option<Rc<dyn Fn() -> AnyPiece>>> = const { RefCell::new(None) };
    static NEW_WINDOW_ACTION: Cell<u64> = const { Cell::new(0) };
}

/// The singleton key every preferences window opens under.
pub const PREFERENCES_KEY: &str = "day.preferences";

/// Declare the app's preferences piece (docs/windows.md) — once, in `root()`, ideally
/// before `app_menu`. Enables the desktop Preferences window (singleton, primary+`,`),
/// the auto Settings…/Preferences menu item, and [`open_preferences`] everywhere (cover
/// fallback where the toolkit cannot open windows). The window titles itself with
/// `options.title`; use [`register_preferences_with`] to localize it or change the size.
pub fn register_preferences(build: impl Fn() -> AnyPiece + 'static) {
    register_preferences_with(
        WindowOptions {
            title: "Settings".into(),
            size: Size::new(520.0, 640.0),
            min_size: None,
            app_name: None,
        },
        build,
    );
}

/// [`register_preferences`] with explicit window options (localized title, size).
pub fn register_preferences_with(options: WindowOptions, build: impl Fn() -> AnyPiece + 'static) {
    PREFS.with(|p| *p.borrow_mut() = Some((Rc::new(build), options)));
    if PREFS_ACTION.get() == 0 {
        PREFS_ACTION.set(crate::menu::register_menu_action(Rc::new(|| {
            open_preferences();
        })));
    }
    // Self-heal for registration AFTER `app_menu`: re-forward the retained model so the
    // injection pass sees the now-registered action.
    crate::menu::reinstall_app_menu();
}

/// Open-or-focus the preferences surface (docs/windows.md): a `WindowKind::Preferences`
/// singleton window on desktop, the cover fallback elsewhere — one call for menu items and
/// toolbar gears alike. `false` = no preferences piece is registered (logged).
pub fn open_preferences() -> bool {
    let Some((build, options)) = PREFS.with(|p| p.borrow().clone()) else {
        crate::diag(format_args!(
            "day: open_preferences without register_preferences — ignored"
        ));
        return false;
    };
    open_window(
        Some(PREFERENCES_KEY),
        options,
        WindowKind::Preferences,
        move || build(),
    );
    true
}

/// Register the builder behind File ▸ New Window and the macOS tab-bar "+" (docs/windows.md):
/// each call opens another `WindowKind::Normal` window. A `menu_role(MenuRole::NewWindow)`
/// item lowers to this action; without a registration it lowers disabled.
pub fn register_new_window(build: impl Fn() -> AnyPiece + 'static) {
    NEW_WINDOW.with(|p| *p.borrow_mut() = Some(Rc::new(build)));
    if NEW_WINDOW_ACTION.get() == 0 {
        NEW_WINDOW_ACTION.set(crate::menu::register_menu_action(Rc::new(|| {
            open_new_window();
        })));
    }
    crate::menu::reinstall_app_menu();
}

/// Open a window through the registered new-window builder (the `newWindowForTab:` /
/// File ▸ New Window path). `None` = no builder registered.
pub fn open_new_window() -> Option<WindowHandle> {
    let build = NEW_WINDOW.with(|p| p.borrow().clone())?;
    // The primary window's options are long gone; new windows describe themselves —
    // the builder's content sets the title via the handle if it cares. Size mirrors the
    // primary's CURRENT content size so a "duplicate window" lands familiar.
    let size = with_tree(|t| {
        let root = t.root_node();
        t.node_frame(root).map(|f| f.size)
    })
    .unwrap_or(Size::new(800.0, 600.0));
    Some(open_window(
        None,
        WindowOptions {
            title: String::new(),
            size,
            min_size: None,
            app_name: None,
        },
        WindowKind::Normal,
        move || build(),
    ))
}

/// The dispatch id of the auto Preferences menu action (0 = unregistered). Backends use it
/// to wire their default-menu Settings item; the injection pass uses it for `app_menu`.
pub fn preferences_action_id() -> u64 {
    PREFS_ACTION.get()
}

/// The dispatch id of the New Window action (0 = unregistered) — `MenuRole::NewWindow`
/// lowering and the backends' tab-bar "+" wiring.
pub fn new_window_action_id() -> u64 {
    NEW_WINDOW_ACTION.get()
}

/// Test/diagnostic surface: the number of open secondary windows.
pub fn open_window_count() -> usize {
    WINDOWS.with(|w| w.borrow().len())
}

/// The `WindowKind` of the open window at `root`, for backends and tests.
pub fn window_kind_of(handle: &WindowHandle) -> Option<WindowKind> {
    WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .find(|r| r.root == handle.root)
            .map(|r| r.kind)
    })
}

/// Reset the registry + registrations (tests — pairs with `uninstall_tree`).
pub fn reset_windows() {
    WINDOWS.with(|w| w.borrow_mut().clear());
    INITIAL_WINDOW.with(|c| c.set(None));
    crate::toolbar::reset_toolbars();
    PREFS.with(|p| *p.borrow_mut() = None);
    NEW_WINDOW.with(|p| *p.borrow_mut() = None);
    // Action ids stay registered (the closures are inert without a builder) — cheap, and
    // re-registration reuses them.
}

/// Adopt the app's FIRST window into the registry, so it is an ordinary primary window rather
/// than a privileged one (docs/windows.md close policy).
///
/// Called once at boot with the root container the backend handed back. `scope` owns the root
/// content, so closing this window disposes exactly its own tree and nothing else — which is
/// why the caller builds that content in a child of the root scope rather than in the root
/// scope itself. State an app wants to outlive its windows still does: `Signal::global` lives
/// on the root scope, above this one.
pub fn adopt_initial_window(root: RNode, scope: Scope) {
    register(root, None, WindowKind::Normal, scope, Tier::Native);
    wire_window_events(root);
    INITIAL_WINDOW.with(|c| c.set(Some(root)));
}

/// A handle to the app's first window, once adopted. `None` before boot completes, and after
/// that window closes — it is an ordinary window and can be closed like any other.
pub fn initial_window() -> Option<WindowHandle> {
    let root = INITIAL_WINDOW.with(|c| c.get())?;
    WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .any(|r| r.root == root)
            .then_some(WindowHandle { root })
    })
}

/// The window root's spec-boundary id (backends key their per-window maps by it).
pub fn window_node_id(handle: &WindowHandle) -> NodeId {
    rnode_to_id(handle.root)
}
