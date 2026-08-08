// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-core — the Piece model, realized tree, mounter, layout engine, and event routing
//! (DESIGN.md §5, §7). Build-once: pieces are constructed exactly once; all dynamism flows
//! through reactive bindings (day-reactive) writing to the thread-local tree.

mod anim;
mod build;
pub mod frame;
mod layout;
pub mod lifecycle;
pub mod list;
pub mod menu;
mod nav;
mod present;
pub mod shield;
pub mod toolbar;
mod tree;
pub mod windows;

pub use anim::{current_anim, with_animation};
pub use build::*;
pub use frame::{
    FrameConsumer, add_frame_consumer, frame_consumer_count, install_frame_requester,
    remove_frame_consumer,
};
pub use layout::*;
pub use lifecycle::{dispatch_lifecycle, lifecycle_supported, on_lifecycle};
pub use list::{
    BuiltRow, ListDriver, ListReorderDriver, install_list, list_reload, list_scroll_to_end,
    list_scroll_to_row, list_set_selected, list_try_reorder,
};
pub use menu::{dispatch_menu_action, register_menu_action, set_app_menu};
pub use nav::*;
pub use present::*;
pub use toolbar::{
    current_window, dispatch_toolbar_value, patch_toolbar, patch_window_toolbar,
    register_toolbar_value, set_toolbar, set_window_toolbar,
};
// The resource seam lives in day-spec (backends depend only on day-spec); re-export for the facade.
pub use day_spec::resource::{
    AssetName, FontFamily, ImageName, Resource, ResourceOpener, VectorName, resource,
    set_resource_opener,
};
pub use tree::*;
pub use windows::{
    WindowHandle, finish_window_open, focused_window, open_new_window, open_preferences,
    open_window, register_new_window, register_preferences, register_preferences_with,
    window_by_key,
};

/// The app-wide layout direction (docs/localization): mirrors every horizontal placement in
/// the place pass when [`day_geometry::LayoutDirection::Rtl`]. Resolved lazily from the
/// `DAY_LOCALE` launch environment (so toolkits can read it before any UI exists);
/// `set_layout_direction` (called by `install_locales` for the resolved locale) overrides.
/// Fixed for the life of the process — switching locale at runtime does not re-mirror.
pub fn layout_direction() -> day_geometry::LayoutDirection {
    DIRECTION.with(|d| {
        if let Some(dir) = d.get() {
            return dir;
        }
        let dir = std::env::var("DAY_LOCALE")
            .map(|l| direction_of_locale(&l))
            .unwrap_or_default();
        d.set(Some(dir));
        dir
    })
}

thread_local! {
    /// The window's safe-area insets (points) — see [`safe_area`]. ROOT-scoped: backends write
    /// it from native inset callbacks that can fire while transient scopes are current.
    static SAFE_AREA: std::cell::OnceCell<day_reactive::Signal<day_geometry::Insets>> =
        const { std::cell::OnceCell::new() };
}

fn safe_area_signal() -> day_reactive::Signal<day_geometry::Insets> {
    SAFE_AREA.with(|c| {
        *c.get_or_init(|| {
            // Seeded from the launch environment so a page built during startup — before the
            // backend's first live report lands — already sees the right top inset (the same
            // lazy-env pattern as `layout_direction`/DAY_LOCALE). Points, not px.
            let initial = std::env::var("DAY_SAFE_AREA_TOP")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .map(|top| day_geometry::Insets {
                    top,
                    ..Default::default()
                })
                .unwrap_or_default();
            day_reactive::Scope::root().enter(|| day_reactive::Signal::global(initial))
        })
    })
}

