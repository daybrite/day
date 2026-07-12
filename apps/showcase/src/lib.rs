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

thread_local! {
    /// The most recent app-lifecycle phase, shown live on the Menus page (docs/lifecycle.md).
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
        Text => "text",
        Canvas => "canvas",
        System => "system",
        Services => "services",
        Menus => "menus",
        Modals => "modals",
        List => "list",
        Tabs => "tabs",
        Stack => "stack",
        Media => "media",
        Resources => "resources",
        WebView => "webview",
        Tweaks => "tweaks",
        Map => "map",
        About => "about",
    }
}

pub fn root() -> AnyPiece {
    install_locales(
        "en",
        &[
            ("en", include_str!("../locales/en/app.ftl")),
            ("fr", include_str!("../locales/fr/app.ftl")),
            ("zh-CN", include_str!("../locales/zh-CN/app.ftl")),
            ("ar", include_str!("../locales/ar/app.ftl")),
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
    let nav = selector(section)
        .style(SelectorStyle::Sidebar)
        .title(tr("app-title"))
        .header(sidebar_header)
        .item(Section::Controls, tr("nav-controls"), controls_page)
        .item(Section::Text, tr("nav-text"), text_page)
        .item(Section::Canvas, tr("nav-canvas"), canvas_page)
        .item(Section::System, tr("nav-system"), system_page)
        .item(Section::Services, tr("nav-services"), services_page)
        .item(Section::Menus, tr("nav-menus"), menus_page)
        .item(Section::Modals, tr("nav-modals"), modals_page)
        .item(Section::List, tr("nav-list"), list_page)
        .item(Section::Tabs, tr("nav-tabs"), tabs_page)
        .item(Section::Stack, tr("nav-stack"), stack_page)
        .item(Section::Media, tr("nav-media"), media_page)
        .item(Section::Resources, tr("nav-resources"), resources_page)
        .item(Section::WebView, tr("nav-webview"), webview_page)
        .item(Section::Tweaks, tr("nav-tweaks"), tweaks_page);
    // A native MapKit map — Apple platforms only (docs/map.md).
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let nav = nav.item(Section::Map, tr("nav-map"), map_page);
    nav.item(Section::About, tr("nav-about"), about_page)
        .id("nav")
}

fn sidebar_header() -> AnyPiece {
    row((
        image("day_logo").frame(28.0, 28.0),
        label(tr("app-title")).font(Font::Headline).id("home-title"),
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
