//! The app's domain object, and the per-window `Scene` that holds one.
//!
//! Everything a window shows is a projection of its `Scene`: the store, the filter, the
//! selection. `#[derive(Observable)]` makes every `Item` field a two-way binding, so the editor
//! writes the store directly and each edit wakes only the readers of that field
//! (https://daybrite.dev/docs/reactivity).
//!
//! Persistence is one JSON blob under a `day::prefs` key, saved by one subscription in
//! [`Scene::persist`]. It is the part you will most likely replace first, and nothing above
//! this file cares.

use crate::Section;
use day::model::Op;
use day::prelude::*;
use serde::{Deserialize, Serialize};

/// One row. `id` is the stable key: the list, the routes, and `elem(id)` all address a row by
/// it, never by index.
#[derive(Observable, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
    /// ISO-8601, kept as a string so the JSON stays readable.
    pub date: String,
    /// Index into `KINDS`.
    pub kind: usize,
    pub done: bool,
    pub notes: String,
    pub rating: usize,
    /// `#RRGGBB`.
    pub color: String,
}

/// The kind picker's options, as Fluent keys so they localize.
pub(crate) const KINDS: [&str; 3] = ["item_kind_note", "item_kind_task", "item_kind_idea"];

const STORE_KEY: &str = "app.items";
const SHOW_DONE_KEY: &str = "app.show_done";
const SEED_COUNT: u32 = 100;

/// Everything one window owns: its document, its filter, and where it is looking.
///
/// `Copy`, because every field is a handle, so a `Scene` goes into any closure without an
/// `Rc`. The window shell provides one with `Scene::scoped`, pages read it back with
/// `Scene::ambient()`, and the menu bar reaches the front window's with `Scene::focused()`.
#[derive(Clone, Copy)]
pub(crate) struct Scene {
    /// This window's document.
    pub items: Store<Keyed<Item>>,
    /// Whether finished items are listed. A view preference, but persisted anyway: a filter you
    /// have to reapply every launch is one you stop using.
    pub show_done: Signal<bool>,
    /// The section the navigation is showing.
    pub section: Signal<Section>,
    /// The row the editor is editing; the one field the list and the editor share.
    pub selected: Signal<Option<u32>>,
    /// A row the list should scroll to, cleared once it has.
    pub scroll_to: Signal<Option<usize>>,
    /// Whether the editor is showing, on the shapes that show one pane at a time. The nav host
    /// drives it both ways through `.detail_visible` in `lib.rs`.
    pub detail_open: Signal<bool>,
}

impl Ambient for Scene {
    /// A window's state, seeded from the last save or a fresh sample list. A second window
    /// starts as a copy of the saved document and diverges from there.
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
    /// Save the document and filter to `day::prefs` on every change.
    ///
    /// Only the primary window installs this; if every window saved, the last one touched would
    /// win. To have all windows share one document instead, swap `Scene::scoped` for
    /// `Scene::app` in `window_shell`.
    pub(crate) fn persist(self) {
        // On the root scope, so closing the window that installed these does not stop saving.
        Scope::root().enter(move || {
            watch(
                move || self.show_done.get(),
                |v, _| {
                    day::prefs::set(SHOW_DONE_KEY, if *v { "1" } else { "0" });
                },
            );
            // One coarse subscription: a whole-store read wakes on any write, and the version
            // number is the cheap value `watch` diffs.
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

    /// Row keys in display order: finished ones first, or hidden when the filter says so, each
    /// group in the user's own order.
    ///
    /// Sorting here keeps the list, the editor, and the reorder indices agreeing on what "row 3"
    /// means. It reads only the shape, the filter, and each row's `done`, so renaming an item
    /// never reloads the list.
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
        // Stable, so the user's order survives inside each group.
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
        // The same accessor the editor binds, so every writer takes one path.
        self.items.elem(id as u64).done().update(|d| *d = !*d);
    }

    /// Move a row by its display indices. Those differ from storage order once a finished item
    /// has floated up, so both ends resolve to keys first.
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

    // --- the commands the toolbar, the menu bar, and the row menus share -------------------

    /// Open `id` in the editor. Where it appears is the nav host's call: beside the list on a
    /// wide window, pushed over it on a narrow one.
    pub(crate) fn open(self, id: u32) {
        self.selected.set(Some(id));
        self.detail_open.set(true);
    }

    /// Close the editor for a cleared selection. `detail_open` stays put, so a pushed phone
    /// page is not popped from under the user.
    pub(crate) fn clear_selection(self) {
        self.selected.set(None);
    }

    /// Create an item and open it, so the row you just made is the one you are typing into.
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
        // A hundred rows in, a new row lands off screen; scroll to its display index.
        self.scroll_to
            .set(self.ordered_keys().iter().position(|k| *k == id as u64));
    }

    pub(crate) fn delete_selected(self) {
        if let Some(id) = self.selected.get_untracked() {
            self.remove(id);
            self.selected.set(None);
            // Nothing left to edit; the shapes that pushed the editor pop back.
            self.detail_open.set(false);
        }
    }

    pub(crate) fn done_selected(self) {
        if let Some(id) = self.selected.get_untracked() {
            self.toggle_done(id);
        }
    }
}

/// Today as `YYYY-MM-DD`, via the date piece's calendar, so the app needs no date crate.
fn today_iso() -> String {
    let d = day_piece_datetime::DayDate::today();
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// The first-launch list. A hundred rows, enough to make scrolling and reordering real.
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
