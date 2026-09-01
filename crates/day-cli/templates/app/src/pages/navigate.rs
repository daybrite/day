use super::detail::{color_of, editor_pane};
use crate::model::{Item, ItemFields, KINDS, Scene};
use crate::res;
use day::prelude::*;

// This file owns CHOOSING an item — the list, the selection, the routes — and `detail.rs` owns
// editing the one that was chosen. The seam between the two is one field of the window's
// [`Scene`]: `scene.selected`.
//
// Nothing here is a global. Every page function starts by asking the environment for the window
// it is being built into (`Scene::ambient()`, https://daybrite.dev/docs/state) and passes that
// value down, so the same functions serve the first window and every File ▸ New Window without
// knowing which is which.

/// The pushed editor's navigation-bar title — `selector(…).detail_title` in `lib.rs`
/// (https://daybrite.dev/docs/navigation). The item being edited, by name, so the bar answers
/// "what am I looking at" the way a native detail page does; the section's own title stands in
/// while the name is empty (a just-created item) or nothing is selected. Reading the name
/// through its field binding is what keeps the bar live as the user types into the name field.
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

/// The Navigate section's DETAIL — the editor for whichever row the content list has selected.
///
/// There is no width check here and no second layout: the list is the section's CONTENT-LIST
/// pane (`item_list_pane`, handed to the selector in `lib.rs`), so the navigation host owns the
/// columns and re-presents them itself as the window changes — a real split on a desktop, a
/// pushed middle layer on a phone (https://daybrite.dev/docs/navigation).
///
/// A bare `fn() -> impl Piece`, because that is what `selector(…).item_icon(…)` takes — so the
/// window it belongs to arrives through the environment rather than through an argument. This is
/// the case ambient state exists for (https://daybrite.dev/docs/state).
pub(crate) fn navigate_page() -> impl Piece {
    editor_pane(Scene::ambient()).grow()
}

/// The content-list pane: the item list in its own column.
pub(crate) fn item_list_pane() -> impl Piece {
    item_list(Scene::ambient()).grow()
}

/// The list itself — one widget, every layout, driven straight by this window's STORE.
///
/// `items.rows(ordered_keys)` hands the list a projection of row KEYS; the rows themselves bind
/// their fields through the slot. The division of labor is the whole performance story
/// (https://daybrite.dev/docs/list): editing a name patches one label in one row — no reload,
/// no rebind, nothing cloned — while a change the ORDER depends on (a done toggle, the filter,
/// an insert) re-runs only the key projection and reloads natively.
///
/// Reorder and delete are turned on unconditionally and the backends decide what that means:
/// every toolkit has a drag gesture, and the phones add swipe-to-delete while the desktops
/// answer `Unsupported` for it (https://daybrite.dev/docs/list). That is why the context menu
/// below carries Delete too — a list that must be editable everywhere pairs the gesture with an
/// explicit control, rather than assuming the gesture exists.
fn item_list(scene: Scene) -> impl Piece {
    list(
        scene.items.rows(move || scene.ordered_keys()),
        move |slot| row_view(scene, slot),
    )
    .row_height(RowHeight::Uniform(58.0))
    // `on_selection`, not `on_select`: only the full set can report a CLEARED selection.
    .on_selection(move |rows: Vec<Elem<Item>>| match rows.first() {
        Some(it) => scene.open(it.key() as u32),
        None => scene.clear_selection(),
    })
    // Two-way selection. `on_select` writes the signal; this reads it back, so a row opened
    // any other way — the "+" command, a restored launch — highlights in the list rather
    // than leaving the editor and the list disagreeing about what is open.
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

/// One row: the kind's glyph in the item's own color, its name and kind, its rating, and a
/// check when it is finished.
///
/// The slot is the row's live connection to the store. Every read is a per-FIELD tracked read
/// inside a reactive closure, so an edit to one field patches exactly the widgets showing it —
/// and when the recycling list rebinds this physical row to a different item, the same closures
/// follow, because the slot resolves its row on every read
/// (https://daybrite.dev/docs/list).
fn row_view(scene: Scene, slot: ModelSlot<Item>) -> impl Piece {
    row((
        // The kind says WHAT it is, the tint says which one it is — two facts in one glyph, and
        // the same color the editor's well sets.
        //
        // `each` over a single-element list, rather than a bare `vector(…)`: a vector's name and
        // tint are fixed when it is built, and a recycled row rebinds to a different item. Keying
        // on the pair rebuilds the glyph exactly when one of them changes, and never otherwise.
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
        // Filled stars up to the rating, hollow after — readable at a glance without a number.
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
    // Secondary-click / long-press. The same two commands the menu bar carries, so however the
    // user reaches for them they run one closure. The key is read when the command RUNS, not
    // when the row is built — a recycled row points at a different item by then, and the slot
    // follows it.
    .context_menu(vec![
        menu_item(res::str::cmd_done().format())
            .action(move || scene.toggle_done(slot.key() as u32)),
        menu_separator(),
        menu_item(res::str::cmd_delete().format()).action(move || scene.remove(slot.key() as u32)),
    ])
}

/// The glyph for a kind, by index — the one place the `KINDS` order and the icons are tied
/// together, so adding a kind is a line here and a line there.
fn kind_icon(kind: usize) -> VectorName {
    match kind {
        1 => res::vectors::kind_task,
        2 => res::vectors::kind_idea,
        _ => res::vectors::kind_note,
    }
}
