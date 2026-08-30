//! {{title}}, a [Day](https://daybrite.dev) app. `root()` is the whole UI, shared by every
//! platform; each navigation destination lives in its own module under `pages/`.

use day::prelude::*;

mod model;
mod pages;
use crate::pages::*;

// The mobile / embedded entry point; a plain cargo desktop build enters through src/main.rs.
day::day_start!(options: window(), root);

/// The window every entry point opens. The locale catalog and title are handed to `launch`
/// rather than installed here (https://daybrite.dev/docs/localization).
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        locales: Some((res::locales::DEFAULT, res::locales::CATALOG)),
        title_fn: Some(|| res::str::app_title().format()),
        // A desktop default; mobile fills the screen regardless.
        size: day::prelude::Size::new(960.0, 640.0),
        ..Default::default()
    }
}

// Typed constants for everything under `resource/` (https://daybrite.dev/docs/resources).
day::resources!();

/// The two settings' `day::prefs` keys: read at startup, written by the Settings page.
const THEME_KEY: &str = "app.theme";
const LOCALE_KEY: &str = "app.locale";

day::routes! {
    /// The app's sections, typed (https://daybrite.dev/docs/navigation).
    pub(crate) enum Section {
        Welcome => "welcome",
        Navigate => "navigate",
        Settings => "settings",
    }
}

/// Whether this platform has a menu bar; there, Settings lives in the App menu instead of the
/// navigation (https://daybrite.dev/docs/menus).
pub(crate) fn has_menu_bar() -> bool {
    capability(Cap::AppMenu) != Support::Unsupported
}

pub fn root() -> impl Piece {
    // `info!` and friends need no setup: Day installs a logger at launch.
    info!("{{title}} starting");
    // Re-apply the saved theme and language before anything is built.
    day_piece_settings::apply_startup(THEME_KEY, LOCALE_KEY);
    model::load();

    // A real Settings window plus the App ▸ Settings… item on desktop; a fullscreen cover
    // where windows are unsupported (https://daybrite.dev/docs/windows).
    day::register_preferences(settings_body);
    app_menu(menus());
    // Desktop-only by nature; the phones carry the same commands as list actions below.
    toolbar_reactive(|| {
        vec![
            toolbar_sidebar_toggle("tb-sidebar", res::str::cmd_sidebar()),
            toolbar_flexible_space(),
            // Bound to the list's own filter signal, so button, menu, and list always agree.
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

    // A selector is adaptive by default: tabs on a phone, a rail on a tablet, a sidebar on a
    // desktop (https://daybrite.dev/docs/navigation).
    let section = Signal::new(Section::Welcome);
    selector(section)
        .title(res::str::app_title())
        // The item list as a real content-list pane: its own column where the toolkit has one,
        // the pushed middle layer on a phone (https://daybrite.dev/docs/navigation).
        .content_list(item_list_pane)
        .content_list_width(320.0)
        // Only Navigate has a list; Welcome and Settings keep the whole detail area.
        .content_list_for(|s: &Section| matches!(s, Section::Navigate))
        // Whether the editor is up, on the shapes that show one pane at a time.
        .detail_visible(pages::detail_open())
        // The pushed editor's bar names the item it shows, live.
        .detail_title(pages::detail_title)
        // List commands: these ride the list pane's navigation bar on the phones
        // (https://daybrite.dev/docs/toolbars).
        .list_action(res::vectors::filter, res::str::cmd_show_done(), || {
            crate::model::show_done().update(|v| *v = !*v)
        })
        .list_action(
            res::vectors::check,
            res::str::cmd_done(),
            pages::done_selected,
        )
        .list_action(res::vectors::add, res::str::cmd_add(), pages::new_item)
        .item_icon(
            Section::Welcome,
            res::str::nav_welcome(),
            res::vectors::tab_welcome,
            welcome_page,
        )
        // One tint per section, so the icons read apart at a glance.
        .icon_tint(Color::hex(0xF59E0B))
        .item_icon(
            Section::Navigate,
            res::str::nav_navigate(),
            res::vectors::tab_navigate,
            navigate_page,
        )
        .icon_tint(Color::hex(0x3B82F6))
        // Settings is a row only where there is no menu bar (see `has_menu_bar`).
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

/// The desktop menu bar; mobile toolkits ignore it (https://daybrite.dev/docs/menus).
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
        // The desktop counterpart to the list's swipe actions (https://daybrite.dev/docs/list).
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
