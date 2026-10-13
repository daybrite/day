//! The document and the small amount of state shared by the list and editor.
//!
//! Reactive fields and keyed identity: https://daybrite.dev/docs/internal/model/

use crate::Section;
use day::model::Op;
use day::prelude::*;
use serde::{Deserialize, Serialize};

/// Persisted item data; `Observable` generates the field bindings used by the editor.
#[derive(Observable, Clone, Serialize, Deserialize)]
pub(crate) struct Item {
    /// Stable identity, independent of the item's position in the list.
    #[obs(key)]
    pub id: u64,
    /// User-entered text; an empty name displays the localized Untitled label.
    pub name: String,
    /// Completion state shared by the editor toggle and list checkmark.
    pub done: bool,
}

const STORE_KEY: &str = "app.items";

/// State shared within one window; `Copy` duplicates handles, not the document.
/// `Ambient` supplies it to descendant pages: https://daybrite.dev/docs/internal/state/
#[derive(Clone, Copy)]
pub(crate) struct Scene {
    /// Items in display order; structural edits go through `restructure`.
    pub items: Store<Keyed<Item>>,
    /// Two-way selection for the navigation host.
    pub section: Signal<Section>,
    /// Selected item ID, so reordering does not change which item is edited.
    pub selected: Signal<Option<u64>>,
    /// Row to reveal after adding an item.
    pub scroll_to: Signal<Option<usize>>,
    /// Shared with navigation so the native Back action can close the editor.
    pub detail_open: Signal<bool>,
}

