// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Day — the umbrella crate apps depend on. One backend feature per binary (§3.2).

#[cfg(any(
    all(feature = "appkit", feature = "gtk"),
    all(feature = "appkit", feature = "mock"),
    all(feature = "gtk", feature = "mock"),
    all(feature = "qt", feature = "appkit"),
    all(feature = "qt", feature = "gtk"),
    all(feature = "qt", feature = "mock"),
    all(feature = "xaml", feature = "appkit"),
    all(feature = "xaml", feature = "gtk"),
    all(feature = "xaml", feature = "qt"),
    all(feature = "xaml", feature = "mock"),
    all(feature = "dom", feature = "appkit"),
    all(feature = "dom", feature = "gtk"),
    all(feature = "dom", feature = "qt"),
    all(feature = "dom", feature = "xaml"),
    all(feature = "dom", feature = "mock"),
))]
compile_error!("day: enable exactly one backend feature");

/// Programmatic navigation on the deep-link rail (docs/deep-links.md, docs/navigation.md):
/// route the app to `route` — buffered before the root is ready, applied after, exactly like a
/// cold launcher shortcut. The route grammar is what `day::routes!` keys and dayscript speak.
pub use day_core::request_route;
pub use day_core::{
    AnyPiece, BuildCx, Piece, PieceSeq, TaskHandle, dark_mode, safe_area, set_app_badge,
    set_appearance, size_class, sleep, task,
};
pub use day_core::{AssetDir, AssetName, FontFamily, ImageName, Resource, VectorName, resource};
pub use day_spec::AppBadge;
/// An app-writable scratch directory. The OS temp dir is NOT app-writable on every target
/// (Android reports `getCacheDir()`), so a backend records the right location at startup and
/// this is how an app asks for it. For derived files an app can rebuild: rendered documents,
/// thumbnails, export staging.
pub use day_spec::present::app_temp_dir;
pub use day_spec::{KeyEvent, Modifiers};
// Secondary windows (docs/windows.md): open/find windows, the kind that shapes their
// chrome, and the preferences/new-window registrations behind the auto menu items.
pub use day_core::{
    WindowHandle, focused_window, open_new_window, open_preferences, open_window,
    register_new_window, register_preferences, register_preferences_with, window_by_key,
};
pub use day_spec::WindowKind;
/// The reactive core, whole (docs/async.md): `day::reactive::{Resource, Load}` for async data
/// loading — namespaced because the prelude's `Resource` is the ASSET handle above, a different
/// type that predates the async one.
pub mod reactive {
    pub use day_reactive::*;
}

/// Persistent settings (docs/prefs.md): a small key/value string store backed by each platform's
/// native facility — `NSUserDefaults` on Apple, `SharedPreferences` on Android, a file store
/// elsewhere. `day::prefs::{get, set, remove, contains}`, plus `bind` to persist a `Signal` and
/// `install_nav_store` to make `.restore(key)` navigation survive a relaunch
/// (docs/navigation.md).
///
/// Promoted into the facade rather than left a satellite because it is what nearly every app
/// reaches for first, and because it is the one part that lives in the reactive layer. It stays
/// its own crate (`day-part-prefs`), so existing `day_part_prefs::…` paths keep working; this is
/// the same API under a shorter name. Opt out with `default-features = false`.
/// The per-property observable store (docs/model.md): `Store`/`Keyed`/`Elem`/`Field`, the
/// change log, and the `#[derive(Observable)]` accessors' machinery.
#[cfg(feature = "model")]
pub mod model {
    pub use day_model::*;
}

/// SQLite persistence for the observable model (docs/persistence.md): `ModelContainer`,
/// `#[derive(Model)]`, drivers and codecs. The engine is a facade feature — bundled by
/// default, `sqlite-system` or `sqlite-cipher` instead.
#[cfg(feature = "persistence")]
pub mod persistence {
    pub use day_persistence::*;
}

/// Wire an [`model::UndoStack`] to the platform: native fronts (the stock Edit menu, ⌘Z,
/// iOS's three-finger gestures) where the toolkit has them, and the stack's own signals for
/// the app's buttons everywhere (docs/model.md). One call, after the stack exists.
pub use day_core::invoke_edit;
pub use day_spec::EditOp;

#[cfg(feature = "model")]
pub fn install_undo(stack: &day_model::UndoStack) {
    let s = stack.clone();
    day_core::install_undo_bridge(
        stack.can_undo(),
        stack.can_redo(),
        stack.undo_label(),
        stack.redo_label(),
        move |redo| {
            if redo {
                s.redo();
            } else {
                s.undo();
            }
        },
    );
}

