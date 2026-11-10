// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Sharing with the app's own chooser actions (docs/sharing.md): the request the toolkit
//! receives, the control it points at, a chosen action running, and an earlier chooser's
//! actions no longer answering once the next one opens.

use std::cell::Cell;
use std::rc::Rc;

use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Symbol, WindowOptions};

fn boot() -> MockProbe {
    day_core::uninstall_tree();
    let (toolkit, probe) = MockToolkit::new();
    day_core::launch_with(toolkit, WindowOptions::default(), || {
        button("Share").id(id!(share)).any()
    });
    flush_sync();
    probe
}

#[test]
fn a_share_carries_its_actions_and_anchor() {
    let probe = boot();
    let bookmarked = Rc::new(Cell::new(0));
    let found = Rc::new(Cell::new(0));
    let (b, f) = (bookmarked.clone(), found.clone());
    assert!(
        Share::url("https://example.com/")
            .title("Example Domain")
            .action(
                share_action("Add Bookmark")
                    .symbol(Symbol::Bookmark)
                    .action(move || b.set(b.get() + 1))
            )
            .action(share_action("Find on Page").action(move || f.set(f.get() + 1)))
            .anchor(id!(share))
            .present()
    );
    let shares = probe.shares();
    assert_eq!(shares.len(), 1);
    let (spec, anchor) = &shares[0];
    assert_eq!(spec.url, "https://example.com/");
    assert_eq!(spec.title, "Example Domain");
    let titles: Vec<_> = spec.actions.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["Add Bookmark", "Find on Page"]);
    assert_eq!(spec.actions[0].symbol, Some(Symbol::Bookmark));
    assert_eq!(spec.actions[1].symbol, None);
    assert!(anchor.is_some(), "the chooser points at the share button");

    assert!(probe.choose_share_action(0));
    flush_sync();
    assert_eq!((bookmarked.get(), found.get()), (1, 0));
    assert!(!probe.choose_share_action(2));
}

#[test]
fn opening_a_chooser_retires_the_last_ones_actions() {
    let probe = boot();
    let first = Rc::new(Cell::new(0));
    let f = first.clone();
    let _ = Share::url("https://a.example/")
        .action(share_action("One").action(move || f.set(f.get() + 1)))
        .present();
    let old_id = probe.shares()[0].0.actions[0].id;
    let _ = Share::url("https://b.example/").present();
    assert_eq!(probe.shares()[1].1, None, "no anchor: the window's corner");
    probe.emit(day_spec::WINDOW_NODE, day_spec::Event::MenuAction(old_id));
    flush_sync();
    assert_eq!(first.get(), 0, "a retired action does nothing");
}
