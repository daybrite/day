// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The derive, at its call sites: simple struct, keyed collection, nesting.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::{MockProbe, MockToolkit};
use day_model::{Keyed, Store};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, Size, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Address {
    pub city: String,
    pub postcode: String,
}

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    /// Doc comments and attributes in between must not confuse the parser.
    pub name: String,
    pub count: i64,
    pub done: bool,
    pub address: Address,
    /// A field the UI never observes, so it gets neither an accessor, a path nor a trigger.
    #[obs(skip)]
    pub cache: Option<Vec<u8>>,
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 400.0),
            ..Default::default()
        },
        root,
    );
    probe
}

fn seed() -> Keyed<Item> {
    Keyed::new(
        (0..5)
            .map(|n| Item {
                id: n,
                name: format!("row {n}"),
                count: n as i64,
                address: Address {
                    city: format!("city {n}"),
                    postcode: "SW1".into(),
                },
                ..Default::default()
            })
            .collect(),
    )
}

#[test]
fn the_derive_lists_what_is_observable() {
    assert_eq!(
        Item::OBSERVED_FIELDS,
        &["id", "name", "count", "done", "address"],
        "`cache` is skipped"
    );
    assert_eq!(Address::OBSERVED_FIELDS, &["city", "postcode"]);
}

#[test]
fn a_plain_struct_store_gets_accessors() {
    // The simple case: no collection at all.
    let settings = Store::new(Address {
        city: "London".into(),
        postcode: "SW1".into(),
    });
    let probe = boot(move || text_field(settings.city()).any());
    let node = NodeId(probe.find_by_kind("day.text_field")[0].1.node);
    assert_eq!(probe.find_by_kind("day.text_field")[0].1.text, "London");
    probe.emit(node, Event::TextChanged("Leeds".into()));
    flush_sync();
    assert_eq!(settings.with_untracked(|a| a.city.clone()), "Leeds");
}

#[test]
fn an_element_of_a_collection_gets_the_same_accessors() {
    let store = Store::new(seed());
    let probe = boot(move || {
        let it = store.elem(3);
        column((text_field(it.name()), toggle(it.done()))).any()
    });
    let node = NodeId(probe.find_by_kind("day.text_field")[0].1.node);
    probe.emit(node, Event::TextChanged("edited".into()));
    flush_sync();
    assert_eq!(
        store.with_untracked(|k| k.get(3).unwrap().name.clone()),
        "edited"
    );
}

#[test]
fn nested_structs_chain() {
    let store = Store::new(seed());
    let city_runs = Rc::new(Cell::new(0usize));
    let name_runs = Rc::new(Cell::new(0usize));
    let (c, n) = (city_runs.clone(), name_runs.clone());
    boot(move || {
        let it = store.elem(2);
        column((
            label(move || {
                c.set(c.get() + 1);
                it.address().city().with(|v| v.cloned().unwrap_or_default())
            }),
            label(move || {
                n.set(n.get() + 1);
                it.name().with(|v| v.cloned().unwrap_or_default())
            }),
        ))
        .any()
    });
    flush_sync();
    let (bc, bn) = (city_runs.get(), name_runs.get());

    // Three levels down, and still precise: the sibling field does not wake.
    store.elem(2).address().city().write("Bath".into());
    flush_sync();
    assert_eq!(city_runs.get(), bc + 1);
    assert_eq!(
        name_runs.get(),
        bn,
        "name did not wake for a change to address.city"
    );

    // …and the postcode does not wake the city.
    store.elem(2).address().postcode().write("BA1".into());
    flush_sync();
    assert_eq!(city_runs.get(), bc + 1);
}

#[test]
fn a_structural_change_wakes_the_list_but_not_a_field_reader() {
    let store = Store::new(seed());
    let list_runs = Rc::new(Cell::new(0usize));
    let field_runs = Rc::new(Cell::new(0usize));
    let (l, f) = (list_runs.clone(), field_runs.clone());
    boot(move || {
        let it = store.elem(1);
        column((
            label(move || {
                l.set(l.get() + 1);
                format!("{} rows", store.keys().len())
            }),
            label(move || {
                f.set(f.get() + 1);
                it.name().with(|v| v.cloned().unwrap_or_default())
            }),
        ))
        .any()
    });
    flush_sync();
    let (bl, bf) = (list_runs.get(), field_runs.get());

    // A field write does not re-run the list.
    store.elem(1).name().write("renamed".into());
    flush_sync();
    assert_eq!(list_runs.get(), bl, "the list's shape did not change");
    assert_eq!(field_runs.get(), bf + 1);

    // An insert does.
    store.restructure("push", day_model::Op::Insert, 99, |k| {
        k.push(Item {
            id: 99,
            name: "new".into(),
            ..Default::default()
        })
    });
    flush_sync();
    assert_eq!(list_runs.get(), bl + 1);
}

/// What a derive layered on `Observable` expands to: the struct is written by hand and
/// `observable_impls!` is handed only the fields to observe. `shadow` is left out of the
/// description, so it gets nothing, exactly as `#[obs(skip)]` would have left it.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Described {
    pub id: u64,
    pub title: String,
    pub shadow: u8,
}

day_macros::observable_impls! {
    struct Described {
        #[obs(key)]
        id: u64,
        title: String,
    }
}

#[test]
fn a_described_struct_gets_what_the_derive_gives() {
    assert_eq!(Described::OBSERVED_FIELDS, ["id", "title"]);
    let store = Store::new(Keyed::new(vec![Described {
        id: 7,
        title: "seven".into(),
        shadow: 1,
    }]));
    store.elem(7u64).title().write("VII".into());
    assert_eq!(
        store.elem(7u64).title().with(|v| v.cloned()),
        Some("VII".to_string())
    );
}

/// A struct that derives day-persistence's `Model` where it is stored can derive `Observable`
/// where it is not, behind `cfg_attr`, without restating itself: `#[model(…)]` is accepted,
/// its `id` is the key, and the rest is another derive's grammar.
#[derive(Observable, Clone, Default, PartialEq, Debug)]
#[model(table = "tracks", index("title", "plays"))]
pub struct Track {
    #[model(id)]
    pub id: u32,
    #[model(unique, column = "name")]
    pub title: String,
    #[model(transient)]
    pub plays: u32,
}

#[test]
fn model_attributes_are_tolerated_and_id_is_the_key() {
    assert_eq!(Track::OBSERVED_FIELDS, ["id", "title", "plays"]);
    let store = Store::new(Keyed::new(vec![Track {
        id: 3,
        title: "three".into(),
        plays: 0,
    }]));
    store.elem(3).plays().write(9);
    assert_eq!(store.elem(3).plays().with(|v| v.copied()), Some(9));
}
