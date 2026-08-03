//! The Day showcase (DESIGN.md Appendix A): every implemented piece behind a native navigation
//! host (docs/navigation.md) — stack presentation on mobile, sidebar + detail split on desktop.
//!
//! This crate root wires the navigation together in [`root`] and owns the app-wide lifecycle
//! plumbing; each navigation destination lives in its own module under [`pages`], and reusable
//! pieces shared by several pages live in [`widgets`].

use day::prelude::*;
use std::cell::OnceCell;

mod pages;
mod palette;
mod widgets;

use crate::pages::*;

/// Typed constants for the files under `resource/`, generated at build time by `day-build` (§18.5):
/// `res::images::<stem>`, `res::assets::<file>`, `res::fonts::<family>`. The showcase references its
/// bundled resources through these, so a renamed/removed file is a compile error, not a runtime miss.
pub mod res {
    include!(concat!(env!("OUT_DIR"), "/day_resources.rs"));
}

thread_local! {
    /// The most recent app-lifecycle phase, shown live on the About page (docs/lifecycle.md).
    static LIFECYCLE_LOG: OnceCell<Signal<String>> = const { OnceCell::new() };
}
pub(crate) fn lifecycle_log() -> Signal<String> {
    // `global`, NOT `new`: the first read can come from inside a page scope (on desktop-split
    // web the About page is the default detail), and a scope-owned signal would die with that
    // page — the second About visit would read a disposed signal.
    LIFECYCLE_LOG.with(|c| *c.get_or_init(|| Signal::global("—".into())))
}

