// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Row fit policies (docs/size-classes.md "Row fit policies") on the mock toolkit: where
//! `RowFit::WrapColumns` places its cells, with and without a growing child.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_spec::WindowOptions;

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

/// Frames of every `day.label`, in handle-creation (= declaration) order.
fn label_frames(probe: &MockProbe) -> Vec<Rect> {
    probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.frame)
        .collect()
}

/// Three labels in a 100pt `WrapColumns` row with 10pt gutters, the first optionally growing.
/// The leading column places the row at its own 100pt rather than the window's width.
fn wrap_columns(grow_first: bool) -> AnyPiece {
    let first = if grow_first {
        label("aa").grow_w().any()
    } else {
        label("aa").any()
    };
    column((row((first, label("bbbb"), label("c")))
        .spacing(10.0)
        .fit(RowFit::WrapColumns { run_spacing: 6.0 })
        .width(100.0),))
    .align(HAlign::Leading)
    .any()
}

#[test]
fn wrap_columns_take_the_widest_child_width() {
    let probe = boot(|| wrap_columns(false));
    // Mock metrics are 8pt/char × 16pt line: the widest label is "bbbb" at 32pt, so 100pt fits
    // floor((100 + 10) / (32 + 10)) = 2 columns of 32pt, and the leftover trails them.
    let f = label_frames(&probe);
    assert_eq!(f[0], Rect::new(0.0, 0.0, 32.0, 16.0), "{f:?}");
    assert_eq!(f[1], Rect::new(42.0, 0.0, 32.0, 16.0), "{f:?}");
    assert_eq!(f[2], Rect::new(0.0, 22.0, 32.0, 16.0), "second line");
}

#[test]
fn a_growing_child_stretches_wrap_columns_to_the_width() {
    let probe = boot(|| wrap_columns(true));
    // Still 2 columns (32pt is the narrowest a column gets), now (100 − 10) / 2 = 45pt each, so
    // the first line spans the row. Every cell takes the column, rigid ones included.
    let f = label_frames(&probe);
    assert_eq!(f[0], Rect::new(0.0, 0.0, 45.0, 16.0), "{f:?}");
    assert_eq!(f[1], Rect::new(55.0, 0.0, 45.0, 16.0), "{f:?}");
    assert_eq!(f[2], Rect::new(0.0, 22.0, 45.0, 16.0), "second line");
}

#[test]
fn a_child_wider_than_the_row_wraps_inside_it() {
    // "a much longer label" is 19 characters, 152pt, in a 100pt row: its cell takes the row's
    // width and the text wraps, rather than the cell running past the row's edge.
    let probe = boot(|| {
        column((row((label("a much longer label"), label("c")))
            .spacing(10.0)
            .fit(RowFit::WrapColumns { run_spacing: 6.0 })
            .width(100.0),))
        .align(HAlign::Leading)
        .any()
    });
    let f = label_frames(&probe);
    assert_eq!(f[0].size.width, 100.0, "{f:?}");
    assert!(f[0].size.height > 16.0, "wrapped onto more lines: {f:?}");
    assert_eq!(f[1].origin.y, f[0].size.height + 6.0, "{f:?}");
}
