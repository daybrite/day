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
    // Top-level navigation is a NavigationSplitView (docs/navigation.md): a `selector` bound
    // to an app-owned `Signal<Option<Section>>` of the active section (`None` = the collapsed
    // mobile list). Desktop shows sidebar + detail (an AdwNavigationSplitView on GTK); mobile
    // collapses to a list that pushes the detail.
    install_app_menu();
    // Lifecycle handlers (docs/lifecycle.md). On mobile this is the registration point; on desktop
    // `main` already registered them before launch (to also catch WillLaunch) — the call is idempotent.
    install_lifecycle_handlers();
    // Remember the last-opened section across launches (docs/navigation.md). Web only, matching
    // this app's prefs policy (controls.rs): a browser reload is normal life on the web, so the
    // store is installed there and the top-level selector's `.restore` persists the section;
    // native launches install no store, so `.restore` is a silent no-op and every run starts
    // fresh — which is what the walkthrough asserts.
    #[cfg(target_arch = "wasm32")]
    day_part_prefs::install_nav_store();
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
    let nav = selector(section)
        .style(SelectorStyle::Sidebar)
        .title(crate::res::str::app_title())
        .header(sidebar_header)
        // Reopen on the last-viewed section (web only — see the install_nav_store note above).
        .restore("nav.section")
        // ALPHABETICAL by the US-English display title — keep it that way when adding a
        // page (including the cfg'd Map rebinding below, which holds its slot). About is
        // both alphabetically first and the desktop split's default detail (the split
        // selects the FIRST item when nothing is chosen).
        .item_icon(
            Section::About,
            crate::res::str::nav_about(),
            res::images::nav_about,
            about_page,
        )
        .item_icon(
            Section::Animation,
            crate::res::str::nav_animation(),
            res::images::nav_animation,
            animation_page,
        )
        .item_icon(
            Section::Canvas,
            crate::res::str::nav_canvas(),
            res::images::nav_canvas,
            canvas_page,
        )
        .item_icon(
            Section::Controls,
            crate::res::str::nav_controls(),
            res::images::nav_controls,
            controls_page,
        )
        .item_icon(
            Section::CrashReporting,
            crate::res::str::nav_crash(),
            res::images::nav_crash,
            crash_page,
        )
        .item_icon(
            Section::Dates,
            crate::res::str::nav_dates(),
            res::images::nav_dates,
            dates_page,
        )
        .item_icon(
            Section::System,
            crate::res::str::nav_system(),
            res::images::nav_system,
            system_page,
        )
        .item_icon(
            Section::Focus,
            crate::res::str::nav_focus(),
            res::images::nav_focus,
            focus_page,
        )
        .item_icon(
            Section::Grid,
            crate::res::str::nav_grid(),
            res::images::nav_grid,
            grid_page,
        )
        .item_icon(
            Section::List,
            crate::res::str::nav_list(),
            res::images::nav_list,
            list_page,
        )
        .item_icon(
            Section::Localization,
            crate::res::str::nav_localization(),
            res::images::nav_localization,
            localization_page,
        );
    // Map is Apple-only (docs/map.md) — a cfg'd rebinding keeps it in its alphabetical slot.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let nav = nav.item_icon(
        Section::Map,
        crate::res::str::nav_map(),
        res::images::nav_map,
        map_page,
    );
    let nav = nav
        .item_icon(
            Section::Media,
            crate::res::str::nav_media(),
            res::images::nav_media,
            media_page,
        )
        .item_icon(
            Section::Menus,
            crate::res::str::nav_menus(),
            res::images::nav_menus,
            menus_page,
        )
        .item_icon(
            Section::Services,
            crate::res::str::nav_services(),
            res::images::nav_services,
            services_page,
        )
        .item_icon(
            Section::Refresh,
            crate::res::str::nav_refresh(),
            res::images::nav_refresh,
            refresh_page,
        )
        .item_icon(
            Section::Resources,
            crate::res::str::nav_resources(),
            res::images::nav_resources,
            resources_page,
        )
        .item_icon(
            Section::Stack,
            crate::res::str::nav_stack(),
            res::images::nav_stack,
            stack_page,
        )
        .item_icon(
            Section::Tabs,
            crate::res::str::nav_tabs(),
            res::images::nav_tabs,
            tabs_page,
        )
        .item_icon(
            Section::Text,
            crate::res::str::nav_text(),
            res::images::nav_text,
            text_page,
        )
        .item_icon(
            Section::TextAreas,
            crate::res::str::nav_textareas(),
            res::images::nav_textareas,
            text_areas_page,
        )
        .item_icon(
            Section::Tweaks,
            crate::res::str::nav_tweaks(),
            res::images::nav_tweaks,
            tweaks_page,
        )
        .item_icon(
            Section::WebView,
            crate::res::str::nav_webview(),
            res::images::nav_webview,
            webview_page,
        );
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
