// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Wide keys: Uuid and string keys intern to path handles, `ModelId` types the surface, and
//! the granularity story is unchanged — one field write wakes one field's readers, whatever
//! the key's shape.

use day_macros::Observable;
use day_model::{Identified, Key, Keyed, ModelId, Op, Store, Uuid};
use day_reactive::Binding;

#[derive(Observable, Clone, Default, PartialEq, Debug)]
struct Card {
    #[obs(key)]
    id: Uuid,
    title: String,
    done: bool,
}

#[derive(Observable, Clone, Default, PartialEq, Debug)]
struct Page {
    #[obs(key)]
    slug: String,
    body: String,
}

#[derive(Observable, Clone, Default, PartialEq, Debug)]
struct Row {
    #[obs(key)]
    id: u32,
    n: i64,
}

fn card(id: Uuid, title: &str) -> Card {
    Card {
        id,
        title: title.into(),
        done: false,
    }
}

#[test]
fn uuid_keys_address_elements_and_stay_granular() {
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let store = Store::new(Keyed::new(vec![card(a, "alpha"), card(b, "beta")]));

    // Address by the raw Uuid, by a typed id, and by the handle — all the same row.
    assert_eq!(store.elem(a).title().peek(), "alpha");
    assert_eq!(store.elem(ModelId::<Card>::of(a)).title().peek(), "alpha");
    let handle = store.elem(a).key();
    assert_eq!(store.elem(handle).title().peek(), "alpha");

    // One field write announces one field of one row — same precision as integer keys.
    let (_, changes) = day_model::record_changes(|| {
        store.elem(a).title().write("edited".into());
    });
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].label, "title");
    assert_eq!(changes[0].components[1], handle, "the row's own handle");
    assert_eq!(
        store.elem(b).title().peek(),
        "beta",
        "the sibling row is untouched"
    );
}

#[test]
fn string_keys_work_end_to_end() {
    let store = Store::new(Keyed::new(vec![
        Page {
            slug: "welcome".into(),
            body: "hi".into(),
        },
        Page {
            slug: "faq".into(),
            body: "answers".into(),
        },
    ]));

    assert_eq!(store.elem("welcome").body().peek(), "hi");
    store.elem("faq").body().write("updated".into());
    assert_eq!(store.elem("faq").body().peek(), "updated");

    // The typed id round-trips to the natural key.
    let id: ModelId<Page> = "faq".into();
    assert_eq!(id.key().as_str(), Some("faq"));
    assert_eq!(store.elem(id).body().peek(), "updated");
}

#[test]
fn handles_are_stable_and_deduplicated() {
    let u = Uuid::now_v7();
    let h1 = ModelId::<Card>::of(u).handle();
    let h2 = ModelId::<Card>::of(u).handle();
    let h3 = Key::Uuid(u.as_u128()).handle();
    assert_eq!(h1, h2, "the same key always interns to the same handle");
    assert_eq!(h1, h3, "whatever door it arrives through");

    // Reverse lookup recovers the real key.
    assert_eq!(ModelId::<Card>::from_handle(h1).key().as_uuid(), Some(u));
    assert_eq!(Key::of_handle(h1), Some(Key::Uuid(u.as_u128())));
}

#[test]
fn integer_keys_are_their_own_handle() {
    // No interner involvement at all: the handle IS the key value.
    assert_eq!(ModelId::<Row>::of(42u32).handle(), 42);
    assert_eq!(Key::U64(7).handle(), 7);
    assert_eq!(Key::of_handle(7), Some(Key::U64(7)));

    let store = Store::new(Keyed::new(vec![Row { id: 5, n: 1 }]));
    // Literal, typed, and converted addressing agree.
    assert_eq!(store.elem(5).n().peek(), 1);
    assert_eq!(store.elem(5u64).n().peek(), 1);
    assert_eq!(store.elem(ModelId::<Row>::of(5usize)).n().peek(), 1);
}

#[test]
fn two_stores_with_the_same_key_stay_isolated() {
    let u = Uuid::now_v7();
    let a = Store::new(Keyed::new(vec![card(u, "in a")]));
    let b = Store::new(Keyed::new(vec![card(u, "in b")]));

    let (_, changes) = day_model::record_changes(|| {
        a.elem(u).title().write("edited in a".into());
    });
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].components[0],
        a.store_id(),
        "the change names store A"
    );
    assert_eq!(
        b.elem(u).title().peek(),
        "in b",
        "store B never heard of it"
    );
}

