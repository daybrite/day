// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A live container behind a mock-toolkit list: autosave at every turn's end must not cost the
//! UI a single patch. Reproduces the Showcase walkthrough shape (type into a bound field, the
//! row label follows) headlessly.

use day_core::AnyPiece;
use day_macros::Model;
use day_model::{Keyed, Op, Store};
use day_persistence::{ModelContainer, Sqlite, schema};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "rows")]
pub struct Task {
    #[model(id)]
    pub id: u32,
    pub name: String,
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> day_mock::MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = day_mock::MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        root,
    );
    probe
}

fn count(probe: &day_mock::MockProbe, needle: &str) -> usize {
    probe.log().iter().filter(|l| l.contains(needle)).count()
}

#[test]
fn keystrokes_into_a_container_store_keep_patching_the_row_label() {
    let container =
        ModelContainer::open(Sqlite::memory(), schema![Task]).expect("open memory container");
    let store: Store<Keyed<Task>> = container.store::<Task>();
    store.restructure("seed", Op::Insert, 1, |v| {
        for n in 1..=3u32 {
            v.push(Task {
                id: n,
                name: format!("Task {n}"),
            });
        }
    });

    let probe = boot(move || {
        list(store, |slot: ModelSlot<Task>| {
            label(move || slot.name().read())
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    probe.list_bind(host, 0, day_mock::MockHandle(9400));
    flush_sync();
    let labels = count(&probe, "update day.label");

    let mut text = String::new();
    for ch in "Renamed task".chars() {
        text.push(ch);
        store.elem(1).name().write(text.clone());
        flush_sync(); // one turn per keystroke — autosave flushes SQL between each
    }

    assert_eq!(
        count(&probe, "update day.label") - labels,
        "Renamed task".chars().count(),
        "every keystroke patched the bound row label with autosave active"
    );
    let last = probe
        .log()
        .iter()
        .rev()
        .find(|l| l.contains("update day.label"))
        .cloned()
        .unwrap_or_default();
    assert!(
        last.contains("Renamed task"),
        "final text reached the cell: {last}"
    );
}
