// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-core — the Piece model, realized tree, mounter, layout engine, and event routing
//! (DESIGN.md §5, §7). Build-once: pieces are constructed exactly once; all dynamism flows
//! through reactive bindings (day-reactive) writing to the thread-local tree.

day_reactive::tls_root! {
    layout: crate::layout::TlsGroupSlots,
    ambient: crate::ambient::TlsGroupSlots,
    anim: crate::anim::TlsGroupSlots,
    frame: crate::frame::TlsGroupSlots,
    lifecycle: crate::lifecycle::TlsGroupSlots,
    menu: crate::menu::TlsGroupSlots,
    nav: crate::nav::TlsGroupSlots,
    present: crate::present::TlsGroupSlots,
    root: crate::TlsGroupSlots,
    shield: crate::shield::TlsGroupSlots,
    toolbar: crate::toolbar::TlsGroupSlots,
    tree: crate::tree::TlsGroupSlots,
    windows: crate::windows::TlsGroupSlots,
}

/// Re-exported so every crate above day-core can reach it without taking a direct
/// day-reactive dependency (see day-reactive for what it does and why Android needs it).
pub use day_reactive::tls_group;

mod ambient;
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
pub mod tree_driver;
pub mod windows;

pub use ambient::{
    override_size_class, reset_ambient, restore_reported_size_class, safe_area, set_safe_area,
    set_size_class, set_window_safe_area, set_window_size_class, size_class, window_safe_area,
    window_size_class, window_size_class_untracked,
};
pub use anim::{current_anim, with_animation};
pub use build::*;
pub use frame::{
    FrameConsumer, add_frame_consumer, frame_consumer_count, install_frame_requester,
    remove_frame_consumer,
};
pub use layout::*;
pub use lifecycle::{dispatch_lifecycle, lifecycle_supported, on_lifecycle};
pub use list::{
    BuiltRow, ListDeleteDriver, ListDriver, ListReorderDriver, ListSwipeDriver, install_list,
    list_reload, list_scroll_to_end, list_scroll_to_row, list_set_selected, list_splice,
    list_try_delete, list_try_reorder, list_try_swipe,
};
pub use menu::{
    dispatch_menu_action, register_menu_action, register_scoped_menu_action, set_app_menu,
};
pub use nav::*;
pub use present::*;
pub use toolbar::{
    current_window, dispatch_toolbar_value, patch_toolbar, patch_window_toolbar,
    register_toolbar_value, set_toolbar, set_window_search, set_window_toolbar,
};
pub use tree_driver::{
    TreeBuiltRow, TreeDriver, TreeMovesDriver, install_tree, tree_driver, tree_reload, tree_reveal,
    tree_set_expanded, tree_set_selected, tree_try_move, tree_visible_rows,
};
// The resource seam lives in day-spec (backends depend only on day-spec); re-export for the facade.
pub use day_spec::resource::{
    AssetDir, AssetName, FontFamily, ImageName, Resource, ResourceOpener, VectorName, resource,
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

day_reactive::tls_slots! {
    root;
    static DIRECTION: std::cell::Cell<Option<day_geometry::LayoutDirection>> =
        const { std::cell::Cell::new(None) };

    static UNDO_INVOKE: std::cell::RefCell<Option<UndoInvoke>> =
        const { std::cell::RefCell::new(None) };

    /// The dayscript executor's stand-in for held modifiers (a synthetic tap cannot hold a
    /// real shift key); `None` = ask the toolkit.
    static MODIFIER_OVERRIDE: std::cell::Cell<Option<day_spec::Modifiers>> =
        const { std::cell::Cell::new(None) };

    static EDIT_INVOKE: std::cell::RefCell<Option<EditInvoke>> =
        const { std::cell::RefCell::new(None) };

    static UNDO_ACTION_IDS: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
    static EDIT_ACTION_IDS: std::cell::Cell<(u64, u64, u64, u64)> =
        const { std::cell::Cell::new((0, 0, 0, 0)) };

    static DARK_SIGNAL: std::cell::OnceCell<day_reactive::Signal<bool>> =
        const { std::cell::OnceCell::new() };
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

// ---- logging (docs/logging.md) ---------------------------------------------------------------

/// Where a formatted line goes. Unset means the process's own stderr, which is right on every
/// native target: a terminal on the desktop, Xcode's console on Apple, and logcat on Android
/// (day-android `dup2`s fd 2 into it). `wasm32-unknown-unknown` is the one target where that is
/// wrong rather than merely different — std's stdio there accepts the bytes and DROPS them, so
/// every line vanishes — and day-dom installs a sink reaching the browser console instead.
static LOG_SINK: std::sync::OnceLock<fn(log::Level, &str)> = std::sync::OnceLock::new();

/// Install the line sink. First registration wins (one host per process). Called before launch —
/// `day::web::start` does it for the browser.
pub fn set_log_sink(f: fn(log::Level, &str)) {
    let _ = LOG_SINK.set(f);
}

/// The width the level column is padded to. Shared by the formatter and the coloring below, which
/// splits the finished line at exactly this offset.
const LEVEL_WIDTH: usize = 5;

/// The terminal color for a level — `env_logger`'s palette, so a Day app's output reads like any
/// other Rust app's, and the same one `day-cli` uses when it re-emits these lines under
/// `day launch` (day-cli `src/term.rs`).
#[cfg(not(target_arch = "wasm32"))]
fn level_style(level: log::Level) -> anstyle::Style {
    let color = match level {
        log::Level::Error => anstyle::AnsiColor::Red,
        log::Level::Warn => anstyle::AnsiColor::Yellow,
        log::Level::Info => anstyle::AnsiColor::Green,
        log::Level::Debug => anstyle::AnsiColor::Blue,
        log::Level::Trace => anstyle::AnsiColor::Cyan,
    };
    anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(color)))
}

