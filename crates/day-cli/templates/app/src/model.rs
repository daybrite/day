//! The app's domain object, and the per-window state that holds one.
//!
//! Everything a window shows is a projection of ONE [`Scene`] — its store, its filter, its
//! selection — so no page owns state and no page has to tell another that something changed.
//! The store is observable PER PROPERTY (https://daybrite.dev/docs/model):
//! `#[derive(Observable)]` turns every field of [`Item`] into a typed accessor, and
//! `scene.items.elem(id).name()` is a two-way binding a `text_field` takes directly — the editor
//! needs no draft signals and no write-back plumbing. Editing a name wakes exactly the readers of
//! that name; the list re-runs only when the collection's SHAPE changes.
//!
//! A `Scene` is per WINDOW, not per app (https://daybrite.dev/docs/state). That is the whole
//! reason it is a struct rather than a handful of `thread_local!` globals: File ▸ New Window
//! builds the same shell a second time, and a global would hand both windows one selection.
//!
//! Persistence is deliberately boring: the whole list is one JSON blob under one `day::prefs`
//! key, written by one coarse subscription in [`Scene::persist`]. That is enough for a starter,
//! survives an Android process death (prefs is disk-backed), and is the piece you are most
//! likely to replace first — swap it for your database and nothing above this file changes.

use crate::Section;
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

/// Everything ONE WINDOW owns: its document, its filter, and where it is looking
/// (https://daybrite.dev/docs/state).
///
/// `Copy`, because every field is a HANDLE — `Store` and `Signal` are both pointer-sized and
/// both cheap — so a `Scene` rides into an event handler or a page function with no `Rc` and no
/// `clone()`. That is what makes passing state around cost as little as reaching for a global.
///
/// [`Ambient`] is how pieces get one without threading it through every signature: the window's
/// shell provides it once with `Scene::scoped(…)` and anything built below reads it back with
/// `Scene::ambient()` — SwiftUI's `@EnvironmentObject`. The app-wide menu bar, which belongs to
/// no window, reaches the front one's with `Scene::focused()`.
#[derive(Clone, Copy)]
pub(crate) struct Scene {
    /// This window's document.
    pub items: Store<Keyed<Item>>,
    /// Whether finished items are listed at all. A VIEW preference rather than data, but the
    /// primary window persists it all the same — a filter the user has to re-apply on every
    /// launch is a filter they stop using.
    pub show_done: Signal<bool>,
    /// Which section the navigation is showing.
    pub section: Signal<Section>,
    /// The row the editor is editing. The seam between the list and the editor.
    pub selected: Signal<Option<u32>>,
    /// The row the list should scroll into view; cleared once it has.
    pub scroll_to: Signal<Option<usize>>,
    /// Whether the EDITOR is showing. A three-column window always shows it beside the list; a
    /// shape that shows one pane at a time pushes it over the list and pops back, and this is
    /// the signal the navigation host drives in both directions — `selector(…).detail_visible`
    /// in `lib.rs` (https://daybrite.dev/docs/navigation).
    pub detail_open: Signal<bool>,
}

impl Ambient for Scene {
    /// A window's state, seeded from what was last saved (or from a fresh sample list on first
    /// launch). Called once per window: a second window starts as a COPY of the saved document
    /// and then diverges, because each window owns its own store.
    fn create() -> Self {
        let saved = day::prefs::get(STORE_KEY)
            .and_then(|s| serde_json::from_str::<Vec<Item>>(&s).ok())
            .filter(|v| !v.is_empty());
        Scene {
            items: Store::new(Keyed::new(saved.unwrap_or_else(seed))),
            show_done: Signal::new(
                day::prefs::get(SHOW_DONE_KEY)
                    .map(|v| v != "0")
                    .unwrap_or(true),
            ),
            section: Signal::new(Section::Welcome),
            selected: Signal::new(None),
            scroll_to: Signal::new(None),
            detail_open: Signal::new(false),
        }
    }
}

impl Scene {
    /// Write this scene's document and filter back to `day::prefs` on every change.
    ///
    /// Installed by the PRIMARY window only (`window_shell` in lib.rs). Every window owns its
    /// own store, so letting each one save would make the last window touched the winner and
    /// the others' edits vanish on the next launch. If you would rather every window edit ONE
    /// shared document, replace `Scene::scoped` in `window_shell` with `Scene::app` — then
    /// there is a single scene, this runs once, and the question does not arise.
    pub(crate) fn persist(self) {
        // On the ROOT scope, not the window's: closing the window that installed these must not
        // stop the app saving (https://daybrite.dev/docs/state).
        Scope::root().enter(move || {
            watch(
                move || self.show_done.get(),
                |v, _| {
                    day::prefs::set(SHOW_DONE_KEY, if *v { "1" } else { "0" });
                },
            );
            // ONE coarse subscription instead of a save call at every write site. The tracked
            // whole-store read wakes for any field write, insert, delete or reorder — precision
            // is something a reader opts OUT of by reading coarsely — and the version number is
            // the cheap value `watch` diffs.
            let store = self.items;
            watch(
                move || {
                    store.with(|_| {});
                    store.version()
                },
                move |_, _| {
                    if let Ok(json) = store.with_untracked(|k| serde_json::to_string(k.items())) {
                        day::prefs::set(STORE_KEY, &json);
                    }
                },
            );
        });
    }

