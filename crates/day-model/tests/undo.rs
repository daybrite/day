// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The undo stack: units are turns, inverted — and preview sessions, whose sixty writes cost
//! one record. All headless, against the plain in-memory store (persistence is optional to
//! undo, not the other way around).

use day_macros::Observable;
use day_model::{Keyed, Op, Store, UndoStack};
use day_reactive::Binding;

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
}

fn store() -> Store<Keyed<Item>> {
    Store::new(Keyed::new(vec![
        Item {
            id: 1,
            name: "one".into(),
            count: 10,
        },
        Item {
            id: 2,
            name: "two".into(),
            count: 20,
        },
    ]))
}

fn stack_over(store: Store<Keyed<Item>>) -> UndoStack {
    let stack = UndoStack::new(100);
    stack.watch(store);
    stack
}

#[test]
fn an_edit_undoes_and_redoes() {
    let store = store();
    let stack = stack_over(store);

    store.elem(1).name().write("renamed".into());
    day_reactive::flush_sync();
    assert!(stack.can_undo().get_untracked());
    assert_eq!(stack.undo_label().get_untracked(), "Name");

    assert!(stack.undo());
    assert_eq!(store.elem(1).name().peek(), "one");
    assert!(stack.can_redo().get_untracked());
    assert!(!stack.can_undo().get_untracked());

    assert!(stack.redo());
    assert_eq!(store.elem(1).name().peek(), "renamed");
    assert!(stack.can_undo().get_untracked());
}

#[test]
fn one_turn_is_one_unit() {
    let store = store();
    let stack = stack_over(store);

    day_reactive::batch(|| {
        store.elem(1).name().write("a".into());
        store.elem(1).count().write(99);
        store.elem(2).name().write("b".into());
    });
    day_reactive::flush_sync();

    assert!(stack.undo(), "one unit holds the whole turn");
    assert_eq!(store.elem(1).name().peek(), "one");
    assert_eq!(store.elem(1).count().peek(), 10);
    assert_eq!(store.elem(2).name().peek(), "two");
    assert!(!stack.can_undo().get_untracked(), "nothing left");
}

#[test]
fn an_undone_delete_comes_back_with_its_data() {
    let store = store();
    let stack = stack_over(store);

    store.restructure("remove", Op::Delete, 1, |v| {
        v.remove(1);
    });
    day_reactive::flush_sync();
    assert!(!store.elem(1).exists());

    assert!(stack.undo());
    assert!(store.elem(1).exists());
    assert_eq!(
        store.elem(1).name().peek(),
        "one",
        "the row's data came back whole"
    );
    assert_eq!(store.elem(1).count().peek(), 10);

    assert!(stack.redo());
    assert!(!store.elem(1).exists());
}

#[test]
fn an_undone_insert_goes_away() {
    let store = store();
    let stack = stack_over(store);

    store.restructure("add", Op::Insert, 7, |v| {
        v.push(Item {
            id: 7,
            name: "seven".into(),
            count: 7,
        });
    });
    day_reactive::flush_sync();
    assert!(store.elem(7).exists());

    assert!(stack.undo());
    assert!(!store.elem(7).exists());
    assert!(stack.redo());
    assert_eq!(store.elem(7).name().peek(), "seven");
}

#[test]
fn a_new_edit_forks_history() {
    let store = store();
    let stack = stack_over(store);

    store.elem(1).name().write("first".into());
    day_reactive::flush_sync();
    stack.undo();
    assert!(stack.can_redo().get_untracked());

    store.elem(1).name().write("second".into());
    day_reactive::flush_sync();
    assert!(!stack.can_redo().get_untracked(), "redo died with the fork");
}