/// The window's safe-area insets, in points. Zero on every backend that clamps Day's root to
/// the safe area natively (the default everywhere); nonzero only where a backend runs the root
/// edge-to-edge — today day-android's opt-in immersive mode (docs/layout.md, the android
/// platform page). Compose it yourself where a background should run under the system bars:
/// paint the background unpadded, pad the content by these insets. The read is tracked, but
/// layout attributes like `.padding` capture the value at build time — a mid-run inset change
/// (rotation) does not re-pad already-built pages.
pub fn safe_area() -> day_geometry::Insets {
    safe_area_signal().get()
}

/// Backend-facing: report the window's safe-area insets (points). Call from the native inset
/// pass whenever the value changes; apps observe it through [`safe_area`].
pub fn set_safe_area(insets: day_geometry::Insets) {
    safe_area_signal().set(insets);
}

/// Override the layout direction (normally from `install_locales`). Must be called before the
/// first layout pass to take effect everywhere.
pub fn set_layout_direction(dir: day_geometry::LayoutDirection) {
    DIRECTION.with(|d| d.set(Some(dir)));
}

/// Whether the app is being rendered right-to-left (docs/localization) — a convenience over
/// [`layout_direction`]. The layout engine already mirrors widget *placement* under an RTL locale,
/// but a `canvas` draws in its own coordinate space, so a custom drawing that has a reading
/// direction (a battery that drains one way, an arrow, a progress sweep) can call this to mirror
/// itself. Fixed for the life of the process, like [`layout_direction`].
pub fn is_rtl() -> bool {
    layout_direction() == day_geometry::LayoutDirection::Rtl
}

/// The writing direction a locale implies (language subtag match).
pub fn direction_of_locale(locale: &str) -> day_geometry::LayoutDirection {
    let lang = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match lang.as_str() {
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ku" => {
            day_geometry::LayoutDirection::Rtl
        }
        _ => day_geometry::LayoutDirection::Ltr,
    }
}

thread_local! {
    static DIRECTION: std::cell::Cell<Option<day_geometry::LayoutDirection>> =
        const { std::cell::Cell::new(None) };
}

use day_spec::{Platform, WindowOptions};

// ---- crash observation (§8.5) --------------------------------------------------------------

/// Observer called AFTER day-core contains a panic at one of its trampoline boundaries
/// (`contain_posted_panic` here, `tree::pump_events`) — on the panicking thread, after the
/// reactive-runtime reset. A crash reporter (day-break, docs/break.md) registers one to
/// downgrade the report its panic hook just wrote: the panic was caught, the process is not
/// dying. A plain `fn` (no closure) so the containment path allocates nothing.
static CONTAINED_PANIC_OBSERVER: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Register the contained-panic observer. First registration wins; later calls are no-ops
/// (there is one crash reporter per process).
pub fn set_contained_panic_observer(f: fn()) {
    let _ = CONTAINED_PANIC_OBSERVER.set(f);
}

pub(crate) fn notify_contained_panic() {
    if let Some(f) = CONTAINED_PANIC_OBSERVER.get() {
        f();
    }
}

/// The compile-time target key of the running backend (`"macos-appkit"`, `"ios-uikit"`, …,
/// the `Platform::TARGET` string), recorded by [`launch_with`]. `None` before launch.
pub fn backend_name() -> Option<&'static str> {
    BACKEND_NAME.get().copied()
}

static BACKEND_NAME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The toolkit key of the running backend (`"appkit"`, `"gtk"`, … — the `Platform::TOOLKIT`
/// string), recorded by [`launch_with`]. `None` before launch.
pub fn toolkit_key() -> Option<&'static str> {
    TOOLKIT_KEY.get().copied()
}

