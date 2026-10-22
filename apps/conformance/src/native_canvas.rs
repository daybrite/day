// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Real mouse regression fixture for scripts/windows-canvas.py. Repaint continuously
//! between pointer press and release: replacing the hit surface used to lose both taps and drags.
use day::prelude::*;

pub fn page() -> impl Piece {
    let frames = Signal::new(0u32);
    let taps = Signal::new(0u32);
    let drags = Signal::new(0u32);
    column((
        rectangle()
            .fill(move || if frames.get() % 2 == 0 { Color::hex(0x4488cc) } else { Color::hex(0x5588cc) })
            .on_tap(move || taps.update(|v| *v += 1))
            .on_drag(move |drag| if drag.phase == DragPhase::Ended { drags.update(|v| *v += 1) })
            .frame(300.0, 200.0)
            .a11y(|a| a.label("Native canvas input"))
            .id(day::id!(native_canvas::pad)),
        label(move || format!("Taps: {}", taps.get())).id(day::id!(native_canvas::taps)),
        label(move || format!("Drags: {}", drags.get())).id(day::id!(native_canvas::drags)),
        label(move || format!("Frames: {}", frames.get())).id(day::id!(native_canvas::frames)),
        frame_clock(move |_| frames.update(|v| *v += 1)),
    ))
}