/// Wire the app's shape/object Cut/Copy/Paste to the platform's own edit commands
/// (docs/menus.md): the SAME menu items, shortcuts, and responder precedence text editing
/// uses — a focused text field keeps its clipboard behavior, everything else reaches these
/// handlers. `can_copy` is a tracked read (drive it from your selection); `copy`/`cut`
/// return the serialized payload (its format is the app's: Day-Sketch uses SVG), which day
/// places on the system clipboard; `paste` receives whatever text the clipboard holds.
/// Menu items come from `menu_role(MenuRole::Cut/Copy/Paste)`.
pub fn install_edit_commands(
    can_copy: impl Fn() -> bool + 'static,
    copy: impl Fn() -> Option<String> + 'static,
    cut: impl Fn() -> Option<String> + 'static,
    paste: impl Fn(&str) + 'static,
    select_all: impl Fn() + 'static,
) {
    day_core::install_edit_bridge(
        move || {
            let can = can_copy();
            day_spec::EditState {
                can_cut: can,
                can_copy: can,
                can_paste: true,
                can_select_all: true,
            }
        },
        move |op| match op {
            day_spec::EditOp::Copy => {
                if let Some(payload) = copy() {
                    let _ = day_part_clipboard::set_text(&payload);
                }
            }
            day_spec::EditOp::Cut => {
                if let Some(payload) = cut() {
                    let _ = day_part_clipboard::set_text(&payload);
                }
            }
            day_spec::EditOp::Paste => {
                if let Some(text) = day_part_clipboard::get_text() {
                    paste(&text);
                }
            }
            day_spec::EditOp::SelectAll => select_all(),
        },
    );
}

/// The keyboard modifiers held right now — for interactions whose meaning they change
/// (shift-click adds to a selection instead of replacing it). Touch platforms answer
/// all-false; a dayscript step's declared modifiers take precedence while it dispatches.
pub fn modifiers() -> day_spec::Modifiers {
    day_core::modifiers()
}

#[cfg(feature = "prefs")]
pub mod prefs {
    pub use day_part_prefs::*;
}
// Localization text source + arg trait (§12) at the crate root so generated `res::str::<key>(…)`
// functions can name `day::LocalizedText` / `day::tr` / `day::IntoFArg` (also in the prelude).
pub use day_core::{lifecycle_supported, on_lifecycle};
pub use day_fluent::{IntoFArg, IntoNumberFArg, LocalizedText, tr};

/// The `log` crate itself, for an app that needs more than the macros (a `LevelFilter`, a custom
/// `log::Log`). Re-exported by NAME so `use day::prelude::*` is enough — the same rule the model
/// and persistence re-exports follow.
pub use ::log;
/// Raise or lower the level Day's default logger emits, at runtime. `DAY_LOG` sets it at startup
/// on native targets; the web launch path reads `?DAY_LOG=` from the page URL.
pub use day_core::set_log_level;
/// Logging (docs/logging.md). `day::info!("…")` and friends are the `log` crate's macros,
/// re-exported so an app needs no `log` dependency of its own for the common case — and, because
/// they ARE `log`'s, anything that speaks `log` (`env_logger`, `tracing-log`, a hand-written
/// `log::Log`) works without adapters.
///
/// Day installs a default logger at launch, so these come out with no setup on every platform:
/// stderr natively, the browser console on web-dom. An app that wants something else calls
/// `log::set_logger` (or `env_logger::init()`) BEFORE `day::launch` and keeps it.
pub use log::{debug, error, info, log_enabled, trace, warn};

/// A PNG of this window, as the app itself sees it (docs/window-image.md).
///
/// The same capture the dayscript `screenshot` step takes, offered to the app: the window's
/// CONTENT — what Day drew — without the platform's chrome around it. [`WindowImage::chrome`]
/// asks for the chrome as well.
///
/// Synchronous, because every backend that can do this at all has a synchronous way to: an
/// offscreen render on the desktop toolkits, `UIGraphicsImageRenderer` on UIKit, `View.draw` on
/// Android, `componentSnapshot.getSync` on ArkUI. Feed the bytes straight to
/// [`save_file`](day_pieces::save_file), which is the async half.
///
/// ```no_run
/// # use day::prelude::*;
/// day::task(async move {
///     if let Ok(png) = day::window_image().capture() {
///         let _ = save_file(png).suggested_name("shot.png").filter("PNG", &["png"]).await;
///     }
/// });
/// ```
///
/// `Err` where the toolkit cannot rasterize its own window — today only web-dom, which would need
/// a rasterizer shipped with it. Ask [`window_image_support`] first when the answer decides
/// whether to show a button at all.
pub fn window_image() -> WindowImage {
    WindowImage { chrome: false }
}

/// Whether this toolkit can hand the app a picture of its own window (`Cap::Snapshot`).
pub fn window_image_support() -> day_spec::Support {
    day_core::with_tree(|t| t.window_image_support())
}

/// The request built by [`window_image`].
#[derive(Clone, Copy, Debug)]
pub struct WindowImage {
    chrome: bool,
}

impl WindowImage {
    /// Include the window's own chrome — title bar, toolbar, whatever the platform draws around
    /// the content. A backend with nothing to add (or no way to separate the two) answers with
    /// the content capture rather than failing.
    pub fn chrome(mut self) -> Self {
        self.chrome = true;
        self
    }

    /// Take the picture.
    pub fn capture(self) -> Result<Vec<u8>, String> {
        day_core::with_tree(|t| {
            if self.chrome {
                t.snapshot_chrome()
            } else {
                t.snapshot()
            }
        })
    }
}

