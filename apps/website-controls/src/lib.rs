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
pub fn root() -> impl Piece {
    let value = Signal::new(40.0);
    form((
        label(res::str::current_value(value)).font(Font::Title2),
        labeled(res::str::adjust(), slider(value).range(0.0..=100.0).step(1.0)),
        labeled(res::str::edit(), stepper(value).range(0.0..=100.0)),
        progress(move || value.get() / 100.0),
        button(res::str::reset()).prominent().action(move || value.set(40.0)),
    ))
    .padding(Space::L)
}

// The iframe host forwards appearance changes without reloading or resetting the signal.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn website_demo_theme(dark: u32) {
    day::set_appearance(Some(dark != 0));
}
