use crate::model::{self, Item, KINDS};
use crate::{res, wide};
use day::prelude::*;

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

fn selected() -> Signal<Option<u32>> {
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
    scroll_to().set(model::ordered().iter().position(|i| i.id == id));
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
pub(crate) fn navigate_page() -> AnyPiece {
    // TWO `when` arms rather than an `if`: a page's builder runs once, so a plain `if wide()`
    // would freeze this page in whichever shape the window had when the section was first opened.
    // `when` re-derives on the tracked read, which is what makes the layout follow the window
    // (https://daybrite.dev/docs/size-classes).
    column((when(wide, master_detail), when(|| !wide(), pushed_stack)))
        .grow()
        .any()
}

/// Wide: list and editor side by side. A `selector` sidebar cannot carry rows this rich — its
/// rows are a label and an icon — so the two panes are composed from ordinary pieces, which is
/// also what lets the same list widget serve both layouts.
fn master_detail() -> AnyPiece {
    row((item_list().width(320.0), editor_pane().grow()))
        .grow()
        .any()
}

/// Narrow: the list fills the window and the editor pushes over it with a native back button.
fn pushed_stack() -> AnyPiece {
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
        .any()
}

/// The detail pane on a desktop: the selected row's editor, or the empty state.
///
/// `each` over a nought-or-one list rather than a conditional, because the editor has to be
/// rebuilt when the SELECTION changes, not merely shown and hidden — keying it on the id is what
/// gives each item its own scope and its own field signals.
fn editor_pane() -> AnyPiece {
    column((
        when(
            move || selected().get().is_none_or(|id| model::find(id).is_none()),
            || {
                column((label(res::str::item_none()).font(Font::Title3),))
                    .align(HAlign::Center)
                    .padding(24.0)
                    .grow()
            },
        ),
        each(
            move || {
                selected()
                    .get()
                    .filter(|id| model::find(*id).is_some())
                    .into_iter()
                    .collect::<Vec<u32>>()
            },
            |id: &u32| *id,
            |slot: ItemSlot<u32, u32>| editor(slot.key()),
        ),
    ))
    .grow()
    .any()
}

/// The list itself — one widget, both layouts.
///
/// Reorder and delete are turned on unconditionally and the backends decide what that means:
/// every toolkit has a drag gesture, and the phones add swipe-to-delete while the desktops
/// answer `Unsupported` for it (https://daybrite.dev/docs/list). That is why the context menu
/// below carries Delete too — a list that must be editable everywhere pairs the gesture with an
/// explicit control, rather than assuming the gesture exists.
fn item_list() -> AnyPiece {
    list(model::ordered, |i: &Item| i.id, row_view)
        .row_height(RowHeight::Uniform(58.0))
        .on_select(move |id: u32| {
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
                .and_then(|id| model::ordered().iter().position(|i| i.id == id))
                .into_iter()
                .collect()
        })
        .scroll_to_row(scroll_to())
        .reorderable(true)
        .on_reorder(model::move_row)
        .deletable(true)
        .delete_label(res::str::cmd_delete().format())
        .on_delete(|index| {
            if let Some(it) = model::ordered().get(index) {
                model::remove(it.id);
            }
        })
        .id("item-list")
        .any()
}

/// One row: the kind's glyph in the item's own color, its name and kind, its rating, and a
/// check when it is finished.
///
/// Every read of `slot` is inside a reactive closure. A recycling list rebinds one physical row
/// to many items as it scrolls, and only reactive bindings follow that rebind — an eager
/// `let name = slot.get()` would freeze the row at whichever item it first showed
/// (https://daybrite.dev/docs/list).
fn row_view(slot: ItemSlot<Item, u32>) -> AnyPiece {
    row((
        // The kind says WHAT it is, the tint says which one it is — two facts in one glyph, and
        // the same color the editor's well sets.
        //
        // `each` over a single-element list, rather than a bare `vector(…)`: a vector's name and
        // tint are fixed when it is built, and a recycled row rebinds to a different item. Keying
        // on the pair rebuilds the glyph exactly when one of them changes, and never otherwise.
        each(
            move || vec![slot.field(|i| (i.kind, i.color.clone()))],
            |kc: &(usize, String)| kc.clone(),
            |k: ItemSlot<(usize, String), (usize, String)>| {
                let (kind, color) = k.key();
                vector(kind_icon(kind))
                    .tint(parse_hex(&color))
                    .frame(20.0, 20.0)
            },
        ),
        column((
            label(move || slot.field(|i| i.name.clone())),
            label(move || slot.field(|i| tr(KINDS[i.kind.min(KINDS.len() - 1)]).format()))
                .font(Font::Caption),
        ))
        .spacing(1.0)
        .align(HAlign::Leading)
        .grow(),
        // Filled stars up to the rating, hollow after — readable at a glance without a number.
        label(move || {
            slot.field(|i| "\u{2605}".repeat(i.rating) + &"\u{2606}".repeat(5 - i.rating.min(5)))
        })
        .font(Font::Caption)
        .color(Color::hex(0xF5A524)),
        label(move || slot.field(|i| i.count.to_string())).tabular(),
        when(
            move || slot.field(|i| i.done),
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
    // user reaches for them they run one closure. The id is read when the command RUNS, not when
    // the row is built — a recycled row points at a different item by then.
    .context_menu(vec![
        menu_item(res::str::cmd_done().format()).action(move || model::toggle_done(slot.key())),
        menu_separator(),
        menu_item(res::str::cmd_delete().format()).action(move || model::remove(slot.key())),
    ])
    .any()
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

/// The editor: every standard two-way binding over one item, in a native form
/// (https://daybrite.dev/docs/forms). Each field reads the item out of the store and writes the
/// change straight back, so there is no draft state to reconcile and no Save button to forget.
fn editor(id: u32) -> AnyPiece {
    let Some(it) = model::find(id) else {
        return label(res::str::item_none()).any();
    };

    let name = Signal::new(it.name.clone());
    let count = Signal::new(it.count);
    let date = Signal::new(
        day_piece_datetime::DayDate::parse_iso(&it.date)
            .unwrap_or_else(day_piece_datetime::DayDate::today),
    );
    let kind = Signal::new(it.kind);
    let done = Signal::new(it.done);
    let notes = Signal::new(it.notes.clone());
    let rating = Signal::new(it.rating);
    let color = Signal::new(parse_hex(&it.color));

    // One effect per field, each writing its own change back. `watch` fires on CHANGE only, so
    // seeding the signals above does not immediately rewrite what was just read.
    watch(
        move || name.get(),
        move |v, _| model::edit(id, |i| i.name = v.clone()),
    );
    watch(
        move || count.get(),
        move |v, _| model::edit(id, |i| i.count = *v),
    );
    watch(
        move || date.get(),
        move |v, _| {
            let s = format!("{:04}-{:02}-{:02}", v.year, v.month, v.day);
            model::edit(id, |i| i.date = s)
        },
    );
    watch(
        move || kind.get(),
        move |v, _| model::edit(id, |i| i.kind = *v),
    );
    watch(
        move || done.get(),
        move |v, _| model::edit(id, |i| i.done = *v),
    );
    watch(
        move || notes.get(),
        move |v, _| model::edit(id, |i| i.notes = v.clone()),
    );
    watch(
        move || rating.get(),
        move |v, _| model::edit(id, |i| i.rating = *v),
    );
    watch(
        move || color.get(),
        move |v, _| {
            let s = hex_of(*v);
            model::edit(id, |i| i.color = s)
        },
    );

    form((
        section((
            labeled(
                res::str::field_name(),
                text_field(name)
                    .placeholder(res::str::field_name_hint())
                    .id("field-name"),
            ),
            labeled(res::str::field_count(), stepper(count)),
            labeled(
                res::str::field_date(),
                day_piece_datetime::date_picker(date).id("field-date"),
            ),
        ))
        .title(res::str::section_basics()),
        section((
            labeled(
                res::str::field_kind(),
                picker(KINDS.iter().map(|k| tr(k).format()), kind)
                    .segmented()
                    .id("field-kind"),
            ),
            labeled(res::str::field_done(), toggle(done).id("field-done")),
            labeled(
                res::str::field_rating(),
                day_piece_rating::rating(rating).max(5).id("field-rating"),
            ),
            labeled(
                res::str::field_color(),
                day_piece_colorpicker::color_picker(color).id("field-color"),
            ),
        ))
        .title(res::str::section_details()),
        section((text_area(notes).min_lines(5).id("field-notes"),))
            .title(res::str::section_notes()),
    ))
    // A form fills its pane, and on a phone that pane IS the window — so without this the rows
    // run into both edges. The desktop's detail pane inherits the same breathing room.
    .padding(Insets {
        top: 0.0,
        leading: 16.0,
        bottom: 0.0,
        trailing: 16.0,
    })
    .any()
}

/// A number field with its own increment/decrement pair. No toolkit in Day's set ships a stepper
/// as a single widget, so this is what one looks like composed — three pieces over one signal,
/// which is also the shortest example of how any control you are missing gets built.
fn stepper(value: Signal<i64>) -> AnyPiece {
    row((
        button("−")
            .action(move || value.update(|v| *v = (*v - 1).max(0)))
            .id("field-count-dec"),
        label(move || value.get().to_string())
            .tabular()
            .reserving("000")
            .id("field-count"),
        button("+")
            .action(move || value.update(|v| *v = (*v + 1).min(999)))
            .id("field-count-inc"),
    ))
    .spacing(8.0)
    .any()
}

/// `#RRGGBB` → color. Malformed values fall back rather than failing: the field is user data.
fn parse_hex(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    u32::from_str_radix(h, 16)
        .ok()
        .filter(|_| h.len() == 6)
        .map(Color::hex)
        .unwrap_or(Color::hex(0x3B82F6))
}

fn hex_of(c: Color) -> String {
    let (r, g, b) = (
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    );
    format!("#{r:02X}{g:02X}{b:02X}")
}
