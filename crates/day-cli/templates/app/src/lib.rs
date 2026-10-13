//! A small Day app: shared state, adaptive navigation, and native controls.

use day::prelude::*;

mod model;
mod pages;

use model::Scene;

// Desktop enters through main.rs; mobile hosts use this entry point.
// Build pipeline: https://daybrite.dev/docs/project-structure/
day::day_start!(options: window(), root);
// Generate typed accessors for resource/: https://daybrite.dev/docs/resources/
day::resources!();

const THEME_KEY: &str = "app.theme";
const LOCALE_KEY: &str = "app.locale";

day::routes! {
    /// Typed navigation keys also supply the routes used by links and dayscript.
    pub(crate) enum Section {
        Welcome => "welcome",
        Items => "items",
        Settings => "settings",
    }
}

/// Configure the native window and locale catalog; the title is formatted in the active locale.
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        locales: Some((res::locales::DEFAULT, res::locales::CATALOG)),
        title_fn: Some(|| res::str::app_title().format()),
        size: Size::new(960.0, 640.0),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..Default::default()
    }
}

/// Choose Settings placement by toolkit capability, keeping platform checks out of the pages.
pub(crate) fn has_menu_bar() -> bool {
    capability(Cap::AppMenu) != Support::Unsupported
}

/// Provide one window's shared state and compose its pages: https://daybrite.dev/docs/internal/state/
pub fn root() -> impl Piece {
    // Connect `.restore` below to preferences so the last section survives a relaunch.
    day::prefs::install_nav_store();
    day_piece_settings::apply_startup(THEME_KEY, LOCALE_KEY);
    day::register_preferences(pages::settings::settings_body);

    Scene::scoped(|scene| {
        scene.persist();
        app_menu(menus(scene));

        nav(scene.section)
            .title(res::str::app_title())
            // The list and editor sit side by side on desktop and stack on a phone.
            // Navigation adapts the same pieces: https://daybrite.dev/docs/navigation/
            .content_list(pages::items::item_list_pane)
            .content_list_width(320.0)
            .content_list_for(|s: &Section| matches!(s, Section::Items))
            .detail_visible(scene.detail_open)
            .detail_title(move || pages::items::detail_title(scene))
            .item_icon(
                Section::Welcome,
                res::str::nav_welcome(),
                res::vectors::tab_welcome,
                pages::welcome::welcome_page,
            )
            .item_icon(
                Section::Items,
                res::str::nav_items(),
                res::vectors::tab_items,
                pages::items::items_page,
            )
            // Use the desktop's conventional Settings menu/window; without a menu bar,
            // keep Settings reachable as a navigation section.
            .items(
                || {
                    if has_menu_bar() {
                        Vec::new()
                    } else {
                        vec![Section::Settings]
                    }
                },
                |s: &Section| item(*s, res::str::nav_settings()).icon(res::vectors::tab_settings),
            )
            .destination(|_: &Section| pages::settings::settings_page())
            .restore("app.section")
            .id("nav")
    })
}

/// Share model actions with toolbars; menu roles provide native editing commands.
/// Menus and preferences: https://daybrite.dev/docs/guide-desktop/
fn menus(scene: Scene) -> Vec<MenuEntry> {
    vec![
        sub_menu(
            res::str::menu_file().format(),
            vec![
                menu_item(res::str::cmd_add().format())
                    .shortcut(Shortcut::new("n"))
                    .action(move || scene.new_item()),
                menu_role(MenuRole::CloseWindow),
            ],
        ),
        sub_menu(
            res::str::menu_edit().format(),
            vec![
                menu_item(res::str::cmd_delete().format())
                    .shortcut(Shortcut::new("Delete"))
                    .action(move || scene.delete_selected()),
                menu_separator(),
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
    ]
}
