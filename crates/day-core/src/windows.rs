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
use crate::build::{Boundary, BuildCx, Piece};
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

impl WindowRecord {
    /// Whether this is a cover whose dismissal has been requested and not yet confirmed.
    fn closing(&self) -> bool {
        matches!(&self.tier, Tier::Cover { closing, .. } if closing.get())
    }
}

day_reactive::tls_slots! {
    windows;
    static WINDOWS: RefCell<Vec<WindowRecord>> = const { RefCell::new(Vec::new()) };

    /// The app's first window, so [`initial_window`] can name it. Nothing about the close
    /// policy consults this: that window is an ordinary registry record and counts exactly
    /// like the ones opened after it.
    static INITIAL_WINDOW: Cell<Option<RNode>> = const { Cell::new(None) };

    static PREFS: RefCell<Option<PrefsRegistration>> = const { RefCell::new(None) };
    static PREFS_ACTION: Cell<u64> = const { Cell::new(0) };
    static NEW_WINDOW: RefCell<Option<Rc<dyn Fn() -> AnyPiece>>> = const { RefCell::new(None) };
    static NEW_WINDOW_ACTION: Cell<u64> = const { Cell::new(0) };

    /// What the app asked `launch` for, so a window opened later can describe itself the same
    /// way (docs/windows.md). Title above all: every platform's automatic window management
    /// keys on it — the macOS Window menu and tab bar, the iPad app switcher, the Android
    /// recents card — and an untitled window is simply absent from all of them.
    static LAUNCH_OPTIONS: RefCell<Option<WindowOptions>> = const { RefCell::new(None) };
}

