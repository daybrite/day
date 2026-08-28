use super::detail::{color_of, editor_pane};
use crate::model::{self, Item, ItemFields, KINDS};
use crate::res;
use day::prelude::*;

// This file owns CHOOSING an item — the list, the selection, the commands — and `detail.rs`
// owns editing the one that was chosen. The seam is `selected()` and `detail_open()`, below.
//
// Both are globals rather than page state because the navigation host may rebuild either pane
// when the window crosses a breakpoint: the selection has to outlive the layout it was made in.
thread_local! {
    // `Signal::global`: this outlives the page that first reads it. With `Signal::new` it would
    // be created inside the Navigate page's scope and disposed the moment the user switched to
    // Welcome — after which every read panics (https://daybrite.dev/docs/state).
    static SELECTED: Signal<Option<u32>> = Signal::global(None);
    // The row the list should scroll into view; cleared once it has.
    static SCROLL_TO: Signal<Option<usize>> = Signal::global(None);
    // Whether the EDITOR is showing. A three-column window always shows it beside the list; a
    // shape that shows one pane at a time pushes it over the list and pops back, and this is
    // the signal the navigation host drives in both directions — `selector(…).detail_visible`
    // in `lib.rs` (https://daybrite.dev/docs/navigation).
    static DETAIL_OPEN: Signal<bool> = Signal::global(false);
}

/// The seam between this file and `detail.rs`: the editor pane reads it, the list writes it.
pub(crate) fn selected() -> Signal<Option<u32>> {
    SELECTED.with(|s| *s)
}

fn scroll_to() -> Signal<Option<usize>> {
    SCROLL_TO.with(|s| *s)
}

/// The other half of that seam: whether the editor is up. The navigation host both reads this
/// (to push the editor) and writes it (when a native back gesture pops back to the list).
pub(crate) fn detail_open() -> Signal<bool> {
    DETAIL_OPEN.with(|s| *s)
}

/// Open `id` in the editor — the one path every "show me this item" command takes. The layout
/// is not this function's business: a wide window already has the editor beside the list, a
/// narrow one pushes it, and the host decides which (https://daybrite.dev/docs/navigation).
fn open(id: u32) {
    selected().set(Some(id));
    detail_open().set(true);
}

// --- commands, shared by the toolbar, the menu bar, and the row context menus ---------------

/// Create an item and open its editor straight away — the "new" flow every list app has, where
/// the row you just made is the row you want to be typing into.
pub(crate) fn new_item() {
    let id = model::add();
    open(id);
    // A hundred rows in, a new one lands off screen. Ask the list to bring it into view by its
    // DISPLAY index, which is not the order it was appended in — finished items float to the top
    // (https://daybrite.dev/docs/list).
    scroll_to().set(model::ordered_keys().iter().position(|k| *k == id as u64));
}

pub(crate) fn delete_selected() {
    if let Some(id) = selected().get_untracked() {
        model::remove(id);
        selected().set(None);
        // Nothing left to edit: on the shapes that pushed the editor, this pops back to the list.
        detail_open().set(false);
    }
}

pub(crate) fn done_selected() {
    if let Some(id) = selected().get_untracked() {
        model::toggle_done(id);
    }
}

// --- the page -------------------------------------------------------------------------------

/// The Navigate section's DETAIL — the editor for whichever row the content list has selected.
///
/// There is no width check here and no second layout: the list is the section's CONTENT-LIST
/// pane (`item_list_pane`, handed to the selector in `lib.rs`), so the navigation host owns the
/// columns and re-presents them itself as the window changes — a real split on a desktop, a
/// pushed middle layer on a phone (https://daybrite.dev/docs/navigation).
pub(crate) fn navigate_page() -> impl Piece {
    editor_pane().grow()
}

/// The content-list pane: the item list in its own column.
pub(crate) fn item_list_pane() -> impl Piece {
    item_list().grow()
}

/// The list itself — one widget, every layout, driven straight by the STORE.
///
/// `items().rows(ordered_keys)` hands the list a projection of row KEYS; the rows themselves
/// bind their fields through the slot. The division of labor is the whole performance story
/// (https://daybrite.dev/docs/list): editing a name patches one label in one row — no reload,
/// no rebind, nothing cloned — while a change the ORDER depends on (a done toggle, the filter,
/// an insert) re-runs only the key projection and reloads natively.
///
/// Reorder and delete are turned on unconditionally and the backends decide what that means:
/// every toolkit has a drag gesture, and the phones add swipe-to-delete while the desktops
/// answer `Unsupported` for it (https://daybrite.dev/docs/list). That is why the context menu
/// below carries Delete too — a list that must be editable everywhere pairs the gesture with an
/// explicit control, rather than assuming the gesture exists.
fn item_list() -> impl Piece {
    list(model::items().rows(model::ordered_keys), row_view)
        .row_height(RowHeight::Uniform(58.0))
        .on_select(move |it: Elem<Item>| open(it.key() as u32))
        // Two-way selection. `on_select` writes the signal; this reads it back, so a row opened
        // any other way — the "+" command, a restored launch — highlights in the list rather
        // than leaving the editor and the list disagreeing about what is open.
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

/// One row: the kind's glyph in the item's own color, its name and kind, its rating, and a
/// check when it is finished.
///
/// The slot is the row's live connection to the store. Every read is a per-FIELD tracked read
/// inside a reactive closure, so an edit to one field patches exactly the widgets showing it —
/// and when the recycling list rebinds this physical row to a different item, the same closures
/// follow, because the slot resolves its row on every read
/// (https://daybrite.dev/docs/list).
fn row_view(slot: ModelSlot<Item>) -> impl Piece {
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
            .action(move || model::toggle_done(slot.key() as u32)),
        menu_separator(),
        menu_item(res::str::cmd_delete().format()).action(move || model::remove(slot.key() as u32)),
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