static TOOLKIT_KEY: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The development tag every window title carries in a DEBUG build:
/// `(<version>/<toolkit>[/<script>])` — `(1.1.0/appkit)`, or
/// `(1.1.0/gtk/walkthrough.yaml)` while a dayscript is driving. With several apps, toolkits and
/// scripted runs open at once, the title bar is the only place that says which window is which.
///
/// `None` in a release build (this is a development aid and must never ship), and before
/// [`launch_with`] has named the backend. The version and the script name come from
/// `DAY_APP_VERSION` and `DAY_SCRIPT`, which the `day` CLI sets on every launch; run the binary
/// some other way and the tag simply carries the parts it knows.
pub fn debug_title_tag() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let toolkit = toolkit_key()?;
    // The mock backend has no window and no title bar, so there is nothing for a tag to
    // disambiguate — it would only corrupt what a headless test asserts about a title.
    if toolkit == "mock" {
        return None;
    }
    let mut parts = Vec::new();
    if let Ok(v) = std::env::var("DAY_APP_VERSION")
        && !v.is_empty()
    {
        parts.push(v);
    }
    parts.push(toolkit.to_string());
    if let Ok(s) = std::env::var("DAY_SCRIPT")
        && !s.is_empty()
    {
        parts.push(s);
    }
    Some(format!("({})", parts.join("/")))
}

/// Append [`debug_title_tag`] to a window title. Every title day sets goes through here — the
/// primary window's, each secondary window's, and every [`crate::windows::WindowHandle::set_title`].
///
/// An EMPTY title stays empty: a window the app deliberately left untitled should not grow a
/// title bar full of build metadata. An already-tagged title is left alone, since the same
/// window can be retitled repeatedly.
pub(crate) fn decorate_window_title(title: &str) -> String {
    tag_title(title, debug_title_tag().as_deref())
}

/// The join rule, split out from the environment so it can be tested.
fn tag_title(title: &str, tag: Option<&str>) -> String {
    match tag {
        Some(tag) if !title.is_empty() && !title.ends_with(tag) => format!("{title} {tag}"),
        _ => title.to_string(),
    }
}

#[cfg(test)]
mod title_tag_tests {
    use super::tag_title;

    #[test]
    fn the_tag_is_appended_once_and_never_to_an_empty_title() {
        assert_eq!(
            tag_title("Day Sheets", Some("(0.1.0/appkit)")),
            "Day Sheets (0.1.0/appkit)"
        );
        // A release build has no tag, so the title is the app's own, untouched.
        assert_eq!(tag_title("Day Sheets", None), "Day Sheets");
        // An untitled window stays untitled rather than growing a bar of build metadata.
        assert_eq!(tag_title("", Some("(0.1.0/appkit)")), "");
        // Retitling an already-tagged window must not stack tags.
        assert_eq!(
            tag_title("Day Sheets (0.1.0/appkit)", Some("(0.1.0/appkit)")),
            "Day Sheets (0.1.0/appkit)"
        );
    }
}

/// Write a framework diagnostic line to stderr, IGNORING I/O errors. `eprintln!`/`println!` PANIC
/// when the write fails — most commonly a broken/closed stderr pipe, which happens routinely when
/// the parent `day launch` tears the app down or the controlling terminal goes away. Such a panic
/// raised from inside a native trampoline (the event sink, a lifecycle callback, a GCD/glib block)
/// unwinds into non-Rust frames and ABORTS the process (`panic_cannot_unwind`) — turning a clean
/// exit into a spurious crash. Framework logging on those paths must never panic, so it goes
/// through here rather than the `*println!` macros.
pub(crate) fn diag(args: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{args}");
}

/// Run a posted main-thread task, CONTAINING any panic (the `pump_events` twin for the poster /
/// scheduler doors): log the cause and reset the reactive runtime so the app keeps running
/// (degraded) instead of aborting across the native trampoline's non-unwind boundary.
fn contain_posted_panic(f: Box<dyn FnOnce() + Send>) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        diag(format_args!(
            "day: a posted main-thread task panicked and was contained — the app continues, but \
             reactive/UI state may be inconsistent until the next interaction. Cause: {msg}"
        ));
        day_reactive::recover_from_panic();
        notify_contained_panic();
    }
}

/// A runtime route request from the backend (`Event::RouteRequested` — web-dom's URL hash
/// changing via browser back/forward or a hand-edited hash). Echoes of our own `set_route`
/// match the current route and are dropped.
fn handle_route_request(route: &str) {
    nav::apply_route_request(route);
}

