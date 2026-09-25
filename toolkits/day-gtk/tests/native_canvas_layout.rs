// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Run with a display (on headless Linux: xvfb-run cargo test -p day-gtk --test native_canvas_layout).
use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Rect, Size, Toolkit, kinds, props::CanvasProps};

fn main() {
    gtk4::init().expect("native canvas layout test needs a display");
    let mut toolkit = Gtk::new();
    let canvas = toolkit.realize(kinds::CANVAS, &CanvasProps::default(), NodeId(41));

    // The exact proposal comes from aspect_ratio's square. GrowLayout must be able to
    // measure the un-grown height without requiring an extra .grow_h() on every canvas.
    for edge in [144.0, 280.0, 96.0] {
        let square = Size::new(edge, edge);
        assert_eq!(
            toolkit.measure(&canvas, kinds::CANVAS, Proposal::exact(square)),
            square
        );
        toolkit.set_frame(&canvas, Rect::from_size(square), None);
    }
    // Prior allocations must not become an intrinsic size or prevent shrinking. An
    // unproposed axis stays zero; the drawing does not supply an intrinsic size either.
    for (proposal, expected) in [
        (Proposal::new(Some(72.0), None), Size::new(72.0, 0.0)),
        (Proposal::new(None, Some(80.0)), Size::new(0.0, 80.0)),
        (Proposal::new(None, None), Size::ZERO),
        (Proposal::exact(Size::ZERO), Size::ZERO),
    ] {
        assert_eq!(toolkit.measure(&canvas, kinds::CANVAS, proposal), expected);
    }
    toolkit.release(canvas);
    println!(
        "native canvas measurement: square proposals, shrinking, and unconstrained axes passed"
    );
}
