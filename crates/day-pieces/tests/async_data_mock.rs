// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Synthetic load states test subtree identity and reactive reads independently of HTTP.
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};
use std::{cell::Cell, rc::Rc};

fn boot(root: impl FnOnce() -> day_core::AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        root,
    );
    probe
}

#[test]
fn refresh_keeps_content_and_local_state_then_updates_bindings() {
    struct Value(u32); // Deliberately not Clone.
    let state = Signal::new(LoadState::<Value, String>::Ready(Rc::new(Value(1))));
    let builds = Rc::new(Cell::new(0));
    let built = builds.clone();
    let probe = boot(move || {
        load(state, move |data| {
            built.set(built.get() + 1);
            label(move || data.with(|v| v.0.to_string()))
        })
        .loading(|| label("loading fixture"))
        .failed(|e| label(e.clone()))
        .any()
    });
    let original = probe.find_by_kind("day.label")[0].0;
    state.set(LoadState::Refreshing(Rc::new(Value(1))));
    flush_sync();
    state.set(LoadState::Failed {
        error: Rc::new("refresh failure fixture".into()),
        previous: Some(Rc::new(Value(1))),
    });
    flush_sync();
    assert_eq!(probe.find_by_kind("day.label")[0].0, original);
    state.set(LoadState::Ready(Rc::new(Value(2))));
    flush_sync();
    assert_eq!(builds.get(), 1);
    let label = &probe.find_by_kind("day.label")[0];
    assert_eq!(label.0, original);
    assert_eq!(label.1.text, "2");
    state.set(LoadState::Loading);
    flush_sync();
    assert_eq!(probe.find_by_kind("day.label")[0].1.text, "loading fixture");
}

#[test]
fn replacement_is_opt_in_and_new_errors_replace_old_error_text() {
    let state = Signal::new(LoadState::<u32, String>::Ready(Rc::new(1)));
    let probe = boot(move || {
        load(state, |value| label(move || value.get().to_string()))
            .loading(|| label("loading fixture"))
            .failed(|error| label(error.clone()))
            .replace_on_refresh()
            .any()
    });
    state.set(LoadState::Refreshing(Rc::new(1)));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.label")[0].1.text, "loading fixture");
    for message in ["first fixture", "second fixture"] {
        state.set(LoadState::Failed {
            error: Rc::new(message.into()),
            previous: Some(Rc::new(1)),
        });
        flush_sync();
        assert_eq!(probe.find_by_kind("day.label")[0].1.text, message);
    }
    state.set(LoadState::Idle);
    flush_sync();
    assert!(probe.find_by_kind("day.label").is_empty());
}