/// Write one already-formatted line to stderr, never panicking. That matters because the
/// `*println!` macros panic on a failed write, a closed stderr pipe is routine when `day launch`
/// tears the app down, and a panic raised inside a native trampoline aborts the process.
///
/// Only the level column is colored — a fully colored line is noise at `INFO`, and leaving the
/// message plain keeps it greppable. `anstream` decides whether the escapes survive: they are
/// stripped when stderr is not a color terminal, which covers the pipe `day launch` gives us
/// (day-cli re-colors from its own end), logcat, Xcode's console, `NO_COLOR`, and `TERM=dumb`.
#[cfg(not(target_arch = "wasm32"))]
fn write_line(level: log::Level, line: &str) {
    use std::io::Write as _;
    let style = level_style(level);
    // The column is ASCII and padded to `LEVEL_WIDTH` by the caller, so this never lands inside a
    // character; `split_at_checked` keeps even a malformed line from panicking.
    let (lvl, rest) = line.split_at_checked(LEVEL_WIDTH).unwrap_or((line, ""));
    let _ = writeln!(anstream::stderr(), "{style}{lvl}{style:#}{rest}");
}

/// The wasm fallback, used only if no sink was installed — `day::web::start` installs one before
/// an app's first line. std's stdio on `wasm32-unknown-unknown` accepts these bytes and drops
/// them, so this writes into nothing; it exists so the logger has a total function.
#[cfg(target_arch = "wasm32")]
fn write_line(_level: log::Level, line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// Day's default [`log::Log`].
///
/// Not `env_logger` or another off-the-shelf backend, for the reason above: they print through
/// `*println!`, and a panic on that path is an abort rather than a lost line. An app that wants
/// one installs it itself — see [`init_logging`].
struct DayLogger;

impl log::Log for DayLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        // The level filter is `log::set_max_level`'s job, checked by the macros before they even
        // format; answering `true` here keeps `log_enabled!` honest for anything that asks.
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // `WARN day_gtk: could not register bundled font …` — the level first so a scan down the
        // column finds problems, then the emitting crate, which is what says *which* backend or
        // piece is unhappy. The old hand-written `day: ` prefixes said only that it was us.
        let sink = LOG_SINK.get();
        // Formatted plain: a sink is a structured destination (the browser console, a test
        // collector, an app's own transport) and wants the text, not terminal escapes. Color is
        // `write_line`'s business, applied only on the way to a real stderr.
        let line = format!(
            "{:<LEVEL_WIDTH$} {}: {}",
            record.level(),
            record.target(),
            record.args()
        );
        match sink {
            Some(f) => f(record.level(), &line),
            None => write_line(record.level(), &line),
        }
    }

    fn flush(&self) {}
}

