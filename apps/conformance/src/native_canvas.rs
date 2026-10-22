// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Real mouse regression fixture for scripts/windows-canvas.py. Repaint continuously
//! between pointer press and release: replacing the hit surface used to lose both taps and drags.
use day::prelude::*;

pub fn page() -> impl Piece {
    let frames = Signal::new(0u32);
    let taps = Signal::new(0u32);
    let drags = Signal::new(0u32);
    let began = Signal::new(0u32);
    let changed = Signal::new(false);
    let delta = Signal::new(Point::ZERO);
    column((
        rectangle()
            .fill(move || {
                if frames.get() % 2 == 0 {
                    Color::hex(0x4488cc)
                } else {
                    Color::hex(0x5588cc)
                }
            })
            .on_tap(move || taps.update(|v| *v += 1))
            .on_drag(move |drag| match drag.phase {
                DragPhase::Began => began.update(|v| *v += 1),
                DragPhase::Changed => changed.set(true),
                DragPhase::Ended => {
                    delta.set(drag.translation);
                    drags.update(|v| *v += 1);
                }
            })
            .frame(300.0, 200.0)
            .a11y(|a| a.label("Native canvas input"))
            .id(day::id!(native_canvas::pad)),
        label(move || format!("Taps: {}", taps.get())).id(day::id!(native_canvas::taps)),
        label(move || format!("Drags: {}", drags.get())).id(day::id!(native_canvas::drags)),
        label(move || format!("Began: {}", began.get())).id(day::id!(native_canvas::began)),
        label(move || format!("Changed: {}", changed.get())).id(day::id!(native_canvas::changed)),
        label(move || format!("Delta: {:.0},{:.0}", delta.get().x, delta.get().y))
            .id(day::id!(native_canvas::delta)),
        // Native manipulation recognizers remove a small drag threshold from translation.
        label(move || {
            let expected = match drags.get() {
                1 => Point::new(80.0, 30.0),
                2 => Point::new(320.0, 0.0),
                _ => Point::ZERO,
            };
            let actual = delta.get();
            format!(
                "Delta valid: {}",
                (actual.x - expected.x).abs() <= 8.0 && (actual.y - expected.y).abs() <= 8.0
            )
        })
        .id(day::id!(native_canvas::delta_valid)),
        label(move || format!("Frames: {}", frames.get())).id(day::id!(native_canvas::frames)),
        frame_clock(move |_| frames.update(|v| *v += 1)),
    ))
}