/// An app-environment value, portably: the process environment on native targets, the page
/// URL's query string on web-dom (where a browser sandbox has no process environment —
/// `day launch --env K=V` forwards each pair as a query parameter, docs/web.md). Prefer this
/// over `std::env::var` for anything a `--env` flag should be able to set on every target.
pub fn env(key: &str) -> Option<String> {
    #[cfg(all(target_arch = "wasm32", feature = "dom"))]
    {
        day_dom::host_env(key)
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "dom")))]
    {
        std::env::var(key).ok()
    }
}
// Same reason: the generated `res::locales::install()` names `day::install_locales` (§18.5).
pub use day_fluent::install as install_locales;
// Locale-aware comparison/sorting (docs/localization.md "Sorting") — icu4x collation, so e.g. a
// Chinese list sorts by pinyin. `compare` and `sort_localized` track the locale signal.
pub use day_fluent::{compare, compare_in, sort_localized};
// Search matching (docs/localization.md "Searching"): case-insensitive, at the start of any
// word, with words found by the locale's own segmentation. `matches_search` tracks the locale.
pub use day_fluent::{matches_search, matches_search_in};
// The current-locale Signal itself, for apps that show or branch on it (`locale().get()` is a
// tracked read; `set_locale` in the prelude writes it).
pub use day_fluent::locale;
// Tweaks (docs/tweaks.md): the realized-node id, the size-invalidation hook for native
// mutations Day can't see, and the retained ref live in the prelude via day-pieces.
pub use day_core::{RNode, invalidate_size};
pub use day_pieces::NativeRef;
// Typed routes (docs/navigation.md): `day::routes! { enum Section { Home => "home", … } }`.
pub use day_pieces::routes;
pub use day_spec::{Lifecycle, WindowOptions};

/// dayscript **recording** and **action logging** (docs/dayscript.md "Recording", DESIGN §14.6):
/// install an observer that sees the user's taps, edits, selections, and navigation.
/// `day::record::{start, start_into, start_to_file, stop, is_recording, recording_signal, script,
/// steps, save, clear, exclude_prefix}` captures them into a replayable dayscript; a recorder also
/// arms headlessly from `day launch --record <file>` (the `DAY_RECORD` env, honored inside
/// `day_script::init`). What it records is an ordinary dayscript, so it replays cross-toolkit
/// through [`play_script`] or `day launch -p <target> --script <file>`.
///
/// `day::record::log_actions(true)` (or `DAY_LOG_ACTIONS=1`) turns on the narration alone — every
/// action echoed to stdout in the same vocabulary, keeping nothing:
///
/// ```text
/// dayscript ▸ navigate → dates  "Date & time"
/// dayscript ▸ tap list-shuffle  "Shuffle"
/// ```
pub mod record {
    pub use day_script::record::*;
}

/// Replay a dayscript against the running app, in-process (docs/dayscript.md "Recording"): parse
/// `yaml` and run each step through the embedded engine, on the main thread between flushes — the
/// same executor `day launch --script` drives. Returns an error while a recording is live (a replay
/// must not record itself) and on web (no background thread — drive the page over the WebSocket
/// transport there). See [`record`].
pub fn play_script(yaml: &str) -> Result<(), String> {
    day_script::play(yaml)
}

/// The display name of the toolkit compiled into THIS binary — `"AppKit"`, `"GTK"`, `"Qt"`,
/// `"UIKit"`, `"Android"`, `"XAML"`, `"ArkUI"`, `"DOM"` (or `"Mock"`). Handy for a window
/// title that names its backend.
pub const fn toolkit_name() -> &'static str {
    #[cfg(feature = "appkit")]
    {
        return "AppKit";
    }
    #[cfg(feature = "gtk")]
    {
        return "GTK";
    }
    #[cfg(feature = "qt")]
    {
        return "Qt";
    }
    #[cfg(feature = "uikit")]
    {
        return "UIKit";
    }
    #[cfg(feature = "mdc")]
    {
        return "Android";
    }
    #[cfg(feature = "xaml")]
    {
        return "XAML";
    }
    #[cfg(feature = "arkui")]
    {
        return "ArkUI";
    }
    #[cfg(feature = "dom")]
    {
        return "DOM";
    }
    #[allow(unreachable_code)]
    {
        "Mock"
    }
}