/// Record what the app handed `launch`, for [`open_new_window`] to inherit. Called once by
/// `launch_with` with the options it is about to open the primary window with — already
/// title-decorated, which is idempotent (`tag_title`).
pub fn set_launch_options(options: &WindowOptions) {
    LAUNCH_OPTIONS.with(|o| *o.borrow_mut() = Some(options.clone()));
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
pub fn open_window<P: Piece>(
    key: Option<&str>,
    mut options: WindowOptions,
    kind: WindowKind,
    build: impl FnOnce() -> P + 'static,
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
            if options.size_to_fit {
                // Measured AFTER the first layout, because that is the only point where the
                // content's real height is known — it depends on the user's text size, their
                // language, and which rows the app decided to show.
                //
                // The content root has exactly one child — the piece the builder returned — and
                // its laid-out frame IS the content's natural height.
                let fitted = with_tree(|t| {
                    t.first_child(root)
                        .and_then(|c| t.node_frame(c))
                        .map(|f| f.origin.y + f.size.height)
                        .unwrap_or(0.0)
                });
                // Never grow past what the caller asked for: `size` is the ceiling, so a panel
                // with more content than fits scrolls rather than running off the screen.
                if fitted > 0.0 && fitted < options.size.height {
                    with_tree(|t| {
                        t.fit_window(root, Size::new(options.size.width, fitted));
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
            }
            WindowHandle { root }
        }
        WindowRootReply::Pending(root) => {
            register(
                root,
                key,
                kind,
                scope,
                Tier::PendingNative {
                    build: Some(Box::new(move || AnyPiece::new(build()))),
                    title: options.title.clone(),
                },
            );
            wire_window_events(root);
            WindowHandle { root }
        }
        WindowRootReply::Unsupported => {
            open_as_cover(key, kind, scope, Box::new(move || AnyPiece::new(build())))
        }
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
            // The same rule as `focused_scope`: a dismissing cover is already behind us.
            .find(|r| r.focused && !r.closing())
            .map(|r| WindowHandle { root: r.root })
    })
}

/// The scope owning the content of the window that currently has FOCUS — the scope an
/// app-wide command should resolve per-window state through (docs/state.md).
///
/// Falls back to the app's primary window, which is the same rule [`focused_window`] states:
/// a backend reports focus for the windows it opened, and "no record is focused" means the
/// primary is key (on AppKit the primary's delegate carries no node and never emits
/// `WindowFocused` at all, so that IS the primary's steady state). `None` only before boot
/// and after the last window is gone.
pub fn focused_scope() -> Option<Scope> {
    WINDOWS.with(|w| {
        let windows = w.borrow();
        // A cover on its way out (dismiss requested, `CoverHidden` not yet back — the phone
        // animates it) is no longer the front window: the one behind it is. Its record stays
        // registered until the hide confirms, so without this a command issued during the
        // animation resolves to a sheet that is gone and acts on nothing.
        let focused = windows.iter().find(|r| r.focused && !r.closing());
        let initial = INITIAL_WINDOW.with(|c| c.get());
        focused
            .or_else(|| windows.iter().find(|r| Some(r.root) == initial))
            .map(|r| r.scope)
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
    // This window's own size class, before its content builds (docs/size-classes.md) — a second
    // window can sit in a different class from the first, which is why the signal is per-window.
    crate::ambient::set_window_size_class(
        root,
        day_spec::SizeClass::from_size(size.width, size.height),
    );
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
        let mut windows = w.borrow_mut();
        // A window that just opened IS the key window — every platform orders it front. Seeding
        // that here rather than waiting for `Event::WindowFocused` is what makes it TRUE for the
        // first one: the toolkit makes the window key while creating it, which on AppKit fires
        // `windowDidBecomeKey` before `wire_window_events` has installed a handler to hear it.
        // Without this, a window opened by File ▸ New Window is never marked focused, and
        // `focused_scope` — how an app-wide menu command finds the front window's state
        // (docs/state.md) — resolves to the primary until the user clicks away and back.
        for r in windows.iter_mut() {
            r.focused = false;
        }
        windows.push(WindowRecord {
            root,
            key: key.map(str::to_string),
            kind,
            role: WindowRole::from(kind),
            scope,
            tier,
            focused: true,
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
                    // …and re-bucket THIS window (docs/size-classes.md). The primary's rail does
                    // the same in `launch_with`; without it here a secondary window relayouts at
                    // its new size but keeps the class it opened at, so dragging the second
                    // window from narrow to wide never re-presented its navigation. Invisible
                    // until a window could be resized at all, which on the phones it now can.
                    crate::ambient::set_window_size_class(
                        root,
                        day_spec::SizeClass::from_size(s.width, s.height),
                    );
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
    crate::ambient::forget_window(root);
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
/// window (docs/cover.md semantics — NavBack dismisses, `Event::CoverHidden` confirms).
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
                    Event::CoverHidden if closing.get() => {
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

/// The singleton key every preferences window opens under.
pub const PREFERENCES_KEY: &str = "day.preferences";

/// Declare the app's preferences piece (docs/windows.md) — once, in `root()`, ideally
/// before `app_menu`. Enables the desktop Preferences window (singleton, primary+`,`),
/// the auto Settings…/Preferences menu item, and [`open_preferences`] everywhere (cover
/// fallback where the toolkit cannot open windows). The window titles itself with
/// `options.title`; use [`register_preferences_with`] to localize it or change the size.
pub fn register_preferences<P: Piece>(build: impl Fn() -> P + 'static) {
    register_preferences_with(
        WindowOptions {
            title: "Settings".into(),
            // The width a settings panel wants, and a CEILING for the height rather than the
            // height itself: `size_to_fit` shrinks the window to whatever the rows actually
            // measure. A fixed 640 either clips the last row or leaves a band of empty panel
            // under it, and which one depends on the user's text size — so nobody can pick a
            // number that is right for everyone.
            size: Size::new(520.0, 640.0),
            min_size: None,
            size_to_fit: true,
            app_name: None,
            // Secondary windows: the app-launch ceremony belongs to `launch` alone.
            locales: None,
            title_fn: None,
        },
        build,
    );
}

/// [`register_preferences`] with explicit window options (localized title, size).
pub fn register_preferences_with<P: Piece>(
    options: WindowOptions,
    build: impl Fn() -> P + 'static,
) {
    PREFS.with(|p| *p.borrow_mut() = Some((Rc::new(move || AnyPiece::new(build())), options)));
    if PREFS_ACTION.with(|c| c.get()) == 0 {
        PREFS_ACTION.with(|c| {
            c.set(crate::menu::register_menu_action(Rc::new(|| {
                open_preferences();
            })))
        });
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
        log::warn!("open_preferences without register_preferences — ignored");
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
pub fn register_new_window<P: Piece>(build: impl Fn() -> P + 'static) {
    NEW_WINDOW.with(|p| *p.borrow_mut() = Some(Rc::new(move || AnyPiece::new(build()))));
    if NEW_WINDOW_ACTION.with(|c| c.get()) == 0 {
        NEW_WINDOW_ACTION.with(|c| {
            c.set(crate::menu::register_menu_action(Rc::new(|| {
                open_new_window();
            })))
        });
    }
    crate::menu::reinstall_app_menu();
}

/// Open a window through the registered new-window builder (the `newWindowForTab:` /
/// File ▸ New Window path). `None` = no builder registered.
pub fn open_new_window() -> Option<WindowHandle> {
    let build = NEW_WINDOW.with(|p| p.borrow().clone())?;
    // Another window of THIS app, so it describes itself the way the app described its first
    // one: same title, same minimum size, same display name. A window with no title is missing
    // from the macOS Window menu and shows a blank tab, so inheriting is what makes File ▸ New
    // Window produce something the platform can manage (docs/windows.md). Content that wants a
    // title of its own says so with `window_title` from inside the window.
    let launch = LAUNCH_OPTIONS.with(|o| o.borrow().clone());
    // Size mirrors the primary's CURRENT content size so a "duplicate window" lands familiar.
    let size = with_tree(|t| {
        let root = t.root_node();
        t.node_frame(root).map(|f| f.size)
    })
    .unwrap_or(Size::new(800.0, 600.0));
    Some(open_window(
        None,
        WindowOptions {
            title: launch.as_ref().map(|o| o.title.clone()).unwrap_or_default(),
            size,
            min_size: launch.as_ref().and_then(|o| o.min_size),
            size_to_fit: false,
            app_name: launch.as_ref().and_then(|o| o.app_name.clone()),
            // Secondary windows: the app-launch ceremony belongs to `launch` alone.
            locales: None,
            // Already resolved into `title` above — calling it again would re-run app code
            // outside the launch sequence it was written for.
            title_fn: None,
        },
        WindowKind::Normal,
        move || build(),
    ))
}

/// Bind the title of the window this piece is BUILDING INTO to a reactive closure
/// (docs/windows.md) — how a window comes to be named after what it shows.
///
/// The window-level counterpart to a navigation title. It matters more than it looks: the macOS
/// Window menu, the tab bar, Mission Control, the iPad app switcher and the Android recents card
/// all label a window by its title, so two windows that share one title are two windows the user
/// cannot tell apart anywhere the system lists them.
///
/// ```ignore
/// // inside a window's shell, so each window titles itself:
/// day::window_title(move || match scene.selected.get() {
///     Some(id) => scene.name_of(id),
///     None => app_title(),
/// });
/// ```
///
/// Reactive like any binding: the title follows what the closure reads. Which window it targets
/// is resolved ONCE, here, for the same reason `toolbar_reactive` captures it — the binding
/// re-runs long after this build, when "the window being built" is no longer this one.
pub fn window_title(f: impl Fn() -> String + 'static) {
    let root = crate::toolbar::current_window();
    day_reactive::bind(f, move |title| {
        let title = crate::decorate_window_title(title);
        with_tree(|t| t.set_native_window_title(root, &title));
    });
}

/// The dispatch id of the auto Preferences menu action (0 = unregistered). Backends use it
/// to wire their default-menu Settings item; the injection pass uses it for `app_menu`.
pub fn preferences_action_id() -> u64 {
    PREFS_ACTION.with(|c| c.get())
}

/// The dispatch id of the New Window action (0 = unregistered) — `MenuRole::NewWindow`
/// lowering and the backends' tab-bar "+" wiring.
pub fn new_window_action_id() -> u64 {
    NEW_WINDOW_ACTION.with(|c| c.get())
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
    // Deliberately does NOT dispose each record's content scope, which is a change that was
    // made and then REVERTED after it broke navigation (docs/appearance.md "What was tried").
    //
    // Disposing looks right — the reactive graph a window built otherwise outlives it, and on an
    // Android re-mount that showed up as the previous window's navigation host re-registering
    // over the new one's ("two routed one-of-N surfaces … at the same navigation level"). But
    // the disposal cascade runs app cleanup that reads signals the same cascade has already
    // disposed; the panic is contained, the disposal stops half-way, and the REBUILD that
    // follows registers no routes at all. The symptom is an app whose tab bar draws and whose
    // tabs do nothing.
    //
    // Leaking the old graph is the lesser fault: it warns, and the app works. Disposing safely
    // needs the cascade to tear down in dependency order, which is its own piece of work.
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
