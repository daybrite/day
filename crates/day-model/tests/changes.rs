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
