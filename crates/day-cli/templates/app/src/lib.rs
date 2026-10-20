//! A small Day app: shared state, adaptive navigation, and native controls.

use day::prelude::*;
use day_piece_settings::prelude::*;

mod model;
mod pages;

use model::Scene;

// Desktop enters through main.rs; mobile hosts use this entry point.
// Build pipeline: https://daybrite.dev/docs/project-structure/
day::day_start!(options: window(), root);
// Generate typed accessors for resource/: https://daybrite.dev/docs/resources/
day::resources!();

day::routes! {
    /// Typed navigation keys also supply the routes used by links and dayscript.
    pub(crate) enum Section {
        Welcome => "welcome",
        Items => "items",
        Settings => "settings",
    }
}

/// The window: the generated options carry the locale catalog, the localized title and the
/// version; the size is for desktop (phones fill the screen).
pub fn window() -> day::WindowOptions {
    res::window_options().size(960.0, 640.0)
}

/// Provide one window's shared state and compose its pages: https://daybrite.dev/docs/internal/state/
pub fn root() -> impl Piece {
    // Connect `.restore` below to preferences so the last section survives a relaunch.
    day::prefs::install_nav_store();
    // Settings and About: appearance and language from Day's catalog, the app's name and
    // version from Day.toml. On desktop this adds the app menu's Settings… and About items and
    // their windows; on a phone it is the page behind the Settings section below.
    // https://daybrite.dev/docs/settings/
    let prefs = Preferences::new()
        .about(About::from_manifest().link(day::tr("day-about-website"), "https://daybrite.dev"))
        .group(general_group(res::locales::ALL))
        .install();

    Scene::scoped(move |scene| {
        scene.persist();
        app_menu(menus(scene));

        nav(scene.section)
            .title(res::str::app_title())
            // The list and editor sit side by side on desktop and stack on a phone.
            // Navigation adapts the same pieces: https://daybrite.dev/docs/navigation/
            .content_list(
                ContentList::new(pages::items::item_list_pane)
                    .width(320.0)
                    .only_for(|s: &Section| matches!(s, Section::Items))
                    .detail_visible(scene.detail_open)
                    .detail_title(move || pages::items::detail_title(scene)),
            )
            .items((
                item(Section::Welcome, res::str::nav_welcome())
                    .icon(res::vectors::tab_welcome)
                    .page(pages::welcome::welcome_page),
                item(Section::Items, res::str::nav_items())
                    .icon(res::vectors::tab_items)
                    .page(pages::items::items_page),
                // The desktop's conventional Settings menu/window; without a menu bar, Settings
                // stays reachable as a navigation section (`None` where the menu has it).
                prefs
                    .nav_item(Section::Settings)
                    .map(|i| i.icon(res::vectors::tab_settings)),
            ))
            .restore("app.section")
            .id("nav")
    })
}

/// Share model actions with toolbars; menu roles provide native editing commands.
/// Menus and preferences: https://daybrite.dev/docs/guide-desktop/
fn menus(scene: Scene) -> Vec<MenuEntry> {
    vec![
        sub_menu(
            res::str::menu_file(),
            vec![
                menu_item(res::str::cmd_add())
                    .shortcut(Shortcut::primary("n"))
                    .action(move || scene.new_item()),
                menu_role(MenuRole::CloseWindow),
            ],
        ),
        sub_menu(
            res::str::menu_edit(),
            vec![
                menu_item(res::str::cmd_delete())
                    .shortcut(Shortcut::primary("Delete"))
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
