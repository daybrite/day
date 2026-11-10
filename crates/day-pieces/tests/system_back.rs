// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `Decorate::on_system_back` (docs/navigation.md "Handling the system Back"): which handler a
//! back reaches, and whether the platform's system back is armed for it, as the piece tree and
//! the selected page change. The ordering after covers and pushed pages is in `mock_e2e.rs`.

use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

fn boot<P: Piece>(root: impl FnOnce() -> P + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (toolkit, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(toolkit, options, move || root().any());
    flush_sync();
    probe
}

/// The platform's own back, which reaches the app only while Day armed it.
fn native_back() -> bool {
    let ok = day_core::with_tree(|t| t.native_back());
    flush_sync();
    ok
}

/// A tab bar keeps every page it built, so a handler in a tab that is not showing stays
/// registered. Selecting that tab from code (a signal write, `navigate`) passes through no
/// event, and still has to arm the system back, or the press leaves the app instead of reaching
/// the handler. Switching away releases it again.
#[test]
fn selecting_a_resident_tab_from_code_arms_its_handler() {
    let sel = Signal::new("one".to_string());
    let ran = Signal::new(0usize);
    let probe = boot(move || {
        nav(sel)
            .style(NavStyle::Tabs)
            .item("one", "One", || label("one-content"))
            .item("two", "Two", move || {
                label("two-content").on_system_back(true, move || ran.update(|n| *n += 1))
            })
    });
    assert!(
        !probe.system_back_armed(),
        "the handler's tab is not showing"
    );
    assert!(!native_back());
    assert_eq!(ran.get_untracked(), 0);

    sel.set("two".into());
    flush_sync();
    assert!(probe.system_back_armed(), "a signal write selected the tab");
    assert!(native_back());
    assert_eq!(ran.get_untracked(), 1);

    assert!(navigate("one"));
    flush_sync();
    assert!(!probe.system_back_armed(), "navigate left the tab");
    assert!(!native_back());
    assert_eq!(ran.get_untracked(), 1);

    assert!(navigate("two"));
    flush_sync();
    assert!(probe.system_back_armed(), "navigate returned to the tab");
    assert!(native_back());
    assert_eq!(ran.get_untracked(), 2);
}

/// Among enabled handlers in the same layer, the one nested deepest in the piece tree wins,
/// whatever order the pieces mounted in: a later sibling higher up does not take the back from
/// a handler inside the other branch.
#[test]
fn the_deepest_handler_wins_over_a_later_shallower_one() {
    let deep_on = Signal::new(true);
    let deep = Signal::new(0usize);
    let shallow = Signal::new(0usize);
    let probe = boot(move || {
        column((
            column((column((label("deep").on_system_back(
                move || deep_on.get(),
                move || deep.update(|n| *n += 1),
            ),)),)),
            label("shallow").on_system_back(true, move || shallow.update(|n| *n += 1)),
        ))
    });
    assert!(probe.system_back_armed());
    let counts = || (deep.get_untracked(), shallow.get_untracked());

    assert!(nav_back());
    assert!(native_back());
    assert_eq!(counts(), (2, 0), "the deeper handler takes both rails");

    deep_on.set(false);
    flush_sync();
    assert!(
        probe.system_back_armed(),
        "the shallow handler still wants it"
    );
    assert!(nav_back());
    assert_eq!(counts(), (2, 1));

    // Re-enabled, the deep handler takes the back again.
    deep_on.set(true);
    flush_sync();
    assert!(native_back());
    assert_eq!(counts(), (3, 1));
}

/// Nesting counts navigation pages too: a handler inside a pushed stack page sits below the
/// stack, so it beats a handler beside the stack that mounted after it, and equally deep
/// handlers fall back to the most recently mounted.
#[test]
fn a_handler_inside_a_page_beats_a_sibling_beside_its_host() {
    let page = Signal::new(0usize);
    let beside = Signal::new(0usize);
    let twin_a = Signal::new(0usize);
    let twin_b = Signal::new(0usize);
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || {
        column((
            nav_stack(
                path,
                label("root").on_system_back(true, move || page.update(|n| *n += 1)),
            )
            .destination(|key: &String| label(format!("detail:{key}"))),
            label("beside").on_system_back(true, move || beside.update(|n| *n += 1)),
        ))
    });
    assert!(probe.system_back_armed());
    assert!(nav_back());
    assert_eq!((page.get_untracked(), beside.get_untracked()), (1, 0));
    drop(probe);

    let probe = boot(move || {
        row((
            label("a").on_system_back(true, move || twin_a.update(|n| *n += 1)),
            label("b").on_system_back(true, move || twin_b.update(|n| *n += 1)),
        ))
    });
    assert!(probe.system_back_armed());
    assert!(nav_back());
    assert_eq!(
        (twin_a.get_untracked(), twin_b.get_untracked()),
        (0, 1),
        "siblings: the most recently mounted"
    );
}
