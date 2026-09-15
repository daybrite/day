// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The composed inspector's trailing pane on a WIDE window: the pane follows the window's
//! width (docs/inspector.md) and its panel is laid out inside it.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_spec::{Size, WindowOptions};

fn boot_wide(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(1280.0, 800.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

#[test]
fn the_side_pane_takes_320_on_a_wide_window_and_holds_its_panel() {
    let probe = boot_wide(|| {
        let visible = Signal::new(true);
        inspector(visible, label("content"), || label("panel-row").any()).any()
    });
    let labels = probe.find_by_kind("day.label");
    let scroll = &probe.find_by_kind("day.scroll")[0].1;
    assert_eq!(
        scroll.frame,
        day_spec::Rect::new(1.0, 0.0, 319.0, 800.0),
        "the panel fills the pane past the hairline divider"
    );
    let content = labels.iter().find(|(_, w)| w.text == "content");
    let panel = labels.iter().find(|(_, w)| w.text == "panel-row");
    assert!(content.is_some() && panel.is_some(), "{:?}", probe.log());
    let panel = &panel.unwrap().1;
    assert!(
        panel.frame.size.width > 0.0,
        "the panel is laid out: {:?}",
        panel.frame
    );
    let content = &content.unwrap().1;
    assert_eq!(
        content.frame.size.width, 960.0,
        "the content yields the 320 pane"
    );
}

#[test]
fn hidden_and_sheet_inspectors_leave_the_full_content_width() {
    use day_reactive::flush_sync;
    use day_spec::{Event, WINDOW_NODE};
    use std::{cell::Cell, rc::Rc};

    let state = Rc::new(Cell::new(None));
    let output = state.clone();
    let probe = boot_wide(move || {
        let visible = Signal::new(false);
        output.set(Some(visible));
        inspector(visible, label("content"), || label("panel-row")).any()
    });
    let visible = state.get().unwrap();
    let width = || {
        probe
            .find_by_kind("day.label")
            .into_iter()
            .find(|(_, node)| node.text == "content")
            .unwrap()
            .1
            .frame
            .size
            .width
    };
    assert_eq!(width(), 1280.0, "a closed inspector reserves no width");
    visible.set(true);
    flush_sync();
    assert_eq!(width(), 960.0);
    visible.set(false);
    flush_sync();
    assert_eq!(width(), 1280.0, "closing restores the content width");

    probe.emit(WINDOW_NODE, Event::WindowResized(Size::new(390.0, 844.0)));
    flush_sync();
    assert_eq!(width(), 390.0);
    visible.set(true);
    flush_sync();
    assert_eq!(
        width(),
        390.0,
        "a sheet does not narrow the page beneath it"
    );
    let done = day_core::with_tree(|tree| tree.find_by_id("day-inspector-done")).unwrap();
    probe.emit(day_core::rnode_to_id(done), Event::Pressed);
    flush_sync();
    assert!(
        !visible.get_untracked(),
        "sheet dismissal updates the binding"
    );
    assert_eq!(width(), 390.0);

    visible.set(true);
    flush_sync();
    probe.emit(WINDOW_NODE, Event::WindowResized(Size::new(1280.0, 800.0)));
    flush_sync();
    assert_eq!(
        width(),
        960.0,
        "an open sheet becomes a side pane on resize"
    );
    visible.set(false);
    flush_sync();
    assert_eq!(width(), 1280.0);
    day_core::uninstall_tree();
}
