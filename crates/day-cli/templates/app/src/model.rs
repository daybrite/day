//! The items, and the state the list and the editor share: https://daybrite.dev/docs/reactivity

use crate::{Section, res};
use day::prelude::*;
use day_piece_settings::prelude::Pref;
use serde::{Deserialize, Serialize};

/// One item; `Observable` generates the field bindings the editor uses.
#[derive(Observable, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Item {
    /// Stable identity, independent of the item's place in the list.
    #[obs(key)]
    pub id: u64,
    pub name: String,
    pub done: bool,
    pub kind: Kind,
    pub tint: Tint,
}

/// What an item is, shown by its icon.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub(crate) enum Kind {
    #[default]
    Note,
    Idea,
    Todo,
}

impl Kind {
    /// Each kind with its title, as a picker lists them.
    pub(crate) fn options() -> [(Kind, LocalizedText); 3] {
        [
            (Kind::Note, res::str::kind_note()),
            (Kind::Idea, res::str::kind_idea()),
            (Kind::Todo, res::str::kind_todo()),
        ]
    }

    /// The kind's icon, a vector under `resource/vectors/`.
    pub(crate) fn icon(self) -> VectorName {
        match self {
            Kind::Note => res::vectors::kind_note,
            Kind::Idea => res::vectors::kind_idea,
            Kind::Todo => res::vectors::kind_todo,
        }
    }
}

/// The color of an item's icon.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub(crate) enum Tint {
    #[default]
    Blue,
    Green,
    Orange,
    Purple,
}

impl Tint {
    /// Each tint with its title, as a picker lists them.
    pub(crate) fn options() -> [(Tint, LocalizedText); 4] {
        [
            (Tint::Blue, res::str::tint_blue()),
            (Tint::Green, res::str::tint_green()),
            (Tint::Orange, res::str::tint_orange()),
            (Tint::Purple, res::str::tint_purple()),
        ]
    }

    pub(crate) fn color(self) -> Color {
        Color::hex(match self {
            Tint::Blue => 0x3478F6,
            Tint::Green => 0x30B355,
            Tint::Orange => 0xF2910A,
            Tint::Purple => 0xA550D6,
        })
    }
}

/// One of `options` at random, so new items differ at a glance. Each `RandomState` hashes with
/// fresh keys, which is all the randomness this needs.
fn any<T: Copy, L>(options: &[(T, L)]) -> T {
    use std::hash::BuildHasher;
    let n = std::collections::hash_map::RandomState::new().hash_one(());
    options[n as usize % options.len()].0
}

/// An app setting, shown in Settings: https://daybrite.dev/docs/guide-storage
pub(crate) static SHOW_DONE: Pref<bool> = Pref::new("app.show_done", || true);

/// One window's state. `Copy` copies the handles, not the items.
#[derive(Clone, Copy)]
pub(crate) struct Scene {
    /// The items, in display order.
    pub items: Store<Keyed<Item>>,
    /// The section navigation shows.
    pub section: Signal<Section>,
    /// The key of the item the editor shows.
    pub selected: Signal<Option<u64>>,
    /// The search field's text.
    pub query: Signal<String>,
}

const ITEMS_KEY: &str = "app.items";

impl Ambient for Scene {
    fn create() -> Self {
        let items = Store::new(Keyed::default());
        // The first launch starts with one item to edit; later ones with the saved list.
        if !day::prefs::contains(ITEMS_KEY) {
            items.push(Item {
                id: 1,
                ..Default::default()
            });
        }
        // Load the saved items and save every change: https://daybrite.dev/docs/guide-storage
        day::prefs::bind_json(ITEMS_KEY, items);
        Self {
            items,
            section: Signal::new(Section::Welcome),
            selected: Signal::new(None),
            query: Signal::new(String::new()),
        }
    }
}

impl Scene {
    /// The keys the list shows: the items matching the search, without completed ones when
    /// Settings hides them. A tracked read, so the list follows every input.
    pub(crate) fn visible_keys(self) -> Vec<u64> {
        let query = self.query.get().to_lowercase();
        let show_done = SHOW_DONE.signal().get();
        let shown = |id: &u64| {
            let item = self.items.elem(*id);
            (show_done || !item.done().read()) && item.name().read().to_lowercase().contains(&query)
        };
        self.items.keys().into_iter().filter(shown).collect()
    }

