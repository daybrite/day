//! The Day showcase (DESIGN.md Appendix A, staged): every implemented piece behind a
//! native navigation host (docs/navigation.md) — stack presentation on mobile, sidebar +
//! detail split on desktop. Three destinations: controls, gauge, about.

use day::prelude::*;
use day_part_haptics::Haptic;
use day_piece_activity::activity;
use day_piece_combobox::combo_box;
use day_piece_media::media;
use day_piece_picker::picker;
use day_piece_rating::{Card, badge, rating};
use day_piece_searchfield::search_field;
use day_piece_webview::web_view;
use std::cell::OnceCell;

thread_local! {
    /// The last menu action fired — shared between the app menu (installed in `root`) and the Menus
    /// page so both demonstrate action dispatch. Created lazily inside the reactive runtime.
    static MENU_LOG: OnceCell<Signal<String>> = const { OnceCell::new() };
    /// The most recent app-lifecycle phase, shown live on the Menus page (docs/lifecycle.md).
    static LIFECYCLE_LOG: OnceCell<Signal<String>> = const { OnceCell::new() };
}
fn menu_log() -> Signal<String> {
    MENU_LOG.with(|c| *c.get_or_init(|| Signal::new("—".into())))
}
fn lifecycle_log() -> Signal<String> {
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
// Lottie renders a native LottieAnimationView — iOS + Android only, so both the import and the page
// are gated to those targets (its front-end compiles everywhere, but it only renders there).
#[cfg(any(target_os = "ios", target_os = "android"))]
use day_piece_lottie::lottie;
// A native MapKit map view — Apple platforms only (docs/map.md).
#[cfg(any(target_os = "macos", target_os = "ios"))]
use day_piece_map::map;

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

/// One button that plays a haptic and records the style name into `#haptics-last-played`.
fn haptic_button(
    id: &'static str,
    title: LocalizedText,
    h: Haptic,
    last: Signal<String>,
) -> AnyPiece {
    button(title)
        .action(move || {
            day_part_haptics::play(h);
            last.set(
                tr("haptics-last-played")
                    .arg("style", format!("{h:?}"))
                    .format(),
            );
        })
        .id(id)
        .any()
}

/// Haptics playground (docs/haptics.md): the headless `day-part-haptics` part fires a native haptic
/// for each style; `#haptics-last-played` echoes the last one so the walkthrough can assert it.
fn haptics_page() -> AnyPiece {
    let last = Signal::new(tr("haptics-none").format());
    // Report whether this platform has a haptic engine (each branch a full `tr(...)` for `day lint`).
    let supported = if day_part_haptics::is_supported() {
        tr("haptics-supported-yes")
    } else {
        tr("haptics-supported-no")
    };
    column((
        label(tr("nav-haptics"))
            .font(Font::Title)
            .id("haptics-title"),
        label(tr("haptics-caption")),
        label(supported).id("haptics-supported"),
        // Impact intensities.
        row((
            haptic_button("haptics-light", tr("haptics-light"), Haptic::Light, last),
            haptic_button("haptics-medium", tr("haptics-medium"), Haptic::Medium, last),
            haptic_button("haptics-heavy", tr("haptics-heavy"), Haptic::Heavy, last),
        ))
        .spacing(8.0),
        // Notification outcomes.
        row((
            haptic_button(
                "haptics-success",
                tr("haptics-success"),
                Haptic::Success,
                last,
            ),
            haptic_button(
                "haptics-warning",
                tr("haptics-warning"),
                Haptic::Warning,
                last,
            ),
            haptic_button("haptics-error", tr("haptics-error"), Haptic::Error, last),
        ))
        .spacing(8.0),
        haptic_button(
            "haptics-selection",
            tr("haptics-selection"),
            Haptic::Selection,
            last,
        ),
        divider(),
        row((
            label(tr("haptics-last")),
            label(move || last.get()).id("haptics-last-played"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Preferences playground (docs/prefs.md): the headless `day-part-prefs` part persists a
/// string under a fixed key. Save writes the text field, Load reads it back into `#prefs-value`,
/// and Clear deletes it. The value survives app launches (NSUserDefaults / SharedPreferences / a
/// config file, per platform), so Load returns the stored value even after the field is typed over.
fn prefs_page() -> AnyPiece {
    const KEY: &str = "showcase.remembered";
    let field = Signal::new(String::new());
    let value = Signal::new(tr("prefs-empty").format());
    let status = Signal::new(tr("prefs-idle").format());
    column((
        label(tr("nav-prefs")).font(Font::Title).id("prefs-title"),
        label(tr("prefs-caption")),
        text_field(field)
            .placeholder(tr("prefs-placeholder"))
            .id("prefs-field"),
        row((
            button(tr("prefs-save"))
                .action(move || {
                    let ok = field.with(|t| day_part_prefs::set(KEY, t));
                    let msg = if ok {
                        tr("prefs-saved")
                    } else {
                        tr("prefs-save-failed")
                    };
                    status.set(msg.format());
                })
                .id("prefs-save"),
            button(tr("prefs-load"))
                .action(move || match day_part_prefs::get(KEY) {
                    Some(v) => {
                        value.set(v);
                        status.set(tr("prefs-loaded").format());
                    }
                    None => {
                        value.set(tr("prefs-empty").format());
                        status.set(tr("prefs-missing").format());
                    }
                })
                .id("prefs-load"),
            button(tr("prefs-clear"))
                .action(move || {
                    day_part_prefs::remove(KEY);
                    value.set(tr("prefs-empty").format());
                    status.set(tr("prefs-cleared").format());
                })
                .id("prefs-clear"),
        ))
        .spacing(8.0),
        label(move || status.get()).id("prefs-status"),
        row((
            label(tr("prefs-value-label")),
            label(move || value.get()).id("prefs-value"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

fn deviceinfo_page() -> AnyPiece {
    // Read the device identity once now (headless day-part-deviceinfo); Refresh re-polls it.
    let (m, s, sim) = deviceinfo_lines();
    let model = Signal::new(m);
    let system = Signal::new(s);
    let simulator = Signal::new(sim);
    column((
        label(tr("nav-deviceinfo"))
            .font(Font::Title)
            .id("deviceinfo-title"),
        label(tr("deviceinfo-caption")),
        label(move || model.get()).id("deviceinfo-model"),
        label(move || system.get()).id("deviceinfo-system"),
        label(move || simulator.get()).id("deviceinfo-simulator"),
        button(tr("deviceinfo-refresh"))
            .action(move || {
                let (m, s, sim) = deviceinfo_lines();
                model.set(m);
                system.set(s);
                simulator.set(sim);
            })
            .id("deviceinfo-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Read the native device identity and format each field as a localized line:
/// `(model, "name version", simulator)`. Values vary by host, so nothing is asserted exactly.
fn deviceinfo_lines() -> (String, String, String) {
    let d = day_part_deviceinfo::get();
    let model = tr("deviceinfo-model").arg("value", d.model).format();
    let system = tr("deviceinfo-system")
        .arg("name", d.system_name)
        .arg("version", d.system_version)
        .format();
    // Each branch is a full literal tr(...) call so `day lint` sees both keys (never tr(if ...)).
    let sim_value = if d.is_simulator {
        tr("deviceinfo-yes").format()
    } else {
        tr("deviceinfo-no").format()
    };
    let simulator = tr("deviceinfo-simulator").arg("value", sim_value).format();
    (model, system, simulator)
}

/// Bundled resources (§18.3): an image loaded *by name* from the `images/` resource (the native
/// image pipeline), plus efficient random-access reads of arbitrary embedded data via `resource()`.
fn resources_page() -> AnyPiece {
    let (numbers_line, greeting_line) = resource_lines();
    column((
        label(tr("nav-resources"))
            .font(Font::Title)
            .id("resources-title"),
        label(tr("resources-caption")),
        // `image("day_logo")` resolves `images/day_logo.png` by name through the backend's native
        // image path (bundle file / Assets.car / R.drawable / …). `.frame` gives it a fixed box;
        // it scales to Fit (default content mode) — preserving aspect, never stretching.
        image("day_logo").frame(96.0, 96.0),
        label(move || numbers_line.clone()).id("resources-numbers"),
        label(move || greeting_line.clone()).id("resources-greeting"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Open two bundled data resources and format one random-access read from each. `numbers.bin` holds
/// the bytes `0..=255`, so `byte[100]` must be `100`; `greeting.txt` is a short UTF-8 string.
fn resource_lines() -> (String, String) {
    let numbers = match resource("numbers.bin") {
        Some(r) => {
            let mut b = [0u8; 1];
            r.read_at(100, &mut b);
            tr("resources-numbers")
                .arg("len", r.len() as f64)
                .arg("byte", b[0] as f64)
                .format()
        }
        None => "numbers.bin: (not bundled)".to_string(),
    };
    let greeting = match resource("greeting.txt") {
        Some(r) => tr("resources-greeting")
            .arg("text", String::from_utf8_lossy(r.as_slice()).into_owned())
            .format(),
        None => "greeting.txt: (not bundled)".to_string(),
    };
    (numbers, greeting)
}

fn activity_page() -> AnyPiece {
    // The spinner's running state is a Signal<bool> shared by the piece, the toggle, and a status
    // label that mirrors it reactively (each `tr(...)` branch is a full literal call for `day lint`).
    let spinning = Signal::new(true);
    let status = move || {
        if spinning.get() {
            tr("activity-on")
        } else {
            tr("activity-off")
        }
        .format()
    };
    column((
        label(tr("nav-activity"))
            .font(Font::Title)
            .id("activity-title"),
        label(tr("activity-caption")),
        row((
            activity().animating(spinning).id("activity-spinner"),
            label(status).id("activity-status"),
        ))
        .spacing(12.0),
        row((
            label(tr("activity-animating")),
            toggle(spinning).id("activity-toggle"),
        ))
        .spacing(8.0),
        divider(),
        // A separate, always-animating large spinner keeps a visible spinning indicator on screen
        // regardless of the toggle (nice for the walkthrough screenshot).
        label(tr("activity-large-label")).font(Font::Headline),
        activity().large(true).id("activity-large"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

fn search_page() -> AnyPiece {
    let query = Signal::new(String::new());
    column((
        label(tr("nav-search")).font(Font::Title).id("search-title"),
        label(tr("search-caption")),
        // A native search field bound two-way to `query` + a Clear button that sets it to ""
        // (proving the reverse binding patches the native control).
        row((
            search_field(query)
                .placeholder(tr("search-placeholder"))
                .id("search-input"),
            button(tr("search-clear"))
                .action(move || query.set(String::new()))
                .id("search-clear"),
        ))
        .spacing(8.0),
        // First match (a value, not prose) or an em-dash when nothing matches.
        label(move || search_first_match(&query.get())).id("search-result"),
        // The filtered fruit list — each row is a reactive `when`-gated label.
        column((
            search_fruit_row(query, "Apple"),
            search_fruit_row(query, "Banana"),
            search_fruit_row(query, "Cherry"),
            search_fruit_row(query, "Date"),
            search_fruit_row(query, "Elderberry"),
        ))
        .spacing(4.0)
        .align(HAlign::Leading),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

const SEARCH_FRUITS: [&str; 5] = ["Apple", "Banana", "Cherry", "Date", "Elderberry"];

/// Case-insensitive substring match; an empty query matches everything.
fn search_matches(query: &str, fruit: &str) -> bool {
    query.is_empty() || fruit.to_lowercase().contains(&query.to_lowercase())
}

/// The first fruit matching `query` (a data value), or an em-dash when none match.
fn search_first_match(query: &str) -> String {
    for fruit in SEARCH_FRUITS {
        if search_matches(query, fruit) {
            return fruit.to_string();
        }
    }
    "\u{2014}".to_string()
}

/// One filtered row: a `when`-gated label that appears only while its fruit matches the query.
fn search_fruit_row(query: Signal<String>, fruit: &'static str) -> AnyPiece {
    when(
        move || search_matches(&query.get(), fruit),
        move || label(fruit),
    )
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
/// A native map view (day-piece-map, an EXTERNAL standalone piece) — Apple platforms only. Preset
/// buttons recenter the map live via a bound coordinate `Signal` (a `Center` patch to the native
/// `MKMapView`). The map fills its `.frame`, and a marker pins the initial San Francisco center.
fn map_page() -> AnyPiece {
    const SF: (f64, f64) = (37.7749, -122.4194);
    const NYC: (f64, f64) = (40.7128, -74.0060);
    let center = Signal::new(SF);
    column((
        label(tr("nav-map")).font(Font::Title).id("map-title"),
        label(tr("map-caption")).id("map-caption"),
        row((
            button(tr("map-sf"))
                .action(move || center.set(SF))
                .id("map-sf"),
            button(tr("map-nyc"))
                .action(move || center.set(NYC))
                .id("map-nyc"),
        ))
        .spacing(8.0),
        label(move || {
            let (lat, lon) = center.get();
            format!("{lat:.4}, {lon:.4}")
        })
        .id("map-coords"),
        map()
            .center_signal(center)
            .span(0.05)
            .marker(SF.0, SF.1)
            .id("map")
            .grow(),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .grow()
}

/// The application menu bar (native NSMenu / GtkPopoverMenuBar / QMenuBar; app-bar overflow on Android;
/// UIMenuBuilder on iPadOS). Custom items carry keyboard shortcuts and update the shared `menu_log`;
/// the Edit menu uses standard roles so Cut/Copy/Paste target the focused control natively.
fn install_app_menu() {
    let log = |what: &'static str| move || menu_log().set(what.into());
    app_menu(vec![
        sub_menu(
            "File",
            vec![
                menu_item("New").key("n").action(log("File ▸ New")),
                menu_item("Open…").key("o").action(log("File ▸ Open")),
                // A nested submenu with keyboard shortcuts.
                sub_menu(
                    "Open Recent",
                    vec![
                        menu_item("report.pdf").action(log("Recent ▸ report.pdf")),
                        menu_item("budget.xlsx").action(log("Recent ▸ budget.xlsx")),
                        menu_separator(),
                        menu_item("Clear Menu").action(log("Recent ▸ Clear")),
                    ],
                ),
                menu_separator(),
                menu_item("Save").key("s").action(log("File ▸ Save")),
                menu_item("Save As…")
                    .shortcut(Shortcut::new("s").shift())
                    .action(log("File ▸ Save As")),
                menu_separator(),
                menu_role(MenuRole::CloseWindow),
                // Quit is a standard role: ⌘Q / Ctrl+Q, it exits the app and fires the
                // `WillTerminate` lifecycle phase (docs/lifecycle.md). macOS also keeps the
                // conventional Quit in the App menu.
                menu_role(MenuRole::Quit),
            ],
        ),
        // Standard edit commands — native items that target the focused control (default shortcuts).
        sub_menu(
            "Edit",
            vec![
                menu_role(MenuRole::Undo),
                menu_role(MenuRole::Redo),
                menu_separator(),
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
        sub_menu(
            "View",
            vec![
                menu_item("Reload").key("r").action(log("View ▸ Reload")),
                menu_item("Actual Size")
                    .key("0")
                    .action(log("View ▸ Actual Size")),
                menu_separator(),
                menu_role(MenuRole::Fullscreen),
            ],
        ),
    ]);
}

/// Menus playground: a context menu (secondary-click on desktop, long-press on mobile) with nested
/// submenus, standard roles, and shortcuts, plus a live readout of the last menu action fired — from
/// EITHER the app menu bar or this context menu. See docs/menus.md.
fn menus_page() -> AnyPiece {
    column((
        label(tr("nav-menus")).font(Font::Title).id("menus-title"),
        label(tr("menus-caption")).font(Font::Subheadline),
        // Live readouts: the last menu action (app menu or context menu), and the last app-lifecycle
        // phase (docs/lifecycle.md) — Quit fires WillTerminate; switching apps fires resign/active.
        column((
            label(move || format!("{}  {}", tr("menus-last").format(), menu_log().get()))
                .id("menus-last"),
            label(move || {
                format!(
                    "{}  {}",
                    tr("menus-lifecycle").format(),
                    lifecycle_log().get()
                )
            })
            .id("menus-lifecycle"),
        ))
        .spacing(6.0)
        .align(HAlign::Leading),
        divider(),
        label(tr("menus-context-hint")).font(Font::Headline),
        // A target for the context menu: nested submenu + a separator + a standard role.
        label(tr("menus-target"))
            .font(Font::Body)
            .padding(Insets::symmetric(20.0, 28.0))
            .id("menus-context-target")
            .context_menu(vec![
                menu_item("Rename").action(move || menu_log().set("Context ▸ Rename".into())),
                menu_item("Duplicate")
                    .key("d")
                    .action(move || menu_log().set("Context ▸ Duplicate".into())),
                menu_separator(),
                sub_menu(
                    "Move To",
                    vec![
                        menu_item("Inbox")
                            .action(move || menu_log().set("Context ▸ Move ▸ Inbox".into())),
                        menu_item("Archive")
                            .action(move || menu_log().set("Context ▸ Move ▸ Archive".into())),
                    ],
                ),
                menu_separator(),
                menu_role(MenuRole::Copy),
                menu_item("Delete")
                    .shortcut(Shortcut::plain("Delete"))
                    .action(move || menu_log().set("Context ▸ Delete".into())),
            ]),
        divider(),
        label(tr("menus-shortcut-hint")).font(Font::Footnote),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Typography playground: every semantic text style (mapped to the platform's native styles + Dynamic
/// Type / font-scale accessibility sizing), font weights, bold/italic, color, and accessibility-scaled
/// custom sizes. See docs/text.md.
fn text_page() -> AnyPiece {
    // A style name rendered IN its own style — a self-documenting type specimen.
    fn specimen(name: &'static str, f: Font) -> AnyPiece {
        label(name).font(f).id_keyed("text-style", name).any()
    }
    // Every semantic style (largest → smallest), each rendered in its own style.
    let styles = column((
        label(tr("text-styles-header")).font(Font::Headline),
        specimen("Large Title", Font::LargeTitle),
        specimen("Title", Font::Title),
        specimen("Title 2", Font::Title2),
        specimen("Title 3", Font::Title3),
        specimen("Headline", Font::Headline),
        specimen("Subheadline", Font::Subheadline),
        specimen("Body", Font::Body),
        specimen("Callout", Font::Callout),
        specimen("Footnote", Font::Footnote),
        specimen("Caption", Font::Caption),
        specimen("Caption 2", Font::Caption2),
    ))
    .spacing(6.0)
    .align(HAlign::Leading);
    // Font weights on a body-size line.
    let weights = column((
        label(tr("text-weights-header")).font(Font::Headline),
        label("Ultra Light")
            .weight(FontWeight::UltraLight)
            .id("text-w-ultralight"),
        label("Light").weight(FontWeight::Light),
        label("Regular").weight(FontWeight::Regular),
        label("Medium").weight(FontWeight::Medium),
        label("Semibold").weight(FontWeight::Semibold),
        label("Bold").weight(FontWeight::Bold).id("text-w-bold"),
        label("Heavy").weight(FontWeight::Heavy),
        label("Black").weight(FontWeight::Black),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Bold / italic / both, and everything-at-once.
    let styling = column((
        label(tr("text-styling-header")).font(Font::Headline),
        label("Bold text").bold().id("text-bold"),
        label("Italic text").italic().id("text-italic"),
        label("Bold italic").bold().italic().id("text-bolditalic"),
        label("Emphasis")
            .font(Font::Title2)
            .weight(FontWeight::Heavy)
            .italic()
            .color(Color::hex(0x8E44AD))
            .id("text-emphasis"),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Color.
    let colors = column((
        label(tr("text-colors-header")).font(Font::Headline),
        row((
            label("Red").color(Color::hex(0xE74C3C)),
            label("Green").color(Color::hex(0x27AE60)),
            label("Blue").color(Color::hex(0x2F6FDE)),
            label("Orange").color(Color::hex(0xE67E22)),
        ))
        .spacing(12.0),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Custom sizes — Font::System(pt), still scaled by the platform accessibility text size.
    let custom = column((
        label(tr("text-custom-header")).font(Font::Headline),
        label(tr("text-custom-note")).font(Font::Footnote),
        label("13 pt").font(Font::System(13.0)),
        label("20 pt").font(Font::System(20.0)),
        label("28 pt").font(Font::System(28.0)).id("text-custom-28"),
        label("40 pt")
            .font(Font::System(40.0))
            .weight(FontWeight::Bold),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);

    scroll(
        column((
            label(tr("nav-text"))
                .font(Font::LargeTitle)
                .id("text-title"),
            label(tr("text-caption")).font(Font::Subheadline),
            styles,
            divider(),
            weights,
            divider(),
            styling,
            divider(),
            colors,
            divider(),
            custom,
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}

/// A native Lottie animation (day-piece-lottie): a LottieAnimationView driven by airbnb's lottie-ios
/// (SwiftPM) / lottie-android (Gradle). Renders the bundled `hello.json`, looping. iOS + Android only.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn lottie_page() -> AnyPiece {
    // Playback rate, bound two ways: the slider drives it and `.speed(speed)` pushes it to the
    // native LottieAnimationView live (a `Speed` patch per change).
    let speed = Signal::new(1.0);
    column((
        label(tr("nav-lottie")).font(Font::Title).id("lottie-title"),
        label(tr("lottie-caption")).id("lottie-caption"),
        lottie("hello")
            .speed(speed)
            .frame(220.0, 220.0)
            .id("lottie-view"),
        // — speed slider with live readout (0.25×–3×) —
        row((
            label(tr("lottie-speed")),
            slider(speed)
                .range(0.25..=3.0)
                .step(0.25)
                .id("lottie-speed-slider"),
            label(move || format!("{:.2}×", speed.get())).id("lottie-speed-value"),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// A native recycling list (docs/list.md): 500 rows, but only the visible cells are ever built —
/// the platform's NSTableView / RecyclerView / GtkListView / QListView owns scrolling + reuse.
fn list_page() -> AnyPiece {
    let count = Signal::new(500i64);
    column((
        row((
            label(tr("nav-list")).font(Font::Title).id("list-title"),
            spacer(),
            button(tr("list-add"))
                .action(move || count.update(|c| *c += 100))
                .id("list-add"),
        )),
        label(tr("list-caption").arg("count", count)).id("list-caption"),
        list(
            move || {
                (1..=count.get())
                    .map(|i| format!("Row {i}"))
                    .collect::<Vec<_>>()
            },
            |s: &String| s.clone(),
            |row: ItemSlot<String, String>| {
                label(move || row.get())
                    .padding(Insets::symmetric(12.0, 8.0))
                    .id_keyed("list-row", row.key())
            },
        )
        .row_height(RowHeight::Uniform(36.0))
        .on_select(|k| println!("selected {k}"))
        .id("demo-list"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// The sidebar's header (logo + app name); the `selector` renders the item list below it as
/// the native source list / navigation-sidebar / bottom list.
fn sidebar_header() -> AnyPiece {
    row((
        image("day_logo").frame(28.0, 28.0),
        label(tr("app-title")).font(Font::Headline).id("home-title"),
    ))
    .spacing(8.0)
    .padding(12.0)
    .any()
}

/// Every interactive control, with stable ids for the walkthrough (§14).
fn controls_page() -> AnyPiece {
    let count = Signal::new(0i64);
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);
    let flavors = Signal::new(vec![
        "vanilla".to_string(),
        "chocolate".into(),
        "pistachio".into(),
    ]);
    let flavor = Signal::new(Some(0usize));

    scroll(
        column((
            label(tr("nav-controls"))
                .font(Font::Title)
                .id("controls-title"),
            // — state: counter —
            row((
                // The buttons log to the two standard streams (stderr / stdout) so
                // `day launch` can demonstrate forwarding both, per platform.
                button(tr("decrement"))
                    .action(move || {
                        count.update(|c| *c -= 1);
                        eprintln!("counter decremented to {}", count.get_untracked());
                    })
                    .id("decrement-button"),
                label(tr("counter-value").arg("count", count)).id("counter-label"),
                button(tr("increment"))
                    .action(move || {
                        count.update(|c| *c += 1);
                        println!("counter incremented to {}", count.get_untracked());
                    })
                    .id("increment-button"),
            ))
            .spacing(8.0),
            divider(),
            // — text input + conditional —
            text_field(name)
                .placeholder(tr("name-placeholder"))
                .id("name-field"),
            when(
                move || !name.with(|s| s.is_empty()),
                move || label(tr("greeting").arg("name", name)).id("greeting-label"),
            ),
            // — slider with live readout —
            row((
                label(tr("volume-label")),
                slider(volume).range(0.0..=100.0).id("volume-slider"),
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
            ))
            .spacing(8.0),
            // — a determinate progress bar tracking the slider live, and a spinner —
            row((
                label(tr("progress-label")),
                progress(move || volume.get() / 100.0)
                    .id("volume-progress")
                    .a11y(|a| a.role(Role::Meter).label("Volume level")),
            ))
            .spacing(8.0),
            row((label(tr("busy-label")), spinner().id("busy-spinner"))).spacing(8.0),
            toggle(subscribed)
                .id("subscribe-toggle")
                .a11y(|a| a.label("Subscribe to updates")), // a11y strings localize at M6.5
            // — an EXTERNAL Day Piece, registered like any built-in (§8.2, DP-21) —
            row((
                label(tr("flavor-label")),
                combo_box(flavors, flavor).id("flavor-combo"),
                label(move || {
                    let names = flavors.get();
                    flavor
                        .get()
                        .and_then(|i| names.get(i).cloned())
                        .unwrap_or_default()
                })
                .id("flavor-value"),
            ))
            .spacing(8.0),
            divider(),
            // — keyed collection (watch + monotonic keys, §5.4 / A.1) —
            history(count),
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}

/// Canvas gauge (§11) driven by its own slider.
fn gauge_page() -> AnyPiece {
    let level = Signal::new(40.0f64);
    column((
        row((
            label(tr("volume-label")),
            slider(level).range(0.0..=100.0).id("gauge-slider"),
        ))
        .spacing(8.0),
        gauge(level),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Shape pieces (docs/shapes.md): the unified `shape` piece rendered atop the canvas — every
/// kind, fill/stroke, a slider-bound rotation, tap-to-recolor, and drag-to-move. Shapes bind to
/// signals and transform through the canvas CTM, so all of this is free reactivity + zero backend
/// geometry code.
fn shapes_page() -> AnyPiece {
    let angle = Signal::new(0.0f64);
    let tapped = Signal::new(false);
    let pos = Signal::new((0.0f64, 0.0f64));
    let base = Signal::new((0.0f64, 0.0f64));
    column((
        label(tr("nav-shapes")).font(Font::Title).id("shapes-title"),
        // Every shape kind: fills and strokes (two rows so all fit the split-detail pane).
        label(tr("shapes-kinds")).font(Font::Headline),
        row((
            rectangle()
                .fill(Color::hex(0x2F6FDE))
                .frame(56.0, 56.0)
                .id("shape-rect"),
            rounded_rectangle(12.0)
                .fill(Color::hex(0x8E44AD))
                .frame(56.0, 56.0)
                .id("shape-rrect"),
            circle()
                .fill(Color::hex(0x27AE60))
                .frame(56.0, 56.0)
                .id("shape-circle"),
        ))
        .spacing(12.0),
        row((
            capsule()
                .fill(Color::hex(0xE67E22))
                .frame(76.0, 40.0)
                .id("shape-capsule"),
            ellipse()
                .stroke(Color::hex(0xC0392B), 4.0)
                .frame(76.0, 48.0)
                .id("shape-ellipse"),
            arc(135.0, 270.0)
                .stroke(Color::hex(0x16A085), 6.0)
                .frame(56.0, 56.0)
                .id("shape-arc"),
        ))
        .spacing(12.0),
        // A rounded rectangle rotated live by a slider (canvas CTM transform).
        label(tr("shapes-transform")).font(Font::Headline),
        row((
            label(tr("shapes-angle")),
            slider(angle).range(0.0..=360.0).id("shapes-angle-slider"),
        ))
        .spacing(8.0),
        rounded_rectangle(10.0)
            .fill(Color::hex(0x2F6FDE))
            .rotate(move || angle.get())
            // Inset so the rotated square's corners stay within the canvas frame (backends that
            // clip children to bounds — e.g. Qt — would otherwise shave the corners at an angle).
            .inset(20.0)
            .frame(120.0, 120.0)
            .id("shapes-rotator"),
        // Tap to recolor (path-precise hit-testing).
        label(tr("shapes-tap")).font(Font::Headline),
        circle()
            .fill(move || {
                if tapped.get() {
                    Color::hex(0xE74C3C)
                } else {
                    Color::hex(0x3498DB)
                }
            })
            .on_tap(move || tapped.update(|t| *t = !*t))
            // `.id` before `.frame` so the identifier lands on the shape leaf (the gesture target),
            // not the frame wrapper — lets dayscript/autodrive address the tap directly.
            .id("shapes-tap-circle")
            .frame(90.0, 90.0),
        // Drag to move (offset bound to the drag translation).
        label(tr("shapes-drag")).font(Font::Headline),
        rectangle()
            .fill(Color::hex(0x9B59B6))
            .offset(move || pos.get().0, move || pos.get().1)
            .on_drag(move |dr| match dr.phase {
                DragPhase::Began => base.set(pos.get_untracked()),
                _ => {
                    let b = base.get_untracked();
                    pos.set((b.0 + dr.translation.x, b.1 + dr.translation.y));
                }
            })
            .id("shapes-drag-rect")
            .frame(90.0, 90.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Picker pieces (docs/picker.md): one `picker` bound two-way to a `Signal<usize>`, in all three
/// SwiftUI-style stylings — each a distinct NATIVE control. A live label mirrors each selection.
fn pickers_page() -> AnyPiece {
    let size = Signal::new(1usize);
    let color = Signal::new(0usize);
    let plan = Signal::new(0usize);
    let sizes = ["Small", "Medium", "Large"];
    let colors = ["Red", "Green", "Blue"];
    let plans = ["Free", "Pro", "Team"];
    column((
        label(tr("nav-pickers"))
            .font(Font::Title)
            .id("pickers-title"),
        // Segmented — a horizontal one-of-N control.
        label(tr("picker-segmented")).font(Font::Headline),
        picker(sizes, size).segmented().id("picker-segmented"),
        label(move || sizes[size.get().min(2)].to_string()).id("picker-segmented-value"),
        // Menu — a pop-up / dropdown.
        label(tr("picker-menu")).font(Font::Headline),
        picker(colors, color).menu().id("picker-menu"),
        label(move || colors[color.get().min(2)].to_string()).id("picker-menu-value"),
        // Inline — a vertical radio group.
        label(tr("picker-inline")).font(Font::Headline),
        picker(plans, plan).inline().id("picker-inline"),
        label(move || plans[plan.get().min(2)].to_string()).id("picker-inline-value"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// The composition-first tier (DESIGN §8): every widget here is built PURELY from Day's core
/// primitives — NO native/per-backend code and NO cargo features. `day-piece-rating` is a plain
/// dependency with no per-backend feature wiring, so it works on every backend for free; native
/// pieces are the exception, not the rule.
fn compose_page() -> AnyPiece {
    // A shared rating signal, driven by tapping stars and read back by the value label.
    let stars = Signal::new(3usize);
    // A custom ambient value flowed via `with_environment` and read back by a descendant.
    #[derive(Clone, Copy)]
    struct Accent(Color);
    let accent = Color::hex(0x30_B0_60);

    column((
        label(tr("nav-compose"))
            .font(Font::Title)
            .id("compose-title"),
        label(tr("compose-caption")),
        // 1) Interactive star rating (canvas-polygon compose piece) + live value label.
        label(tr("compose-rating-label")).font(Font::Headline),
        rating(stars).id("compose-rating"),
        label(move || {
            tr("compose-rating-value")
                .arg("value", stars.get() as i64)
                .format()
        })
        .id("compose-rating-value"),
        // 2) Card modifier — a reusable surface wrapping arbitrary content.
        label(tr("compose-card-label")).font(Font::Headline),
        column((
            label(tr("compose-card-title")).font(Font::Headline),
            label(tr("compose-card-body")),
        ))
        .spacing(4.0)
        .align(HAlign::Leading)
        .modifier(Card),
        // 3) badge overlay — a numbered pill on an icon's top-trailing corner.
        label(tr("compose-badge-label")).font(Font::Headline),
        badge(
            3,
            rounded_rectangle(10.0)
                .fill(Color::hex(0x8E_8E_93))
                .frame(48.0, 48.0),
        ),
        // 4) ButtonStyle — a FilledButtonStyle button next to a plain one for contrast.
        label(tr("compose-buttons-label")).font(Font::Headline),
        row((
            button(tr("compose-plain-btn")).id("compose-plain-btn"),
            button(tr("compose-styled-btn"))
                .style(FilledButtonStyle {
                    color: Color::hex(0x0A_84_FF),
                })
                .id("compose-styled-btn"),
        ))
        .spacing(12.0),
        // 5) Ambient environment flow — a descendant tints itself from the provided Accent.
        label(tr("compose-env-label")).font(Font::Headline),
        with_environment(Accent(accent), || {
            let tint = environment::<Accent>().map(|a| a.0).unwrap_or(Color::BLACK);
            label(tr("compose-env-value"))
                .font(Font::Headline)
                .color(tint)
                .id("compose-env-value")
        }),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// A native web view (day-piece-webview, an EXTERNAL standalone piece): WKWebView / QWebEngineView /
/// android.webkit.WebView. The URL bar is bound two-way to the view — type + Go loads it, and
/// navigation reports the URL back so the field follows. Back/Forward/Stop/Reload drive history via
/// `Trigger`s the piece watches. The view fills the remaining space (a growing leaf).
fn webview_page() -> AnyPiece {
    let url = Signal::new("https://daybrite.dev".to_string());
    let go = Trigger::new();
    let back = Trigger::new();
    let forward = Trigger::new();
    let stop = Trigger::new();
    let reload = Trigger::new();
    column((
        label(tr("nav-webview"))
            .font(Font::Title)
            .id("webview-title"),
        // URL bar: the field is bound to the view's URL; Go loads whatever it holds.
        row((
            text_field(url)
                .placeholder(tr("webview-url-hint"))
                .id("webview-url"),
            button(tr("webview-go"))
                .action(move || go.notify())
                .id("webview-go"),
        ))
        .spacing(8.0),
        // History controls. "Stop" is the demo's cancel.
        row((
            button(tr("webview-back"))
                .action(move || back.notify())
                .id("webview-back"),
            button(tr("webview-forward"))
                .action(move || forward.notify())
                .id("webview-forward"),
            button(tr("webview-stop"))
                .action(move || stop.notify())
                .id("webview-stop"),
            button(tr("webview-reload"))
                .action(move || reload.notify())
                .id("webview-reload"),
        ))
        .spacing(8.0),
        web_view(url)
            .go(go)
            .back(back)
            .forward(forward)
            .stop(stop)
            .reload(reload)
            .id("webview"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Imperative modals (docs/dialogs.md): each button opens a native dialog from within an
/// async task and writes a fixed result token to `modal-result` (locale-independent so the
/// walkthrough can assert it).
fn modals_page() -> AnyPiece {
    let last = Signal::new(String::new());
    column((
        label(tr("nav-modals")).font(Font::Title).id("modals-title"),
        button(tr("modal-alert"))
            .action(move || {
                day::task(async move {
                    alert(tr("alert-title"))
                        .message(tr("alert-body"))
                        .button(tr("ok"), ())
                        .present()
                        .await;
                    last.set("alert-ok".into());
                })
            })
            .id("btn-alert"),
        button(tr("modal-confirm"))
            .action(move || {
                day::task(async move {
                    let ok = confirm(tr("confirm-title"))
                        .message(tr("confirm-body"))
                        .await;
                    last.set(if ok { "confirm-yes" } else { "confirm-no" }.into());
                })
            })
            .id("btn-confirm"),
        button(tr("modal-delete"))
            .action(move || {
                day::task(async move {
                    let ok = confirm(tr("delete-title"))
                        .message(tr("delete-body"))
                        .confirm_label(tr("delete"))
                        .destructive()
                        .await;
                    last.set(if ok { "delete-yes" } else { "delete-no" }.into());
                })
            })
            .id("btn-delete"),
        button(tr("modal-sheet"))
            .action(move || {
                day::task(async move {
                    let choice = Alert::new(tr("flavor-title"))
                        .sheet()
                        .button(tr("vanilla"), 0i64)
                        .button(tr("pistachio"), 1i64)
                        .cancel(tr("cancel"))
                        .present()
                        .await;
                    last.set(match choice {
                        Some(i) => format!("sheet-{i}"),
                        None => "sheet-cancel".into(),
                    });
                })
            })
            .id("btn-sheet"),
        button(tr("modal-prompt"))
            .action(move || {
                day::task(async move {
                    let name = prompt(tr("name-placeholder")).await;
                    last.set(match name {
                        Some(t) => format!("prompt-{t}"),
                        None => "prompt-none".into(),
                    });
                })
            })
            .id("btn-prompt"),
        divider(),
        label(move || last.get()).id("modal-result"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Native file open/save pickers (docs/files.md). Both buttons open a native picker from an async
/// task; the chosen file crosses back as a `FileUrl`. "Open" reads the file into the editor;
/// "Save" writes the editor's text out. Status tokens are locale-independent so the walkthrough
/// can assert them.
fn files_page() -> AnyPiece {
    // The editor text: what "Save" writes and what "Open" loads into.
    let content = Signal::new(String::from("Hello from Day!\nEdit me, then Save."));
    let status = Signal::new(String::new());
    let opened = Signal::new(String::new());
    column((
        label(tr("nav-files")).font(Font::Title).id("files-title"),
        label(tr("files-caption")),
        text_field(content)
            .placeholder(tr("files-placeholder"))
            .id("files-content"),
        row((
            button(tr("files-open"))
                .action(move || {
                    day::task(async move {
                        match open_file()
                            .title(tr("files-open"))
                            .filter("Text", &["txt", "md"])
                            .await
                        {
                            Some(file) => match file.read_to_string() {
                                Ok(text) => {
                                    content.set(text);
                                    opened.set(file.file_name().unwrap_or_default());
                                    status.set("opened".into());
                                }
                                Err(_) => status.set("open-error".into()),
                            },
                            None => status.set("open-cancel".into()),
                        }
                    })
                })
                .id("btn-open-file"),
            button(tr("files-save"))
                .action(move || {
                    day::task(async move {
                        let data = content.get_untracked().into_bytes();
                        match save_file(data)
                            .title(tr("files-save"))
                            .suggested_name("day-notes.txt")
                            .filter("Text", &["txt"])
                            .await
                        {
                            Some(dest) => status
                                .set(format!("saved:{}", dest.file_name().unwrap_or_default())),
                            None => status.set("save-cancel".into()),
                        }
                    })
                })
                .id("btn-save-file"),
        ))
        .spacing(8.0),
        divider(),
        when(
            move || !opened.with(|s| s.is_empty()),
            move || label(tr("files-opened").arg("name", opened)).id("files-opened-name"),
        ),
        label(move || status.get()).id("files-status"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Native tabbed container (docs/tabs.md): a `selector` with `SelectorStyle::Tabs`, bound to a
/// `Signal<String>` of the active tab key. NSTabView / UITabBarController / GtkNotebook /
/// QTabWidget / Android tab strip. Keys are routes, so deep links and dayscript select tabs.
fn tabs_page() -> AnyPiece {
    fn pane(title: LocalizedText, body: LocalizedText, content_id: &'static str) -> AnyPiece {
        column((label(title).font(Font::Title), label(body).id(content_id)))
            .spacing(10.0)
            .align(HAlign::Leading)
            .padding(16.0)
            .any()
    }
    let tab = Signal::new("one".to_string());
    selector(tab)
        .style(SelectorStyle::Tabs)
        .item("one", tr("tab-one"), || {
            pane(tr("tab-one"), tr("tab-one-body"), "tab-one-content")
        })
        .item("two", tr("tab-two"), || {
            pane(tr("tab-two"), tr("tab-two-body"), "tab-two-content")
        })
        .item("three", tr("tab-three"), || {
            pane(tr("tab-three"), tr("tab-three-body"), "tab-three-content")
        })
        .id("demo-tabs")
}

/// Genuine push/pop navigation (docs/navigation.md): `stack` bound to a `Signal<Vec<String>>`
/// path. Pushing a detail appends to the path; Day reconciles the native UINavigationController
/// / AdwNavigationView / back-stack; the native back button writes the pop back into the path.
fn stack_page() -> AnyPiece {
    fn push(path: Signal<Vec<String>>) {
        let mut v = path.get_untracked();
        let n = v.len() + 1;
        v.push(format!("{n}"));
        path.set(v);
    }
    let path = Signal::new(Vec::<String>::new());
    let root = column((
        label(tr("stack-root-body")).id("stack-root"),
        button(tr("stack-push"))
            .action(move || push(path))
            .id("stack-push"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0);
    stack(path, root)
        .destination(move |key| {
            let depth = key.to_string();
            column((
                label(tr("stack-detail-title").arg("depth", depth))
                    .font(Font::Title)
                    .id("stack-detail"),
                label(tr("stack-detail-body")),
                button(tr("stack-push"))
                    .action(move || push(path))
                    .id("stack-deeper"),
            ))
            .spacing(12.0)
            .align(HAlign::Leading)
            .padding(16.0)
        })
        .id("demo-stack")
}

fn about_page() -> AnyPiece {
    column((
        image("day_logo").frame(96.0, 96.0),
        label(tr("app-title")).font(Font::Headline),
        label(tr("about-text")).id("about-text"),
        // A HEADLESS capability crate (day-part-battery, docs/battery.md): app Rust calls
        // `day_part_battery::status()` directly — no UI Piece — and shows the platform's native reading.
        label(battery_line()).id("battery-line"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// The current battery reading as a localized line (Fluent; the state name stays the API's
/// enum debug form — it is a value, not prose).
fn battery_line() -> LocalizedText {
    match day_part_battery::status() {
        Some(b) => tr("battery-reading")
            .arg(
                "percent",
                b.percent()
                    .map(|p| format!("{p}%"))
                    .unwrap_or_else(|| "?".into()),
            )
            .arg("state", format!("{:?}", b.state)),
        None => tr("battery-reading-none"),
    }
}

fn gauge(value: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 {
            return;
        }
        let r = Rect::from_size(size).inset(8.0);
        let track = Color::rgba(0.5, 0.5, 0.55, 0.35);
        let accent = Color::hex(0x2F6FDE);
        d.stroke(
            Shape::Arc {
                rect: r,
                start_deg: 135.0,
                sweep_deg: 270.0,
            },
            track,
            6.0,
        );
        let frac = (value.get() / 100.0).clamp(0.0, 1.0);
        if frac > 0.0 {
            d.stroke(
                Shape::Arc {
                    rect: r,
                    start_deg: 135.0,
                    sweep_deg: 270.0 * frac,
                },
                accent,
                6.0,
            );
        }
        d.text(
            &format!("{:.0}", value.get()),
            Point::new(size.width / 2.0, size.height / 2.0),
            TextStyle {
                size: 22.0,
                color: accent,
                anchor: TextAnchor::Centered,
            },
        );
    })
    // Accessibility (§13): a canvas has no inherent role, so Day applies `Meter` + a spoken value
    // and label. `.id`/`.a11y` go on the canvas leaf (before `.frame`, a handle-less layout node),
    // so they reach the native widget. Value is a build-time snapshot (reactive a11y is a follow-up).
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(tr("volume-label").format())
            .value(format!("{:.0}", value.get_untracked()))
    })
    .id("gauge")
    .frame(110.0, 110.0)
}

/// Battery playground (docs/battery.md): the headless `day-part-battery` part feeds a canvas-drawn
/// battery visualization — level fill colored by charge band, a bolt when charging. The preview
/// slider + toggle drive arbitrary states; "Read Device Battery" snaps back to the real reading.
fn battery_page() -> AnyPiece {
    // Seed the preview signals from the device's real reading (a demo value when there's none).
    let status = day_part_battery::status();
    let level = Signal::new(
        status
            .and_then(|b| b.percent())
            .map(f64::from)
            .unwrap_or(80.0),
    );
    let charging = Signal::new(status.map(|b| b.is_charging()).unwrap_or(false));
    let reading = Signal::new(battery_line().format());
    column((
        label(tr("nav-battery"))
            .font(Font::Title)
            .id("battery-title"),
        label(tr("battery-caption")),
        battery_view(level, charging),
        row((
            button(tr("battery-refresh"))
                .action(move || {
                    reading.set(battery_line().format());
                    if let Some(b) = day_part_battery::status() {
                        if let Some(p) = b.percent() {
                            level.set(f64::from(p));
                        }
                        charging.set(b.is_charging());
                    }
                })
                .id("battery-refresh"),
            label(move || reading.get()).id("battery-reading"),
        ))
        .spacing(8.0),
        divider(),
        // Preview controls: explore the visualization at any level / charge state.
        label(tr("battery-preview")).font(Font::Headline),
        row((
            label(tr("battery-level")),
            slider(level).range(0.0..=100.0).id("battery-level"),
            label(move || format!("{:.0}%", level.get())).id("battery-level-value"),
        ))
        .spacing(8.0),
        row((
            label(tr("battery-charging")),
            toggle(charging).id("battery-charging"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Draw a battery on a canvas: rounded body + terminal nub, a level fill colored by band
/// (red < 20% ≤ amber < 50% ≤ green), a lightning bolt when charging, and a percent caption.
fn battery_view(level: Signal<f64>, charging: Signal<bool>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let frac = (level.get() / 100.0).clamp(0.0, 1.0);
        let band = if frac < 0.2 {
            Color::hex(0xFF3B30) // red
        } else if frac < 0.5 {
            Color::hex(0xFF9F0A) // amber
        } else {
            Color::hex(0x34C759) // green
        };
        let outline = Color::rgba(0.55, 0.55, 0.6, 0.9);

        // Geometry: the body fills the canvas minus the nub on the right and a caption strip below.
        let caption_h = 26.0;
        let nub_w = (size.width * 0.05).clamp(6.0, 14.0);
        let body = Rect::new(
            2.0,
            2.0,
            size.width - nub_w - 6.0,
            size.height - caption_h - 4.0,
        );
        let nub_h = body.size.height * 0.4;
        let nub = Rect::new(
            body.max_x() + 2.0,
            body.center().y - nub_h / 2.0,
            nub_w,
            nub_h,
        );
        d.stroke(Shape::RoundedRect(body, 12.0), outline, 3.0);
        d.fill(Shape::RoundedRect(nub, 3.0), outline);

        // The charge fill, inset within the body and clipped to the level fraction.
        let well = body.inset(6.0);
        let fill_w = well.size.width * frac;
        if fill_w > 0.5 {
            let fill_rect = Rect::new(well.min_x(), well.min_y(), fill_w, well.size.height);
            d.fill(
                Shape::RoundedRect(fill_rect, 7.0_f64.min(fill_w / 2.0)),
                band,
            );
        }

        // Charging: a lightning bolt centered in the body (white with a dark edge, so it reads on
        // both the colored fill and the empty well).
        if charging.get() {
            let c = body.center();
            let (bw, bh) = (body.size.height * 0.42, body.size.height * 0.72);
            let p =
                |rx: f64, ry: f64| Point::new(c.x - bw / 2.0 + rx * bw, c.y - bh / 2.0 + ry * bh);
            let bolt = vec![
                p(0.62, 0.0),
                p(0.0, 0.58),
                p(0.42, 0.58),
                p(0.38, 1.0),
                p(1.0, 0.42),
                p(0.58, 0.42),
            ];
            d.fill(
                Shape::Polygon(bolt.clone()),
                Color::rgba(1.0, 1.0, 1.0, 0.95),
            );
            d.stroke(Shape::Polygon(bolt), Color::rgba(0.0, 0.0, 0.0, 0.35), 1.5);
        }

        // Percent caption below the battery, in the band color.
        d.text(
            &format!("{:.0}%", level.get()),
            Point::new(size.width / 2.0, size.height - caption_h / 2.0),
            TextStyle {
                size: 16.0,
                color: band,
                anchor: TextAnchor::Centered,
            },
        );
    })
    // Accessibility (§13): like the gauge, the canvas gets an explicit Meter role + spoken
    // label/value (value is a build-time snapshot; reactive a11y is a follow-up).
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(tr("nav-battery").format())
            .value(format!("{:.0}%", level.get_untracked()))
    })
    .id("battery")
    .frame(260.0, 120.0)
}

/// Sensors playground (docs/sensors.md): the headless `day-part-sensors` part polls the device's
/// motion sensors natively. Sensors are push-model on Android/OHOS, so the first read arms the
/// listener — Refresh twice on a fresh launch; desktops/simulators report "unavailable".
fn sensors_page() -> AnyPiece {
    use day_part_sensors::SensorKind;
    fn sensor_line(kind: SensorKind, unit: &str) -> String {
        match day_part_sensors::read(kind) {
            Some(r) => tr("sensor-reading")
                .arg("x", format!("{:+.2}", r.x))
                .arg("y", format!("{:+.2}", r.y))
                .arg("z", format!("{:+.2}", r.z))
                .arg("unit", unit)
                .format(),
            None if day_part_sensors::is_available(kind) => tr("sensor-waiting").format(),
            None => tr("sensor-unavailable").format(),
        }
    }
    let accel = Signal::new(sensor_line(SensorKind::Accelerometer, "m/s²"));
    let gyro = Signal::new(sensor_line(SensorKind::Gyroscope, "rad/s"));
    let magnet = Signal::new(sensor_line(SensorKind::Magnetometer, "µT"));
    column((
        label(tr("nav-sensors"))
            .font(Font::Title)
            .id("sensors-title"),
        label(tr("sensors-caption")),
        row((
            label(tr("sensor-accelerometer")),
            label(move || accel.get()).id("sensor-accel"),
        ))
        .spacing(8.0),
        row((
            label(tr("sensor-gyroscope")),
            label(move || gyro.get()).id("sensor-gyro"),
        ))
        .spacing(8.0),
        row((
            label(tr("sensor-magnetometer")),
            label(move || magnet.get()).id("sensor-magnet"),
        ))
        .spacing(8.0),
        button(tr("sensors-refresh"))
            .action(move || {
                accel.set(sensor_line(SensorKind::Accelerometer, "m/s²"));
                gyro.set(sensor_line(SensorKind::Gyroscope, "rad/s"));
                magnet.set(sensor_line(SensorKind::Magnetometer, "µT"));
            })
            .id("sensors-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Clipboard playground (docs/clipboard.md): the headless `day-part-clipboard` part round-trips
/// plain text through the system clipboard — type, Copy, then Paste reads it back natively.
fn clipboard_page() -> AnyPiece {
    let draft = Signal::new(String::new());
    let pasted = Signal::new(String::new());
    let status = Signal::new(tr("clipboard-idle").format());
    column((
        label(tr("nav-clipboard"))
            .font(Font::Title)
            .id("clipboard-title"),
        label(tr("clipboard-caption")),
        text_field(draft)
            .placeholder(tr("clipboard-placeholder"))
            .id("clipboard-field"),
        row((
            button(tr("clipboard-copy"))
                .action(move || {
                    let ok = draft.with(|t| day_part_clipboard::set_text(t));
                    let msg = if ok {
                        tr("clipboard-copied")
                    } else {
                        tr("clipboard-copy-failed")
                    };
                    status.set(msg.format());
                })
                .id("clipboard-copy"),
            button(tr("clipboard-paste"))
                .action(move || match day_part_clipboard::get_text() {
                    Some(text) => {
                        pasted.set(text);
                        status.set(tr("clipboard-pasted").format());
                    }
                    None => status.set(tr("clipboard-empty").format()),
                })
                .id("clipboard-paste"),
        ))
        .spacing(8.0),
        label(move || status.get()).id("clipboard-status"),
        label(move || pasted.get()).id("clipboard-pasted"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Network playground (docs/network.md): the headless `day-part-network` part feeds an
/// online/offline readout with the connection kind; "Read Network" re-polls the snapshot.
fn network_page() -> AnyPiece {
    let reading = Signal::new(network_line().format());
    column((
        label(tr("nav-network"))
            .font(Font::Title)
            .id("network-title"),
        label(tr("network-caption")),
        row((
            button(tr("network-refresh"))
                .action(move || reading.set(network_line().format()))
                .id("network-refresh"),
            label(move || reading.get()).id("network-reading"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// The current connectivity snapshot as a localized line (Fluent; kind stays the API's enum
/// debug form — it is a value, not prose).
fn network_line() -> LocalizedText {
    match day_part_network::status() {
        Some(n) => {
            let line = if n.online {
                tr("network-reading-online")
            } else {
                tr("network-reading-offline")
            };
            line.arg("kind", format!("{:?}", n.kind)).arg(
                "expensive",
                match n.expensive {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "?",
                },
            )
        }
        None => tr("network-reading-none"),
    }
}

/// A native media player (day-piece-media, an EXTERNAL standalone piece): AVPlayerView /
/// AVPlayerViewController / QMediaPlayer+QVideoWidget / android.widget.VideoView / GtkVideo.
/// Transport is imperative via `Trigger`s the piece watches; native chrome (where the toolkit
/// has one) offers its own controls too. The player fills the remaining space (a growing leaf).
fn media_page() -> AnyPiece {
    let url = Signal::new(
        "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4".to_string(),
    );
    let play = Trigger::new();
    let pause = Trigger::new();
    let load = Trigger::new();
    column((
        label(tr("nav-media")).font(Font::Title).id("media-title"),
        row((
            button(tr("media-play"))
                .action(move || play.notify())
                .id("media-play"),
            button(tr("media-pause"))
                .action(move || pause.notify())
                .id("media-pause"),
            button(tr("media-load"))
                .action(move || load.notify())
                .id("media-load"),
        ))
        .spacing(8.0),
        // muted: CI walkthroughs screenshot this page — don't blast audio on runners.
        media(url)
            .looping(true)
            .muted(true)
            .play(play)
            .pause(pause)
            .load(load)
            .id("media"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

fn history(count: Signal<i64>) -> AnyPiece {
    let entries = Signal::new(Vec::<(u64, i64)>::new());
    let next_id = Signal::new(0u64);
    watch(
        move || count.get(),
        move |new, _old| {
            let id = next_id.get_untracked();
            next_id.set(id + 1);
            let v = *new;
            entries.update(|e| {
                e.push((id, v));
                if e.len() > 8 {
                    e.remove(0);
                }
            });
        },
    );
    column((
        label(tr("history-title")).font(Font::Headline),
        each(
            move || entries.get(),
            |e| e.0,
            move |slot: ItemSlot<(u64, i64), u64>| {
                label(move || {
                    tr("history-entry")
                        .arg("value", slot.field(|t| t.1))
                        .format()
                })
            },
        ),
    ))
    .spacing(4.0)
    .align(HAlign::Leading)
    .any()
}

// Mobile / embedded entries (DESIGN.md §17.4): the iOS Runner binds `day_main`, DayBridge binds the
// `Java_…` natives, and the HarmonyOS ArkTS host binds `day_arkui_start`. Every macro emits nothing
// off its own target.
day::ios_main!("Day Showcase", root);
day::android_main!(root);
day::arkui_main!(root);
