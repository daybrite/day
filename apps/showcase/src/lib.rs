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

pub fn root() -> AnyPiece {
    install_locales(
        "en",
        &[
            ("en", include_str!("../locales/en/app.ftl")),
            ("fr", include_str!("../locales/fr/app.ftl")),
        ],
    );
    // Top-level navigation is a NavigationSplitView (docs/navigation.md): a `selector` bound
    // to an app-owned `Signal<String>` of the active section. Desktop shows sidebar + detail
    // (an AdwNavigationSplitView on GTK); mobile collapses to a list that pushes the detail.
    install_app_menu();
    // Lifecycle handlers (docs/lifecycle.md). On mobile this is the registration point; on desktop
    // `main` already registered them before launch (to also catch WillLaunch) — the call is idempotent.
    install_lifecycle_handlers();
    // Deep-link: open directly on a section when `DAY_DEMO_ROUTE` is set (`day launch --env
    // DAY_DEMO_ROUTE=gauge`), else start at the root menu. Handy for driving the emulator when
    // synthetic input is unreliable.
    let section = Signal::new(std::env::var("DAY_DEMO_ROUTE").unwrap_or_default());
    let nav = selector(section)
        .style(SelectorStyle::Sidebar)
        .title(tr("app-title"))
        .header(sidebar_header)
        .item("controls", tr("nav-controls"), controls_page)
        .item("menus", tr("nav-menus"), menus_page)
        .item("text", tr("nav-text"), text_page)
        .item("gauge", tr("nav-gauge"), gauge_page)
        .item("battery", tr("nav-battery"), battery_page)
        .item("sensors", tr("nav-sensors"), sensors_page)
        .item("clipboard", tr("nav-clipboard"), clipboard_page)
        .item("network", tr("nav-network"), network_page)
        .item("haptics", tr("nav-haptics"), haptics_page)
        .item("prefs", tr("nav-prefs"), prefs_page)
        .item("deviceinfo", tr("nav-deviceinfo"), deviceinfo_page)
        .item("shapes", tr("nav-shapes"), shapes_page)
        .item("pickers", tr("nav-pickers"), pickers_page)
        .item("compose", tr("nav-compose"), compose_page)
        .item("tweaks", tr("nav-tweaks"), tweaks_page)
        .item("activity", tr("nav-activity"), activity_page)
        .item("search", tr("nav-search"), search_page)
        .item("modals", tr("nav-modals"), modals_page)
        .item("files", tr("nav-files"), files_page)
        .item("tabs", tr("nav-tabs"), tabs_page)
        .item("stack", tr("nav-stack"), stack_page)
        .item("list", tr("nav-list"), list_page)
        .item("media", tr("nav-media"), media_page)
        .item("resources", tr("nav-resources"), resources_page)
        .item("webview", tr("nav-webview"), webview_page);
    // A native Lottie animation view — iOS + Android only (docs/lottie.md).
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let nav = nav.item("lottie", tr("nav-lottie"), lottie_page);
    // A native MapKit map — Apple platforms only (docs/map.md).
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let nav = nav.item("map", tr("nav-map"), map_page);
    nav.item("about", tr("nav-about"), about_page).id("nav")
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