pub mod prelude {
    pub use day_fluent::{
        LocalizedText, install as install_locales, matches_search, set_locale, sort_localized, tr,
    };
    pub use day_pieces::prelude::*;
    pub use day_spec::{Lifecycle, Size, WindowOptions};
    // Canvas drawing vocabulary (docs/canvas.md): arbitrary paths and their fill rule, plus the
    // stroke style a dashed or round-capped line needs.
    pub use day_spec::{FillRule, LineCap, LineJoin, Path, PathSeg, StrokeStyle};
    // SVG path data to a `PathBuilder` chain, at compile time (docs/canvas.md).
    pub use day_macros::build_path;
    // The observable store (docs/model.md). The crate itself is re-exported by NAME because the
    // derive's generated code says `day_model::…` — this is what makes `use day::prelude::*`
    // enough. day-model's own `Path` stays out: the prelude's `Path` is the canvas one.
    #[cfg(feature = "model")]
    pub use ::day_model;
    #[cfg(feature = "model")]
    pub use ::day_model::{Elem, Field, Key, Keyed, ModelId, Source, Store, Uuid};
    #[cfg(feature = "model")]
    pub use day_macros::Observable;
    // Persistence (docs/persistence.md). Same by-NAME rule: `#[derive(Model)]`'s generated
    // code says `day_persistence::…`.
    #[cfg(feature = "persistence")]
    pub use ::day_persistence;
    #[cfg(feature = "persistence")]
    pub use ::day_persistence::{
        DeleteRule, Many, Model, ModelContainer, One, Recorder, Secret, Sqlite, schema,
    };
    #[cfg(feature = "persistence")]
    pub use day_macros::Model;
    pub use day_spec::Point;
    // Logging (docs/logging.md): `info!`/`warn!`/`error!`/`debug!`/`trace!` with no setup and no
    // `log` dependency in the app's own Cargo.toml. These are `log`'s macros, so an app that later
    // installs `env_logger` or `tracing` keeps every call site it already wrote.
    pub use super::{debug, error, info, log, log_enabled, set_log_level, trace, warn};
    pub use {super::lifecycle_supported, super::on_lifecycle};
    // Bundled-resource random-access API (§18.3): `resource("name")` -> `Resource`.
    pub use day_core::{
        AssetDir, AssetName, FontFamily, ImageName, Resource, VectorName, resource,
    };
    pub use day_spec::present::app_temp_dir;
    // Toolkit capability probe (docs): lets app/piece content adapt to the backend, e.g. skip a
    // title the native nav already shows (`Cap::NavHeader`). `capability(cap) -> Support`.
    pub use day_core::capability;
    // Safe-area insets (docs/layout.md): zero everywhere except edge-to-edge backends — pad
    // content by it where a background runs under the system bars.
    pub use day_core::safe_area;
    // The window's size class (docs/size-classes.md): what a `nav()` host resolves its own
    // presentation from, and what an app lays out from when it wants to make the same call —
    // two columns on a wide window, one on a narrow one. `None` on a backend that reports no
    // geometry. Tracked, so a piece reading it rebuilds when the window crosses a breakpoint.
    pub use day_core::size_class;
    pub use day_spec::{Cap, HeightClass, SizeClass, Support, WidthClass};
    // Layout direction (docs/localization): `is_rtl()` lets a piece mirror its own drawing under a
    // right-to-left locale — the layout engine mirrors placement, but a `canvas` owns its coordinates.
    pub use day_core::{is_rtl, layout_direction};
    pub use day_spec::LayoutDirection;
}

/// App-lifecycle support for the backend compiled into THIS binary (docs/lifecycle.md).
///
/// Register handlers with [`on_lifecycle`]; guard phases a platform may not deliver either at runtime
/// (`if day::lifecycle::supported(p) { … }`) or at compile time with [`require_lifecycle!`].
pub mod lifecycle {
    pub use day_core::{lifecycle_supported, on_lifecycle};
    pub use day_spec::Lifecycle;

    /// Does the backend compiled into this binary deliver `phase`? A `const fn`, so it drives both a
    /// runtime guard and the compile-time [`crate::require_lifecycle!`] assertion. Agrees with the
    /// runtime [`day_core::lifecycle_supported`] once the app is running.
    pub const fn supported(phase: Lifecycle) -> bool {
        #[cfg(feature = "appkit")]
        {
            return day_appkit::lifecycle_supported(phase);
        }
        #[cfg(feature = "gtk")]
        {
            return day_gtk::lifecycle_supported(phase);
        }
        #[cfg(feature = "qt")]
        {
            return day_qt::lifecycle_supported(phase);
        }
        #[cfg(all(feature = "uikit", target_os = "ios"))]
        {
            return day_uikit::lifecycle_supported(phase);
        }
        #[cfg(all(feature = "mdc", target_os = "android"))]
        {
            return day_android::lifecycle_supported(phase);
        }
        #[cfg(all(feature = "xaml", windows))]
        {
            return day_xaml::lifecycle_supported(phase);
        }
        // No concrete backend (mock, or a mobile backend compiled for the host to check): the
        // universal phases are always deliverable.
        #[allow(unreachable_code)]
        {
            phase.is_universal()
        }
    }
}

/// Compile-time assert that the backend in this binary delivers `$phase`, else a build error. Use it
/// to make a hard dependency on a platform-specific phase explicit:
/// `day::require_lifecycle!(day::Lifecycle::DidEnterBackground);` fails to compile on desktop.
/// For soft handling, guard with [`lifecycle::supported`] / [`lifecycle_supported`] instead.
#[macro_export]
macro_rules! require_lifecycle {
    ($phase:expr) => {
        const {
            ::core::assert!(
                $crate::lifecycle::supported($phase),
                "this Day backend does not deliver that lifecycle phase (see docs/lifecycle.md)",
            )
        }
    };
}

/// Launch through an EXTERNAL toolkit's backend (docs/extending.md "External toolkits") — the
/// cfg-free counterpart of the feature-gated `launch` entries below, for platform-toolkit pairs
/// registered via `[package.metadata.day.toolkit]`. Feature-independent on purpose: `day build
/// -p <external>` compiles the app with only the external toolkit's own feature, so none of the
/// launchers below exist in that build. Starts the dayscript engine exactly as they do — which
/// is what keeps `day launch --script`, `day drive`, and the session registry working on a
/// backend this repository has never heard of.
pub fn launch_external<P: day_spec::Platform, R: Piece>(
    backend: P,
    options: WindowOptions,
    root: impl FnOnce() -> R + 'static,
) {
    day_script::init();
    start(backend, options, root);
}