/// Launch a Day app on the given platform backend: sets up the reactive scheduler and the
/// cross-thread poster, mounts the root piece into the window's content container, runs the
/// initial layout, and installs the turn-end layout callback (§3.3). The backend then owns
/// the native main loop.
pub fn launch_with<P: Platform>(
    backend: P,
    mut options: WindowOptions,
    root_piece: impl FnOnce() -> AnyPiece + 'static,
) {
    // Record the backend identity for runtime introspection (crash reports, diagnostics).
    let _ = BACKEND_NAME.set(P::TARGET);
    let _ = TOOLKIT_KEY.set(P::TOOLKIT);
    // Tag the window title with version/toolkit/script in debug builds. Pin the app's display
    // name to the UNDECORATED title first: backends fall back to `title` for the macOS App menu
    // and the About panel, which must keep reading "Day Sheets", not "Day Sheets (0.1.0/appkit)".
    if options.app_name.is_none() && !options.title.is_empty() {
        options.app_name = Some(options.title.clone());
    }
    options.title = decorate_window_title(&options.title);
    // `DAY_WINDOW=900x700` overrides the app's initial window size — responsive-layout testing
    // (scripted runs can exercise a narrow window without a resize gesture). Desktop only in
    // effect; mobile/web backends size to the screen and ignore `options.size` anyway.
    if let Ok(v) = std::env::var("DAY_WINDOW")
        && let Some((w, h)) = v.split_once('x')
        && let (Ok(w), Ok(h)) = (w.trim().parse::<f64>(), h.trim().parse::<f64>())
        && w > 0.0
        && h > 0.0
    {
        options.size = day_spec::Size::new(w, h);
    }
    // Reactive plumbing rides the platform's main-loop poster. Both doors CONTAIN panics (the
    // `pump_events` rationale, tree.rs): posted closures run inside native main-loop trampolines
    // (a glib idle, a GCD block) that ABORT the process on unwind (`panic_cannot_unwind`) — so a
    // panic in a `Setter` write's drain or a scheduled `flush_sync` would SIGABRT the app instead
    // of surfacing. Contain at this single backend-agnostic boundary and reset the runtime.
    day_reactive::install_main_poster(|f| {
        P::post(Box::new(move || contain_posted_panic(f)));
    });
    // The timer door (docs/async.md `day::sleep`): same panic containment as the poster.
    day_reactive::install_delayed_poster(|ms, f| {
        P::post_delayed(ms, Box::new(move || contain_posted_panic(f)));
    });
    day_reactive::install_scheduler(|| {
        P::post(Box::new(|| {
            contain_posted_panic(Box::new(day_reactive::flush_sync));
        }))
    });
    // The async-spawn door (docs/async.md): day-reactive's `Resource` runs its fetch futures on
    // this executor; the returned closure aborts (a no-op once the task completed — the contract
    // Resource's eager-poll ordering relies on).
    day_reactive::install_spawner(|fut| {
        let handle = present::task(fut);
        Box::new(move || handle.abort())
    });
    // The frame clock (§8.4): the animation driver re-arms the platform's vsync callback while any
    // frame consumer (game loop / self-driven animation) is live.
    frame::install_frame_requester(|cb| P::request_frame(cb));

    // WillLaunch: before the window/UI exists (docs/lifecycle.md). Fired uniformly by day-core so
    // it is reliable on every backend; handlers must not touch the tree (there isn't one yet).
    lifecycle::dispatch_lifecycle(day_spec::Lifecycle::WillLaunch);

    P::run(
        backend,
        options,
        Box::new(move |mut toolkit, root_handle, size| {
            day_spec::Toolkit::set_event_sink(&mut toolkit, Box::new(tree::enqueue_event));
            let tree = Tree::new(toolkit, root_handle, size);
            let root = tree.root();
            tree::install_tree(Box::new(tree));

            // The backend is now known: warn about any lifecycle handlers already registered for
            // phases this platform doesn't deliver (docs/lifecycle.md).
            lifecycle::warn_unsupported_registrations();

            // Window resize → relayout. Route requests (runtime deep links: web-dom's URL
            // hash changing under the app via browser back/forward or a hand-edited hash) →
            // navigate, guarded against echo — the request the backend reflects back after
            // our own `set_route` matches the current route and is dropped here.
            with_tree(|t| {
                let rn = root;
                t.on_event(
                    rn,
                    std::rc::Rc::new(move |ev| match ev {
                        day_spec::Event::WindowResized(size) => {
                            let s = *size;
                            with_tree(|t| t.set_window_size(s));
                        }
                        day_spec::Event::RouteRequested(route) => handle_route_request(route),
                        _ => {}
                    }),
                );
            });

            // Build the root piece under the window container.
            let piece = root_piece();
            let mut cx = BuildCx::new(root);
            let _ = piece.build(&mut cx);

            // Initial layout, then keep laying out at every turn boundary.
            with_tree(|t| {
                t.mark_layout_dirty();
                t.layout_if_needed();
            });
            day_reactive::on_turn_end(|| with_tree(|t| t.layout_if_needed()));

            // DidLaunch: the UI is mounted and laid out, the app is about to run (docs/lifecycle.md).
            lifecycle::dispatch_lifecycle(day_spec::Lifecycle::DidLaunch);

            // Startup deep link (docs/navigation.md): uniform across platforms — desktop
            // sets `DAY_DEEPLINK` directly, mobile shells forward the launch URL/intent into
            // it, and web-dom (no process environment) records the URL hash via
            // `set_launch_deeplink`. Deferred one turn so the first frame mounts before the
            // destination pushes. The turn-end ROUTE SYNC (`Toolkit::set_route` on change —
            // web-dom mirrors it into the URL hash) installs in the same deferred closure,
            // AFTER the deep link resolves, so the launch route is never clobbered by a sync
            // of the pre-navigation state.
            day_reactive::on_main(move || {
                if let Some(route) = nav::launch_deeplink()
                    && !nav::navigate(&route)
                {
                    eprintln!("day: launch deep link {route:?} did not match a route");
                }
                // Consume the `request_route` buffer the line above may have read: a tap that
                // cold-started the process has now been applied, and leaving it set would
                // re-navigate on the next request.
                let _ = nav::take_requested_route();
                // First reflection runs eagerly — a launch with no deep link ends no turn.
                let route = nav::current_route().unwrap_or_default();
                with_tree(|t| t.set_route(&route));
                let last = std::cell::RefCell::new(Some(route));
                day_reactive::on_turn_end(move || {
                    let route = nav::current_route().unwrap_or_default();
                    if last.borrow().as_deref() != Some(route.as_str()) {
                        *last.borrow_mut() = Some(route.clone());
                        with_tree(|t| t.set_route(&route));
                    }
                });
            });

            // Verification hook (headless CI / no-input environments): drive the app through
            // Day's own event path once the native loop starts (delayed past first allocation
            // so snapshots see a laid-out window). Precursor of dayscript (§14).
            if let Ok(spec) = std::env::var("DAY_AUTODRIVE") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    day_reactive::on_main(move || autodrive(&spec));
                });
            }
        }),
    );
}

