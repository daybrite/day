use day::prelude::*;

use crate::widgets::{battery_line, page};

/// About — the closing page: an identity hero (logo + name + blurb), then one info card of
/// live facts about this build and the platform it landed on: version, the native toolkit
/// compiled into the binary, the battery reading (day-part-battery, a headless part), and the
/// most recent app-lifecycle phase (docs/lifecycle.md).
pub(crate) fn about_page() -> AnyPiece {
    let hero = column((
        image(crate::res::images::day_logo).frame(96.0, 96.0),
        label(crate::res::str::app_title()).font(Font::Title2),
        label(crate::res::str::about_text())
            .font(Font::Footnote)
            .id("about-text"),
    ))
    .spacing(8.0)
    .align(HAlign::Center)
    // Centered within the page column (which is leading-aligned): grow to the full width so
    // the hero's own centering is visible.
    .grow_w();

    let info = section((
        labeled(
            crate::res::str::about_version(),
            label(env!("CARGO_PKG_VERSION")).id("about-version"),
        ),
        labeled(
            crate::res::str::about_toolkit(),
            label(day::toolkit_name()).id("about-toolkit"),
        ),
        labeled(
            crate::res::str::about_battery(),
            label(battery_line()).id("battery-line"),
        ),
        labeled(
            crate::res::str::menus_lifecycle(),
            label(move || crate::lifecycle_log().get()).id("about-lifecycle"),
        ),
    ))
    .title(crate::res::str::about_app_section());

    page(
        crate::res::str::nav_about(),
        "about-title",
        Some(crate::res::str::about_caption()),
        column((hero, form((info,))))
            .spacing(16.0)
            .align(HAlign::Leading)
            .any(),
    )
}