impl Ambient for Scene {
    fn create() -> Self {
        let items = day::prefs::get(STORE_KEY)
            .and_then(|json| match serde_json::from_str::<Vec<Item>>(&json) {
                Ok(items) => Some(items),
                Err(error) => {
                    warn!("Could not load items: {error}");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            items: Store::new(Keyed::new(items)),
            section: Signal::new(Section::Welcome),
            selected: Signal::new(None),
            scroll_to: Signal::new(None),
            detail_open: Signal::new(false),
        }
    }
}

impl Scene {
    /// Save on document changes; preferences suit this small example, not large documents.
    /// Storage choices: https://daybrite.dev/docs/guide-storage/
    pub(crate) fn persist(self) {
        // Track the document once, then read it untracked in the saving callback.
        watch(
            move || self.items.with(|_| ()),
            move |_, _| match self
                .items
                .with_untracked(|items| serde_json::to_string(items.items()))
            {
                Ok(json) => {
                    // Day routes logs to the platform console: https://daybrite.dev/docs/logging/
                    if day::prefs::set(STORE_KEY, &json) {
                        debug!("Saved items to preferences");
                    } else {
                        warn!("Could not save items to preferences");
                    }
                }
                Err(error) => warn!("Could not save items: {error}"),
            },
        );
    }

    /// Select by stable ID and reveal the editor in the adaptive navigation host.
    pub(crate) fn open(self, id: u64) {
        self.selected.set(Some(id));
        self.detail_open.set(true);
    }

    /// Append an empty item, open its editor, and request that the list scroll to it.
    pub(crate) fn new_item(self) {
        let next = self.items.with_untracked(|items| {
            items
                .items()
                .iter()
                .map(|item| item.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .filter(|id| *id < (1 << 63)) // Day reserves the high bit for internal keys.
        });
        let Some(id) = next else {
            warn!("Cannot add an item: no more item IDs");
            return;
        };
        self.items.restructure("add", Op::Insert, id, |items| {
            items.push(Item {
                id,
                name: String::new(),
                done: false,
            });
        });
        info!("Created item {id}");
        self.section.set(Section::Items);
        self.open(id);
        self.scroll_to.set(Some(
            self.items.with_untracked(|items| items.items().len() - 1),
        ));
    }

    /// Delete by ID through one path shared by menus, toolbar, and native list gestures.
    pub(crate) fn remove(self, id: u64) {
        // All deletion paths close the editor when its item disappears.
        if self.selected.get_untracked() == Some(id) {
            self.selected.set(None);
            self.detail_open.set(false);
        }
        self.items.restructure("remove", Op::Delete, id, |items| {
            if items.remove(id).is_some() {
                info!("Deleted item {id}");
            }
        });
    }

    /// Native reorder indices match storage order; selection stays attached to the item ID.
    pub(crate) fn move_item(self, from: usize, to: usize) {
        let id = self.items.with_untracked(|items| {
            if from == to || to >= items.len() {
                return None;
            }
            items.items().get(from).map(|item| item.id)
        });
        let Some(id) = id else { return };
        self.items.restructure("move", Op::Move, id, |items| {
            items.move_item(from, to);
        });
        info!("Moved item {id} from row {from} to {to}");
    }

    /// Delete the current item; commands with no selection are harmless.
    pub(crate) fn delete_selected(self) {
        if let Some(id) = self.selected.get_untracked() {
            self.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        Scene {
            items: Store::new(Keyed::new(Vec::new())),
            section: Signal::new(Section::Welcome),
            selected: Signal::new(None),
            scroll_to: Signal::new(None),
            detail_open: Signal::new(false),
        }
    }

    #[test]
    fn deleting_another_item_preserves_the_editor() {
        let scene = scene();
        scene.new_item();
        let first = scene.selected.get().unwrap();
        scene.new_item();
        let second = scene.selected.get().unwrap();
        scene.remove(first);
        assert_eq!(scene.items.keys(), vec![second]);
        assert_eq!(scene.selected.get(), Some(second));
        assert!(scene.detail_open.get());
    }

    #[test]
    fn deleting_the_open_item_clears_selection_and_closes_the_editor() {
        let scene = scene();
        scene.new_item();
        scene.remove(scene.selected.get().unwrap());
        assert!(scene.items.keys().is_empty());
        assert_eq!(scene.selected.get(), None);
        assert!(!scene.detail_open.get());
        scene.delete_selected(); // No selection is a harmless no-op.
    }

    #[test]
    fn reordering_preserves_selection_and_item_bindings() {
        let scene = scene();
        for _ in 0..3 {
            scene.new_item();
        }
        let selected = scene.selected.get().unwrap();
        let item = scene.items.elem(selected);
        scene.move_item(2, 0);
        assert_eq!(scene.items.keys(), vec![3, 1, 2]);
        assert_eq!(scene.selected.get(), Some(selected));
        assert!(scene.detail_open.get());
        item.name().write("Moved fixture".into());
        assert_eq!(scene.items.elem(selected).name().read(), "Moved fixture");
        scene.move_item(0, 2);
        scene.move_item(1, 1);
        scene.move_item(99, 0);
        scene.move_item(0, 99);
        assert_eq!(scene.items.keys(), vec![1, 2, 3]);
    }

    #[test]
    fn new_items_do_not_use_the_reserved_key_range() {
        let scene = scene();
        let id = (1 << 63) - 1;
        scene.items.restructure("fixture", Op::Insert, id, |items| {
            items.push(Item {
                id,
                name: "Fixture".into(),
                done: false,
            });
        });
        scene.new_item();
        assert_eq!(scene.items.keys(), vec![id]);
        assert_eq!(scene.selected.get(), None);
    }

    // These platforms provide a preferences store in a plain test process.
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn edits_and_an_empty_document_survive_reload() {
        use day::reactive::batch;

        // The test executable has its own preferences domain. Restore any previous run's value.
        struct Restore(Option<String>, Scope);
        impl Drop for Restore {
            fn drop(&mut self) {
                self.1.dispose();
                if let Some(json) = &self.0 {
                    day::prefs::set(STORE_KEY, json);
                } else {
                    day::prefs::remove(STORE_KEY);
                }
            }
        }
        let scope = Scope::child();
        let _restore = Restore(day::prefs::get(STORE_KEY), scope);
        assert!(day::prefs::set(STORE_KEY, "[]"));
        scope.enter(|| {
            let scene = Scene::create();
            assert!(scene.items.keys().is_empty());
            scene.persist();
            batch(|| scene.new_item());
            let id = scene.selected.get().unwrap();
            batch(|| {
                scene.items.elem(id).name().write("Saved fixture".into());
                scene.items.elem(id).done().write(true);
            });
            let reloaded = Scene::create();
            assert_eq!(reloaded.items.keys(), vec![id]);
            assert_eq!(reloaded.items.elem(id).name().read(), "Saved fixture");
            assert!(reloaded.items.elem(id).done().read());
            batch(|| scene.new_item());
            let second = scene.selected.get().unwrap();
            batch(|| scene.move_item(1, 0));
            assert_eq!(Scene::create().items.keys(), vec![second, id]);
            batch(|| scene.remove(id));
            batch(|| scene.delete_selected());
            assert_eq!(day::prefs::get(STORE_KEY).as_deref(), Some("[]"));
            assert!(Scene::create().items.keys().is_empty());
        });
    }
}
