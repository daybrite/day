//! The app's domain object and its store.
//!
//! Everything the UI shows is a projection of ONE store — `items()` — so no page owns state and
//! no page has to tell another that something changed. The store is observable PER PROPERTY
//! (https://daybrite.dev/docs/model): `#[derive(Observable)]` turns every field of [`Item`] into
//! a typed accessor, and `items().elem(id).name()` is a two-way binding a `text_field` takes
//! directly — the editor needs no draft signals and no write-back plumbing. Editing a name wakes
//! exactly the readers of that name; the list re-runs only when the collection's SHAPE changes.
//!
//! Persistence is deliberately boring: the whole list is one JSON blob under one `day::prefs`
//! key, written by one coarse subscription in [`load`]. That is enough for a starter, survives
//! an Android process death (prefs is disk-backed), and is the piece you are most likely to
//! replace first — swap `load`/`save` for your database and nothing above this file changes.

use day::model::Op;
use day::prelude::*;
use serde::{Deserialize, Serialize};

/// One row. `id` is stable across reorders and edits: the list keys on it, a route segment
/// carries it, and `#[obs(key)]` makes it the key an `elem(id)` handle addresses — never the
/// index, which changes the moment a row moves.
#[derive(Observable, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
    /// ISO-8601 (`YYYY-MM-DD`). Stored as a string so the JSON stays readable and the date
    /// piece's own type does not leak into the persisted shape.
    pub date: String,
    /// Index into `KINDS` — a segmented picker's selection.
    pub kind: usize,
    pub done: bool,
    pub notes: String,
    pub rating: usize,
    /// `#RRGGBB`, the color well's value.
    pub color: String,
}

/// The `kind` picker's options. Fluent keys rather than literals so they localize
/// (https://daybrite.dev/docs/localization).
pub(crate) const KINDS: [&str; 3] = ["item_kind_note", "item_kind_task", "item_kind_idea"];

const STORE_KEY: &str = "app.items";
const SHOW_DONE_KEY: &str = "app.show_done";
const SEED_COUNT: u32 = 100;

thread_local! {
    // A `Store` handle is `Copy` and process-lifetime, like `Signal::global`: created inside
    // whatever scope first touches it, it does NOT die with that scope, which is what an
    // app-wide store needs (https://daybrite.dev/docs/model).
    static ITEMS: Store<Keyed<Item>> = Store::new(Keyed::default());
    /// Whether finished items are listed at all. A VIEW preference rather than data, but it is
    /// persisted all the same — a filter the user has to re-apply on every launch is a filter
    /// they stop using.
    static SHOW_DONE: Signal<bool> = Signal::global(true);
}

/// The one store every page reads. Reads track what they touch: a field binding follows its own
/// field, `ordered()` follows the whole collection, and neither wakes for the other's changes.
pub(crate) fn items() -> Store<Keyed<Item>> {
    ITEMS.with(|s| *s)
}

pub(crate) fn show_done() -> Signal<bool> {
    SHOW_DONE.with(|s| *s)
}

/// Read the saved list, or seed one on first launch. Call once, before the UI mounts.
pub(crate) fn load() {
    let saved = day::prefs::get(STORE_KEY)
        .and_then(|s| serde_json::from_str::<Vec<Item>>(&s).ok())
        .filter(|v| !v.is_empty());
    items().update("load", |k| *k = Keyed::new(saved.unwrap_or_else(seed)));
    show_done().set(
        day::prefs::get(SHOW_DONE_KEY)
            .map(|v| v != "0")
            .unwrap_or(true),
    );
    // Persist the filter on every change, so it is one `watch` rather than a write at each of
    // the three places that can flip it (the toolbar, the menu, and a restored launch).
    watch(
        move || show_done().get(),
        |v, _| {
            day::prefs::set(SHOW_DONE_KEY, if *v { "1" } else { "0" });
        },
    );
    // Persist the list the same way: ONE coarse subscription instead of a save call at every
    // write site. The tracked whole-store read wakes for any field write, insert, delete or
    // reorder — precision is something a reader opts OUT of by reading coarsely — and the
    // version number is the cheap value `watch` diffs.
    let store = items();
    watch(
        move || {
            store.with(|_| {});
            store.version()
        },
        move |_, _| save(),
    );
}

