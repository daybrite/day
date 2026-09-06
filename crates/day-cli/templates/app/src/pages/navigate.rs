use super::detail::{color_of, editor_pane};
use crate::model::{Item, ItemFields, KINDS, Scene};
use crate::res;
use day::prelude::*;

// This file chooses an item: the list, the selection, the routes. `detail.rs` edits the one
// chosen. Every page asks the environment for its window with `Scene::ambient()` rather than
// touching a global, so the same code serves every File ▸ New Window.

/// The pushed editor's bar title: the item's name, live as the user types it, or the section's
/// title while nothing is named.
pub(crate) fn detail_title(scene: Scene) -> String {
    let name = scene
        .selected
        .get()
        .filter(|id| scene.find(*id).is_some())
        .map(|id| scene.items.elem(id as u64).name().read())
        .unwrap_or_default();
    if name.is_empty() {
        res::str::nav_navigate().format()
    } else {
        name
    }
}

/// The Navigate section's detail: the editor for whichever row the content list selected.
/// The nav host owns the columns, so there is no width check here; it splits on a desktop and
/// pushes on a phone (https://daybrite.dev/docs/navigation).
pub(crate) fn navigate_page() -> impl Piece {
    let scene = Scene::ambient();
    // Done acts on the open item, so it belongs to the editor's own chrome and comes and goes
    // with it (https://daybrite.dev/docs/guide-desktop).
    editor_pane(scene).grow().toolbar(
        toolbar_button("tb-done", res::str::cmd_done())
            .icon(Symbol::Check)
            .tooltip(res::str::cmd_done())
            // Disabled until an item is open.
            .enabled_when(move || scene.selected.get().is_some())
            .action(move || scene.done_selected()),
    )
}

/// The content-list pane, with the commands that act on it. Declared here, they ride the
/// pane's own chrome: a column toolbar on a desktop, the navigation bar on a phone.
pub(crate) fn item_list_pane() -> impl Piece {
    let scene = Scene::ambient();
    item_list(scene).grow().toolbar([
        toolbar_toggle("tb-show-done", res::str::cmd_show_done(), scene.show_done)
            .icon(Symbol::Filter)
            .tooltip(res::str::cmd_show_done()),
        toolbar_button("tb-add", res::str::cmd_add())
            .icon(Symbol::Add)
            .tooltip(res::str::cmd_add())
            .placement(ToolbarPlacement::Primary)
            .action(move || scene.new_item()),
    ])
}

/// The list, driven straight by the store. Rows bind their fields through the slot, so editing
/// a name patches one label, while a change to the order re-runs only the key projection.
///
/// Reorder and delete are always on; the phones add swipe-to-delete and the desktops answer
/// `Unsupported`, which is why the context menu carries Delete too.
fn item_list(scene: Scene) -> impl Piece {
    list(
        scene.items.rows(move || scene.ordered_keys()),
        move |slot| row_view(scene, slot),
    )
    .row_height(RowHeight::Uniform(58.0))
    // `on_selection`, not `on_select`: only the full set can report a cleared selection.
    .on_selection(move |rows: Vec<Elem<Item>>| match rows.first() {
        Some(it) => scene.open(it.key() as u32),
        None => scene.clear_selection(),
    })
    // Read the selection back too, so a row opened any other way highlights in the list.
    .selected_rows(move || {
        scene
            .selected
            .get()
            .and_then(|id| scene.ordered_keys().iter().position(|k| *k == id as u64))
            .into_iter()
            .collect()
    })
    .scroll_to_row(scene.scroll_to)
    .reorderable(true)
    .on_reorder(move |from, to| scene.move_row(from, to))
    .deletable(true)
    .delete_label(res::str::cmd_delete().format())
    .on_delete(move |index| {
        if let Some(&k) = scene.ordered_keys().get(index) {
            scene.remove(k as u32);
        }
    })
    .id("item-list")
}

/// One row: the kind's glyph in the item's color, name and kind, rating, and a check when done.
/// Every read goes through the slot, so a recycled row follows whichever item it is bound to.
fn row_view(scene: Scene, slot: ModelSlot<Item>) -> impl Piece {
    row((
        // `each` over a one-element list rather than a bare `vector(…)`: a vector's name and
        // tint are fixed at build, and a recycled row rebinds to a different item.
        each(
            items(
                move || vec![(slot.kind().read(), slot.color().read())],
                |kc: &(usize, String)| kc.clone(),
            ),
            |k: ItemSlot<(usize, String), (usize, String)>| {
                let (kind, color) = k.key();
                vector(kind_icon(kind))
                    .tint(color_of(&color))
                    .frame(20.0, 20.0)
            },
        ),
        column((
            label(move || slot.name().read()),
            label(move || tr(KINDS[slot.kind().read().min(KINDS.len() - 1)]).format())
                .font(Font::Caption),
        ))
        .spacing(1.0)
        .align(HAlign::Leading)
        .grow(),
        // Filled stars up to the rating, hollow after.
        label(move || {
            let r = slot.rating().read();
            "\u{2605}".repeat(r) + &"\u{2606}".repeat(5 - r.min(5))
        })
        .font(Font::Caption)
        .color(Color::hex(0xF5A524)),
        label(move || slot.count().read().to_string()).tabular(),
        when(
            move || slot.done().read(),
            move || {
                vector(res::vectors::check)
                    .tint(Color::hex(0x10B981))
                    .frame(16.0, 16.0)
            },
        ),
    ))
    .spacing(10.0)
    .padding(Insets {
        top: 8.0,
        leading: 12.0,
        bottom: 8.0,
        trailing: 12.0,
    })
    // Secondary click or long press, running the same closures as the menu bar. The key is
    // read when the command runs, since a recycled row points elsewhere by then.
    .context_menu(vec![
        menu_item(res::str::cmd_done().format())
            .action(move || scene.toggle_done(slot.key() as u32)),
        menu_separator(),
        menu_item(res::str::cmd_delete().format()).action(move || scene.remove(slot.key() as u32)),
    ])
}

/// The glyph for a kind, by index. Adding a kind is a line here and a line in `KINDS`.
fn kind_icon(kind: usize) -> VectorName {
    match kind {
        1 => res::vectors::kind_task,
        2 => res::vectors::kind_idea,
        _ => res::vectors::kind_note,
    }
}
