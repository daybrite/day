//! {{title}} — a [Day](https://daybrite.dev) app. `root()` is the whole UI, shared by every
//! platform; each navigation destination lives in its own module under `pages/`.

use day::prelude::*;

mod model;
mod pages;
use crate::pages::*;

// The mobile / embedded entry point. Expands to the export each platform's shell binds against —
// and to nothing at all on a plain cargo desktop build, where src/main.rs is the entry instead.
day::day_start!(window_title(), root);

/// The window (and web document) title, from the locale catalog rather than a literal — so it
/// is translated with every other user-facing string and the app's name lives in ONE place.
///
/// The catalog is installed HERE as well as in [`root`] on purpose: the entry points above
/// evaluate this title before `root` runs, and a lookup with no catalog installed renders the
/// key itself — `⟨app_title⟩` in the title bar. Re-registering is cheap and keeps the locale
/// signal bindings already created (https://daybrite.dev/docs/localization).
pub fn window_title() -> String {
    res::locales::install();
    res::str::app_title().format()
}

/// Typed constants for the files under `resource/`, generated at build time by `day-build` (§18.5):
/// `res::images::<stem>`, `res::assets::<file>`, `res::fonts::<family>`, `res::str::<key>()`, and
/// the `res::locales` catalog. Reference bundled resources
/// through these — `image(res::images::app_logo)` — so a typo is a compile error and the resource is
/// guaranteed present. Drop a file into `resource/images/` and its constant appears on the next build.
pub mod res {
    include!(concat!(env!("OUT_DIR"), "/day_resources.rs"));
}

/// Where the two settings live in `day::prefs`. Named once here because startup reads them
/// before the UI exists and the Settings page writes them afterwards.
const THEME_KEY: &str = "app.theme";
const LOCALE_KEY: &str = "app.locale";

day::routes! {
    /// The app's sections, typed (https://daybrite.dev/docs/navigation): each variant's key is
    /// what deep links, dayscript, and `current_route()` speak, and the `.item(Section::…)`
    /// declarations below are compile-checked against this enum.
    pub(crate) enum Section {
        Welcome => "welcome",
        Navigate => "navigate",
        Settings => "settings",
    }
}

/// `true` on a platform with an application MENU BAR — where `register_preferences` puts a
/// Settings… item automatically, so the navigation should not carry one as well.
///
/// A question about the PLATFORM, not the window, which is why it asks a capability rather than a
/// size class: keying it off the width would put Settings back in the navigation the moment
/// someone made a desktop window narrow. And `Cap::AppMenu` rather than `Cap::Toolbar` — the web
/// draws a window toolbar but has no menu bar, so the toolbar would strand Settings with nowhere
/// to reach it (https://daybrite.dev/docs/menus).
pub(crate) fn has_menu_bar() -> bool {
    capability(Cap::AppMenu) != Support::Unsupported
}