/// Start `backend`, seeding the ambient locale first.
///
/// The ORDER is the whole point: the OS's language preference has to reach day-l10n before the
/// app's `res::locales::install()` runs, and that call lives inside the root builder — so the
/// hints are read from the live backend here, one step before it is handed the root. Without this
/// step `Toolkit::locale_hints` is a trait method nobody calls, and every native app opens in its
/// default language whatever the device is set to (docs/localization.md).
// Every caller is a `launch` behind its own backend feature (§3.2), so a featureless
// `cargo check -p day` — which is what a bare workspace check builds — has none of them and would
// otherwise report this as dead. Listing the backends here instead would be a second copy of the
// cfg set below, drifting the first time one is added.
#[allow(dead_code)]
fn start<P: day_spec::Platform, R: Piece>(
    backend: P,
    mut options: WindowOptions,
    root: impl FnOnce() -> R + 'static,
) {
    day_fluent::add_launch_locales(&backend.locale_hints());
    // The app's catalog, now that the hints are in: registering it here rather than inside the
    // root builder is what lets a title — drawn before any piece exists — come out of the
    // catalog too (docs/localization.md).
    if let Some((default, catalog)) = options.locales {
        day_fluent::install(default, catalog);
    }
    if let Some(f) = options.title_fn {
        options.title = f();
    }
    day_core::launch_with(backend, options, move || AnyPiece::new(root()));
}

/// Launch the app on the selected backend (blocks; owns the native main loop).
#[cfg(feature = "appkit")]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    day_script::init();
    start(day_appkit::AppKit::new(), options, root);
}

#[cfg(feature = "gtk")]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    day_script::init();
    start(day_gtk::Gtk::new(), options, root);
}

#[cfg(feature = "qt")]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    day_script::init();
    start(day_qt::Qt::new(), options, root);
}

#[cfg(all(feature = "uikit", target_os = "ios"))]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    day_script::init();
    start(day_uikit::Uikit::new(), options, root);
}

#[cfg(all(feature = "xaml", windows))]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    day_script::init();
    start(day_xaml::Xaml::new(), options, root);
}

#[cfg(all(feature = "dom", target_arch = "wasm32"))]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    // No day_script::init(): dayscript's TCP transport has no wasm equivalent yet
    // (a WebSocket transport is planned; see docs/web.md).
    start(day_dom::Dom::new(), options, root);
}

#[cfg(feature = "mock")]
pub fn launch<P: Piece>(options: WindowOptions, root: impl FnOnce() -> P + 'static) {
    let (mock, _probe) = day_mock::MockToolkit::new();
    start(mock, options, root);
}

// ---------------------------------------------------------------------------
// App entry macros (§17.4): the mobile shells bind fixed exported symbols
// (Runner/main.swift → `day_main`; dev.daybrite.day.bridge.DayBridge → `Java_…` natives).
// These expand to that glue so an app's lib.rs carries one line per platform.
// Both emit nothing off their target OS, so apps invoke them unconditionally.
// ---------------------------------------------------------------------------

/// One entry point for every platform that needs one — the single line an app's `lib.rs` carries
/// instead of one macro per platform.
///
/// ```ignore
/// day::day_start!("My App", root);    // or: day::day_start!(root);
/// ```
///
/// It expands to every platform macro below. Each of those is gated on its own target
/// (`ios` / `macos` / `android` / the `ohos` env / `wasm32`), and those gates are mutually
/// exclusive, so exactly one survives a given build — and none at all on a plain cargo desktop
/// build, where `src/main.rs` is the entry instead.
///
/// The title reaches the platforms that have nowhere else to get one. Android and HarmonyOS take
/// their label from the app manifest (`android:label`, `app_name` in `string.json`) rather than
/// from Rust, so they accept the argument and ignore it; passing it here keeps one call site for
/// every target instead of making the caller remember which two are different.
#[macro_export]
macro_rules! day_start {
    ($root:expr) => {
        $crate::day_start!("", $root);
    };
    // The full description, shared with `src/main.rs` so both entry points open the same window
    // and perform the same ceremony — `options: window()` rather than a bare title. The literal
    // `options:` is what tells this arm apart from the title one below.
    (options: $options:expr, $root:expr) => {
        $crate::day_start_ios!(options: $options, $root);
        $crate::day_start_macos!(options: $options, $root);
        $crate::day_start_android!($root);
        $crate::day_start_arkui!($root);
        $crate::day_start_web!(options: $options, $root);
    };
    ($title:expr, $root:expr) => {
        $crate::day_start_ios!($title, $root);
        $crate::day_start_macos!($title, $root);
        $crate::day_start_android!($root);
        $crate::day_start_arkui!($root);
        $crate::day_start_web!($title, $root);
    };
}

