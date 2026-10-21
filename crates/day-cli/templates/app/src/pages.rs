//! The pages: Welcome, the item list, and the item editor.

use crate::model::{ItemFields, Kind, SHOW_DONE, Scene, Tint, display_name};
use crate::res;
use day::prelude::*;

/// The opening page: the app mark, a greeting, and Markdown prose from the catalog, whose
/// `#items` and `#settings` links navigate to those routes.
pub(crate) fn welcome() -> impl Piece {
    // Scrolls only when the window is too short for it, such as a phone turned sideways.
    scroll(
        column((
            spacer(),
            // Decorative art stays out of the accessibility tree: https://daybrite.dev/docs/accessibility
            vector(res::vectors::app_mark)
                .decorative()
                .frame(132.0, 132.0)
                .corner_radius(30.0)
                .id(day::id!(welcome_mark)),
            label(res::str::welcome_title())
                .font(Font::LargeTitle)
                .a11y(|a| a.role(Role::Heading(1)))
                .align(TextAlign::Center)
                .id(day::id!(welcome_title)),
            label(res::str::welcome_body())
                .markdown()
                .align(TextAlign::Center)
                .max_width(440.0)
                .id(day::id!(welcome_body)),
            spacer(),
        ))
        .spacing(Space::L)
        // Fill the pane before padding it, so the column centers in the whole pane.
        .grow()
        .padding(Space::XL),
    )
}

/// The item list: a native list of the items matching the search, whose selection, reordering,
/// swipe and delete gestures and context menu work on the scene's items.
pub(crate) fn item_list() -> impl Piece {
    let scene = Scene::ambient();
    // The Done command a row's context menu and its leading swipe share.
    let done_label = move |id: u64| match scene.items.elem(id).done().peek() {
        true => res::str::cmd_mark_not_done(),
        false => res::str::cmd_mark_done(),
    };
    column((
        when(
            move || scene.visible_keys().is_empty(),
            move || {
                label(move || match scene.query.get().is_empty() {
                    true => res::str::items_empty().format(),
                    false => res::str::search_empty().format(),
                })
                .secondary()
                .padding(Space::L)
                .id(day::id!(items_empty))
            },
        ),
        // Rows are recycled, so each one reads its current item through `slot`. A list item
        // is the platform's own row: its fonts, icon size, check mark and spacing.
        list(
            scene.items.rows(move || scene.visible_keys()),
            move |slot| {
                list_item(move || display_name(slot.name().read()))
                    .icon(move || slot.kind().read().icon())
                    .icon_tint(move || slot.tint().read().color())
                    .checked(move || slot.done().read())
                    .context_menu(move || {
                        let id = slot.key();
                        vec![
                            menu_item(done_label(id)).action(move || scene.toggle_done(id)),
                            menu_item(res::str::cmd_delete()).action(move || scene.remove(id)),
                        ]
                    })
            },
        )
        .selection(scene.selected)
        // Rows map to the store's order only while nothing is filtered out.
        .reorderable(
            Reorder::new(move |from, to| {
                scene.items.move_row(from, to);
            })
            .enabled(move || scene.query.get().is_empty() && SHOW_DONE.signal().get()),
        )
        .swipe_leading(move |row| {
            let id = scene.key_at(row);
            id.map(|id| {
                swipe_action(done_label(id))
                    .symbol(Symbol::Check)
                    .action(move || scene.toggle_done(id))
            })
            .into_iter()
            .collect()
        })
        .deletable(
            Delete::new(move |row| {
                if let Some(id) = scene.key_at(row) {
                    scene.remove(id);
                }
            })
            .label(res::str::cmd_delete()),
        )
        .a11y(|a| a.label(res::str::nav_items()))
        .id(day::id!(item_list))
        .grow(),
    ))
    .grow()
    // The window toolbar holds the field on desktop; a phone shows it with the list.
    .searchable(Search::new(scene.query).prompt(res::str::search_items()))
    .toolbar(
        toolbar_button(day::id!(tb_add), res::str::cmd_add())
            .icon(Symbol::Add)
            .tooltip(res::str::cmd_add())
            .placement(ToolbarPlacement::Primary)
            .action(move || scene.new_item()),
    )
}

/// The editor: native form controls bound to the selected item's fields.
/// https://daybrite.dev/docs/reactivity
pub(crate) fn editor() -> impl Piece {
    let scene = Scene::ambient();
    column((
        when(
            move || scene.selected.get().is_none(),
            || label(res::str::item_none()).secondary().padding(Space::XL),
        ),
        // A new key rebuilds the form, so its bindings always edit the selected item.
        keyed(
            move || scene.selected.get(),
            move |&id| {
                let item = scene.items.elem(id);
                form((section((
                    labeled(
                        res::str::field_name(),
                        text_field(item.name())
                            .placeholder(res::str::field_name_hint())
                            .id(day::id!(field_name)),
                    ),
                    labeled(
                        res::str::field_done(),
                        toggle(item.done()).id(day::id!(field_done)),
                    ),
                    labeled(
                        res::str::field_kind(),
                        picker(item.kind(), Kind::options())
                            .segmented()
                            .id(day::id!(field_kind)),
                    ),
                    labeled(
                        res::str::field_tint(),
                        picker(item.tint(), Tint::options()).id(day::id!(field_tint)),
                    ),
                )),))
                .padding(Space::L)
            },
        ),
    ))
    .grow()
    .toolbar(
        toolbar_button(day::id!(tb_delete), res::str::cmd_delete())
            .icon(Symbol::Delete)
            .tooltip(res::str::cmd_delete())
            .enabled(move || scene.selected.get().is_some())
            .action(move || scene.delete_selected()),
    )
}
