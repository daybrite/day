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
        row((label(res::str::value(value)).font(Font::Title2), progress(value))).spacing(Space::M),
        labeled(res::str::slide(), slider(value).step(0.01)),
        labeled(res::str::step(), stepper(value).range(0.0..=1.0).step(0.01).format(res::str::percentage, parse_percentage)),
        rounded_rectangle(8.0).height(32.0).fill(primary_tint()).opacity(value).a11y(|a| a.decorative()),
        button(res::str::reset()).prominent().action(move || value.set(0.4)).aligned(Alignment::Center).grow(Dimension::Width),
    )).padding(Space::L)
}

// This demo ships an English catalog. Accept its decimal notation, with or without
// a trailing percent sign: both "40" and "40%" mean 0.4. Fluent owns the display text.
fn parse_percentage(text: &str) -> Option<f64> {
    let text =
        text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{2068}' | '\u{2069}'));
    text.strip_suffix('%').unwrap_or(text).trim().parse::<f64>().ok().map(|v| v / 100.0)
}

// App helper, not a Day API: the browser bridge supplies its theme's current accent.
// Native builds use this app's blue; Day does not yet expose a native accent-color getter.
fn primary_tint() -> Signal<Color> {
    let tint = Signal::new(Color::hex(0x0a66ff));
    #[cfg(target_arch = "wasm32")]
    PRIMARY_TINT.set(Some(tint));
    tint
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PRIMARY_TINT: std::cell::Cell<Option<Signal<Color>>> = const { std::cell::Cell::new(None) };
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn website_demo_tint(rgb: u32) {
    if let Some(tint) = PRIMARY_TINT.get() {
        tint.set(Color::hex(rgb));
    }
}

// The iframe host forwards appearance changes without reloading or resetting the signal.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn website_demo_theme(dark: u32) {
    day::set_appearance(Some(dark != 0));
}