    /// Add an empty item and open it in the editor.
    pub(crate) fn new_item(self) {
        let id = self.items.next_key();
        self.items.push(Item {
            id,
            kind: any(&Kind::options()),
            tint: any(&Tint::options()),
            ..Default::default()
        });
        // Day routes logs to the platform console: https://daybrite.dev/docs/logging
        info!("Created item {id}");
        self.query.set(String::new());
        self.section.set(Section::Items);
        self.selected.set(Some(id));
    }

    /// Delete an item. The menus, the toolbar and the list's gestures all come here.
    pub(crate) fn remove(self, id: u64) {
        if self.selected.get_untracked() == Some(id) {
            self.selected.set(None);
        }
        if self.items.remove(id) {
            info!("Deleted item {id}");
        }
    }

    /// The key in row `row` of the list, as its gestures report rows.
    pub(crate) fn key_at(self, row: usize) -> Option<u64> {
        day::reactive::untrack(|| self.visible_keys().get(row).copied())
    }

    /// Delete the selected item, if there is one.
    pub(crate) fn delete_selected(self) {
        if let Some(id) = self.selected.get_untracked() {
            self.remove(id);
        }
    }

    /// Mark an item done, or not done again.
    pub(crate) fn toggle_done(self, id: u64) {
        let done = self.items.elem(id).done();
        done.write(!done.peek());
    }

    /// The number of items still to do, for the Items badge; empty when there are none.
    pub(crate) fn open_count(self) -> String {
        let keys = self.items.keys();
        let open = keys
            .iter()
            .filter(|id| !self.items.elem(**id).done().read());
        match open.count() {
            0 => String::new(),
            n => n.to_string(),
        }
    }

    /// The editor's title: the selected item's name, read reactively so it follows edits.
    pub(crate) fn selected_title(self) -> String {
        match self.selected.get() {
            Some(id) => display_name(self.items.elem(id).name().read()),
            None => res::str::nav_items().format(),
        }
    }
}

/// An item's name as the UI shows it: the localized Untitled for an empty one.
pub(crate) fn display_name(name: String) -> String {
    if name.is_empty() {
        res::str::item_untitled().format()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_launch_starts_with_one_item() {
        let scene = Scene::create();
        assert_eq!(scene.items.keys(), vec![1]);
        scene.remove(1);
        day::reactive::flush_sync();
        assert!(
            Scene::create().items.keys().is_empty(),
            "an emptied list stays empty"
        );
    }

    #[test]
    fn deleting_the_open_item_clears_the_selection() {
        let scene = Scene::create();
        scene.new_item();
        scene.new_item();
        let [_, first, second] = scene.items.keys()[..] else {
            panic!("the first item and two more")
        };
        assert_eq!(scene.selected.get(), Some(second));
        scene.remove(first);
        assert_eq!(scene.selected.get(), Some(second));
        scene.delete_selected();
        assert_eq!(scene.items.keys(), vec![1]);
        assert_eq!(scene.selected.get(), None);
    }

    #[test]
    fn items_survive_a_relaunch() {
        let scene = Scene::create();
        scene.new_item();
        let id = scene.selected.get().expect("the new item is selected");
        scene.items.elem(id).name().write("Saved".into());
        // The save runs at the end of the app's turn; a test ends the turn itself.
        day::reactive::flush_sync();
        let relaunched = Scene::create();
        assert_eq!(relaunched.items.keys(), vec![1, id]);
        assert_eq!(relaunched.items.elem(id).name().read(), "Saved");
    }

    #[test]
    fn the_search_and_the_done_setting_filter_the_list() {
        let scene = Scene::create();
        for name in ["Milk", "Bread"] {
            scene.new_item();
            let id = scene.selected.get().expect("the new item is selected");
            scene.items.elem(id).name().write(name.into());
        }
        let [first, milk, bread] = scene.items.keys()[..] else {
            panic!("the first item and two more")
        };
        scene.query.set("mil".into());
        assert_eq!(scene.visible_keys(), vec![milk]);
        scene.query.set(String::new());
        scene.toggle_done(milk);
        SHOW_DONE.signal().set(false);
        assert_eq!(scene.visible_keys(), vec![first, bread]);
        assert_eq!(scene.open_count(), "2");
    }
}