/// Surface the resources `day-build` generated for this crate as `pub mod res` (§18.5).
///
/// ```ignore
/// day::resources!();   // then: res::str::app_title(), res::images::logo, res::locales::CATALOG
/// ```
///
/// Expands to the `include!` of `$OUT_DIR/day_resources.rs`, so the crate needs the `day-build`
/// build script that writes it — an app with no `build.rs` has no `OUT_DIR` and should not call
/// this. Kept separate from [`day_start!`] deliberately: entry points are per-target `cfg`,
/// while `res` has to exist on every build, including the desktop one whose entry is
/// `src/main.rs`.
#[macro_export]
macro_rules! resources {
    () => {
        /// Typed constants for the files under `resource/`, generated at build time by
        /// `day-build` (§18.5): `res::images::<stem>`, `res::assets::<file>`,
        /// `res::fonts::<family>`, `res::str::<key>()`, and the `res::locales` catalog.
        /// Reference bundled resources through these — `image(res::images::app_logo)` — so a
        /// typo is a compile error and the resource is guaranteed present. Drop a file into
        /// `resource/images/` and its constant appears on the next build.
        pub mod res {
            include!(concat!(env!("OUT_DIR"), "/day_resources.rs"));
        }
    };
}

/// Expands to the `day_main` C export the iOS Runner's `main.swift` calls
/// (`@_silgen_name("day_main")`). The optional title is currently unused on
/// iOS (the window fills the screen bounds); accepted for future window-scene use.
///
/// Apps normally reach this through [`day_start!`] rather than calling it directly.
///
/// ```ignore
/// day::day_start_ios!(root);              // or: day::day_start_ios!("My App", root);
/// ```
#[macro_export]
macro_rules! day_start_ios {
    ($root:expr) => {
        $crate::day_start_ios!("", $root);
    };
    (options: $options:expr, $root:expr) => {
        /// iOS entry: the Runner's main.swift calls this from the app staticlib (§17.4).
        #[cfg(target_os = "ios")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_main() {
            $crate::launch($options, $root);
        }
    };
    ($title:expr, $root:expr) => {
        $crate::day_start_ios!(
            options: $crate::WindowOptions {
                title: ($title).into(),
                ..::core::default::Default::default()
            },
            $root
        );
    };
}

/// Expands to the `day_main` C export the macOS Runner's `main.swift` calls — the
/// `platform/macos/` Xcode host project's entry (§17.4). The cargo-driven build keeps using
/// the app's own `src/main.rs`; both paths call the same [`launch`], so the app behaves
/// identically however it was built.
///
/// ```ignore
/// day::day_start_macos!(root);            // or: day::day_start_macos!("My App", root);
/// ```
#[macro_export]
macro_rules! day_start_macos {
    ($root:expr) => {
        $crate::day_start_macos!("", $root);
    };
    (options: $options:expr, $root:expr) => {
        /// macOS entry: the Runner's main.swift calls this from the app staticlib (§17.4).
        #[cfg(target_os = "macos")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_main() {
            $crate::launch($options, $root);
        }
    };
    ($title:expr, $root:expr) => {
        $crate::day_start_macos!(
            options: $crate::WindowOptions {
                title: ($title).into(),
                // The same desktop default the scaffold's src/main.rs uses.
                size: $crate::prelude::Size::new(960.0, 640.0),
                ..::core::default::Default::default()
            },
            $root
        );
    };
}

