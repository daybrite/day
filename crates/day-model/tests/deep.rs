// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! How deep can a path go?

use day_macros::Observable;
use day_model::{Keyed, Source, Store};
use day_pieces::Binding;

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Country {
    pub code: String,
    pub name: String,
}

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Region {
    pub country: Country,
    pub label: String,
}

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Address {
    pub region: Region,
    pub city: String,
}

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    pub address: Address,
    pub name: String,
}

fn store() -> Store<Keyed<Item>> {
    Store::new(Keyed::new(vec![Item {
        id: 1,
        ..Default::default()
    }]))
}

#[test]
fn depth_four_is_fine() {
    // store → elem → address → city
    let s = store();
    s.elem(1).address().city().write("Bath".into());
    assert_eq!(
        s.with_untracked(|k| k.get(1).unwrap().address.city.clone()),
        "Bath"
    );
}

#[test]
fn depth_five_and_beyond() {
    // store → elem → address → region → country → code
    let s = store();
    s.elem(1)
        .address()
        .region()
        .country()
        .code()
        .write("GB".into());
    assert_eq!(
        s.with_untracked(|k| k.get(1).unwrap().address.region.country.code.clone()),
        "GB"
    );
}

#[test]
fn depth_is_unbounded_and_still_precise() {
    let s = store();
    let code_runs = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let city_runs = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let (c, y) = (code_runs.clone(), city_runs.clone());

    // Six levels: store → elem → address → region → country → code
    day_core::uninstall_tree();
    let (mock, _probe) = day_mock::MockToolkit::new();
    day_core::launch_with(
        mock,
        day_spec::WindowOptions {
            title: "t".into(),
            size: day_spec::Size::new(300.0, 300.0),
            ..Default::default()
        },
        move || {
            use day_pieces::prelude::*;
            let it = s.elem(1);
            column((
                label(move || {
                    c.set(c.get() + 1);
                    it.address()
                        .region()
                        .country()
                        .code()
                        .with(|v| v.cloned().unwrap_or_default())
                }),
                label(move || {
                    y.set(y.get() + 1);
                    it.address().city().with(|v| v.cloned().unwrap_or_default())
                }),
            ))
            .any()
        },
    );
    day_reactive::flush_sync();
    let (bc, by) = (code_runs.get(), city_runs.get());

    // A write six levels down wakes its own reader and nobody else.
    s.elem(1)
        .address()
        .region()
        .country()
        .code()
        .write("GB".into());
    day_reactive::flush_sync();
    assert_eq!(code_runs.get(), bc + 1);
    assert_eq!(
        city_runs.get(),
        by,
        "address.city is not address.region.country.code"
    );

    // A sibling six levels down is still separate.
    s.elem(1)
        .address()
        .region()
        .country()
        .name()
        .write("United Kingdom".into());
    day_reactive::flush_sync();
    assert_eq!(code_runs.get(), bc + 1);

    // …and a coarse reader at level three still wakes for both.
    assert_eq!(
        s.with_untracked(|k| k.get(1).unwrap().address.region.country.code.clone()),
        "GB"
    );
}

#[test]
fn interning_is_paid_per_handle_not_per_read() {
    let s = store();
    let before = day_model::interned_nodes();
    let it = s.elem(1); // interns the element
    let after_handle = day_model::interned_nodes();

    // A thousand leaf reads intern nothing further.
    for _ in 0..1000 {
        it.name().with_untracked(|_| {});
    }
    assert_eq!(day_model::interned_nodes(), after_handle);
    assert!(after_handle - before <= 1, "one node for the element");

    // Nesting interns the intermediate field, once, however often it is used.
    for _ in 0..1000 {
        it.address().city().with_untracked(|_| {});
    }
    assert_eq!(
        day_model::interned_nodes() - after_handle,
        1,
        "one node for `address`"
    );
}
