use super::detail::{color_of, editor, editor_pane};
use crate::model::{self, Item, ItemFields, KINDS};
use crate::{res, wide};
use day::prelude::*;

// This file owns CHOOSING an item — the list, the selection, the routes, the commands — and
// `detail.rs` owns editing the one that was chosen. The seam is `selected()`, below.
//
// The row the editor is showing. One signal drives BOTH shapes this page takes — the pushed
// stack on a phone and the two-pane layout on a desktop — so crossing a breakpoint rebuilds the
// layout without losing the user's place.
thread_local! {
    // `Signal::global`: this outlives the page that first reads it. With `Signal::new` it would
    // be created inside the Navigate page's scope and disposed the moment the user switched to
    // Welcome — after which every read panics (https://daybrite.dev/docs/state).
    static SELECTED: Signal<Option<u32>> = Signal::global(None);
    // The row the list should scroll into view; cleared once it has.
    static SCROLL_TO: Signal<Option<usize>> = Signal::global(None);
}

/// The seam between this file and `detail.rs`: the editor pane reads it, the list writes it.
pub(crate) fn selected() -> Signal<Option<u32>> {
    SELECTED.with(|s| *s)
}

fn scroll_to() -> Signal<Option<usize>> {
    SCROLL_TO.with(|s| *s)
}

/// A typed route carrying the item's id (https://daybrite.dev/docs/navigation): `Row { id }`
/// encodes as `item-<id>` and parses back, so a deep link to `navigate/item-42` validates on the
/// way in and the destination builder receives a parsed value rather than a string to split.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Row {
    pub id: u32,
}

impl Route for Row {
    fn key(&self) -> String {
        format!("item-{}", self.id)
    }
    fn from_key(key: &str) -> Option<Self> {
        key.strip_prefix("item-")?.parse().ok().map(|id| Row { id })
    }
    /// What the native navigation bar shows above the editor.
    fn title(&self) -> String {
        model::find(self.id)
            .map(|i| i.name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| res::str::item_untitled().format())
    }
}

// --- commands, shared by the toolbar, the menu bar, and the row context menus ---------------

/// Create an item and open its editor straight away — the "new" flow every list app has, where
/// the row you just made is the row you want to be typing into.
pub(crate) fn new_item() {
    let id = model::add();
    selected().set(Some(id));
    // A hundred rows in, a new one lands off screen. Ask the list to bring it into view by its
    // DISPLAY index, which is not the order it was appended in — finished items float to the top
    // (https://daybrite.dev/docs/list).
    scroll_to().set(model::ordered_keys().iter().position(|k| *k == id as u64));
    if !wide() {
        route(&crate::Section::Navigate)
            .then(&Row { id })
            .navigate();
    }
}

pub(crate) fn delete_selected() {
    if let Some(id) = selected().get_untracked() {
        model::remove(id);
        selected().set(None);
    }
}

pub(crate) fn done_selected() {
    if let Some(id) = selected().get_untracked() {
        model::toggle_done(id);
    }
}

// --- the page -------------------------------------------------------------------------------

/// A list that drills into an editor.
///
/// The SHAPE follows the window and the two shapes share everything but their frame: a desktop
/// shows list and editor side by side, a phone pushes the editor over the list with a native back
/// button. Both read `SELECTED`, both write through `model`, and neither knows about the other.
pub(crate) fn navigate_page() -> impl Piece {
    // TWO `when` arms rather than an `if`: a page's builder runs once, so a plain `if wide()`
    // would freeze this page in whichever shape the window had when the section was first opened.
    // `when` re-derives on the tracked read, which is what makes the layout follow the window
    // (https://daybrite.dev/docs/size-classes).
    column((when(wide, master_detail), when(|| !wide(), pushed_stack))).grow()
}

/// Wide: list and editor side by side. A `selector` sidebar cannot carry rows this rich — its
/// rows are a label and an icon — so the two panes are composed from ordinary pieces, which is
/// also what lets the same list widget serve both layouts.
fn master_detail() -> impl Piece {
    row((item_list().width(320.0), editor_pane().grow())).grow()
}

/// Narrow: the list fills the window and the editor pushes over it with a native back button.
fn pushed_stack() -> impl Piece {
    let path = Signal::new(Vec::<Row>::new());
    stack(path, item_list())
        .destination(|r: &Row| editor(r.id))
        // The phones have no window toolbar, so the same commands ride the navigation bar instead
        // (https://daybrite.dev/docs/navigation) — the same three the desktop toolbar carries, in
        // the same order, so the app is one app on either.
        //
        // `list_action`, not `bar_action`: all three act on the LIST. Pushed over the editor they
        // would be acting on something the user can no longer see — a phone has covered the list
        // with the item they tapped — so they belong to the list's own bar and stop there.
        .list_action(res::vectors::filter, res::str::cmd_show_done(), || {
            crate::model::show_done().update(|v| *v = !*v)
        })
        .list_action(res::vectors::check, res::str::cmd_done(), done_selected)
        .list_action(res::vectors::add, res::str::cmd_add(), new_item)
    // No `.id()` here. Where this stack MERGES into an enclosing one — which is what happens
    // on a phone, so the whole chain is a single native navigation controller — it returns
    // its ROOT's node rather than a host of its own. An id here would therefore retag the
    // list, and the list's own id would be the one that vanished
    // (https://daybrite.dev/docs/navigation).
}

/// The list itself — one widget, both layouts, driven straight by the STORE.
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
        .on_select(move |it: Elem<Item>| {
            let id = it.key() as u32;
            selected().set(Some(id));
            if !wide() {
                route(&crate::Section::Navigate)
                    .then(&Row { id })
                    .navigate();
            }
        })
        // Two-way selection. `on_select` writes the signal; this reads it back, so a row opened
        // any other way — the "+" command, a deep link, a restored launch — highlights in the
        // list rather than leaving the form and the list disagreeing about what is open.
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