/// `DAY_AUTODRIVE="<id>:press;<id>:text:Ada;<id>:value:80;<id>:toggle:true;<id>:tap;
/// <id>:drag:40:60;shot:/tmp/x.png"` — synthesized Day events by element id, plus snapshots.
fn autodrive(spec: &str) {
    use day_spec::{DragPhase, Event, Point};
    for step in spec.split(';').filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = step.splitn(3, ':').collect();
        if parts[0] == "shot" {
            let path = parts[1..].join(":");
            let png = with_tree(|t| t.snapshot());
            match png {
                Ok(bytes) => {
                    let _ = std::fs::write(&path, bytes);
                }
                Err(e) => eprintln!("day autodrive: snapshot failed: {e}"),
            }
            continue;
        }
        let node = with_tree(|t| t.find_by_id(parts[0]));
        let Some(node) = node else {
            eprintln!("day autodrive: id {:?} not found", parts[0]);
            continue;
        };
        // Gesture drivers (docs/shapes.md): tap fires at the node's local centre; drag runs a
        // Began→Changed→Ended sequence translated by dx,dy — exercising `.on_tap`/`.on_drag`
        // hit-testing through Day's own event path (the native recognizers deliver the same events).
        if parts.get(1) == Some(&"tap") {
            if let Some(f) = with_tree(|t| t.node_frame(node)) {
                let c = Point::new(f.size.width / 2.0, f.size.height / 2.0);
                tree::enqueue_event(tree::rnode_to_id(node), Event::Tap(c));
            }
            continue;
        }
        if parts.get(1) == Some(&"drag") {
            if let Some(f) = with_tree(|t| t.node_frame(node)) {
                let c = Point::new(f.size.width / 2.0, f.size.height / 2.0);
                let (dx, dy) = parts
                    .get(2)
                    .and_then(|s| s.split_once(':'))
                    .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
                    .unwrap_or((0.0, 0.0));
                let end = Point::new(c.x + dx, c.y + dy);
                let id = tree::rnode_to_id(node);
                tree::enqueue_event(
                    id,
                    Event::Drag {
                        phase: DragPhase::Began,
                        location: c,
                        translation: Point::ZERO,
                    },
                );
                tree::enqueue_event(
                    id,
                    Event::Drag {
                        phase: DragPhase::Changed,
                        location: end,
                        translation: Point::new(dx, dy),
                    },
                );
                tree::enqueue_event(
                    id,
                    Event::Drag {
                        phase: DragPhase::Ended,
                        location: end,
                        translation: Point::new(dx, dy),
                    },
                );
            }
            continue;
        }
        if let (Some("text"), Some(v)) = (parts.get(1).copied(), parts.get(2).copied()) {
            synthesize_text(node, v.to_string());
            continue;
        }
        let ev = match (parts.get(1).copied(), parts.get(2).copied()) {
            (Some("press"), _) => Event::Pressed,
            (Some("toggle"), Some(v)) => Event::ToggleChanged(v == "true"),
            (Some("value"), Some(v)) => Event::ValueChanged(v.parse().unwrap_or(0.0)),
            (Some("select"), Some(v)) => Event::SelectionChanged(v.parse().unwrap_or(-1)),
            _ => continue,
        };
        tree::enqueue_event(tree::rnode_to_id(node), ev);
    }
}