/// Install Day's default logger and level, unless the app already installed its own.
///
/// Called from [`launch_with`], so logging works with no ceremony at all: an app writes
/// `log::info!(…)` (or `day::info!`, the same macro re-exported through the prelude) and the line
/// comes out on every platform. `set_logger` answers `Err` when a logger is already registered —
/// that is the whole customization story, and why the error is deliberately ignored: an app that
/// called `env_logger::init()` (or installed `tracing`, or its own `log::Log`) before
/// `day::launch` keeps it, and Day does not fight for the slot.
///
/// The default level is `Debug` in a debug build and `Info` in a release one; `DAY_LOG` overrides
/// it with a level name (`off`/`error`/`warn`/`info`/`debug`/`trace`). Reading an environment
/// variable is a native-only affordance — on the web there is no process environment, so the
/// launch path sets the level explicitly from the page's query string instead.
pub fn init_logging() {
    let level = std::env::var("DAY_LOG")
        .ok()
        .and_then(|v| v.parse::<log::LevelFilter>().ok())
        .unwrap_or(if cfg!(debug_assertions) {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        });
    set_log_level(level);
    // Err = the app got there first, which is exactly the intended escape hatch.
    let _ = log::set_logger(&DayLogger);
}

/// Set the maximum level that will be emitted. Separate from [`init_logging`] because the web
/// launch path has to supply the level from the page URL rather than the environment, and because
/// an app may want to raise or lower it at runtime.
pub fn set_log_level(level: log::LevelFilter) {
    log::set_max_level(level);
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
        log::warn!(
            "a posted main-thread task panicked and was contained — the app continues, but \
             reactive/UI state may be inconsistent until the next interaction. Cause: {msg}"
        );
        day_reactive::recover_from_panic();
        notify_contained_panic();
    }
}

/// The app's undo/redo entry point as the platform front calls it — `true` means redo.
/// `Rc` because the invocation clones it out of the cell before running, so the borrow is
/// released before app code (which may install a new bridge) gets control.
type UndoInvoke = std::rc::Rc<dyn Fn(bool)>;

/// Wire an undo history to the platform (docs/model.md): the four signals mirror into the
/// toolkit's native front where one exists (`Cap::UndoBridge` — the stock Edit menu retitles
/// and enables itself, the platform's gestures land), and every invocation the platform
/// delivers comes back through `on_invoke(redo)`. On a toolkit without a native undo system
/// the state goes nowhere and the app's own affordances call the stack directly — installing
/// the bridge is still harmless. Call once, after launch; installing again replaces the wiring.
pub fn install_undo_bridge(
    can_undo: day_reactive::Signal<bool>,
    can_redo: day_reactive::Signal<bool>,
    undo_label: day_reactive::Signal<String>,
    redo_label: day_reactive::Signal<String>,
    on_invoke: impl Fn(bool) + 'static,
) {
    UNDO_INVOKE.with(|u| *u.borrow_mut() = Some(std::rc::Rc::new(on_invoke)));
    day_reactive::bind(
        move || day_spec::UndoState {
            can_undo: can_undo.get(),
            can_redo: can_redo.get(),
            undo_label: undo_label.get(),
            redo_label: redo_label.get(),
        },
        |state: &day_spec::UndoState| {
            let state = state.clone();
            with_tree(|t| t.set_undo_state(&state));
        },
    );
}

/// The standing dispatch id behind a `MenuRole::Cut`/`Copy`/`Paste` item on a toolkit whose
/// role items come back as plain menu actions (docs/menus.md): the closure invokes the
/// installed edit bridge. Durable across menu installs, like the undo pair.
pub fn edit_action_id(op: day_spec::EditOp) -> u64 {
    use day_spec::EditOp as E;
    let (c, y, p, a) = EDIT_ACTION_IDS.with(|s| s.get());
    let have = match op {
        E::Cut => c,
        E::Copy => y,
        E::Paste => p,
        E::SelectAll => a,
    };
    if have != 0 {
        return have;
    }
    let id = menu::register_menu_action(std::rc::Rc::new(move || dispatch_edit_invoke(op)));
    EDIT_ACTION_IDS.with(|s| {
        let (c0, y0, p0, a0) = s.get();
        s.set(match op {
            E::Cut => (id, y0, p0, a0),
            E::Copy => (c0, id, p0, a0),
            E::Paste => (c0, y0, id, a0),
            E::SelectAll => (c0, y0, p0, id),
        });
    });
    id
}

fn dispatch_undo_invoke(redo: bool) {
    let _ = invoke_undo(redo);
}

/// Invoke the installed undo bridge — the SAME handler a native front (⌘Z, the Edit menu,
/// a three-finger swipe) reaches — returning whether one is installed. The dayscript
/// `undo:`/`redo:` steps drive history through this, so a walkthrough needs no app-provided
/// undo button on any target.
pub fn invoke_undo(redo: bool) -> bool {
    let f = UNDO_INVOKE.with(|u| u.borrow().clone());
    match f {
        Some(f) => {
            day_reactive::batch(|| f(redo));
            true
        }
        None => false,
    }
}