/// Register app-lifecycle handlers (docs/lifecycle.md). Call this from `main` BEFORE `day::launch`
/// so the launch phases are captured. Each handler logs to the console and to a live UI readout.
///
/// The mobile-only phases are registered only where the compiled-in backend actually delivers them,
/// using the compile-time-accurate guard `day::lifecycle::supported(..)` — on desktop those `if`s are
/// `false` and the handlers are never registered, so no "unsupported phase" warning is produced.
pub fn install_lifecycle_handlers() {
    use day::Lifecycle::*;

    // Idempotent: desktop calls this from `main` (to catch WillLaunch), mobile from `root`.
    thread_local! { static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if INSTALLED.with(|c| c.replace(true)) {
        return;
    }

    let note = |phase: day::Lifecycle| {
        move || {
            eprintln!("day lifecycle: {}", phase.name());
            lifecycle_log().set(phase.name().into());
        }
    };

    // Universal phases — every backend delivers these.
    for phase in [
        WillLaunch,
        DidLaunch,
        DidBecomeActive,
        WillResignActive,
        WillTerminate,
    ] {
        day::on_lifecycle(phase, note(phase));
    }
    // Mobile-only phases — guard so we register only where they're delivered (iOS / Android).
    for phase in [
        WillEnterForeground,
        DidEnterBackground,
        DidReceiveMemoryWarning,
    ] {
        if day::lifecycle::supported(phase) {
            day::on_lifecycle(phase, note(phase));
        }
    }
}

day::routes! {
    /// The top-level sections, typed (docs/navigation.md): each variant's key is what deep
    /// links, dayscript, and `current_route()` speak; the `.item(Section::…)` declarations
    /// and any `navigate_to`/`route` call sites are compile-checked against this enum.
    pub(crate) enum Section {
        Controls => "controls",
        Dates => "dates",
        Focus => "focus",
        Text => "text",
        TextAreas => "textareas",
        Toolbars => "toolbars",
        Localization => "localization",
        Canvas => "canvas",
        Animation => "animation",
        Grid => "grid",
        List => "list",
        Refresh => "refresh",
        Tabs => "tabs",
        Stack => "stack",
        Media => "media",
        WebView => "webview",
        Menus => "menus",
        System => "system",
        Services => "services",
        Resources => "resources",
        Tweaks => "tweaks",
        CrashReporting => "crash",
        Map => "map",
        About => "about",
    }
}

/// Arm crash reporting (docs/break.md) — the Crash Reporting page demonstrates it. Idempotent
/// (day-break's `init` is single-shot); safe to call from every entry point.
pub fn install_crash_reporting() {
    let _ = day_break::Config::new()
        // "Send report" opens a prefilled email to the developer (no server needed).
        .reporter(day_break::EmailReporter::new("crashdemo@daybrite.dev"))
        .init();
}

pub fn root() -> AnyPiece {
    // Arm crash capture before the UI mounts so the Crash Reporting page's crashes are recorded.
    install_crash_reporting();
    // Every locale under `resource/locales/` (en, fr, ar, zh-CN), embedded and registered by the
    // generated catalog (§18.5) — adding a language is a new directory, nothing to edit here.
    res::locales::install();
    // Persisted theme/language overrides (docs/windows.md; the launch env wins — CI variant
    // loops with DAY_THEME/--locale stay deterministic).
    day_piece_settings::apply_startup("showcase.theme", "showcase.locale");
    // The Preferences window (Settings…/⌘, on macOS; primary+`,` elsewhere; a fullscreen
    // cover on backends without windows) and File ▸ New Window / the macOS tab-bar "+"
    // (docs/windows.md). Registered before the menu so its items lower live.
    day::register_preferences_with(
        day::WindowOptions {
            title: crate::res::str::prefs_window_title().format(),
            size: Size::new(520.0, 420.0),
            min_size: None,
            app_name: None,
        },
        pages::preferences_window,
    );
    day::register_new_window(|| {
        // Each window gets its own toolbar; the install targets the window being built.
        pages::toolbars::install();
        window_root(false)
    });
    install_app_menu();
    // The main window's own toolbar (docs/toolbars.md) — the Toolbars page drives it.
    pages::toolbars::install();
    // Lifecycle handlers (docs/lifecycle.md). On mobile this is the registration point; on desktop
    // `main` already registered them before launch (to also catch WillLaunch) — the call is idempotent.
    install_lifecycle_handlers();
    window_root(true)
}

/// One sidebar destination: the row's title and icon, and the page it opens.
///
/// `Clone` because `.items(…)` re-derives the list on every query keystroke; the fields are a
/// key, two fn pointers and a name, so a clone is cheap.
///
/// A TABLE, not a chain of `.item_icon(…)` calls, because the sidebar is filterable — its rows
/// are derived from the search query, and `.items(…)` wants a list it can re-derive. The table
/// is also what `.destination` looks a key up in, so a row and its page can never drift apart.
#[derive(Clone)]
struct Dest {
    section: Section,
    /// The generated `res::str` accessor, not a resolved `String`: the title has to be
    /// re-resolved on every derive so the rows re-title (and re-filter) on a locale switch.
    title: fn() -> day::LocalizedText,
    icon: day::prelude::ImageName,
    page: fn() -> AnyPiece,
}

/// Every destination, in the order the sidebar shows them — ALPHABETICAL by the US-English
/// display title. Keep it that way when adding a page. About is both alphabetically first and
/// the desktop split's default detail (the split selects the first row when nothing is chosen).
fn destinations() -> Vec<Dest> {
    vec![
        Dest {
            section: Section::About,
            title: crate::res::str::nav_about,
            icon: res::images::nav_about,
            page: about_page,
        },
        Dest {
            section: Section::Animation,
            title: crate::res::str::nav_animation,
            icon: res::images::nav_animation,
            page: animation_page,
        },
        Dest {
            section: Section::Canvas,
            title: crate::res::str::nav_canvas,
            icon: res::images::nav_canvas,
            page: canvas_page,
        },
        Dest {
            section: Section::Controls,
            title: crate::res::str::nav_controls,
            icon: res::images::nav_controls,
            page: controls_page,
        },
        Dest {
            section: Section::CrashReporting,
            title: crate::res::str::nav_crash,
            icon: res::images::nav_crash,
            page: crash_page,
        },
        Dest {
            section: Section::Dates,
            title: crate::res::str::nav_dates,
            icon: res::images::nav_dates,
            page: dates_page,
        },
        Dest {
            section: Section::System,
            title: crate::res::str::nav_system,
            icon: res::images::nav_system,
            page: system_page,
        },
        Dest {
            section: Section::Focus,
            title: crate::res::str::nav_focus,
            icon: res::images::nav_focus,
            page: focus_page,
        },
        Dest {
            section: Section::Grid,
            title: crate::res::str::nav_grid,
            icon: res::images::nav_grid,
            page: grid_page,
        },
        Dest {
            section: Section::List,
            title: crate::res::str::nav_list,
            icon: res::images::nav_list,
            page: list_page,
        },
        Dest {
            section: Section::Localization,
            title: crate::res::str::nav_localization,
            icon: res::images::nav_localization,
            page: localization_page,
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        Dest {
            section: Section::Map,
            title: crate::res::str::nav_map,
            icon: res::images::nav_map,
            page: map_page,
        },
        Dest {
            section: Section::Media,
            title: crate::res::str::nav_media,
            icon: res::images::nav_media,
            page: media_page,
        },
        Dest {
            section: Section::Menus,
            title: crate::res::str::nav_menus,
            icon: res::images::nav_menus,
            page: menus_page,
        },
        Dest {
            section: Section::Services,
            title: crate::res::str::nav_services,
            icon: res::images::nav_services,
            page: services_page,
        },
        Dest {
            section: Section::Refresh,
            title: crate::res::str::nav_refresh,
            icon: res::images::nav_refresh,
            page: refresh_page,
        },
        Dest {
            section: Section::Resources,
            title: crate::res::str::nav_resources,
            icon: res::images::nav_resources,
            page: resources_page,
        },
        Dest {
            section: Section::Stack,
            title: crate::res::str::nav_stack,
            icon: res::images::nav_stack,
            page: stack_page,
        },
        Dest {
            section: Section::Tabs,
            title: crate::res::str::nav_tabs,
            icon: res::images::nav_tabs,
            page: tabs_page,
        },
        Dest {
            section: Section::Text,
            title: crate::res::str::nav_text,
            icon: res::images::nav_text,
            page: text_page,
        },
        Dest {
            section: Section::TextAreas,
            title: crate::res::str::nav_textareas,
            icon: res::images::nav_textareas,
            page: text_areas_page,
        },
        Dest {
            section: Section::Toolbars,
            title: crate::res::str::nav_toolbars,
            icon: res::images::nav_toolbars,
            page: toolbars_page,
        },
        Dest {
            section: Section::Tweaks,
            title: crate::res::str::nav_tweaks,
            icon: res::images::nav_tweaks,
            page: tweaks_page,
        },
        Dest {
            section: Section::WebView,
            title: crate::res::str::nav_webview,
            icon: res::images::nav_webview,
            page: webview_page,
        },
    ]
}

/// One showcase shell — the primary window's content, and (via `register_new_window`) each
/// File ▸ New Window's. Every call creates its own section signal, so windows navigate
/// independently; app-global state (menu log, lifecycle log, controls prefs) is shared.
/// Only the PRIMARY shell joins the route namespace — secondary windows are `.local()`
/// (docs/navigation.md), so `navigate()`/dayscript keep driving the primary unambiguously.
fn window_root(primary: bool) -> AnyPiece {
    // Remember the last-opened section across launches (docs/navigation.md). Web only, matching
    // this app's prefs policy (controls.rs): a browser reload is normal life on the web, so the
    // store is installed there and the top-level selector's `.restore` persists the section;
    // native launches install no store, so `.restore` is a silent no-op and every run starts
    // fresh — which is what the walkthrough asserts.
    #[cfg(target_arch = "wasm32")]
    day::prefs::install_nav_store();
    // Deep-link: open directly on a section when `DAY_DEMO_ROUTE` is set (`day launch --env
    // DAY_DEMO_ROUTE=canvas`), else start at the root menu. Handy for driving the emulator when
    // synthetic input is unreliable.
    let section = Signal::new(
        std::env::var("DAY_DEMO_ROUTE")
            .ok()
            .and_then(|r| Section::from_key(r.split(['/', '?']).next().unwrap_or(""))),
    );
    // Each destination carries a bundled Material icon (images/nav_*.png) shown in the native nav
    // where the backend supports it (e.g. the Windows NavigationView pane).
    // The sidebar filters live on what the toolbar's search field holds (docs/localization.md
    // "Searching"): a row survives when the query is a case-insensitive prefix of one of its
    // title's words, with the words found by the current locale's own segmentation.
    let query = pages::toolbars::search_query();
    let nav = selector(section)
        .style(SelectorStyle::Sidebar)
        .title(crate::res::str::app_title())
        .header(sidebar_header)
        // Reopen on the last-viewed section (web only — see the install_nav_store note above).
        .restore("nav.section")
        .items(
            move || {
                // TRACKED: reads the query AND (through `matches_search`) the locale, so the
                // rows re-filter on a keystroke and re-title on a language switch.
                let q = query.get();
                destinations()
                    .into_iter()
                    .filter(|d| matches_search(&(d.title)().format(), &q))
                    .collect::<Vec<_>>()
            },
            |d: &Dest| item(d.section, (d.title)()).icon(d.icon.clone()),
        )
        // Dynamic rows carry no page builder of their own — the key is looked up here.
        .destination(|key: &Option<Section>| match key {
            Some(sec) => destinations()
                .into_iter()
                .find(|d| d.section == *sec)
                .map(|d| (d.page)())
                .unwrap_or_else(|| column(()).any()),
            None => column(()).any(),
        });
    let nav = if primary { nav } else { nav.local() };
    nav.id("nav")
}

fn sidebar_header() -> AnyPiece {
    // The identity block above the section list: logo beside the title with a one-line
    // tagline under it. On mobile the nav bar already shows the app title, so the tagline
    // keeps this row from reading as a duplicate; on desktop it crowns the sidebar.
    row((
        image(res::images::day_logo).frame(32.0, 32.0),
        column((
            label(crate::res::str::app_title())
                .font(Font::Headline)
                .id("home-title"),
            label(crate::res::str::app_tagline())
                .font(Font::Caption)
                .color(Color::rgba(0.55, 0.57, 0.62, 1.0)),
        ))
        .spacing(1.0)
        .align(HAlign::Leading),
    ))
    .spacing(10.0)
    .padding(12.0)
    .any()
}

// Mobile / embedded entries (DESIGN.md §17.4): the iOS Runner binds `day_main`, DayBridge binds the
// `Java_…` natives, the HarmonyOS ArkTS host binds `day_arkui_start`, and the web host page binds
// `day_dom_main`. Every macro emits nothing off its own target.
day::ios_main!("Day Showcase", root);
day::android_main!(root);
day::arkui_main!(root);
day::web_main!("Day Showcase", root);
