// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use day::prelude::*;

day::resources!();
day::day_start!(options: window_options(), root);

pub fn window_options() -> WindowOptions {
    let mut options = res::window_options();
    options.title_fn = Some(|| res::str::app_title().format());
    options
}

// The website displays this function verbatim; the surrounding app setup stays in this file.
// Keep one component per line for the landing page; long lines scroll horizontally there.
#[rustfmt::skip]
pub fn root() -> impl Piece {
    let value = Signal::new(0.4);
    form((
        row((label(res::str::value(value)).font(Font::Title2), spacer(), progress(value))),
        labeled(res::str::slide(), slider(value).step(0.01)),
        labeled(res::str::step(), stepper(value).range(0.0..=1.0).step(0.01).format(NumberFormat::percent())),
        rectangle().height(32.0).fill(move || Color::hsl(value.get() * 360.0, 0.65, 0.5)),
        button(res::str::reset()).prominent().action(move || value.set(0.4)).aligned(Alignment::Center).grow(Dimension::Width),
    )).padding(Space::L)
}

// The iframe host forwards appearance changes without reloading or resetting the signal.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn website_demo_theme(dark: u32) {
    day::set_appearance(Some(dark != 0));
}