pub fn root() -> impl Piece {
    // Logging (docs/logging.md). `info!`/`warn!`/`error!`/`debug!`/`trace!` come from the prelude
    // and need no setup: Day installs a logger at launch, so this line reaches the terminal on the
    // desktop, logcat on Android, the Xcode console on Apple, and the browser's JS console on the
    // web — where a plain `println!` would be silently dropped. Raise the level with
    // `DAY_LOG=debug day launch -p …`, or install `env_logger`/`tracing` before `day::launch` and
    // Day steps aside.
    info!("{{title}} starting");
    // Registers every locale under `resource/locales/` (generated, §18.5). To add a language,
    // copy `resource/locales/en/` to e.g. `resource/locales/fr/` and translate it — this line
    // already covers it.
    res::locales::install();
    // Re-apply the saved theme and language BEFORE anything is built, so the first frame is
    // already in the user's choices rather than flashing the defaults.
    day_piece_settings::apply_startup(THEME_KEY, LOCALE_KEY);
    model::load();

    // Desktop gets a real Settings WINDOW rather than a section: this one call gives the
    // App ▸ Settings… item, its ⌘, shortcut, and a singleton window — and on a toolkit that
    // cannot open windows it falls back to a fullscreen cover, so the same call is right
    // everywhere (https://daybrite.dev/docs/windows).
    day::register_preferences(settings_body);
    app_menu(menus());
    // The window toolbar is desktop-only by nature; the phones carry the same command as a
    // navigation bar action instead (see `navigate_page`).
    toolbar_reactive(|| {
        vec![
            toolbar_sidebar_toggle("tb-sidebar", res::str::cmd_sidebar()),
            toolbar_flexible_space(),
            // Reflects the LIST's filter, which is a real signal — so the button, the menu item
            // and the list can never disagree about whether finished items are showing.
            toolbar_toggle(
                "tb-show-done",
                res::str::cmd_show_done(),
                crate::model::show_done(),
            )
            .icon(Symbol::Filter)
            .tooltip(res::str::cmd_show_done()),
            toolbar_button("tb-done", res::str::cmd_done())
                .icon(Symbol::Check)
                .tooltip(res::str::cmd_done())
                .action(pages::done_selected),
            toolbar_button("tb-add", res::str::cmd_add())
                .icon(Symbol::Add)
                .tooltip(res::str::cmd_add())
                .action(pages::new_item),
        ]
    });

    // `Sidebar`, not the adaptive default: the item list below is a CONTENT-LIST pane, and a
    // pane needs a presentation with a column to put it in. The sidebar family has one at every
    // width — three columns on a desktop, and on a phone the same three as a push sequence
    // (sections → list → editor). A tab bar has nowhere to place it
    // (https://daybrite.dev/docs/navigation).
    let section = Signal::new(Section::Welcome);
    selector(section)
        .style(SelectorStyle::Sidebar)
        .title(res::str::app_title())
        // The item list is a real CONTENT-LIST pane (https://daybrite.dev/docs/navigation): its
        // own column between the sidebar and the editor where the toolkit has one — a
        // `contentList` split item on macOS, the supplementary column on iPadOS — the pushed
        // middle layer on a phone, and composed beside the editor everywhere else. That is what
        // makes this a TRUE three-column window rather than two panes drawn inside one, and it
        // is why neither this file nor `navigate.rs` branches on the window width any more.
        .content_list(item_list_pane)
        .content_list_width(320.0)
        // Only the section that HAS a list gets the middle column; Welcome and Settings keep the
        // whole detail area.
        .content_list_for(|s: &Section| matches!(s, Section::Navigate))
        // On the shapes that show one pane at a time, this is what says whether the editor is up:
        // the host pushes it when the signal goes true and pops back to the list when it clears.
        .detail_visible(pages::detail_open())
        // These three act on the LIST, so they ride the list pane's own navigation bar on the
        // phones — where a window toolbar does not exist — and the desktop split ignores them
        // because the toolbar above already carries the same commands
        // (https://daybrite.dev/docs/toolbars).
        .list_action(res::vectors::filter, res::str::cmd_show_done(), || {
            crate::model::show_done().update(|v| *v = !*v)
        })
        .list_action(res::vectors::check, res::str::cmd_done(), pages::done_selected)
        .list_action(res::vectors::add, res::str::cmd_add(), pages::new_item)
        .item_icon(
            Section::Welcome,
            res::str::nav_welcome(),
            res::vectors::tab_welcome,
            welcome_page,
        )
        // A tint per section, so the icons read apart at a glance — the one place in this app
        // where color carries meaning rather than decoration (https://daybrite.dev/docs/vectors).
        .icon_tint(Color::hex(0xF59E0B))
        .item_icon(
            Section::Navigate,
            res::str::nav_navigate(),
            res::vectors::tab_navigate,
            navigate_page,
        )
        .icon_tint(Color::hex(0x3B82F6))
        // Settings is a ROW only on platforms with nowhere better to put it. Where there is a
        // menu bar it lives in the App menu — that is where those users look — and a narrow
        // desktop window must not move it back into the navigation, which is why this asks a
        // capability rather than the window's width.
        .items(
            move || {
                if has_menu_bar() {
                    Vec::new()
                } else {
                    vec![Section::Settings]
                }
            },
            |s: &Section| {
                item(*s, res::str::nav_settings())
                    .icon(res::vectors::tab_settings)
                    .icon_tint(Color::hex(0x10B981))
            },
        )
        .destination(|_: &Section| settings_page())
        .restore("app.section")
        .id("nav")
}

/// The desktop menu bar. Mobile toolkits have no menu bar and ignore it, so this is written once
/// rather than behind a platform check (https://daybrite.dev/docs/menus).
fn menus() -> Vec<MenuEntry> {
    vec![
        sub_menu(
            res::str::menu_file().format(),
            vec![
                menu_item(res::str::cmd_add().format())
                    .shortcut(Shortcut::new("n"))
                    .action(pages::new_item),
                menu_separator(),
                menu_role(MenuRole::CloseWindow),
            ],
        ),
        // Editing the list from the menu bar is the desktop counterpart to a swipe: the phones
        // get swipe-to-delete from the list itself, and no desktop toolkit has a swipe idiom
        // (https://daybrite.dev/docs/list).
        sub_menu(
            res::str::menu_edit().format(),
            vec![
                menu_item(res::str::cmd_delete().format())
                    .shortcut(Shortcut::new("Delete"))
                    .action(pages::delete_selected),
                menu_item(res::str::cmd_done().format())
                    .shortcut(Shortcut::new("d"))
                    .action(pages::done_selected),
                menu_item(res::str::cmd_show_done().format())
                    .shortcut(Shortcut::new("h"))
                    .action(|| crate::model::show_done().update(|v| *v = !*v)),
                menu_separator(),
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
    ]
}
