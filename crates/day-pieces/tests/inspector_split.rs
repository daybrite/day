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