/// Whether the platform is rendering in dark appearance (see `Toolkit::dark_mode`): the
/// branch apps take when painting custom OPAQUE surfaces so fills track the theme that the
/// default text colors already follow.
pub fn dark_mode() -> bool {
    dark_signal().get()
}

thread_local! {
    static DARK_SIGNAL: std::cell::OnceCell<day_reactive::Signal<bool>> =
        const { std::cell::OnceCell::new() };
}

/// The reactive backing for [`dark_mode`], lazily seeded from the toolkit's answer.
fn dark_signal() -> day_reactive::Signal<bool> {
    DARK_SIGNAL.with(|c| {
        *c.get_or_init(|| {
            let seed = tree::with_tree(|t| t.dark_mode());
            day_reactive::Scope::detached().enter(|| day_reactive::Signal::new(seed))
        })
    })
}

/// Re-read the toolkit's appearance into the reactive [`dark_mode`] signal. Backends call
/// this when the SYSTEM appearance changes under a running app (macOS theme switch, GTK
/// style-manager change), and [`set_appearance`] calls it after applying an override — so
/// closures reading `dark_mode()` recolor live instead of going stale until a rebuild.
pub fn note_appearance_changed() {
    // May fire before the tree is installed (GTK's StyleManager emits `dark` notifies while
    // `startup` applies a forced DAY_THEME scheme; AppKit's observer dispatches async): no
    // tree means no palette closures exist yet, and the signal seeds from the tree on first
    // use — skip rather than panic inside a native callback.
    if let Some(d) = tree::try_with_tree(|t| t.dark_mode()) {
        dark_signal().set(d);
    }
}

