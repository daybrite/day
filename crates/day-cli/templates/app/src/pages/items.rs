//! A native list and adaptive editor: https://daybrite.dev/docs/internal/list/

use super::detail::editor_pane;
use crate::model::{Item, ItemFields, Scene};
use crate::res;
use day::prelude::*;

/// Read the selected name reactively so the navigation title follows edits.
pub(crate) fn detail_title(scene: Scene) -> String {
    let name = scene
        .selected
        .get()
        .map(|id| scene.items.elem(id).name().read())
        .unwrap_or_default();
    if name.is_empty() {
        res::str::nav_items().format()
    } else {
        name
    }
}

/// Build the detail destination and attach the selected-item Delete command to its toolbar.
pub(crate) fn items_page() -> impl Piece {
    let scene = Scene::ambient();
    editor_pane(scene).grow().toolbar(
        toolbar_button("tb-delete", res::str::cmd_delete())
            .icon(Symbol::Delete)
            .tooltip(res::str::cmd_delete())
            .enabled_when(move || scene.selected.get().is_some())
            .action(move || scene.delete_selected()),
    )
}

/// Build the list pane, synchronizing native selection and ordering with shared state.
pub(crate) fn item_list_pane() -> impl Piece {
    let scene = Scene::ambient();
    // Back closes the editor on a phone and clears the corresponding list selection.
    watch(
        move || scene.detail_open.get(),
        move |open, _| {
            if !open {
                scene.selected.set(None);
            }
        },
    );
    column((
        when(
            move || scene.items.keys().is_empty(),
            || {
                label(res::str::items_empty())
                    .secondary()
                    .padding(16.0)
                    .id("items-empty")
            },
        ),
        // Native rows are recycled: read through `slot` so bindings follow the current item.
        list(scene.items.rows(move || scene.items.keys()), move |slot| {
            row((
                label(move || {
                    let name = slot.name().read();
                    if name.is_empty() {
                        res::str::item_untitled().format()
                    } else {
                        name
                    }
                })
                .grow(),
                when(
                    move || slot.done().read(),
                    || {
                        vector(res::vectors::check)
                            .template()
                            .a11y(|a| a.label(res::str::field_done()))
                            .frame(16.0, 16.0)
                    },
                ),
            ))
            .spacing(8.0)
            .padding(12.0)
            .context_menu(vec![
                menu_item(res::str::cmd_delete().format()).action(move || scene.remove(slot.key())),
            ])
        })
        // Unlike `on_select`, this also reports an empty selection when the user clears it.
        .on_selection(move |rows: Vec<Elem<Item>>| match rows.first() {
            Some(item) => scene.open(item.key()),
            None => scene.selected.set(None),
        })
        .selected_rows(move || {
            scene
                .selected
                .get()
                .and_then(|id| scene.items.keys().iter().position(|key| *key == id))
                .into_iter()
                .collect()
        })
        // Enable the backend's native drag gesture; the model persists the resulting order.
        .reorderable(true)
        .on_reorder(move |from, to| scene.move_item(from, to))
        .a11y(|a| a.label(res::str::nav_items()))
        .scroll_to_row(scene.scroll_to)
        .deletable(true)
        .delete_label(res::str::cmd_delete().format())
        .on_delete(move |index| {
            if let Some(&id) = scene.items.keys().get(index) {
                scene.remove(id);
            }
        })
        .id("item-list")
        .grow(),
    ))
    .grow()
    .toolbar(
        toolbar_button("tb-add", res::str::cmd_add())
            .icon(Symbol::Add)
            .tooltip(res::str::cmd_add())
            .placement(ToolbarPlacement::Primary)
            .action(move || scene.new_item()),
    )
}