    /// Row KEYS as the list shows them: finished ones first, optionally hidden altogether, each
    /// group keeping the user's own order.
    ///
    /// Sorting and filtering HERE rather than in the page is what keeps the list, the editor's
    /// neighbors, and the reorder indices agreeing on what "row 3" means. And it is a projection
    /// of KEYS, not items: nothing is cloned, and its tracked reads are exactly the collection's
    /// shape, the filter flag, and each row's `done` — the only facts the ORDER depends on.
    /// Renaming an item cannot re-run it, so the list never reloads for a keystroke in the editor.
    pub(crate) fn ordered_keys(self) -> Vec<u64> {
        let show = self.show_done.get();
        let store = self.items;
        let mut keys: Vec<(u64, bool)> = store
            .keys()
            .into_iter()
            .filter_map(|k| {
                let done = store.elem(k).done().with(|d| d.copied().unwrap_or(false));
                (show || !done).then_some((k, done))
            })
            .collect();
        // A STABLE sort, so the user's own order survives inside each group.
        keys.sort_by_key(|(_, done)| !done);
        keys.into_iter().map(|(k, _)| k).collect()
    }

    pub(crate) fn find(self, id: u32) -> Option<Item> {
        self.items
            .with(|k| k.and_then(|k| k.get(id as u64).cloned()))
    }

    pub(crate) fn remove(self, id: u32) {
        self.items
            .restructure("remove", Op::Delete, id as u64, |k| {
                k.remove(id as u64);
            });
    }

    pub(crate) fn toggle_done(self, id: u32) {
        // A field write through the same accessor the editor binds — one path for every writer.
        self.items.elem(id as u64).done().update(|d| *d = !*d);
    }

    /// Move row `from` to row `to` in the UNDERLYING list. The list hands us DISPLAY indices,
    /// which differ from storage order whenever a finished item has floated to the top — so both
    /// ends are resolved back to keys before anything moves.
    pub(crate) fn move_row(self, from: usize, to: usize) {
        let display = self.ordered_keys();
        let (Some(&a), Some(&b)) = (display.get(from), display.get(to)) else {
            return;
        };
        self.items.restructure("move", Op::Move, a, |k| {
            let (Some(i), Some(j)) = (
                k.items().iter().position(|x| x.id as u64 == a),
                k.items().iter().position(|x| x.id as u64 == b),
            ) else {
                return;
            };
            k.move_item(i, j);
        });
    }

    // --- the commands the toolbar, the menu bar, and the row menus all run ------------------

    /// Open `id` in the editor — the one path every "show me this item" command takes. The
    /// layout is not this function's business: a wide window already has the editor beside the
    /// list, a narrow one pushes it, and the host decides which
    /// (https://daybrite.dev/docs/navigation).
    pub(crate) fn open(self, id: u32) {
        self.selected.set(Some(id));
        self.detail_open.set(true);
    }

    /// Create an item and open its editor straight away — the "new" flow every list app has,
    /// where the row you just made is the row you want to be typing into.
    pub(crate) fn new_item(self) {
        let id = self
            .items
            .with_untracked(|k| k.items().iter().map(|i| i.id).max().unwrap_or(0))
            + 1;
        self.items.restructure("add", Op::Insert, id as u64, |k| {
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
        self.open(id);
        // A hundred rows in, a new one lands off screen. Ask the list to bring it into view by
        // its DISPLAY index, which is not the order it was appended in — finished items float to
        // the top (https://daybrite.dev/docs/list).
        self.scroll_to
            .set(self.ordered_keys().iter().position(|k| *k == id as u64));
    }

    pub(crate) fn delete_selected(self) {
        if let Some(id) = self.selected.get_untracked() {
            self.remove(id);
            self.selected.set(None);
            // Nothing left to edit: on the shapes that pushed the editor, this pops back.
            self.detail_open.set(false);
        }
    }

    pub(crate) fn done_selected(self) {
        if let Some(id) = self.selected.get_untracked() {
            self.toggle_done(id);
        }
    }
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