type EditInvoke = std::rc::Rc<dyn Fn(day_spec::EditOp)>;

/// The keyboard modifiers held right now — for interactions whose meaning they change
/// (shift-click adds to a selection). Touch backends answer all-false; a dayscript step's
/// declared modifiers take precedence while it dispatches.
pub fn modifiers() -> day_spec::Modifiers {
    if let Some(m) = MODIFIER_OVERRIDE.with(|o| o.get()) {
        return m;
    }
    with_tree(|t| t.modifiers())
}

/// Scoped modifier stand-in for the dayscript executor: `Some` while a step with declared
/// modifiers dispatches, back to `None` after.
pub fn set_modifier_override(m: Option<day_spec::Modifiers>) {
    MODIFIER_OVERRIDE.with(|o| o.set(m));
}

/// Wire the app's standard-edit handlers to the platform (docs/menus.md): `state` is a
/// TRACKED read whose value mirrors into the toolkit (`Cap::EditBridge` — native menu
/// validation enables the stock Cut/Copy/Paste exactly as it does for text widgets), and
/// every invocation a platform route delivers comes back through `on_invoke`. On a toolkit
/// with no native route, the `menu_role(Cut/Copy/Paste)` items dispatch here instead (the
/// same standing-id fallback the undo pair uses). Call once, after launch; installing again
/// replaces the wiring. Most apps want [`day::install_edit_commands`], which adds the
/// clipboard transport.
pub fn install_edit_bridge(
    state: impl Fn() -> day_spec::EditState + 'static,
    on_invoke: impl Fn(day_spec::EditOp) + 'static,
) {
    EDIT_INVOKE.with(|u| *u.borrow_mut() = Some(std::rc::Rc::new(on_invoke)));
    day_reactive::bind(state, |state: &day_spec::EditState| {
        let state = *state;
        with_tree(|t| t.set_edit_state(&state));
    });
}

fn dispatch_edit_invoke(op: day_spec::EditOp) {
    let _ = invoke_edit(op);
}

/// Invoke the installed edit bridge — the SAME handler the platform's own Cut/Copy/Paste
/// route reaches (clipboard transport included) — returning whether one is installed. An
/// app's own affordances (a context menu's Cut/Copy) call this rather than duplicating the
/// clipboard plumbing (docs/menus.md).
pub fn invoke_edit(op: day_spec::EditOp) -> bool {
    let f = EDIT_INVOKE.with(|u| u.borrow().clone());
    match f {
        Some(f) => {
            day_reactive::batch(|| f(op));
            true
        }
        None => false,
    }
}