/// Write the list back. Registered once in [`load`], so persistence is not something a write
/// site can forget.
fn save() {
    let json = items().with_untracked(|k| serde_json::to_string(k.items()));
    if let Ok(json) = json {
        day::prefs::set(STORE_KEY, &json);
    }
}

/// Rows as the list shows them: finished ones first, optionally hidden altogether, each group
/// keeping the user's own order.
///
/// Sorting and filtering HERE rather than in the page is what keeps the list, the editor's
/// neighbors, and the reorder indices agreeing on what "row 3" means — a page that filtered its
/// own copy would hand `on_reorder` an index the model could not resolve.
pub(crate) fn ordered() -> Vec<Item> {
    let show = show_done().get();
    let mut v: Vec<Item> = items().with(|k| {
        k.map(|k| {
            k.items()
                .iter()
                .filter(|i| show || !i.done)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    });
    // A STABLE sort, so the user's own order survives inside each group.
    v.sort_by_key(|i| !i.done);
    v
}

pub(crate) fn find(id: u32) -> Option<Item> {
    items().with(|k| k.and_then(|k| k.get(id as u64).cloned()))
}

pub(crate) fn remove(id: u32) {
    items().restructure("remove", Op::Delete, id as u64, |k| {
        k.remove(id as u64);
    });
}

pub(crate) fn toggle_done(id: u32) {
    // A field write through the same accessor the editor binds — one path for every writer.
    items().elem(id as u64).done().update(|d| *d = !*d);
}

/// Append a fresh row and answer its id, so the caller can drill straight into its editor.
pub(crate) fn add() -> u32 {
    let id = items().with_untracked(|k| k.items().iter().map(|i| i.id).max().unwrap_or(0)) + 1;
    items().restructure("add", Op::Insert, id as u64, |k| {
        k.push(Item {
            id,
            name: String::new(),
            count: 1,
            date: today_iso(),
            kind: 0,
            done: false,
            notes: String::new(),
            rating: 0,
            color: "#3B82F6".into(),
        })
    });
    id
}

/// Move row `from` to row `to` in the UNDERLYING list. The list hands us DISPLAY indices, which
/// differ from storage order whenever a finished item has floated to the top — so both ends are
/// resolved back to ids before anything moves.
pub(crate) fn move_row(from: usize, to: usize) {
    let display = ordered();
    let (Some(a), Some(b)) = (display.get(from), display.get(to)) else {
        return;
    };
    let (a, b) = (a.id, b.id);
    items().restructure("move", Op::Move, a as u64, |k| {
        let (Some(i), Some(j)) = (
            k.items().iter().position(|x| x.id == a),
            k.items().iter().position(|x| x.id == b),
        ) else {
            return;
        };
        k.move_item(i, j);
    });
}

/// Today as `YYYY-MM-DD`, via the date piece's own calendar so the app carries no date crate.
fn today_iso() -> String {
    let d = day_piece_datetime::DayDate::today();
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// The first-launch list. Enough rows to make scrolling, reordering, and recycling real —
/// a ten-row list proves nothing about a list widget.
fn seed() -> Vec<Item> {
    let base = day_piece_datetime::DayDate::today().to_epoch_days();
    (1..=SEED_COUNT)
        .map(|n| {
            let d = day_piece_datetime::DayDate::from_epoch_days(base + (n as i64 % 30) - 15);
            Item {
                id: n,
                name: format!("Item {n}"),
                count: (n as i64 % 9) + 1,
                date: format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
                kind: (n as usize) % KINDS.len(),
                done: n % 4 == 0,
                notes: String::new(),
                rating: (n as usize) % 6,
                color: ["#3B82F6", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6"][(n as usize) % 5]
                    .into(),
            }
        })
        .collect()
}