#[test]
fn restructure_and_merge_row_take_wide_ids() {
    let u = Uuid::now_v7();
    let store = Store::new(Keyed::new(vec![card(u, "first")]));

    let fresh = Uuid::now_v7();
    store.restructure("add", Op::Insert, fresh, |k| k.push(card(fresh, "second")));
    assert_eq!(store.keys().len(), 2);

    let mut edited = card(u, "merged");
    edited.done = true;
    assert!(store.merge_row(u, edited, &["title", "done"]));
    assert_eq!(store.elem(u).title().peek(), "merged");

    store.restructure("remove", Op::Delete, fresh, |k| {
        k.remove(ModelId::<Card>::of(fresh).handle());
    });
    assert_eq!(store.keys().len(), 1);
}

#[test]
fn ids_are_typed_keys_with_their_own_display() {
    let u = Uuid::now_v7();
    let id = ModelId::<Card>::of(u);
    // Debug carries the real key, not the opaque handle.
    assert!(format!("{id:?}").contains(&u.to_string()));
    assert_eq!(format!("{}", Key::U64(9)), "9");
    assert_eq!(format!("{}", Key::Str("faq".into())), "faq");

    let store = Store::new(Keyed::new(vec![card(u, "x")]));
    assert_eq!(store.elem(u).model_id(), id);
    assert_eq!(store.ids(), vec![id]);
    store.with_untracked(|k| {
        assert_eq!(
            k.items()[0].model_id(),
            id,
            "Identified::model_id agrees with the store"
        );
    });
}

#[test]
fn a_worker_thread_interns_the_same_handles() {
    // The interner is process-global, not thread-local: a background transaction's reindex
    // mints the SAME handles the main thread resolves — the divergence bug the path system's
    // components seam guards against cannot recur here.
    let u = Uuid::now_v7();
    let store = Store::new(Keyed::new(vec![card(u, "before")]));
    let main_handle = store.elem(u).key();

    let done = std::thread::spawn(move || {
        let mut tx = store.transact();
        let handle = tx.data().items()[0].handle();
        tx.data().items_mut()[0].title = "after".into();
        tx.touched(
            vec![store.store_id(), handle, day_model::field_id("title")],
            "title",
        );
        handle
    })
    .join()
    .expect("worker");
    assert_eq!(done, main_handle, "same key, same handle, either thread");

    let (_, changes) = day_model::record_changes(|| {
        assert_eq!(store.pump(), 1);
    });
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].label, "title");
    assert_eq!(store.elem(u).title().peek(), "after");
}

#[test]
fn undo_inverts_edits_on_wide_keyed_rows() {
    let u = Uuid::now_v7();
    let store = Store::new(Keyed::new(vec![card(u, "original")]));
    let stack = day_model::UndoStack::new(10);
    stack.watch(store);

    store.elem(u).title().write("edited".into());
    day_reactive::flush_sync();
    assert!(stack.undo(), "the edit sealed into a unit");
    assert_eq!(
        store.elem(u).title().peek(),
        "original",
        "the handle stayed valid through the replay"
    );

    store.restructure("delete", Op::Delete, u, |k| {
        k.remove(ModelId::<Card>::of(u).handle());
    });
    day_reactive::flush_sync();
    assert!(stack.undo(), "the delete sealed");
    assert_eq!(
        store.elem(u).title().peek(),
        "original",
        "the deleted row came back under the same key"
    );
}

#[test]
fn duplicate_keys_resolve_to_the_last_row() {
    // Two rows with the same key is an app bug the index cannot represent; the documented
    // behavior is last-writer-wins in the index while both rows stay in the items list.
    let u = Uuid::now_v7();
    let store = Store::new(Keyed::new(vec![card(u, "first"), card(u, "second")]));
    assert_eq!(store.elem(u).title().peek(), "second");
    store.with_untracked(|k| assert_eq!(k.len(), 2));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reserve the top bit")]
fn a_top_bit_integer_key_is_refused_in_debug() {
    let _ = day_model::AsKey::as_key(&(u64::MAX - 1));
}