/// The standing dispatch id behind a `MenuRole::Undo`/`Redo` item on a toolkit with no native
/// undo responder (docs/menus.md): activating the item comes back as a plain menu action, and
/// this id's closure invokes the installed undo bridge. Registered once and durable across
/// menu installs, like the preferences id. Toolkits WITH a native undo system (appkit) keep
/// their responder-chain selector instead, so a focused text field's own undo stays ahead of
/// the app stack there.
pub fn undo_action_id(redo: bool) -> u64 {
    let (u, r) = UNDO_ACTION_IDS.with(|c| c.get());
    let have = if redo { r } else { u };
    if have != 0 {
        return have;
    }
    let id = menu::register_menu_action(std::rc::Rc::new(move || dispatch_undo_invoke(redo)));
    UNDO_ACTION_IDS.with(|c| {
        let (u0, r0) = c.get();
        c.set(if redo { (u0, id) } else { (id, r0) });
    });
    id
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
    // Before anything else that might want to report: from here on `log::warn!` and friends come
    // out, on every backend, with no app-side setup. An app that installed its own logger first
    // keeps it (docs/logging.md).
    init_logging();
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
    // Backend FFI trampolines (JNI up-calls, C callbacks, posted closures) contain panics
    // through day-spec's `ffi_guard`; hand it the recovery hook here — the one layer that
    // knows day-reactive — so a contained panic can't strand the observer stack or leave a
    // half-open batch behind.
    day_spec::ffi_guard::set_recovery(day_reactive::recover_from_panic);
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

            // Seed the window's size class from the size the backend just reported
            // (docs/size-classes.md), BEFORE the root piece builds below — a nav host resolving
            // an automatic presentation reads it during its own build. Backends push later
            // changes themselves; one that never does simply keeps this launch value.
            ambient::set_window_size_class(
                root,
                day_spec::SizeClass::from_size(size.width, size.height),
            );

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
                            // Re-bucket the window (docs/size-classes.md). Derived HERE, from the
                            // geometry every backend already reports, so there is one breakpoint
                            // table rather than one per toolkit — and only a class CHANGE
                            // notifies, so dragging an edge within a bucket costs nothing.
                            ambient::set_window_size_class(
                                rn,
                                day_spec::SizeClass::from_size(s.width, s.height),
                            );
                        }
                        day_spec::Event::RouteRequested(route) => handle_route_request(route),
                        // A native undo front's invocation (⌘Z through the stock Edit menu, a
                        // three-finger swipe): route to whatever stack the app installed.
                        day_spec::Event::Undo { redo } => dispatch_undo_invoke(*redo),
                        // A platform edit-command route (Edit ▸ Cut/Copy/Paste, the
                        // browser's clipboard events) with no text widget claiming it.
                        day_spec::Event::Edit(op) => dispatch_edit_invoke(*op),
                        _ => {}
                    }),
                );
            });

            // The first window is an ordinary primary window (docs/windows.md close policy):
            // it goes in the registry like any other, so closing it runs the same teardown and
            // the same "was that the last primary?" question. Its content builds in a CHILD of
            // the root scope — disposing that on close takes this window's tree and nothing
            // else, while `Signal::global` state stays on the root scope above it.
            let win_scope = day_reactive::Scope::root().enter(day_reactive::Scope::child);
            windows::adopt_initial_window(root, win_scope);

            // Build the root piece under the window container.
            let piece = root_piece();
            win_scope.enter(|| {
                let mut cx = BuildCx::new(root);
                let _ = piece.build(&mut cx);
            });

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
                    log::warn!("launch deep link {route:?} did not match a route");
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
                Err(e) => log::warn!("day autodrive: snapshot failed: {e}"),
            }
            continue;
        }
        let node = with_tree(|t| t.find_by_id(parts[0]));
        let Some(node) = node else {
            log::warn!("day autodrive: id {:?} not found", parts[0]);
            continue;
        };
        // Gesture drivers (docs/shapes.md): tap fires at the node's local center; drag runs a
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

// ---------------------------------------------------------------------------
// Webview-evaluation seam (docs/webview-eval.md): a webview PIECE registers its evaluator
// here; the dayscript `web_eval` step drives it. day-core owns only the handoff — the
// script engine and the piece never depend on each other.
// ---------------------------------------------------------------------------

/// A webview piece's evaluator: run RAW `script` (the piece applies its own envelope) against
/// a node of the piece's kind, delivering `Ok(json)` — the value as JSON text — or `Err` (a
/// human-readable message) to the callback at a later event drain, never inline.
pub type WebviewEvalFn = std::rc::Rc<dyn Fn(RNode, &str, WebviewEvalDone)>;
/// The reply callback [`WebviewEvalFn`] must eventually call (a dropped callback shows up as
/// a step that waits out its retry window — deliver an `Err` instead where possible).
pub type WebviewEvalDone = Box<dyn FnOnce(Result<String, String>)>;

thread_local! {
    static WEBVIEW_EVAL: std::cell::RefCell<Option<(day_spec::PieceKind, WebviewEvalFn)>> =
        const { std::cell::RefCell::new(None) };
}

/// Register the evaluator for webview nodes of `kind`. Called by the webview piece's
/// constructors (idempotent — the last registration wins); an app that never builds a web
/// view never registers one, and the `web_eval` step says so.
pub fn register_webview_eval(kind: day_spec::PieceKind, f: WebviewEvalFn) {
    WEBVIEW_EVAL.with(|c| *c.borrow_mut() = Some((kind, f)));
}

/// Start an evaluation for the dayscript `web_eval` step: `false` when no evaluator is
/// registered or `node` is not the registered webview kind (the step fails, non-retryably,
/// with that story); `true` means the evaluator took the request and WILL call `done`.
pub fn webview_eval(node: RNode, script: &str, done: WebviewEvalDone) -> bool {
    let Some((kind, f)) = WEBVIEW_EVAL.with(|c| c.borrow().clone()) else {
        return false;
    };
    if tree::with_tree(|t| t.node_kind(node)) != Some(kind) {
        return false;
    }
    f(node, script, done);
    true
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
