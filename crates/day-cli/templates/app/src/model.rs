//! The app's domain object and its store.
//!
//! Everything the UI shows is a projection of ONE signal — `items()` — so no page owns state and
//! no page has to tell another that something changed. Editing a field writes the signal; the
//! list, the row badge, and the editor all follow because they read it
//! (https://daybrite.dev/docs/state).
//!
//! Persistence is deliberately boring: the whole list is one JSON blob under one `day::prefs`
//! key. That is enough for a starter, survives an Android process death (prefs is disk-backed),
//! and is the piece you are most likely to replace first — swap `load`/`flush` for your database
//! and nothing above this file changes.
//!
//! Writes are DEFERRED. The signal is the live copy and every edit updates it immediately; the
//! blob is re-serialized only when the app is leaving (background, resign, terminate). Saving on
//! each mutation means serializing the whole list on every keystroke, which is work proportional
//! to the list for a change of one character.

use day::prelude::*;
use serde::{Deserialize, Serialize};

/// One row. `id` is stable across reorders and edits, which is what the list keys on and what a
/// route segment carries — never the index, which changes the moment a row moves.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Item {
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
    // `Signal::global`, NOT `Signal::new`. A lazily-initialized global is created inside whatever
    // scope first touches it — a pushed page, a `when` arm, a tab — and dies with that scope, so
    // every later read panics. `global` allocates it in the root scope instead, which is what an
    // app-wide store needs (https://daybrite.dev/docs/state).
    static ITEMS: Signal<Vec<Item>> = Signal::global(Vec::new());
    /// Whether finished items are listed at all. A VIEW preference rather than data, but it is
    /// persisted all the same — a filter the user has to re-apply on every launch is a filter
    /// they stop using.
    static SHOW_DONE: Signal<bool> = Signal::global(true);
    /// Whether the list has changed since it was last written.
    static DIRTY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The one signal every page reads. Tracked, so a list rebinds and an editor field updates
/// whenever anything writes it.
pub(crate) fn items() -> Signal<Vec<Item>> {
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
    items().set(saved.unwrap_or_else(seed));
    show_done().set(
        day::prefs::get(SHOW_DONE_KEY)
            .map(|v| v != "0")
            .unwrap_or(true),
    );
    // Persist the filter on every change, so it is one `watch` rather than a write at each of
    // the three places that can flip it (the toolbar, the menu, and a restored launch). One
    // character, not a list — cheap enough to write as it happens.
    watch(
        move || show_done().get(),
        |v, _| {
            day::prefs::set(SHOW_DONE_KEY, if *v { "1" } else { "0" });
        },
    );
    // The list itself is written when the app is on its way out. `WillResignActive` covers the
    // desktops, which never background; the two mobile phases cover a process the OS may kill
    // without a further word. Registering all four is deliberate — a phase a backend does not
    // deliver simply never fires (https://daybrite.dev/docs/lifecycle).
    for phase in [
        Lifecycle::WillResignActive,
        Lifecycle::DidEnterBackground,
        Lifecycle::WillTerminate,
        Lifecycle::DidReceiveMemoryWarning,
    ] {
        on_lifecycle(phase, flush);
    }
}

/// Write the list out if anything has changed since the last write.
///
/// Public because a real app has more moments worth saving at than a starter does — a sync
/// button, a document close, an autosave timer — and they all want this one call.
pub(crate) fn flush() {
    if !DIRTY.with(|d| d.replace(false)) {
        return;
    }
    if let Ok(json) = serde_json::to_string(&items().get_untracked()) {
        day::prefs::set(STORE_KEY, &json);
    }
}

/// Apply a change to the list and mark it for the next [`flush`].
///
/// Every mutation below funnels through here, so nothing can change the list without the app
/// knowing it has to be written.
pub(crate) fn update(f: impl FnOnce(&mut Vec<Item>)) {
    items().update(f);
    DIRTY.with(|d| d.set(true));
}

/// Rows as the list shows them: finished ones first, optionally hidden altogether, each group
/// keeping the user's own order.
///
/// Sorting and filtering HERE rather than in the page is what keeps the list, the editor's
/// neighbors, and the reorder indices agreeing on what "row 3" means — a page that filtered its
/// own copy would hand `on_reorder` an index the model could not resolve.
pub(crate) fn ordered() -> Vec<Item> {
    let show = show_done().get();
    let mut v: Vec<Item> = items()
        .get()
        .into_iter()
        .filter(|i| show || !i.done)
        .collect();
    // A STABLE sort, so the user's own order survives inside each group.
    v.sort_by_key(|i| !i.done);
    v
}

pub(crate) fn find(id: u32) -> Option<Item> {
    items().get().into_iter().find(|i| i.id == id)
}

/// Edit one item in place by id.
pub(crate) fn edit(id: u32, f: impl FnOnce(&mut Item)) {
    update(|v| {
        if let Some(it) = v.iter_mut().find(|i| i.id == id) {
            f(it);
        }
    });
}

pub(crate) fn remove(id: u32) {
    update(|v| v.retain(|i| i.id != id));
}

pub(crate) fn toggle_done(id: u32) {
    edit(id, |i| i.done = !i.done);
}

/// Append a fresh row and answer its id, so the caller can drill straight into its editor.
pub(crate) fn add() -> u32 {
    let id = items().get().iter().map(|i| i.id).max().unwrap_or(0) + 1;
    update(|v| {
        v.push(Item {
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
/// differ from storage order whenever a favorite has floated to the top — so both ends are
/// resolved back to ids before anything moves.
pub(crate) fn move_row(from: usize, to: usize) {
    let display = ordered();
    let (Some(a), Some(b)) = (display.get(from), display.get(to)) else {
        return;
    };
    let (a, b) = (a.id, b.id);
    update(|v| {
        let (Some(i), Some(j)) = (
            v.iter().position(|x| x.id == a),
            v.iter().position(|x| x.id == b),
        ) else {
            return;
        };
        let it = v.remove(i);
        v.insert(j, it);
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
