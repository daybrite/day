use day::prelude::*;

use crate::widgets::battery_line;

pub(crate) fn about_page() -> AnyPiece {
    column((
        image(crate::res::images::day_logo).frame(96.0, 96.0),
        label(crate::res::str::app_title()).font(Font::Headline),
        label(crate::res::str::about_text()).id("about-text"),
        // A HEADLESS capability crate (day-part-battery, docs/battery.md): app Rust calls
        // `day_part_battery::status()` directly — no UI Piece — and shows the platform's native reading.
        label(battery_line()).id("battery-line"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
