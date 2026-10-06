// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A scroll view whose toolkit draws a classic scroll bar (docs/scroll.md): overflowing content
//! is laid out beside the bar, not under it, and content that fits keeps the full width.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_spec::{Size, WindowOptions};

fn boot(inset: f64, rows: usize) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    probe.state.borrow_mut().scroll_bar_inset = inset;
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, move || {
        let labels: Vec<AnyPiece> = (0..rows).map(|r| label(format!("row {r}")).any()).collect();
        scroll(column(PieceVec(labels))).any()
    });
    probe
}

fn content(probe: &MockProbe) -> Size {
    probe.find_by_kind("day.scroll")[0].1.scroll_content
}

#[test]
fn overflowing_content_fits_beside_a_classic_bar() {
    let probe = boot(15.0, 80);
    let c = content(&probe);
    assert!(
        c.height > 600.0,
        "the column must overflow to bring the bar up: {c:?}"
    );
    assert_eq!(c.width, 385.0, "laid out beside the 15pt bar, not under it");
}

#[test]
fn content_that_fits_keeps_the_full_width() {
    // No overflow, no bar: the inset must not narrow a page the bar never appears on.
    let probe = boot(15.0, 3);
    assert_eq!(content(&probe).width, 400.0);
}

#[test]
fn overlay_bars_take_no_room() {
    let probe = boot(0.0, 80);
    assert_eq!(content(&probe).width, 400.0);
}