#[test]
fn grouped_changes_are_one_named_unit() {
    let store = store();
    let stack = stack_over(store);

    stack.grouped("rename-everything", || {
        store.elem(1).name().write("x".into());
        day_reactive::flush_sync();
        store.elem(2).name().write("y".into());
        day_reactive::flush_sync();
    });
    assert_eq!(stack.undo_label().get_untracked(), "Rename everything");

    assert!(stack.undo());
    assert_eq!(store.elem(1).name().peek(), "one");
    assert_eq!(store.elem(2).name().peek(), "two");
    assert!(!stack.can_undo().get_untracked());
}

#[test]
fn depth_is_bounded() {
    let store = store();
    let stack = UndoStack::new(3);
    stack.watch(store);

    for i in 0..10 {
        store.elem(1).count().write(i);
        day_reactive::flush_sync();
    }
    let mut undone = 0;
    while stack.undo() {
        undone += 1;
    }
    assert_eq!(undone, 3, "history holds exactly `levels` units");
    assert_eq!(
        store.elem(1).count().peek(),
        6,
        "back three steps, no further"
    );
}

#[test]
fn a_storm_of_previews_is_one_record_and_one_unit() {
    let store = store();
    let stack = stack_over(store);
    let field = store.elem(1).count();

    let ((), changes) = day_model::record_changes(|| {
        for i in 0..60 {
            field.write_preview(i);
        }
    });
    assert_eq!(changes.len(), 0, "sixty previews, zero records");
    assert_eq!(field.peek(), 59, "the store follows live");

    let ((), changes) = day_model::record_changes(|| {
        field.write_commit(50);
    });
    assert_eq!(changes.len(), 1, "one committed record for the gesture");
    assert_eq!(
        changes[0].prior_as::<i64>(),
        Some(&10),
        "prior predates the storm"
    );
    assert_eq!(changes[0].value_as::<i64>(), Some(&50));

    day_reactive::flush_sync();
    assert!(stack.undo());
    assert_eq!(
        field.peek(),
        10,
        "undo restores the pre-drag value in one step"
    );
}

#[test]
fn a_cancelled_session_leaves_nothing() {
    let store = store();
    let stack = stack_over(store);
    let field = store.elem(1).name();

    let s = field.session();
    let ((), changes) = day_model::record_changes(|| {
        s.preview("half-ty".into());
        s.preview("half-typed".into());
        s.cancel();
    });
    assert_eq!(changes.len(), 0);
    assert_eq!(field.peek(), "one", "Escape restores");
    day_reactive::flush_sync();
    assert!(!stack.can_undo().get_untracked(), "no unit either");
}

#[test]
fn preview_wakes_readers_but_not_sinks() {
    let store = store();
    let field = store.elem(1).count();
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let s2 = seen.clone();
    day_reactive::Effect::new(move || {
        let _ = field.read();
        s2.set(s2.get() + 1);
    });
    assert_eq!(seen.get(), 1);

    field.write_preview(77);
    day_reactive::flush_sync();
    assert_eq!(
        seen.get(),
        2,
        "the label tracking the slider follows the drag"
    );
}

#[test]
fn replay_changes_carry_the_author_tag() {
    let store = store();
    let stack = stack_over(store);
    store.elem(1).name().write("tagged".into());
    day_reactive::flush_sync();

    let ((), changes) = day_model::record_changes(|| {
        stack.undo();
    });
    assert!(!changes.is_empty());
    assert!(
        changes.iter().all(|c| c.author == Some("undo")),
        "consumers can tell an undo from the user: {changes:?}"
    );
}

#[test]
fn a_wholesale_rewrite_clears_history() {
    let store = store();
    let stack = stack_over(store);
    store.elem(1).name().write("x".into());
    day_reactive::flush_sync();
    assert!(stack.can_undo().get_untracked());

    store.update("import", |k| {
        *k = Keyed::new(vec![Item {
            id: 9,
            name: "fresh".into(),
            count: 0,
        }]);
    });
    day_reactive::flush_sync();
    assert!(
        !stack.can_undo().get_untracked(),
        "history no longer describes reachable states"
    );
}
