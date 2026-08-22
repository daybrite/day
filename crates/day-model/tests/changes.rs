// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The change log's value capture: prior and new values ride along when a consumer asks, and
//! cost nothing when nobody does.

use day_macros::Observable;
use day_model::{Keyed, Op, Store};
use day_reactive::Binding;

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
}

fn store() -> Store<Keyed<Item>> {
    Store::new(Keyed::new(vec![Item {
        id: 1,
        name: "before".into(),
        count: 4,
    }]))
}

#[test]
fn a_write_records_prior_and_new_values_when_asked() {
    let store = store();
    let (_, changes) = day_model::record_values(|| {
        store.elem(1).name().write("after".into());
        store.elem(1).count().update(|c| *c += 5);
    });

    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].label, "name");
    assert_eq!(changes[0].op, Op::Set);
    assert_eq!(
        changes[0].prior_as::<String>().map(String::as_str),
        Some("before")
    );
    assert_eq!(
        changes[0].value_as::<String>().map(String::as_str),
        Some("after")
    );
    assert_eq!(changes[1].prior_as::<i64>(), Some(&4));
    assert_eq!(changes[1].value_as::<i64>(), Some(&9));
}

#[test]
fn values_are_not_captured_when_nobody_asks() {
    let store = store();
    let (_, changes) = day_model::record_changes(|| {
        store.elem(1).name().write("after".into());
    });
    assert_eq!(changes.len(), 1);
    assert!(
        changes[0].prior.is_none(),
        "no consumer asked, so no clone was paid"
    );
    assert!(changes[0].value.is_none());
}

#[test]
fn the_components_name_the_full_path() {
    let store = store();
    let (_, changes) = day_model::record_changes(|| {
        store.elem(1).count().update(|c| *c = 7);
    });
    // store root, element key, field id — outermost first.
    assert_eq!(changes[0].components.len(), 3);
    assert_eq!(changes[0].components[1], 1, "the element's key");
}

#[test]
fn a_mapped_write_captures_the_stored_type() {
    let store = store();
    let idx = store.elem(1).count().map(|c| *c as usize, |i| *i as i64);
    let (_, changes) = day_model::record_values(|| {
        idx.write(2usize);
    });
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].prior_as::<i64>(),
        Some(&4),
        "the STORED type, not the view's"
    );
    assert_eq!(changes[0].value_as::<i64>(), Some(&2));
}

#[test]
fn merge_row_announces_the_named_fields_with_the_current_author() {
    let store = store();
    let (_, changes) = day_model::record_changes(|| {
        day_model::with_author("database", || {
            let merged = store.merge_row(
                1,
                Item {
                    id: 1,
                    name: "external".into(),
                    count: 9,
                },
                &["name", "count"],
            );
            assert!(merged);
        });
    });
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].label, "name");
    assert_eq!(changes[0].op, Op::Set);
    assert_eq!(changes[0].author, Some("database"));
    assert_eq!(changes[0].components.len(), 3);
    assert_eq!(changes[0].components[1], 1, "the element's key");
    assert_eq!(changes[1].label, "count");
    store.with_untracked(|k| {
        let row = k.get(1).expect("the row is present");
        assert_eq!(row.name, "external");
        assert_eq!(row.count, 9);
    });
}

#[test]
fn merge_row_of_an_absent_row_announces_nothing() {
    let store = store();
    let (_, changes) = day_model::record_changes(|| {
        assert!(!store.merge_row(404, Item::default(), &["name"]));
    });
    assert!(changes.is_empty());
}

#[test]
fn a_standing_sink_sees_every_change_until_removed() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let store = store();
    let seen: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    let sink = day_model::install_change_sink(move |c| s.borrow_mut().push(c.label));

    store.elem(1).name().write("one".into());
    store.elem(1).count().update(|c| *c += 1);
    assert_eq!(*seen.borrow(), vec!["name", "count"]);

    day_model::remove_change_sink(sink);
    store.elem(1).name().write("two".into());
    assert_eq!(seen.borrow().len(), 2, "a removed sink hears nothing");
}

#[test]
fn a_sink_and_the_recorder_see_the_same_changes() {
    use std::cell::Cell;
    use std::rc::Rc;

    let store = store();
    let sink_count = Rc::new(Cell::new(0usize));
    let s = sink_count.clone();
    let sink = day_model::install_change_sink(move |_| s.set(s.get() + 1));

    let (_, log) = day_model::record_changes(|| {
        store.elem(1).name().write("both".into());
    });
    assert_eq!(log.len(), 1);
    assert_eq!(sink_count.get(), 1);
    day_model::remove_change_sink(sink);
}
