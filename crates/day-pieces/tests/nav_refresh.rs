// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Sidebar activation and refresh contracts; native gestures are additionally tested by apps.
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Event, NodeId, Size, Support, WindowOptions, kinds, props::RefreshPatch};

fn boot(root: impl FnOnce() -> day_core::AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    probe.set_nav_content_list(Support::Emulated);
    probe.set_no_nav_reorder(true);
    probe.describe_patch::<RefreshPatch>(|p| format!("refreshing={}", p.refreshing));
    day_core::launch_with(
        mock,
        WindowOptions {
            size: Size::new(390.0, 844.0),
            ..Default::default()
        },
        root,
    );
    probe
}

#[test]
fn requests_follow_app_busy_state_and_disposal() {
    let busy = Signal::new(false);
    let accept = Signal::new(true);
    let requests = Signal::new(0);
    let mounted = Signal::new(true);
    let probe = boot(move || {
        when(
            move || mounted.get(),
            move || {
                nav(Signal::new(None::<String>))
                    .style(NavStyle::Sidebar)
                    .item(Some("all".to_string()), "All", || label("Fixture articles"))
                    .id("fixture-nav") // Builder forwarding must work after decoration too.
                    .refreshable(busy, move || {
                        requests.update(|n| *n += 1);
                        busy.set(accept.get_untracked());
                    })
            },
        )
        .any()
    });
    let menu = NodeId(probe.find_by_kind(kinds::NAV_MENU)[0].1.node);
    probe.emit(menu, Event::RefreshRequested);
    assert_eq!(requests.get_untracked(), 1);
    assert!(busy.get_untracked());
    probe.emit(menu, Event::RefreshRequested);
    assert_eq!(
        requests.get_untracked(),
        1,
        "duplicate native requests while busy are ignored"
    );
    busy.set(false); // success, failure and cancellation share the same completion contract
    flush_sync();
    accept.set(false);
    probe.clear_log();
    probe.emit(menu, Event::RefreshRequested);
    assert_eq!(requests.get_untracked(), 2);
    assert!(
        probe.log().iter().any(|op| op.contains("refreshing=false")),
        "declining work must explicitly end the native indicator"
    );
    busy.set(true); // external refresh renders state but never calls the action
    flush_sync();
    assert_eq!(requests.get_untracked(), 2);
    assert!(probe.log().iter().any(|op| op.contains("refreshing=true")));
    mounted.set(false);
    flush_sync();
    probe.emit(menu, Event::RefreshRequested);
    assert_eq!(
        requests.get_untracked(),
        2,
        "disposed callbacks cannot start work"
    );
    day_core::uninstall_tree();
}

#[test]
fn selected_row_activation_presents_the_resident_list_again() {
    let selection = Signal::new(Some("all".to_string()));
    let probe = boot(move || {
        nav(selection)
            .style(NavStyle::Sidebar)
            .item(Some("all".to_string()), "All", || label("Fixture reader"))
            .content_list(|| label("No fixture articles"))
            .detail_visible(Signal::new(false))
            .any()
    });
    let menu = NodeId(probe.find_by_kind(kinds::NAV_MENU)[0].1.node);
    probe.clear_log();
    probe.emit(menu, Event::SelectionChanged(0));
    assert!(
        probe
            .log()
            .iter()
            .any(|op| op.contains("nav list in-stack=true")),
        "same-route activation must present the existing list: {:?}",
        probe.log()
    );
    assert!(
        !probe.log().iter().any(|op| op.starts_with("create ")),
        "activation must not recreate resident content"
    );
    day_core::uninstall_tree();
}
