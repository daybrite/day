//! One observable store every page projects (https://daybrite.dev/docs/model); persisted as a
//! single JSON blob in `day::prefs` (the first thing you would swap for a real database).

use day::model::Op;
use day::prelude::*;
use serde::{Deserialize, Serialize};

/// One row; `id` is the stable `#[obs(key)]` the list, routes, and `elem(id)` handles address.
#[derive(Observable, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
    /// ISO-8601 (`YYYY-MM-DD`).
    pub date: String,
    /// Index into `KINDS`.
    pub kind: usize,
    pub done: bool,
    pub notes: String,
    pub rating: usize,
    /// `#RRGGBB`.
    pub color: String,
}

/// The `kind` picker's options: Fluent keys, so they localize.
pub(crate) const KINDS: [&str; 3] = ["item_kind_note", "item_kind_task", "item_kind_idea"];

const STORE_KEY: &str = "app.items";
const SHOW_DONE_KEY: &str = "app.show_done";
const SEED_COUNT: u32 = 100;

thread_local! {
    // Process-lifetime, like `Signal::global`: an app-wide store must outlive the scope that
    // first touches it (https://daybrite.dev/docs/model).
    static ITEMS: Store<Keyed<Item>> = Store::new(Keyed::default());
    /// Whether finished items are listed; persisted like any other preference.
    static SHOW_DONE: Signal<bool> = Signal::global(true);
}

/// The one store every page reads.
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
    // One watch persists the filter, wherever it is flipped from.
    watch(
        move || show_done().get(),
        |v, _| {
            day::prefs::set(SHOW_DONE_KEY, if *v { "1" } else { "0" });
        },
    );
    // One coarse subscription persists the list: a whole-store read wakes for any change.
    let store = items();
    watch(
        move || {
            store.with(|_| {});
            store.version()
        },
        move |_, _| save(),
    );
}

/// Registered once in [`load`], so no write site can forget to persist.
fn save() {
    let json = items().with_untracked(|k| serde_json::to_string(k.items()));
    if let Ok(json) = json {
        day::prefs::set(STORE_KEY, &json);
    }
}

/// Row keys as the list shows them: unfinished first, finished optionally hidden. A projection
/// of keys only; a rename never re-runs it (https://daybrite.dev/docs/list).
pub(crate) fn ordered_keys() -> Vec<u64> {
    let show = show_done().get();
    let store = items();
    let mut keys: Vec<(u64, bool)> = store
        .keys()
        .into_iter()
        .filter_map(|k| {
            let done = store.elem(k).done().with(|d| d.copied().unwrap_or(false));
            (show || !done).then_some((k, done))
        })
        .collect();
    // Stable sort: the user's own order survives inside each group.
    keys.sort_by_key(|(_, done)| !done);
    keys.into_iter().map(|(k, _)| k).collect()
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
    items().elem(id as u64).done().update(|d| *d = !*d);
}

/// Append a fresh row and answer its id.
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

/// Move row `from` to `to`: display indices, resolved back to keys before anything moves.
pub(crate) fn move_row(from: usize, to: usize) {
    let display = ordered_keys();
    let (Some(&a), Some(&b)) = (display.get(from), display.get(to)) else {
        return;
    };
    items().restructure("move", Op::Move, a, |k| {
        let (Some(i), Some(j)) = (
            k.items().iter().position(|x| x.id as u64 == a),
            k.items().iter().position(|x| x.id as u64 == b),
        ) else {
            return;
        };
        k.move_item(i, j);
    });
}

/// Today as `YYYY-MM-DD`, via the date piece so the app carries no date crate.
fn today_iso() -> String {
    let d = day_piece_datetime::DayDate::today();
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// The first-launch list: enough rows to make scrolling, reordering, and recycling real.
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