/// Deliver synthetic TYPING to a text control (the dayscript `input` step and the autodrive
/// string commands both route here): paint the widget via the ordinary app-write patch, then
/// enqueue the `TextChanged` event. Both halves are needed — when a real user types, the text
/// is already in the native field by the time its change event fires, so the two-way
/// binding's echo guard deliberately suppresses the write-back (§4.4); a synthesized event
/// alone would drive the app's signal while the widget kept showing its old text.
pub fn synthesize_text(node: RNode, text: String) {
    tree::with_tree(|t| match t.node_kind(node) {
        Some(day_spec::kinds::TEXT_FIELD) => t.patch(
            node,
            Box::new(day_spec::props::TextFieldPatch::Text {
                text: text.clone(),
                from_native: false,
            }),
            false,
        ),
        Some(day_spec::kinds::TEXT_AREA) => t.patch(
            node,
            Box::new(day_spec::props::TextAreaPatch::SetText(text.clone())),
            false,
        ),
        // Other kinds (external pieces like the combobox) own their display.
        _ => {}
    });
    tree::enqueue_event(tree::rnode_to_id(node), day_spec::Event::TextChanged(text));
}

/// Override the app's appearance: `Some(true)` dark, `Some(false)` light, `None` follow the
/// system again. On backends reporting `Cap::Appearance` the native widgets restyle in place
/// and [`dark_mode`] answers the override; app-painted surfaces pick it up on their next
/// rebuild. Other backends ignore the call — probe before offering a theme picker.
pub fn set_appearance(dark: Option<bool>) {
    tree::with_tree(|t| t.set_appearance(dark));
    note_appearance_changed();
}

/// Put a badge on the app's icon — the Dock, launcher, home screen, or taskbar (docs/badge.md).
///
/// Fire-and-forget: a payload the running toolkit cannot render is ignored, so probe
/// `capability(Cap::AppBadgeCount | AppBadgeText | AppBadgeDot)` first and choose. Nothing is ever
/// substituted, because a wrong number on a user's icon is worse than no badge.
///
/// ```no_run
/// # use day_core::{set_app_badge};
/// # use day_spec::AppBadge;
/// set_app_badge(&AppBadge::Count(7));
/// set_app_badge(&AppBadge::None); // clear
/// ```
///
/// An iOS badge belongs to the INSTALLED APP and survives termination, so an app that sets one
/// usually clears it from a `WillTerminate` handler (docs/lifecycle.md); a macOS Dock badge dies
/// with the process.
pub fn set_app_badge(badge: &day_spec::AppBadge) {
    tree::with_tree(|t| t.set_app_badge(badge));
}

#[cfg(test)]
mod posted_panic_tests {
    /// A panic inside a posted main-thread task must be CONTAINED (logged + runtime reset), never
    /// unwind into the native trampoline that posted it (`panic_cannot_unwind` → SIGABRT). This is
    /// the poster/scheduler twin of `pump_events`' containment.
    #[test]
    fn posted_panic_is_contained() {
        super::contain_posted_panic(Box::new(|| panic!("boom in a posted task")));
        // Reaching here IS the assertion: the panic did not propagate. The runtime was reset, so
        // subsequent reactive work still runs.
        let s = day_reactive::Signal::new(1i32);
        s.set(2);
        assert_eq!(s.get_untracked(), 2);
    }
}