/// Expands to the three JNI exports `dev.daybrite.day.bridge.DayBridge`'s natives resolve
/// against in the app cdylib (`nativeStart`/`nativeOnEvent`/`nativeRunPosted`),
/// wired to the given root piece.
///
/// ```ignore
/// day::day_start_android!(root);
/// ```
#[macro_export]
macro_rules! day_start_android {
    ($root:expr) => {
        // jni 0.22 native methods receive the FFI-safe `EnvUnowned`; `with_env` upgrades it to the
        // real `Env` (sharing the frame's `'local`, so the object args pass straight in) and wraps
        // the body in a `catch_unwind` so a panic never unwinds across the JNI boundary.
        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeStart<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            root: $crate::android::jni::objects::JObject<'local>,
            density: $crate::android::jni::sys::jfloat,
            w: $crate::android::jni::sys::jint,
            h: $crate::android::jni::sys::jint,
            autodrive: $crate::android::jni::objects::JString<'local>,
            locale: $crate::android::jni::objects::JString<'local>,
            env_blob: $crate::android::jni::objects::JString<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    let a = $crate::android::read_jstring(env, &autodrive);
                    let l = $crate::android::read_jstring(env, &locale);
                    let e = $crate::android::read_jstring(env, &env_blob);
                    $crate::android::start(env, root, density, w, h, a, l, e, $root);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeStartWindow<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            root: $crate::android::jni::objects::JObject<'local>,
            node: $crate::android::jni::sys::jlong,
            _density: $crate::android::jni::sys::jfloat,
            w: $crate::android::jni::sys::jint,
            h: $crate::android::jni::sys::jint,
        ) -> $crate::android::jni::sys::jboolean {
            let mut ok = false;
            let _ = env
                .with_env(|env| {
                    ok = $crate::android::window_started(env, root, node, w, h);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
            ok as $crate::android::jni::sys::jboolean
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeOnEvent<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            id: $crate::android::jni::sys::jlong,
            kind: $crate::android::jni::sys::jint,
            num: $crate::android::jni::sys::jdouble,
            s: $crate::android::jni::objects::JString<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    $crate::android::dispatch_event(env, id, kind, num, &s);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeRunPosted(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            token: $crate::android::jni::sys::jlong,
        ) {
            $crate::android::run_posted(token);
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeDoFrame(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            token: $crate::android::jni::sys::jlong,
            frame_nanos: $crate::android::jni::sys::jlong,
        ) {
            $crate::android::run_frame(token, frame_nanos);
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListLen(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
        ) -> $crate::android::jni::sys::jint {
            $crate::android::list_len(host_id) as $crate::android::jni::sys::jint
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListBind<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            host_id: $crate::android::jni::sys::jlong,
            position: $crate::android::jni::sys::jint,
            cell: $crate::android::jni::objects::JObject<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    $crate::android::list_bind(env, host_id, position, cell);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListRecycle<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            host_id: $crate::android::jni::sys::jlong,
            cell: $crate::android::jni::objects::JObject<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    $crate::android::list_recycle(env, host_id, cell);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListIsSelected(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
            position: $crate::android::jni::sys::jint,
        ) -> $crate::android::jni::sys::jboolean {
            $crate::android::list_is_selected(host_id, position)
                as $crate::android::jni::sys::jboolean
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListCanDrop(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
            from: $crate::android::jni::sys::jint,
            to: $crate::android::jni::sys::jint,
        ) -> $crate::android::jni::sys::jboolean {
            $crate::android::list_can_drop(host_id, from, to) as $crate::android::jni::sys::jboolean
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListMove(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
            from: $crate::android::jni::sys::jint,
            to: $crate::android::jni::sys::jint,
        ) -> $crate::android::jni::sys::jboolean {
            $crate::android::list_move(host_id, from, to) as $crate::android::jni::sys::jboolean
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListCanDelete(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
            index: $crate::android::jni::sys::jint,
        ) -> $crate::android::jni::sys::jboolean {
            $crate::android::list_can_delete(host_id, index) as $crate::android::jni::sys::jboolean
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListDelete(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
            index: $crate::android::jni::sys::jint,
        ) -> $crate::android::jni::sys::jboolean {
            $crate::android::list_delete(host_id, index) as $crate::android::jni::sys::jboolean
        }
    };
}

/// Android glue (§17.4): the app cdylib's JNI exports forward here.
#[cfg(all(feature = "mdc", target_os = "android"))]
pub mod android {
    pub use day_android::jni;
    pub use day_android::{
        dispatch_event, list_bind, list_can_delete, list_can_drop, list_delete, list_is_selected,
        list_len, list_move, list_recycle, read_jstring, run_frame, run_posted, window_started,
    };

    #[allow(clippy::too_many_arguments)]
    pub fn start<R: crate::Piece>(
        env: &mut jni::Env,
        root: jni::objects::JObject,
        density: f32,
        w: i32,
        h: i32,
        autodrive: Option<String>,
        locale: Option<String>,
        env_blob: Option<String>,
        root_piece: impl FnOnce() -> R + 'static,
    ) {
        // Before any println!: send stdout/stderr to logcat (Android drops them otherwise).
        day_android::redirect_stdio_to_logcat();
        if let Some(a) = autodrive {
            unsafe { std::env::set_var("DAY_AUTODRIVE", a) };
        }
        if let Some(l) = locale {
            unsafe { std::env::set_var("DAY_LOCALE", l) };
        }
        if let Some(blob) = env_blob {
            for line in blob.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    unsafe { std::env::set_var(k, v) };
                }
            }
        }
        day_android::init(env, root, density, w, h);
        day_script::init();
        // Through `crate::start`, not `launch_with`: that is where the device's language
        // preference is read off the backend and handed to the localization engine, and this
        // entry is the app's only door on Android (docs/localization.md).
        crate::start(
            day_android::Android::new(),
            crate::WindowOptions::default(),
            root_piece,
        );
    }
}

/// Expands to the `day_arkui_start` C export the HarmonyOS ArkUI shim's `start(...)` NAPI wrapper
/// calls (from ArkTS: `import native from 'libday_arkui.so'; native.start(nodeContent, w, h, density)`).
/// It mounts the app's `root` piece into the ArkTS `NodeContent` and runs the loop.
///
/// ```ignore
/// day::day_start_arkui!(root);
/// ```
#[macro_export]
macro_rules! day_start_arkui {
    ($root:expr) => {
        /// HarmonyOS entry: the ArkUI shim's NAPI `start` calls this from the app cdylib (§17.4).
        #[cfg(target_env = "ohos")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_arkui_start(
            content: *mut ::core::ffi::c_void,
            w: f64,
            h: f64,
            density: f64,
        ) {
            $crate::arkui::start(content, w, h, density, $root);
        }

        /// Deep-link intake (docs/deep-links.md): the shim's NAPI `deepLink(uri)` calls this
        /// from the app cdylib for cold and warm links alike — `request_route` buffers before
        /// launch and navigates on the UI thread after.
        #[cfg(target_env = "ohos")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_arkui_deeplink(uri: *const ::core::ffi::c_char) {
            $crate::arkui::deeplink(uri);
        }
    };
}

/// Expands to the `day_dom_main` C export the web host's `shim.js` calls once the wasm module
/// is instantiated (`wasm.day_dom_main()` at the end of `start()` in `crates/day-cli/resources/web/shim.js`).
///
/// ```ignore
/// day::day_start_web!(root);              // or: day::day_start_web!("My App", root);
/// ```
#[macro_export]
macro_rules! day_start_web {
    ($root:expr) => {
        $crate::day_start_web!("", $root);
    };
    (options: $options:expr, $root:expr) => {
        /// Web entry: the host page's shim calls this from the app cdylib (§17.4).
        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_dom_main() {
            $crate::web::start($options, $root);
        }
    };
    ($title:expr, $root:expr) => {
        $crate::day_start_web!(
            options: $crate::WindowOptions {
                title: ($title).into(),
                ..::core::default::Default::default()
            },
            $root
        );
    };
}

/// Web glue (§17.4): the app cdylib's `day_dom_main` export forwards here.
#[cfg(all(feature = "dom", target_arch = "wasm32"))]
pub mod web {
    /// One dayscript request line from the page's WebSocket (docs/web.md). Lives here — not in
    /// day-dom — because backends depend only on day-spec; the umbrella is where the backend
    /// and the engine meet.
    #[unsafe(no_mangle)]
    pub extern "C" fn day_dom_script_line(ptr: *mut u8, len: usize) {
        let line = day_dom::take_alloc_string(ptr, len);
        day_script::web_line(&line);
    }

    /// Install the panic hook (panics report through the browser console before the trap),
    /// hand the page's locale (`?locale=` else the browser languages) to the localization
    /// engine and its URL hash to the deep-link seam, arm the dayscript web transport when
    /// the serving `day launch` session invites it (`?dayscript=` token), and launch `root`
    /// into the host page's day root.
    pub fn start<P: crate::Piece>(
        options: crate::WindowOptions,
        root: impl FnOnce() -> P + 'static,
    ) {
        day_dom::install_panic_hook();
        // Point the logger at the browser console BEFORE anything can log (docs/logging.md).
        // Installed here rather than inside day-dom because a backend depends only on day-spec —
        // the facade is where the toolkit and the core are allowed to meet. Without this every
        // line would be silently dropped: std's stdio on wasm32-unknown-unknown takes the bytes
        // and discards them.
        day_core::set_log_sink(day_dom::console_sink);
        // `DAY_LOG` has no process environment to live in here; the launch server forwards it as
        // a query parameter. `init_logging` (from `launch_with`) sets the default level, so this
        // only has to override when the page actually asked.
        if let Some(level) = day_dom::launch_log_level() {
            day_core::set_log_level(level);
        }
        if let Some(locale) = day_dom::launch_locale() {
            day_fluent::set_launch_locale(&locale);
        }
        if let Some(route) = day_dom::launch_route() {
            day_core::set_launch_deeplink(&route);
        }
        if let Some(token) = day_dom::dayscript_token() {
            day_script::web_init(token, day_dom::script_send);
        }
        crate::launch(options, root);
    }
}

/// HarmonyOS ArkUI glue (§17.4): the app cdylib's `day_arkui_start` export forwards here.
#[cfg(all(feature = "arkui", target_env = "ohos"))]
pub mod arkui {
    use core::ffi::c_void;

    /// Mount `root` into the ArkTS `NodeContent` and run the loop. `w_vp`/`h_vp` are the content
    /// size in vp; `density` is px-per-vp (both passed by the ArkTS host).
    pub fn start<R: crate::Piece>(
        content: *mut c_void,
        w_vp: f64,
        h_vp: f64,
        density: f64,
        root: impl FnOnce() -> R + 'static,
    ) {
        day_arkui::init(content, w_vp, h_vp, density);
        // Point the logger at hilog BEFORE anything can log (docs/logging.md): std's stdio
        // goes nowhere in an OHOS ability, so without this every `log::` line is dropped —
        // the same facade-installed sink rule as web.
        day_core::set_log_sink(day_arkui::hilog_sink);
        day_script::init();
        // `crate::start` seeds the device's language preference before the root builds
        // (docs/localization.md); ArkUI's own hint list arrives once day-arkui implements
        // `locale_hints`.
        crate::start(
            day_arkui::ArkUi::new(),
            crate::WindowOptions::default(),
            root,
        );
    }

    /// A deep link from the ArkTS host (docs/deep-links.md): a cold `want.uri` (delivered
    /// before `start`) or a warm `onNewWant` one. `request_route` makes the two the same
    /// call — buffered until the first mount, applied on the UI thread after it.
    pub fn deeplink(uri: *const core::ffi::c_char) {
        if uri.is_null() {
            return;
        }
        // SAFETY: the shim passes a NUL-terminated copy of the ArkTS string, valid for the call.
        let uri = unsafe { core::ffi::CStr::from_ptr(uri) };
        if let Ok(uri) = uri.to_str() {
            day_core::request_route(&day_spec::route_of_url(uri));
        }
    }
}
