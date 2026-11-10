// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Settings routes on a platform with no app menu (a phone; the mock reports none): an app
//! that places Settings in its own navigation owns `settings/<group>`, and an app that does not
//! gets the Settings cover on that group.

use std::cell::Cell;
use std::rc::Rc;

use day_mock::{MockProbe, MockToolkit};
use day_piece_settings::prelude::*;
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::WindowOptions;

fn prefs() -> Preferences {
    Preferences::new()
        .presentation(Presentation::Navigation)
        .group(Group::new("search", "Search").row(rows::note("Engines")))
        .group(Group::new("privacy", "Privacy").row(rows::note("Data")))
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (toolkit, probe) = MockToolkit::new();
    day_core::launch_with(toolkit, WindowOptions::default(), root);
    flush_sync();
    probe
}

fn preference_windows(probe: &MockProbe) -> usize {
    probe
        .state
        .borrow()
        .windows
        .iter()
        .filter(|w| w.open && w.kind == "preferences")
        .count()
}

/// The reported case: Settings embedded through the free `page()` in the app's own section,
/// which the sidebar has not built yet (a page builds on its first visit). `settings/search`
/// must select that section and open its Search group, not present a cover over Home.
#[test]
fn an_unvisited_settings_section_owns_its_routes() {
    let section = Signal::new("home".to_string());
    let built = Rc::new(Cell::new(false));
    let mark = built.clone();
    let probe = boot(move || {
        prefs().install();
        nav(section)
            .style(NavStyle::Sidebar)
            .item("home", "Home", || label("Home"))
            .item("settings", "Settings", move || {
                mark.set(true);
                day_piece_settings::page()
            })
            .any()
    });
    assert!(
        !built.get(),
        "the Settings page is not built before its first visit"
    );
    assert!(navigate("settings/search"));
    flush_sync();
    assert!(built.get());
    assert_eq!(section.get_untracked(), "settings");
    assert_eq!(current_route().as_deref(), Some("settings/search"));
    assert_eq!(
        preference_windows(&probe),
        0,
        "no cover over the app's own Settings"
    );
}

/// The same through the nav item `Prefs` hands out.
#[test]
fn a_settings_nav_item_owns_its_routes() {
    let section = Signal::new("home".to_string());
    let probe = boot(move || {
        let p = prefs().install();
        nav(section)
            .item("home", "Home", || label("Home"))
            .items(p.nav_item_everywhere("settings"))
            .any()
    });
    assert!(navigate("settings/search"));
    flush_sync();
    assert_eq!(section.get_untracked(), "settings");
    assert_eq!(preference_windows(&probe), 0);
}

/// An app with no Settings in its navigation (a browser drawing its own chrome) still reaches
/// a group by route: the cover opens.
#[test]
fn without_a_settings_section_the_cover_opens_on_the_group() {
    let probe = boot(move || {
        prefs().install();
        label("A browser's own chrome").any()
    });
    assert!(navigate("settings/search"));
    flush_sync();
    assert_eq!(preference_windows(&probe), 1);
}

/// A row's sub-page shows its own title in the bar, not the key it is routed under.
#[test]
fn a_row_sub_page_is_titled_by_its_title() {
    use day_pieces::Route;
    let route = day_piece_settings::SettingsRoute::Group("page:Filter Lists".into());
    assert_eq!(route.title(), "Filter Lists");
    assert_eq!(route.key(), "page:Filter Lists");
}
