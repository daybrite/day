//! The Day showcase (DESIGN.md Appendix A): every implemented piece behind a native navigation
//! host (docs/navigation.md) — stack presentation on mobile, sidebar + detail split on desktop.
//!
//! This crate root wires the navigation together in [`root`] and owns the app-wide lifecycle
//! plumbing; each navigation destination lives in its own module under [`pages`], and reusable
//! pieces shared by several pages live in [`widgets`].

use day::prelude::*;
use std::cell::OnceCell;

mod pages;
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
    LIFECYCLE_LOG.with(|c| *c.get_or_init(|| Signal::new("—".into())))
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
        Localization => "localization",
        Canvas => "canvas",
        List => "list",
        Refresh => "refresh",
        Scrolling => "scrolling",
        Tabs => "tabs",
        Stack => "stack",
        Media => "media",
        WebView => "webview",
        Menus => "menus",
        System => "system",
        Services => "services",
        Resources => "resources",
        Tweaks => "tweaks",
        Map => "map",
        About => "about",
    }
}

pub fn root() -> AnyPiece {
    install_locales(
        "en",
        &[
            ("en", include_str!("../resource/locales/en/app.ftl")),
            ("fr", include_str!("../resource/locales/fr/app.ftl")),
            ("zh-CN", include_str!("../resource/locales/zh-CN/app.ftl")),
            ("ar", include_str!("../resource/locales/ar/app.ftl")),
        ],
    );
    // Top-level navigation is a NavigationSplitView (docs/navigation.md): a `selector` bound
    // to an app-owned `Signal<Option<Section>>` of the active section (`None` = the collapsed
    // mobile list). Desktop shows sidebar + detail (an AdwNavigationSplitView on GTK); mobile
    // collapses to a list that pushes the detail.
    install_app_menu();
    // Lifecycle handlers (docs/lifecycle.md). On mobile this is the registration point; on desktop
    // `main` already registered them before launch (to also catch WillLaunch) — the call is idempotent.
    install_lifecycle_handlers();
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
        // Ordered as a story: author content (controls, text, drawing), put it in collections
        // and navigate it, embed rich views, then reach the platform around the app — with the
        // meta pages (resources, tweaks, about) closing the list.
        .item_icon(
            Section::Controls,
            crate::res::str::nav_controls(),
            res::images::nav_controls,
            controls_page,
        )
        .item_icon(
            Section::Dates,
            crate::res::str::nav_dates(),
            res::images::nav_dates,
            dates_page,
        )
        .item_icon(
            Section::Focus,
            crate::res::str::nav_focus(),
            res::images::nav_focus,
            focus_page,
        )
        .item_icon(
            Section::Text,
            crate::res::str::nav_text(),
            res::images::nav_text,
            text_page,
        )
        .item_icon(
            Section::Localization,
            crate::res::str::nav_localization(),
            res::images::nav_localization,
            localization_page,
        )
        .item_icon(
            Section::Canvas,
            crate::res::str::nav_canvas(),
            res::images::nav_canvas,
            canvas_page,
        )
        .item_icon(
            Section::List,
            crate::res::str::nav_list(),
            res::images::nav_list,
            list_page,
        )
        .item_icon(
            Section::Refresh,
            crate::res::str::nav_refresh(),
            res::images::nav_refresh,
            refresh_page,
        )
        .item_icon(
            Section::Scrolling,
            crate::res::str::nav_scrolling(),
            res::images::nav_scrolling,
            scrolling_page,
        )
        .item_icon(
            Section::Tabs,
            crate::res::str::nav_tabs(),
            res::images::nav_tabs,
            tabs_page,
        )
        .item_icon(
            Section::Stack,
            crate::res::str::nav_stack(),
            res::images::nav_stack,
            stack_page,
        )
        .item_icon(
            Section::Media,
            crate::res::str::nav_media(),
            res::images::nav_media,
            media_page,
        )
        .item_icon(
            Section::WebView,
            crate::res::str::nav_webview(),
            res::images::nav_webview,
            webview_page,
        )
        .item_icon(
            Section::Menus,
            crate::res::str::nav_menus(),
            res::images::nav_menus,
            menus_page,
        )
        .item_icon(
            Section::System,
            crate::res::str::nav_system(),
            res::images::nav_system,
            system_page,
        )
        .item_icon(
            Section::Services,
            crate::res::str::nav_services(),
            res::images::nav_services,
            services_page,
        )
        .item_icon(
            Section::Resources,
            crate::res::str::nav_resources(),
            res::images::nav_resources,
            resources_page,
        )
        .item_icon(
            Section::Tweaks,
            crate::res::str::nav_tweaks(),
            res::images::nav_tweaks,
            tweaks_page,
        );
    // A native MapKit map — Apple platforms only (docs/map.md).
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let nav = nav.item_icon(
        Section::Map,
        crate::res::str::nav_map(),
        res::images::nav_map,
        map_page,
    );
    nav.item_icon(
        Section::About,
        crate::res::str::nav_about(),
        res::images::nav_about,
        about_page,
    )
    .id("nav")
}

fn sidebar_header() -> AnyPiece {
    row((
        image(res::images::day_logo).frame(28.0, 28.0),
        label(crate::res::str::app_title())
            .font(Font::Headline)
            .id("home-title"),
    ))
    .spacing(8.0)
    .padding(12.0)
    .any()
}

// Mobile / embedded entries (DESIGN.md §17.4): the iOS Runner binds `day_main`, DayBridge binds the
// `Java_…` natives, and the HarmonyOS ArkTS host binds `day_arkui_start`. Every macro emits nothing
// off its own target.
day::ios_main!("Day Showcase", root);
day::android_main!(root);
day::arkui_main!(root);
