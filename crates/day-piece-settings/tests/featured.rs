// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A featured group, live sections and a row's picture (docs/settings.md "Featured groups and
//! live sections"), on the navigation presentation the phones use. The mock toolkit draws the
//! composed pages, which is the shape every native arm follows.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use day_mock::MockToolkit;
use day_piece_settings::prelude::*;
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::WindowOptions;

fn boot(root: impl FnOnce() -> AnyPiece + 'static) {
    day_core::uninstall_tree();
    let (toolkit, _probe) = MockToolkit::new();
    day_core::launch_with(toolkit, WindowOptions::default(), root);
    flush_sync();
}

fn on_screen(id: &day_core::ElementId) -> bool {
    day_core::with_tree(|t| t.find_by_id(id.as_str())).is_some()
}

fn group_entry(id: &'static str) -> day_core::ElementId {
    day_core::id!(prefs_group, id)
}

fn row(name: &'static str) -> day_core::ElementId {
    day_core::id!(test_row, name)
}

fn general() -> Group {
    Group::new("general", "General").row(rows::note("Units").id(row("units")))
}

fn fair() -> Group {
    Group::new("fair", "The Fair")
        .subtitle("More apps")
        .image(Arc::new(vec![0x89, b'P', b'N', b'G']))
        .featured()
        .row(rows::note("About").id(row("about")))
}

/// The case the option exists for: an app with one group shows its rows on the root, and a
/// crate that contributes a featured group must not turn that root into a list of two.
#[test]
fn a_featured_group_leaves_a_single_group_inline() {
    boot(|| {
        Preferences::new()
            .presentation(Presentation::Navigation)
            .group(general())
            .group(fair())
            .install();
        day_piece_settings::page()
    });
    assert!(
        on_screen(&row("units")),
        "the one listed group stays inline"
    );
    assert!(
        on_screen(&group_entry("fair")),
        "the featured group has its card"
    );
    assert!(
        !on_screen(&group_entry("general")),
        "the inline group is not also listed as a row"
    );
    assert!(
        !on_screen(&row("about")),
        "the featured group's rows are on its page, not the root"
    );
}

/// With several listed groups the root is a list, and the featured card is there beside it.
#[test]
fn a_featured_group_joins_a_list_of_groups() {
    boot(|| {
        Preferences::new()
            .presentation(Presentation::Navigation)
            .group(general())
            .group(Group::new("privacy", "Privacy").row(rows::note("Data")))
            .group(fair())
            .install();
        day_piece_settings::page()
    });
    for id in ["fair", "general", "privacy"] {
        assert!(on_screen(&group_entry(id)), "{id} is on the root");
    }
    assert!(!on_screen(&row("units")), "no group is inline in a list");
}

/// A featured group alone is still a card that opens a page, never an inline page.
#[test]
fn a_featured_group_alone_is_a_card() {
    boot(|| {
        Preferences::new()
            .presentation(Presentation::Navigation)
            .group(fair())
            .install();
        day_piece_settings::page()
    });
    assert!(on_screen(&group_entry("fair")));
    assert!(!on_screen(&row("about")));
}

/// Live sections are read when the group's page is built, and a change to what they read
/// rebuilds the page.
#[test]
fn live_sections_follow_what_they_read() {
    let names = Signal::new(vec!["one"]);
    let reads = Rc::new(Cell::new(0u32));
    let count = reads.clone();
    let live = move || {
        Group::new("live", "Live")
            .row(rows::note("Fixed").id(row("fixed")))
            .sections_with({
                let count = count.clone();
                move || {
                    count.set(count.get() + 1);
                    vec![
                        Section::new()
                            .title("Listed")
                            .rows(names.get().into_iter().map(|n| rows::note(n).id(row(n)))),
                    ]
                }
            })
    };

    // On the root, a group with live sections is a row, and nothing has read them yet.
    boot({
        let live = live.clone();
        move || {
            Preferences::new()
                .presentation(Presentation::Navigation)
                .group(live())
                .install();
            day_piece_settings::page()
        }
    });
    assert!(
        on_screen(&group_entry("live")),
        "listed, though it is the only group"
    );
    assert!(!on_screen(&row("fixed")), "not inline");
    assert_eq!(
        reads.get(),
        0,
        "the sections are not read before the page opens"
    );

    // Its page shows the declared rows and the live ones.
    boot(move || {
        let prefs = Preferences::new()
            .presentation(Presentation::Navigation)
            .group(live())
            .install();
        prefs.group_page("live")
    });
    assert!(reads.get() > 0, "building the page reads the sections");
    assert!(on_screen(&row("fixed")));
    assert!(on_screen(&row("one")));
    assert!(!on_screen(&row("two")));

    names.set(vec!["two", "three"]);
    flush_sync();
    assert!(
        on_screen(&row("fixed")),
        "the declared rows are rebuilt with the page"
    );
    assert!(
        !on_screen(&row("one")),
        "a row the sections dropped is gone"
    );
    assert!(on_screen(&row("two")));
    assert!(on_screen(&row("three")));
}

/// A row with a picture is still the row: its id is on screen and it sits in its section.
#[test]
fn a_row_with_a_picture_keeps_its_id() {
    boot(|| {
        let prefs = Preferences::new()
            .presentation(Presentation::Navigation)
            .group(
                Group::new("apps", "Apps")
                    .row(
                        rows::link("Fleet Reader", "https://example.org/fleet")
                            .caption("Feeds")
                            .image(Arc::new(vec![0x89, b'P', b'N', b'G']))
                            .id(row("fleet")),
                    )
                    .row(
                        rows::info("Signed in", "marc")
                            .image(Arc::new(vec![0x89, b'P', b'N', b'G']))
                            .id(row("account")),
                    ),
            )
            .install();
        prefs.group_page("apps")
    });
    assert!(on_screen(&row("fleet")));
    assert!(on_screen(&row("account")));
}
