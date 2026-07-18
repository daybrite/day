//! Mock e2e for the pull-refresh piece (the day-pieces mock_e2e pattern): the emulated
//! composition path — the piece's overlay container mounts the wrapped scrollable, the spinner
//! overlay appears exactly while `refreshing` is true, `ToggleChanged` (dayscript's `toggle:`)
//! drives synthetic begin/end, and `on_refresh` runs once per begin.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit, MockWidget};
use day_piece_pullrefresh::pull_to_refresh;
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, Size, WindowOptions};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

/// The piece's own container: the `day.container` that directly parents the `day.scroll`.
fn refresh_host(probe: &MockProbe) -> MockWidget {
    let scroll = probe.find_by_kind("day.scroll");
    assert_eq!(scroll.len(), 1, "the wrapped scrollable mounted");
    let scroll_h = scroll[0].0.0;
    let mut hosts: Vec<MockWidget> = probe
        .find_by_kind("day.container")
        .into_iter()
        .map(|(_, w)| w)
        .filter(|w| w.children.contains(&scroll_h))
        .collect();
    assert_eq!(hosts.len(), 1, "exactly one container parents the scroll");
    hosts.remove(0)
}

fn spinner_count(probe: &MockProbe) -> usize {
    probe.find_by_kind("day.progress").len()
}

#[test]
fn mounts_child_and_toggle_drives_overlay() {
    let probe = boot(|| {
        let refreshing = day_reactive::Signal::new(false);
        pull_to_refresh(refreshing, scroll(column((label("row 1"), label("row 2"))))).any()
    });
    let host = refresh_host(&probe); // child mounted inside the piece's container
    assert_eq!(spinner_count(&probe), 0, "no overlay while idle");

    // dayscript's `toggle: {value: true}` → synthetic begin → the overlay spinner appears.
    probe.emit(NodeId(host.node), Event::ToggleChanged(true));
    flush_sync();
    assert_eq!(spinner_count(&probe), 1, "overlay spinner while refreshing");

    // `toggle: {value: false}` → end → the overlay is disposed.
    probe.emit(NodeId(host.node), Event::ToggleChanged(false));
    flush_sync();
    assert_eq!(spinner_count(&probe), 0, "overlay dismissed");
}

#[test]
fn programmatic_signal_drives_overlay() {
    let cell: Rc<Cell<Option<day_reactive::Signal<bool>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let refreshing = day_reactive::Signal::new(false);
        cell2.set(Some(refreshing));
        pull_to_refresh(refreshing, scroll(label("content"))).any()
    });
    let refreshing = cell.get().expect("signal captured");
    assert_eq!(spinner_count(&probe), 0);

    refreshing.set(true); // app-initiated refresh (e.g. a Refresh-now button)
    flush_sync();
    assert_eq!(
        spinner_count(&probe),
        1,
        "programmatic begin shows the overlay"
    );

    refreshing.set(false); // reload finished
    flush_sync();
    assert_eq!(spinner_count(&probe), 0, "completion dismisses the overlay");
}

#[test]
fn on_refresh_runs_once_per_begin() {
    let count = Rc::new(Cell::new(0u32));
    let count2 = count.clone();
    let probe = boot(move || {
        let refreshing = day_reactive::Signal::new(false);
        pull_to_refresh(refreshing, scroll(label("content")))
            .on_refresh(move || count2.set(count2.get() + 1))
            .any()
    });
    let host = refresh_host(&probe);

    probe.emit(NodeId(host.node), Event::ToggleChanged(true));
    flush_sync();
    assert_eq!(count.get(), 1, "begin runs on_refresh");

    // A second begin while already refreshing is a no-op (idempotent begin path).
    probe.emit(NodeId(host.node), Event::ToggleChanged(true));
    flush_sync();
    assert_eq!(count.get(), 1, "no double-fire while refreshing");

    probe.emit(NodeId(host.node), Event::ToggleChanged(false));
    probe.emit(NodeId(host.node), Event::ToggleChanged(true));
    flush_sync();
    assert_eq!(count.get(), 2, "a fresh pull after completion fires again");
}
