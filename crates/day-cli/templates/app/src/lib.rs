//! Day Rise: a welcome page, a list of items with an editor, and settings.

use day::prelude::*;
use day_piece_settings::prelude::*;

mod model;
mod pages;

use model::{SHOW_DONE, Scene};

// Typed constants for everything under resource/: https://daybrite.dev/docs/how-to/resources
day::resources!();
// The mobile and web entry point; desktop enters through src/main.rs.
day::day_start!(options: res::window_options(), root);

day::routes! {
    /// The app's sections, which are also the routes links and dayscript navigate to.
    pub(crate) enum Section {
        Welcome => "welcome",
        Items => "items",
        Settings => "settings",
    }
}

/// The window's content: shared state, then adaptive navigation over the pages.
/// https://daybrite.dev/docs/how-to/navigation
pub fn root() -> impl Piece {
    // Settings and About open from the app menu on desktop and as a section on a phone. The
    // app's own settings are a group of rows over `Pref`s.
    let prefs = Preferences::new()
        .about(About::from_manifest().website("https://daybrite.dev"))
        .group(general_group(res::locales::ALL))
        .group(
            Group::new("items", res::str::nav_items())
                .icon(res::vectors::tab_items)
                .row(rows::toggle(&SHOW_DONE, res::str::show_done())),
        )
        .install();

    Scene::scoped(move |scene| {
        app_menu(menus(scene));
        undo_history(scene);
        nav(scene.section)
            .title(res::str::app_title())
            // The list and the editor sit side by side on desktop and stack on a phone.
            .content_list(
                ContentList::new(pages::item_list)
                    .only_for(|s: &Section| *s == Section::Items)
                    .detail_for(scene.selected)
                    .detail_title(move || scene.selected_title()),
            )
            .items((
                item(Section::Welcome, res::str::nav_welcome())
                    .icon(res::vectors::tab_welcome)
                    .page(pages::welcome),
                item(Section::Items, res::str::nav_items())
                    .icon(res::vectors::tab_items)
                    .badge(move || scene.open_count())
                    .page(pages::editor),
                // `None` where the desktop app menu holds Settings instead.
                prefs
                    .nav_item(Section::Settings)
                    .map(|i| i.icon(res::vectors::tab_settings)),
            ))
            .restore("app.section")
            .id(day::id!(nav))
    })
}

/// The menu bar; menu roles supply the native editing commands.
/// https://daybrite.dev/docs/how-to/guide-desktop
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
                menu_role(MenuRole::Undo),
                menu_role(MenuRole::Redo),
                menu_separator(),
                menu_item(res::str::cmd_delete())
                    .shortcut(Shortcut::primary("Delete"))
                    .action(move || scene.delete_selected()),
                menu_separator(),
                menu_item(res::str::cmd_find())
                    .shortcut(Shortcut::primary("f"))
                    .action(day::focus_search),
                menu_separator(),
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
    ]
}

/// Undo and Redo for every change to the items, through the Edit menu and the platform's own
/// gestures. Each change is named in the menu by its field or command.
/// https://daybrite.dev/docs/explanation/reactivity
fn undo_history(scene: Scene) {
    let history = day::model::UndoStack::new(100);
    history.watch(scene.items);
    history.set_label_resolver(|label| {
        match label {
            "push" => res::str::cmd_add(),
            "remove" => res::str::cmd_delete(),
            "move" => res::str::undo_move(),
            "name" => res::str::field_name(),
            "done" => res::str::field_done(),
            "kind" => res::str::field_kind(),
            "tint" => res::str::field_tint(),
            other => return other.to_owned(),
        }
        .format()
    });
    day::install_undo(&history);
}
