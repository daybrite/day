use super::detail::{color_of, editor_pane};
use crate::model::{self, Item, ItemFields, KINDS};
use crate::res;
use day::prelude::*;

// This file owns choosing an item (list, selection, commands); `detail.rs` owns editing it.
thread_local! {
    // `Signal::global`: outlives the page scope that first reads it
    // (https://daybrite.dev/docs/state).
    static SELECTED: Signal<Option<u32>> = Signal::global(None);
    // The row the list should scroll into view; cleared once it has.
    static SCROLL_TO: Signal<Option<usize>> = Signal::global(None);
    // Whether the editor is showing; `selector(…).detail_visible` drives it both ways.
    static DETAIL_OPEN: Signal<bool> = Signal::global(false);
}

/// Shared with `detail.rs`: the editor pane reads it, the list writes it.
pub(crate) fn selected() -> Signal<Option<u32>> {
    SELECTED.with(|s| *s)
}

fn scroll_to() -> Signal<Option<usize>> {
    SCROLL_TO.with(|s| *s)
}

/// Whether the editor is up; the navigation host both reads and writes it.
pub(crate) fn detail_open() -> Signal<bool> {
    DETAIL_OPEN.with(|s| *s)
}

/// The pushed editor's bar title: the edited item's name, live as the user types.
pub(crate) fn detail_title() -> String {
    let name = selected()
        .get()
        .filter(|id| model::find(*id).is_some())
        .map(|id| model::items().elem(id as u64).name().read())
        .unwrap_or_default();
    if name.is_empty() {
        res::str::nav_navigate().format()
    } else {
        name
    }
}

/// Open `id` in the editor; every "show me this item" command funnels through here.
fn open(id: u32) {
    selected().set(Some(id));
    detail_open().set(true);
}

// --- commands, shared by the toolbar, the menu bar, and the row context menus ---------------

/// Create an item and open its editor straight away.
pub(crate) fn new_item() {
    let id = model::add();
    open(id);
    // Bring the new row into view by its display index (https://daybrite.dev/docs/list).
    scroll_to().set(model::ordered_keys().iter().position(|k| *k == id as u64));
}

pub(crate) fn delete_selected() {
    if let Some(id) = selected().get_untracked() {
        model::remove(id);
        selected().set(None);
        // Pops back to the list on the shapes that pushed the editor.
        detail_open().set(false);
    }
}

pub(crate) fn done_selected() {
    if let Some(id) = selected().get_untracked() {
        model::toggle_done(id);
    }
}

// --- the page -------------------------------------------------------------------------------

/// The Navigate section's detail: the editor for whichever row the content list selected.
/// The navigation host owns the columns (https://daybrite.dev/docs/navigation).
pub(crate) fn navigate_page() -> impl Piece {
    editor_pane().grow()
}

/// The content-list pane: the item list in its own column.
pub(crate) fn item_list_pane() -> impl Piece {
    item_list().grow()
}

/// One list, every layout, driven straight by the store: rows bind per field, so an edit
/// patches widgets in place and only order changes reload (https://daybrite.dev/docs/list).
fn item_list() -> impl Piece {
    list(model::items().rows(model::ordered_keys), row_view)
        .row_height(RowHeight::Uniform(58.0))
        .on_select(move |it: Elem<Item>| open(it.key() as u32))
        // Read back too, so a row opened any other way highlights in the list.
        .selected_rows(move || {
            selected()
                .get()
                .and_then(|id| model::ordered_keys().iter().position(|k| *k == id as u64))
                .into_iter()
                .collect()
        })
        .scroll_to_row(scroll_to())
        .reorderable(true)
        .on_reorder(model::move_row)
        .deletable(true)
        .delete_label(res::str::cmd_delete().format())
        .on_delete(|index| {
            if let Some(&k) = model::ordered_keys().get(index) {
                model::remove(k as u32);
            }
        })
        .id("item-list")
}

/// One row. Slot reads are per-field and re-resolve on every read, so a recycled row follows
/// the item it was rebound to (https://daybrite.dev/docs/list).
fn row_view(slot: ModelSlot<Item>) -> impl Piece {
    row((
        // Keyed on (kind, color): a vector's name and tint are fixed at build, so the glyph is
        // rebuilt exactly when either changes.
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
    // Same commands as the menu bar; the key is read when the command runs, not at build.
    .context_menu(vec![
        menu_item(res::str::cmd_done().format())
            .action(move || model::toggle_done(slot.key() as u32)),
        menu_separator(),
        menu_item(res::str::cmd_delete().format()).action(move || model::remove(slot.key() as u32)),
    ])
}

/// The glyph for a kind, by `KINDS` index.
fn kind_icon(kind: usize) -> VectorName {
    match kind {
        1 => res::vectors::kind_task,
        2 => res::vectors::kind_idea,
        _ => res::vectors::kind_note,
    }
}
